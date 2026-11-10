#![forbid(unsafe_code)]

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TaskId(pub String);
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CriterionId(pub String);
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CheckName(pub String);
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FindingId(pub String);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Criterion {
    pub id: CriterionId,
    pub check: CheckName,
    pub mandatory: bool,
    /// The commands a receipt for this criterion may cite. Contract loaders reject an empty
    /// list; should one reach the core anyway, it accepts no receipt.
    pub command: Vec<String>,
}
impl Criterion {
    /// Exact text after trimming both sides; no normalisation of shells, quoting or paths.
    pub fn accepts(&self, command: &str) -> bool {
        self.command.iter().any(|c| c.trim() == command.trim())
    }
    /// The Duet C1 check: the reported failure reproduced on the task's starting candidate.
    pub fn is_baseline(&self) -> bool {
        self.check.0 == "baseline_reproduces_reported_failure"
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Budget {
    pub max_repair_rounds: u32,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TaskContract {
    pub id: TaskId,
    pub goal: String,
    pub criteria: Vec<Criterion>,
    /// The core state machine does not observe effects. The Claude Code adapter refuses the
    /// typed entries (`deny_path:`, `deny_git:`, `deny_argv_prefix:`, `deny_git_push_target:`,
    /// `deny_git_force_push`) at PreToolUse in strict mode; any other entry is recorded only.
    pub forbidden_effects: Vec<String>,
    pub scope: Vec<String>,
    pub budget: Budget,
    /// Contract v2: the commit the task started from; `None` outside a git repository and
    /// for a migrated v1 contract.
    pub base_sha: Option<String>,
    /// Who or what applied the contract. Recorded, not proven.
    pub approval: Option<Approval>,
    pub limits: Limits,
    /// Repository-relative paths the task must leave behind. Stored, not yet enforced.
    pub artifacts: Vec<String>,
    pub review: Review,
    pub isolation: Isolation,
    /// Loaded from a v1 file through the in-memory migration; the file itself stays v1.
    pub migrated_from_v1: bool,
}
/// Recorded, not proven: the writer's own account of who applied a contract.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Approval {
    pub by: Option<String>,
    /// Seconds since the Unix epoch.
    pub at: Option<u64>,
    /// The writer, e.g. `task-start`, `agent-proposal`, `v1-migration`.
    pub via: String,
}
/// Per-task ceilings. A turn is one PreToolUse call judged for the task; wall clock runs from
/// `approval.at`. The caller counts both; the core only compares (see [`Limits::reached`]).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Limits {
    pub wall_clock_s: Option<std::num::NonZeroU64>,
    pub turns: Option<std::num::NonZeroU64>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LimitKind {
    Turns,
    WallClock,
}
/// A limit at or past its maximum: `used` turns or elapsed seconds against `max`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LimitReached {
    pub kind: LimitKind,
    pub used: u64,
    pub max: u64,
}
impl Limits {
    pub fn is_set(&self) -> bool {
        self.turns.is_some() || self.wall_clock_s.is_some()
    }
    /// `turns_used` judged calls before this one, `elapsed_s` since approval (`None` when the
    /// approval time is unknown, which means no wall-clock limit). `turns: n` admits exactly n
    /// calls; the turn limit is reported first when both are reached.
    pub fn reached(&self, turns_used: u64, elapsed_s: Option<u64>) -> Option<LimitReached> {
        let turns = self.turns.map(|max| LimitReached {
            kind: LimitKind::Turns,
            used: turns_used,
            max: max.get(),
        });
        let clock = self
            .wall_clock_s
            .zip(elapsed_s)
            .map(|(max, used)| LimitReached {
                kind: LimitKind::WallClock,
                used,
                max: max.get(),
            });
        [turns, clock]
            .into_iter()
            .flatten()
            .find(|l| l.used >= l.max)
    }
}
/// The contract's review policy, judged by [`review_met`]. `independent` implies `required`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Review {
    pub required: bool,
    pub independent: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReviewDecision {
    Approved,
    Rejected,
}
/// `Observed`: a reviewer the caller saw change nothing in the task. `CallerAsserted`: the
/// builder's own review, or the chair's; it never counts as independent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReviewProvenance {
    Observed,
    CallerAsserted,
}
/// One reviewer's decision on one candidate. An inconclusive answer is not a decision and is
/// never recorded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReviewRecord {
    pub reviewer: String,
    /// As the harness reported it; not proven.
    pub model: Option<String>,
    pub decision: ReviewDecision,
    pub findings: Vec<String>,
    /// Seconds since the Unix epoch.
    pub at: u64,
    pub candidate: CandidateId,
    pub provenance: ReviewProvenance,
}
/// Whether `reviews` meet `policy` on `candidate`. With the policy off nothing blocks. Any
/// rejection on the candidate refuses, whatever approvals sit beside it, until the content
/// changes; otherwise an approval is needed, and under `independent` an observed one.
/// shortcut: a reviewer cannot withdraw its own rejection; judge each reviewer's latest decision
/// instead if mistaken rejections become a real problem.
pub fn review_met(
    policy: &Review,
    reviews: &[ReviewRecord],
    candidate: &CandidateId,
) -> Result<(), GateError> {
    if !policy.required && !policy.independent {
        return Ok(());
    }
    let on: Vec<&ReviewRecord> = reviews
        .iter()
        .filter(|r| &r.candidate == candidate)
        .collect();
    if let Some(r) = on.iter().find(|r| r.decision == ReviewDecision::Rejected) {
        return Err(GateError::ReviewRejected {
            reviewer: r.reviewer.clone(),
            findings: r.findings.len(),
        });
    }
    let approvals: Vec<_> = on
        .into_iter()
        .filter(|r| r.decision == ReviewDecision::Approved)
        .collect();
    if approvals.is_empty() {
        Err(GateError::ReviewRequired)
    } else if policy.independent
        && approvals
            .iter()
            .all(|r| r.provenance == ReviewProvenance::CallerAsserted)
    {
        Err(GateError::ReviewNotIndependent)
    } else {
        Ok(())
    }
}
/// Stored, not yet enforced.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Isolation {
    /// `"none"` in contract.json: the task works in the shared checkout.
    #[default]
    Shared,
    Worktree,
}

/// File scopes are exact; directory scopes end with `/`. Git paths must be normalized
/// relative paths so a matching prefix cannot hide traversal or platform roots.
pub fn scope_contains(scope: &str, path: &str) -> bool {
    let normalized = |value: &str| {
        !value.is_empty()
            && !value.contains(['\\', ':'])
            && value
                .split('/')
                .all(|part| !part.is_empty() && part != "." && part != "..")
    };
    let directory = scope.ends_with('/');
    let scope_path = scope.strip_suffix('/').unwrap_or(scope);
    normalized(path)
        && normalized(scope_path)
        && if directory {
            path.starts_with(scope)
        } else {
            path == scope
        }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CandidateId {
    pub base_commit: String,
    pub dirty_digest: String,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Candidate {
    pub id: CandidateId,
    pub provenance: String,
    pub changed_paths: Vec<String>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VerificationReceipt {
    pub criterion: CriterionId,
    pub candidate: CandidateId,
    pub command: String,
    pub cwd: String,
    pub exit_code: Option<i32>,
    pub output_ref: String,
    pub claim_supported: bool,
    pub started_at: u64,
    pub finished_at: u64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Severity {
    Low,
    Medium,
    High,
    Critical,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Disposition {
    Open,
    Resolved,
    Accepted,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Finding {
    pub id: FindingId,
    pub candidate: CandidateId,
    pub severity: Severity,
    pub location: String,
    pub evidence_ref: String,
    pub disposition: Disposition,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TaskState {
    Queued,
    Preparing,
    Verifying,
    ReadyForReview,
    Repairing,
    Blocked,
    BudgetExhausted,
    Failed,
    Cancelled,
    Completed,
}
impl TaskState {
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Blocked
                | Self::BudgetExhausted
                | Self::Failed
                | Self::Cancelled
                | Self::Completed
        )
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Checkpoint {
    pub state: TaskState,
    pub candidate: Option<CandidateId>,
    pub open_findings: Vec<Finding>,
    pub consumed_repair_rounds: u32,
    pub next_action: String,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReceiptStatus {
    Current,
    Stale,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GateError {
    InvalidTransition {
        from: TaskState,
        event: &'static str,
    },
    CandidateNotObserved,
    ReceiptMissingExitCode,
    ReceiptUnsupported,
    UnknownCriterion(CriterionId),
    WrongCommand {
        criterion: CriterionId,
        cited: String,
        required: Vec<String>,
    },
    ReceiptForOtherCandidate {
        expected: CandidateId,
        got: CandidateId,
    },
    MissingReceipt(CriterionId),
    StaleReceipt(CriterionId),
    FailingReceipt {
        criterion: CriterionId,
        exit_code: i32,
    },
    OutOfScope(Vec<String>),
    OpenFinding(FindingId),
    /// Reproduction required: the baseline did not fail on the starting candidate.
    NotReproduced(CriterionId),
    UnknownFinding(FindingId),
    RepairBudgetExhausted {
        max: u32,
    },
    /// The contract requires a review and no approval is recorded on the candidate.
    ReviewRequired,
    /// The contract requires an independent review and every approval on the candidate is
    /// caller-asserted.
    ReviewNotIndependent,
    ReviewRejected {
        reviewer: String,
        findings: usize,
    },
}

pub struct Task {
    contract: TaskContract,
    state: TaskState,
    candidate: Option<Candidate>,
    candidate_identity: Option<CandidateId>,
    receipts: Vec<(VerificationReceipt, ReceiptStatus)>,
    findings: Vec<Finding>,
    consumed_repair_rounds: u32,
    rejected_claims: Vec<String>,
    awaiting_observation: bool,
    next_action: String,
    /// The first candidate the task observed: what a baseline receipt is bound to (K6).
    starting: Option<CandidateId>,
    /// Strict mode: a baseline criterion is met by a failure on the starting candidate (M7).
    reproduction: bool,
    reviews: Vec<ReviewRecord>,
}
impl Task {
    pub fn new(contract: TaskContract) -> Self {
        Self {
            contract,
            state: TaskState::Queued,
            candidate: None,
            candidate_identity: None,
            receipts: Vec::new(),
            findings: Vec::new(),
            consumed_repair_rounds: 0,
            rejected_claims: Vec::new(),
            awaiting_observation: false,
            next_action: String::new(),
            starting: None,
            reproduction: false,
            reviews: Vec::new(),
        }
    }
    fn ensure_observed(&self) -> Result<(), GateError> {
        if self.awaiting_observation {
            Err(GateError::CandidateNotObserved)
        } else {
            Ok(())
        }
    }
    fn require(&self, state: TaskState, event: &'static str) -> Result<(), GateError> {
        self.ensure_observed()?;
        if self.state == state {
            Ok(())
        } else {
            Err(GateError::InvalidTransition {
                from: self.state,
                event,
            })
        }
    }
    pub fn start_preparing(&mut self) -> Result<(), GateError> {
        self.require(TaskState::Queued, "start_preparing")?;
        self.state = TaskState::Preparing;
        Ok(())
    }
    pub fn observe_candidate(&mut self, candidate: Candidate) -> Result<(), GateError> {
        if !matches!(
            self.state,
            TaskState::Preparing
                | TaskState::Verifying
                | TaskState::Repairing
                | TaskState::ReadyForReview
        ) && !self.awaiting_observation
        {
            return Err(GateError::InvalidTransition {
                from: self.state,
                event: "observe_candidate",
            });
        }
        if self.starting.is_none() {
            self.starting = Some(candidate.id.clone());
        }
        if self.candidate_identity.as_ref() != Some(&candidate.id) {
            // K6: a baseline receipt on the starting candidate shows the failure before the
            // fix; changing the code cannot make that observation stale.
            for (receipt, status) in &mut self.receipts {
                let baseline = self
                    .contract
                    .criteria
                    .iter()
                    .any(|c| c.id == receipt.criterion && c.is_baseline());
                if !(baseline && Some(&receipt.candidate) == self.starting.as_ref()) {
                    *status = ReceiptStatus::Stale;
                }
            }
            if self.state == TaskState::ReadyForReview {
                self.state = TaskState::Verifying;
            }
        }
        self.candidate_identity = Some(candidate.id.clone());
        self.candidate = Some(candidate);
        self.awaiting_observation = false;
        if self.state == TaskState::Preparing {
            self.state = TaskState::Verifying;
        }
        Ok(())
    }
    pub fn record_receipt(&mut self, receipt: VerificationReceipt) -> Result<(), GateError> {
        self.ensure_observed()?;
        if !matches!(self.state, TaskState::Verifying | TaskState::ReadyForReview) {
            return Err(GateError::InvalidTransition {
                from: self.state,
                event: "record_receipt",
            });
        }
        if receipt.exit_code.is_none() {
            return Err(GateError::ReceiptMissingExitCode);
        }
        if !receipt.claim_supported {
            return Err(GateError::ReceiptUnsupported);
        }
        let expected = self
            .candidate_identity
            .as_ref()
            .ok_or(GateError::CandidateNotObserved)?;
        let Some(criterion) = self
            .contract
            .criteria
            .iter()
            .find(|c| c.id == receipt.criterion)
        else {
            return Err(GateError::UnknownCriterion(receipt.criterion));
        };
        // K6: a baseline may cite the starting candidate after the fix changed the code.
        let baseline_at_start =
            criterion.is_baseline() && Some(&receipt.candidate) == self.starting.as_ref();
        if &receipt.candidate != expected && !baseline_at_start {
            return Err(GateError::ReceiptForOtherCandidate {
                expected: expected.clone(),
                got: receipt.candidate,
            });
        }
        if !criterion.accepts(&receipt.command) {
            return Err(GateError::WrongCommand {
                criterion: criterion.id.clone(),
                cited: receipt.command.trim().to_owned(),
                required: criterion.command.clone(),
            });
        }
        self.receipts.push((receipt, ReceiptStatus::Current));
        Ok(())
    }
    /// A review of the current candidate. Under an active review policy a rejection takes a task
    /// waiting in review back to verifying, as a candidate change does; it opens no finding.
    pub fn record_review(&mut self, review: ReviewRecord) -> Result<(), GateError> {
        self.ensure_observed()?;
        if self.state.is_terminal() {
            return Err(GateError::InvalidTransition {
                from: self.state,
                event: "record_review",
            });
        }
        let expected = self
            .candidate_identity
            .as_ref()
            .ok_or(GateError::CandidateNotObserved)?;
        if &review.candidate != expected {
            return Err(GateError::ReceiptForOtherCandidate {
                expected: expected.clone(),
                got: review.candidate,
            });
        }
        let policy = &self.contract.review;
        if review.decision == ReviewDecision::Rejected
            && (policy.required || policy.independent)
            && self.state == TaskState::ReadyForReview
        {
            self.state = TaskState::Verifying;
        }
        self.reviews.push(review);
        Ok(())
    }
    /// Restore the stored reviews after `resume`, before receipts are replayed; no transition.
    pub fn set_reviews(&mut self, reviews: Vec<ReviewRecord>) {
        self.reviews = reviews;
    }
    pub fn reviews(&self) -> &[ReviewRecord] {
        &self.reviews
    }
    pub fn contract(&self) -> &TaskContract {
        &self.contract
    }
    pub fn record_worker_claim(&mut self, text: &str) -> Result<(), GateError> {
        self.ensure_observed()?;
        self.rejected_claims.push(text.to_owned());
        Ok(())
    }
    pub fn raise_finding(&mut self, finding: Finding) -> Result<(), GateError> {
        self.ensure_observed()?;
        if self.state.is_terminal() {
            return Err(GateError::InvalidTransition {
                from: self.state,
                event: "raise_finding",
            });
        }
        self.findings.push(finding);
        Ok(())
    }
    pub fn resolve_finding(&mut self, id: &FindingId) -> Result<(), GateError> {
        self.ensure_observed()?;
        if self.state.is_terminal() {
            return Err(GateError::InvalidTransition {
                from: self.state,
                event: "resolve_finding",
            });
        }
        let finding = self
            .findings
            .iter_mut()
            .find(|f| &f.id == id)
            .ok_or_else(|| GateError::UnknownFinding(id.clone()))?;
        finding.disposition = Disposition::Resolved;
        Ok(())
    }
    pub fn request_repair(&mut self) -> Result<(), GateError> {
        self.ensure_observed()?;
        if !matches!(self.state, TaskState::Verifying | TaskState::ReadyForReview) {
            return Err(GateError::InvalidTransition {
                from: self.state,
                event: "request_repair",
            });
        }
        let max = self.contract.budget.max_repair_rounds;
        if self.consumed_repair_rounds >= max {
            self.state = TaskState::BudgetExhausted;
            return Err(GateError::RepairBudgetExhausted { max });
        }
        self.consumed_repair_rounds += 1;
        self.state = TaskState::Repairing;
        Ok(())
    }
    pub fn finish_repair(&mut self) -> Result<(), GateError> {
        self.require(TaskState::Repairing, "finish_repair")?;
        self.state = TaskState::Verifying;
        Ok(())
    }
    pub fn try_accept(&mut self) -> Result<(), GateError> {
        self.require(TaskState::Verifying, "try_accept")?;
        let candidate = self
            .candidate
            .as_ref()
            .ok_or(GateError::CandidateNotObserved)?;
        for criterion in self.contract.criteria.iter().filter(|c| c.mandatory) {
            let receipt = self
                .receipts
                .iter()
                .rev()
                .find(|(r, s)| r.criterion == criterion.id && *s == ReceiptStatus::Current);
            if self.reproduction && criterion.is_baseline() {
                match receipt {
                    Some((r, _))
                        if Some(&r.candidate) == self.starting.as_ref()
                            && r.exit_code.is_some_and(|code| code != 0) => {}
                    Some(_) => return Err(GateError::NotReproduced(criterion.id.clone())),
                    None => return Err(GateError::MissingReceipt(criterion.id.clone())),
                }
                continue;
            }
            match receipt {
                Some((r, _)) => match r.exit_code {
                    Some(0) => (),
                    Some(exit_code) => {
                        return Err(GateError::FailingReceipt {
                            criterion: criterion.id.clone(),
                            exit_code,
                        });
                    }
                    None => return Err(GateError::ReceiptMissingExitCode),
                },
                None if self
                    .receipts
                    .iter()
                    .any(|(r, _)| r.criterion == criterion.id) =>
                {
                    return Err(GateError::StaleReceipt(criterion.id.clone()));
                }
                None => return Err(GateError::MissingReceipt(criterion.id.clone())),
            }
        }
        let outside: Vec<_> = candidate
            .changed_paths
            .iter()
            .filter(|path| {
                !self
                    .contract
                    .scope
                    .iter()
                    .any(|scope| scope_contains(scope, path))
            })
            .cloned()
            .collect();
        if !outside.is_empty() {
            return Err(GateError::OutOfScope(outside));
        }
        if let Some(f) = self.findings.iter().find(|f| {
            f.disposition == Disposition::Open
                && matches!(f.severity, Severity::High | Severity::Critical)
        }) {
            return Err(GateError::OpenFinding(f.id.clone()));
        }
        review_met(&self.contract.review, &self.reviews, &candidate.id)?;
        self.state = TaskState::ReadyForReview;
        Ok(())
    }
    pub fn complete(&mut self) -> Result<(), GateError> {
        self.require(TaskState::ReadyForReview, "complete")?;
        self.state = TaskState::Verifying;
        if let Err(error) = self.try_accept() {
            self.state = TaskState::ReadyForReview;
            return Err(error);
        }
        self.state = TaskState::Completed;
        Ok(())
    }
    fn stop(
        &mut self,
        state: TaskState,
        event: &'static str,
        reason: String,
    ) -> Result<(), GateError> {
        if self.state.is_terminal() {
            return Err(GateError::InvalidTransition {
                from: self.state,
                event,
            });
        }
        self.state = state;
        self.next_action = reason;
        Ok(())
    }
    pub fn block(&mut self, reason: &str) -> Result<(), GateError> {
        self.stop(TaskState::Blocked, "block", reason.to_owned())
    }
    pub fn fail(&mut self, reason: &str) -> Result<(), GateError> {
        self.stop(TaskState::Failed, "fail", reason.to_owned())
    }
    pub fn cancel(&mut self) -> Result<(), GateError> {
        self.stop(TaskState::Cancelled, "cancel", String::new())
    }
    /// A task limit was reached: no further work. Like `block`, this needs no observed
    /// candidate, and a terminal task (a completed one included) never changes.
    pub fn exhaust_limit(&mut self, reached: LimitReached) -> Result<(), GateError> {
        let what = match reached.kind {
            LimitKind::Turns => "turns",
            LimitKind::WallClock => "seconds",
        };
        self.stop(
            TaskState::BudgetExhausted,
            "exhaust_limit",
            format!(
                "task limit reached: {} of {} {what}",
                reached.used, reached.max
            ),
        )
    }
    pub fn checkpoint(&self) -> Checkpoint {
        Checkpoint {
            state: self.state,
            candidate: self.candidate_identity.clone(),
            open_findings: self.open_findings(),
            consumed_repair_rounds: self.consumed_repair_rounds,
            next_action: self.next_action.clone(),
        }
    }
    pub fn resume(contract: TaskContract, checkpoint: Checkpoint) -> Self {
        Self {
            contract,
            state: checkpoint.state,
            candidate: None,
            candidate_identity: checkpoint.candidate,
            receipts: Vec::new(),
            findings: checkpoint.open_findings,
            consumed_repair_rounds: checkpoint.consumed_repair_rounds,
            rejected_claims: Vec::new(),
            awaiting_observation: true,
            next_action: checkpoint.next_action,
            starting: None,
            reproduction: false,
            reviews: Vec::new(),
        }
    }
    /// The first candidate this task observed; a resumed task gets it back from its store.
    pub fn starting_candidate(&self) -> Option<&CandidateId> {
        self.starting.as_ref()
    }
    /// Restore the starting candidate after `resume`, before receipts are replayed.
    pub fn set_starting_candidate(&mut self, id: CandidateId) {
        self.starting.get_or_insert(id);
    }
    /// Judge baseline criteria as the reported failure reproduced on the starting candidate
    /// (exit code not 0) instead of as an ordinary passing check (M7, strict mode).
    pub fn require_reproduction(&mut self) {
        self.reproduction = true;
    }
    pub fn requires_reproduction(&self) -> bool {
        self.reproduction
    }
    pub fn state(&self) -> TaskState {
        self.state
    }
    pub fn candidate(&self) -> Option<&CandidateId> {
        self.candidate_identity.as_ref()
    }
    pub fn open_findings(&self) -> Vec<Finding> {
        self.findings
            .iter()
            .filter(|f| f.disposition == Disposition::Open)
            .cloned()
            .collect()
    }
    pub fn consumed_repair_rounds(&self) -> u32 {
        self.consumed_repair_rounds
    }
    pub fn rejected_claims(&self) -> &[String] {
        &self.rejected_claims
    }
    pub fn receipt_status(&self, id: &CriterionId) -> Option<ReceiptStatus> {
        self.receipts
            .iter()
            .rev()
            .find(|(r, _)| &r.criterion == id)
            .map(|(_, s)| *s)
    }
}
