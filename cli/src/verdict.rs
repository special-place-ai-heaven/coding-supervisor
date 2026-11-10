//! `supervisor verdict`: recompute whether every mandatory criterion of the active task has a
//! current receipt that satisfies the required provenance. It writes nothing: it takes a shared
//! hold on an existing state lock, recovers no transaction, and runs git without optional locks.
//! Receipts, the hook run journal and the live git candidate decide; the stored gate state is read
//! only to fail a budget_exhausted task.

use std::io::{self, Write};
use std::path::{Path, PathBuf};

use coding_supervisor::{
    CandidateId, Criterion, GateError, Review, TaskState, VerificationReceipt, review_met,
};
use serde_json::{Value, json};
use supervisor_claude_code::adapter::{MCP_ASSERTED, now, receipts_observed};
use supervisor_claude_code::candidate::{GitCandidate, Unobserved};
use supervisor_claude_code::level::{self, Level};
use supervisor_claude_code::lifecycle;
use supervisor_claude_code::location::{canonical_project, resolve_state_dir_read_only};
use supervisor_claude_code::review;
use supervisor_claude_code::state::{Stored, StoredReview, load_gate, read_contract};
use supervisor_claude_code::transaction::read_lock;

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
    /// `observed`: a hook completion journal backs the receipt.
    fn of(receipt: &VerificationReceipt, observed: bool) -> Self {
        if receipt.output_ref.starts_with(MCP_ASSERTED) {
            Self::AssertedMcp
        } else if observed {
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
    Unknown(HeadReason),
}

/// Why HEAD is unknown. The free-text cause goes to stderr only.
#[derive(Clone, Copy)]
enum HeadReason {
    GitUnavailable,
    GitFailed,
    WorktreeUnreadable,
    /// git answered, but not with a 40-hex sha and a 64-hex digest.
    HeadUnparseable,
}

impl HeadReason {
    fn code(self) -> &'static str {
        match self {
            Self::GitUnavailable => "git_unavailable",
            Self::GitFailed => "git_failed",
            Self::WorktreeUnreadable => "worktree_unreadable",
            Self::HeadUnparseable => "head_unparseable",
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum StaleReason {
    ReceiptAtOtherHead,
    WorktreeChanged,
    HeadUnknown,
}

#[derive(Clone, Copy, PartialEq)]
enum FailReason {
    ExitNonZero,
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
            Self::Failed(FailReason::ExitNonZero) => Some("exit_nonzero"),
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
    /// Lenient (D2, before M6): an MCP-asserted receipt also satisfies, and is flagged.
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
            Some(_) => State::Failed(FailReason::ExitNonZero),
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

/// The active task the verdict is about.
struct TaskRef<'a> {
    task_id: &'a str,
    contract_id: &'a str,
    /// SHA-256 of contract.json's bytes.
    contract_digest: &'a str,
    /// `Some` for a budget_exhausted task: whether a limit is its recorded cause, and the task
    /// limit reached now, if any.
    exhausted: Option<(bool, Option<String>)>,
}

/// WP5: the contract's review policy judged on the head candidate by the core rule `try_accept`
/// uses, as `{required, independent, met, reason, latest}`, and whether it is met (`true` when
/// HEAD is unknown, which fails on its own). A review copied across a commit by the hooks counts;
/// this read-only verdict copies nothing itself.
fn review_report(policy: &Review, reviews: &[StoredReview], head: &Head) -> (Value, bool) {
    let Head::Known(candidate) = head else {
        return (
            json!({"required":policy.required,"independent":policy.independent,"met":null,"reason":null,"latest":null}),
            true,
        );
    };
    let records: Vec<_> = reviews.iter().map(|s| s.review.clone()).collect();
    let reason = review_met(policy, &records, candidate)
        .err()
        .map(|e| match e {
            GateError::ReviewRejected { .. } => "review_rejected",
            GateError::ReviewNotIndependent => "review_not_independent",
            _ => "review_required",
        });
    let latest = review::latest(reviews, candidate).map(review::record_json);
    (
        json!({"required":policy.required,"independent":policy.independent,"met":reason.is_none(),"reason":reason,"latest":latest}),
        reason.is_none(),
    )
}

/// The whole verdict as JSON, plus its exit code (0 pass, [`EXIT_UNMET`] fail). Every key is
/// always present; absent values are `null`.
fn report(
    task: Option<&TaskRef<'_>>,
    head: &Head,
    require: Require,
    verdicts: &[CriterionVerdict<'_>],
    (effective, level_source): (Level, &str),
    review: Option<(Value, bool)>,
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
                flags.push(json!({
                    "criterion": id,
                    "code": "asserted_not_observed",
                    "mandatory": v.criterion.mandatory,
                }));
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
                    "recorded_at": rfc3339_millis(r.finished_at),
                    "output_ref": r.output_ref,
                })),
                "satisfies": satisfies,
            })
        })
        .collect();
    let reason = if task.is_none() {
        Some("no_active_task")
    } else if matches!(head, Head::Unknown(_)) {
        Some("head_unknown")
    } else if task.is_some_and(|t| t.exhausted.is_some()) {
        Some("budget_exhausted")
    } else if !verdicts.iter().any(|v| v.criterion.mandatory) {
        Some("no_mandatory_criteria")
    } else if !unmet.is_empty() {
        Some("mandatory_unmet")
    } else if review.as_ref().is_some_and(|(_, met)| !met) {
        Some("review_unmet")
    } else {
        None
    };
    let pass = reason.is_none();
    let exit_code = if pass { 0 } else { EXIT_UNMET };
    let value = json!({
        "schema_version": 1,
        "verdict": if pass { "pass" } else { "fail" },
        "reason": reason,
        "exit_code": exit_code,
        "require": require.code(),
        "effective_level": effective.name(),
        "level_source": level_source,
        "task": task.map(|t| json!({
            "task_id": t.task_id,
            "contract_id": t.contract_id,
            "contract_digest": t.contract_digest,
        })),
        "head": match head {
            Head::Known(c) => json!({"sha": c.base_commit, "dirty_digest": c.dirty_digest, "reason": null}),
            Head::Unknown(why) => json!({"sha": null, "dirty_digest": null, "reason": why.code()}),
        },
        "criteria": criteria,
        "unmet": unmet,
        "flags": flags,
        "review": review.map(|(value, _)| value),
    });
    (value, exit_code)
}

/// Unix seconds as RFC 3339 UTC with milliseconds, e.g. `2000-02-29T00:00:02.000Z`.
pub(crate) fn rfc3339_millis(secs: u64) -> String {
    let (days, time) = (secs / 86_400, secs % 86_400);
    // Howard Hinnant's civil_from_days, for days since 1970-01-01.
    let z = days + 719_468;
    let (era, doe) = (z / 146_097, z % 146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = era * 400 + yoe + u64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.000Z",
        time / 3_600,
        time % 3_600 / 60,
        time % 60
    )
}

fn is_lower_hex(s: &str, len: usize) -> bool {
    s.len() == len && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// The live candidate, or why it is unknown with the free-text cause for stderr.
fn observe_head(project: &Path) -> (Head, Option<String>) {
    let (reason, why) = match (GitCandidate {
        cwd: project.to_path_buf(),
    })
    .observe_git()
    {
        Ok(c) if is_lower_hex(&c.id.base_commit, 40) && is_lower_hex(&c.id.dirty_digest, 64) => {
            return (Head::Known(c.id), None);
        }
        Ok(c) => (
            HeadReason::HeadUnparseable,
            format!(
                "git gave HEAD {:?} with digest {:?}",
                c.id.base_commit, c.id.dirty_digest
            ),
        ),
        Err(e @ Unobserved::GitUnavailable(_)) => (HeadReason::GitUnavailable, e.to_string()),
        Err(e @ Unobserved::GitFailed(_)) => (HeadReason::GitFailed, e.to_string()),
        Err(e @ Unobserved::WorktreeUnreadable(_)) => {
            (HeadReason::WorktreeUnreadable, e.to_string())
        }
    };
    (Head::Unknown(reason), Some(why))
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
    let env = |key: &str| std::env::var(key).ok();
    let (root, _) = resolve_state_dir_read_only(&project, &env)
        .map_err(|e| Failure::Error(format!("verdict: project state unusable: {e}")))?;
    let _lock = read_lock(&root)
        .map_err(|e| Failure::Error(format!("verdict: project state not readable: {e}")))?;
    let stamp = level::in_effect(&root, level::config_dir(&env).as_deref());
    // One HEAD snapshot, taken before any receipt is read, is what every receipt is judged by.
    let (head, why) = observe_head(&project);
    let task = lifecycle::active_task(&root)
        .map_err(|e| Failure::Error(format!("verdict: active task pointer invalid: {e}")))?;
    // X3: with no active task that is the answer; the unknown head is only a side note.
    if let Some(why) = why {
        let lead = if task.is_some() {
            "verdict: head unknown"
        } else {
            "verdict: no active task (head also unknown)"
        };
        let _ = writeln!(io::stderr(), "{lead}: {why}");
    }
    if let Some(wanted) = flags.value("--task")
        && task.as_ref().is_none_or(|t| t.task_id != wanted)
    {
        return Err(Failure::Error(
            "verdict: --task is not the active task".to_owned(),
        ));
    }
    let mut exhausted_note = None;
    let (value, exit_code) = match task {
        None => report(None, &head, require, &[], stamp, None),
        Some(task) => {
            let (contract, digest) = read_contract(&task.dir).map_err(|e| {
                Failure::Error(format!("verdict: active contract unavailable: {e}"))
            })?;
            let contract = contract
                .ok_or_else(|| Failure::Error("verdict: active contract missing".to_owned()))?;
            let (receipts, gate_state, next_action, reviews) = match load_gate(&task.dir, &digest) {
                Ok(Stored::Valid(gate)) => (
                    gate.receipts,
                    Some(gate.checkpoint.state),
                    gate.checkpoint.next_action,
                    gate.reviews,
                ),
                Ok(Stored::Absent) => (Vec::new(), None, String::new(), Vec::new()),
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
            let observed = receipts_observed(&task.dir, &task.task_id, &receipts)
                .map_err(|e| Failure::Error(format!("verdict: run journal unreadable: {e}")))?;
            let judged: Vec<(&VerificationReceipt, Provenance)> = receipts
                .iter()
                .zip(observed)
                .map(|(r, observed)| (r, Provenance::of(r, observed)))
                .collect();
            let verdicts: Vec<CriterionVerdict<'_>> = contract
                .criteria
                .iter()
                .map(|c| judge(c, &judged, &head))
                .collect();
            // WP4: a budget_exhausted task never passes. The text output names a limit as the
            // cause only when the gate recorded it as one (`Task::exhaust_limit`); a limit that
            // is merely reached now is shown apart.
            let exhausted = (gate_state == Some(TaskState::BudgetExhausted)).then(|| {
                let reached = supervisor_claude_code::limits::usage(&task.dir, &contract, now())
                    .ok()
                    .flatten()
                    .and_then(|usage| usage.reached)
                    .map(|reached| supervisor_claude_code::limits::reached_text(&reached));
                (next_action.starts_with("task limit reached"), reached)
            });
            exhausted_note = exhausted.clone();
            let task_ref = TaskRef {
                task_id: &task.task_id,
                contract_id: &contract.id.0,
                contract_digest: &digest,
                exhausted,
            };
            let review = review_report(&contract.review, &reviews, &head);
            report(
                Some(&task_ref),
                &head,
                require,
                &verdicts,
                stamp,
                Some(review),
            )
        }
    };
    if flags.switch("--json") {
        println!("{value}");
    } else {
        let mut text = human(&value);
        if let Some((limit_caused, reached)) = exhausted_note {
            let note = match reached {
                Some(limit) if limit_caused => {
                    format!("task state: budget_exhausted (task limit reached: {limit})")
                }
                Some(limit) => format!("task state: budget_exhausted\nlimit usage now: {limit}"),
                None => "task state: budget_exhausted".to_owned(),
            };
            let at = text.rfind('\n').map_or(0, |i| i + 1);
            text.insert_str(at, &format!("{note}\n"));
        }
        println!("{text}");
    }
    if exit_code == 0 {
        Ok(())
    } else {
        Err(Failure::Unmet)
    }
}
