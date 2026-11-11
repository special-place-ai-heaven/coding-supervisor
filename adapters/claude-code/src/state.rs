//! The files under `.supervisor/`: contract, gate state, run journal and event journal.

use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::{ErrorKind, Write};
use std::path::Path;

use coding_supervisor::{
    Approval, Budget, CandidateId, CheckName, Checkpoint, Criterion, CriterionId, Disposition,
    Finding, FindingId, Isolation, Limits, Review, ReviewDecision, ReviewProvenance, ReviewRecord,
    Severity, TaskContract, TaskId, TaskState, VerificationReceipt,
};
use serde_json::{Value, json};

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
const DECISIONS: [(ReviewDecision, &str); 2] = [
    (ReviewDecision::Approved, "approved"),
    (ReviewDecision::Rejected, "rejected"),
];
const PROVENANCES: [(ReviewProvenance, &str); 2] = [
    (ReviewProvenance::Observed, "observed"),
    (ReviewProvenance::CallerAsserted, "caller-asserted"),
];
pub fn decision_name(decision: ReviewDecision) -> &'static str {
    name(&DECISIONS, decision)
}
pub fn provenance_name(provenance: ReviewProvenance) -> &'static str {
    name(&PROVENANCES, provenance)
}

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
    /// The candidate the last repair round was charged on; a round is charged once per
    /// version of the code. Written only when set, so older gate files read unchanged.
    /// Boxed so `Stored::Valid` stays close in size to the other variants.
    pub repair_charged: Option<Box<CandidateId>>,
    /// The task's first candidate, which a baseline receipt binds to (K6). Written only when
    /// set, so older gate files read unchanged.
    pub starting: Option<Box<CandidateId>>,
    /// WP5: every review recorded, each bound to its candidate. Written only when not empty.
    pub reviews: Vec<StoredReview>,
}

/// A review as gate.json keeps it: the core record plus the worktree tree when it was recorded,
/// which lets it follow a commit of identical content. One stored with no tree is shed by a
/// commit, as a receipt without a tree is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoredReview {
    pub review: ReviewRecord,
    pub tree: Option<String>,
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
            repair_charged: None,
            starting: None,
            reviews: Vec::new(),
        }
    }

    fn to_json(&self) -> Value {
        let mut v = self.base_json();
        if let (Some(c), Some(o)) = (&self.repair_charged, v.as_object_mut()) {
            o.insert("repair_charged".to_owned(), candidate_json(c));
        }
        if let (Some(c), Some(o)) = (&self.starting, v.as_object_mut()) {
            o.insert("starting_candidate".to_owned(), candidate_json(c));
        }
        if let (false, Some(o)) = (self.reviews.is_empty(), v.as_object_mut()) {
            o.insert(
                "reviews".to_owned(),
                self.reviews.iter().map(review_json).collect(),
            );
        }
        v
    }

    fn base_json(&self) -> Value {
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
            repair_charged: match v.get("repair_charged") {
                None | Some(Value::Null) => None,
                Some(c) => Some(Box::new(candidate_from(c)?)),
            },
            starting: match v.get("starting_candidate") {
                None | Some(Value::Null) => None,
                Some(c) => Some(Box::new(candidate_from(c)?)),
            },
            reviews: match v.get("reviews") {
                None | Some(Value::Null) => Vec::new(),
                Some(_) => array(v, "reviews")?
                    .iter()
                    .map(review_from)
                    .collect::<Result<_, _>>()?,
            },
            handback_refusals: refusals
                .iter()
                .map(|(k, n)| {
                    n.as_u64()
                        .and_then(|n| u32::try_from(n).ok())
                        .map(|n| (k.clone(), n))
                        .ok_or_else(|| format!("handback_refusals.{k} is not a count"))
                })
                .collect::<Result<_, _>>()?,
        })
    }
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

fn review_json(s: &StoredReview) -> Value {
    let r = &s.review;
    obj([
        ("reviewer", r.reviewer.clone().into()),
        ("model", r.model.clone().into()),
        ("decision", decision_name(r.decision).into()),
        ("findings", r.findings.clone().into()),
        ("at", r.at.into()),
        ("candidate", candidate_json(&r.candidate)),
        ("provenance", provenance_name(r.provenance).into()),
        ("tree", s.tree.clone().into()),
    ])
}
fn review_from(v: &Value) -> Result<StoredReview, String> {
    Ok(StoredReview {
        review: ReviewRecord {
            reviewer: string(v, "reviewer")?,
            model: optional_string(v, "model", "review model")?,
            decision: lookup(&DECISIONS, &string(v, "decision")?)?,
            findings: strings(v, "findings")?,
            at: uint(v, "at")?,
            candidate: candidate_from(field(v, "candidate")?)?,
            provenance: lookup(&PROVENANCES, &string(v, "provenance")?)?,
        },
        tree: optional_string(v, "tree", "review tree")?,
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

/// The writer a migrated v1 contract records in its approval; never written to a file.
pub const V1_MIGRATION: &str = "v1-migration";
/// Journaled in `reasons` on every call that judged a migrated v1 contract.
pub const V1_MIGRATED_REASON: &str =
    "contract v1 migrated in memory: isolation none, no limits, no review requirement";

/// A contract of any schema this binary reads. `schema_version` as a JSON integer of 2 is v2
/// and above 2 is refused rather than read as v2. Anything else (absent, null, 0, 1, a string,
/// a float) is v1, because main ignored the field, and loads only through [`migrate_v1`]; the
/// file itself is never rewritten, since gate.json is chained to its bytes. Unknown fields are
/// ignored, as they always were.
pub fn contract_from(v: &Value) -> Result<TaskContract, String> {
    match present(v, "schema_version").and_then(Value::as_u64) {
        Some(2) => contract_v2(v),
        Some(n) if n > 2 => Err(format!(
            "contract schema_version {n} is newer than this supervisor {} reads (up to 2); update the supervisor",
            env!("CARGO_PKG_VERSION")
        )),
        _ => migrate_v1(v),
    }
}

/// v1 in memory: shared checkout, no limits, no review requirement, no base, and an approval
/// that says only that it was migrated. Nothing that loaded on main is refused here.
fn migrate_v1(v: &Value) -> Result<TaskContract, String> {
    Ok(TaskContract {
        approval: Some(Approval {
            by: None,
            at: None,
            via: V1_MIGRATION.to_owned(),
        }),
        migrated_from_v1: true,
        ..shared_fields(v)?
    })
}

fn contract_v2(v: &Value) -> Result<TaskContract, String> {
    let object = |key: &str| {
        present(v, key)
            .map(|x| {
                x.is_object()
                    .then_some(x)
                    .ok_or_else(|| format!("field {key} is not an object"))
            })
            .transpose()
    };
    let approval = object("approval")?
        .map(|a| {
            Ok::<_, String>(Approval {
                by: optional_string(a, "by", "approval.by")?,
                at: present(a, "at")
                    .map(|t| {
                        t.as_u64()
                            .ok_or("field approval.at is not a non-negative integer")
                    })
                    .transpose()?,
                via: string(a, "via").map_err(|e| format!("approval: {e}"))?,
            })
        })
        .transpose()?;
    let limits = object("limits")?;
    let limit = |key: &str| {
        limits
            .and_then(|l| present(l, key))
            .map(|n| {
                n.as_u64()
                    .and_then(std::num::NonZeroU64::new)
                    .ok_or_else(|| format!("field limits.{key} is not a positive integer"))
            })
            .transpose()
    };
    let review = object("review")?
        .map(|r| {
            let flag = |key: &str| {
                field(r, key)?
                    .as_bool()
                    .ok_or_else(|| format!("field review.{key} is not a boolean"))
            };
            Ok::<_, String>(Review {
                required: flag("required")?,
                independent: flag("independent")?,
            })
        })
        .transpose()?
        .unwrap_or_default();
    let isolation = match present(v, "isolation").map(Value::as_str) {
        None | Some(Some("none")) => Isolation::Shared,
        Some(Some("worktree")) => Isolation::Worktree,
        Some(_) => return Err("field isolation is not \"none\" or \"worktree\"".to_owned()),
    };
    let artifacts = match present(v, "artifacts") {
        None => Vec::new(),
        Some(_) => strings(v, "artifacts")?,
    };
    let shared = shared_fields(v)?;
    inside_repository("scope", &shared.scope)?;
    inside_repository("artifacts", &artifacts)?;
    Ok(TaskContract {
        base_sha: optional_string(v, "base_sha", "base_sha")?,
        approval,
        limits: Limits {
            wall_clock_s: limit("wall_clock_s")?,
            turns: limit("turns")?,
        },
        artifacts,
        review,
        isolation,
        ..shared
    })
}

/// A field that is there and not null.
fn present<'a>(v: &'a Value, key: &str) -> Option<&'a Value> {
    v.get(key).filter(|x| !x.is_null())
}
fn optional_string(v: &Value, key: &str, name: &str) -> Result<Option<String>, String> {
    present(v, key)
        .map(|s| {
            s.as_str()
                .map(str::to_owned)
                .ok_or_else(|| format!("field {name} is not a string"))
        })
        .transpose()
}

/// v2 only: repository-relative paths, no `..` segment (also as a `{a,b}` alternative), no
/// root, drive or UNC prefix, in either separator. This is a check of the text; scope matching
/// itself runs on canonical paths at judgement time. Nothing else is refused.
fn inside_repository(key: &str, paths: &[String]) -> Result<(), String> {
    for path in paths {
        let p = path.replace('\\', "/");
        let drive = p.as_bytes().first().is_some_and(u8::is_ascii_alphabetic)
            && p.as_bytes().get(1) == Some(&b':');
        if p.starts_with('/') || drive || p.split(['/', '{', '}', ',']).any(|s| s == "..") {
            return Err(format!(
                "{key} entry {path:?} leaves the repository; use a repository-relative path without `..`"
            ));
        }
    }
    Ok(())
}

/// The fields v1 and v2 share, with the v2 additions at their defaults.
fn shared_fields(v: &Value) -> Result<TaskContract, String> {
    let criteria = array(v, "criteria")?
        .iter()
        .map(|c| {
            let id = string(c, "id")?;
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
        base_sha: None,
        approval: None,
        limits: Limits::default(),
        artifacts: vec![],
        review: Review::default(),
        isolation: Isolation::default(),
        migrated_from_v1: false,
    })
}

/// Who writes a task's contract.json: the approval's `via`, the project whose HEAD becomes
/// `base_sha`, and the OS user, which only a person's own `supervisor task start` records (an
/// agent's environment would otherwise name a person as the approver). Recorded, not proven.
pub struct Writer<'a> {
    via: &'static str,
    project: Option<&'a Path>,
    by: Option<String>,
}
impl<'a> Writer<'a> {
    /// The project the task starts in, when the writer knows one.
    pub(crate) fn project(&self) -> Option<&'a Path> {
        self.project
    }
    /// `supervisor task start` or `task next`, run by a person.
    pub fn task_start(project: Option<&'a Path>) -> Self {
        let by = std::env::var("USERNAME")
            .or_else(|_| std::env::var("USER"))
            .ok();
        Self {
            via: "task-start",
            project,
            by,
        }
    }
    /// The MCP `task_start` tool, called by an agent.
    pub fn mcp(project: &'a Path) -> Self {
        Self {
            via: "mcp-task-start",
            project: Some(project),
            by: None,
        }
    }
    /// A prompt-bound agent proposal.
    pub fn agent(project: &'a Path) -> Self {
        Self {
            via: "agent-proposal",
            project: Some(project),
            by: None,
        }
    }
}

/// contract.json as a writer stores it: the caller's contract as v2, with `base_sha` the
/// project's HEAD (null outside a repository, before the first commit, or with no project) and
/// the writer's approval. `isolation` is `none` until worktree creation exists (WP3). The
/// caller checks the result with [`contract_from`] before saving it.
pub fn stamp_v2(contract: &Value, writer: &Writer, now: u64) -> Value {
    let head = writer
        .project
        .and_then(|p| crate::git::run(p, &["rev-parse", "--verify", "--quiet", "HEAD"]).ok())
        .filter(|out| out.status.success())
        .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_owned())
        .filter(|sha| !sha.is_empty());
    let mut c = contract.clone();
    if let Some(fields) = c.as_object_mut() {
        fields.insert("schema_version".to_owned(), 2.into());
        fields.insert("base_sha".to_owned(), head.into());
        fields.insert(
            "approval".to_owned(),
            json!({"by": writer.by, "at": now, "via": writer.via}),
        );
        fields.insert("isolation".to_owned(), "none".into());
    }
    c
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
    let path = dir.join("contract.json");
    let Some(text) = read_optional(&path)? else {
        return Ok((None, sha256::hex(b"")));
    };
    let digest = sha256::hex(text.as_bytes());
    let v: Value = serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
    let contract = contract_from(&v).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok((Some(contract), digest))
}

/// M6: each criterion's `min_tests` from the task's contract.json; read beside the core
/// contract, which does not carry it.
pub fn min_tests(dir: &Path) -> Result<BTreeMap<String, u64>, String> {
    let Some(text) = read_optional(&dir.join("contract.json"))? else {
        return Ok(BTreeMap::new());
    };
    let v: Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    Ok(v["criteria"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|c| Some((c["id"].as_str()?.to_owned(), c["min_tests"].as_u64()?)))
        .collect())
}

/// What gate.json holds after its hash chain was checked.
pub enum Stored {
    Absent,
    /// Boxed: the gate is far larger than the other variants.
    Valid(Box<Gate>),
    /// Names the broken link: `receipt <index>` or `checkpoint`.
    Tampered(String),
}

/// gate.json is only ever written whole by `save_gate` (temp file then rename), so a file that
/// does not parse or does not match its chain was edited by something else.
pub fn load_gate(dir: &Path, contract_sha256: &str) -> Result<Stored, String> {
    load_gate_head(dir, contract_sha256).map(|(stored, _)| stored)
}

/// WP6: [`load_gate`] plus the chain head it verified (`checkpoint_hash`), from one read. The
/// head is `Some` exactly when the gate is `Valid`.
pub fn load_gate_head(
    dir: &Path,
    contract_sha256: &str,
) -> Result<(Stored, Option<String>), String> {
    let Some(text) = read_optional(&dir.join("gate.json"))? else {
        return Ok((Stored::Absent, None));
    };
    let tampered = |what: String| Ok((Stored::Tampered(what), None));
    let Ok(v) = serde_json::from_str::<Value>(&text) else {
        return tampered("checkpoint".to_owned());
    };
    if let Err(what) = chain::verify(&v, contract_sha256) {
        return tampered(what);
    }
    match (Gate::from_json(&v), v["checkpoint_hash"].as_str()) {
        (Ok(gate), Some(head)) => Ok((Stored::Valid(Box::new(gate)), Some(head.to_owned()))),
        _ => tampered("checkpoint".to_owned()),
    }
}

pub fn save_gate(dir: &Path, gate: &Gate, contract_sha256: &str) -> Result<(), String> {
    crate::transaction::atomic_write(
        &dir.join("gate.json"),
        gate_text(gate, contract_sha256).as_bytes(),
    )
}

/// gate.json's sealed text, for writers that commit it through a `Transaction`.
pub fn gate_text(gate: &Gate, contract_sha256: &str) -> String {
    let mut v = gate.to_json();
    chain::seal(&mut v, contract_sha256);
    format!("{v}\n")
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
