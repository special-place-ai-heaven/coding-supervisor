//! WP2: contract v2. A v2 contract loads with every field; a v1 contract loads only through the
//! in-memory migration and its file is never rewritten; a newer schema is refused; scope and
//! artifact paths that leave the repository are refused at load while every shape that loaded
//! before still loads; and the writers write v2.
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

use coding_supervisor::{Candidate, CandidateId, Isolation};
use serde_json::{Value, json};
use supervisor_claude_code::adapter::{Decision, Env, handle};
use supervisor_claude_code::candidate::CandidateProvider;
use supervisor_claude_code::level::{Level, Setting};
use supervisor_claude_code::state::{self, Stored, contract_from, load_gate, read_contract};
use supervisor_claude_code::{autonomy, lifecycle, sha256};

static NEXT: AtomicU64 = AtomicU64::new(0);
const ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../");

struct Dir(PathBuf);
impl Drop for Dir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn dir(name: &str) -> Dir {
    let path = std::env::temp_dir().join(format!(
        "supervisor-contract-v2-{name}-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = fs::remove_dir_all(&path);
    fs::create_dir_all(&path).unwrap();
    Dir(path)
}

fn v1() -> Value {
    json!({"id":"wp2","goal":"contract v2","scope":["src/"],"forbidden_effects":[],
        "max_repair_rounds":2,
        "criteria":[{"id":"C2","check":"test","mandatory":true,"command":"cargo test"}]})
}
fn v2() -> Value {
    let mut c = v1();
    c["schema_version"] = 2.into();
    c["base_sha"] = "0123456789abcdef0123456789abcdef01234567".into();
    c["approval"] = json!({"by":"person","at":1_700_000_000u64,"via":"task-start"});
    c["limits"] = json!({"wall_clock_s":3600,"turns":40});
    c["artifacts"] = json!(["docs/report.md"]);
    c["review"] = json!({"required":true,"independent":true});
    c["isolation"] = "worktree".into();
    c
}

#[test]
fn v2_loads_with_every_field() {
    let c = contract_from(&v2()).unwrap();
    assert_eq!(
        c.base_sha.as_deref(),
        Some("0123456789abcdef0123456789abcdef01234567")
    );
    let approval = c.approval.expect("approval");
    assert_eq!(approval.by.as_deref(), Some("person"));
    assert_eq!(approval.at, Some(1_700_000_000));
    assert_eq!(approval.via, "task-start");
    assert_eq!(c.limits.wall_clock_s.map(|n| n.get()), Some(3600));
    assert_eq!(c.limits.turns.map(|n| n.get()), Some(40));
    assert_eq!(c.artifacts, ["docs/report.md"]);
    assert!(c.review.required && c.review.independent);
    assert_eq!(c.isolation, Isolation::Worktree);
    // Every v2 field is optional; base_sha may be null outside a repository.
    let mut bare = v1();
    bare["schema_version"] = 2.into();
    bare["base_sha"] = Value::Null;
    let c = contract_from(&bare).unwrap();
    assert_eq!(c.base_sha, None);
    assert_eq!(c.approval, None);
    assert_eq!(c.isolation, Isolation::Shared);
    // The init draft's shape: only schema_version and isolation, no approval and no base.
    let mut draft = v1();
    draft["schema_version"] = 2.into();
    draft["isolation"] = "none".into();
    let c = contract_from(&draft).unwrap();
    assert_eq!((c.base_sha, c.approval), (None, None));
}

#[test]
fn v2_field_shapes_are_validated() {
    for (field, value) in [
        ("limits", json!({"turns":0})),
        ("limits", json!({"wall_clock_s":-1})),
        ("limits", json!({"turns":"ten"})),
        ("review", json!({"required":"yes","independent":false})),
        ("isolation", json!("container")),
        ("base_sha", json!(7)),
        ("approval", json!({"by":"person"})),
        ("artifacts", json!("docs/report.md")),
        ("limits", json!("1h")),
        ("limits", json!([])),
        ("review", json!(true)),
        ("review", json!([])),
    ] {
        let mut c = v2();
        c[field] = value.clone();
        assert!(contract_from(&c).is_err(), "{field}: {value}");
    }
}

#[test]
fn v1_loads_through_the_migration_with_fixed_defaults() {
    // main ignored `schema_version`, so anything but an integer of 2 or more is still v1.
    let versions = [
        None,
        Some(Value::Null),
        Some(json!(0)),
        Some(json!(1)),
        Some(json!(-2)),
        Some(json!("1")),
        Some(json!("2")),
        Some(json!(1.0)),
        Some(json!(2.0)),
        Some(json!(true)),
    ];
    for version in versions {
        let mut c = v1();
        if let Some(version) = &version {
            c["schema_version"] = version.clone();
        }
        let c = contract_from(&c).unwrap_or_else(|e| panic!("{version:?}: {e}"));
        assert_eq!(c.isolation, Isolation::Shared);
        assert_eq!(c.limits, Default::default());
        assert_eq!(c.review, Default::default());
        assert_eq!(c.base_sha, None);
        let approval = c
            .approval
            .expect("a migrated contract records its migration");
        assert_eq!(approval.via, "v1-migration");
        assert_eq!((approval.by, approval.at), (None, None));
    }
}

#[test]
fn a_newer_schema_is_refused_naming_this_binary() {
    let mut c = v1();
    c["schema_version"] = 3.into();
    let why = contract_from(&c).unwrap_err();
    assert!(why.contains("schema_version 3"), "{why}");
    assert!(why.contains(env!("CARGO_PKG_VERSION")), "{why}");
}

#[test]
fn paths_leaving_the_repository_are_refused_in_scope_and_artifacts() {
    for path in [
        "..",
        "../x",
        "src/../../x",
        "src\\..\\x",
        "/etc/passwd",
        "/",
        "C:\\x",
        "C:/x",
        "c:x",
        "\\\\server\\share",
        "\\x",
        "**/../x",
        "{..,src}/x",
        "src/{a,..}/x",
        "/**",
    ] {
        for field in ["scope", "artifacts"] {
            let mut c = v2();
            c[field] = json!([path]);
            let result = contract_from(&c);
            assert!(result.is_err(), "{field} {path:?} loaded");
        }
    }
}

/// Positive controls: every one of these loads on main, by design.
#[test]
fn every_shape_that_loaded_before_still_loads() {
    let mut contracts = vec![
        serde_json::from_str::<Value>(
            &fs::read_to_string(format!("{ROOT}supervisor-contract.json")).unwrap(),
        )
        .unwrap(),
    ];
    // The paths the intent path writes, the init draft's scope, an empty scope, and shapes
    // nothing writes but a person may have: backslashes, `./`, a glob inside the repository.
    for scope in [
        json!(["src/", "tests/"]),
        json!(["src/", "docs/README.md", "candidate.txt", "captures/"]),
        json!([]),
        json!([
            "src\\lib.rs",
            "./src/",
            "src/**/*.rs",
            "*.md",
            "a..b/",
            "~/x"
        ]),
        // A v1 scope leaving the repository loaded on main (it never matches a changed path),
        // so v1 still loads it; only v2 refuses such entries.
        json!([
            "../shared/",
            "/abs",
            "C:/x",
            "\\\\server\\share",
            "{..,src}/x"
        ]),
    ] {
        let mut c = v1();
        c["scope"] = scope;
        contracts.push(c);
    }
    for c in &contracts {
        contract_from(c).unwrap_or_else(|e| panic!("{c}: {e}"));
    }
    for fixture in [
        "fixtures/claude-code-enforcement/.supervisor/contract.json",
        "fixtures/claude-code-install/contract.json",
        "fixtures/codex-enforcement/captures/installed/contract.json",
        "fixtures/omp-enforcement/captures/installed/contract.json",
    ] {
        let v: Value =
            serde_json::from_str(&fs::read_to_string(format!("{ROOT}{fixture}")).unwrap()).unwrap();
        match contract_from(&v) {
            Ok(_) => {}
            // These two captures predate required commands; they did not load on main either.
            Err(why) => assert!(
                fixture.contains("installed") && why.contains("has no command"),
                "{fixture}: {why}"
            ),
        }
    }
}

struct Fake;
impl CandidateProvider for Fake {
    fn observe(&self) -> Result<Candidate, String> {
        Ok(Candidate {
            id: CandidateId {
                base_commit: "base".into(),
                dirty_digest: "one".into(),
            },
            provenance: "fake".into(),
            changed_paths: vec!["src/lib.rs".into()],
        })
    }
}

fn env(d: &Dir, contract: &Value, level: Level) -> Env {
    let state = d.0.join(".supervisor");
    fs::create_dir_all(&state).unwrap();
    fs::write(state.join("contract.json"), format!("{contract}\n")).unwrap();
    Env {
        state_dir: state,
        state_location: "test",
        candidate: Box::new(Fake),
        clock: || 1_000,
        level: Setting {
            level,
            ..Default::default()
        },
    }
}
fn write(env: &Env, id: &str, path: &Path) -> Value {
    json!({"hook_event_name":"PreToolUse","cwd":env.state_dir.parent().unwrap(),
        "session_id":"session","agent_id":"main","tool_use_id":id,"tool_name":"Write",
        "tool_input":{"file_path":path,"content":"x"}})
}
fn stop(env: &Env) -> Value {
    json!({"hook_event_name":"Stop","cwd":env.state_dir.parent().unwrap(),"session_id":"session",
        "stop_hook_active":false,"last_assistant_message":"done"})
}

#[test]
fn a_migrated_v1_contract_is_never_rewritten_and_its_gate_still_verifies() {
    let d = dir("migrate");
    let mut env = env(&d, &v1(), Level::Enforce);
    let path = env.state_dir.join("contract.json");
    let bytes = fs::read(&path).unwrap();
    handle(&write(&env, "t1", &d.0.join("src/lib.rs")), &mut env);
    handle(&stop(&env), &mut env);
    assert!(env.state_dir.join("gate.json").exists());
    handle(&stop(&env), &mut env);
    assert_eq!(fs::read(&path).unwrap(), bytes, "contract.json rewritten");
    let (contract, digest) = read_contract(&env.state_dir).unwrap();
    assert_eq!(digest, sha256::hex(&bytes));
    assert_eq!(contract.unwrap().approval.unwrap().via, "v1-migration");
    assert!(matches!(
        load_gate(&env.state_dir, &digest).unwrap(),
        Stored::Valid(_)
    ));
    let journal = fs::read_to_string(env.state_dir.join("events.jsonl")).unwrap();
    for line in journal.lines() {
        let line: Value = serde_json::from_str(line).unwrap();
        assert_eq!(line["v"], 1, "{line}");
        assert!(
            line["reasons"].as_array().unwrap().iter().any(|r| r
                .as_str()
                .unwrap()
                .contains("contract v1 migrated in memory")),
            "{line}"
        );
    }
}

fn denied_for_scope(decision: &Decision) -> bool {
    matches!(decision, Decision::Deny(why) if why.contains("outside the task's contract scope"))
}

#[test]
fn a_link_inside_the_repository_pointing_outside_is_out_of_scope() {
    let d = dir("link");
    let outside = dir("outside");
    fs::create_dir_all(d.0.join("src")).unwrap();
    let link = d.0.join("src").join("out");
    #[cfg(unix)]
    std::os::unix::fs::symlink(&outside.0, &link).unwrap();
    #[cfg(windows)]
    assert!(
        Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(&link)
            .arg(&outside.0)
            .output()
            .unwrap()
            .status
            .success(),
        "mklink /J failed"
    );
    let mut env = env(&d, &v2(), Level::Strict);
    let escape = handle(&write(&env, "t1", &link.join("x.rs")), &mut env);
    assert!(denied_for_scope(&escape), "{escape:?}");
    // A plain new file inside the scope, in a directory that does not exist yet.
    for (id, path) in [
        ("t2", d.0.join("src").join("new.rs")),
        ("t3", d.0.join("src").join("deep").join("new.rs")),
    ] {
        let inside = handle(&write(&env, id, &path), &mut env);
        assert!(!denied_for_scope(&inside), "{path:?}: {inside:?}");
    }
    #[cfg(windows)]
    let _ = fs::remove_dir(&link);
}

/// A link whose target does not exist cannot be resolved, so its name inside the scope proves
/// nothing: writing through it creates the target, wherever that is.
#[test]
fn a_dangling_link_inside_the_scope_is_out_of_scope() {
    let d = dir("dangling");
    let outside = dir("dangling-outside");
    fs::create_dir_all(d.0.join("src")).unwrap();
    #[cfg(unix)]
    let target = {
        let link = d.0.join("src").join("out.rs");
        std::os::unix::fs::symlink(outside.0.join("new.rs"), &link).unwrap();
        link
    };
    #[cfg(windows)]
    let target = {
        let link = d.0.join("src").join("out");
        let gone = outside.0.join("gone");
        fs::create_dir_all(&gone).unwrap();
        assert!(
            Command::new("cmd")
                .args(["/C", "mklink", "/J"])
                .arg(&link)
                .arg(&gone)
                .output()
                .unwrap()
                .status
                .success(),
            "mklink /J failed"
        );
        fs::remove_dir(&gone).unwrap();
        link.join("x.rs")
    };
    let mut env = env(&d, &v2(), Level::Strict);
    let decision = handle(&write(&env, "t1", &target), &mut env);
    assert!(denied_for_scope(&decision), "{decision:?}");
    #[cfg(windows)]
    let _ = fs::remove_dir(d.0.join("src").join("out"));
}

fn written(dir: &Path) -> Value {
    serde_json::from_str(&fs::read_to_string(dir.join("contract.json")).unwrap()).unwrap()
}
fn assert_v2(c: &Value, via: &str) {
    assert_eq!(c["schema_version"], 2, "{c}");
    assert_eq!(c["isolation"], "none", "{c}");
    assert_eq!(c["approval"]["via"], via, "{c}");
    assert!(c["approval"]["at"].as_u64().is_some(), "{c}");
    assert!(c.get("base_sha").is_some(), "{c}");
    contract_from(c).unwrap();
}

#[test]
fn task_start_writes_v2() {
    let d = dir("start");
    let root = d.0.join("state");
    let started = lifecycle::start(&root, &v1(), false, None, None).unwrap();
    let task = root
        .join("tasks")
        .join(started["task_id"].as_str().unwrap());
    let c = written(&task);
    assert_v2(&c, "task-start");
    // No project is named here, so no repository head is known.
    assert_eq!(c["base_sha"], Value::Null);
    // Replaying the same start request is still recognised as the same request.
    let mut request = v1();
    request["event_id"] = "once".into();
    let first = lifecycle::start(&d.0.join("replay"), &request, false, None, None).unwrap();
    let again = lifecycle::start(&d.0.join("replay"), &request, false, None, None).unwrap();
    assert_eq!(again["replayed"], true, "{again}");
    assert_eq!(first["task_id"], again["task_id"]);
}

fn git(project: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(project)
        .args([
            "-c",
            "user.name=fixture",
            "-c",
            "user.email=fixture@example.invalid",
        ])
        .args(["-c", "commit.gpgsign=false"])
        .args(args)
        .output()
        .unwrap();
    assert!(out.status.success(), "git {args:?}: {out:?}");
    String::from_utf8(out.stdout).unwrap().trim().to_owned()
}

#[test]
fn agent_proposals_write_v2_and_a_revision_keeps_the_base() {
    let d = dir("autonomy");
    let (project, root) = (d.0.join("project"), d.0.join("state"));
    fs::create_dir_all(&project).unwrap();
    fs::write(
        project.join("Cargo.toml"),
        "[package]\nname = \"fixture\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    git(&project, &["init", "-q"]);
    git(&project, &["add", "Cargo.toml"]);
    git(&project, &["commit", "-q", "-m", "one"]);
    let base = git(&project, &["rev-parse", "HEAD"]);
    let apply = |id: &str, action: &str, task: Option<&str>, contract: Value| {
        let event = autonomy::record_prompt(
            &root,
            &project,
            &json!({"session_id":"session","prompt_id":id,"prompt":"Implement a small change"}),
        )
        .unwrap();
        autonomy::propose(
            &root,
            &project,
            &json!({"event_id":event["event_id"],"nonce":event["nonce"],"prompt_hash":event["prompt_hash"],"action":action,"task_id":task,"contract":contract}),
        )
        .unwrap()
    };
    let mut proposal = v1();
    proposal["criteria"][0]["command"] = "cargo test -p fixture".into();
    let started = apply("first", "start", None, proposal);
    let id = started["task_id"].as_str().unwrap().to_owned();
    let task = root.join("tasks").join(&id);
    let c = written(&task);
    assert_v2(&c, "agent-proposal");
    assert_eq!(c["base_sha"], base.as_str());
    // The agent's own environment names the user; that must not read as a person's approval.
    assert_eq!(c["approval"]["by"], Value::Null, "{c}");

    fs::write(project.join("README"), "two\n").unwrap();
    git(&project, &["add", "README"]);
    git(&project, &["commit", "-q", "-m", "two"]);
    let mut revised = c.clone();
    revised["criteria"]
        .as_array_mut()
        .unwrap()
        .push(json!({"id":"C9","check":"extra","mandatory":false,"command":"cargo test"}));
    // An agent cannot assert who approved its contract; the writer records itself.
    revised["approval"] = json!({"by":"someone else","at":1,"via":"task-start"});
    apply("second", "revise", Some(&id), revised);
    let c = written(&task);
    assert_v2(&c, "agent-proposal");
    assert_eq!(
        c["base_sha"],
        base.as_str(),
        "a revision keeps the task's base"
    );
    assert_ne!(c["approval"]["by"], "someone else");
    assert!(state::read_contract(&task).unwrap().0.is_some());
}

fn bad_inputs() -> Vec<Value> {
    [
        ("limits", json!({"turns":0})),
        ("limits", json!({"wall_clock_s":"60"})),
        ("review", json!({"required":true})),
        ("artifacts", json!(["../out"])),
        ("artifacts", json!("x")),
    ]
    .into_iter()
    .map(|(field, value)| {
        let mut c = v1();
        c[field] = value;
        c
    })
    .collect()
}

/// What is saved is what is checked: a v1-shaped request whose v2 fields are malformed is
/// refused before anything is written, instead of saving a contract that never loads again.
#[test]
fn a_saved_contract_is_checked_before_it_is_written() {
    for (i, c) in bad_inputs().into_iter().enumerate() {
        let d = dir(&format!("bad-{i}"));
        let root = d.0.join("state");
        assert!(
            lifecycle::start(&root, &c, false, None, None).is_err(),
            "{c}"
        );
        assert!(!root.join("tasks").exists(), "{c}: a task was written");
        assert!(
            !root.join("active.json").exists(),
            "{c}: a task was activated"
        );
    }
}

#[test]
fn an_agent_proposal_start_is_checked_before_it_is_written() {
    let d = dir("bad-agent");
    let (project, root) = (d.0.join("project"), d.0.join("state"));
    fs::create_dir_all(&project).unwrap();
    fs::write(
        project.join("Cargo.toml"),
        "[package]\nname = \"fixture\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    for (i, mut proposal) in bad_inputs().into_iter().enumerate() {
        proposal["criteria"][0]["command"] = "cargo test -p fixture".into();
        let event = autonomy::record_prompt(
            &root,
            &project,
            &json!({"session_id":"session","prompt_id":format!("p{i}"),"prompt":"Implement a small change"}),
        )
        .unwrap();
        let result = autonomy::propose(
            &root,
            &project,
            &json!({"event_id":event["event_id"],"nonce":event["nonce"],"prompt_hash":event["prompt_hash"],"action":"start","task_id":null,"contract":proposal}),
        );
        assert!(result.is_err(), "{proposal}: {result:?}");
        assert!(
            !root.join("tasks").exists(),
            "{proposal}: a task was written"
        );
    }
}

/// WP4 counts wall clock from `approval.at`, so a revision keeps the approval, the isolation
/// and the base the task started with, whatever the revised contract says.
#[test]
fn a_revision_keeps_the_approval_isolation_and_base() {
    let d = dir("revise-keeps");
    let (project, root) = (d.0.join("project"), d.0.join("state"));
    fs::create_dir_all(&project).unwrap();
    fs::create_dir(project.join(".git")).unwrap();
    let mut operator = v1();
    operator["criteria"][0]["command"] = "python verify.py".into();
    let started = lifecycle::start(&root, &operator, false, None, None).unwrap();
    let id = started["task_id"].as_str().unwrap().to_owned();
    let task = root.join("tasks").join(&id);
    let before = written(&task);
    let apply = |prompt: &str, action: &str, contract: Option<Value>| {
        let event = autonomy::record_prompt(
            &root,
            &project,
            &json!({"session_id":"session","prompt_id":prompt,"prompt":"Continue the approved work"}),
        )
        .unwrap();
        autonomy::propose(
            &root,
            &project,
            &json!({"event_id":event["event_id"],"nonce":event["nonce"],"prompt_hash":event["prompt_hash"],"action":action,"task_id":id,"contract":contract}),
        )
        .unwrap()
    };
    apply("continue", "continue", None);
    let mut revised = before.clone();
    revised["criteria"]
        .as_array_mut()
        .unwrap()
        .push(json!({"id":"C9","check":"extra","mandatory":false,"command":"python verify.py"}));
    revised["approval"] = json!({"by":null,"at":1,"via":"agent-proposal"});
    revised["isolation"] = "worktree".into();
    revised["base_sha"] = "0123456789abcdef0123456789abcdef01234567".into();
    apply("revise", "revise", Some(revised));
    let after = written(&task);
    assert_eq!(after["criteria"].as_array().unwrap().len(), 2, "{after}");
    for key in ["approval", "isolation", "base_sha"] {
        assert_eq!(after.get(key), before.get(key), "{key}: {after}");
    }
}

/// A caller-supplied isolation, approval or base_sha is replaced by the writer's record; the
/// start says so, as it does for forbidden effects.
#[test]
fn start_warns_when_it_replaces_caller_fields() {
    let d = dir("warn");
    let mut c = v1();
    c["isolation"] = "worktree".into();
    c["approval"] = json!({"by":"someone","at":1,"via":"elsewhere"});
    c["base_sha"] = "0123456789abcdef0123456789abcdef01234567".into();
    let started = lifecycle::start(&d.0.join("state"), &c, false, None, None).unwrap();
    let warnings = started["warnings"].to_string();
    for key in ["isolation", "approval", "base_sha"] {
        assert!(warnings.contains(key), "{key}: {started}");
    }
}
