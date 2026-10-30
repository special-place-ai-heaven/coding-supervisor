//! Deterministic hook replay; the cwd case also executes harmless local Python checks.
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

use coding_supervisor::{Candidate, CandidateId};
use serde_json::{Value, json};
use supervisor_claude_code::adapter::{Decision, Env, handle};
use supervisor_claude_code::candidate::CandidateProvider;

static NEXT: AtomicU64 = AtomicU64::new(0);
struct Fixture(PathBuf);
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
struct Fixed;
impl CandidateProvider for Fixed {
    fn observe(&self) -> Result<Candidate, String> {
        Ok(Candidate {
            id: CandidateId {
                base_commit: "base".into(),
                dirty_digest: "candidate".into(),
            },
            provenance: "execution-context fixture".into(),
            changed_paths: vec!["src/lib.rs".into()],
        })
    }
}
fn command(program: &str, args: &[&str], cwd: &Path) -> std::process::Output {
    let mut command = Command::new(program);
    command.args(args).current_dir(cwd);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    command.output().unwrap()
}
fn python_cmd() -> &'static str {
    // The fixtures spawn a real interpreter; Windows installs `python`, while
    // Linux images ship `python3` and usually no `python` launcher.
    if cfg!(windows) { "python" } else { "python3" }
}
fn setup(required: &str) -> (Fixture, Env) {
    let fixture = Fixture(std::env::temp_dir().join(format!(
        "supervisor-execution-context-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )));
    let state = fixture.0.join("state");
    fs::create_dir_all(&state).unwrap();
    assert!(
        command("git", &["init", "--quiet"], &fixture.0)
            .status
            .success()
    );
    fs::write(
        state.join("contract.json"),
        json!({"id":"context", "goal":"check the root candidate", "scope":["src/"],
            "forbidden_effects":[], "max_repair_rounds":2,
            "criteria":[{"id":"C2","check":"test","mandatory":true,"command":required}]})
        .to_string(),
    )
    .unwrap();
    let env = Env {
        state_dir: state,
        state_location: "test",
        candidate: Box::new(Fixed),
        clock: || 100,
    };
    (fixture, env)
}
fn payload(env: &Env, route: &str, id: &str) -> Value {
    let root = env.state_dir.parent().unwrap();
    let input = if route == "shell_exec" {
        json!({"shell_line":"cargo test", "cwd":root})
    } else {
        json!({"argv":["cargo","test"], "cwd":root})
    };
    json!({"hook_event_name":"PreToolUse", "cwd":root, "session_id":"session",
        "agent_id":"main", "tool_use_id":id,
        "tool_name":format!("mcp__terminal_commander__{route}"), "tool_input":input})
}
fn finish(env: &mut Env, mut payload: Value, status: &str, exit: Option<i32>) {
    handle(&payload, env);
    payload["hook_event_name"] = "PostToolUse".into();
    payload["tool_response"] = json!({"job_id":format!("job-{}",payload["tool_use_id"]),
        "bucket_id":"bucket", "state":status, "exit_code":exit,
        "complete":status == "exited", "outcome_trust":"observed",
        "degraded":false, "pipeline_exit_masked":false});
    handle(&payload, env);
}
fn stop(env: &mut Env) -> Decision {
    handle(
        &json!({"hook_event_name":"Stop", "cwd":env.state_dir.parent().unwrap(),
            "session_id":"session", "agent_id":"main", "last_assistant_message":"Done."}),
        env,
    )
}
fn refused_override(route: &str) {
    let (_fixture, mut env) = setup("cargo test");
    let mut event = payload(&env, route, "modified");
    event["tool_input"]["env"] = json!([{"key":"NODE_OPTIONS", "value":"--no-warnings"}]);
    finish(&mut env, event, "exited", Some(0));
    assert!(matches!(stop(&mut env), Decision::Block(_)));
}

#[test]
fn argv_run_and_watch_environment_requires_approval() {
    refused_override("run_and_watch");
}
#[test]
fn argv_command_start_environment_requires_approval() {
    refused_override("command_start_combed");
}
#[test]
fn shell_execution_environment_requires_approval() {
    refused_override("shell_exec");
}
#[test]
fn explicit_shell_override_requires_approval() {
    let (_fixture, mut env) = setup("cargo test");
    let mut event = payload(&env, "shell_exec", "shell");
    event["tool_input"]["shell"] = "fixture-shell".into();
    finish(&mut env, event, "exited", Some(0));
    assert!(matches!(stop(&mut env), Decision::Block(_)));
}
#[test]
fn latest_unapproved_context_cannot_reuse_prior_success() {
    let (_fixture, mut env) = setup("cargo test");
    let event = payload(&env, "run_and_watch", "good");
    finish(&mut env, event, "exited", Some(0));
    let mut event = payload(&env, "run_and_watch", "modified");
    event["tool_input"]["env"] = json!([{"key":"NODE_OPTIONS", "value":"--no-warnings"}]);
    finish(&mut env, event, "exited", Some(0));
    assert!(matches!(stop(&mut env), Decision::Block(_)));
}
#[test]
fn changed_post_dispatch_environment_does_not_link() {
    let (_fixture, mut env) = setup("cargo test");
    let mut event = payload(&env, "run_and_watch", "changed");
    handle(&event, &mut env);
    event["tool_input"]["env"] = json!([{"key":"NODE_OPTIONS", "value":"--no-warnings"}]);
    event["hook_event_name"] = "PostToolUse".into();
    event["tool_response"] = json!({"job_id":"job", "bucket_id":"bucket",
        "state":"exited", "exit_code":0, "outcome_trust":"observed", "degraded":false,
        "pipeline_exit_masked":false});
    handle(&event, &mut env);
    assert!(matches!(stop(&mut env), Decision::Block(_)));
}
#[test]
fn legacy_completed_receipts_need_new_context_provenance() {
    let (_fixture, mut env) = setup("cargo test");
    let event = payload(&env, "run_and_watch", "legacy");
    finish(&mut env, event, "exited", Some(0));
    assert_eq!(stop(&mut env), Decision::None);
    let path = env.state_dir.join("runs.jsonl");
    let rows = fs::read_to_string(&path)
        .unwrap()
        .lines()
        .map(|line| {
            let mut row: Value = serde_json::from_str(line).unwrap();
            row["identity_schema"] = 2.into();
            row.as_object_mut().unwrap().remove("execution_profile");
            row.as_object_mut().unwrap().remove("execution_root");
            row.to_string()
        })
        .collect::<Vec<_>>()
        .join("\n");
    fs::write(&path, format!("{rows}\n")).unwrap();
    let refusal = stop(&mut env);
    assert!(
        matches!(&refusal, Decision::Block(reason) if reason.contains("execution context")),
        "{refusal:?}"
    );
}
#[test]
fn failed_and_unknown_results_never_authorize_completion() {
    for (state, exit) in [("exited", Some(9)), ("running", None), ("cancelled", None)] {
        let (_fixture, mut env) = setup("cargo test");
        let event = payload(&env, "run_and_watch", "result");
        finish(&mut env, event, state, exit);
        assert!(matches!(stop(&mut env), Decision::Block(_)));
    }
}
#[test]
fn harmless_root_and_nested_checks_keep_distinct_execution_context() {
    let (fixture, mut env) = setup("python check.py");
    let nested = fixture.0.join("nested");
    fs::create_dir_all(&nested).unwrap();
    for root in [&fixture.0, &nested] {
        fs::write(
            root.join("check.py"),
            "from pathlib import Path\nPath('executed-cwd.txt').write_text(str(Path.cwd()))\n",
        )
        .unwrap();
        let actual = command(python_cmd(), &["check.py"], root);
        assert!(actual.status.success());
        let marker = fs::read_to_string(root.join("executed-cwd.txt")).unwrap();
        assert_eq!(
            PathBuf::from(marker).canonicalize().unwrap(),
            root.canonicalize().unwrap()
        );
        let mut event = payload(
            &env,
            "run_and_watch",
            if root == &nested { "nested" } else { "root" },
        );
        event["tool_input"]["argv"] = json!(["python", "check.py"]);
        event["tool_input"]["cwd"] = json!(root);
        // Deliver only the nested success first: matching text is insufficient.
        if root == &nested {
            finish(&mut env, event, "exited", actual.status.code());
            let rows = fs::read_to_string(env.state_dir.join("runs.jsonl")).unwrap();
            let completed: Value = rows
                .lines()
                .map(|l| serde_json::from_str::<Value>(l).unwrap())
                .find(|v| v["kind"] == "completed")
                .unwrap();
            assert_eq!(
                completed["cwd"],
                format!(
                    "{}/nested",
                    supervisor_claude_code::location::canonical_project(&fixture.0)
                )
            );
            assert!(matches!(stop(&mut env), Decision::Block(_)));
        }
    }
    let mut root_event = payload(&env, "run_and_watch", "root");
    root_event["tool_input"]["argv"] = json!(["python", "check.py"]);
    finish(&mut env, root_event, "exited", Some(0));
    assert_eq!(stop(&mut env), Decision::None);
}

// Child processes avoid process-global environment mutations racing other tests.
#[test]
fn cargo_target_profile_child() {
    let Ok(case) = std::env::var("SUPERVISOR_CONTEXT_CHILD") else {
        return;
    };
    let (_fixture, mut env) = setup("cargo test");
    let mut event = payload(&env, "run_and_watch", "target");
    let value = if case == "mismatch" {
        "unapproved-target"
    } else {
        "approved-target"
    };
    if case == "changed" {
        handle(&event, &mut env);
        event["hook_event_name"] = "PostToolUse".into();
        event["tool_input"]["env"] = json!([{"key":"CARGO_TARGET_DIR","value":value}]);
        event["tool_response"] = json!({"job_id":"job", "state":"exited", "exit_code":0,
            "outcome_trust":"observed", "degraded":false, "pipeline_exit_masked":false});
        handle(&event, &mut env);
    } else {
        event["tool_input"]["env"] = json!([{"key":"CARGO_TARGET_DIR","value":value}]);
        finish(&mut env, event, "exited", Some(0));
    }
    if case == "match" {
        assert_eq!(stop(&mut env), Decision::None);
    } else {
        assert!(matches!(stop(&mut env), Decision::Block(_)));
    }
    let rows = fs::read_to_string(env.state_dir.join("runs.jsonl")).unwrap_or_default();
    assert!(!rows.contains("approved-target"));
    assert!(!rows.contains("unapproved-target"));
}
#[test]
fn cargo_target_override_requires_exact_inherited_approval() {
    for case in ["match", "mismatch", "missing", "changed"] {
        let mut child = Command::new(std::env::current_exe().unwrap());
        child
            .args(["--exact", "cargo_target_profile_child", "--nocapture"])
            .env("SUPERVISOR_CONTEXT_CHILD", case);
        if case == "missing" {
            child.env_remove("CARGO_TARGET_DIR");
        } else {
            child.env("CARGO_TARGET_DIR", "approved-target");
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            child.creation_flags(0x08000000);
        }
        let output = child.output().unwrap();
        assert!(
            output.status.success(),
            "target profile case {case}: {}",
            String::from_utf8_lossy(&output.stdout)
        );
    }
}

#[test]
fn cargo_target_dispatch_child() {
    let Ok(root) = std::env::var("SUPERVISOR_CONTEXT_DISPATCH_ROOT") else {
        return;
    };
    let mut env = Env {
        state_dir: PathBuf::from(root).join("state"),
        state_location: "test",
        candidate: Box::new(Fixed),
        clock: || 100,
    };
    let mut event = payload(&env, "run_and_watch", "trusted-target");
    event["tool_input"]["env"] = json!([{"key":"CARGO_TARGET_DIR",
        "value":std::env::var("CARGO_TARGET_DIR").unwrap()}]);
    if std::env::var("SUPERVISOR_CONTEXT_PHASE").unwrap() == "post" {
        event["hook_event_name"] = "PostToolUse".into();
        event["tool_response"] = json!({"job_id":"job", "state":"exited", "exit_code":0,
            "outcome_trust":"observed", "degraded":false, "pipeline_exit_masked":false});
    }
    handle(&event, &mut env);
}
#[test]
fn approved_target_identity_is_bound_across_hook_processes() {
    let (fixture, mut env) = setup("cargo test");
    for (phase, target) in [
        ("pre", "trusted-first-target"),
        ("post", "trusted-other-target"),
    ] {
        let mut child = Command::new(std::env::current_exe().unwrap());
        child
            .args(["--exact", "cargo_target_dispatch_child", "--nocapture"])
            .env("SUPERVISOR_CONTEXT_DISPATCH_ROOT", &fixture.0)
            .env("SUPERVISOR_CONTEXT_PHASE", phase)
            .env("CARGO_TARGET_DIR", target);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            child.creation_flags(0x08000000);
        }
        let output = child.output().unwrap();
        assert!(output.status.success());
    }
    assert!(matches!(stop(&mut env), Decision::Block(_)));
}
