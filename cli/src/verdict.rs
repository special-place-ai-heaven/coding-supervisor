//! `supervisor verdict`: recompute, read-only, whether every mandatory criterion of the active
//! task has a current receipt that satisfies the required provenance. The stored gate state is
//! never consulted; only receipts, the hook run journal and the live git candidate are.

use std::path::{Path, PathBuf};

use coding_supervisor::{CandidateId, Criterion, VerificationReceipt};
use serde_json::{Value, json};
use supervisor_claude_code::adapter::{MCP_ASSERTED, receipt_is_observed};
use supervisor_claude_code::candidate::{CandidateProvider, GitCandidate};
use supervisor_claude_code::lifecycle;
use supervisor_claude_code::location::{canonical_project, resolve_state_dir_checked};
use supervisor_claude_code::state::{Stored, load_gate, read_contract};

use crate::{Failure, Flags};

/// Exit code when the verdict was computed and is `fail`.
pub const EXIT_UNMET: u8 = 3;

const TC_RECONSTRUCTED: &str = "tc-reconstructed:";

/// Who observed the run a receipt cites.
#[derive(Clone, Copy, PartialEq)]
enum Observer {
    Hook,
    Tc,
}

/// Where a receipt really came from. Derived from the receipt and the hook run journal, never
/// stored: a receipt is `Observed` only when a hook-written run record backs it.
#[derive(Clone, Copy, PartialEq)]
enum Provenance {
    Observed(Observer),
    /// Recorded through MCP `receipt_record`: the caller's word only.
    AssertedMcp,
    /// Legacy or unrecognised `output_ref`, or an observed-looking reference with no matching
    /// run record. Fails closed: never counts as observed.
    Unverified,
}

impl Provenance {
    fn of(dir: &Path, task_id: &str, receipt: &VerificationReceipt) -> Self {
        if receipt.output_ref.starts_with(MCP_ASSERTED) {
            Self::AssertedMcp
        } else if receipt_is_observed(dir, task_id, receipt) {
            Self::Observed(if receipt.output_ref.starts_with(TC_RECONSTRUCTED) {
                Observer::Tc
            } else {
                Observer::Hook
            })
        } else {
            Self::Unverified
        }
    }
    fn code(self) -> &'static str {
        match self {
            Self::Observed(Observer::Hook) => "hook",
            Self::Observed(Observer::Tc) => "tc",
            Self::AssertedMcp => "mcp",
            Self::Unverified => "unverified",
        }
    }
}

/// The candidate the verdict is evaluated against.
enum Head {
    Known(CandidateId),
    Unknown(String),
}

#[derive(Clone, Copy, PartialEq)]
enum StaleReason {
    ReceiptAtOtherHead,
    WorktreeChanged,
    HeadUnknown,
}

#[derive(Clone, Copy, PartialEq)]
enum FailReason {
    NonZeroExit(i32),
    NoExitCode,
    ClaimUnsupported,
}

#[derive(Clone, Copy, PartialEq)]
enum State {
    Observed,
    AssertedNotObserved,
    Unverified,
    Failed(FailReason),
    Stale(StaleReason),
    Missing,
}

impl State {
    fn code(self) -> &'static str {
        match self {
            Self::Observed => "observed",
            Self::AssertedNotObserved => "asserted_not_observed",
            Self::Unverified => "unverified",
            Self::Failed(_) => "failed",
            Self::Stale(_) => "stale",
            Self::Missing => "missing",
        }
    }
    fn reason(self) -> Option<&'static str> {
        match self {
            Self::Failed(FailReason::NonZeroExit(_)) => Some("exit_nonzero"),
            Self::Failed(FailReason::NoExitCode) => Some("no_exit_code"),
            Self::Failed(FailReason::ClaimUnsupported) => Some("claim_unsupported"),
            Self::Stale(StaleReason::ReceiptAtOtherHead) => Some("receipt_at_other_head"),
            Self::Stale(StaleReason::WorktreeChanged) => Some("worktree_changed"),
            Self::Stale(StaleReason::HeadUnknown) => Some("head_unknown"),
            Self::Unverified => Some("no_observation_record"),
            Self::Observed | Self::AssertedNotObserved | Self::Missing => None,
        }
    }
}

/// The provenance a mandatory criterion must reach to pass.
#[derive(Clone, Copy)]
enum Require {
    /// Strict: only hook- or TC-observed receipts satisfy.
    Observed,
    /// Lenient (D2, through M3): an MCP-asserted receipt also satisfies, and is flagged.
    Asserted,
}

impl Require {
    fn parse(value: Option<&str>) -> Result<Self, Failure> {
        match value {
            None | Some("observed") => Ok(Self::Observed),
            Some("asserted") => Ok(Self::Asserted),
            Some(other) => Err(Failure::Usage(format!(
                "verdict --require must be observed or asserted, not {other}"
            ))),
        }
    }
    fn code(self) -> &'static str {
        match self {
            Self::Observed => "observed",
            Self::Asserted => "asserted",
        }
    }
    fn satisfied_by(self, state: State) -> bool {
        match self {
            Self::Observed => state == State::Observed,
            Self::Asserted => matches!(state, State::Observed | State::AssertedNotObserved),
        }
    }
}

struct CriterionVerdict<'a> {
    criterion: &'a Criterion,
    state: State,
    /// The receipt that decided the state; `None` when missing.
    receipt: Option<(&'a VerificationReceipt, Provenance)>,
}

/// Judge one criterion. `receipts` are in recording order; later receipts win.
fn judge<'a>(
    criterion: &'a Criterion,
    receipts: &[(&'a VerificationReceipt, Provenance)],
    head: &Head,
) -> CriterionVerdict<'a> {
    let mine: Vec<(&VerificationReceipt, Provenance)> = receipts
        .iter()
        .copied()
        .filter(|(r, _)| r.criterion == criterion.id)
        .collect();
    let current = |r: &VerificationReceipt| matches!(head, Head::Known(c) if &r.candidate == c);
    let latest = |want: fn(Provenance) -> bool| {
        mine.iter()
            .rev()
            .find(|(r, p)| want(*p) && current(r))
            .copied()
    };
    let outcome = |r: &VerificationReceipt, pass: State| {
        if !r.claim_supported {
            return State::Failed(FailReason::ClaimUnsupported);
        }
        match r.exit_code {
            Some(0) => pass,
            Some(code) => State::Failed(FailReason::NonZeroExit(code)),
            None => State::Failed(FailReason::NoExitCode),
        }
    };
    let (state, receipt) = if let Some(hit) = latest(|p| matches!(p, Provenance::Observed(_))) {
        (outcome(hit.0, State::Observed), Some(hit))
    } else if let Some(hit) = latest(|p| p == Provenance::AssertedMcp) {
        (outcome(hit.0, State::AssertedNotObserved), Some(hit))
    } else if let Some(hit) = latest(|p| p == Provenance::Unverified) {
        (State::Unverified, Some(hit))
    } else if let Some(&hit) = mine.last() {
        let reason = match head {
            Head::Unknown(_) => StaleReason::HeadUnknown,
            Head::Known(c) if hit.0.candidate.base_commit != c.base_commit => {
                StaleReason::ReceiptAtOtherHead
            }
            Head::Known(_) => StaleReason::WorktreeChanged,
        };
        (State::Stale(reason), Some(hit))
    } else {
        (State::Missing, None)
    };
    CriterionVerdict {
        criterion,
        state,
        receipt,
    }
}

/// The whole verdict as JSON, plus its exit code (0 pass, [`EXIT_UNMET`] fail).
fn report(
    task: Option<(&str, &str)>,
    head: &Head,
    require: Require,
    verdicts: &[CriterionVerdict<'_>],
) -> (Value, u8) {
    let mut unmet = Vec::new();
    let mut flags = Vec::new();
    let criteria: Vec<Value> = verdicts
        .iter()
        .map(|v| {
            let id = v.criterion.id.0.as_str();
            let satisfies = require.satisfied_by(v.state);
            if v.criterion.mandatory && !satisfies {
                unmet.push(json!(id));
            }
            if v.state == State::AssertedNotObserved {
                flags.push(json!(format!("{id}: asserted, not observed")));
            }
            json!({
                "id": id,
                "mandatory": v.criterion.mandatory,
                "state": v.state.code(),
                "reason": v.state.reason(),
                "provenance": v.receipt.map(|(_, p)| p.code()),
                "receipt": v.receipt.map(|(r, _)| json!({
                    "head_sha": r.candidate.base_commit,
                    "dirty_digest": r.candidate.dirty_digest,
                    "command": r.command,
                    "exit_code": r.exit_code,
                    "recorded_at": r.finished_at,
                })),
                "satisfies": satisfies,
            })
        })
        .collect();
    let head_unknown = matches!(head, Head::Unknown(_));
    let pass = task.is_some() && !head_unknown && unmet.is_empty();
    let exit_code = if pass { 0 } else { EXIT_UNMET };
    let reason = if task.is_none() {
        Some("no_active_task")
    } else if head_unknown {
        Some("head_unknown")
    } else if !unmet.is_empty() {
        Some("mandatory_unmet")
    } else {
        None
    };
    let value = json!({
        "schema": "supervisor.verdict/1",
        "verdict": if pass { "pass" } else { "fail" },
        "reason": reason,
        "exit_code": exit_code,
        "require": require.code(),
        "task": task.map(|(task_id, contract_id)| json!({"task_id": task_id, "contract_id": contract_id})),
        "head": match head {
            Head::Known(c) => json!({"sha": c.base_commit, "dirty_digest": c.dirty_digest, "reason": null}),
            Head::Unknown(why) => json!({"sha": null, "dirty_digest": null, "reason": why}),
        },
        "criteria": criteria,
        "unmet": unmet,
        "flags": flags,
    });
    (value, exit_code)
}

fn short(sha: &str) -> &str {
    sha.get(..12).unwrap_or(sha)
}

/// One line per criterion, then the verdict line.
fn human(report: &Value) -> String {
    let mut lines = Vec::new();
    for c in report["criteria"].as_array().into_iter().flatten() {
        let id = c["id"].as_str().unwrap_or("?");
        let phrase = match (c["state"].as_str(), c["reason"].as_str()) {
            (Some("observed"), _) => {
                format!("observed ({})", c["provenance"].as_str().unwrap_or("?"))
            }
            (Some("asserted_not_observed"), _) => "asserted, not observed".to_owned(),
            (Some("unverified"), _) => "unverified (no observation record)".to_owned(),
            (Some("failed"), Some("exit_nonzero")) => {
                format!("failed (exit {})", c["receipt"]["exit_code"])
            }
            (Some("failed"), Some(reason)) => format!("failed ({})", reason.replace('_', " ")),
            (Some("stale"), Some("receipt_at_other_head")) => format!(
                "stale (receipt at other head {})",
                short(c["receipt"]["head_sha"].as_str().unwrap_or("?"))
            ),
            (Some("stale"), Some(reason)) => format!("stale ({})", reason.replace('_', " ")),
            (Some(state), _) => state.to_owned(),
            (None, _) => "unknown".to_owned(),
        };
        let optional = if c["mandatory"] == false {
            " [optional]"
        } else {
            ""
        };
        lines.push(format!("{id}: {phrase}{optional}"));
    }
    let head = report["head"]["sha"]
        .as_str()
        .map_or_else(|| "unknown".to_owned(), |s| short(s).to_owned());
    let task = report["task"]["task_id"].as_str().unwrap_or("none");
    let reason = report["reason"]
        .as_str()
        .map_or(String::new(), |r| format!("; {}", r.replace('_', " ")));
    lines.push(format!(
        "verdict: {} (require {}; head {head}; task {task}{reason})",
        report["verdict"].as_str().unwrap_or("fail"),
        report["require"].as_str().unwrap_or("observed"),
    ));
    lines.join("\n")
}

pub fn run(flags: &Flags) -> Result<(), Failure> {
    let require = Require::parse(flags.value("--require"))?;
    let project = PathBuf::from(canonical_project(&flags.project()?));
    let (root, _) = resolve_state_dir_checked(&project, &|key| std::env::var(key).ok())
        .map_err(|_| Failure::Error("verdict could not resolve project state safely".to_owned()))?;
    let _transaction =
        supervisor_claude_code::transaction::Transaction::open(&root).map_err(|_| {
            Failure::Error("verdict could not read a consistent project snapshot".to_owned())
        })?;
    let head = match (GitCandidate {
        cwd: project.clone(),
    })
    .observe()
    {
        Ok(candidate) => Head::Known(candidate.id),
        Err(why) => Head::Unknown(why),
    };
    let task = lifecycle::active_task(&root)
        .map_err(|e| Failure::Error(format!("verdict: active task pointer invalid: {e}")))?;
    if let Some(wanted) = flags.value("--task")
        && task.as_ref().is_none_or(|t| t.task_id != wanted)
    {
        return Err(Failure::Error(
            "verdict: --task is not the active task".to_owned(),
        ));
    }
    let (value, exit_code) = match task {
        None => report(None, &head, require, &[]),
        Some(task) => {
            let (contract, digest) = read_contract(&task.dir).map_err(|e| {
                Failure::Error(format!("verdict: active contract unavailable: {e}"))
            })?;
            let contract = contract
                .ok_or_else(|| Failure::Error("verdict: active contract missing".to_owned()))?;
            let receipts = match load_gate(&task.dir, &digest) {
                Ok(Stored::Valid(gate)) => gate.receipts,
                Ok(Stored::Absent) => Vec::new(),
                Ok(Stored::Tampered(what)) => {
                    return Err(Failure::Error(format!(
                        "verdict: task checkpoint tampered ({what})"
                    )));
                }
                Err(e) => {
                    return Err(Failure::Error(format!(
                        "verdict: task checkpoint unavailable: {e}"
                    )));
                }
            };
            let judged: Vec<(&VerificationReceipt, Provenance)> = receipts
                .iter()
                .map(|r| (r, Provenance::of(&task.dir, &task.task_id, r)))
                .collect();
            let verdicts: Vec<CriterionVerdict<'_>> = contract
                .criteria
                .iter()
                .map(|c| judge(c, &judged, &head))
                .collect();
            report(
                Some((&task.task_id, &contract.id.0)),
                &head,
                require,
                &verdicts,
            )
        }
    };
    if flags.switch("--json") {
        println!("{value}");
    } else {
        println!("{}", human(&value));
    }
    if exit_code == 0 {
        Ok(())
    } else {
        Err(Failure::Unmet)
    }
}
