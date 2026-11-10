//! WP4: task time and turn limits, recomputed from the contract, the task's events.jsonl and the
//! clock wherever they are needed; nothing about them is stored.
//!
//! A turn is one PreToolUse hook call judged for the task: it reached the task gate and appended
//! a task events.jsonl line (`"event":"PreToolUse"`), read-only calls that reach it included. A
//! cached replay appends nothing and is not a turn; with prompts registered, read-only tool
//! calls and intent proposals are answered before the task gate and are not turns either. Wall
//! clock is the hook clock minus `approval.at`; without an approval time there is no wall-clock
//! limit, and pauses do not stop it. The limit caps judged hook calls, not processes: a
//! background job started before it keeps running.
//!
//! The journal is not hash-chained, so a same-user agent past the state-path guard can lower
//! the count by removing journal lines, and that is not detected. A stored high-water mark was
//! considered and left out: gate.json is part of the replay digest (`lifecycle::task_digest`),
//! so a mark changing on every call would refuse legitimate replays of earlier calls.
use std::fs;
use std::io::ErrorKind;
use std::path::Path;

use coding_supervisor::{LimitKind, LimitReached, Limits, TaskContract};
use serde_json::{Value, json};

/// What a task has used against the limits its contract sets.
pub struct Usage {
    pub limits: Limits,
    pub turns_used: u64,
    /// `None` when the contract records no approval time.
    pub elapsed_s: Option<u64>,
    /// The clock reads earlier than `approval.at`; `elapsed_s` is then 0.
    pub clock_behind: bool,
    pub reached: Option<LimitReached>,
}

/// Judged PreToolUse calls in `dir`'s events.jsonl. A line that does not parse counts as one
/// turn: it never loosens a limit and never locks a task. A journal that cannot be read is an
/// error.
// shortcut: parses the whole journal on every hook call (O(journal) per call); keep a running
// count once task journals grow large.
pub fn turns_used(dir: &Path) -> Result<u64, String> {
    let path = dir.join("events.jsonl");
    let bytes = match fs::read(&path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(0),
        Err(e) => return Err(format!("{}: {e}", path.display())),
    };
    let turns = bytes
        .split(|b| *b == b'\n')
        .filter(|line| !line.trim_ascii().is_empty())
        .filter(|line| match serde_json::from_slice::<Value>(line) {
            Ok(v) => v.get("event").and_then(Value::as_str) == Some("PreToolUse"),
            Err(_) => true,
        })
        .count();
    Ok(turns as u64)
}

/// `None` when the contract sets no limit; the journal is not read then.
pub fn usage(dir: &Path, contract: &TaskContract, now: u64) -> Result<Option<Usage>, String> {
    let limits = &contract.limits;
    if !limits.is_set() {
        return Ok(None);
    }
    let at = contract.approval.as_ref().and_then(|a| a.at);
    // `adapter::now` reads a clock before the epoch as 0; that must not read as no time passed.
    if limits.wall_clock_s.is_some() && at.is_some() && now == 0 {
        return Err("system clock unavailable".to_owned());
    }
    let turns_used = turns_used(dir)?;
    let elapsed_s = at.map(|at| now.saturating_sub(at));
    Ok(Some(Usage {
        limits: limits.clone(),
        turns_used,
        elapsed_s,
        clock_behind: at.is_some_and(|at| now < at),
        reached: limits.reached(turns_used, elapsed_s),
    }))
}

/// For `supervisor status`, `supervisor verdict` and MCP `task_state`: `None` without a limit,
/// `{"state":"unknown"}` when the journal or clock cannot be read; a display never fails.
pub fn usage_json(dir: &Path, contract: &TaskContract, now: u64) -> Option<Value> {
    match usage(dir, contract, now) {
        Ok(usage) => usage.map(|u| u.json()),
        Err(_) => Some(json!({"state":"unknown"})),
    }
}

/// "40 of 40 turns used" or "elapsed 612 s of 600 s".
pub fn reached_text(reached: &LimitReached) -> String {
    match reached.kind {
        LimitKind::Turns => format!("{} of {} turns used", reached.used, reached.max),
        LimitKind::WallClock => format!("elapsed {} s of {} s", reached.used, reached.max),
    }
}

/// How new work starts once a task is at a limit: an autonomous task is cancelled first, a
/// person's task is replaced with `task next`.
pub fn way_out(dir: &Path, task_id: &str) -> Result<String, String> {
    Ok(if crate::autonomy::is_autonomous_task(dir)? {
        format!(
            "cancel it with `supervisor task cancel --project <dir> --task {task_id} --reason <text>`, then send a new request"
        )
    } else {
        format!(
            "replace it with `supervisor task next --task {task_id} --contract <file> --disposition budget_exhausted`"
        )
    })
}

impl Usage {
    /// Counts only; hook payloads carry no cost.
    pub fn json(&self) -> Value {
        json!({
            "turns": self.limits.turns.map(|n| n.get()),
            "wall_clock_s": self.limits.wall_clock_s.map(|n| n.get()),
            "turns_used": self.turns_used,
            "elapsed_s": self.elapsed_s,
            "clock_behind_approval": self.clock_behind,
            "reached": self.reached.map(|r| match r.kind {
                LimitKind::Turns => "turns",
                LimitKind::WallClock => "wall_clock",
            }),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Display degrades: a journal that cannot be read shows as unknown, never fails the caller.
    #[test]
    fn an_unreadable_journal_shows_as_unknown_instead_of_failing() {
        let dir =
            std::env::temp_dir().join(format!("supervisor-limits-json-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("events.jsonl")).unwrap();
        let contract =
            crate::state::contract_from(&json!({"schema_version":2,"id":"c","goal":"g","scope":[],
            "forbidden_effects":[],"max_repair_rounds":1,"limits":{"turns":3},
            "criteria":[{"id":"C2","check":"t","mandatory":true,"command":"x"}]}))
            .unwrap();
        let shown = usage_json(&dir, &contract, 1_000);
        let unlimited = crate::state::contract_from(&json!({"id":"c","goal":"g","scope":[],
            "forbidden_effects":[],"max_repair_rounds":1,
            "criteria":[{"id":"C2","check":"t","mandatory":true,"command":"x"}]}))
        .unwrap();
        let none = usage_json(&dir, &unlimited, 1_000);
        let _ = fs::remove_dir_all(&dir);
        assert_eq!(shown, Some(json!({"state":"unknown"})));
        assert_eq!(none, None);
    }

    /// A wall-clock-only contract does not read the journal, so it cannot fail on it.
    #[test]
    fn a_wall_clock_only_contract_does_not_read_the_journal() {
        let dir = std::env::temp_dir().join(format!(
            "supervisor-limits-wall-only-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("events.jsonl")).unwrap();
        let contract = crate::state::contract_from(&json!({"schema_version":2,"id":"c",
            "goal":"g","scope":[],"forbidden_effects":[],"max_repair_rounds":1,
            "approval":{"by":null,"at":1_000,"via":"task-start"},"limits":{"wall_clock_s":600},
            "criteria":[{"id":"C2","check":"t","mandatory":true,"command":"x"}]}))
        .unwrap();
        let usage = usage(&dir, &contract, 1_700);
        let _ = fs::remove_dir_all(&dir);
        let usage = usage.expect("the journal was read").expect("a limit is set");
        assert!(usage.reached.is_some());
    }
}
