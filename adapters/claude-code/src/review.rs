//! WP5 reviewer protocol: a final message whose first non-empty line is `REVIEW: APPROVED`
//! (exactly) or `REVIEW: REJECTED` (read loosely), with `FINDING: <text>` lines directly below it. The rest of the message is
//! left for the handback header, so a builder can review its own work and report as usual.

use coding_supervisor::{CandidateId, GateError, Review, ReviewDecision, review_met};
use serde_json::Value;

use crate::state::{StoredReview, decision_name, obj, provenance_name};

/// shortcut: fixed caps on what a reviewer can store; raise them if real reviews need more.
const MAX_FINDINGS: usize = 20;
const MAX_FINDING_CHARS: usize = 300;

pub struct Parsed<'a> {
    /// `None`: the line names no decision (inconclusive, empty, a timeout text).
    pub decision: Option<ReviewDecision>,
    pub findings: Vec<String>,
    /// The message after the review block.
    pub rest: &'a str,
}

/// `None` when the first non-empty line is not a `REVIEW:` line.
pub fn parse(message: &str) -> Option<Parsed<'_>> {
    let mut offset = 0;
    let mut lines = message.split_inclusive('\n').map(|line| {
        let start = offset;
        offset += line.len();
        (start, line.trim())
    });
    let (_, first) = lines.by_ref().find(|(_, line)| !line.is_empty())?;
    // A review line is read loosely (markdown marks, any case), and so is a rejection, so a
    // refusal written a little differently is not lost; an approval must be exact.
    let plain: String = first
        .chars()
        .filter(|c| !matches!(c, '*' | '_' | '`'))
        .collect::<String>()
        .to_ascii_lowercase();
    let value = plain.trim().strip_prefix("review:")?.trim_start();
    let rejected = value
        .strip_prefix("rejected")
        .is_some_and(|rest| !rest.starts_with(|c: char| c.is_alphanumeric()));
    let decision = if first == "REVIEW: APPROVED" {
        Some(ReviewDecision::Approved)
    } else if rejected {
        Some(ReviewDecision::Rejected)
    } else {
        None
    };
    let mut findings = Vec::new();
    let mut rest = "";
    for (start, line) in lines {
        if line.is_empty() {
            continue;
        }
        let Some(finding) = line.strip_prefix("FINDING:") else {
            rest = &message[start..];
            break;
        };
        // Control characters (ESC and C1 included) would reach a terminal through `status`.
        let finding: String = finding.chars().filter(|c| !c.is_control()).collect();
        let finding: String = finding.trim().chars().take(MAX_FINDING_CHARS).collect();
        if !finding.is_empty() && findings.len() < MAX_FINDINGS {
            findings.push(finding);
        }
    }
    Some(Parsed {
        decision,
        findings,
        rest,
    })
}

/// The message without a leading review block.
pub fn remainder(message: &str) -> &str {
    parse(message).map_or(message, |p| p.rest)
}

/// One stored review for display: identifiers, decision, provenance and the finding count,
/// never the finding text.
pub fn record_json(s: &StoredReview) -> Value {
    let r = &s.review;
    obj([
        ("reviewer", r.reviewer.clone().into()),
        ("model", r.model.clone().into()),
        ("decision", decision_name(r.decision).into()),
        ("provenance", provenance_name(r.provenance).into()),
        ("findings", r.findings.len().into()),
        ("at", r.at.into()),
    ])
}

/// The latest review recorded for `candidate`.
pub fn latest<'a>(
    reviews: &'a [StoredReview],
    candidate: &CandidateId,
) -> Option<&'a StoredReview> {
    reviews
        .iter()
        .rev()
        .find(|s| &s.review.candidate == candidate)
}

/// Why the core rule [`review_met`] refuses `reviews` on `candidate`, as the reason code that
/// verdict, status and `task_state` share; `None` when it is met.
pub fn unmet_reason(
    policy: &Review,
    reviews: &[StoredReview],
    candidate: &CandidateId,
) -> Option<&'static str> {
    let records: Vec<_> = reviews.iter().map(|s| s.review.clone()).collect();
    review_met(policy, &records, candidate)
        .err()
        .map(|e| match e {
            GateError::ReviewRejected { .. } => "review_rejected",
            GateError::ReviewNotIndependent => "review_not_independent",
            _ => "review_required",
        })
}

/// `{required, independent, met, reason, latest}` for status and `task_state`: the latest review
/// is never shown without whether the rule is met (a rejection beside it still refuses). `met`
/// and `reason` are null without a candidate.
pub fn summary_json(
    policy: &Review,
    reviews: &[StoredReview],
    candidate: Option<&CandidateId>,
) -> Value {
    let reason = candidate.map(|c| unmet_reason(policy, reviews, c));
    obj([
        ("required", policy.required.into()),
        ("independent", policy.independent.into()),
        ("met", reason.map(|r| r.is_none()).into()),
        ("reason", reason.flatten().into()),
        (
            "latest",
            candidate
                .and_then(|c| latest(reviews, c))
                .map_or(Value::Null, record_json),
        ),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_review_line_must_come_first() {
        assert!(parse("Fine.\nREVIEW: APPROVED").is_none());
        assert!(parse("VERDICT: PASS\nCHECKS RUN: 0").is_none());
        let p = parse("\n  REVIEW: REJECTED \n\nFINDING: a\nFINDING:\nFINDING:  b \nVERDICT: PASS\nCHECKS RUN: 0").unwrap();
        assert_eq!(p.decision, Some(ReviewDecision::Rejected));
        assert_eq!(p.findings, ["a", "b"]);
        assert_eq!(p.rest, "VERDICT: PASS\nCHECKS RUN: 0");
    }

    #[test]
    fn anything_else_after_review_is_not_a_decision() {
        for line in [
            "REVIEW:",
            "REVIEW: INCONCLUSIVE",
            "REVIEW: approved",
            "REVIEW: APPROVED!",
        ] {
            assert_eq!(parse(line).unwrap().decision, None, "{line}");
        }
    }

    /// Loose forms count for a rejection only: a refusal written a little differently must not
    /// be lost, while an approval has to be exact.
    #[test]
    fn a_rejection_is_read_loosely_and_an_approval_exactly() {
        for line in [
            "**REVIEW: REJECTED**",
            "REVIEW: REJECTED.",
            "REVIEW: REJECTED - see below",
            "Review: rejected",
            "`review: Rejected`",
            "__REVIEW:__ REJECTED",
        ] {
            let message = format!("{line}\nFINDING: x");
            let p = parse(&message).unwrap_or_else(|| panic!("{line}"));
            assert_eq!(p.decision, Some(ReviewDecision::Rejected), "{line}");
            assert_eq!(p.findings, ["x"], "{line}");
        }
        for line in [
            "REVIEW: APPROVED - with caveats",
            "**REVIEW: APPROVED**",
            "Review: approved",
            "REVIEW: REJECTEDNESS",
        ] {
            assert_eq!(parse(line).map(|p| p.decision), Some(None), "{line}");
        }
        assert_eq!(
            parse("REVIEW: APPROVED").unwrap().decision,
            Some(ReviewDecision::Approved)
        );
    }

    /// A finding is shown in a terminal by `status`; control characters (ESC included) never
    /// reach gate.json, and they do not count toward the cap.
    #[test]
    fn control_characters_are_stripped_from_findings() {
        let message =
            "REVIEW: REJECTED\nFINDING: \u{1b}[31mred\u{1b}[0m text\u{7}\u{9b}2J end \u{1b}";
        let p = parse(message).unwrap();
        assert_eq!(p.findings, ["[31mred[0m text2J end"]);
        let long = format!("FINDING: {}{}", "\u{1b}".repeat(50), "x".repeat(400));
        let message = format!("REVIEW: REJECTED\n{long}");
        let p = parse(&message).unwrap();
        assert_eq!(p.findings[0], "x".repeat(MAX_FINDING_CHARS));
    }

    #[test]
    fn findings_are_capped_in_number_and_length() {
        let long = "é".repeat(400);
        let message: String = std::iter::once("REVIEW: REJECTED".to_owned())
            .chain((0..30).map(|_| format!("FINDING: {long}")))
            .collect::<Vec<_>>()
            .join("\r\n");
        let p = parse(&message).unwrap();
        assert_eq!(p.findings.len(), MAX_FINDINGS);
        assert!(
            p.findings
                .iter()
                .all(|f| f.chars().count() == MAX_FINDING_CHARS)
        );
        assert_eq!(p.rest, "");
    }
}
