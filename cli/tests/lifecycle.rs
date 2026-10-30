//! Real subprocesses share one transactional task; synthetic payloads do not prove command execution.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use serde_json::{Value, json};
use supervisor_claude_code::{lifecycle, location, state};

const BIN: &str = env!("CARGO_BIN_EXE_supervisor");
struct Fixture {
    root: PathBuf,
    project: PathBuf,
    state: PathBuf,
    contract: PathBuf,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
impl Fixture {
    fn new(name: &str) -> Self {
        let root =
            std::env::temp_dir().join(format!("sup-cli-lifecycle-{name}-{}", std::process::id()));
        fs::create_dir_all(root.join("project")).unwrap();
        let contract = root.join("contract.json");
        fs::write(&contract,json!({"id":"same-contract","goal":"fixture","criteria":[{"id":"C2","check":"candidate_passes_reported_test","mandatory":true,"command":"cargo test"}],"scope":["src/"],"forbidden_effects":[],"max_repair_rounds":2}).to_string()).unwrap();
        Self {
            project: root.join("project"),
            state: root.join("state"),
            contract,
            root,
        }
    }
    fn command(&self) -> Command {
        let mut command = Command::new(BIN);
        command.env("SUPERVISOR_STATE_DIR", &self.state);
        command
    }
    fn task(&self, action: &str, extra: &[&str]) -> Output {
        self.command()
            .args(["task", action, "--project"])
            .arg(&self.project)
            .args(extra)
            .output()
            .unwrap()
    }
    fn state_root(&self) -> PathBuf {
        location::resolve_state_dir(&self.project, &|key| {
            (key == "SUPERVISOR_STATE_DIR").then(|| self.state.to_string_lossy().into_owned())
        })
        .0
    }
}
fn value(output: Output) -> Value {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}
fn rpc(project: &Path, state: &Path, tool: &str, args: Value) -> Value {
    let mut child = Command::new(BIN)
        .arg("mcp")
        .arg("--project")
        .arg(project)
        .env("SUPERVISOR_STATE_DIR", state)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    writeln!(child.stdin.take().unwrap(),"{}",json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":tool,"arguments":args}})).unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let reply: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(reply["result"]["isError"], false, "{reply}");
    serde_json::from_str(reply["result"]["content"][0]["text"].as_str().unwrap()).unwrap()
}

#[test]
fn supported_commands_complete_two_tasks_and_restore_inactive_state() {
    let f = Fixture::new("sequential");
    let one = value(f.task("start", &["--contract", f.contract.to_str().unwrap()]));
    let one = one["task_id"].as_str().unwrap();
    let mut archived = Vec::new();
    for (n, id) in [(1, one.to_owned()), (2, String::new())] {
        let id = if n == 1 {
            id
        } else {
            value(f.task(
                "next",
                &["--contract", f.contract.to_str().unwrap(), "--task", one],
            ))["task_id"]
                .as_str()
                .unwrap()
                .to_owned()
        };
        rpc(
            &f.project,
            &f.state,
            "candidate_observe",
            json!({"task_id":id,"base_commit":"fixture","dirty_digest":"same","changed_paths":[]}),
        );
        rpc(
            &f.project,
            &f.state,
            "receipt_record",
            json!({"task_id":id,"event_id":"receipt","criterion":"C2","command":"cargo test","cwd":f.project,"exit_code":0,"output_ref":"fixture-assertion","claim_supported":true}),
        );
        rpc(&f.project, &f.state, "accept_try", json!({"task_id":id}));
        let completed = rpc(
            &f.project,
            &f.state,
            "task_complete",
            json!({"task_id":id,"event_id":"complete"}),
        );
        let replay = rpc(
            &f.project,
            &f.state,
            "task_complete",
            json!({"task_id":id,"event_id":"complete"}),
        );
        assert_eq!(completed, replay);
        let task = lifecycle::active_task(&f.state_root()).unwrap().unwrap();
        archived.push((
            task.dir.clone(),
            fs::read(task.dir.join("gate.json")).unwrap(),
        ));
        if n == 2 {
            value(f.task("deactivate", &["--task", &id]));
        }
    }
    assert!(lifecycle::active_task(&f.state_root()).unwrap().is_none());
    for (path, before) in archived {
        assert_eq!(fs::read(path.join("gate.json")).unwrap(), before);
    }
}

#[test]
fn concurrent_hook_and_mcp_processes_preserve_all_findings_and_journals() {
    let f = Fixture::new("concurrent");
    let started = value(f.task("start", &["--contract", f.contract.to_str().unwrap()]));
    let id = started["task_id"].as_str().unwrap().to_owned();
    rpc(
        &f.project,
        &f.state,
        "candidate_observe",
        json!({"task_id":id,"base_commit":"fixture","dirty_digest":"same","changed_paths":[]}),
    );
    let mut children = Vec::new();
    for n in 0..8 {
        let project = f.project.clone();
        let state = f.state.clone();
        let id = id.clone();
        children.push(std::thread::spawn(move||{
            rpc(&project,&state,"finding_raise",json!({"task_id":id,"event_id":format!("finding-{n}"),"id":format!("low-{n}"),"severity":"low","location":"fixture","evidence_ref":"fixture"}));
        }));
        let project = f.project.clone();
        let state = f.state.clone();
        children.push(std::thread::spawn(move||{
            let mut child=Command::new(BIN).arg("hook").env("SUPERVISOR_STATE_DIR",&state).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
            writeln!(child.stdin.take().unwrap(),"{}",json!({"cwd":project,"session_id":format!("session-{n}"),"event_id":format!("hook-{n}"),"hook_event_name":"PreToolUse","tool_name":"Read","tool_input":{"file_path":"src/lib.rs"}})).unwrap();
            let output=child.wait_with_output().unwrap();
            assert!(output.status.success());assert!(output.stdout.is_empty(),"{}",String::from_utf8_lossy(&output.stdout));
        }));
    }
    for child in children {
        child.join().unwrap();
    }
    let task = lifecycle::active_task(&f.state_root()).unwrap().unwrap();
    let (_, digest) = state::read_contract(&task.dir).unwrap();
    let state::Stored::Valid(gate) = state::load_gate(&task.dir, &digest).unwrap() else {
        panic!("concurrent state invalid")
    };
    assert_eq!(gate.checkpoint.open_findings.len(), 8);
    let events = fs::read_to_string(task.dir.join("events.jsonl")).unwrap();
    assert_eq!(events.lines().count(), 17);
    assert!(
        events
            .lines()
            .all(|line| serde_json::from_str::<Value>(line).unwrap()["tampered"] == false)
    );
}
