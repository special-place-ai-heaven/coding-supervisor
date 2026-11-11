//! WP7 test fixture shared by `receipt_verify.rs` and `receipt_in_tree.rs`: a temporary git
//! project, a person's task started with the real binary, real hook calls that reach `completed`
//! on hook-observed receipts, and the file key store in the temporary directory (never the real
//! one).
#![allow(dead_code)]

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use serde_json::{Value, json};

pub const BIN: &str = env!("CARGO_BIN_EXE_supervisor");
pub const BANNED: [&str; 3] = ["missing", "not found", "no such file"];
/// Set as the OS user of the person's `task start`; it must never reach a bundle.
pub const USER_MARKER: &str = "osuser-marker-7f3a";

pub struct Fixture {
    pub root: PathBuf,
    pub project: PathBuf,
    pub state: PathBuf,
    pub key: PathBuf,
    next_tool: std::cell::Cell<u32>,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

impl Fixture {
    /// A repository with `src/lib.rs`, `README.md` and a docs-paths list naming `README.md`.
    pub fn new(name: &str) -> Self {
        Self::new_with(name, "one\n")
    }

    /// [`Fixture::new`] with `readme` as the first commit's README, so two fixtures made in the
    /// same second still get different root commits.
    pub fn new_with(name: &str, readme: &str) -> Self {
        let root = std::env::temp_dir().join(format!("sup-wp7-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("project").join("src")).unwrap();
        fs::create_dir_all(root.join("project").join(".github")).unwrap();
        fs::write(root.join("gitconfig"), "").unwrap();
        let f = Self {
            project: root.join("project"),
            state: root.join("state"),
            key: root.join("keys").join("signing-key"),
            root,
            next_tool: std::cell::Cell::new(0),
        };
        fs::write(f.project.join("src").join("lib.rs"), "// one\n").unwrap();
        fs::write(f.project.join("README.md"), readme).unwrap();
        fs::write(
            f.project.join(".github").join("docs-paths.txt"),
            "README.md\ndocs/**\n",
        )
        .unwrap();
        f.git(&["init", "-q"]);
        f.commit("one");
        f
    }

    /// A child that sees no user or system git configuration and no repository above the root.
    pub fn command(&self, program: &str) -> Command {
        let mut cmd = Command::new(program);
        cmd.env("HOME", &self.root)
            .env("SUPERVISOR_CONFIG_DIR", self.root.join("config"))
            .env("SUPERVISOR_STATE_DIR", &self.state)
            .env("SUPERVISOR_SIGNING_KEY_FILE", &self.key)
            .env("USERNAME", USER_MARKER)
            .env("USER", USER_MARKER)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", self.root.join("gitconfig"))
            .env("GIT_CEILING_DIRECTORIES", &self.root)
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .env_remove("GIT_INDEX_FILE")
            .env_remove("GIT_OBJECT_DIRECTORY");
        cmd
    }

    pub fn git_in(&self, dir: &Path, args: &[&str]) -> String {
        let out = self
            .command("git")
            .arg("-C")
            .arg(dir)
            .args([
                "-c",
                "user.name=fixture",
                "-c",
                "user.email=fixture@example.invalid",
                "-c",
                "commit.gpgsign=false",
                "-c",
                "core.autocrlf=false",
            ])
            .args(args)
            .output()
            .unwrap();
        assert!(out.status.success(), "git {args:?}: {out:?}");
        String::from_utf8_lossy(&out.stdout).trim().to_owned()
    }
    pub fn git(&self, args: &[&str]) -> String {
        self.git_in(&self.project.clone(), args)
    }
    pub fn commit(&self, message: &str) {
        self.git(&["add", "-A"]);
        self.git(&["commit", "-q", "--allow-empty", "-m", message]);
    }
    pub fn head(&self) -> String {
        self.git(&["rev-parse", "HEAD"])
    }

    pub fn run(&self, args: &[&str]) -> Output {
        self.command(BIN)
            .args(args)
            .arg("--project")
            .arg(&self.project)
            .output()
            .unwrap()
    }

    /// A person's `task start` with `contract` (fields over a one-criterion default).
    pub fn start(&self, extra: Value) -> String {
        let mut contract = json!({"id":"wp7","goal":"exercise receipt bundles",
            "criteria":[{"id":"C1","check":"candidate_passes_reported_test","mandatory":true,
                "command":"cargo test -p fixture"}],
            "scope":["src/"],"forbidden_effects":[],"max_repair_rounds":2});
        for (key, value) in extra.as_object().into_iter().flatten() {
            contract[key] = value.clone();
        }
        let file = self.root.join("contract.json");
        fs::write(&file, contract.to_string()).unwrap();
        let out = self.run(&["task", "start", "--contract", file.to_str().unwrap()]);
        assert!(out.status.success(), "task start: {out:?}");
        let v: Value = serde_json::from_slice(&out.stdout).unwrap();
        v["task_id"].as_str().unwrap().to_owned()
    }

    pub fn hook(&self, mut payload: Value) -> Value {
        payload["cwd"] = self.project.to_str().unwrap().into();
        if payload.get("session_id").is_none() {
            payload["session_id"] = "wp7-session".into();
        }
        let mut child = self
            .command(BIN)
            .arg("hook")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(payload.to_string().as_bytes())
            .unwrap();
        let out = child.wait_with_output().unwrap();
        assert_eq!(out.status.code(), Some(0), "{out:?}");
        let text = String::from_utf8(out.stdout).unwrap();
        if text.trim().is_empty() {
            Value::Null
        } else {
            serde_json::from_str(&text).unwrap()
        }
    }

    /// One Bash run of `command` by the chair, observed by the hooks, printing `stdout`.
    pub fn run_check(&self, command: &str, exit: i32, stdout: &str) {
        let n = self.next_tool.get() + 1;
        self.next_tool.set(n);
        let mut payload = json!({"hook_event_name":"PreToolUse","tool_use_id":format!("toolu_wp7_{n}"),
            "agent_id":"main","tool_name":"Bash","tool_input":{"command":command}});
        let decision = self.hook(payload.clone());
        assert_ne!(
            decision["hookSpecificOutput"]["permissionDecision"], "deny",
            "{decision}"
        );
        if exit == 0 {
            payload["hook_event_name"] = "PostToolUse".into();
            payload["tool_response"] = json!({"stdout":stdout,"stderr":"","interrupted":false});
        } else {
            payload["hook_event_name"] = "PostToolUseFailure".into();
            payload["error"] = format!("Exit code {exit}\n{stdout}").into();
        }
        self.hook(payload);
    }

    pub fn stop(&self, message: &str) -> Value {
        self.hook(json!({"hook_event_name":"Stop","stop_hook_active":false,
            "last_assistant_message":message}))
    }

    /// A reviewer agent that changed nothing hands back `message`.
    pub fn review(&self, agent: &str, message: &str) -> Value {
        self.hook(
            json!({"hook_event_name":"SubagentStop","stop_hook_active":false,
            "agent_id":agent,"agent_type":"code-reviewer","last_assistant_message":message}),
        )
    }

    /// Run the mandatory check passing and stop: the task completes on an observed receipt.
    pub fn complete(&self, stdout: &str) {
        self.run_check("cargo test -p fixture", 0, stdout);
        let stop = self.stop("Done; cargo test -p fixture passed.");
        assert_ne!(stop["decision"], "block", "stop refused: {stop}");
    }

    pub fn export(&self) -> (i32, Value) {
        parse(&self.run(&["receipt", "export"]))
    }

    /// Export, assert it wrote the bundle, and return its path, text, and the keys file listing
    /// the key it was signed with (outside the project).
    pub fn exported(&self) -> (PathBuf, String, PathBuf) {
        let (code, value) = self.export();
        assert_eq!(code, 0, "export: {value}");
        assert_eq!(value["export"], "written", "{value}");
        let path = self.project.join(value["bundle"].as_str().unwrap());
        let text = fs::read_to_string(&path).unwrap();
        let keys = self.root.join("supervisor-keys.json");
        fs::write(&keys, json!({"keys":[value["keys_entry"]]}).to_string()).unwrap();
        (path, text, keys)
    }

    /// The usual flow: start, edit `src/lib.rs`, pass the check, export, commit everything.
    pub fn ready(&self) -> (PathBuf, PathBuf) {
        self.start(json!({}));
        fs::write(self.project.join("src").join("lib.rs"), "// two\n").unwrap();
        self.complete("ok");
        let (bundle, _, keys) = self.exported();
        self.commit("work and its receipt");
        (bundle, keys)
    }

    /// Re-sign an edited bundle with this fixture's key, recomputing the body digest and the
    /// statement as the documented format says (written here independently of the binary).
    pub fn resign(&self, bundle: &mut Value) {
        let mut body = bundle.clone();
        body.as_object_mut().unwrap().remove("signature");
        let body_sha = supervisor_claude_code::sha256::hex(
            supervisor_claude_code::chain::canonical(&body).as_bytes(),
        );
        let roots: Vec<&str> = bundle["repository"]["root_commits"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        let s = |k: &str| bundle[k].as_str().unwrap().to_owned();
        let statement = format!(
            "coding-supervisor receipt bundle v2\nrepository:{}\ntask:{}\nexported_contract_sha256:{}\ngate_state:{}\nchain_head:{}\ncandidate:{}:{}\ntree:{}\nbody_sha256:{}\n",
            roots.join(","),
            s("task_id"),
            s("exported_contract_sha256"),
            s("gate_state"),
            s("chain_head"),
            bundle["candidate"]["base_commit"].as_str().unwrap(),
            bundle["candidate"]["dirty_digest"].as_str().unwrap(),
            s("tree"),
            body_sha
        );
        let secret = fs::read_to_string(&self.key).unwrap();
        let secret: Vec<u8> = (0..32)
            .map(|i| u8::from_str_radix(&secret.trim()[2 * i..2 * i + 2], 16).unwrap())
            .collect();
        let key = ed25519_dalek::SigningKey::from_bytes(&secret.try_into().unwrap());
        let signature = ed25519_dalek::Signer::sign(&key, statement.as_bytes());
        let hex: String = signature
            .to_bytes()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        bundle["signature"]["body_sha256"] = body_sha.into();
        bundle["signature"]["signature"] = hex.into();
    }

    /// Replace the bundled contract text through `edit` and update its hash.
    pub fn edit_contract(&self, path: &Path, edit: impl FnOnce(&str) -> String) {
        self.edit_bundle(path, |b| {
            let text = edit(b["contract"].as_str().unwrap());
            b["exported_contract_sha256"] =
                supervisor_claude_code::sha256::hex(text.as_bytes()).into();
            b["contract"] = text.into();
        });
    }

    /// One MCP tool call through the real binary; the tool's answer.
    pub fn mcp(&self, tool: &str, arguments: Value) -> Value {
        let mut child = self
            .command(BIN)
            .arg("mcp")
            .arg("--project")
            .arg(&self.project)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        writeln!(
            child.stdin.take().unwrap(),
            "{}",
            json!({"jsonrpc":"2.0","id":1,"method":"tools/call",
                "params":{"name":tool,"arguments":arguments}})
        )
        .unwrap();
        let out = child.wait_with_output().unwrap();
        let reply: Value = serde_json::from_slice(&out.stdout).unwrap();
        assert_eq!(reply["result"]["isError"], false, "{reply}");
        serde_json::from_str(reply["result"]["content"][0]["text"].as_str().unwrap()).unwrap()
    }

    /// Read, edit, re-sign and write back a bundle.
    pub fn edit_bundle(&self, path: &Path, edit: impl FnOnce(&mut Value)) {
        let mut bundle: Value = serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap();
        edit(&mut bundle);
        self.resign(&mut bundle);
        fs::write(path, serde_json::to_string_pretty(&bundle).unwrap()).unwrap();
    }

    /// `receipt verify --bundle` in `dir` (default the project), never with a state folder.
    pub fn verify_in(
        &self,
        dir: &Path,
        bundle: &Path,
        keys: &Path,
        extra: &[&str],
    ) -> (i32, Value) {
        let out = self
            .command(BIN)
            .env("SUPERVISOR_STATE_DIR", self.root.join("no-state-here"))
            .args(["receipt", "verify", "--bundle"])
            .arg(bundle)
            .arg("--keys")
            .arg(keys)
            .arg("--project")
            .arg(dir)
            .args(extra)
            .output()
            .unwrap();
        assert!(
            !self.root.join("no-state-here").exists(),
            "bundle verify created a state folder"
        );
        parse(&out)
    }
    pub fn verify(&self, bundle: &Path, keys: &Path, extra: &[&str]) -> (i32, Value) {
        self.verify_in(&self.project.clone(), bundle, keys, extra)
    }
}

pub fn parse(out: &Output) -> (i32, Value) {
    let stderr = String::from_utf8_lossy(&out.stderr);
    let stdout = String::from_utf8_lossy(&out.stdout);
    for word in BANNED {
        assert!(
            !stdout.to_lowercase().contains(word) && !stderr.to_lowercase().contains(word),
            "banned word {word:?}: {stdout} / {stderr}"
        );
    }
    let value = serde_json::from_slice(&out.stdout)
        .unwrap_or_else(|e| panic!("JSON ({e}): {stdout} / {stderr}"));
    (out.status.code().unwrap(), value)
}
