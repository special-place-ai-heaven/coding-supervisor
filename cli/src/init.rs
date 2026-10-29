//! `supervisor init`: a starting contract in the gate's state directory, never overwritten.

use std::fs;

use serde_json::json;
use supervisor_claude_code::location::resolve_state_dir;

use crate::install::HARNESSES;
use crate::{Failure, Flags};

pub fn run(flags: &Flags) -> Result<(), Failure> {
    let project = flags.project()?;
    let (state_dir, rule) = resolve_state_dir(&project, &|name| std::env::var(name).ok());
    println!("project: {}", project.display());
    println!("state directory: {} (rule: {rule})", state_dir.display());

    let path = state_dir.join("contract.json");
    if path.exists() {
        println!("contract exists, left unchanged: {}", path.display());
    } else {
        let id = project
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "task".into());
        // C1..C4 are the Duet checks; only C2 and C3 are mandatory in the starting contract.
        let criterion = |id: &str, check: &str, mandatory: bool| json!({ "id": id, "check": check, "mandatory": mandatory });
        let contract = json!({
            "id": id,
            "goal": "Describe what this task must achieve.",
            "criteria": [
                criterion("C1", "baseline_reproduces_reported_failure", false),
                criterion("C2", "candidate_passes_reported_test", true),
                criterion("C3", "regression_suite_passes", true),
                criterion("C4", "diff_within_contract_scope", false),
            ],
            "forbidden_effects": [],
            "scope": ["src/", "tests/"],
            "max_repair_rounds": 2,
        });
        let text = serde_json::to_string_pretty(&contract)
            .map_err(|e| Failure::Error(format!("contract: {e}")))?;
        fs::create_dir_all(&state_dir)
            .and_then(|()| fs::write(&path, text + "\n"))
            .map_err(|e| Failure::Error(format!("{}: {e}", path.display())))?;
        println!("contract written: {}", path.display());
    }
    println!(
        "Edit the contract's goal, criteria and scope there, then register the gate with one of:"
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
