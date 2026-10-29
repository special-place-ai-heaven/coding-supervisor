use std::fs;
use std::path::{Path, PathBuf};

use serde_json::json;
use supervisor_claude_code::adapter::Decision;
use supervisor_claude_code::guard::state_write_denial;
use supervisor_claude_code::location::{project_key, resolve_state_dir};

struct Dir(PathBuf);
impl Drop for Dir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn dir(name: &str) -> Dir {
    let path = std::env::temp_dir().join(format!("supervisor-loc-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&path);
    fs::create_dir_all(&path).unwrap();
    Dir(path)
}

fn only<'a>(name: &'a str, value: &'a str) -> impl Fn(&str) -> Option<String> + 'a {
    move |k| (k == name).then(|| value.to_owned())
}

const CWD: &str = "E:\\project\\x";

#[cfg(windows)]
#[test]
fn project_key_replaces_non_alphanumerics() {
    assert_eq!(project_key(Path::new(CWD)), "E--project-x");
}

#[test]
fn env_variable_rule_wins() {
    let d = dir("env");
    let base = d.0.to_string_lossy().into_owned();
    let lookup = |k: &str| match k {
        "SUPERVISOR_STATE_DIR" => Some(base.clone()),
        "XDG_STATE_HOME" | "HOME" | "LOCALAPPDATA" => Some("ignored".to_owned()),
        _ => None,
    };
    let (path, rule) = resolve_state_dir(Path::new(CWD), &lookup);
    assert_eq!(rule, "env");
    assert_eq!(path, d.0.join(project_key(Path::new(CWD))));
    assert!(path.is_dir());
}

#[test]
fn empty_env_variable_is_ignored() {
    let d = dir("empty");
    let base = d.0.to_string_lossy().into_owned();
    let lookup = |k: &str| match k {
        "SUPERVISOR_STATE_DIR" => Some(String::new()),
        "XDG_STATE_HOME" => Some(base.clone()),
        _ => None,
    };
    let (_, rule) = resolve_state_dir(Path::new(CWD), &lookup);
    assert_eq!(rule, "xdg");
}

#[cfg(windows)]
#[test]
fn local_app_data_rule() {
    let d = dir("lad");
    let base = d.0.to_string_lossy().into_owned();
    let (path, rule) = resolve_state_dir(Path::new(CWD), &only("LOCALAPPDATA", &base));
    assert_eq!(rule, "local-app-data");
    assert_eq!(
        path,
        d.0.join("coding-supervisor")
            .join("state")
            .join("E--project-x")
    );
}

#[test]
fn xdg_and_home_rules() {
    let d = dir("xdg");
    let base = d.0.to_string_lossy().into_owned();
    let key = project_key(Path::new(CWD));
    let (path, rule) = resolve_state_dir(Path::new(CWD), &only("XDG_STATE_HOME", &base));
    assert_eq!(rule, "xdg");
    assert_eq!(path, d.0.join("coding-supervisor").join("state").join(&key));
    let (path, rule) = resolve_state_dir(Path::new(CWD), &only("HOME", &base));
    assert_eq!(rule, "home");
    assert_eq!(
        path,
        d.0.join(".local")
            .join("state")
            .join("coding-supervisor")
            .join("state")
            .join(&key)
    );
}

#[test]
fn cwd_fallback_when_nothing_is_set_or_creation_fails() {
    let (path, rule) = resolve_state_dir(Path::new(CWD), &|_| None);
    assert_eq!(
        (path, rule),
        (Path::new(CWD).join(".supervisor"), "cwd-fallback")
    );
    // A base that is a file cannot hold a directory.
    let d = dir("file");
    let file = d.0.join("not-a-dir");
    fs::write(&file, "x").unwrap();
    let base = file.to_string_lossy().into_owned();
    let (path, rule) = resolve_state_dir(Path::new(CWD), &only("SUPERVISOR_STATE_DIR", &base));
    assert_eq!(
        (path, rule),
        (Path::new(CWD).join(".supervisor"), "cwd-fallback")
    );
}

const STATE: &str = "C:\\Users\\u\\AppData\\Local\\coding-supervisor\\state\\k";

fn call(tool: &str, input: serde_json::Value) -> Option<Decision> {
    state_write_denial(
        &json!({"tool_name": tool, "tool_input": input}),
        Path::new(STATE),
    )
}
fn denied(d: Option<Decision>) -> String {
    match d {
        Some(Decision::Deny(reason)) => reason,
        other => panic!("expected a deny, got {other:?}"),
    }
}

#[test]
fn write_into_a_supervisor_directory_is_denied() {
    let r = denied(call(
        "Write",
        json!({"file_path": "E:\\x\\.supervisor\\gate.json"}),
    ));
    assert_eq!(
        r,
        "GATE: Write refused: the gate's own state is not writable by agents (E:\\x\\.supervisor\\gate.json)"
    );
}

#[test]
fn edit_inside_the_state_dir_is_denied() {
    let r = denied(call(
        "Edit",
        json!({"file_path": "c:/users/U/AppData/Local/coding-supervisor/state/k/contract.json"}),
    ));
    assert!(r.starts_with("GATE: Edit refused"));
}

#[test]
fn bash_touching_supervisor_is_denied() {
    let r = denied(call(
        "Bash",
        json!({"command": "echo 1 >> .supervisor/gate.json"}),
    ));
    assert!(r.ends_with("(.supervisor)"));
}

#[test]
fn bash_naming_the_state_dir_is_denied() {
    let command = format!("type {STATE}\\gate.json");
    let r = denied(call("Bash", json!({ "command": command })));
    assert!(r.starts_with("GATE: Bash refused"));
}

#[test]
fn ordinary_calls_are_allowed() {
    assert_eq!(
        call("Write", json!({"file_path": "E:\\x\\src\\lib.rs"})),
        None
    );
    assert_eq!(call("Bash", json!({"command": "cargo test"})), None);
    assert_eq!(
        call("Write", json!({"file_path": "E:\\x\\.supervisord\\conf"})),
        None
    );
}
