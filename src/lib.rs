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
    pub forbidden_effects: Vec<String>,
    pub scope: Vec<String>,
    pub budget: Budget,
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
    UnknownFinding(FindingId),
    RepairBudgetExhausted {
        max: u32,
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
            TaskState::Preparing | TaskState::Verifying | TaskState::Repairing
        ) && !self.awaiting_observation
        {
            return Err(GateError::InvalidTransition {
                from: self.state,
                event: "observe_candidate",
            });
        }
        if self.candidate_identity.as_ref() != Some(&candidate.id) {
            for (_, status) in &mut self.receipts {
                *status = ReceiptStatus::Stale;
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
        if &receipt.candidate != expected {
            return Err(GateError::ReceiptForOtherCandidate {
                expected: expected.clone(),
                got: receipt.candidate,
            });
        }
        if !self
            .contract
            .criteria
            .iter()
            .any(|c| c.id == receipt.criterion)
        {
            return Err(GateError::UnknownCriterion(receipt.criterion));
        }
        self.receipts.push((receipt, ReceiptStatus::Current));
        Ok(())
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
        self.require(TaskState::Verifying, "request_repair")?;
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
                    .any(|prefix| path.starts_with(prefix))
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
        self.ensure_observed()?;
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
        }
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
