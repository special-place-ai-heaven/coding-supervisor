//! A protected-branch check whose git does not answer refuses within the git deadline.
//!
//! Unix only: the stub `git` is a `#!/bin/sh` script, which Windows cannot run from PATH; the
//! timeout path itself is platform-independent and `cli/tests/hang.rs` drives it on Windows.
#![cfg(unix)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::time::{Duration, Instant};

use serde_json::json;
use supervisor_claude_code::adapter::Decision;
use supervisor_claude_code::policy::action_policy_denial;

#[test]
fn a_reset_whose_branch_git_hangs_is_refused_within_the_bound() {
    let root = std::env::temp_dir().join(format!(
        "supervisor-policy-git-timeout-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    let stub = root.join("stub");
    fs::create_dir_all(&stub).unwrap();
    let git = stub.join("git");
    fs::write(&git, "#!/bin/sh\nexec sleep 30\n").unwrap();
    fs::set_permissions(&git, fs::Permissions::from_mode(0o755)).unwrap();
    fs::write(
        root.join("policy.json"),
        json!({"version":1,"protected_branches":["main"]}).to_string(),
    )
    .unwrap();
    let path = std::env::var_os("PATH").unwrap_or_default();
    let path =
        std::env::join_paths(std::iter::once(stub).chain(std::env::split_paths(&path))).unwrap();
    // The only test in this binary, so no other thread reads the environment meanwhile.
    unsafe { std::env::set_var("PATH", &path) };
    let started = Instant::now();
    let decision = action_policy_denial(
        &json!({"tool_name":"Bash","tool_input":{"command":"git reset --hard HEAD"}}),
        &root,
        &root,
    );
    let elapsed = started.elapsed();
    let _ = fs::remove_dir_all(&root);
    assert_eq!(
        decision,
        Ok(Some(Decision::Deny(
            "GATE: action refused: protected-branch context unavailable".into()
        )))
    );
    assert!(elapsed < Duration::from_secs(15), "{elapsed:?}");
}
