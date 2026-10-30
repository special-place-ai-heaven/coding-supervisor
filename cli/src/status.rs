//! Read-only effective status; installation and a connection never prove model loading.

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use supervisor_claude_code::lifecycle;
use supervisor_claude_code::location::{canonical_project, resolve_state_dir_checked};
use supervisor_claude_code::state::{Stored, load_gate, read_contract, state_name};

use crate::{Failure, Flags};

pub fn run(flags: &Flags) -> Result<(), Failure> {
    let project = PathBuf::from(canonical_project(&flags.project()?));
    let harness = flags.value("--harness").unwrap_or("claude-code");
    if !matches!(harness, "claude-code" | "codex") {
        return Err(Failure::Usage(
            "status --harness must be claude-code or codex".to_owned(),
        ));
    }
    let (root, rule) = resolve_state_dir_checked(&project, &|key| std::env::var(key).ok())
        .map_err(|_| Failure::Error("status could not resolve project state safely".to_owned()))?;
    let _transaction =
        supervisor_claude_code::transaction::Transaction::open(&root).map_err(|_| {
            Failure::Error("status could not read a consistent project snapshot".to_owned())
        })?;
    let report = inspect(&project, &root, rule, harness);
    if flags.switch("--json") {
        println!("{report}");
    } else {
        println!("{}", human(&report));
    }
    Ok(())
}

fn configuration(project: &Path, harness: &str) -> Value {
    let path = project.join(if harness == "codex" {
        ".codex/config.toml"
    } else {
        ".mcp.json"
    });
    let text = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return json!({"state":"not_configured"});
        }
        Err(_) => return json!({"state":"unknown","reason":"registration unreadable"}),
    };
    // shortcut: Codex's installer writes this exact basic-string section. Other TOML
    // representations remain unknown until a full TOML reader is justified.
    let command = if harness == "codex" {
        let mut lines = text.lines().map(str::trim);
        if !lines.any(|line| line == "[mcp_servers.supervisor]") {
            return json!({"state":"not_configured"});
        }
        lines
            .take_while(|line| !line.starts_with('['))
            .find_map(|line| {
                let (key, value) = line.split_once('=')?;
                (key.trim() == "command")
                    .then(|| serde_json::from_str::<String>(value.trim()).ok())
                    .flatten()
            })
    } else {
        match serde_json::from_str::<Value>(&text) {
            Ok(value) => {
                if value.pointer("/mcpServers/supervisor").is_none() {
                    return json!({"state":"not_configured"});
                }
                value
                    .pointer("/mcpServers/supervisor/command")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            }
            Err(_) => return json!({"state":"unknown","reason":"registration invalid"}),
        }
    };
    let Some(command) = command else {
        return json!({"state":"unknown","reason":"supervisor command unrecognized"});
    };
    let binary = Path::new(&command);
    // Do not expose arbitrary config values. Only recognize a native supervisor filename.
    let recognized = binary
        .file_name()
        .and_then(|s| s.to_str())
        .is_some_and(|name| {
            matches!(
                name,
                "supervisor" | "supervisor.exe" | "supervisor-mcp" | "supervisor-mcp.exe"
            )
        });
    json!({"state":"configured","binary":if recognized { json!(command) } else { Value::Null },"binary_exists":if recognized { json!(binary.is_file()) } else { Value::Null },"configured_version":"unknown","trust":"unknown"})
}

fn hook_configuration(project: &Path) -> Value {
    let path = project.join(".claude/settings.json");
    let value = match fs::read_to_string(path) {
        Ok(text) => match serde_json::from_str::<Value>(&text) {
            Ok(value) => value,
            Err(_) => return json!({"state":"unknown"}),
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return json!({"state":"not_configured","events":[]});
        }
        Err(_) => return json!({"state":"unknown"}),
    };
    let local = match fs::read_to_string(project.join(".claude/settings.local.json")) {
        Ok(text) => match serde_json::from_str::<Value>(&text) {
            Ok(value) => Some(value),
            Err(_) => {
                return json!({"state":"unknown","events":[],"reason":"local hook configuration invalid"});
            }
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(_) => {
            return json!({"state":"unknown","events":[],"reason":"local hook configuration unreadable"});
        }
    };
    if value.get("disableAllHooks").and_then(Value::as_bool) == Some(true)
        || local
            .as_ref()
            .and_then(|v| v.get("disableAllHooks"))
            .and_then(Value::as_bool)
            == Some(true)
    {
        return json!({"state":"disabled","events":[],"loaded_in_model_session":"unknown"});
    }
    let names = [
        "UserPromptSubmit",
        "PreToolUse",
        "PostToolUse",
        "PostToolUseFailure",
        "SubagentStop",
        "Stop",
    ];
    let mut events = Vec::new();
    let mut restricted = Vec::new();
    for event in names {
        let mut recognized = false;
        let mut unrestricted = false;
        for config in std::iter::once(&value).chain(local.as_ref()) {
            if let Some(groups) = config
                .get("hooks")
                .and_then(|h| h.get(event))
                .and_then(Value::as_array)
            {
                for group in groups {
                    if let Some(hooks) = group.get("hooks").and_then(Value::as_array) {
                        for hook in hooks {
                            if hook
                                .get("command")
                                .and_then(Value::as_str)
                                .is_some_and(configured_hook)
                            {
                                recognized = true;
                                let matcher_supported = match group.get("matcher") {
                                    None => true,
                                    Some(Value::String(matcher)) => {
                                        matches!(matcher.as_str(), "" | "*" | ".*")
                                    }
                                    _ => false,
                                };
                                if hook.get("type").and_then(Value::as_str) == Some("command")
                                    && matches!(hook.get("async"), None | Some(Value::Bool(false)))
                                    && matcher_supported
                                {
                                    unrestricted = true;
                                }
                            }
                        }
                    }
                }
            }
        }
        if unrestricted {
            events.push(event);
        } else if recognized {
            restricted.push(event);
        }
    }
    json!({"state":if !restricted.is_empty(){"partial"}else if events.is_empty(){"not_configured"}else{"configured"},"events":events,"restricted_or_unsupported_events":restricted,"global_configuration":"unobserved","loaded_in_model_session":"unknown"})
}

fn configured_hook(command: &str) -> bool {
    let Some(program) = command.trim().strip_suffix(" hook") else {
        return false;
    };
    let program = program.trim();
    let program = if program.starts_with('"') && program.ends_with('"') {
        &program[1..program.len() - 1]
    } else {
        if program.chars().any(char::is_whitespace) {
            return false;
        }
        program
    };
    if program.contains([';', '|', '&', '`', '$', '\n', '\r', '"']) {
        return false;
    }
    matches!(
        program.rsplit(['/', '\\']).next().unwrap_or(program),
        "supervisor"
            | "supervisor.exe"
            | "supervisor.cmd"
            | "supervisor-claude-code"
            | "supervisor-claude-code.exe"
    )
}

pub(crate) fn inspect(project: &Path, root: &Path, rule: &str, harness: &str) -> Value {
    let (active, provenance) = match lifecycle::active_task(root) {
        Err(_) => (
            json!({"state":"unknown","reason":"active pointer invalid"}),
            json!({"observed":0,"reconstructed":0,"asserted":0,"unknown":true}),
        ),
        Ok(None) => (
            json!({"state":if root.join("contract.json").exists() || root.join("legacy-location.json").exists(){"migration_required"}else{"inactive"}}),
            json!({"observed":0,"reconstructed":0,"asserted":0,"unknown":false}),
        ),
        Ok(Some(task)) => match read_contract(&task.dir) {
            Ok((Some(contract), digest)) => match load_gate(&task.dir, &digest) {
                Ok(Stored::Valid(gate)) => {
                    let asserted = gate
                        .receipts
                        .iter()
                        .filter(|r| r.output_ref.starts_with("mcp-asserted:"))
                        .count();
                    let reconstructed = gate
                        .receipts
                        .iter()
                        .filter(|r| {
                            r.output_ref.starts_with("tc-reconstructed:")
                                && supervisor_claude_code::adapter::receipt_is_observed(
                                    &task.dir,
                                    &task.task_id,
                                    r,
                                )
                        })
                        .count();
                    let observed = gate
                        .receipts
                        .iter()
                        .filter(|r| {
                            !r.output_ref.starts_with("mcp-asserted:")
                                && !r.output_ref.starts_with("tc-reconstructed:")
                                && supervisor_claude_code::adapter::receipt_is_observed(
                                    &task.dir,
                                    &task.task_id,
                                    r,
                                )
                        })
                        .count();
                    (
                        json!({"state":"active","task_id":task.task_id,"contract_id":contract.id.0,"gate_state":state_name(gate.checkpoint.state)}),
                        json!({"observed":observed,"reconstructed":reconstructed,"asserted":asserted,"unverified":gate.receipts.len()-asserted-observed-reconstructed,"unknown":false}),
                    )
                }
                Ok(Stored::Absent) => (
                    json!({"state":"active","task_id":task.task_id,"contract_id":contract.id.0,"gate_state":"not_started"}),
                    json!({"observed":0,"reconstructed":0,"asserted":0,"unknown":false}),
                ),
                _ => (
                    json!({"state":"unknown","reason":"task checkpoint unavailable or tampered"}),
                    json!({"observed":0,"reconstructed":0,"asserted":0,"unknown":true}),
                ),
            },
            _ => (
                json!({"state":"unknown","reason":"active contract unavailable"}),
                json!({"observed":0,"reconstructed":0,"asserted":0,"unknown":true}),
            ),
        },
    };
    let mut active = active;
    active["lifecycle_state"] = active["state"].clone();
    active["autonomous"] = false.into();
    if let Ok(Some(task)) = lifecycle::active_task(root) {
        match (
            supervisor_claude_code::autonomy::is_autonomous_task(&task.dir),
            supervisor_claude_code::autonomy::is_paused(&task.dir),
        ) {
            (Ok(autonomous), Ok(paused)) => {
                active["autonomous"] = autonomous.into();
                if paused {
                    active["lifecycle_state"] = "paused".into();
                }
            }
            _ => active["lifecycle_state"] = "unknown".into(),
        }
    }
    let hooks = if harness == "claude-code" {
        hook_configuration(project)
    } else {
        json!({"state":"unsupported","events":[]})
    };
    let required_hooks = ["PreToolUse", "Stop", "SubagentStop"].iter().all(|event| {
        hooks["events"]
            .as_array()
            .is_some_and(|events| events.iter().any(|v| v.as_str() == Some(*event)))
    });
    let mode = if harness == "claude-code" && active["state"] == "active" && required_hooks {
        "enforcing_on_named_hook_paths_if_loaded"
    } else {
        "advisory"
    };
    let completion: Vec<&str> = [
        ("PreToolUse", "PreToolUse denial"),
        (
            "Stop",
            "Stop refusal (adapter 3 refusal cap; harness cap 8)",
        ),
        (
            "SubagentStop",
            "SubagentStop refusal (adapter 3 refusal cap; harness cap 8)",
        ),
    ]
    .into_iter()
    .filter(|(event, _)| {
        hooks["events"]
            .as_array()
            .is_some_and(|events| events.iter().any(|v| v.as_str() == Some(*event)))
    })
    .map(|(_, label)| label)
    .collect();
    json!({
        "schema_version":1,"harness":harness,"project":project,"project_state":root,"location_rule":rule,
        "reporting_binary":{"path":std::env::current_exe().ok(),"version":env!("CARGO_PKG_VERSION"),"running":true},
        "registration":configuration(project,harness),"hooks":hooks,
        "connection":connection(root,harness),
        "action_policy":supervisor_claude_code::policy::configuration_status(root),
        "task":active,"mode":mode,"receipt_provenance":provenance,
        "coverage":{"completion":completion,"basis":"supported unrestricted project hook configuration; adapter fixtures and recorded Claude 2.1.312 trials; global settings and current model loading unproven","action_policy":"configured simple commands and exact tools only; see COVERAGE.md","same_user_security_boundary":false}
    })
}

fn connection(root: &Path, harness: &str) -> Value {
    let value = fs::read_to_string(root.join("connection.json"))
        .ok()
        .and_then(|text| serde_json::from_str::<Value>(&text).ok());
    let sources: Vec<Value> = value
        .as_ref()
        .and_then(Value::as_object)
        .into_iter()
        .flat_map(|object| object.iter())
        .filter_map(|(source, record)| {
            if !matches!(source.as_str(), "mcp-initialize" | "claude-hook")
                || (harness == "codex" && source == "claude-hook")
            {
                return None;
            }
            let timestamp = record.get("observed_at").and_then(Value::as_u64)?;
            Some(json!({"source":source,"observed_at":timestamp}))
        })
        .collect();
    json!({"state":if sources.is_empty(){"unobserved"}else{"observed"},"observations":sources,"harness_attribution":if harness=="codex"{"unknown (shared MCP process observation)"}else{"hook source identifies Claude; MCP source is shared"},"model_session_loaded":"unknown"})
}

fn human(report: &Value) -> String {
    let s = |pointer: &str| {
        report
            .pointer(pointer)
            .and_then(Value::as_str)
            .unwrap_or("unknown")
    };
    let count = |pointer: &str| {
        report
            .pointer(pointer)
            .and_then(Value::as_u64)
            .map_or_else(|| "unknown".to_owned(), |n| n.to_string())
    };
    let completion = report["coverage"]["completion"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_else(|| "unknown".to_owned());
    format!(
        "supervisor {} ({})\nharness: {}\nproject: {}\nregistration: {} (configured binary: {}; configured version: {})\nconnection: {}; model session loaded: {}\ntask: {} (gate: {}; id: {})\nmode: {}\nreceipts: {} observed; {} reconstructed; {} asserted; {} unverified\ncompletion coverage: {}\nbasis: {}\naction policy: {}\ncoverage: {}; same-user security boundary: false",
        s("/reporting_binary/version"),
        s("/reporting_binary/path"),
        s("/harness"),
        s("/project"),
        s("/registration/state"),
        s("/registration/binary"),
        s("/registration/configured_version"),
        s("/connection/state"),
        s("/connection/model_session_loaded"),
        report
            .pointer("/task/lifecycle_state")
            .and_then(Value::as_str)
            .unwrap_or_else(|| s("/task/state")),
        s("/task/gate_state"),
        s("/task/task_id"),
        s("/mode"),
        count("/receipt_provenance/observed"),
        count("/receipt_provenance/reconstructed"),
        count("/receipt_provenance/asserted"),
        count("/receipt_provenance/unverified"),
        completion,
        s("/coverage/basis"),
        s("/action_policy/state"),
        s("/coverage/basis")
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn human_report_does_not_promote_connection_to_loaded() {
        let report = json!({"connection":{"state":"observed","model_session_loaded":"unknown"}});
        let text = human(&report);
        assert!(text.contains("connection: observed; model session loaded: unknown"));
    }

    #[test]
    fn hook_shape_rejects_echo_and_compound_commands() {
        assert!(!configured_hook("echo supervisor hook"));
        assert!(!configured_hook("echo safe; supervisor hook"));
        assert!(configured_hook("supervisor hook"));
        assert!(configured_hook("\"C:/Program Files/supervisor.exe\" hook"));
    }

    #[test]
    fn human_report_includes_gate_state_named_coverage_and_reconstructed_count() {
        let report = json!({
            "task": {"state":"active","gate_state":"completed","task_id":"task-1"},
            "receipt_provenance": {
                "observed":1,
                "reconstructed":2,
                "asserted":3,
                "unverified":4
            },
            "coverage": {
                "completion":["PreToolUse denial","Stop refusal"],
                "basis":"fixture"
            }
        });
        let text = human(&report);
        assert!(text.contains("task: active (gate: completed; id: task-1)"));
        assert!(text.contains("receipts: 1 observed; 2 reconstructed; 3 asserted; 4 unverified"));
        assert!(text.contains("completion coverage: PreToolUse denial, Stop refusal"));
        assert!(text.contains("basis: fixture"));
    }

    #[test]
    fn inspect_separates_reconstructed_receipts_in_json() {
        use coding_supervisor::{
            CandidateId, Checkpoint, CriterionId, TaskState, VerificationReceipt,
        };
        use supervisor_claude_code::state::{Gate, save_gate};

        let root = std::env::temp_dir().join(format!(
            "supervisor-status-provenance-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        let project = root.join("project");
        fs::create_dir_all(&project).unwrap();
        let contract = json!({
            "id":"status-provenance",
            "goal":"exercise status receipt provenance",
            "criteria":[{
                "id":"C2",
                "check":"candidate_passes_reported_test",
                "mandatory":true,
                "command":"fixture-check"
            }],
            "scope":[],
            "forbidden_effects":[],
            "max_repair_rounds":1
        });
        lifecycle::start(&root, &contract, false, None, None).unwrap();
        let task = lifecycle::active_task(&root).unwrap().unwrap();
        let (_, contract_digest) = read_contract(&task.dir).unwrap();
        let cwd = canonical_project(&project);
        let candidate = CandidateId {
            base_commit: "fixture".to_owned(),
            dirty_digest: "candidate".to_owned(),
        };
        let receipt = |output_ref: String, started_at, finished_at| VerificationReceipt {
            criterion: CriterionId("C2".to_owned()),
            candidate: candidate.clone(),
            command: "fixture-check".to_owned(),
            cwd: cwd.clone(),
            exit_code: Some(0),
            output_ref,
            claim_supported: true,
            started_at,
            finished_at,
        };
        let observed_ref = format!("observed:{}:live", task.task_id);
        let reconstructed_ref = format!("tc-reconstructed:{}:recovered", task.task_id);
        let mut gate = Gate::new(
            "status-provenance".to_owned(),
            Checkpoint {
                state: TaskState::Completed,
                candidate: Some(candidate.clone()),
                open_findings: Vec::new(),
                consumed_repair_rounds: 0,
                next_action: "none".to_owned(),
            },
        );
        gate.receipts = vec![
            receipt(observed_ref, 1, 2),
            receipt(reconstructed_ref, 3, 4),
            receipt("mcp-asserted:fixture".to_owned(), 5, 6),
            receipt("unverified:fixture".to_owned(), 7, 8),
        ];
        save_gate(&task.dir, &gate, &contract_digest).unwrap();

        let completed_run = |completion_trust: &str, dispatch_id: &str, t, finished| {
            json!({
                "kind":"completed",
                "completion_trust":completion_trust,
                "execution_project":cwd,
                "execution_root":cwd,
                "execution_profile":{
                    "name":"local-default-v1",
                    "approved":true,
                    "cargo_target_sha256":null
                },
                "cwd":cwd,
                "task_id":task.task_id,
                "agent_id":"main",
                "identity_schema":3,
                "tool":"mcp__terminal_commander__run_and_watch",
                "identity":{"argv":["fixture-check"]},
                "command":"fixture-check",
                "candidate":{
                    "base_commit":"fixture",
                    "dirty_digest":"candidate"
                },
                "exit_code":0,
                "dispatch_id":dispatch_id,
                "t":t,
                "finished":finished
            })
        };
        let journal = [
            completed_run("observed", "live", 1, 2),
            completed_run("tc-reconstructed", "recovered", 3, 4),
        ]
        .into_iter()
        .map(|record| record.to_string())
        .collect::<Vec<_>>()
        .join("\n");
        fs::write(task.dir.join("runs.jsonl"), format!("{journal}\n")).unwrap();

        fs::create_dir_all(project.join(".claude")).unwrap();
        let hooks = ["PreToolUse", "Stop", "SubagentStop"]
            .into_iter()
            .map(|event| {
                (
                    event.to_owned(),
                    json!([{"hooks":[{"type":"command","command":"supervisor hook"}]}]),
                )
            })
            .collect::<serde_json::Map<_, _>>();
        fs::write(
            project.join(".claude/settings.json"),
            json!({"hooks":hooks}).to_string(),
        )
        .unwrap();

        let report = inspect(&project, &root, "fixture", "claude-code");
        assert_eq!(report["task"]["gate_state"], "completed");
        assert_eq!(report["receipt_provenance"]["observed"], 1);
        assert_eq!(report["receipt_provenance"]["reconstructed"], 1);
        assert_eq!(report["receipt_provenance"]["asserted"], 1);
        assert_eq!(report["receipt_provenance"]["unverified"], 1);
        let text = human(&report);
        assert!(text.contains("1 observed; 1 reconstructed; 1 asserted; 1 unverified"));
        assert!(text.contains("PreToolUse denial"));
        assert!(text.contains("Stop refusal"));
        let _ = fs::remove_dir_all(&root);
    }
}
