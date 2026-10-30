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
