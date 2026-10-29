//! Does a final message claim the task is complete?

use crate::header::{self, Verdict};

const PHRASES: [&str; 8] = [
    "done",
    "complete",
    "completed",
    "finished",
    "ready for review",
    "verified",
    "all checks pass",
    "tests pass",
];

const NEGATIONS: [&str; 23] = [
    "not",
    "n't",
    "can't",
    "never",
    "cannot",
    "unable",
    "refuse",
    "refused",
    "isn't",
    "haven't",
    "hasn't",
    "won't",
    "blocked",
    "unverified",
    "incomplete",
    "undone",
    "nothing",
    "none",
    "no",
    "nobody",
    "nowhere",
    "neither",
    "nor",
];

/// A clause claims completion when it has a claim phrase and no negation token, both
/// word-bounded. A `VERDICT: PASS` header always claims completion.
pub fn claims_completion(text: &str) -> bool {
    if header::parse(text).is_some_and(|h| h.verdict == Verdict::Pass) {
        return true;
    }
    let prose = without_quotes(&without_fences(text))
        .to_lowercase()
        .replace('\u{2019}', "'");
    // Clauses, not just sentences: "This is not complete, but the parser is done." is a claim.
    prose
        .split(['.', '!', '?', '\n', ',', ';', ':'])
        .any(|clause| {
            PHRASES.iter().any(|p| has_word(clause, p, true))
                && !NEGATIONS.iter().any(|n| has_word(clause, n, *n != "n't"))
        })
}

fn without_fences(text: &str) -> String {
    let mut fenced = false;
    let mut out = String::new();
    for line in text.lines() {
        if line.trim_start().starts_with("```") {
            fenced = !fenced;
        } else if !fenced {
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

/// Drops text inside double quotes or backticks: a message that quotes an earlier claim while
/// withdrawing it ("My earlier \"Done.\" was only a quote") is not itself a claim (L5 live run).
/// An unbalanced quote keeps the rest of the line.
fn without_quotes(text: &str) -> String {
    let mut out = String::new();
    for line in text.lines() {
        let mut rest = line;
        loop {
            let Some(open) = rest.find(['"', '`']) else {
                out.push_str(rest);
                break;
            };
            let quote = rest.as_bytes()[open] as char;
            out.push_str(&rest[..open]);
            match rest[open + 1..].find(quote) {
                Some(close) => {
                    out.push(' ');
                    rest = &rest[open + 1 + close + 1..];
                }
                None => {
                    out.push_str(&rest[open + 1..]);
                    break;
                }
            }
        }
        out.push('\n');
    }
    out
}

/// `phrase` occurs ending at a word boundary, and also starting at one when `bounded_start`
/// (false only for the suffix token "n't", as in "didn't").
fn has_word(clause: &str, phrase: &str, bounded_start: bool) -> bool {
    clause.match_indices(phrase).any(|(at, _)| {
        let before = clause.get(..at).and_then(|s| s.chars().next_back());
        let after = clause
            .get(at + phrase.len()..)
            .and_then(|s| s.chars().next());
        !(bounded_start && before.is_some_and(char::is_alphanumeric))
            && !after.is_some_and(char::is_alphanumeric)
    })
}
