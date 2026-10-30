//! `supervisor init`: an inactive draft contract, never overwritten.

use serde_json::json;
use supervisor_claude_code::location::resolve_state_dir_checked;

use crate::install::HARNESSES;
use crate::{Failure, Flags};

pub fn run(flags: &Flags) -> Result<(), Failure> {
    let project = flags.project()?;
    let (state_dir, rule) = resolve_state_dir_checked(&project, &|name| std::env::var(name).ok())
        .map_err(Failure::Error)?;
    println!("project: {}", project.display());
    println!("state directory: {} (rule: {rule})", state_dir.display());

    let path = flags
        .value("--output")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| project.join("supervisor-contract.json"));
    if path.starts_with(&state_dir) {
        return Err(Failure::Error(
            "draft output must be outside protected state".to_owned(),
        ));
    }
    if path.exists() {
        println!("contract exists, left unchanged: {}", path.display());
    } else {
        let id = project
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "task".into());
        // C1..C4 are the Duet checks; only C2 and C3 are mandatory in the starting contract.
        // The commands are placeholders: edit them to the exact commands a receipt must cite.
        let criterion = |id: &str, check: &str, mandatory: bool, command: &str| json!({ "id": id, "check": check, "mandatory": mandatory, "command": command });
        let contract = json!({
            "id": id,
            "goal": "Describe what this task must achieve.",
            "criteria": [
                criterion("C1", "baseline_reproduces_reported_failure", false, "cargo test"),
                criterion("C2", "candidate_passes_reported_test", true, "cargo test"),
                criterion("C3", "regression_suite_passes", true, "cargo test --workspace"),
                criterion("C4", "diff_within_contract_scope", false, "git diff --name-only"),
            ],
            "forbidden_effects": [],
            "scope": ["src/", "tests/"],
            "max_repair_rounds": 2,
        });
        let text = serde_json::to_string_pretty(&contract)
            .map_err(|e| Failure::Error(format!("contract: {e}")))?;
        supervisor_claude_code::transaction::atomic_write(&path, (text + "\n").as_bytes())
            .map_err(Failure::Error)?;
        println!("contract written: {}", path.display());
    }
    println!(
        "Edit the draft's goal, exact commands and scope, then activate it with supervisor task start --contract <file>. Install with one of:"
    );
    for harness in HARNESSES {
        println!(
            "  supervisor install --harness {} --project {}",
            harness.id,
            project.display()
        );
    }
    Ok(())
}
