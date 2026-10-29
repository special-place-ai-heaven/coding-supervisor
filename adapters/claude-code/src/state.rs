//! The files under `.supervisor/`: contract, gate state, run journal and event journal.

use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::{ErrorKind, Write};
use std::path::Path;

use coding_supervisor::{
    Budget, CandidateId, CheckName, Checkpoint, Criterion, CriterionId, Disposition, Finding,
    FindingId, Severity, TaskContract, TaskId, TaskState, VerificationReceipt,
};
use serde_json::Value;

use crate::{chain, sha256};

const STATES: [(TaskState, &str); 10] = [
    (TaskState::Queued, "queued"),
    (TaskState::Preparing, "preparing"),
    (TaskState::Verifying, "verifying"),
    (TaskState::ReadyForReview, "ready_for_review"),
    (TaskState::Repairing, "repairing"),
    (TaskState::Blocked, "blocked"),
    (TaskState::BudgetExhausted, "budget_exhausted"),
    (TaskState::Failed, "failed"),
    (TaskState::Cancelled, "cancelled"),
    (TaskState::Completed, "completed"),
];
const SEVERITIES: [(Severity, &str); 4] = [
    (Severity::Low, "low"),
    (Severity::Medium, "medium"),
    (Severity::High, "high"),
    (Severity::Critical, "critical"),
];
const DISPOSITIONS: [(Disposition, &str); 3] = [
    (Disposition::Open, "open"),
    (Disposition::Resolved, "resolved"),
    (Disposition::Accepted, "accepted"),
];

fn name<T: PartialEq + Copy>(table: &[(T, &'static str)], value: T) -> &'static str {
    table
        .iter()
        .find(|(v, _)| *v == value)
        .map_or("unknown", |(_, n)| n)
}

fn lookup<T: Copy>(table: &[(T, &str)], text: &str) -> Result<T, String> {
    table
        .iter()
        .find(|(_, n)| *n == text)
        .map(|(v, _)| *v)
        .ok_or_else(|| format!("unknown value {text}"))
}

pub fn state_name(state: TaskState) -> &'static str {
    name(&STATES, state)
}

/// A JSON object from key/value pairs. Used instead of `json!`, which unwraps internally.
pub fn obj<const N: usize>(pairs: [(&str, Value); N]) -> Value {
    Value::Object(pairs.into_iter().map(|(k, v)| (k.to_owned(), v)).collect())
}

/// `gate.json`: the core checkpoint plus what the adapter needs across hook calls.
pub struct Gate {
    pub contract_id: String,
    pub prompt_id: Option<String>,
    pub checkpoint: Checkpoint,
    /// Every receipt the core ever accepted, each bound to its candidate.
    pub receipts: Vec<VerificationReceipt>,
    /// Stop refusals in the current prompt.
    pub continuations: u32,
    /// SubagentStop refusals per worker agent id.
    pub handback_refusals: BTreeMap<String, u32>,
    /// Per payload `session_id`. Absent in a gate.json written before levels existed.
    // shortcut: never pruned, one small entry per session; prune by age once gate.json grows noticeably
    pub sessions: BTreeMap<String, SessionRecord>,
    /// The candidate at the last Stop that completed or confirmed completion.
    pub accepted_candidate: Option<CandidateId>,
}

/// What the gate remembers about one Claude Code session.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SessionRecord {
    /// The first candidate observed in the session.
    pub baseline: Option<CandidateId>,
    /// Per criterion id, how its command failed on the baseline (the untouched starting tree).
    /// Noted on refusals and in `supervisor status`; the level is never lowered for it.
    pub baseline_failures: BTreeMap<String, String>,
    /// A lite-level notice for the next UserPromptSubmit, injected once.
    pub notice: Option<String>,
}

impl Gate {
    pub fn new(contract_id: String, checkpoint: Checkpoint) -> Self {
        Self {
            contract_id,
            prompt_id: None,
            checkpoint,
            receipts: Vec::new(),
            continuations: 0,
            handback_refusals: BTreeMap::new(),
            sessions: BTreeMap::new(),
            accepted_candidate: None,
        }
    }

    fn to_json(&self) -> Value {
        let cp = &self.checkpoint;
        obj([
            ("contract_id", self.contract_id.clone().into()),
            ("prompt_id", self.prompt_id.clone().into()),
            ("state", state_name(cp.state).into()),
            (
                "candidate",
                cp.candidate.as_ref().map(candidate_json).into(),
            ),
            (
                "open_findings",
                cp.open_findings.iter().map(finding_json).collect(),
            ),
            ("consumed_repair_rounds", cp.consumed_repair_rounds.into()),
            ("next_action", cp.next_action.clone().into()),
            ("receipts", self.receipts.iter().map(receipt_json).collect()),
            ("continuations", self.continuations.into()),
            (
                "handback_refusals",
                Value::Object(
                    self.handback_refusals
                        .iter()
                        .map(|(k, n)| (k.clone(), (*n).into()))
                        .collect(),
                ),
            ),
            (
                "sessions",
                Value::Object(
                    self.sessions
                        .iter()
                        .map(|(k, s)| {
                            let record = obj([
                                ("baseline", s.baseline.as_ref().map(candidate_json).into()),
                                (
                                    "baseline_failures",
                                    Value::Object(
                                        s.baseline_failures
                                            .iter()
                                            .map(|(id, why)| (id.clone(), why.clone().into()))
                                            .collect(),
                                    ),
                                ),
                                ("notice", s.notice.clone().into()),
                            ]);
                            (k.clone(), record)
                        })
                        .collect(),
                ),
            ),
            (
                "accepted_candidate",
                self.accepted_candidate.as_ref().map(candidate_json).into(),
            ),
        ])
    }

    fn from_json(v: &Value) -> Result<Self, String> {
        let candidate = match field(v, "candidate")? {
            Value::Null => None,
            c => Some(candidate_from(c)?),
        };
        let refusals = field(v, "handback_refusals")?
            .as_object()
            .ok_or("handback_refusals is not an object")?;
        Ok(Self {
            contract_id: string(v, "contract_id")?,
            prompt_id: v
                .get("prompt_id")
                .and_then(Value::as_str)
                .map(str::to_owned),
            checkpoint: Checkpoint {
                state: lookup(&STATES, &string(v, "state")?)?,
                candidate,
                open_findings: array(v, "open_findings")?
                    .iter()
                    .map(finding_from)
                    .collect::<Result<_, _>>()?,
                consumed_repair_rounds: small(v, "consumed_repair_rounds")?,
                next_action: string(v, "next_action")?,
            },
            receipts: array(v, "receipts")?
                .iter()
                .map(receipt_from)
                .collect::<Result<_, _>>()?,
            continuations: small(v, "continuations")?,
            handback_refusals: refusals
                .iter()
                .map(|(k, n)| {
                    n.as_u64()
                        .and_then(|n| u32::try_from(n).ok())
                        .map(|n| (k.clone(), n))
                        .ok_or_else(|| format!("handback_refusals.{k} is not a count"))
                })
                .collect::<Result<_, _>>()?,
            sessions: match v.get("sessions") {
                None => BTreeMap::new(),
                Some(sessions) => sessions
                    .as_object()
                    .ok_or("sessions is not an object")?
                    .iter()
                    .map(|(k, s)| Ok((k.clone(), session_from(s)?)))
                    .collect::<Result<_, String>>()?,
            },
            accepted_candidate: optional_candidate(v.get("accepted_candidate"))?,
        })
    }
}

fn optional_candidate(v: Option<&Value>) -> Result<Option<CandidateId>, String> {
    match v {
        None | Some(Value::Null) => Ok(None),
        Some(c) => candidate_from(c).map(Some),
    }
}
fn optional_string(v: &Value, key: &str) -> Result<Option<String>, String> {
    match v.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s.clone())),
        Some(_) => Err(format!("field {key} is not a string")),
    }
}
fn session_from(v: &Value) -> Result<SessionRecord, String> {
    Ok(SessionRecord {
        baseline: optional_candidate(v.get("baseline"))?,
        baseline_failures: match v.get("baseline_failures") {
            None => BTreeMap::new(),
            Some(map) => map
                .as_object()
                .ok_or("baseline_failures is not an object")?
                .iter()
                .map(|(id, why)| {
                    why.as_str()
                        .map(|w| (id.clone(), w.to_owned()))
                        .ok_or_else(|| format!("baseline_failures.{id} is not a string"))
                })
                .collect::<Result<_, String>>()?,
        },
        notice: optional_string(v, "notice")?,
    })
}

/// One observed command run from `runs.jsonl`.
pub(crate) struct Run {
    pub agent_id: String,
    pub command: String,
    pub exit_code: Option<i32>,
    /// The tool_use_id, or `runs.jsonl:<line>` when there is none.
    pub reference: String,
    pub t: u64,
    /// The candidate observed at the run; `None` for a record written before it was journaled.
    pub candidate: Option<CandidateId>,
}

fn field<'a>(v: &'a Value, key: &str) -> Result<&'a Value, String> {
    v.get(key).ok_or_else(|| format!("missing field {key}"))
}
fn string(v: &Value, key: &str) -> Result<String, String> {
    field(v, key)?
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| format!("field {key} is not a string"))
}
fn uint(v: &Value, key: &str) -> Result<u64, String> {
    field(v, key)?
        .as_u64()
        .ok_or_else(|| format!("field {key} is not a non-negative integer"))
}
fn small(v: &Value, key: &str) -> Result<u32, String> {
    u32::try_from(uint(v, key)?).map_err(|_| format!("field {key} is too large"))
}
fn array<'a>(v: &'a Value, key: &str) -> Result<&'a [Value], String> {
    field(v, key)?
        .as_array()
        .map(Vec::as_slice)
        .ok_or_else(|| format!("field {key} is not an array"))
}
fn strings(v: &Value, key: &str) -> Result<Vec<String>, String> {
    array(v, key)?
        .iter()
        .map(|s| {
            s.as_str()
                .map(str::to_owned)
                .ok_or_else(|| format!("field {key} holds a non-string"))
        })
        .collect()
}

pub fn candidate_json(c: &CandidateId) -> Value {
    obj([
        ("base_commit", c.base_commit.clone().into()),
        ("dirty_digest", c.dirty_digest.clone().into()),
    ])
}
fn candidate_from(v: &Value) -> Result<CandidateId, String> {
    Ok(CandidateId {
        base_commit: string(v, "base_commit")?,
        dirty_digest: string(v, "dirty_digest")?,
    })
}

fn receipt_json(r: &VerificationReceipt) -> Value {
    obj([
        ("criterion", r.criterion.0.clone().into()),
        ("candidate", candidate_json(&r.candidate)),
        ("command", r.command.clone().into()),
        ("cwd", r.cwd.clone().into()),
        ("exit_code", r.exit_code.into()),
        ("output_ref", r.output_ref.clone().into()),
        ("claim_supported", r.claim_supported.into()),
        ("started_at", r.started_at.into()),
        ("finished_at", r.finished_at.into()),
    ])
}
fn receipt_from(v: &Value) -> Result<VerificationReceipt, String> {
    let exit_code = match field(v, "exit_code")? {
        Value::Null => None,
        n => Some(
            n.as_i64()
                .and_then(|n| i32::try_from(n).ok())
                .ok_or("receipt exit_code is not an i32")?,
        ),
    };
    Ok(VerificationReceipt {
        criterion: CriterionId(string(v, "criterion")?),
        candidate: candidate_from(field(v, "candidate")?)?,
        command: string(v, "command")?,
        cwd: string(v, "cwd")?,
        exit_code,
        output_ref: string(v, "output_ref")?,
        claim_supported: field(v, "claim_supported")?
            .as_bool()
            .ok_or("claim_supported is not a boolean")?,
        started_at: uint(v, "started_at")?,
        finished_at: uint(v, "finished_at")?,
    })
}

pub fn finding_json(f: &Finding) -> Value {
    obj([
        ("id", f.id.0.clone().into()),
        ("candidate", candidate_json(&f.candidate)),
        ("severity", name(&SEVERITIES, f.severity).into()),
        ("location", f.location.clone().into()),
        ("evidence_ref", f.evidence_ref.clone().into()),
        ("disposition", name(&DISPOSITIONS, f.disposition).into()),
    ])
}
fn finding_from(v: &Value) -> Result<Finding, String> {
    Ok(Finding {
        id: FindingId(string(v, "id")?),
        candidate: candidate_from(field(v, "candidate")?)?,
        severity: lookup(&SEVERITIES, &string(v, "severity")?)?,
        location: string(v, "location")?,
        evidence_ref: string(v, "evidence_ref")?,
        disposition: lookup(&DISPOSITIONS, &string(v, "disposition")?)?,
    })
}

/// A criterion's `command`: one string or a non-empty list of strings, none of them blank.
fn commands(c: &Value, id: &str) -> Result<Vec<String>, String> {
    let list = match c.get("command") {
        None => {
            return Err(format!(
                "criterion {id} has no command; name the command a receipt for it must cite"
            ));
        }
        Some(Value::String(s)) => vec![s.clone()],
        Some(Value::Array(items)) => items
            .iter()
            .map(|s| s.as_str().map(str::to_owned))
            .collect::<Option<_>>()
            .ok_or_else(|| format!("criterion {id} command holds a non-string"))?,
        Some(_) => {
            return Err(format!(
                "criterion {id} command is not a string or a list of strings"
            ));
        }
    };
    if list.is_empty() || list.iter().any(|s| s.trim().is_empty()) {
        return Err(format!("criterion {id} command is empty"));
    }
    Ok(list)
}

/// Ids of the criteria declared `"tier": "full"`; the default tier is `quick`.
pub fn full_tier(v: &Value) -> Vec<String> {
    v.get("criteria")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|c| c.get("tier").and_then(Value::as_str) == Some("full"))
        .filter_map(|c| c.get("id").and_then(Value::as_str).map(str::to_owned))
        .collect()
}

pub fn contract_from(v: &Value) -> Result<TaskContract, String> {
    let criteria = array(v, "criteria")?
        .iter()
        .map(|c| {
            let id = string(c, "id")?;
            if !matches!(
                c.get("tier").map(Value::as_str),
                None | Some(Some("quick" | "full"))
            ) {
                return Err(format!("criterion {id} tier is not quick or full"));
            }
            Ok(Criterion {
                check: CheckName(string(c, "check")?),
                mandatory: field(c, "mandatory")?
                    .as_bool()
                    .ok_or("criterion mandatory is not a boolean")?,
                command: commands(c, &id)?,
                id: CriterionId(id),
            })
        })
        .collect::<Result<_, String>>()?;
    Ok(TaskContract {
        id: TaskId(string(v, "id")?),
        goal: string(v, "goal")?,
        criteria,
        forbidden_effects: strings(v, "forbidden_effects")?,
        scope: strings(v, "scope")?,
        budget: Budget {
            max_repair_rounds: small(v, "max_repair_rounds")?,
        },
    })
}

fn read_optional(path: &Path) -> Result<Option<String>, String> {
    match fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("{}: {e}", path.display())),
    }
}

/// The contract (if present) and the SHA-256 of contract.json's bytes (of nothing when absent).
pub fn read_contract(dir: &Path) -> Result<(Option<TaskContract>, String), String> {
    let (contract, digest) = read_contract_tiers(dir)?;
    Ok((contract.map(|(c, _)| c), digest))
}

/// A contract and the ids of its full-tier criteria.
pub type Tiered = (TaskContract, Vec<String>);

/// `read_contract` plus the ids of the full-tier criteria (see [`full_tier`]).
pub fn read_contract_tiers(dir: &Path) -> Result<(Option<Tiered>, String), String> {
    let path = dir.join("contract.json");
    let Some(text) = read_optional(&path)? else {
        return Ok((None, sha256::hex(b"")));
    };
    let digest = sha256::hex(text.as_bytes());
    let v: Value = serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
    let contract = contract_from(&v).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok((Some((contract, full_tier(&v))), digest))
}

/// What gate.json holds after its hash chain was checked.
// One value per hook call, moved out at once: boxing the gate would buy nothing.
#[allow(clippy::large_enum_variant)]
pub enum Stored {
    Absent,
    Valid(Gate),
    /// Names the broken link: `receipt <index>` or `checkpoint`.
    Tampered(String),
}

/// gate.json is only ever written whole by `save_gate` (temp file then rename), so a file that
/// does not parse or does not match its chain was edited by something else.
pub fn load_gate(dir: &Path, contract_sha256: &str) -> Result<Stored, String> {
    let Some(text) = read_optional(&dir.join("gate.json"))? else {
        return Ok(Stored::Absent);
    };
    let Ok(v) = serde_json::from_str::<Value>(&text) else {
        return Ok(Stored::Tampered("checkpoint".to_owned()));
    };
    if let Err(what) = chain::verify(&v, contract_sha256) {
        return Ok(Stored::Tampered(what));
    }
    Ok(Gate::from_json(&v)
        .map_or_else(|_| Stored::Tampered("checkpoint".to_owned()), Stored::Valid))
}

pub fn save_gate(dir: &Path, gate: &Gate, contract_sha256: &str) -> Result<(), String> {
    // shortcut: no lock around gate.json, so two overlapping hook calls can lose one update; add a lock file once overlapping calls show up in events.jsonl
    let path = dir.join("gate.json");
    let tmp = dir.join("gate.json.tmp");
    let mut v = gate.to_json();
    chain::seal(&mut v, contract_sha256);
    fs::write(
        &tmp,
        format!(
            "{v}
"
        ),
    )
    .map_err(|e| format!("{}: {e}", tmp.display()))?;
    fs::rename(&tmp, &path).map_err(|e| format!("{}: {e}", path.display()))
}

pub fn append_line(path: &Path, line: &Value) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .and_then(|mut file| file.write_all(format!("{line}\n").as_bytes()))
        .map_err(|e| format!("{}: {e}", path.display()))
}

/// Malformed lines are skipped: they cannot support any CHECK line.
pub(crate) fn read_runs(dir: &Path) -> Result<Vec<Run>, String> {
    let Some(text) = read_optional(&dir.join("runs.jsonl"))? else {
        return Ok(Vec::new());
    };
    Ok(text
        .lines()
        .enumerate()
        .filter_map(|(i, line)| {
            let v: Value = serde_json::from_str(line).ok()?;
            let line_ref = format!("runs.jsonl:{}", i + 1);
            Some(Run {
                agent_id: v.get("agent_id")?.as_str()?.to_owned(),
                command: v.get("command")?.as_str()?.to_owned(),
                exit_code: v
                    .get("exit_code")
                    .and_then(Value::as_i64)
                    .and_then(|n| i32::try_from(n).ok()),
                reference: v
                    .get("tool_use_id")
                    .and_then(Value::as_str)
                    .filter(|s| !s.is_empty())
                    .map_or(line_ref, str::to_owned),
                t: v.get("t").and_then(Value::as_u64).unwrap_or(0),
                candidate: v.get("candidate").and_then(|c| candidate_from(c).ok()),
            })
        })
        .collect())
}
