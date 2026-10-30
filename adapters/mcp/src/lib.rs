//! MODE: advisory via MCP on every harness; the server answers and never blocks; receipts are caller-asserted (the server does not observe commands). On Claude Code, the hook adapter ignores these receipts when it decides whether a completion may stop. Journal format v1.
//!
//! Model Context Protocol server over the completion gate: newline-delimited JSON-RPC 2.0 on
//! stdin/stdout. It shares contract.json, gate.json and events.jsonl with the Claude Code
//! adapter, so one project has one gate whichever path writes it.
//!
//! ## Receipts through MCP
//!
//! A receipt recorded with `receipt_record` is stored with an `output_ref` that starts with
//! `mcp-asserted:`. This server counts it in its own advisory answers; the Claude Code hook
//! adapter leaves it out when it decides acceptance and journals it as an unlinked CHECK.
#![forbid(unsafe_code)]

use std::fs;
use std::io::{self, BufRead, ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use coding_supervisor::{
    Candidate, CandidateId, Criterion, CriterionId, Disposition, Finding, FindingId, GateError,
    ReceiptStatus, Severity, TaskState, VerificationReceipt,
};
use serde_json::{Map, Value};
use supervisor_claude_code::adapter::{MCP_ASSERTED, Session, now, render};
use supervisor_claude_code::candidate::{CandidateProvider, GitCandidate};
use supervisor_claude_code::journal::{self, Trace};
use supervisor_claude_code::lifecycle;
use supervisor_claude_code::location::resolve_state_dir;
use supervisor_claude_code::state::{
    self, Gate, Stored, candidate_json, finding_json, obj, state_name,
};

const PROTOCOL_VERSIONS: [&str; 3] = ["2024-11-05", "2025-03-26", "2025-06-18"];
const DEFAULT_PROTOCOL: &str = "2025-06-18";
const TOOLS: &str = include_str!("tools.json");
/// The last candidate reported through `candidate_observe`. gate.json keeps only the identity,
/// and the scope check needs the changed paths.
// shortcut: outside the hash chain; a hand edit can change the paths, which the caller can assert anyway. Move it into gate.json when the gate format next changes.
const CANDIDATE_FILE: &str = "mcp-candidate.json";
const USAGE: &str = "usage: [--project <dir>]  (the project defaults to the current directory)";

/// The tool definitions served by `tools/list`.
pub fn tools() -> Value {
    serde_json::from_str(TOOLS).unwrap_or(Value::Null)
}

/// Serve on stdin/stdout with the process environment. `args` is `[]` or `["--project", dir]`.
pub fn run_stdio(args: &[String]) -> ExitCode {
    let project = match args {
        [] => std::env::current_dir().map_err(|e| e.to_string()),
        [flag, dir] if flag == "--project" => Ok(PathBuf::from(dir)),
        _ => Err(USAGE.to_owned()),
    };
    let project = match project {
        Ok(p) => p,
        Err(why) => {
            let _ = writeln!(io::stderr(), "{why}");
            return ExitCode::from(2);
        }
    };
    let server = Server::new(&project, &|name| std::env::var(name).ok());
    match serve(&server, io::stdin().lock(), io::stdout().lock()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            let _ = writeln!(io::stderr(), "stdio failed: {e}");
            ExitCode::from(1)
        }
    }
}

/// One JSON-RPC message per input line; each reply on its own output line.
pub fn serve(server: &Server, input: impl BufRead, mut output: impl Write) -> io::Result<()> {
    for line in input.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        if let Some(reply) = server.handle_line(&line) {
            writeln!(output, "{reply}")?;
            output.flush()?;
        }
    }
    Ok(())
}

pub struct Server {
    /// Where `candidate_observe` reads git when the caller gives no identity.
    project: PathBuf,
    state_dir: PathBuf,
    state_location: &'static str,
    state_error: Option<String>,
}

/// A validated tool input.
enum Op {
    Observe(Candidate),
    Receipt {
        criterion: String,
        command: String,
        cwd: String,
        exit_code: Option<i32>,
        output_ref: String,
        claim_supported: bool,
    },
    Finding {
        id: String,
        severity: Severity,
        location: String,
        evidence_ref: String,
    },
    Repair,
    Accept,
    Complete,
    State,
}

impl Server {
    /// The state directory is resolved as the Claude Code adapter resolves it, from `project`
    /// and the variables `env_lookup` returns.
    pub fn new(project: &Path, env_lookup: &dyn Fn(&str) -> Option<String>) -> Self {
        let (state_dir, state_location) = resolve_state_dir(project, env_lookup);
        let state_error =
            supervisor_claude_code::location::prepare_state(&state_dir, project).err();
        Self {
            project: project.to_path_buf(),
            state_dir,
            state_location,
            state_error,
        }
    }

    pub fn state_dir(&self) -> &Path {
        &self.state_dir
    }

    pub fn task_dir(&self) -> Result<PathBuf, String> {
        Ok(lifecycle::active_task(&self.state_dir)?
            .map_or_else(|| self.state_dir.clone(), |t| t.dir))
    }

    /// One input line in, at most one reply line out.
    pub fn handle_line(&self, line: &str) -> Option<String> {
        let reply = match serde_json::from_str::<Value>(line) {
            Ok(message) => self.handle(&message)?,
            Err(e) => rpc_error(Value::Null, -32700, &format!("parse error: {e}")),
        };
        Some(reply.to_string())
    }

    /// One JSON-RPC message in; `None` for notifications and for responses from the client.
    pub fn handle(&self, message: &Value) -> Option<Value> {
        let Some(request) = message.as_object() else {
            return Some(rpc_error(
                Value::Null,
                -32600,
                "message is not a JSON object",
            ));
        };
        let id = request.get("id").cloned();
        let Some(method) = request.get("method").and_then(Value::as_str) else {
            let is_response = request.contains_key("result") || request.contains_key("error");
            return id
                .filter(|_| !is_response)
                .map(|id| rpc_error(id, -32600, "request has no method"));
        };
        // Notifications (`notifications/initialized` and any other) never get a reply.
        let id = id?;
        let params = request.get("params").unwrap_or(&Value::Null);
        let result = match method {
            "initialize" => self
                .state_error
                .clone()
                .map_or_else(
                    || lifecycle::observe_connection(&self.state_dir, "mcp-initialize"),
                    Err,
                )
                .map(|()| initialize(params))
                .map_err(|e| (-32603, e)),
            "ping" => Ok(obj([])),
            "tools/list" => Ok(obj([("tools", tools())])),
            "tools/call" => self.tools_call(params),
            _ => Err((-32601, format!("method not found: {method}"))),
        };
        Some(match result {
            Ok(result) => obj([("jsonrpc", "2.0".into()), ("id", id), ("result", result)]),
            Err((code, message)) => rpc_error(id, code, &message),
        })
    }

    fn tools_call(&self, params: &Value) -> Result<Value, (i64, String)> {
        let name = params
            .get("name")
            .and_then(Value::as_str)
            .ok_or((-32602, "params.name is missing or not a string".to_owned()))?;
        let known = tools()
            .as_array()
            .is_some_and(|t| t.iter().any(|t| t.get("name") == Some(&name.into())));
        if !known {
            return Err((-32602, format!("unknown tool: {name}")));
        }
        let empty = Value::Object(Map::new());
        let args = match params.get("arguments") {
            None | Some(Value::Null) => &empty,
            Some(args) => args,
        };
        Ok(self.call(name, args))
    }

    /// Run one tool and journal it. Every outcome is an answer; nothing is blocked.
    fn call(&self, tool: &str, args: &Value) -> Value {
        if let Some(what) = &self.state_error {
            return tool_result(&format!("GATE (advisory): state unavailable: {what}"), true);
        }
        let started = if tool == "task_start" {
            match lifecycle::start(&self.state_dir, args, false, None, None) {
                Ok(started) => Some(started),
                Err(what) => {
                    return tool_result(
                        &format!("GATE (advisory): task_start refused: {what}"),
                        true,
                    );
                }
            }
        } else {
            None
        };
        let expected = started
            .as_ref()
            .and_then(|v| v.get("task_id"))
            .and_then(Value::as_str)
            .or_else(|| args.get("task_id").and_then(Value::as_str));
        // Reading state discovers the active identity; every mutation must name that identity.
        let active_id = if tool == "task_state" {
            lifecycle::active_task(&self.state_dir)
                .ok()
                .flatten()
                .map(|t| t.task_id)
        } else {
            None
        };
        let expected = expected.or(active_id.as_deref());
        let event_id = args
            .get("event_id")
            .and_then(Value::as_str)
            .map(|id| format!("mcp:{tool}:{id}"));
        let candidate = GitCandidate {
            cwd: self.project.clone(),
        }
        .observe()
        .map(|c| format!("{}:{}", c.id.base_commit, c.id.dirty_digest))
        .unwrap_or_else(|_| "unobserved".to_owned());
        let digest = supervisor_claude_code::sha256::hex(format!("{args}:{candidate}").as_bytes());
        match lifecycle::with_task(
            &self.state_dir,
            expected,
            None,
            event_id.as_deref().map(|id| (id, digest.as_str())),
            |dir| {
                let local = Self {
                    project: self.project.clone(),
                    state_dir: dir.to_path_buf(),
                    state_location: self.state_location,
                    state_error: None,
                };
                let mut response = local.call_inner(
                    if started.is_some() {
                        "task_state"
                    } else {
                        tool
                    },
                    args,
                );
                if let Some(content) = response
                    .get_mut("content")
                    .and_then(Value::as_array_mut)
                    .and_then(|a| a.first_mut())
                    && let Some(text) = content.get("text").and_then(Value::as_str)
                    && let Ok(mut details) = serde_json::from_str::<Value>(text)
                {
                    if let Some(id) = expected {
                        details["task_id"] = id.into();
                    }
                    if started.is_some() {
                        details["started"] = true.into();
                    }
                    content["text"] = details.to_string().into();
                }
                Ok(response)
            },
        ) {
            Ok(response) => response,
            Err(what) => tool_result(&format!("GATE (advisory): {tool} refused: {what}"), true),
        }
    }

    fn call_inner(&self, tool: &str, args: &Value) -> Value {
        let mut reasons = Vec::new();
        let mut trace = Trace::default();
        let (text, is_error) = match self.run(tool, args, &mut reasons, &mut trace) {
            Ok(text) => (text, false),
            Err(why) => (format!("GATE (advisory): {tool} refused: {why}"), true),
        };
        let payload = obj([("hook_event_name", format!("mcp:{tool}").into())]);
        let decision = is_error.then(|| obj([("systemMessage", text.clone().into())]));
        let mut line = journal::line(
            &payload,
            self.state_location,
            now(),
            decision,
            &reasons,
            &trace,
        );
        if let Some(fields) = line.as_object_mut() {
            fields.insert("agent_id".to_owned(), Value::Null);
        }
        match state::append_line(&self.state_dir.join("events.jsonl"), &line) {
            Ok(()) => tool_result(&text, is_error),
            Err(what) => tool_result(
                &format!("{text}\nGATE: events journal not written: {what}"),
                true,
            ),
        }
    }

    fn run(
        &self,
        tool: &str,
        args: &Value,
        reasons: &mut Vec<String>,
        trace: &mut Trace,
    ) -> Result<String, String> {
        let dir = &self.state_dir;
        let op = parse(tool, args, &self.project)?;
        let (contract, digest) = state::read_contract(dir)?;
        trace.task_id = contract
            .as_ref()
            .map(|c| lifecycle::task_identity(dir, &c.id.0))
            .transpose()?;
        let stored = load(dir, &digest, trace)?;
        let Some(contract) = contract else {
            return Err("no contract in the gate's state directory; call task_start".to_owned());
        };
        trace.state_before = stored.as_ref().map(|g| g.checkpoint.state);
        let observed = match &op {
            Op::Observe(candidate) => Ok(candidate.clone()),
            _ => self.known_candidate(stored.as_ref()).ok_or_else(|| {
                "candidate not observed through MCP since the gate last changed".to_owned()
            }),
        };
        let candidate_known = observed.is_ok();
        let criteria = contract.criteria.clone();
        if let Op::Receipt {
            criterion,
            command,
            claim_supported,
            exit_code,
            ..
        } = &op
        {
            let required = criteria
                .iter()
                .find(|c| c.id.0 == *criterion)
                .ok_or_else(|| "unknown criterion (unapproved identifier omitted)".to_owned())?;
            let rejection = if !claim_supported {
                Some(GateError::ReceiptUnsupported)
            } else if exit_code.is_none() {
                Some(GateError::ReceiptMissingExitCode)
            } else if !required.accepts(command) {
                Some(GateError::WrongCommand {
                    criterion: required.id.clone(),
                    cited: String::new(),
                    required: required.command.clone(),
                })
            } else {
                None
            };
            if let Some(error) = rejection {
                let safe_command = if required.accepts(command) {
                    command.as_str()
                } else {
                    "<unapproved command omitted>"
                };
                let exit = exit_code.map_or("none".to_owned(), |n| n.to_string());
                let refusal = render(&error);
                reasons.push(format!(
                    "rejected CHECK {} | {safe_command} | exit {exit}: {refusal} (mcp-asserted)",
                    required.id.0
                ));
                return Err(refusal);
            }
        }
        let mut session = Session::open(contract, stored, observed, true, reasons)?;
        let result = apply(&op, &mut session, reasons);
        session.gate.checkpoint = session.task.checkpoint();
        trace.state_after = Some(session.task.state());
        trace.candidate = session.task.candidate().cloned();
        state::save_gate(dir, &session.gate, &digest)?;
        if let (Op::Observe(candidate), Ok(())) = (&op, &result) {
            self.keep_candidate(candidate)?;
        }
        result?;
        let rejected =
            self.rejected_claims(&lifecycle::task_identity(dir, &session.gate.contract_id)?)?;
        Ok(describe(&session, &criteria, candidate_known, rejected).to_string())
    }

    /// The kept candidate, when it is still the identity gate.json holds.
    fn known_candidate(&self, stored: Option<&Gate>) -> Option<Candidate> {
        let text = fs::read_to_string(self.state_dir.join(CANDIDATE_FILE)).ok()?;
        let v: Value = serde_json::from_str(&text).ok()?;
        let candidate = Candidate {
            id: CandidateId {
                base_commit: string(&v, "base_commit").ok()?,
                dirty_digest: string(&v, "dirty_digest").ok()?,
            },
            provenance: string(&v, "provenance").ok()?,
            changed_paths: strings(&v, "changed_paths").ok()?,
        };
        (stored?.checkpoint.candidate.as_ref() == Some(&candidate.id)).then_some(candidate)
    }

    fn keep_candidate(&self, c: &Candidate) -> Result<(), String> {
        let path = self.state_dir.join(CANDIDATE_FILE);
        let v = obj([
            ("base_commit", c.id.base_commit.clone().into()),
            ("dirty_digest", c.id.dirty_digest.clone().into()),
            ("provenance", c.provenance.clone().into()),
            ("changed_paths", c.changed_paths.clone().into()),
        ]);
        fs::write(&path, format!("{v}\n")).map_err(|e| format!("{}: {e}", path.display()))
    }

    /// Receipt claims the gate rejected for this task, read back from events.jsonl so they
    /// survive a restart. Includes rejections journaled by the Claude Code adapter.
    // shortcut: reads the whole journal on every answer; index it once journals reach megabytes.
    fn rejected_claims(&self, task_id: &str) -> Result<Vec<String>, String> {
        let path = self.state_dir.join("events.jsonl");
        let text = match fs::read_to_string(&path) {
            Ok(text) => text,
            Err(e) if e.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(format!("{}: {e}", path.display())),
        };
        Ok(text
            .lines()
            .filter_map(|line| serde_json::from_str::<Value>(line).ok())
            .filter(|v| v.get("task_id").and_then(Value::as_str) == Some(task_id))
            .flat_map(|v| strings(&v, "reasons").unwrap_or_default())
            .filter(|r| r.starts_with("rejected CHECK "))
            .collect())
    }
}

/// gate.json after its hash chain was checked; a broken chain is refused and left untouched.
fn load(dir: &Path, digest: &str, trace: &mut Trace) -> Result<Option<Gate>, String> {
    match state::load_gate(dir, digest)? {
        Stored::Tampered(what) => {
            trace.tampered = true;
            Err(format!(
                "gate state tampered ({what}); restore the state from the journal or start a new task"
            ))
        }
        Stored::Valid(gate) => Ok(Some(gate)),
        Stored::Absent => Ok(None),
    }
}

fn apply(op: &Op, session: &mut Session, reasons: &mut Vec<String>) -> Result<(), String> {
    let task = &mut session.task;
    let current = task
        .candidate()
        .cloned()
        .ok_or(GateError::CandidateNotObserved);
    let outcome = match op {
        Op::Observe(c) => {
            reasons.push(format!("candidate observed, provenance {}", c.provenance));
            if task.state() == TaskState::Repairing {
                reasons.push("repair round finished".to_owned());
                task.finish_repair()
            } else {
                Ok(())
            }
        }
        Op::Receipt {
            criterion,
            command,
            cwd,
            exit_code,
            output_ref,
            claim_supported,
        } => {
            let exit = exit_code.map_or("none".to_owned(), |c| c.to_string());
            let cited = format!("CHECK {criterion} | {command} | exit {exit}");
            let t = now();
            let receipt = current.map(|candidate| VerificationReceipt {
                criterion: CriterionId(criterion.clone()),
                candidate,
                command: command.clone(),
                cwd: cwd.clone(),
                exit_code: *exit_code,
                output_ref: format!("{MCP_ASSERTED}{output_ref}"),
                claim_supported: *claim_supported,
                started_at: t,
                finished_at: t,
            });
            match receipt.and_then(|r| task.record_receipt(r.clone()).map(|()| r)) {
                Ok(r) => {
                    reasons.push(format!("recorded {cited} (mcp-asserted)"));
                    session.gate.receipts.push(r);
                    Ok(())
                }
                Err(e) => {
                    reasons.push(format!("rejected {cited}: {} (mcp-asserted)", render(&e)));
                    Err(e)
                }
            }
        }
        Op::Finding {
            id,
            severity,
            location,
            evidence_ref,
        } => current.and_then(|candidate| {
            task.raise_finding(Finding {
                id: FindingId(id.clone()),
                candidate,
                severity: *severity,
                location: location.clone(),
                evidence_ref: evidence_ref.clone(),
                disposition: Disposition::Open,
            })
        }),
        Op::Repair => task.request_repair(),
        Op::Accept => task.try_accept(),
        Op::Complete => task.complete(),
        Op::State => Ok(()),
    };
    outcome.map_err(|e| render(&e))
}

fn describe(
    session: &Session,
    criteria: &[Criterion],
    candidate_known: bool,
    rejected_claims: Vec<String>,
) -> Value {
    let task = &session.task;
    let cp = task.checkpoint();
    let receipts: Map<String, Value> = criteria
        .iter()
        .map(|c| {
            let status = match task.receipt_status(&c.id) {
                Some(ReceiptStatus::Current) => "current",
                Some(ReceiptStatus::Stale) => "stale",
                None => "missing",
            };
            (c.id.0.clone(), status.into())
        })
        .collect();
    obj([
        ("task_id", session.gate.contract_id.clone().into()),
        (
            "checkpoint",
            obj([
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
                ("next_action", cp.next_action.into()),
            ]),
        ),
        ("candidate_observed", candidate_known.into()),
        // The chair cannot read contract.json (the hooks deny the state directory), so the
        // table it hands a worker comes from here.
        (
            "criteria",
            criteria
                .iter()
                .map(|c| {
                    obj([
                        ("id", c.id.0.clone().into()),
                        ("command", c.command.clone().into()),
                        ("mandatory", c.mandatory.into()),
                    ])
                })
                .collect(),
        ),
        ("receipts", Value::Object(receipts)),
        ("rejected_claims", rejected_claims.into()),
    ])
}

fn parse(tool: &str, args: &Value, project: &Path) -> Result<Op, String> {
    if !args.is_object() {
        return Err("field arguments is not an object".to_owned());
    }
    Ok(match tool {
        "candidate_observe" => Op::Observe(candidate_from_args(args, project)?),
        "receipt_record" => Op::Receipt {
            criterion: string(args, "criterion")?,
            command: string(args, "command")?,
            cwd: string(args, "cwd")?,
            exit_code: match field(args, "exit_code")? {
                Value::Null => None,
                n => Some(
                    n.as_i64()
                        .and_then(|n| i32::try_from(n).ok())
                        .ok_or("field exit_code is not a 32-bit integer or null")?,
                ),
            },
            output_ref: string(args, "output_ref")?,
            claim_supported: field(args, "claim_supported")?
                .as_bool()
                .ok_or("field claim_supported is not a boolean")?,
        },
        "finding_raise" => Op::Finding {
            id: string(args, "id")?,
            severity: match string(args, "severity")?.as_str() {
                "low" => Severity::Low,
                "medium" => Severity::Medium,
                "high" => Severity::High,
                "critical" => Severity::Critical,
                _ => {
                    return Err(
                        "field severity is not one of low, medium, high, critical".to_owned()
                    );
                }
            },
            location: string(args, "location")?,
            evidence_ref: string(args, "evidence_ref")?,
        },
        "repair_request" => Op::Repair,
        "accept_try" => Op::Accept,
        "task_complete" => Op::Complete,
        _ => Op::State,
    })
}

/// The caller's candidate, or, without `base_commit` and `dirty_digest`, the one the hooks
/// derive from git in `project`; given `changed_paths` and `provenance` still apply.
fn candidate_from_args(args: &Value, project: &Path) -> Result<Candidate, String> {
    let given = |key: &str| !matches!(args.get(key), None | Some(Value::Null));
    let mut candidate = match (given("base_commit"), given("dirty_digest")) {
        (true, true) => Candidate {
            id: CandidateId {
                base_commit: string(args, "base_commit")?,
                dirty_digest: string(args, "dirty_digest")?,
            },
            provenance: "mcp-caller".to_owned(),
            changed_paths: strings(args, "changed_paths")?,
        },
        (false, false) => {
            let mut derived = GitCandidate {
                cwd: project.to_path_buf(),
            }
            .observe()
            .map_err(|why| {
                format!("candidate not derived from git ({why}); pass base_commit and dirty_digest")
            })?;
            if given("changed_paths") {
                derived.changed_paths = strings(args, "changed_paths")?;
            }
            derived
        }
        (true, false) => {
            return Err("missing field dirty_digest (give both identity fields or neither)".into());
        }
        (false, true) => {
            return Err("missing field base_commit (give both identity fields or neither)".into());
        }
    };
    if given("provenance") {
        candidate.provenance = string(args, "provenance")?;
    }
    Ok(candidate)
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
fn strings(v: &Value, key: &str) -> Result<Vec<String>, String> {
    field(v, key)?
        .as_array()
        .and_then(|a| a.iter().map(|s| s.as_str().map(str::to_owned)).collect())
        .ok_or_else(|| format!("field {key} is not an array of strings"))
}

fn initialize(params: &Value) -> Value {
    let requested = params.get("protocolVersion").and_then(Value::as_str);
    let version = requested
        .filter(|v| PROTOCOL_VERSIONS.contains(v))
        .unwrap_or(DEFAULT_PROTOCOL);
    obj([
        ("protocolVersion", version.into()),
        ("capabilities", obj([("tools", obj([]))])),
        (
            "serverInfo",
            obj([
                ("name", "supervisor-mcp".into()),
                ("version", env!("CARGO_PKG_VERSION").into()),
            ]),
        ),
    ])
}

fn tool_result(text: &str, is_error: bool) -> Value {
    obj([
        (
            "content",
            vec![obj([("type", "text".into()), ("text", text.into())])].into(),
        ),
        ("isError", is_error.into()),
    ])
}

fn rpc_error(id: Value, code: i64, message: &str) -> Value {
    obj([
        ("jsonrpc", "2.0".into()),
        ("id", id),
        (
            "error",
            obj([("code", code.into()), ("message", message.into())]),
        ),
    ])
}
