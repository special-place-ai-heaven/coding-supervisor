use std::fs;
use std::path::PathBuf;

use coding_supervisor::{Candidate, CandidateId};
use serde_json::{Value, json};
use supervisor_claude_code::adapter::Decision;
use supervisor_claude_code::guard::state_write_denial;
use supervisor_claude_code::policy::action_policy_denial;

#[cfg(windows)]
use std::os::windows::process::CommandExt;

struct Fixture(PathBuf);
impl Fixture {
    fn new(name: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("supervisor-policy-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(path.join("protected")).unwrap();
        Self(path)
    }
    fn policy(&self, policy: Value) {
        fs::write(self.0.join("policy.json"), policy.to_string()).unwrap();
    }
    fn call(&self, tool: &str, input: Value) -> Result<Option<Decision>, String> {
        action_policy_denial(
            &json!({"tool_name":tool,"tool_input":input}),
            &self.0,
            &self.0,
        )
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn no_policy_and_routine_calls_are_silent() {
    let f = Fixture::new("silent");
    assert_eq!(
        f.call("Bash", json!({"command":"cargo test"})).unwrap(),
        None
    );
    f.policy(json!({"version":1,"protected_branches":["main"]}));
    for (tool, input) in [
        ("Write", json!({"file_path":"src/lib.rs"})),
        ("PowerShell", json!({"command":"cargo test"})),
        (
            "mcp__terminal_commander__run_and_watch",
            json!({"argv":["cargo","test"]}),
        ),
    ] {
        assert_eq!(f.call(tool, input).unwrap(), None);
    }
}

#[test]
fn configured_path_deletion_covers_simple_shell_argv_shell_line_and_file_route() {
    let f = Fixture::new("paths");
    let protected = f.0.join("protected");
    f.policy(json!({"version":1,"protected_paths":[protected]}));
    for (tool, input) in [
        ("Bash", json!({"command":"rm -rf protected"})),
        (
            "PowerShell",
            json!({"command":"Remove-Item -Recurse protected"}),
        ),
        (
            "mcp__terminal_commander__command_start_combed",
            json!({"argv":["rm","-rf",protected]}),
        ),
        (
            "mcp__terminal_commander__shell_exec",
            json!({"shell_line":"rm -rf protected"}),
        ),
        (
            "mcp__terminal_commander__file_delete",
            json!({"path":protected}),
        ),
        ("Bash", json!({"command":"rm -rf ."})),
    ] {
        assert!(
            matches!(f.call(tool, input).unwrap(), Some(Decision::Deny(_))),
            "{tool}"
        );
    }
    assert_eq!(
        f.call("Bash", json!({"command":"rm -rf unrelated"}))
            .unwrap(),
        None
    );
}

#[test]
fn tc_command_routes_handle_real_fields_and_unknown_cwd_conservatively() {
    let f = Fixture::new("session-policy");
    let protected = f.0.join("protected");
    let unrelated = f.0.join("unrelated");
    f.policy(json!({
        "version": 1,
        "protected_paths": [protected],
        "protected_branches": ["main"],
        "synthetic_secret_markers": ["SESSION_FIXTURE_MARKER"],
        "denied_argv_prefixes": [["npm", "publish"]],
    }));

    for tool in [
        "mcp__terminal_commander__run_and_watch",
        "mcp__terminal_commander__command_start_combed",
        "mcp__terminal_commander__pty_command_start",
    ] {
        let relative = json!({"argv":["rm","-rf","unrelated"]});
        assert!(
            matches!(f.call(tool, relative).unwrap(), Some(Decision::Deny(_))),
            "{tool} relative target with omitted cwd"
        );
        let absolute = unrelated.to_string_lossy().to_string();
        let outside = json!({"argv":["rm","-rf",absolute]});
        assert_eq!(
            f.call(tool, outside).unwrap(),
            None,
            "{tool} absolute unrelated"
        );
        assert_eq!(
            f.call(tool, json!({"argv":["cargo","test"]})).unwrap(),
            None,
            "{tool} routine command"
        );
        assert!(
            matches!(
                f.call(tool, json!({"argv":["git","reset","--hard","HEAD"]}))
                    .unwrap(),
                Some(Decision::Deny(_))
            ),
            "{tool} current branch with omitted cwd"
        );
    }

    assert!(matches!(
        f.call(
            "mcp__terminal_commander__shell_exec",
            json!({"shell_line":"rm -rf unrelated"}),
        )
        .unwrap(),
        Some(Decision::Deny(_))
    ));
    let unrelated_absolute = unrelated.to_string_lossy();
    assert_eq!(
        f.call(
            "mcp__terminal_commander__shell_exec",
            json!({"shell_line":format!("rm -rf \"{unrelated_absolute}\"")}),
        )
        .unwrap(),
        None
    );
    assert_eq!(
        f.call(
            "mcp__terminal_commander__shell_exec",
            json!({"shell_line":"cargo test"}),
        )
        .unwrap(),
        None
    );
    assert!(matches!(
        f.call(
            "mcp__terminal_commander__shell_exec",
            json!({"shell_line":"git reset --hard HEAD"}),
        )
        .unwrap(),
        Some(Decision::Deny(_))
    ));

    let session = "mcp__terminal_commander__shell_session_exec";
    let call_session = |line: &str| {
        f.call(session, json!({"session_id":"ses_fixture","line":line}))
            .unwrap()
    };
    assert!(matches!(
        call_session("npm publish"),
        Some(Decision::Deny(_))
    ));
    assert!(matches!(
        call_session("rm -rf unrelated"),
        Some(Decision::Deny(_))
    ));
    assert!(matches!(
        call_session("rm -rf protected"),
        Some(Decision::Deny(_))
    ));
    assert!(matches!(
        call_session("git reset --hard HEAD"),
        Some(Decision::Deny(_))
    ));
    assert!(
        f.call(
            session,
            json!({"session_id":"ses_fixture","cwd":f.0,"line":"rm -rf unrelated"}),
        )
        .unwrap()
        .is_some(),
        "unrecognized cwd must not override sticky session context"
    );
    assert_eq!(call_session("cargo test"), None);
    let marker = call_session("echo SESSION_FIXTURE_MARKER");
    match marker {
        Some(Decision::Deny(reason)) => assert!(!reason.contains("SESSION_FIXTURE_MARKER")),
        other => panic!("expected sanitized marker denial: {other:?}"),
    }

    let explicit_cwd = f.0.to_string_lossy();
    assert!(matches!(
        f.call(
            "mcp__terminal_commander__run_and_watch",
            json!({"cwd":explicit_cwd,"argv":["rm","-rf","protected"]}),
        )
        .unwrap(),
        Some(Decision::Deny(_))
    ));

    let branch_repo = f.0.join("branch-repo");
    fs::create_dir_all(&branch_repo).unwrap();
    let mut git = std::process::Command::new("git");
    #[cfg(windows)]
    {
        git.creation_flags(0x08000000);
    }
    assert!(
        git.args(["init", "--quiet", "--initial-branch=main"])
            .arg(&branch_repo)
            .status()
            .unwrap()
            .success()
    );
    let branch_cwd = branch_repo.to_string_lossy();
    assert!(matches!(
        f.call(
            "mcp__terminal_commander__shell_exec",
            json!({"cwd":branch_cwd,"shell_line":"git reset --hard HEAD"}),
        )
        .unwrap(),
        Some(Decision::Deny(_))
    ));
}

#[test]
fn tc_command_fields_follow_advertised_tool_schema() {
    let f = Fixture::new("tc-fields");
    f.policy(json!({
        "version": 1,
        "denied_argv_prefixes": [["npm", "publish"]],
    }));

    assert!(matches!(
        f.call(
            "mcp__terminal_commander__shell_session_exec",
            json!({
                "session_id":"ses_fixture",
                "line":"npm publish",
                "argv":["cargo","test"],
                "command":"cargo test"
            }),
        )
        .unwrap(),
        Some(Decision::Deny(_))
    ));
    assert_eq!(
        f.call(
            "mcp__terminal_commander__shell_session_exec",
            json!({
                "session_id":"ses_fixture",
                "line":"cargo test",
                "argv":["npm","publish"]
            }),
        )
        .unwrap(),
        None
    );
    assert!(matches!(
        f.call(
            "mcp__terminal_commander__shell_exec",
            json!({
                "shell_line":"npm publish",
                "argv":["cargo","test"],
                "command":"cargo test"
            }),
        )
        .unwrap(),
        Some(Decision::Deny(_))
    ));
    assert_eq!(
        f.call(
            "mcp__terminal_commander__shell_exec",
            json!({
                "shell_line":"cargo test",
                "argv":["npm","publish"],
                "command":"npm publish"
            }),
        )
        .unwrap(),
        None
    );
    assert_eq!(
        f.call(
            "mcp__terminal_commander__unrecognized",
            json!({"argv":["npm","publish"]}),
        )
        .unwrap(),
        None
    );
}

#[test]
fn protected_branch_explicit_force_and_configured_external_routes_are_denied() {
    let f = Fixture::new("branch");
    f.policy(json!({"version":1,"protected_branches":["main"],"denied_tools":["mcp__fixture__publish"],"denied_argv_prefixes":[["npm","publish"]]}));
    for (tool, input) in [
        ("Bash", json!({"command":"git push --force origin main"})),
        ("PowerShell", json!({"command":"git branch -D main"})),
        (
            "mcp__terminal_commander__run_and_watch",
            json!({"argv":["git","push","--force-with-lease","origin","HEAD:main"]}),
        ),
        ("mcp__fixture__publish", json!({"destination":"synthetic"})),
        (
            "mcp__terminal_commander__shell_exec",
            json!({"shell_line":"npm publish"}),
        ),
    ] {
        assert!(
            matches!(f.call(tool, input).unwrap(), Some(Decision::Deny(_))),
            "{tool}"
        );
    }
    assert_eq!(
        f.call("Bash", json!({"command":"git push origin topic"}))
            .unwrap(),
        None
    );
}

#[test]
fn synthetic_marker_denials_never_echo_input() {
    let f = Fixture::new("synthetic");
    let marker = "SYNTHETIC_FIXTURE_MARKER_ONLY";
    f.policy(json!({"version":1,"synthetic_secret_markers":[marker]}));
    for tool in [
        "Bash",
        "PowerShell",
        "Write",
        "mcp__terminal_commander__file_write",
        "mcp__fixture__publish",
    ] {
        match f.call(tool, json!({"content":marker})).unwrap() {
            Some(Decision::Deny(reason)) => assert!(!reason.contains(marker)),
            other => panic!("expected sanitized denial: {other:?}"),
        }
    }
}

#[test]
fn malformed_policy_errors_are_sanitized_and_not_allowances() {
    let f = Fixture::new("invalid");
    for text in [
        "private-fixture-content",
        r#"{"version":1,"unknown":"private-fixture-content"}"#,
        r#"{"version":1,"denied_argv_prefixes":[[]]}"#,
    ] {
        fs::write(f.0.join("policy.json"), text).unwrap();
        let error = f.call("Bash", json!({"command":"cargo test"})).unwrap_err();
        assert!(!error.contains("private-fixture-content"));
    }
}

#[test]
fn unsupported_shell_and_unknown_connector_are_explicit_bypass_fixtures() {
    let f = Fixture::new("gaps");
    f.policy(json!({"version":1,"protected_paths":[f.0.join("protected")],"denied_argv_prefixes":[["npm","publish"]]}));
    assert_eq!(
        f.call("Bash", json!({"command":"echo harmless; rm -rf protected"}))
            .unwrap(),
        None
    );
    assert_eq!(
        f.call("mcp__other__delete", json!({"path":f.0.join("protected")}))
            .unwrap(),
        None
    );
    assert_eq!(
        f.call(
            "mcp__terminal_commander__run_and_watch",
            json!({"argv":["sh","-c","rm -rf protected"]})
        )
        .unwrap(),
        None
    );
}

#[test]
fn policy_configuration_itself_is_protected_on_supported_routes() {
    let f = Fixture::new("self-protection");
    let path = f.0.join("policy.json");
    for (tool, input) in [
        ("Write", json!({"file_path":path})),
        ("Edit", json!({"file_path":path})),
        (
            "mcp__terminal_commander__file_write",
            json!({"path":path,"content":"{}"}),
        ),
        (
            "mcp__terminal_commander__shell_exec",
            json!({"shell_line":format!("rm {}",path.display())}),
        ),
    ] {
        assert!(matches!(
            state_write_denial(&json!({"tool_name":tool,"tool_input":input}), &f.0),
            Some(Decision::Deny(_))
        ));
    }
}

#[test]
fn hook_denies_invalid_policy_while_inactive_without_echoing_it() {
    let f = Fixture::new("hook-invalid");
    fs::write(f.0.join("policy.json"), "SYNTHETIC_PRIVATE_CONFIG").unwrap();
    let mut env = supervisor_claude_code::adapter::Env {
        state_dir: f.0.clone(),
        state_location: "test",
        candidate: Box::new(supervisor_claude_code::candidate::GitCandidate { cwd: f.0.clone() }),
        clock: || 1,
    };
    let decision = supervisor_claude_code::adapter::handle(
        &json!({"hook_event_name":"PreToolUse","cwd":f.0,"tool_name":"Bash","tool_input":{"command":"cargo test"}}),
        &mut env,
    );
    assert_eq!(
        decision,
        Decision::Deny("GATE: invalid configured action policy".to_owned())
    );
}

struct FixedCandidate;
impl supervisor_claude_code::candidate::CandidateProvider for FixedCandidate {
    fn observe(&self) -> Result<Candidate, String> {
        Ok(Candidate {
            id: CandidateId {
                base_commit: "fixture".to_owned(),
                dirty_digest: "fixture".to_owned(),
            },
            provenance: "synthetic fixture".to_owned(),
            changed_paths: Vec::new(),
        })
    }
}

#[test]
fn nonheader_completion_refuses_until_adapter_cap_without_marking_completed() {
    let f = Fixture::new("cap");
    let started=supervisor_claude_code::lifecycle::start(&f.0,&json!({"id":"fixture","goal":"fixture","criteria":[{"id":"C2","check":"candidate_passes_reported_test","mandatory":true,"command":"fixture-check"}],"scope":[],"forbidden_effects":[],"max_repair_rounds":2}),false,None,None).unwrap();
    let mut env = supervisor_claude_code::adapter::Env {
        state_dir: f.0.clone(),
        state_location: "test",
        candidate: Box::new(FixedCandidate),
        clock: || 1,
    };
    let payload = json!({"hook_event_name":"Stop","cwd":f.0,"task_id":started["task_id"],"last_assistant_message":"Done, the task is complete.","stop_hook_active":true});
    for _ in 0..3 {
        assert!(matches!(
            supervisor_claude_code::adapter::handle(&payload, &mut env),
            Decision::Block(_)
        ));
    }
    assert!(
        matches!(supervisor_claude_code::adapter::handle(&payload,&mut env),Decision::System(message) if message.contains("cap reached"))
    );
    let task = supervisor_claude_code::lifecycle::active_task(&f.0)
        .unwrap()
        .unwrap();
    let (_, digest) = supervisor_claude_code::state::read_contract(&task.dir).unwrap();
    match supervisor_claude_code::state::load_gate(&task.dir, &digest).unwrap() {
        supervisor_claude_code::state::Stored::Valid(gate) => assert_ne!(
            supervisor_claude_code::state::state_name(gate.checkpoint.state),
            "completed"
        ),
        _ => panic!("missing valid checkpoint"),
    }
}

#[test]
fn hook_input_failure_is_advisory_not_a_policy_denial() {
    let output = supervisor_claude_code::adapter::run_hook("{}").unwrap();
    let output: Value = serde_json::from_str(&output).unwrap();
    assert!(output.get("systemMessage").is_some());
    assert!(output.get("hookSpecificOutput").is_none());
    assert!(output.get("decision").is_none());
}

#[test]
fn configured_current_branch_rejects_reset_and_rebase_without_running_them() {
    let f = Fixture::new("current-branch");
    let mut git = std::process::Command::new("git");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        git.creation_flags(0x08000000);
    }
    assert!(
        git.args(["init", "--quiet", "--initial-branch=main"])
            .arg(&f.0)
            .status()
            .unwrap()
            .success()
    );
    f.policy(json!({"version":1,"protected_branches":["main"]}));
    for command in ["git reset --hard HEAD", "git rebase topic"] {
        assert!(matches!(
            f.call("Bash", json!({"command":command})).unwrap(),
            Some(Decision::Deny(_))
        ));
    }
    assert_eq!(
        f.call("Bash", json!({"command":"git status"})).unwrap(),
        None
    );
    assert_eq!(
        f.call("Bash", json!({"command":"git add -f main"}))
            .unwrap(),
        None
    );
}

// Invoked in a subprocess by the deadline fixture below; otherwise a no-op.
#[test]
fn synthetic_stalled_hook_fixture() {
    let Some(ready) = std::env::var_os("SUPERVISOR_SYNTHETIC_HOOK_READY") else {
        return;
    };
    fs::write(ready, "ready").unwrap();
    loop {
        std::thread::park();
    }
}

#[test]
fn timed_out_hook_process_produces_no_enforcement_decision() {
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};
    let f = Fixture::new("hook-timeout");
    let ready = f.0.join("ready");
    let mut command = Command::new(std::env::current_exe().unwrap());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let mut child = command
        .args(["--exact", "synthetic_stalled_hook_fixture", "--nocapture"])
        .env("SUPERVISOR_SYNTHETIC_HOOK_READY", &ready)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    while !ready.exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(5));
    }
    let reached_hook = ready.exists();
    // Model a runner deadline, then always reap the child before asserting.
    std::thread::sleep(Duration::from_millis(25));
    child.kill().unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(
        reached_hook,
        "synthetic hook did not start before bounded deadline"
    );
    assert!(!output.status.success());
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        if let Ok(value) = serde_json::from_str::<Value>(line) {
            assert!(value.get("hookSpecificOutput").is_none());
            assert!(value.get("decision").is_none());
        }
    }
}
