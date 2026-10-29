//! Shared test helpers: a temp project per test, a server over it, and tool calls in-process.
#![allow(dead_code)]

use std::fs;
use std::path::PathBuf;

use serde_json::{Value, json};
use supervisor_mcp::Server;

pub const GOAL: &str = "GOAL-TEXT-THAT-MUST-NOT-REACH-THE-JOURNAL";

/// A temp directory removed when the test ends.
pub struct Dir(pub PathBuf);
impl Drop for Dir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
pub fn dir(name: &str) -> Dir {
    let path = std::env::temp_dir().join(format!("supervisor-mcp-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&path);
    fs::create_dir_all(path.join("project")).unwrap();
    Dir(path)
}

/// A server over `<dir>/project` whose state lives under `<dir>/state` (SUPERVISOR_STATE_DIR).
pub fn server(d: &Dir) -> Server {
    let state = d.0.join("state").to_string_lossy().into_owned();
    Server::new(&d.0.join("project"), &move |name| {
        (name == "SUPERVISOR_STATE_DIR").then(|| state.clone())
    })
}

pub fn rpc(s: &Server, method: &str, params: Value) -> Value {
    s.handle(&json!({"jsonrpc": "2.0", "id": 7, "method": method, "params": params}))
        .unwrap()
}

/// `(isError, text)` of one tool call.
pub fn call(s: &Server, tool: &str, args: Value) -> (bool, String) {
    let reply = rpc(s, "tools/call", json!({"name": tool, "arguments": args}));
    let result = &reply["result"];
    (
        result["isError"].as_bool().unwrap(),
        result["content"][0]["text"].as_str().unwrap().to_owned(),
    )
}
pub fn ok(s: &Server, tool: &str, args: Value) -> Value {
    let (is_error, text) = call(s, tool, args);
    assert!(!is_error, "{tool} refused: {text}");
    serde_json::from_str(&text).unwrap()
}
pub fn refused(s: &Server, tool: &str, args: Value) -> String {
    let (is_error, text) = call(s, tool, args);
    assert!(is_error, "{tool} answered without refusal: {text}");
    text
}

pub fn contract(criteria: &[(&str, bool)], max_repair_rounds: u32) -> Value {
    json!({
        "id": "mcp-test",
        "goal": GOAL,
        "criteria": criteria.iter().map(|(id, m)| json!({"id": id, "check": id, "mandatory": m, "command": "cargo test"})).collect::<Vec<_>>(),
        "forbidden_effects": [],
        "scope": ["src/", "tests/"],
        "max_repair_rounds": max_repair_rounds
    })
}
pub fn four() -> Value {
    contract(&[("C1", true), ("C2", true), ("C3", true), ("C4", true)], 2)
}
pub fn observe(s: &Server, digest: &str, paths: &[&str]) -> Value {
    ok(
        s,
        "candidate_observe",
        json!({"base_commit": "base", "dirty_digest": digest, "changed_paths": paths}),
    )
}
pub fn receipt(criterion: &str, command: &str, exit_code: Value, supported: bool) -> Value {
    json!({"criterion": criterion, "command": command, "cwd": ".", "exit_code": exit_code,
           "output_ref": "out", "claim_supported": supported})
}
pub fn pass(s: &Server, criterion: &str) -> Value {
    ok(
        s,
        "receipt_record",
        receipt(criterion, "cargo test", json!(0), true),
    )
}
/// task_start with four mandatory criteria, then candidate `one` inside scope.
pub fn started(s: &Server) {
    ok(s, "task_start", four());
    observe(s, "one", &["src/lib.rs"]);
}
pub fn state(v: &Value) -> &str {
    v["checkpoint"]["state"].as_str().unwrap()
}
pub fn task_state(s: &Server) -> Value {
    ok(s, "task_state", json!({}))
}
