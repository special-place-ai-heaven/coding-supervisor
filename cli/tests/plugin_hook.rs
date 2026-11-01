//! The plugin hook's node suite, `node --test hooks/supervisor-hook.test.js`, run by `cargo test`.

use std::io::{ErrorKind, Write};
use std::path::Path;
use std::process::Command;

const SHIM: &str = "the Windows shim's node is never taken from the working directory";

#[test]
fn the_plugin_hook_node_suite_passes() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let suite = root.join("hooks").join("supervisor-hook.test.js");
    assert!(suite.is_file(), "{} is missing", suite.display());
    let out = Command::new("node")
        .args(["--test", "--test-reporter=tap"])
        .arg(&suite)
        .current_dir(root)
        .output();
    let out = match out {
        Ok(out) => out,
        Err(e)
            if e.kind() == ErrorKind::NotFound
                && skip_without_node(std::env::var("RUNNER_ENVIRONMENT").ok().as_deref()) =>
        {
            // Written past the test harness's output capture, so the CI log always shows it.
            writeln!(
                std::io::stderr(),
                "NODE HOOK SUITE NOT RUN: node could not be started on this self-hosted runner: {e}"
            )
            .unwrap();
            return;
        }
        Err(e) => panic!("node could not be started: {e}"),
    };
    let stdout = String::from_utf8_lossy(&out.stdout);
    let log = format!(
        "node exited with {}\n--- stdout\n{stdout}\n--- stderr\n{}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(out.status.success(), "{log}");
    check(&stdout, &log, cfg!(windows));
}

/// Only a self-hosted CI runner may lack node; anywhere else a missing node fails the test.
fn skip_without_node(runner_environment: Option<&str>) -> bool {
    runner_environment == Some("self-hosted")
}

#[test]
fn only_a_self_hosted_runner_may_skip_the_suite_without_node() {
    assert!(skip_without_node(Some("self-hosted")));
    assert!(!skip_without_node(Some("github-hosted")));
    assert!(!skip_without_node(None));
    assert!(!skip_without_node(Some("Self-Hosted")));
}

/// The TAP summary has no failure and at least 10 passes (every OS skips one of the 11 tests: the
/// signal test on Windows, the shim test elsewhere), and on Windows the shim test ran and passed.
fn check(tap: &str, log: &str, windows: bool) {
    let count = |key: &str| -> u32 {
        tap.lines()
            .find_map(|line| line.strip_prefix(&format!("# {key} ")))
            .and_then(|n| n.trim().parse().ok())
            .unwrap_or_else(|| panic!("no `# {key} <n>` line in the TAP summary\n{log}"))
    };
    assert_eq!(count("fail"), 0, "{log}");
    let pass = count("pass");
    assert!(
        pass >= 10,
        "expected at least 10 passing tests, got {pass}\n{log}"
    );
    if windows {
        let shim = tap
            .lines()
            .find(|line| {
                line.starts_with("ok ")
                    && line
                        .split_once(" - ")
                        .is_some_and(|(_, name)| name.starts_with(SHIM))
            })
            .unwrap_or_else(|| panic!("no passing TAP line for `{SHIM}`\n{log}"));
        assert!(
            !shim.contains("# SKIP"),
            "`{SHIM}` was skipped: {shim}\n{log}"
        );
    }
}
