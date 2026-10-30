use coding_supervisor::{Budget, Candidate, CandidateId, GateError, Task, TaskContract, TaskId};

fn acceptance(scope: &str, changed: &str) -> Result<(), GateError> {
    let mut task = Task::new(TaskContract {
        id: TaskId("scope".into()),
        goal: "scope fixture".into(),
        criteria: vec![],
        forbidden_effects: vec![],
        scope: vec![scope.into()],
        budget: Budget {
            max_repair_rounds: 1,
        },
    });
    task.start_preparing().unwrap();
    task.observe_candidate(Candidate {
        id: CandidateId {
            base_commit: "base".into(),
            dirty_digest: "dirty".into(),
        },
        provenance: "fixture".into(),
        changed_paths: vec![changed.into()],
    })
    .unwrap();
    task.try_accept()
}

#[test]
fn file_scope_is_exact_and_directory_scope_is_a_segment_boundary() {
    assert!(acceptance("src/lib.rs", "src/lib.rs").is_ok());
    for path in ["src/lib.rs.bak", "src/lib.rs/escape", "src/lib.rs-other"] {
        assert!(
            matches!(
                acceptance("src/lib.rs", path),
                Err(GateError::OutOfScope(_))
            ),
            "{path}"
        );
    }
    assert!(acceptance("src/", "src/nested/lib.rs").is_ok());
    for path in [
        "src-other/lib.rs",
        "src/../outside",
        "src/./lib.rs",
        "src//lib.rs",
        "/src/lib.rs",
        "src\\..\\outside",
    ] {
        assert!(
            matches!(acceptance("src/", path), Err(GateError::OutOfScope(_))),
            "{path}"
        );
    }
}
