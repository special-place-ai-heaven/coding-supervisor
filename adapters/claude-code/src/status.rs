//! `supervisor status`: the level, the required checks and the completion decisions so far,
//! read from the state directory. Prints no command output: only gate texts and contract fields.

use std::collections::BTreeMap;
use std::fs;
use std::io::ErrorKind;
use std::path::Path;

use serde_json::Value;

use crate::journal::reason_part;
use crate::level::{self, Level};
use crate::state::{self, Stored};

/// The status text for one project's state directory. Fails only when a level file is unreadable.
pub fn render(state_dir: &Path, config_dir: Option<&Path>) -> Result<String, String> {
    let (level, source) = level::effective(state_dir, config_dir)?;
    let mut out = vec![
        format!("level: {} ({source})", level.name()),
        format!("state directory: {}", state_dir.display()),
    ];
    let (contract, digest) = match state::read_contract_tiers(state_dir) {
        Ok(read) => read,
        Err(e) => {
            out.push(format!("contract: unreadable ({e})"));
            (None, String::new())
        }
    };
    // The chain binds gate.json to the contract's bytes; without them it cannot be checked.
    let gate = if digest.is_empty() {
        Err("contract unreadable".to_owned())
    } else {
        state::load_gate(state_dir, &digest)
    };
    match gate {
        Ok(Stored::Valid(gate)) => {
            let failures: Vec<String> = gate
                .sessions
                .iter()
                .flat_map(|(sid, s)| {
                    s.baseline_failures.iter().map(move |(id, why)| {
                        format!("baseline failure: session {sid}, {id}: {why}")
                    })
                })
                .collect();
            if failures.is_empty() {
                out.push("baseline failures: none".to_owned());
            }
            out.extend(failures);
        }
        Ok(Stored::Absent) => out.push("baseline failures: none (no gate state yet)".to_owned()),
        Ok(Stored::Tampered(what)) => {
            out.push(format!(
                "baseline failures: unknown, gate state tampered ({what})"
            ));
        }
        Err(e) => out.push(format!(
            "baseline failures: unknown, gate state unreadable ({e})"
        )),
    }
    match contract {
        None if digest.is_empty() => {}
        None => out.push("contract: none (no contract.json); nothing is judged".to_owned()),
        Some((contract, full_tier)) => {
            out.push("checks (id | command | tier | mandatory | required now):".to_owned());
            for c in &contract.criteria {
                let full = full_tier.contains(&c.id.0);
                let required = c.mandatory && (!full || level == Level::Ultra);
                let commands: Vec<String> = c.command.iter().map(|s| format!("`{s}`")).collect();
                out.push(format!(
                    "  {} | {} | {} | {} | {}",
                    c.id.0,
                    commands.join(" or "),
                    if full { "full" } else { "quick" },
                    if c.mandatory { "yes" } else { "no" },
                    if required { "yes" } else { "no" },
                ));
            }
        }
    }
    out.extend(completions(state_dir));
    Ok(out.join("\n"))
}

/// Accepted completions, refusals and lite notices from events.jsonl, with refusal reasons.
fn completions(state_dir: &Path) -> Vec<String> {
    let path = state_dir.join("events.jsonl");
    let text = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == ErrorKind::NotFound => {
            return vec!["journal: none (no events.jsonl); no completion judged yet".to_owned()];
        }
        Err(e) => return vec![format!("journal: unreadable ({e})")],
    };
    let (mut accepted, mut refused, mut noticed, mut unreadable) = (0, 0, 0, 0);
    let mut reasons: BTreeMap<String, u64> = BTreeMap::new();
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            unreadable += 1;
            continue;
        };
        let event = v.get("event").and_then(Value::as_str).unwrap_or_default();
        if !matches!(event, "Stop" | "SubagentStop") {
            continue;
        }
        let decision = v.get("decision");
        let kind = decision.and_then(|d| d.get("kind")).and_then(Value::as_str);
        let decision_text = decision
            .and_then(|d| d.get("text"))
            .and_then(Value::as_str)
            .unwrap_or_default();
        let lite = v.get("reasons").and_then(Value::as_array).is_some_and(|r| {
            r.iter()
                .any(|r| r.as_str() == Some("level lite: reported, not blocked"))
        });
        if event == "Stop" && v.get("accepted").and_then(Value::as_bool) == Some(true) {
            accepted += 1;
        } else if kind == Some("block") {
            refused += 1;
            *reasons.entry(reason_part(decision_text)).or_default() += 1;
        } else if kind == Some("system") && lite {
            noticed += 1;
            *reasons
                .entry(format!(
                    "{} (lite, not blocked)",
                    reason_part(decision_text)
                ))
                .or_default() += 1;
        }
    }
    let mut out = vec![
        format!("completions accepted: {accepted}"),
        format!("completions refused: {refused}"),
        format!("completions reported, not blocked (lite): {noticed}"),
    ];
    if unreadable > 0 {
        out.push(format!("journal lines not readable, skipped: {unreadable}"));
    }
    let mut by_count: Vec<(String, u64)> = reasons.into_iter().collect();
    by_count.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    if !by_count.is_empty() {
        out.push("refusal reasons:".to_owned());
        out.extend(by_count.iter().map(|(r, n)| format!("  {n}x {r}")));
    }
    out
}
