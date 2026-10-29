//! The contract a project gets when nobody wrote one: its declared checks, read from the
//! project's own manifest files. Marked `"inferred": true` so `supervisor init` may replace it.

use std::fs;
use std::path::Path;

use serde_json::{Value, json};

/// The inferred contract for the project at `cwd`, or `None` when it declares no checks:
/// `Cargo.toml` gives clippy and `cargo test` (quick) and `cargo test --workspace` (full tier);
/// `package.json` gives `npm run lint` and `npm test` for the scripts it defines;
/// `pyproject.toml` gives `pytest`. Several manifests give the union.
pub fn contract(cwd: &Path) -> Option<Value> {
    // (check, command, full tier)
    let mut checks: Vec<(&str, &str, bool)> = Vec::new();
    if cwd.join("Cargo.toml").is_file() {
        checks.push(("lint", "cargo clippy --all-targets -- -D warnings", false));
        checks.push(("test", "cargo test", false));
        checks.push(("suite", "cargo test --workspace", true));
    }
    let scripts = fs::read_to_string(cwd.join("package.json"))
        .ok()
        .and_then(|text| serde_json::from_str::<Value>(&text).ok())
        .and_then(|v| v.get("scripts").cloned());
    let script = |name: &str| scripts.as_ref().is_some_and(|s| s[name].is_string());
    if script("lint") {
        checks.push(("lint", "npm run lint", false));
    }
    if script("test") {
        checks.push(("test", "npm test", false));
    }
    if cwd.join("pyproject.toml").is_file() {
        checks.push(("test", "pytest", false));
    }
    if checks.is_empty() {
        return None;
    }
    let criteria: Vec<Value> = checks
        .iter()
        .enumerate()
        .map(|(i, (check, command, full))| {
            let mut c = json!({"id": format!("C{}", i + 1), "check": check, "mandatory": true, "command": command});
            if *full {
                c["tier"] = "full".into();
            }
            c
        })
        .collect();
    let name = cwd.file_name().map_or_else(
        || "project".to_owned(),
        |n| n.to_string_lossy().into_owned(),
    );
    Some(json!({
        "id": format!("inferred-{name}"),
        "goal": "The project's declared checks pass after the last change.",
        "inferred": true,
        "criteria": criteria,
        "forbidden_effects": [],
        // The empty prefix matches every path: the scope is the whole project.
        "scope": [""],
        "max_repair_rounds": 2,
    }))
}
