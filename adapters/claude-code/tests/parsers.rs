use supervisor_claude_code::claim::claims_completion;
use supervisor_claude_code::header::{CheckLine, Verdict, parse};
use supervisor_claude_code::sha256;

#[test]
fn sha256_known_vectors() {
    assert_eq!(
        sha256::hex(b"abc"),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
    assert_eq!(
        sha256::hex(b""),
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
    // 56 bytes: the length no longer fits in the first block.
    assert_eq!(
        sha256::hex(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"),
        "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
    );
}

#[test]
fn header_valid_pass_with_two_lines() {
    let h = parse(
        "VERDICT: PASS\nCHECKS RUN: 2\nCHECK C1 | cargo test | exit 0\nCHECK C2 | cargo clippy | cwd E:\\x | exit 0",
    )
    .unwrap();
    assert_eq!(h.verdict, Verdict::Pass);
    assert_eq!(h.checks_run, 2);
    assert!(h.consistent);
    assert_eq!(
        h.checks,
        vec![
            CheckLine {
                criterion: "C1".into(),
                command: "cargo test".into(),
                cwd: None,
                exit_code: 0
            },
            CheckLine {
                criterion: "C2".into(),
                command: "cargo clippy".into(),
                cwd: Some("E:\\x".into()),
                exit_code: 0
            },
        ]
    );
}

#[test]
fn header_checks_run_one_with_zero_lines_is_inconsistent() {
    let h = parse("VERDICT: PASS\nCHECKS RUN: 1").unwrap();
    assert!(h.checks.is_empty());
    assert!(!h.consistent);
}

#[test]
fn header_missing_is_none() {
    assert_eq!(parse("All good, tests pass."), None);
    assert_eq!(parse("VERDICT: PASS\nno count here"), None);
    assert_eq!(parse("VERDICT: MAYBE\nCHECKS RUN: 0"), None);
}

#[test]
fn header_lowercase_keys_accepted() {
    let h = parse("verdict: FAIL\nchecks run: 1\ncheck C1 | cargo test | exit 101").unwrap();
    assert_eq!(h.verdict, Verdict::Fail);
    assert!(h.consistent);
    assert_eq!(h.checks[0].exit_code, 101);
}

#[test]
fn header_negative_exit_parses() {
    let h = parse("VERDICT: FAIL\nCHECKS RUN: 1\nCHECK C3 | ./run.sh | exit -1").unwrap();
    assert_eq!(h.checks[0].exit_code, -1);
}

#[test]
fn claim_parser_required_cases() {
    let cases = [
        ("Done, the task is complete.", true),
        ("The task is not done.", false),
        ("Not verified: no check ran.", false),
        ("I can't mark this done.", false),
        ("The task is unverified and I haven't finished.", false),
        ("I've completed the refactoring and all tests pass.", true),
        ("The gate is right to refuse. Nothing is complete.", false),
        ("Ready for review.", true),
        ("This is not complete, but the parser is done.", true),
        ("Another check is done.", true),
        ("Annotation complete, ready for review.", true),
        ("Nothing is complete.", false),
        ("None of the checks pass.", false),
        ("No tests pass yet.", false),
        (
            "My earlier \"Done, the task is complete.\" was only the sentence you asked me to repeat.",
            false,
        ),
        (
            "The log says `all tests pass` but I have not checked it.",
            false,
        ),
        ("The check is `cargo test`, and it is done.", true),
        ("Here is the log:\n```\ndone\n```\nSee above.", false),
    ];
    for (text, expected) in cases {
        assert_eq!(claims_completion(text), expected, "{text:?}");
    }
}

#[test]
fn claim_pass_header_claims_completion() {
    assert!(claims_completion("VERDICT: PASS\nCHECKS RUN: 0"));
    assert!(!claims_completion("VERDICT: UNVERIFIED\nCHECKS RUN: 0"));
}
