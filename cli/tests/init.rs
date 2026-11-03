//! `supervisor init` with the state directory redirected by `SUPERVISOR_STATE_DIR`.

use std::fs;
use std::path::Path;
use std::process::Command;

use serde_json::Value;

const BIN: &str = env!("CARGO_BIN_EXE_supervisor");

fn init(project: &Path, state: &Path) -> String {
    let out = Command::new(BIN)
        .args(["init", "--project"])
        .arg(project)
        .env("SUPERVISOR_STATE_DIR", state)
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(out.status.success(), "{stdout}");
    stdout
}

#[test]
fn init_writes_the_starting_contract_and_never_overwrites_it() {
    let root = std::env::temp_dir().join(format!("sup-cli-init-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    let (project, state) = (root.join("proj"), root.join("state"));
    fs::create_dir_all(&project).unwrap();

    let out = init(&project, &state);
    assert!(out.contains("(rule: env)"), "{out}");
    assert!(out.contains("contract written"), "{out}");
    for harness in ["claude-code", "codex", "cursor", "omp", "grok"] {
        assert!(
            out.contains(&format!("supervisor install --harness {harness} ")),
            "{out}"
        );
    }
    let path = project.join("supervisor-contract.json");
    let state_dirs: Vec<_> = fs::read_dir(&state)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    assert_eq!(state_dirs.len(), 1);
    assert!(!state_dirs[0].join("active.json").exists());
    assert!(!state_dirs[0].join("contract.json").exists());
    let v: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    let criteria = v["criteria"].as_array().unwrap();
    let ids: Vec<&str> = criteria.iter().map(|c| c["id"].as_str().unwrap()).collect();
    assert_eq!(ids, ["C1", "C2", "C3", "C4"]);
    let mandatory: Vec<bool> = criteria
        .iter()
        .map(|c| c["mandatory"].as_bool().unwrap())
        .collect();
    assert_eq!(mandatory, [false, true, true, false]);
    let commands: Vec<&str> = criteria
        .iter()
        .map(|c| c["command"].as_str().unwrap())
        .collect();
    assert_eq!(
        commands,
        [
            "cargo test",
            "cargo test",
            "cargo test --workspace",
            "git diff --name-only"
        ]
    );
    assert_eq!(v["scope"], serde_json::json!(["src/", "tests/"]));
    assert_eq!(v["forbidden_effects"], serde_json::json!([]));
    assert_eq!(v["max_repair_rounds"], 2);
    assert!(!project.join(".supervisor").exists(), "one location only");

    fs::write(&path, "edited by hand").unwrap();
    let again = init(&project, &state);
    assert!(again.contains("contract exists, left unchanged"), "{again}");
    assert_eq!(fs::read_to_string(&path).unwrap(), "edited by hand");
}

#[test]
fn help_prints_the_usage_to_stdout_and_exits_zero() {
    for flag in ["--help", "-h"] {
        let out = Command::new(BIN).arg(flag).output().unwrap();
        assert!(out.status.success(), "{flag}: {:?}", out.status);
        assert!(String::from_utf8_lossy(&out.stdout).starts_with("usage: supervisor mcp"));
    }
    // Anything unknown is still a usage error.
    assert_eq!(
        Command::new(BIN)
            .arg("--nope")
            .output()
            .unwrap()
            .status
            .code(),
        Some(2)
    );
}

/// K4: the draft carries the checks the gate would discover, not a Rust draft in an npm repo.
#[test]
fn init_drafts_the_discovered_checks() {
    let root = std::env::temp_dir().join(format!("sup-cli-init-npm-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    let (project, state) = (root.join("proj"), root.join("state"));
    fs::create_dir_all(&project).unwrap();
    fs::write(
        project.join("package.json"),
        r#"{"scripts":{"test":"jest","lint":"eslint ."}}"#,
    )
    .unwrap();
    init(&project, &state);
    let v: Value = serde_json::from_str(
        &fs::read_to_string(project.join("supervisor-contract.json")).unwrap(),
    )
    .unwrap();
    let commands: Vec<&str> = v["criteria"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["command"].as_str().unwrap())
        .collect();
    assert_eq!(commands, ["npm run test", "npm run lint"]);
    let _ = fs::remove_dir_all(&root);
}

/// X1: `--version` names the binary and its version; K11: help names the MCP tools and the order
/// of a supervised task.
#[test]
fn version_and_help_say_what_runs() {
    let out = Command::new(BIN).arg("--version").output().unwrap();
    assert!(out.status.success(), "{:?}", out.status);
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).trim(),
        format!("supervisor {}", env!("CARGO_PKG_VERSION"))
    );
    let help = Command::new(BIN).arg("--help").output().unwrap();
    let help = String::from_utf8_lossy(&help.stdout);
    for part in ["intent_apply", "task_state", "receipt"] {
        assert!(help.contains(part), "{part} missing from help");
    }
}

/// X3: with no active task, stderr leads with that answer, not with a git error.
#[test]
fn verdict_without_a_task_leads_with_no_active_task() {
    let root = std::env::temp_dir().join(format!("sup-cli-verdict-nogit-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    let (project, state) = (root.join("proj"), root.join("state"));
    fs::create_dir_all(&project).unwrap();
    let out = Command::new(BIN)
        .args(["verdict", "--json", "--project"])
        .arg(&project)
        .env("SUPERVISOR_STATE_DIR", &state)
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("no_active_task"), "{stdout}");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.starts_with("verdict: no active task"), "{stderr}");
    let _ = fs::remove_dir_all(&root);
}
