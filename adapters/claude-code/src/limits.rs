//! WP4: task time and turn limits, recomputed from the contract, the task's events.jsonl and the
//! clock wherever they are needed; nothing about them is stored.
//!
//! A turn is one PreToolUse hook call judged for the task: it reached the task gate and appended
//! a task events.jsonl line (`"event":"PreToolUse"`), read-only calls included. A cached replay
//! appends nothing and is not a turn. Wall clock is the hook clock minus `approval.at`; without an
//! approval time there is no wall-clock limit, and pauses do not stop it.
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
    pub reached: Option<LimitReached>,
}

/// Judged PreToolUse calls in `dir`'s events.jsonl. A line that is not JSON is an error, never
/// skipped: a limit must not loosen because its journal cannot be read.
pub fn turns_used(dir: &Path) -> Result<u64, String> {
    let path = dir.join("events.jsonl");
    let text = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(0),
        Err(e) => return Err(format!("{}: {e}", path.display())),
    };
    let mut turns = 0;
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        let v: Value = serde_json::from_str(line)
            .map_err(|e| format!("{}: unreadable journal line ({e})", path.display()))?;
        if v.get("event").and_then(Value::as_str) == Some("PreToolUse") {
            turns += 1;
        }
    }
    Ok(turns)
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
        reached: limits.reached(turns_used, elapsed_s),
    }))
}

/// "40 of 40 turns used" or "elapsed 612 s of 600 s".
pub fn reached_text(reached: &LimitReached) -> String {
    match reached.kind {
        LimitKind::Turns => format!("{} of {} turns used", reached.used, reached.max),
        LimitKind::WallClock => format!("elapsed {} s of {} s", reached.used, reached.max),
    }
}

impl Usage {
    /// For `supervisor status` and MCP `task_state`. Counts only; hook payloads carry no cost.
    pub fn json(&self) -> Value {
        json!({
            "turns": self.limits.turns.map(|n| n.get()),
            "wall_clock_s": self.limits.wall_clock_s.map(|n| n.get()),
            "turns_used": self.turns_used,
            "elapsed_s": self.elapsed_s,
            "reached": self.reached.map(|r| match r.kind {
                LimitKind::Turns => "turns",
                LimitKind::WallClock => "wall_clock",
            }),
        })
    }
}
