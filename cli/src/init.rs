//! `supervisor init`: a starting contract in the gate's state directory. It replaces a
//! contract the gate inferred and never overwrites one written by hand.

use std::fs;

use serde_json::json;
use supervisor_claude_code::location::resolve_state_dir;
use supervisor_claude_code::state::write_atomic;

use crate::install::HARNESSES;
use crate::{Failure, Flags};

pub fn run(flags: &Flags) -> Result<(), Failure> {
    let project = flags.project()?;
    let (state_dir, rule) = resolve_state_dir(&project, &|name| std::env::var(name).ok());
    println!("project: {}", project.display());
    println!("state directory: {} (rule: {rule})", state_dir.display());

    let path = state_dir.join("contract.json");
    // An inferred contract (written at SessionStart) is replaced; a hand-written one never is.
    let inferred = fs::read_to_string(&path)
        .ok()
        .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
        .is_some_and(|v| v["inferred"] == true);
    if path.exists() && !inferred {
        println!("contract exists, left unchanged: {}", path.display());
    } else {
        let gate = state_dir.join("gate.json");
        if inferred && gate.exists() {
            // gate.json is chained to the contract's bytes: keep the inferred task's state
            // beside it, so the next hook call starts a task on the new contract instead of
            // reporting tampering.
            let aside = state_dir.join("gate.json.inferred");
            fs::rename(&gate, &aside)
                .map_err(|e| Failure::Error(format!("{}: {e}", gate.display())))?;
            println!("inferred task state moved to {}", aside.display());
        }
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
        write_atomic(&path, &(text + "\n")).map_err(Failure::Error)?;
        if inferred {
            println!(
                "contract written (replaces the inferred one): {}",
                path.display()
            );
        } else {
            println!("contract written: {}", path.display());
        }
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
