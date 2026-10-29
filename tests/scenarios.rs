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
                command: vec!["fake".into()],
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

#[derive(Debug, PartialEq, Clone)]
struct Snap {
    state: TaskState,
    candidate: Option<CandidateId>,
    receipts: Vec<Option<ReceiptStatus>>,
    findings: Vec<Finding>,
    rounds: u32,
}
fn snap(t: &Task) -> Snap {
    Snap {
        state: t.state(),
        candidate: t.candidate().cloned(),
        receipts: ["C1", "C2", "C3", "C4"]
            .iter()
            .map(|id| t.receipt_status(&CriterionId((*id).into())))
            .collect(),
        findings: t.open_findings(),
        rounds: t.consumed_repair_rounds(),
    }
}
fn assert_unchanged(before: &Snap, t: &Task) {
    assert_eq!(before, &snap(t));
}
fn in_queued() -> Task {
    Task::new(contract())
}
fn in_preparing() -> Task {
    let mut t = in_queued();
    t.start_preparing().unwrap();
    t
}
fn in_verifying() -> Task {
    task()
}
fn in_verifying_passed() -> Task {
    let mut t = task();
    for c in ["C1", "C2", "C3", "C4"] {
        pass(&mut t, c);
    }
    t
}
fn in_ready() -> Task {
    let mut t = in_verifying_passed();
    t.try_accept().unwrap();
    t
}
fn in_repairing() -> Task {
    let mut t = task();
    t.request_repair().unwrap();
    t
}
fn in_terminal(state: TaskState) -> Task {
    let mut t = if state == TaskState::Completed {
        in_ready()
    } else {
        task()
    };
    match state {
        TaskState::Blocked => t.block("reason").unwrap(),
        TaskState::Failed => t.fail("reason").unwrap(),
        TaskState::Cancelled => t.cancel().unwrap(),
        TaskState::Completed => t.complete().unwrap(),
        TaskState::BudgetExhausted => {
            for _ in 0..2 {
                t.request_repair().unwrap();
                t.finish_repair().unwrap();
            }
            assert_eq!(
                t.request_repair(),
                Err(GateError::RepairBudgetExhausted { max: 2 })
            );
        }
        _ => panic!("not terminal"),
    }
    t
}
fn resumed_unobserved(t: Task) -> Task {
    Task::resume(contract(), t.checkpoint())
}
fn all_states() -> Vec<Task> {
    let mut tasks = vec![
        in_queued(),
        in_preparing(),
        in_verifying(),
        in_ready(),
        in_repairing(),
    ];
    for s in [
        TaskState::Blocked,
        TaskState::BudgetExhausted,
        TaskState::Failed,
        TaskState::Cancelled,
        TaskState::Completed,
    ] {
        tasks.push(in_terminal(s));
    }
    tasks
}
fn invalid(t: &Task, event: &'static str) -> GateError {
    GateError::InvalidTransition {
        from: t.state(),
        event,
    }
}
fn receipt(t: &Task) -> VerificationReceipt {
    FakeRunner::receipt(
        "C1",
        &t.candidate().cloned().unwrap_or(candidate("one").id),
        Some(0),
    )
}
fn low_finding() -> Finding {
    let mut f = finding();
    f.severity = Severity::Low;
    f
}
fn exhausted_ready() -> Task {
    let mut t = in_verifying_passed();
    for _ in 0..2 {
        t.request_repair().unwrap();
        t.finish_repair().unwrap();
    }
    t.try_accept().unwrap();
    t
}
#[test]
fn review_high_finding_repaired_resolved_then_completed() {
    let mut t = in_ready();
    t.raise_finding(finding()).unwrap();
    assert_eq!(t.open_findings(), vec![finding()]);
    assert_eq!(t.request_repair(), Ok(()));
    assert_eq!(t.state(), TaskState::Repairing);
    assert_eq!(t.consumed_repair_rounds(), 1);
    assert_eq!(t.finish_repair(), Ok(()));
    assert_eq!(t.state(), TaskState::Verifying);
    assert_eq!(t.resolve_finding(&finding().id), Ok(()));
    assert!(t.open_findings().is_empty());
    for c in ["C1", "C2", "C3", "C4"] {
        pass(&mut t, c);
    }
    assert_eq!(t.try_accept(), Ok(()));
    assert_eq!(t.complete(), Ok(()));
    assert_eq!(t.state(), TaskState::Completed);
    assert!(t.open_findings().is_empty());
}
#[test]
fn matrix_start_preparing() {
    for mut t in all_states() {
        let before = snap(&t);
        let expected = if t.state() == TaskState::Queued {
            Ok(())
        } else {
            Err(invalid(&t, "start_preparing"))
        };
        assert_eq!(t.start_preparing(), expected);
        if expected.is_ok() {
            assert_eq!(t.state(), TaskState::Preparing);
        } else {
            assert_unchanged(&before, &t);
        }
    }
    let mut t = resumed_unobserved(in_queued());
    let before = snap(&t);
    assert_eq!(t.start_preparing(), Err(GateError::CandidateNotObserved));
    assert_unchanged(&before, &t);
}
#[test]
fn matrix_observe_candidate_same() {
    for mut t in [
        in_verifying_passed(),
        {
            let mut t = in_verifying_passed();
            t.request_repair().unwrap();
            t
        },
        in_ready(),
    ] {
        let before = snap(&t);
        assert_eq!(t.observe_candidate(candidate("one")), Ok(()));
        assert_unchanged(&before, &t);
    }
    let mut t = in_queued();
    let before = snap(&t);
    assert_eq!(
        t.observe_candidate(candidate("one")),
        Err(invalid(&t, "observe_candidate"))
    );
    assert_unchanged(&before, &t);
    let mut t = in_preparing();
    assert_eq!(t.observe_candidate(candidate("one")), Ok(()));
    assert_eq!(t.state(), TaskState::Verifying);
    assert_eq!(t.candidate(), Some(&candidate("one").id));
    for mut t in all_states().into_iter().filter(|t| t.state().is_terminal()) {
        let before = snap(&t);
        assert_eq!(
            t.observe_candidate(candidate("one")),
            Err(invalid(&t, "observe_candidate"))
        );
        assert_unchanged(&before, &t);
    }
    for t in all_states() {
        let mut t = resumed_unobserved(t);
        let old = t.state();
        assert_eq!(t.observe_candidate(candidate("one")), Ok(()));
        assert_eq!(
            t.state(),
            if old == TaskState::Preparing {
                TaskState::Verifying
            } else {
                old
            }
        );
        assert_eq!(t.candidate(), Some(&candidate("one").id));
        assert_eq!(t.record_worker_claim("observed"), Ok(()));
    }
}
#[test]
fn matrix_observe_candidate_new() {
    for mut t in [
        in_verifying_passed(),
        {
            let mut t = in_verifying_passed();
            t.request_repair().unwrap();
            t
        },
        in_ready(),
    ] {
        let old = t.state();
        assert_eq!(t.observe_candidate(candidate("two")), Ok(()));
        assert_eq!(
            t.state(),
            if old == TaskState::ReadyForReview {
                TaskState::Verifying
            } else {
                old
            }
        );
        assert_eq!(t.candidate(), Some(&candidate("two").id));
        assert_eq!(snap(&t).receipts, vec![Some(ReceiptStatus::Stale); 4]);
    }
    for t in all_states() {
        let mut t = resumed_unobserved(t);
        let old = t.state();
        assert_eq!(t.observe_candidate(candidate("two")), Ok(()));
        assert_eq!(
            t.state(),
            if matches!(old, TaskState::Preparing | TaskState::ReadyForReview) {
                TaskState::Verifying
            } else {
                old
            }
        );
        assert_eq!(t.candidate(), Some(&candidate("two").id));
        assert_eq!(t.record_worker_claim("observed"), Ok(()));
    }
    let mut t = task();
    t.raise_finding(low_finding()).unwrap();
    assert_eq!(t.observe_candidate(candidate("two")), Ok(()));
    assert_eq!(t.open_findings(), vec![low_finding()]);
}
#[test]
fn matrix_record_receipt() {
    for mut t in all_states() {
        let before = snap(&t);
        let r = receipt(&t);
        let legal = matches!(t.state(), TaskState::Verifying | TaskState::ReadyForReview);
        let expected = if legal {
            Ok(())
        } else {
            Err(invalid(&t, "record_receipt"))
        };
        assert_eq!(t.record_receipt(r), expected);
        if legal {
            assert_eq!(t.state(), before.state);
            assert_eq!(
                t.receipt_status(&CriterionId("C1".into())),
                Some(ReceiptStatus::Current)
            );
        } else {
            assert_unchanged(&before, &t);
        }
    }
    for make in [in_verifying as fn() -> Task, in_ready] {
        for n in 0..5 {
            let mut t = make();
            let before = snap(&t);
            let mut r = receipt(&t);
            let error = match n {
                0 => {
                    r.exit_code = None;
                    GateError::ReceiptMissingExitCode
                }
                1 => {
                    r.claim_supported = false;
                    GateError::ReceiptUnsupported
                }
                2 => {
                    r.candidate = candidate("two").id;
                    GateError::ReceiptForOtherCandidate {
                        expected: candidate("one").id,
                        got: candidate("two").id,
                    }
                }
                3 => {
                    r.criterion = CriterionId("C9".into());
                    GateError::UnknownCriterion(CriterionId("C9".into()))
                }
                // specified by PROPOSAL gap 1
                _ => {
                    r.command = "other".into();
                    GateError::WrongCommand {
                        criterion: CriterionId("C1".into()),
                        cited: "other".into(),
                        required: vec!["fake".into()],
                    }
                }
            };
            assert_eq!(t.record_receipt(r), Err(error));
            assert_unchanged(&before, &t);
        }
    }
    for t in all_states() {
        let mut t = resumed_unobserved(t);
        let before = snap(&t);
        let r = receipt(&t);
        assert_eq!(t.record_receipt(r), Err(GateError::CandidateNotObserved));
        assert_unchanged(&before, &t);
    }
}
#[test]
fn matrix_record_worker_claim() {
    for mut t in all_states() {
        let before = snap(&t);
        assert_eq!(t.record_worker_claim("claim"), Ok(()));
        assert_unchanged(&before, &t);
        assert_eq!(t.rejected_claims(), &["claim"]);
    }
    for t in all_states() {
        let mut t = resumed_unobserved(t);
        let before = snap(&t);
        assert_eq!(
            t.record_worker_claim("claim"),
            Err(GateError::CandidateNotObserved)
        );
        assert_unchanged(&before, &t);
        assert!(t.rejected_claims().is_empty());
    }
}
#[test]
fn matrix_raise_finding() {
    for mut t in all_states() {
        let before = snap(&t);
        let expected = if t.state().is_terminal() {
            Err(invalid(&t, "raise_finding"))
        } else {
            Ok(())
        };
        assert_eq!(t.raise_finding(finding()), expected);
        if expected.is_ok() {
            assert_eq!(t.state(), before.state);
            assert_eq!(t.open_findings(), vec![finding()]);
        } else {
            assert_unchanged(&before, &t);
        }
    }
    for t in all_states() {
        let mut t = resumed_unobserved(t);
        let before = snap(&t);
        assert_eq!(
            t.raise_finding(finding()),
            Err(GateError::CandidateNotObserved)
        );
        assert_unchanged(&before, &t);
    }
}
#[test]
fn matrix_resolve_finding() {
    for mut t in all_states() {
        if !t.state().is_terminal() {
            let before = snap(&t);
            assert_eq!(
                t.resolve_finding(&finding().id),
                Err(GateError::UnknownFinding(finding().id))
            );
            assert_unchanged(&before, &t);
            t.raise_finding(finding()).unwrap();
            assert_eq!(t.resolve_finding(&finding().id), Ok(()));
            assert_eq!(t.state(), before.state);
            assert!(t.open_findings().is_empty());
        } else {
            let before = snap(&t);
            assert_eq!(
                t.resolve_finding(&finding().id),
                Err(invalid(&t, "resolve_finding"))
            );
            assert_unchanged(&before, &t);
        }
    }
    for t in all_states() {
        let mut t = resumed_unobserved(t);
        let before = snap(&t);
        assert_eq!(
            t.resolve_finding(&finding().id),
            Err(GateError::CandidateNotObserved)
        );
        assert_unchanged(&before, &t);
    }
}
#[test]
fn matrix_request_repair() {
    for mut t in all_states() {
        let before = snap(&t);
        let legal = matches!(t.state(), TaskState::Verifying | TaskState::ReadyForReview);
        let expected = if legal {
            Ok(())
        } else {
            Err(invalid(&t, "request_repair"))
        };
        assert_eq!(t.request_repair(), expected);
        if legal {
            assert_eq!(t.state(), TaskState::Repairing);
            assert_eq!(t.consumed_repair_rounds(), before.rounds + 1);
        } else {
            assert_unchanged(&before, &t);
        }
    }
    let ready = exhausted_ready();
    let mut verifying = exhausted_ready();
    verifying.observe_candidate(candidate("two")).unwrap();
    for mut t in [verifying, ready] {
        t.raise_finding(low_finding()).unwrap();
        let before = snap(&t);
        assert_eq!(
            t.request_repair(),
            Err(GateError::RepairBudgetExhausted { max: 2 })
        );
        let mut expected = before;
        expected.state = TaskState::BudgetExhausted;
        assert_unchanged(&expected, &t);
    }
    for t in all_states() {
        let mut t = resumed_unobserved(t);
        let before = snap(&t);
        assert_eq!(t.request_repair(), Err(GateError::CandidateNotObserved));
        assert_unchanged(&before, &t);
    }
}
#[test]
fn matrix_finish_repair() {
    for mut t in all_states() {
        let before = snap(&t);
        let expected = if t.state() == TaskState::Repairing {
            Ok(())
        } else {
            Err(invalid(&t, "finish_repair"))
        };
        assert_eq!(t.finish_repair(), expected);
        if expected.is_ok() {
            assert_eq!(t.state(), TaskState::Verifying);
        } else {
            assert_unchanged(&before, &t);
        }
    }
    for t in all_states() {
        let mut t = resumed_unobserved(t);
        let before = snap(&t);
        assert_eq!(t.finish_repair(), Err(GateError::CandidateNotObserved));
        assert_unchanged(&before, &t);
    }
}
#[test]
fn matrix_try_accept() {
    for mut t in all_states()
        .into_iter()
        .filter(|t| t.state() != TaskState::Verifying)
    {
        let before = snap(&t);
        assert_eq!(t.try_accept(), Err(invalid(&t, "try_accept")));
        assert_unchanged(&before, &t);
    }
    for t in all_states() {
        let mut t = resumed_unobserved(t);
        let before = snap(&t);
        assert_eq!(t.try_accept(), Err(GateError::CandidateNotObserved));
        assert_unchanged(&before, &t);
    }
    let mut t = in_verifying_passed();
    assert_eq!(t.try_accept(), Ok(()));
    assert_eq!(t.state(), TaskState::ReadyForReview);
    let mut t = task();
    let before = snap(&t);
    assert_eq!(
        t.try_accept(),
        Err(GateError::MissingReceipt(CriterionId("C1".into())))
    );
    assert_unchanged(&before, &t);
    let mut t = in_verifying_passed();
    t.observe_candidate(candidate("two")).unwrap();
    let before = snap(&t);
    assert_eq!(
        t.try_accept(),
        Err(GateError::StaleReceipt(CriterionId("C1".into())))
    );
    assert_unchanged(&before, &t);
    let mut t = in_verifying_passed();
    let r = FakeRunner::receipt("C1", &candidate("one").id, Some(1));
    t.record_receipt(r).unwrap();
    let before = snap(&t);
    assert_eq!(
        t.try_accept(),
        Err(GateError::FailingReceipt {
            criterion: CriterionId("C1".into()),
            exit_code: 1
        })
    );
    assert_unchanged(&before, &t);
    let mut t = in_verifying_passed();
    let mut c = candidate("one");
    c.changed_paths = vec!["other".into()];
    t.observe_candidate(c).unwrap();
    let before = snap(&t);
    assert_eq!(
        t.try_accept(),
        Err(GateError::OutOfScope(vec!["other".into()]))
    );
    assert_unchanged(&before, &t);
    for severity in [
        Severity::High,
        Severity::Critical,
        Severity::Low,
        Severity::Medium,
    ] {
        let mut t = in_verifying_passed();
        let mut f = finding();
        f.severity = severity;
        t.raise_finding(f.clone()).unwrap();
        let before = snap(&t);
        let expected = if matches!(severity, Severity::High | Severity::Critical) {
            Err(GateError::OpenFinding(f.id))
        } else {
            Ok(())
        };
        assert_eq!(t.try_accept(), expected);
        if expected.is_err() {
            assert_unchanged(&before, &t);
        } else {
            assert_eq!(t.state(), TaskState::ReadyForReview);
        }
    }
    let mut t = in_verifying_passed();
    t.raise_finding(finding()).unwrap();
    t.resolve_finding(&finding().id).unwrap();
    assert_eq!(t.try_accept(), Ok(()));
    assert_eq!(t.state(), TaskState::ReadyForReview);
    let mut c = contract();
    c.criteria.push(Criterion {
        id: CriterionId("C5".into()),
        check: CheckName("C5".into()),
        mandatory: false,
        command: vec!["fake".into()],
    });
    for optional_failure in [false, true] {
        let mut t = Task::new(c.clone());
        t.start_preparing().unwrap();
        t.observe_candidate(candidate("one")).unwrap();
        for c in ["C1", "C2", "C3", "C4"] {
            pass(&mut t, c);
        }
        if optional_failure {
            t.record_receipt(FakeRunner::receipt("C5", &candidate("one").id, Some(1)))
                .unwrap();
        }
        assert_eq!(t.try_accept(), Ok(()));
        assert_eq!(t.state(), TaskState::ReadyForReview);
    }
}
#[test]
fn matrix_complete() {
    for mut t in all_states()
        .into_iter()
        .filter(|t| t.state() != TaskState::ReadyForReview)
    {
        let before = snap(&t);
        assert_eq!(t.complete(), Err(invalid(&t, "complete")));
        assert_unchanged(&before, &t);
    }
    for t in all_states() {
        let mut t = resumed_unobserved(t);
        let before = snap(&t);
        assert_eq!(t.complete(), Err(GateError::CandidateNotObserved));
        assert_unchanged(&before, &t);
    }
    let mut t = in_ready();
    assert_eq!(t.complete(), Ok(()));
    assert_eq!(t.state(), TaskState::Completed);
    let mut t = resumed_unobserved(in_ready());
    t.observe_candidate(candidate("one")).unwrap();
    let before = snap(&t);
    assert_eq!(
        t.complete(),
        Err(GateError::MissingReceipt(CriterionId("C1".into())))
    );
    assert_unchanged(&before, &t);
    let mut t = in_ready();
    t.raise_finding(finding()).unwrap();
    let before = snap(&t);
    assert_eq!(t.complete(), Err(GateError::OpenFinding(finding().id)));
    assert_unchanged(&before, &t);
    for resolved in [false, true] {
        let mut t = in_ready();
        let f = if resolved { finding() } else { low_finding() };
        t.raise_finding(f.clone()).unwrap();
        if resolved {
            t.resolve_finding(&f.id).unwrap();
        }
        assert_eq!(t.complete(), Ok(()));
        assert_eq!(t.state(), TaskState::Completed);
    }
}
fn matrix_stop(
    event: &'static str,
    target: TaskState,
    action: fn(&mut Task) -> Result<(), GateError>,
    reason: &str,
) {
    let mut states = all_states();
    let mut evidence = in_verifying_passed();
    evidence.raise_finding(low_finding()).unwrap();
    states.push(evidence);
    let resumed: Vec<_> = all_states().into_iter().map(resumed_unobserved).collect();
    for mut t in states.into_iter().chain(resumed) {
        let before = snap(&t);
        if t.state().is_terminal() {
            assert_eq!(action(&mut t), Err(invalid(&t, event)));
            assert_unchanged(&before, &t);
        } else {
            assert_eq!(action(&mut t), Ok(()));
            let mut expected = before;
            expected.state = target;
            assert_unchanged(&expected, &t);
            assert_eq!(t.checkpoint().next_action, reason);
        }
    }
}
#[test]
fn matrix_block() {
    matrix_stop("block", TaskState::Blocked, |t| t.block("reason"), "reason");
}
#[test]
fn matrix_fail() {
    matrix_stop("fail", TaskState::Failed, |t| t.fail("reason"), "reason");
}
#[test]
fn matrix_cancel() {
    matrix_stop("cancel", TaskState::Cancelled, Task::cancel, "");
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
        command: vec!["fake".into()],
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
#[test]
fn gap1_receipt_must_cite_a_command_the_contract_names() {
    let mut c = contract();
    c.criteria[1].command = vec!["fake".into(), "fake --release".into()];
    let mut t = Task::new(c);
    t.start_preparing().unwrap();
    t.observe_candidate(candidate("one")).unwrap();
    let before = snap(&t);
    let mut r = FakeRunner::receipt("C2", t.candidate().unwrap(), Some(0));
    r.command = " echo ok ".into();
    assert_eq!(
        t.record_receipt(r.clone()),
        Err(GateError::WrongCommand {
            criterion: CriterionId("C2".into()),
            cited: "echo ok".into(),
            required: vec!["fake".into(), "fake --release".into()],
        })
    );
    assert_unchanged(&before, &t);
    r.command = "  fake --release ".into();
    assert_eq!(t.record_receipt(r), Ok(()));
    assert_eq!(
        t.receipt_status(&CriterionId("C2".into())),
        Some(ReceiptStatus::Current)
    );
}
#[test]
fn illegal_transitions_return_typed_errors_and_preserve_state() {
    let mut t = Task::new(contract());
    let err = |from, event| Err(GateError::InvalidTransition { from, event });
    assert_eq!(t.try_accept(), err(TaskState::Queued, "try_accept"));
    assert_eq!(t.request_repair(), err(TaskState::Queued, "request_repair"));
    assert_eq!(
        t.observe_candidate(candidate("one")),
        err(TaskState::Queued, "observe_candidate")
    );
    assert_eq!(t.state(), TaskState::Queued);
    assert_eq!(t.candidate(), None);

    let mut t = task();
    assert_eq!(
        t.start_preparing(),
        err(TaskState::Verifying, "start_preparing")
    );
    assert_eq!(
        t.finish_repair(),
        err(TaskState::Verifying, "finish_repair")
    );
    assert_eq!(t.complete(), err(TaskState::Verifying, "complete"));
    t.request_repair().unwrap();
    let c = t.candidate().unwrap().clone();
    assert_eq!(
        t.record_receipt(FakeRunner::receipt("C1", &c, Some(0))),
        err(TaskState::Repairing, "record_receipt")
    );
    assert_eq!(t.receipt_status(&CriterionId("C1".into())), None);
    assert_eq!(t.state(), TaskState::Repairing);
    assert_eq!(t.consumed_repair_rounds(), 1);

    t.cancel().unwrap();
    assert_eq!(t.block("x"), err(TaskState::Cancelled, "block"));
    assert_eq!(t.fail("x"), err(TaskState::Cancelled, "fail"));
    assert_eq!(
        t.raise_finding(finding()),
        err(TaskState::Cancelled, "raise_finding")
    );
    assert_eq!(
        t.finish_repair(),
        err(TaskState::Cancelled, "finish_repair")
    );
    assert_eq!(t.state(), TaskState::Cancelled);
    assert_eq!(t.open_findings(), vec![]);
}

#[test]
fn block_fail_cancel_are_terminal_and_refuse_complete() {
    for state in [TaskState::Blocked, TaskState::Failed, TaskState::Cancelled] {
        let mut t = task();
        for c in ["C1", "C2", "C3", "C4"] {
            pass(&mut t, c);
        }
        t.raise_finding(finding()).unwrap();
        match state {
            TaskState::Blocked => t.block("r").unwrap(),
            TaskState::Failed => t.fail("r").unwrap(),
            TaskState::Cancelled => t.cancel().unwrap(),
            _ => unreachable!(),
        }
        assert_eq!(t.state(), state);
        for (event, result) in [
            ("complete", t.complete()),
            ("try_accept", t.try_accept()),
            ("request_repair", t.request_repair()),
        ] {
            assert_eq!(
                result,
                Err(GateError::InvalidTransition { from: state, event })
            );
        }
        assert_eq!(t.state(), state);
        assert_eq!(
            t.receipt_status(&CriterionId("C1".into())),
            Some(ReceiptStatus::Current)
        );
        assert_eq!(t.open_findings(), vec![finding()]);
        assert_eq!(t.consumed_repair_rounds(), 0);
    }
}

#[test]
fn resume_from_repairing_reobserves_before_transition() {
    let mut t = task();
    t.raise_finding(finding()).unwrap();
    t.request_repair().unwrap();
    let mut t = Task::resume(contract(), t.checkpoint());
    assert_eq!(t.state(), TaskState::Repairing);
    assert_eq!(t.consumed_repair_rounds(), 1);
    assert_eq!(t.finish_repair(), Err(GateError::CandidateNotObserved));
    assert_eq!(t.state(), TaskState::Repairing);
    t.observe_candidate(candidate("one")).unwrap();
    t.finish_repair().unwrap();
    assert_eq!(t.state(), TaskState::Verifying);
    assert_eq!(
        t.try_accept(),
        Err(GateError::MissingReceipt(CriterionId("C1".into())))
    );
}

#[test]
fn review_candidate_change_invalidates_receipts_and_reverts_to_verifying() {
    let mut t = task();
    for c in ["C1", "C2", "C3", "C4"] {
        pass(&mut t, c);
    }
    t.try_accept().unwrap();
    assert_eq!(t.state(), TaskState::ReadyForReview);
    t.observe_candidate(candidate("one")).unwrap();
    assert_eq!(t.state(), TaskState::ReadyForReview);
    assert_eq!(
        t.receipt_status(&CriterionId("C1".into())),
        Some(ReceiptStatus::Current)
    );
    t.observe_candidate(candidate("two")).unwrap();
    assert_eq!(t.state(), TaskState::Verifying);
    assert_eq!(
        t.receipt_status(&CriterionId("C1".into())),
        Some(ReceiptStatus::Stale)
    );
    assert_eq!(
        t.complete(),
        Err(GateError::InvalidTransition {
            from: TaskState::Verifying,
            event: "complete"
        })
    );
    assert_eq!(
        t.try_accept(),
        Err(GateError::StaleReceipt(CriterionId("C1".into())))
    );
    for c in ["C1", "C2", "C3", "C4"] {
        pass(&mut t, c);
    }
    t.try_accept().unwrap();
    t.complete().unwrap();
}

#[test]
fn review_same_candidate_preserves_evidence() {
    let mut t = task();
    for c in ["C1", "C2", "C3", "C4"] {
        pass(&mut t, c);
    }
    t.try_accept().unwrap();
    t.observe_candidate(candidate("one")).unwrap();
    t.complete().unwrap();
}

#[test]
fn review_finding_enters_bounded_repair() {
    let mut t = task();
    for c in ["C1", "C2", "C3", "C4"] {
        pass(&mut t, c);
    }
    t.try_accept().unwrap();
    // A Low finding does not block acceptance, so review can reach the cap.
    let low = Finding {
        severity: Severity::Low,
        ..finding()
    };
    t.raise_finding(low.clone()).unwrap();
    t.request_repair().unwrap();
    assert_eq!(t.state(), TaskState::Repairing);
    assert_eq!(t.consumed_repair_rounds(), 1);
    t.finish_repair().unwrap();
    t.try_accept().unwrap();
    t.request_repair().unwrap();
    assert_eq!(t.consumed_repair_rounds(), 2);
    t.finish_repair().unwrap();
    t.try_accept().unwrap();
    assert_eq!(t.state(), TaskState::ReadyForReview);
    assert_eq!(
        t.request_repair(),
        Err(GateError::RepairBudgetExhausted { max: 2 })
    );
    assert_eq!(t.state(), TaskState::BudgetExhausted);
    assert_eq!(t.consumed_repair_rounds(), 2);
    assert_eq!(t.open_findings(), vec![low]);
    assert_eq!(
        t.receipt_status(&CriterionId("C1".into())),
        Some(ReceiptStatus::Current)
    );
    assert_eq!(
        t.complete(),
        Err(GateError::InvalidTransition {
            from: TaskState::BudgetExhausted,
            event: "complete"
        })
    );
}

#[test]
fn stopping_a_resumed_task_needs_no_workspace() {
    for state in [TaskState::Blocked, TaskState::Failed, TaskState::Cancelled] {
        let mut t = Task::resume(contract(), task().checkpoint());
        assert_eq!(t.try_accept(), Err(GateError::CandidateNotObserved));
        match state {
            TaskState::Blocked => t.block("workspace gone").unwrap(),
            TaskState::Failed => t.fail("workspace gone").unwrap(),
            TaskState::Cancelled => t.cancel().unwrap(),
            _ => unreachable!(),
        }
        assert_eq!(t.state(), state);
        assert_eq!(
            t.checkpoint().next_action,
            if state == TaskState::Cancelled {
                ""
            } else {
                "workspace gone"
            }
        );
        assert_eq!(t.complete(), Err(GateError::CandidateNotObserved));
    }
}
