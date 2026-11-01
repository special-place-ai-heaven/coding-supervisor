//! The hook answers within a bound when git hangs or another process holds the state lock,
//! and git never runs while the hook holds state.lock.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use supervisor_claude_code::location;

const BIN: &str = env!("CARGO_BIN_EXE_supervisor");
/// Past this a hook counts as hung; it is killed so the test itself never hangs.
const HUNG: Duration = Duration::from_secs(60);

/// A `git` that records whether `STUB_GIT_PROBE` is exclusively locked, sleeps
/// `STUB_GIT_SLEEP_MS` and then fails as outside a repository. Compiled, so Windows finds it.
const STUB: &str = r#"
use std::io::Write;
fn main() {
    if let (Ok(probe), Ok(log)) = (std::env::var("STUB_GIT_PROBE"), std::env::var("STUB_GIT_LOG")) {
        let held = std::fs::File::open(&probe)
            .is_ok_and(|f| matches!(f.try_lock_shared(), Err(std::fs::TryLockError::WouldBlock)));
        let mut log = std::fs::OpenOptions::new().create(true).append(true).open(log).unwrap();
        log.write_all(if held { b"held\n" } else { b"free\n" }).unwrap();
    }
    let ms = std::env::var("STUB_GIT_SLEEP_MS").ok().and_then(|v| v.parse().ok()).unwrap_or(0);
    std::thread::sleep(std::time::Duration::from_millis(ms));
    eprintln!("fatal: not a git repository (stub)");
    std::process::exit(128);
}
"#;

struct Fixture {
    root: PathBuf,
    project: PathBuf,
    state: PathBuf,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
impl Fixture {
    fn new(name: &str) -> Self {
        let root = std::env::temp_dir().join(format!("sup-cli-hang-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("project")).unwrap();
        Self {
            project: root.join("project"),
            state: root.join("state"),
            root,
        }
    }
    fn state_root(&self) -> PathBuf {
        location::resolve_state_dir(&self.project, &|key| {
            (key == "SUPERVISOR_STATE_DIR").then(|| self.state.to_string_lossy().into_owned())
        })
        .0
    }
    /// A PATH whose first `git` is the stub.
    fn stub_path(&self) -> std::ffi::OsString {
        let dir = self.root.join("stub");
        fs::create_dir_all(&dir).unwrap();
        let source = dir.join("git.rs");
        fs::write(&source, STUB).unwrap();
        let rustc = Path::new(env!("CARGO"))
            .with_file_name(format!("rustc{}", std::env::consts::EXE_SUFFIX));
        let built = Command::new(rustc)
            .current_dir(env!("CARGO_MANIFEST_DIR"))
            .args(["--edition", "2024", "-o"])
            .arg(dir.join(format!("git{}", std::env::consts::EXE_SUFFIX)))
            .arg(&source)
            .output()
            .unwrap();
        assert!(
            built.status.success(),
            "{}",
            String::from_utf8_lossy(&built.stderr)
        );
        let path = std::env::var_os("PATH").unwrap_or_default();
        std::env::join_paths(std::iter::once(dir).chain(std::env::split_paths(&path))).unwrap()
    }
    fn task_start(&self) {
        let contract = self.root.join("contract.json");
        fs::write(&contract,json!({"id":"hang","goal":"fixture","criteria":[{"id":"C1","check":"candidate_passes_reported_test","mandatory":true,"command":"cargo test"}],"scope":["src/"],"forbidden_effects":[],"max_repair_rounds":2}).to_string()).unwrap();
        let out = Command::new(BIN)
            .env("SUPERVISOR_STATE_DIR", &self.state)
            .args(["task", "start", "--project"])
            .arg(&self.project)
            .arg("--contract")
            .arg(&contract)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    /// One hook call: its decision and how long it took; panics instead of waiting past [`HUNG`].
    fn hook(&self, payload: Value, env: &[(&str, &std::ffi::OsStr)]) -> (Value, Duration) {
        let mut command = Command::new(BIN);
        command
            .arg("hook")
            .env("SUPERVISOR_STATE_DIR", &self.state)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        for (key, value) in env {
            command.env(key, value);
        }
        let started = Instant::now();
        let mut child = command.spawn().unwrap();
        writeln!(child.stdin.take().unwrap(), "{payload}").unwrap();
        while child.try_wait().unwrap().is_none() {
            if started.elapsed() > HUNG {
                let _ = child.kill();
                let _ = child.wait();
                panic!("hook still running after {HUNG:?}: {payload}");
            }
            thread::sleep(Duration::from_millis(20));
        }
        let elapsed = started.elapsed();
        let output = child.wait_with_output().unwrap();
        assert!(output.status.success());
        let text = String::from_utf8_lossy(&output.stdout);
        let decision = if text.trim().is_empty() {
            Value::Null
        } else {
            serde_json::from_str(&text).unwrap()
        };
        (decision, elapsed)
    }
    fn write(&self) -> Value {
        json!({"cwd":self.project,"session_id":"session-1","hook_event_name":"PreToolUse","tool_name":"Write","tool_input":{"file_path":"src/lib.rs","content":"x"}})
    }
    fn stop(&self) -> Value {
        json!({"cwd":self.project,"session_id":"session-1","hook_event_name":"Stop","stop_hook_active":false})
    }
}

fn denied(decision: &Value) -> &str {
    assert_eq!(
        decision["hookSpecificOutput"]["permissionDecision"], "deny",
        "{decision}"
    );
    decision["hookSpecificOutput"]["permissionDecisionReason"]
        .as_str()
        .unwrap()
}

fn blocked(decision: &Value) -> &str {
    assert_eq!(decision["decision"], "block", "{decision}");
    decision["reason"].as_str().unwrap()
}

#[test]
fn hanging_git_denies_and_blocks_within_the_bound() {
    let f = Fixture::new("git");
    f.task_start();
    let path = f.stub_path();
    let env = [
        ("PATH", path.as_os_str()),
        ("STUB_GIT_SLEEP_MS", "600000".as_ref()),
    ];
    let (write, stop) = thread::scope(|s| {
        let write = s.spawn(|| f.hook(f.write(), &env));
        let stop = s.spawn(|| f.hook(f.stop(), &env));
        (write.join().unwrap(), stop.join().unwrap())
    });
    for ((decision, elapsed), reason) in [(&write.0, write.1), (&stop.0, stop.1)]
        .into_iter()
        .zip([denied(&write.0), blocked(&stop.0)])
    {
        assert!(reason.contains("no answer within 5 s"), "{decision}");
        // Four git calls precede the decision, each cut off at 5 s.
        assert!(elapsed < Duration::from_secs(30), "{elapsed:?}");
    }
}

#[test]
fn held_state_lock_denies_and_blocks_within_the_bound() {
    let f = Fixture::new("lock");
    f.task_start();
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .open(f.state_root().join("state.lock"))
        .unwrap();
    lock.lock().unwrap();
    let (write, stop) = thread::scope(|s| {
        let write = s.spawn(|| f.hook(f.write(), &[]));
        let stop = s.spawn(|| f.hook(f.stop(), &[]));
        (write.join().unwrap(), stop.join().unwrap())
    });
    drop(lock);
    for ((decision, elapsed), reason) in [(&write.0, write.1), (&stop.0, stop.1)]
        .into_iter()
        .zip([denied(&write.0), blocked(&stop.0)])
    {
        assert!(reason.contains("state is busy"), "{decision}");
        assert!(elapsed < Duration::from_secs(15), "{elapsed:?}");
    }
}

#[test]
fn hook_never_runs_git_under_the_state_lock_and_parallel_hooks_do_not_queue_on_it() {
    let f = Fixture::new("parallel");
    f.task_start();
    let path = f.stub_path();
    let probe = f.state_root().join("state.lock");
    let log = f.root.join("git.log");
    let read = |n: usize| json!({"cwd":f.project,"session_id":format!("session-{n}"),"hook_event_name":"PreToolUse","tool_name":"Read","tool_input":{"file_path":"src/lib.rs"}});
    // Alone, so a held lock can only be this hook's own.
    let (decision, _) = f.hook(
        read(0),
        &[
            ("PATH", path.as_os_str()),
            ("STUB_GIT_PROBE", probe.as_os_str()),
            ("STUB_GIT_LOG", log.as_os_str()),
        ],
    );
    assert_eq!(decision, Value::Null);
    let calls = fs::read_to_string(&log).unwrap();
    assert!(calls.lines().count() >= 4, "{calls}");
    assert!(calls.lines().all(|line| line == "free"), "{calls}");
    // Each hook spends seconds in 1 s git calls; under the lock eight would take turns.
    let env = [
        ("PATH", path.as_os_str()),
        ("STUB_GIT_SLEEP_MS", "1000".as_ref()),
    ];
    let started = Instant::now();
    thread::scope(|s| {
        for n in 1..=8 {
            let (f, env, read) = (&f, &env, &read);
            s.spawn(move || assert_eq!(f.hook(read(n), env).0, Value::Null));
        }
    });
    let elapsed = started.elapsed();
    assert!(elapsed < Duration::from_secs(15), "{elapsed:?}");
}
