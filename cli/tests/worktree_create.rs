//! WP3 (gate W3.2): `supervisor task start --worktree <path>` creates a git worktree on branch
//! `supervisor/<task id>` at the contract's base commit, checks its path, branch, base and that
//! it is clean, binds the person's task to it (task.json) with `isolation: worktree`, and the
//! hooks then judge that worktree from the main checkout. Edits in the main checkout are refused
//! while such a task is bound. `supervisor task cleanup` never forces, unlinks links first and
//! keeps the branch and any work without a receipt.

mod support;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Output, Stdio};

use serde_json::{Value, json};
use support::*;

/// The worktree path the tests ask for: beside the project, inside the fixture root.
fn wt(f: &Fixture) -> PathBuf {
    f.root.join("wt")
}

fn contract_file(f: &Fixture, extra: Value) -> PathBuf {
    let mut contract = json!({"id":"wp3","goal":"work in a created worktree",
        "criteria":[{"id":"C1","check":"candidate_passes_reported_test","mandatory":true,
            "command":"cargo test -p fixture"}],
        "scope":["src/"],"forbidden_effects":[],"max_repair_rounds":2});
    for (key, value) in extra.as_object().into_iter().flatten() {
        contract[key] = value.clone();
    }
    let file = f.root.join("contract.json");
    fs::write(&file, contract.to_string()).unwrap();
    file
}

/// `supervisor task start --worktree <path>` for `project`.
fn start_in(f: &Fixture, project: &Path, worktree: &Path) -> Output {
    let file = contract_file(f, json!({}));
    f.command(BIN)
        .args(["task", "start", "--contract"])
        .arg(&file)
        .arg("--worktree")
        .arg(worktree)
        .arg("--project")
        .arg(project)
        .output()
        .unwrap()
}

/// A worktree start for the fixture project that must succeed; its response.
fn start(f: &Fixture) -> Value {
    let out = start_in(f, &f.project, &wt(f));
    assert!(out.status.success(), "task start --worktree: {out:?}");
    serde_json::from_slice(&out.stdout).unwrap()
}

/// The state root of the fixture project (the one holding active.json).
fn state_root(f: &Fixture) -> Option<PathBuf> {
    fs::read_dir(&f.state)
        .ok()?
        .map(|e| e.unwrap().path())
        .find(|p| p.join("active.json").exists())
}

fn active_task(f: &Fixture) -> Option<String> {
    let root = state_root(f)?;
    let pointer: Value =
        serde_json::from_str(&fs::read_to_string(root.join("active.json")).ok()?).ok()?;
    pointer["task_id"].as_str().map(str::to_owned)
}

fn task_file(f: &Fixture, task: &str, name: &str) -> Value {
    let path = state_root(f).unwrap().join("tasks").join(task).join(name);
    serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
}

/// The form the gate stores a path in (`location::normalize`).
fn normalized(path: &Path) -> String {
    let text = fs::canonicalize(path)
        .unwrap()
        .to_string_lossy()
        .replace('\\', "/");
    let text = text.strip_prefix("//?/").unwrap_or(&text).to_owned();
    if cfg!(windows) {
        text.to_lowercase()
    } else {
        text
    }
}

/// A hook call whose `cwd` is `cwd`.
fn hook_at(f: &Fixture, cwd: &Path, mut payload: Value) -> Value {
    payload["cwd"] = cwd.to_str().unwrap().into();
    if payload.get("session_id").is_none() {
        payload["session_id"] = "wp3-session".into();
    }
    let mut child = f
        .command(BIN)
        .arg("hook")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    use std::io::Write as _;
    child
        .stdin
        .take()
        .unwrap()
        .write_all(payload.to_string().as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    let text = String::from_utf8(out.stdout).unwrap();
    if text.trim().is_empty() {
        Value::Null
    } else {
        serde_json::from_str(&text).unwrap()
    }
}

fn write_at(f: &Fixture, cwd: &Path, file: &Path, id: &str) -> Value {
    hook_at(
        f,
        cwd,
        json!({"hook_event_name":"PreToolUse","tool_use_id":id,"agent_id":"main","tool_name":"Write",
            "tool_input":{"file_path":file.to_str().unwrap(),"content":"// edit\n"}}),
    )
}

fn denied(decision: &Value) -> bool {
    decision["hookSpecificOutput"]["permissionDecision"] == "deny"
}

fn reason(decision: &Value) -> String {
    decision["hookSpecificOutput"]["permissionDecisionReason"]
        .as_str()
        .unwrap_or_default()
        .to_owned()
}

/// The mandatory check as a Terminal Commander run in the worktree, from a hook in `hook_cwd`.
fn tc_check(f: &Fixture, hook_cwd: &Path, run_cwd: &Path, id: &str) {
    let mut payload = json!({"hook_event_name":"PreToolUse","tool_use_id":id,"agent_id":"main",
        "tool_name":"mcp__terminal_commander__run_and_watch",
        "tool_input":{"argv":["cargo","test","-p","fixture"],"cwd":run_cwd.to_str().unwrap()}});
    assert!(!denied(&hook_at(f, hook_cwd, payload.clone())));
    payload["hook_event_name"] = "PostToolUse".into();
    payload["tool_response"] = json!({"job_id":format!("job-{id}"),"bucket_id":"bucket",
        "state":"exited","exit_code":0,"complete":true,"outcome_trust":"observed",
        "degraded":false,"pipeline_exit_masked":false});
    hook_at(f, hook_cwd, payload);
}

fn stop_at(f: &Fixture, cwd: &Path) -> Value {
    hook_at(
        f,
        cwd,
        json!({"hook_event_name":"Stop","stop_hook_active":false,
            "last_assistant_message":"Done; cargo test -p fixture passed."}),
    )
}

/// Whether `task` is completed, in whichever state root holds it.
fn completed(f: &Fixture, task: &str) -> bool {
    fs::read_dir(&f.state).unwrap().any(|root| {
        let gate = root
            .unwrap()
            .path()
            .join("tasks")
            .join(task)
            .join("gate.json");
        fs::read_to_string(gate).is_ok_and(|g| g.contains("\"state\":\"completed\""))
    })
}

/// Start in a worktree, edit `src/lib.rs` there from the main checkout and pass the check
/// there: the task completes. Returns the task id.
fn complete_in_worktree(f: &Fixture) -> String {
    let task = start(f)["task_id"].as_str().unwrap().to_owned();
    let file = wt(f).join("src").join("lib.rs");
    assert!(!denied(&write_at(f, &f.project, &file, "edit-1")));
    fs::write(&file, "// two\n").unwrap();
    tc_check(f, &f.project, &wt(f), "check-1");
    let stop = stop_at(f, &f.project);
    assert_ne!(stop["decision"], "block", "{stop}");
    assert!(completed(f, &task));
    task
}

fn cleanup(f: &Fixture, task: &str) -> Output {
    f.run(&["task", "cleanup", "--task", task])
}

fn deactivate(f: &Fixture, task: &str) {
    let out = f.run(&[
        "task",
        "deactivate",
        "--task",
        task,
        "--disposition",
        "cancelled",
    ]);
    assert!(out.status.success(), "{out:?}");
}

fn branches(f: &Fixture) -> String {
    f.git(&["branch", "--list", "supervisor/*"])
}

/// Whether git still lists the fixture's `wt` worktree.
fn listed(f: &Fixture) -> bool {
    f.git(&["worktree", "list", "--porcelain"])
        .lines()
        .filter_map(|l| l.strip_prefix("worktree "))
        .any(|p| p.replace('\\', "/").ends_with("/wt"))
}

#[test]
fn creates_the_worktree_on_its_own_branch_at_the_base_and_clean() {
    let f = Fixture::new("wt-create");
    let head = f.head();
    let response = start(&f);
    let task = response["task_id"].as_str().unwrap();
    let wt = wt(&f);
    assert_eq!(
        response["branch"],
        format!("supervisor/{task}"),
        "{response}"
    );
    assert_eq!(response["base_sha"], head.as_str(), "{response}");
    assert_eq!(
        f.git_in(&wt, &["rev-parse", "--abbrev-ref", "HEAD"]),
        format!("supervisor/{task}")
    );
    assert_eq!(f.git_in(&wt, &["rev-parse", "HEAD"]), head);
    assert_eq!(
        f.git_in(&wt, &["status", "--porcelain", "--untracked-files=all"]),
        ""
    );
    let contract = task_file(&f, task, "contract.json");
    assert_eq!(contract["isolation"], "worktree");
    assert_eq!(contract["base_sha"], head.as_str());
    assert_eq!(
        task_file(&f, task, "task.json")["worktree"],
        normalized(&wt).as_str()
    );
    let status = String::from_utf8(f.run(&["status", "--json"]).stdout).unwrap();
    assert!(status.contains(&normalized(&wt)), "{status}");
}

/// `supervisor task start --contract <file>` with `--worktree <path>` when one is given.
fn start_file(f: &Fixture, file: &Path, worktree: Option<&Path>) -> Output {
    let mut command = f.command(BIN);
    command.args(["task", "start", "--contract"]).arg(file);
    if let Some(worktree) = worktree {
        command.arg("--worktree").arg(worktree);
    }
    command.arg("--project").arg(&f.project).output().unwrap()
}

/// WP3 review r3: a replayed start (same event ID and contract text) must not drop `--worktree`
/// silently; the same worktree replays as before.
#[test]
fn a_replayed_start_with_another_worktree_setting_is_refused() {
    let f = Fixture::new("wt-replay");
    let file = contract_file(&f, json!({"event_id":"start-1"}));
    let out = start_file(&f, &file, None);
    assert!(out.status.success(), "{out:?}");
    let out = start_file(&f, &file, Some(&wt(&f)));
    assert!(!out.status.success(), "{out:?}");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("different worktree setting"), "{stderr}");
    assert!(!wt(&f).exists());

    let g = Fixture::new("wt-replay-same");
    let file = contract_file(&g, json!({"event_id":"start-1"}));
    let first = start_file(&g, &file, Some(&wt(&g)));
    assert!(first.status.success(), "{first:?}");
    let again = start_file(&g, &file, Some(&wt(&g)));
    assert!(again.status.success(), "{again:?}");
    let v: Value = serde_json::from_slice(&again.stdout).unwrap();
    assert_eq!(v["replayed"], true, "{v}");
    let other = start_file(&g, &file, Some(&g.root.join("wt2")));
    assert!(!other.status.success(), "{other:?}");
    assert!(!g.root.join("wt2").exists());
}

/// WP3 review r3: the warning for a caller's `isolation` names what the gate stored.
#[test]
fn a_worktree_start_over_a_callers_isolation_none_says_worktree() {
    let f = Fixture::new("wt-warn");
    let file = contract_file(&f, json!({"isolation":"none"}));
    let out = start_file(&f, &file, Some(&wt(&f)));
    assert!(out.status.success(), "{out:?}");
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    let warnings = v["warnings"].to_string();
    assert!(
        warnings.contains("isolation worktree") && !warnings.contains("isolation none"),
        "{v}"
    );
}

#[test]
fn a_plain_start_keeps_isolation_none_and_no_binding() {
    let f = Fixture::new("wt-plain");
    let task = f.start(json!({}));
    assert_eq!(task_file(&f, &task, "contract.json")["isolation"], "none");
    assert!(task_file(&f, &task, "task.json").get("worktree").is_none());
    let decision = write_at(&f, &f.project, &f.project.join("src").join("lib.rs"), "e");
    assert!(!denied(&decision), "{decision}");
}

#[test]
fn a_worktree_left_dirty_at_checkout_is_refused_and_no_task_starts() {
    let f = Fixture::new("wt-dirty");
    let hooks = f.root.join("hooks");
    fs::create_dir_all(&hooks).unwrap();
    fs::write(
        hooks.join("post-checkout"),
        "#!/bin/sh\necho left > left-by-hook.txt\n",
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(
            hooks.join("post-checkout"),
            fs::Permissions::from_mode(0o755),
        )
        .unwrap();
    }
    f.git(&["config", "core.hooksPath", hooks.to_str().unwrap()]);
    let out = start_in(&f, &f.project, &wt(&f));
    assert!(!out.status.success(), "{out:?}");
    let why = String::from_utf8_lossy(&out.stderr);
    assert!(why.contains("not clean"), "{why}");
    // Never forced: the worktree with the hook's file is left for the person, and named.
    assert!(wt(&f).join("left-by-hook.txt").exists());
    assert!(why.contains("supervisor/task-"), "{why}");
    assert_eq!(active_task(&f), None);
}

#[test]
fn an_existing_path_is_refused_and_nothing_starts() {
    let f = Fixture::new("wt-exists");
    fs::create_dir_all(wt(&f)).unwrap();
    fs::write(wt(&f).join("keep.txt"), "mine\n").unwrap();
    let out = start_in(&f, &f.project, &wt(&f));
    assert!(!out.status.success(), "{out:?}");
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("already exists"),
        "{out:?}"
    );
    assert_eq!(
        fs::read_to_string(wt(&f).join("keep.txt")).unwrap(),
        "mine\n"
    );
    assert_eq!(branches(&f), "");
    assert_eq!(active_task(&f), None);
}

#[test]
fn a_path_inside_the_main_checkout_is_refused() {
    let f = Fixture::new("wt-nested");
    let out = start_in(&f, &f.project, &f.project.join("sub"));
    assert!(!out.status.success(), "{out:?}");
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("inside the main checkout"),
        "{out:?}"
    );
    assert!(!f.project.join("sub").exists());
    assert_eq!(branches(&f), "");
    assert_eq!(active_task(&f), None);
}

#[test]
fn a_repository_without_a_commit_is_refused() {
    let f = Fixture::new("wt-nocommit");
    let empty = f.root.join("empty");
    fs::create_dir_all(&empty).unwrap();
    f.git_in(&empty, &["init", "-q"]);
    let out = start_in(&f, &empty, &f.root.join("empty-wt"));
    assert!(!out.status.success(), "{out:?}");
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("needs a commit"),
        "{out:?}"
    );
    assert!(!f.root.join("empty-wt").exists());
}

#[test]
fn one_task_per_worktree_and_the_path_is_free_again_after_cleanup() {
    let f = Fixture::new("wt-one");
    let first = start(&f)["task_id"].as_str().unwrap().to_owned();
    let file = contract_file(&f, json!({}));
    let next = f.run(&[
        "task",
        "next",
        "--task",
        &first,
        "--disposition",
        "cancelled",
        "--contract",
        file.to_str().unwrap(),
        "--worktree",
        wt(&f).to_str().unwrap(),
    ]);
    assert!(!next.status.success(), "{next:?}");
    assert_eq!(active_task(&f).as_deref(), Some(first.as_str()));
    deactivate(&f, &first);
    let again = start_in(&f, &f.project, &wt(&f));
    assert!(!again.status.success(), "{again:?}");
    let out = cleanup(&f, &first);
    assert!(out.status.success(), "{out:?}");
    let second = start(&f)["task_id"].as_str().unwrap().to_owned();
    assert_ne!(first, second);
}

#[test]
fn a_person_task_is_judged_on_its_worktree_from_the_main_checkout() {
    let f = Fixture::new("wt-judged");
    let task = complete_in_worktree(&f);
    // verdict and export judge the worktree, and the bundle lands in it.
    let verdict = f.run(&["verdict", "--json"]);
    assert_eq!(verdict.status.code(), Some(0), "{verdict:?}");
    let (code, value) = f.export();
    assert_eq!(code, 0, "{value}");
    let bundle = wt(&f)
        .join("supervisor-receipts")
        .join(format!("{task}.json"));
    assert!(bundle.is_file(), "{value}");
    assert!(!f.project.join("supervisor-receipts").exists());
    let exported: Value = serde_json::from_str(&fs::read_to_string(&bundle).unwrap()).unwrap();
    assert_eq!(exported["receipts"][0]["cwd"], ".", "{exported}");
    assert_eq!(
        exported["changed_paths"],
        json!(["src/lib.rs"]),
        "{exported}"
    );
}

/// A session working in the worktree itself: its hook calls are redirected to the main
/// checkout's task, and a Bash check there counts.
#[test]
fn a_bash_check_in_the_worktree_counts() {
    let f = Fixture::new("wt-bash-wt");
    let task = start(&f)["task_id"].as_str().unwrap().to_owned();
    let wt = wt(&f);
    let file = wt.join("src").join("lib.rs");
    assert!(!denied(&write_at(&f, &wt, &file, "edit-1")));
    fs::write(&file, "// two\n").unwrap();
    let mut payload = json!({"hook_event_name":"PreToolUse","tool_use_id":"bash-1",
        "agent_id":"main","tool_name":"Bash","tool_input":{"command":"cargo test -p fixture"}});
    assert!(!denied(&hook_at(&f, &wt, payload.clone())));
    payload["hook_event_name"] = "PostToolUse".into();
    payload["tool_response"] = json!({"stdout":"ok","stderr":"","interrupted":false});
    hook_at(&f, &wt, payload);
    let stop = stop_at(&f, &wt);
    assert_ne!(stop["decision"], "block", "{stop}");
    assert!(completed(&f, &task));
}

#[test]
fn task_next_creates_a_new_worktree_and_archives_the_previous_task() {
    let f = Fixture::new("wt-next");
    let first = start(&f)["task_id"].as_str().unwrap().to_owned();
    let second_path = f.root.join("wt2");
    let file = contract_file(&f, json!({}));
    let out = f.run(&[
        "task",
        "next",
        "--task",
        &first,
        "--disposition",
        "cancelled",
        "--contract",
        file.to_str().unwrap(),
        "--worktree",
        second_path.to_str().unwrap(),
    ]);
    assert!(out.status.success(), "{out:?}");
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    let second = v["task_id"].as_str().unwrap();
    assert_eq!(v["recovery_of"], first.as_str(), "{v}");
    assert_eq!(
        f.git_in(&second_path, &["rev-parse", "--abbrev-ref", "HEAD"]),
        format!("supervisor/{second}")
    );
    assert_eq!(
        task_file(&f, second, "task.json")["worktree"],
        normalized(&second_path).as_str()
    );
    // The first task's worktree stays until its own cleanup.
    assert!(wt(&f).exists());
}

/// A start whose journal is written but whose install fails keeps its new worktree: the journal
/// finishes the start at the next lock, so removing the worktree would bind the task to nothing.
#[test]
fn a_start_left_in_its_journal_keeps_the_new_worktree() {
    let f = Fixture::new("wt-journal");
    let first = start(&f)["task_id"].as_str().unwrap().to_owned();
    let archive = state_root(&f)
        .unwrap()
        .join("archives")
        .join(format!("{first}.json"));
    fs::create_dir_all(archive.parent().unwrap()).unwrap();
    fs::write(&archive, "{\"other\":true}\n").unwrap();
    let second_path = f.root.join("wt2");
    let file = contract_file(&f, json!({}));
    let out = f.run(&[
        "task",
        "next",
        "--task",
        &first,
        "--disposition",
        "cancelled",
        "--contract",
        file.to_str().unwrap(),
        "--worktree",
        second_path.to_str().unwrap(),
    ]);
    assert!(!out.status.success(), "{out:?}");
    let why = String::from_utf8_lossy(&out.stderr);
    assert!(!why.contains("was not started"), "{why}");
    assert!(second_path.exists(), "{why}");
    assert!(branches(&f).lines().count() == 2, "{}", branches(&f));
    // Once the cause is gone, the next command finishes the start, bound to the kept worktree.
    fs::remove_file(&archive).unwrap();
    let status = f.run(&["status", "--json"]);
    assert!(status.status.success(), "{status:?}");
    let second = active_task(&f).unwrap();
    assert_ne!(second, first);
    assert_eq!(
        task_file(&f, &second, "task.json")["worktree"],
        normalized(&second_path).as_str()
    );
    assert_eq!(
        f.git_in(&second_path, &["rev-parse", "--abbrev-ref", "HEAD"]),
        format!("supervisor/{second}")
    );
}

#[test]
fn a_bash_check_from_the_main_checkout_does_not_count_for_a_worktree_task() {
    let f = Fixture::new("wt-bash-main");
    let task = start(&f)["task_id"].as_str().unwrap().to_owned();
    fs::write(wt(&f).join("src").join("lib.rs"), "// two\n").unwrap();
    f.run_check("cargo test -p fixture", 0, "ok");
    let stop = f.stop("Done; cargo test -p fixture passed.");
    assert_eq!(stop["decision"], "block", "{stop}");
    assert!(!completed(&f, &task));
}

#[test]
fn a_base_checkout_edit_is_refused_when_isolation_is_worktree() {
    let f = Fixture::new("wt-base-edit");
    start(&f);
    let base = f.project.join("src").join("lib.rs");
    let decision = write_at(&f, &f.project, &base, "base-1");
    assert!(denied(&decision), "{decision}");
    assert!(reason(&decision).contains("base checkout"), "{decision}");
    let inside = write_at(&f, &f.project, &wt(&f).join("src").join("lib.rs"), "wt-1");
    assert!(!reason(&inside).contains("base checkout"), "{inside}");
    assert!(!denied(&inside), "{inside}");
    // At audit the same edit goes ahead with the refusal reported.
    let level = f.run(&["level", "audit"]);
    assert!(level.status.success(), "{level:?}");
    let audit = write_at(&f, &f.project, &base, "base-2");
    assert!(!denied(&audit), "{audit}");
    assert!(audit.to_string().contains("base checkout"), "{audit}");
}

/// An edit tool cannot rewrite the worktree's `.git` file.
#[test]
fn an_edit_of_the_worktree_git_file_is_refused() {
    let f = Fixture::new("wt-git-edit");
    start(&f);
    let wt = wt(&f);
    let dot_git = wt.join(".git");
    for (cwd, id) in [(&f.project, "git-1"), (&wt, "git-2")] {
        let decision = write_at(&f, cwd, &dot_git, id);
        assert!(denied(&decision), "{decision}");
        assert!(reason(&decision).contains(".git"), "{decision}");
    }
}

/// The base checkout comes from the gate's own state: a `.git` file rewritten some other way
/// (a shell write is a declared limit) does not switch the refusal off.
#[test]
fn a_rewritten_worktree_git_file_keeps_the_base_refusal() {
    let f = Fixture::new("wt-git-file");
    start(&f);
    let dot_git = wt(&f).join(".git");
    let original = fs::read_to_string(&dot_git).unwrap();
    // Still the same repository, so the candidate stays observable; only the link shape changes.
    fs::write(
        &dot_git,
        format!("gitdir: {}\n", f.project.join(".git").display()),
    )
    .unwrap();
    let base = f.project.join("src").join("lib.rs");
    let decision = write_at(&f, &f.project, &base, "base-1");
    fs::write(&dot_git, original).unwrap();
    assert!(denied(&decision), "{decision}");
    assert!(reason(&decision).contains("base checkout"), "{decision}");
}

/// A task started from a linked worktree refuses edits there and in the primary checkout.
#[test]
fn a_start_from_a_linked_worktree_guards_it_and_the_primary_checkout() {
    let f = Fixture::new("wt-from-linked");
    let linked = f.root.join("linked");
    f.git(&[
        "worktree",
        "add",
        "-q",
        "-b",
        "fixture-linked",
        linked.to_str().unwrap(),
        "HEAD",
    ]);
    let out = start_in(&f, &linked, &wt(&f));
    assert!(out.status.success(), "{out:?}");
    for (file, id) in [
        (linked.join("src").join("lib.rs"), "linked-1"),
        (f.project.join("src").join("lib.rs"), "primary-1"),
    ] {
        let decision = write_at(&f, &linked, &file, id);
        assert!(denied(&decision), "{decision}");
        assert!(reason(&decision).contains("base checkout"), "{decision}");
    }
    let inside = write_at(&f, &linked, &wt(&f).join("src").join("lib.rs"), "wt-1");
    assert!(!denied(&inside), "{inside}");
}

/// A session working in the new worktree of a task started from a linked worktree finds that
/// task in the linked worktree's state: edits in both checkouts are refused and its Bash check
/// counts. Before the fix it looked only in the primary checkout's state and found nothing.
#[test]
fn a_session_in_the_worktree_of_a_start_from_a_linked_worktree_finds_its_task() {
    let f = Fixture::new("wt-linked-session");
    let linked = f.root.join("linked");
    f.git(&[
        "worktree",
        "add",
        "-q",
        "-b",
        "fixture-linked",
        linked.to_str().unwrap(),
        "HEAD",
    ]);
    let out = start_in(&f, &linked, &wt(&f));
    assert!(out.status.success(), "{out:?}");
    let response: Value = serde_json::from_slice(&out.stdout).unwrap();
    let task = response["task_id"].as_str().unwrap().to_owned();
    let wt = wt(&f);
    for (file, id) in [
        (linked.join("src").join("lib.rs"), "linked-1"),
        (f.project.join("src").join("lib.rs"), "primary-1"),
    ] {
        let decision = write_at(&f, &wt, &file, id);
        assert!(denied(&decision), "{decision}");
        assert!(reason(&decision).contains("base checkout"), "{decision}");
    }
    let file = wt.join("src").join("lib.rs");
    assert!(!denied(&write_at(&f, &wt, &file, "edit-1")));
    fs::write(&file, "// two\n").unwrap();
    let mut payload = json!({"hook_event_name":"PreToolUse","tool_use_id":"bash-1",
        "agent_id":"main","tool_name":"Bash","tool_input":{"command":"cargo test -p fixture"}});
    assert!(!denied(&hook_at(&f, &wt, payload.clone())));
    payload["hook_event_name"] = "PostToolUse".into();
    payload["tool_response"] = json!({"stdout":"ok","stderr":"","interrupted":false});
    hook_at(&f, &wt, payload);
    let stop = stop_at(&f, &wt);
    assert_ne!(stop["decision"], "block", "{stop}");
    assert!(completed(&f, &task));
}

/// An agent's revise cannot move a person's worktree task: neither a path inside the main
/// checkout (which resolves to the checkout itself) nor one that does not exist yet. Before the
/// fix either was accepted and the base-checkout refusal stopped applying.
#[test]
fn an_agent_revise_cannot_move_a_worktree_task() {
    let f = Fixture::new("wt-revise");
    let task = start(&f)["task_id"].as_str().unwrap().to_owned();
    let contract = task_file(&f, &task, "contract.json");
    for (n, requested) in ["src", "zz"].into_iter().enumerate() {
        let intents = || -> Value {
            serde_json::from_str(
                &fs::read_to_string(state_root(&f).unwrap().join("intents.json"))
                    .unwrap_or_else(|_| "{\"events\":{}}".to_owned()),
            )
            .unwrap()
        };
        let before: Vec<String> = intents()["events"]
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect();
        hook_at(
            &f,
            &f.project,
            json!({"hook_event_name":"UserPromptSubmit","prompt_id":format!("p{n}"),
                "prompt":"Implement the change"}),
        );
        let store = intents();
        let (id, event) = store["events"]
            .as_object()
            .unwrap()
            .iter()
            .find(|(id, _)| !before.contains(id))
            .unwrap();
        let proposal = json!({"event_id":id,"nonce":event["nonce"],"prompt_hash":event["prompt_hash"],
            "action":"revise","task_id":task,"contract":contract,"worktree":requested});
        let out = f.run(&["intent", "apply", "--proposal", &proposal.to_string()]);
        assert!(!out.status.success(), "{requested}: {out:?}");
        assert!(
            String::from_utf8_lossy(&out.stderr).contains("isolation worktree"),
            "{requested}: {out:?}"
        );
        // The prompt is classified without a move, so the next edit reaches the task gate.
        let proposal = json!({"event_id":id,"nonce":event["nonce"],"prompt_hash":event["prompt_hash"],
            "action":"continue","task_id":task});
        let out = f.run(&["intent", "apply", "--proposal", &proposal.to_string()]);
        assert!(out.status.success(), "{requested}: {out:?}");
        let base = f.project.join("src").join("lib.rs");
        let decision = write_at(&f, &f.project, &base, &format!("base-{n}"));
        assert!(denied(&decision), "{requested}: {decision}");
        assert!(reason(&decision).contains("base checkout"), "{decision}");
    }
}

#[test]
fn an_agent_task_bound_to_a_worktree_keeps_isolation_none_and_no_base_refusal() {
    let f = Fixture::new("wt-agent");
    // An agent's start needs a discovered check floor.
    fs::write(
        f.project.join("Cargo.toml"),
        "[package]\nname = \"fixture\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    f.commit("manifest");
    let linked = f.root.join("linked");
    f.git(&[
        "worktree",
        "add",
        "-q",
        "-b",
        "fixture-linked",
        linked.to_str().unwrap(),
        "HEAD",
    ]);
    hook_at(
        &f,
        &f.project,
        json!({"hook_event_name":"UserPromptSubmit","prompt_id":"p1","prompt":"Implement a change"}),
    );
    let root = fs::read_dir(&f.state)
        .unwrap()
        .map(|e| e.unwrap().path())
        .find(|p| p.join("intents.json").exists())
        .unwrap();
    let store: Value =
        serde_json::from_str(&fs::read_to_string(root.join("intents.json")).unwrap()).unwrap();
    let (id, event) = store["events"].as_object().unwrap().iter().next().unwrap();
    let proposal = json!({"event_id":id,"nonce":event["nonce"],"prompt_hash":event["prompt_hash"],
        "action":"start","task_id":null,"worktree":linked.to_str().unwrap(),
        "contract":{"id":"g5","goal":"work in a worktree","scope":["src/"],"forbidden_effects":[],
            "max_repair_rounds":2,
            "criteria":[{"id":"C2","check":"test","mandatory":true,"command":"cargo test"}]}});
    let out = f.run(&["intent", "apply", "--proposal", &proposal.to_string()]);
    assert!(out.status.success(), "{out:?}");
    let task = active_task(&f).unwrap();
    assert_eq!(task_file(&f, &task, "contract.json")["isolation"], "none");
    let decision = write_at(&f, &f.project, &f.project.join("src").join("lib.rs"), "e");
    assert!(!reason(&decision).contains("base checkout"), "{decision}");
    assert!(cleanup(&f, &task).status.code() != Some(0));
    assert!(linked.exists());
}

#[test]
fn cleanup_refuses_the_active_task() {
    let f = Fixture::new("wt-clean-active");
    let task = start(&f)["task_id"].as_str().unwrap().to_owned();
    let out = cleanup(&f, &task);
    assert!(!out.status.success(), "{out:?}");
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("task deactivate"),
        "{out:?}"
    );
    assert!(wt(&f).exists() && listed(&f));
}

#[test]
fn cleanup_refuses_a_task_without_a_created_worktree() {
    let f = Fixture::new("wt-clean-plain");
    let task = f.start(json!({}));
    deactivate(&f, &task);
    let out = cleanup(&f, &task);
    assert!(!out.status.success(), "{out:?}");
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("only a worktree the supervisor created"),
        "{out:?}"
    );
}

#[test]
fn cleanup_refuses_uncommitted_or_untracked_work() {
    let f = Fixture::new("wt-clean-dirty");
    let task = start(&f)["task_id"].as_str().unwrap().to_owned();
    deactivate(&f, &task);
    fs::write(wt(&f).join("new.txt"), "work\n").unwrap();
    let out = cleanup(&f, &task);
    assert!(!out.status.success(), "{out:?}");
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("uncommitted or untracked"),
        "{out:?}"
    );
    assert_eq!(
        fs::read_to_string(wt(&f).join("new.txt")).unwrap(),
        "work\n"
    );
    assert!(listed(&f));
}

/// WP3 review r3: `git worktree remove` deletes ignored files, so cleanup refuses while any are
/// there (a file or a folder holding one), names them and touches nothing.
#[test]
fn cleanup_refuses_ignored_files_and_names_them() {
    let f = Fixture::new("wt-clean-ignored");
    fs::write(f.project.join(".gitignore"), ".env\nbuild\n").unwrap();
    f.commit("ignore .env and build");
    let task = start(&f)["task_id"].as_str().unwrap().to_owned();
    deactivate(&f, &task);
    fs::write(wt(&f).join(".env"), "local\n").unwrap();
    fs::create_dir_all(wt(&f).join("build")).unwrap();
    fs::write(wt(&f).join("build").join("out.bin"), "built\n").unwrap();
    let out = cleanup(&f, &task);
    assert!(!out.status.success(), "{out:?}");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("ignored")
            && stderr.contains(".env")
            && stderr.contains("build/out.bin")
            && stderr.contains("nothing was touched"),
        "{stderr}"
    );
    assert_eq!(fs::read_to_string(wt(&f).join(".env")).unwrap(), "local\n");
    assert!(wt(&f).join("build").join("out.bin").is_file());
    assert!(listed(&f));
}

#[test]
fn cleanup_refuses_committed_work_without_a_receipt() {
    let f = Fixture::new("wt-clean-unreceipted");
    let task = start(&f)["task_id"].as_str().unwrap().to_owned();
    fs::write(wt(&f).join("src").join("lib.rs"), "// two\n").unwrap();
    f.git_in(&wt(&f), &["add", "-A"]);
    f.git_in(&wt(&f), &["commit", "-q", "-m", "work"]);
    let commit = f.git_in(&wt(&f), &["rev-parse", "HEAD"]);
    deactivate(&f, &task);
    let out = cleanup(&f, &task);
    assert!(!out.status.success(), "{out:?}");
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("has no receipt"),
        "{out:?}"
    );
    assert!(listed(&f));
    assert_eq!(f.git_in(&wt(&f), &["rev-parse", "HEAD"]), commit);
}

#[test]
fn cleanup_accepts_committed_work_whose_receipt_is_in_head() {
    let f = Fixture::new("wt-clean-receipted");
    let task = complete_in_worktree(&f);
    let (code, value) = f.export();
    assert_eq!(code, 0, "{value}");
    f.git_in(&wt(&f), &["add", "-A"]);
    f.git_in(&wt(&f), &["commit", "-q", "-m", "work and its receipt"]);
    let commit = f.git_in(&wt(&f), &["rev-parse", "HEAD"]);
    let out = f.run(&["task", "deactivate", "--task", &task]);
    assert!(out.status.success(), "{out:?}");
    let out = cleanup(&f, &task);
    assert!(out.status.success(), "{out:?}");
    assert!(!wt(&f).exists());
    assert!(!listed(&f));
    assert_eq!(f.git(&["rev-parse", &format!("supervisor/{task}")]), commit);
}

#[test]
fn cleanup_removes_an_untouched_worktree_and_keeps_the_branch() {
    let f = Fixture::new("wt-clean-untouched");
    let task = start(&f)["task_id"].as_str().unwrap().to_owned();
    deactivate(&f, &task);
    let out = cleanup(&f, &task);
    assert!(out.status.success(), "{out:?}");
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["branch"], format!("supervisor/{task}"), "{v}");
    assert!(!wt(&f).exists());
    assert!(!listed(&f));
    assert!(branches(&f).contains(&format!("supervisor/{task}")));
}

#[test]
fn cleanup_unlinks_a_junction_and_keeps_its_target() {
    let f = Fixture::new("wt-clean-link");
    // No trailing slash: on unix the link is a symlink, which git sees as a file.
    fs::write(f.project.join(".gitignore"), "node_modules\n").unwrap();
    f.commit("ignore node_modules");
    let shared = f.root.join("shared");
    fs::create_dir_all(&shared).unwrap();
    fs::write(shared.join("keep.txt"), "shared\n").unwrap();
    let task = start(&f)["task_id"].as_str().unwrap().to_owned();
    let link = wt(&f).join("node_modules");
    #[cfg(windows)]
    {
        let out = std::process::Command::new("cmd")
            .args(["/c", "mklink", "/J"])
            .arg(&link)
            .arg(&shared)
            .output()
            .unwrap();
        assert!(out.status.success(), "{out:?}");
        // The cleanup relies on std reporting a junction as a link.
        assert!(
            fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink()
        );
    }
    #[cfg(unix)]
    std::os::unix::fs::symlink(&shared, &link).unwrap();
    deactivate(&f, &task);
    let out = cleanup(&f, &task);
    assert!(out.status.success(), "{out:?}");
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["links_unlinked"], 1, "{v}");
    assert!(!wt(&f).exists());
    assert_eq!(
        fs::read_to_string(shared.join("keep.txt")).unwrap(),
        "shared\n"
    );
}

/// A link named `.git` below the worktree's root is unlinked like any other.
#[test]
fn cleanup_unlinks_a_nested_link_named_dot_git() {
    let f = Fixture::new("wt-clean-dotgit");
    fs::write(f.project.join(".gitignore"), "tmp\n").unwrap();
    f.commit("ignore tmp");
    let shared = f.root.join("shared");
    fs::create_dir_all(&shared).unwrap();
    fs::write(shared.join("keep.txt"), "shared\n").unwrap();
    let task = start(&f)["task_id"].as_str().unwrap().to_owned();
    fs::create_dir_all(wt(&f).join("tmp")).unwrap();
    let link = wt(&f).join("tmp").join(".git");
    #[cfg(windows)]
    {
        let out = std::process::Command::new("cmd")
            .args(["/c", "mklink", "/J"])
            .arg(&link)
            .arg(&shared)
            .output()
            .unwrap();
        assert!(out.status.success(), "{out:?}");
    }
    #[cfg(unix)]
    std::os::unix::fs::symlink(&shared, &link).unwrap();
    deactivate(&f, &task);
    let out = cleanup(&f, &task);
    assert_eq!(
        fs::read_to_string(shared.join("keep.txt")).ok().as_deref(),
        Some("shared\n"),
        "{out:?}"
    );
    assert!(out.status.success(), "{out:?}");
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["links_unlinked"], 1, "{v}");
    assert!(!wt(&f).exists());
}

#[test]
fn an_agent_cannot_run_cleanup() {
    let f = Fixture::new("wt-clean-agent");
    f.start(json!({}));
    let decision = f.hook(json!({"hook_event_name":"PreToolUse","tool_use_id":"c",
        "agent_id":"main","tool_name":"Bash",
        "tool_input":{"command":"supervisor task cleanup --task x"}}));
    assert!(denied(&decision), "{decision}");
}

/// WP3 review r3: ending, rebinding or importing a task is a person's command too; an agent that
/// ended a person's worktree task could start its own task without the worktree refusal. The
/// guard names the words it matched; `task resume` changes nothing and stays allowed.
#[test]
fn an_agent_cannot_deactivate_rebind_or_migrate() {
    let f = Fixture::new("wt-guard-agent");
    let task = start(&f)["task_id"].as_str().unwrap().to_owned();
    for (i, word, args) in [
        (0, "deactivate", "--disposition cancelled"),
        (1, "rebind", "--session other"),
        (2, "migrate", "--legacy old"),
        (3, "resume", ""),
    ] {
        let decision = f.hook(json!({"hook_event_name":"PreToolUse","tool_use_id":format!("g{i}"),
            "agent_id":"main","tool_name":"Bash",
            "tool_input":{"command":format!("supervisor task {word} --task {task} {args}")}}));
        let guarded = reason(&decision).contains(&format!("(supervisor task {word})"));
        assert_eq!(guarded, word != "resume", "{word}: {decision}");
        assert_eq!(denied(&decision), guarded, "{word}: {decision}");
    }
}
