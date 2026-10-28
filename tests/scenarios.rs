use coding_supervisor::*;

struct FakeRunner;
impl FakeRunner {
    fn receipt(id: &str, candidate: &CandidateId, exit_code: Option<i32>) -> VerificationReceipt {
        VerificationReceipt {
            criterion: CriterionId(id.into()),
            candidate: candidate.clone(),
            command: "fake".into(),
            cwd: ".".into(),
            exit_code,
            output_ref: "fake-output".into(),
            claim_supported: true,
            started_at: 1,
            finished_at: 2,
        }
    }
}
fn contract() -> TaskContract {
    TaskContract {
        id: TaskId("task".into()),
        goal: "goal".into(),
        criteria: ["C1", "C2", "C3", "C4"]
            .into_iter()
            .map(|id| Criterion {
                id: CriterionId(id.into()),
                check: CheckName(id.into()),
                mandatory: true,
            })
            .collect(),
        forbidden_effects: vec![],
        scope: vec!["src/".into(), "tests/".into()],
        budget: Budget {
            max_repair_rounds: 2,
        },
    }
}
fn candidate(digest: &str) -> Candidate {
    Candidate {
        id: CandidateId {
            base_commit: "base".into(),
            dirty_digest: digest.into(),
        },
        provenance: "fake".into(),
        changed_paths: vec!["src/lib.rs".into()],
    }
}
fn task() -> Task {
    let mut t = Task::new(contract());
    t.start_preparing().unwrap();
    t.observe_candidate(candidate("one")).unwrap();
    t
}
fn pass(t: &mut Task, id: &str) {
    let c = t.candidate().unwrap().clone();
    t.record_receipt(FakeRunner::receipt(id, &c, Some(0)))
        .unwrap();
}
fn finding() -> Finding {
    Finding {
        id: FindingId("f".into()),
        candidate: candidate("one").id,
        severity: Severity::High,
        location: "src/lib.rs".into(),
        evidence_ref: "e".into(),
        disposition: Disposition::Open,
    }
}

#[test]
fn a18_self_reported_pass_is_not_accepted() {
    let mut t = task();
    t.record_worker_claim("tests passed").unwrap();
    assert_eq!(
        t.try_accept(),
        Err(GateError::MissingReceipt(CriterionId("C1".into())))
    );
    assert_eq!(t.state(), TaskState::Verifying);
    assert_eq!(t.rejected_claims(), &["tests passed"]);
}
#[test]
fn a19_candidate_change_makes_receipt_stale() {
    let mut t = task();
    pass(&mut t, "C1");
    t.observe_candidate(candidate("two")).unwrap();
    assert_eq!(
        t.receipt_status(&CriterionId("C1".into())),
        Some(ReceiptStatus::Stale)
    );
    assert_eq!(
        t.try_accept(),
        Err(GateError::StaleReceipt(CriterionId("C1".into())))
    );
    assert_eq!(t.state(), TaskState::Verifying);
    for c in ["C1", "C2", "C3", "C4"] {
        pass(&mut t, c);
    }
    t.try_accept().unwrap();
    assert_eq!(t.state(), TaskState::ReadyForReview);
}
#[test]
fn a20_resume_restores_contract_budget_findings_candidate() {
    let mut t = task();
    t.raise_finding(finding()).unwrap();
    t.request_repair().unwrap();
    t.finish_repair().unwrap();
    pass(&mut t, "C1");
    let checkpoint = t.checkpoint();
    let mut resumed = Task::resume(contract(), checkpoint);
    assert_eq!(resumed.state(), TaskState::Verifying);
    assert_eq!(resumed.candidate(), Some(&candidate("one").id));
    assert_eq!(resumed.consumed_repair_rounds(), 1);
    assert_eq!(resumed.open_findings(), vec![finding()]);
    assert_eq!(resumed.try_accept(), Err(GateError::CandidateNotObserved));
    resumed.observe_candidate(candidate("one")).unwrap();
    assert_eq!(
        resumed.try_accept(),
        Err(GateError::MissingReceipt(CriterionId("C1".into())))
    );
    assert_eq!(resumed.state(), TaskState::Verifying);
}
#[test]
fn c1_c4_all_four_current_receipts_required() {
    let mut t = task();
    for c in ["C1", "C2", "C3"] {
        pass(&mut t, c);
    }
    assert_eq!(
        t.try_accept(),
        Err(GateError::MissingReceipt(CriterionId("C4".into())))
    );
    assert_eq!(t.state(), TaskState::Verifying);
    pass(&mut t, "C4");
    t.try_accept().unwrap();
    assert_eq!(t.state(), TaskState::ReadyForReview);
    t.complete().unwrap();
    assert_eq!(t.state(), TaskState::Completed);
}

#[test]
fn resumed_review_requires_fresh_receipts() {
    let mut t = task();
    for c in ["C1", "C2", "C3", "C4"] {
        pass(&mut t, c);
    }
    t.try_accept().unwrap();
    let mut t = Task::resume(contract(), t.checkpoint());
    t.observe_candidate(candidate("one")).unwrap();
    assert_eq!(
        t.complete(),
        Err(GateError::MissingReceipt(CriterionId("C1".into())))
    );
    assert_eq!(t.state(), TaskState::ReadyForReview);
    for c in ["C1", "C2", "C3", "C4"] {
        pass(&mut t, c);
    }
    t.complete().unwrap();
    assert_eq!(t.state(), TaskState::Completed);
}
#[test]
fn r23_repair_cap_reached_never_completes() {
    let mut t = task();
    t.raise_finding(finding()).unwrap();
    for _ in 0..2 {
        t.request_repair().unwrap();
        t.finish_repair().unwrap();
    }
    assert_eq!(
        t.request_repair(),
        Err(GateError::RepairBudgetExhausted { max: 2 })
    );
    assert_eq!(t.state(), TaskState::BudgetExhausted);
    assert_ne!(t.state(), TaskState::Completed);
    assert_eq!(t.open_findings(), vec![finding()]);
}
#[test]
fn r26_mandatory_regression_blocks_despite_aggregate() {
    let mut c = contract();
    c.criteria.push(Criterion {
        id: CriterionId("optional".into()),
        check: CheckName("optional".into()),
        mandatory: false,
    });
    let mut t = Task::new(c);
    t.start_preparing().unwrap();
    t.observe_candidate(candidate("one")).unwrap();
    for id in ["C1", "C2", "C4", "optional"] {
        pass(&mut t, id);
    }
    t.record_receipt(FakeRunner::receipt("C3", t.candidate().unwrap(), Some(1)))
        .unwrap();
    assert_eq!(
        t.try_accept(),
        Err(GateError::FailingReceipt {
            criterion: CriterionId("C3".into()),
            exit_code: 1
        })
    );
    assert_eq!(t.state(), TaskState::Verifying);
}
#[test]
fn r28_out_of_scope_change_fails_c4() {
    let mut t = task();
    let mut c = candidate("two");
    c.changed_paths.push("docs/out.md".into());
    t.observe_candidate(c).unwrap();
    for id in ["C1", "C2", "C3", "C4"] {
        pass(&mut t, id);
    }
    assert_eq!(
        t.try_accept(),
        Err(GateError::OutOfScope(vec!["docs/out.md".into()]))
    );
    assert_eq!(t.state(), TaskState::Verifying);
}
#[test]
fn r35_would_run_counts_as_not_run() {
    let mut t = task();
    t.record_worker_claim("would run cargo test").unwrap();
    assert_eq!(
        t.try_accept(),
        Err(GateError::MissingReceipt(CriterionId("C1".into())))
    );
    assert_eq!(t.state(), TaskState::Verifying);
}
#[test]
fn x1_receipt_without_exit_code_rejected() {
    let mut t = task();
    let r = FakeRunner::receipt("C1", t.candidate().unwrap(), None);
    assert_eq!(t.record_receipt(r), Err(GateError::ReceiptMissingExitCode));
    assert_eq!(t.state(), TaskState::Verifying);
}
#[test]
fn x2_receipt_for_other_candidate_rejected() {
    let mut t = task();
    let expected = t.candidate().unwrap().clone();
    let got = candidate("other").id;
    assert_eq!(
        t.record_receipt(FakeRunner::receipt("C1", &got, Some(0))),
        Err(GateError::ReceiptForOtherCandidate { expected, got })
    );
    assert_eq!(t.state(), TaskState::Verifying);
}
