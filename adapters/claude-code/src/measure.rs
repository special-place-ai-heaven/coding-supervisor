//! `measure`: gate-derived numbers from adapter journals and T3 hook captures. Nothing here
//! reads transcripts, and hook payloads carry no USD cost.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

use crate::header::{self, Verdict};
use crate::journal::{decision_parts, receipts_from_reasons, usage};
use crate::state::obj;

/// Refusal categories in report order.
pub const CATEGORIES: [&str; 12] = [
    "missing",
    "stale",
    "failing",
    "out_of_scope",
    "unlinked",
    "no_header",
    "inconsistent",
    "tampered",
    "state_path",
    "terminal_state",
    "unobserved",
    "other",
];

/// First match wins. Fixed refusal sentences come first because their wording contains other
/// keywords (the no-header text names `CHECKS RUN`); evidence problems follow in the order the
/// gate lists them.
const CLASSIFY: [(&str, &str); 11] = [
    ("tampered", "tampered"),
    ("not writable by agents", "state_path"),
    ("no header", "no_header"),
    ("task is ", "terminal_state"),
    ("could not be observed", "unobserved"),
    ("CHECKS RUN", "inconsistent"),
    ("unlinked", "unlinked"),
    ("out of scope", "out_of_scope"),
    ("failing", "failing"),
    ("stale", "stale"),
    ("missing", "missing"),
];

pub fn classify(text: &str) -> &'static str {
    CLASSIFY
        .iter()
        .find(|(needle, _)| text.contains(needle))
        .map_or("other", |(_, category)| category)
}

#[derive(Debug, Default)]
pub struct Stats {
    pub lines: u64,
    pub sessions: BTreeSet<String>,
    pub dispatches: u64,
    pub by_agent_type: BTreeMap<String, u64>,
    pub headers: u64,
    pub pass_supported: u64,
    pub pass_unsupported: u64,
    /// t3-capture only: no receipts exist there, so support cannot be judged.
    pub pass_unjudged: u64,
    pub fail: u64,
    pub unverified: u64,
    /// gate-legacy only: "verdict is not PASS" does not say which.
    pub not_pass_unknown: u64,
    pub no_header: u64,
    pub recorded: u64,
    pub rejected: u64,
    pub unlinked: u64,
    pub refusals: u64,
    pub categories: BTreeMap<&'static str, u64>,
    pub tamper_refusals: u64,
    /// Acceptances decided at a hook (Stop, SubagentStop).
    pub accepted: u64,
    /// `accepted: true` on an `mcp:*` line: the advisory server's own answer, not enforced.
    pub accepted_mcp: u64,
    /// gate-v1 lines without a session id: the MCP tool server writes no session.
    pub no_session_lines: u64,
    pub usage_lines: u64,
    pub total_tokens: u64,
    /// Byte-identical lines seen before in this source (overlapping journal snapshots).
    pub duplicates_dropped: u64,
}

#[derive(Debug, Default)]
pub struct Report {
    pub sources: BTreeMap<&'static str, Stats>,
    pub files_read: Vec<String>,
    pub unreadable: Vec<String>,
    pub unrecognized_lines: u64,
}

enum Seen {
    NoHeader,
    Verdict(Verdict),
    NotPassUnknown,
}

/// One journal line reduced to what the measures need.
struct Observation<'a> {
    source: &'static str,
    event: &'a str,
    /// `None` for a gate-v1 line without a session id.
    session: Option<String>,
    agent_type: Option<&'a str>,
    kind: &'static str,
    text: Option<String>,
    reasons: Vec<String>,
    header: Seen,
    receipts: [usize; 3],
    accepted: bool,
    tokens: Option<u64>,
}

fn s<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key).and_then(Value::as_str)
}

fn strings(v: Option<&Value>) -> Vec<String> {
    v.and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

fn observe<'a>(v: &'a Value, file: &str) -> Option<Observation<'a>> {
    // Older shapes fall back to the file name, which stands for one session there.
    let session = |id: Option<&str>| Some(id.map_or_else(|| file.to_owned(), str::to_owned));
    if v.get("v").and_then(Value::as_u64) == Some(1) {
        let decision = v.get("decision");
        let header = match v.get("header").and_then(|h| s(h, "verdict")) {
            Some("PASS") => Seen::Verdict(Verdict::Pass),
            Some("FAIL") => Seen::Verdict(Verdict::Fail),
            Some("UNVERIFIED") => Seen::Verdict(Verdict::Unverified),
            _ => Seen::NoHeader,
        };
        let receipts = v.get("receipts");
        let count = |k: &str| {
            receipts
                .and_then(|r| r.get(k))
                .and_then(Value::as_array)
                .map_or(0, Vec::len)
        };
        let kind = match decision.and_then(|d| s(d, "kind")) {
            Some("block") => "block",
            Some("deny") => "deny",
            Some("context") => "context",
            Some("system") => "system",
            _ => "none",
        };
        return Some(Observation {
            source: "gate-v1",
            event: s(v, "event").unwrap_or_default(),
            session: s(v, "session_id").map(str::to_owned),
            agent_type: s(v, "agent_type"),
            kind,
            text: decision.and_then(|d| s(d, "text")).map(str::to_owned),
            reasons: strings(v.get("reasons")),
            header,
            receipts: [count("recorded"), count("rejected"), count("unlinked")],
            accepted: v.get("accepted").and_then(Value::as_bool) == Some(true),
            tokens: v
                .get("usage")
                .and_then(|u| u.get("total_tokens"))
                .and_then(Value::as_u64),
        });
    }
    let decision = v.get("decision").unwrap_or(&Value::Null);
    if let (Some(payload), Some(event)) = (v.get("payload"), s(v, "event")) {
        let (kind, text) = decision_parts(decision);
        let header = match header::parse(s(payload, "last_assistant_message").unwrap_or_default()) {
            Some(h) => Seen::Verdict(h.verdict),
            None => Seen::NoHeader,
        };
        return Some(Observation {
            source: "t3-capture",
            event: s(payload, "hook_event_name").unwrap_or(event),
            session: session(s(payload, "session_id")),
            agent_type: s(payload, "agent_type"),
            kind,
            text,
            reasons: Vec::new(),
            header,
            receipts: [0; 3],
            accepted: false,
            tokens: usage(payload).get("total_tokens").and_then(Value::as_u64),
        });
    }
    let (Some(event), true, Some(reasons)) = (
        s(v, "event"),
        v.get("decision").is_some(),
        v.get("reasons").filter(|r| r.is_array()),
    ) else {
        return None;
    };
    let reasons = strings(Some(reasons));
    let (kind, text) = decision_parts(decision);
    let no_header = reasons.iter().any(|r| r.contains("no header"))
        || text.as_deref().is_some_and(|t| t.contains("no header"));
    let header = if no_header {
        Seen::NoHeader
    } else if reasons.iter().any(|r| r.contains("verdict is not PASS")) {
        Seen::NotPassUnknown
    } else {
        Seen::Verdict(Verdict::Pass)
    };
    let [recorded, rejected, unlinked] = receipts_from_reasons(&reasons);
    Some(Observation {
        source: "gate-legacy",
        event,
        session: Some(file.to_owned()),
        agent_type: None,
        kind,
        text,
        header,
        receipts: [recorded.len(), rejected.len(), unlinked.len()],
        accepted: reasons.iter().any(|r| r == "completed"),
        tokens: None,
        reasons,
    })
}

fn add(stats: &mut Stats, o: &Observation) {
    stats.lines += 1;
    match &o.session {
        Some(session) => {
            stats.sessions.insert(session.clone());
        }
        None => stats.no_session_lines += 1,
    }
    let [recorded, rejected, unlinked] = o.receipts.map(|n| n as u64);
    if o.event == "SubagentStop" {
        stats.dispatches += 1;
        *stats
            .by_agent_type
            .entry(o.agent_type.unwrap_or("unknown").to_owned())
            .or_default() += 1;
        match o.header {
            Seen::NoHeader => stats.no_header += 1,
            Seen::NotPassUnknown => {
                stats.headers += 1;
                stats.not_pass_unknown += 1;
            }
            Seen::Verdict(verdict) => {
                stats.headers += 1;
                match verdict {
                    Verdict::Fail => stats.fail += 1,
                    Verdict::Unverified => stats.unverified += 1,
                    Verdict::Pass if o.source == "t3-capture" => stats.pass_unjudged += 1,
                    Verdict::Pass => {
                        let released_unverified = o
                            .reasons
                            .iter()
                            .any(|r| r.contains("recorded as UNVERIFIED"));
                        if o.kind == "none"
                            && rejected == 0
                            && unlinked == 0
                            && !released_unverified
                        {
                            stats.pass_supported += 1;
                        } else {
                            stats.pass_unsupported += 1;
                        }
                    }
                }
            }
        }
    }
    stats.recorded += recorded;
    stats.rejected += rejected;
    stats.unlinked += unlinked;
    if matches!(o.kind, "block" | "deny") {
        let text = o.text.as_deref().unwrap_or_default();
        stats.refusals += 1;
        *stats.categories.entry(classify(text)).or_default() += 1;
        if text.contains("tampered") {
            stats.tamper_refusals += 1;
        }
    }
    if o.accepted {
        if o.event.starts_with("mcp:") {
            stats.accepted_mcp += 1;
        } else if matches!(o.event, "Stop" | "SubagentStop") {
            stats.accepted += 1;
        }
    }
    if let Some(tokens) = o.tokens {
        stats.usage_lines += 1;
        stats.total_tokens += tokens;
    }
}

fn wanted(path: &Path) -> bool {
    path.file_name().and_then(|n| n.to_str()).is_some_and(|n| {
        n == "events.jsonl" || n.ends_with(".adapter-events.jsonl") || n.ends_with(".events.jsonl")
    })
}

fn find(path: &Path, files: &mut Vec<PathBuf>, unreadable: &mut Vec<String>) {
    if path.is_dir() {
        match fs::read_dir(path) {
            Ok(entries) => {
                let mut children: Vec<PathBuf> =
                    entries.filter_map(|e| e.ok().map(|e| e.path())).collect();
                children.sort();
                for child in children {
                    if child.is_dir() || wanted(&child) {
                        find(&child, files, unreadable);
                    }
                }
            }
            Err(e) => unreadable.push(format!("{}: {e}", path.display())),
        }
    } else {
        files.push(path.to_path_buf());
    }
}

pub fn collect(paths: &[PathBuf]) -> Report {
    let mut report = Report::default();
    let mut files = Vec::new();
    for path in paths {
        find(path, &mut files, &mut report.unreadable);
    }
    let mut seen: BTreeMap<&'static str, HashSet<String>> = BTreeMap::new();
    for file in files {
        let name = file.display().to_string();
        let text = match fs::read_to_string(&file) {
            Ok(text) => text,
            Err(e) => {
                report.unreadable.push(format!("{name}: {e}"));
                continue;
            }
        };
        for line in text.lines().filter(|l| !l.trim().is_empty()) {
            let parsed = serde_json::from_str::<Value>(line).ok();
            let Some(o) = parsed.as_ref().and_then(|v| observe(v, &name)) else {
                report.unrecognized_lines += 1;
                continue;
            };
            let stats = report.sources.entry(o.source).or_default();
            // A later snapshot of the same session repeats its earlier lines; count each once.
            if seen.entry(o.source).or_default().insert(line.to_owned()) {
                add(stats, &o);
            } else {
                stats.duplicates_dropped += 1;
            }
        }
        report.files_read.push(name);
    }
    report
}

const NOTE: &str = "Every number is gate-derived: read from gate journals and hook captures only, never from transcripts. Hook payloads carry no USD cost, so none is reported.";

fn frac(n: u64, d: u64) -> String {
    format!("{n}/{d}")
}

fn tokens_cell(st: &Stats) -> String {
    if st.accepted == 0 || st.usage_lines == 0 {
        return format!("n/a ({} usage lines)", st.usage_lines);
    }
    format!(
        "{}/{} = {} ({} usage lines)",
        st.total_tokens,
        st.accepted,
        st.total_tokens / st.accepted,
        st.usage_lines
    )
}

impl Report {
    fn rows(&self) -> Vec<(String, Vec<String>)> {
        let stats: Vec<&Stats> = self.sources.values().collect();
        let row = |label: &str, f: &dyn Fn(&Stats) -> String| {
            (
                format!("gate-derived {label}"),
                stats.iter().map(|s| f(s)).collect(),
            )
        };
        let mut rows = vec![
            row("lines", &|s| s.lines.to_string()),
            row("sessions", &|s| s.sessions.len().to_string()),
            row("tool-server lines (no session)", &|s| {
                s.no_session_lines.to_string()
            }),
            row("judge dispatches (SubagentStop)", &|s| {
                s.dispatches.to_string()
            }),
        ];
        let agent_types: BTreeSet<&String> =
            stats.iter().flat_map(|s| s.by_agent_type.keys()).collect();
        for agent_type in agent_types {
            rows.push(row(&format!("dispatches, agent_type {agent_type}"), &|s| {
                frac(
                    s.by_agent_type.get(agent_type).copied().unwrap_or(0),
                    s.dispatches,
                )
            }));
        }
        rows.push(row("compliant headers", &|s| frac(s.headers, s.dispatches)));
        rows.push(row("PASS supported", &|s| {
            frac(s.pass_supported, s.dispatches)
        }));
        rows.push(row("PASS unsupported", &|s| {
            frac(s.pass_unsupported, s.dispatches)
        }));
        rows.push(row(
            "PASS, support not observable (no receipts in source)",
            &|s| frac(s.pass_unjudged, s.dispatches),
        ));
        rows.push(row("FAIL", &|s| frac(s.fail, s.dispatches)));
        rows.push(row("UNVERIFIED", &|s| frac(s.unverified, s.dispatches)));
        rows.push(row("not PASS, verdict not journaled (legacy)", &|s| {
            frac(s.not_pass_unknown, s.dispatches)
        }));
        rows.push(row("no header", &|s| frac(s.no_header, s.dispatches)));
        rows.push(row("linked receipts", &|s| {
            frac(s.recorded, s.recorded + s.rejected + s.unlinked)
        }));
        rows.push(row("refusals (block or deny)", &|s| s.refusals.to_string()));
        for category in CATEGORIES {
            rows.push(row(&format!("refusals: {category}"), &|s| {
                frac(s.categories.get(category).copied().unwrap_or(0), s.refusals)
            }));
        }
        rows.push(row("tamper refusals", &|s| {
            frac(s.tamper_refusals, s.refusals)
        }));
        rows.push(row("accepted tasks", &|s| {
            frac(s.accepted, s.sessions.len() as u64)
        }));
        rows.push(row("accepted via MCP (advisory)", &|s| {
            s.accepted_mcp.to_string()
        }));
        rows.push(row("worker tokens (final request only)", &tokens_cell));
        rows
    }

    pub fn markdown(&self) -> String {
        let mut out = format!(
            "# Gate-derived measures\n\n{NOTE}\n\nFiles read: {}. Unrecognized lines: {}. Duplicate lines dropped: {}. Cells are n/d where a denominator applies.\n\n",
            self.files_read.len(),
            self.unrecognized_lines,
            self.sources
                .values()
                .map(|s| s.duplicates_dropped)
                .sum::<u64>()
        );
        let sources: Vec<&str> = self.sources.keys().copied().collect();
        out.push_str(&format!("| measure | {} |\n", sources.join(" | ")));
        out.push_str(&format!("|---|{}\n", "---|".repeat(sources.len())));
        for (label, cells) in self.rows() {
            out.push_str(&format!("| {label} | {} |\n", cells.join(" | ")));
        }
        out
    }

    pub fn to_json(&self) -> Value {
        let sources: Map<String, Value> = self
            .sources
            .iter()
            .map(|(name, st)| {
                let categories: Map<String, Value> = CATEGORIES
                    .iter()
                    .map(|c| {
                        (
                            (*c).to_owned(),
                            st.categories.get(c).copied().unwrap_or(0).into(),
                        )
                    })
                    .collect();
                let by_agent_type: Map<String, Value> = st
                    .by_agent_type
                    .iter()
                    .map(|(k, n)| (k.clone(), (*n).into()))
                    .collect();
                let per_accepted =
                    (st.accepted > 0 && st.usage_lines > 0).then(|| st.total_tokens / st.accepted);
                let value = obj([
                    ("lines", st.lines.into()),
                    ("sessions", st.sessions.len().into()),
                    ("tool_server_lines_no_session", st.no_session_lines.into()),
                    ("dispatches", st.dispatches.into()),
                    ("dispatches_by_agent_type", Value::Object(by_agent_type)),
                    ("compliant_headers", st.headers.into()),
                    ("pass_supported", st.pass_supported.into()),
                    ("pass_unsupported", st.pass_unsupported.into()),
                    ("pass_support_not_observable", st.pass_unjudged.into()),
                    ("fail", st.fail.into()),
                    ("unverified", st.unverified.into()),
                    ("not_pass_verdict_not_journaled", st.not_pass_unknown.into()),
                    ("no_header", st.no_header.into()),
                    ("receipts_recorded", st.recorded.into()),
                    ("receipts_rejected", st.rejected.into()),
                    ("receipts_unlinked", st.unlinked.into()),
                    ("refusals", st.refusals.into()),
                    ("refusals_by_category", Value::Object(categories)),
                    ("tamper_refusals", st.tamper_refusals.into()),
                    ("accepted_tasks", st.accepted.into()),
                    ("accepted_via_mcp_advisory", st.accepted_mcp.into()),
                    ("usage_lines", st.usage_lines.into()),
                    ("duplicates_dropped", st.duplicates_dropped.into()),
                    (
                        "worker_total_tokens_final_request_only",
                        st.total_tokens.into(),
                    ),
                    ("worker_tokens_per_accepted_task", per_accepted.into()),
                ]);
                ((*name).to_owned(), value)
            })
            .collect();
        obj([
            ("note", NOTE.into()),
            ("files_read", self.files_read.len().into()),
            ("unrecognized_lines", self.unrecognized_lines.into()),
            ("sources", Value::Object(sources)),
        ])
    }
}
