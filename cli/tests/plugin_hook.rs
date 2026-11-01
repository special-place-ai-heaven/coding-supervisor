//! The plugin hook's node suite, `node --test hooks/supervisor-hook.test.js`, run by `cargo test`.

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
        .output()
        .unwrap_or_else(|e| panic!("node could not be started: {e}"));
    let stdout = String::from_utf8_lossy(&out.stdout);
    let log = format!(
        "node exited with {}\n--- stdout\n{stdout}\n--- stderr\n{}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(out.status.success(), "{log}");
    check(&stdout, &log, cfg!(windows));
}

/// The TAP summary has no failure, and the Windows-only shim test ran and passed on Windows.
fn check(tap: &str, log: &str, windows: bool) {
    let count = |key: &str| -> u32 {
        tap.lines()
            .find_map(|line| line.strip_prefix(&format!("# {key} ")))
            .and_then(|n| n.trim().parse().ok())
            .unwrap_or_else(|| panic!("no `# {key} <n>` line in the TAP summary\n{log}"))
    };
    assert_eq!(count("fail"), 0, "{log}");
    let pass = count("pass");
    if windows {
        assert!(
            pass >= 11,
            "expected at least 11 passing tests, got {pass}\n{log}"
        );
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
    } else {
        assert!(
            pass >= 10,
            "expected at least 10 passing tests, got {pass}\n{log}"
        );
    }
}
