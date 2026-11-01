//! The hook answers within a bound when git hangs or another process holds the state lock,
//! and git never runs while the hook holds state.lock.

use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use supervisor_claude_code::{location, sha256};

const BIN: &str = env!("CARGO_BIN_EXE_supervisor");
/// Past this a hook counts as hung; it is killed so the test itself never hangs.
const HUNG: Duration = Duration::from_secs(60);

/// A `git` that records whether `STUB_GIT_PROBE` is exclusively locked, sleeps
/// `STUB_GIT_SLEEP_MS` and then fails as outside a repository. On `rev-parse HEAD`, with
/// `STUB_GIT_HOLD_AT_HEAD` set, it first starts a copy of itself that holds that lock for 30 s
/// and touches `STUB_GIT_HELD` once it has it, then sleeps `STUB_GIT_SLEEP_AT_HEAD_MS`.
/// Compiled, so Windows finds it.
const STUB: &str = r#"
use std::io::Write;
use std::process::{Command, Stdio};
fn main() {
    if let Ok(lock) = std::env::var("STUB_GIT_HOLDER") {
        let file = std::fs::OpenOptions::new().read(true).write(true).open(lock).unwrap();
        file.lock().unwrap();
        std::fs::write(std::env::var("STUB_GIT_HELD").unwrap(), "").unwrap();
        std::thread::sleep(std::time::Duration::from_secs(30));
        return;
    }
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Ok(lock) = std::env::var("STUB_GIT_HOLD_AT_HEAD")
        && args.ends_with(&["rev-parse".to_owned(), "HEAD".to_owned()])
    {
        Command::new(std::env::current_exe().unwrap())
            .env("STUB_GIT_HOLDER", lock)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let held = std::env::var("STUB_GIT_HELD").unwrap();
        while !std::path::Path::new(&held).exists() {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let ms = std::env::var("STUB_GIT_SLEEP_AT_HEAD_MS").ok().and_then(|v| v.parse().ok());
        std::thread::sleep(std::time::Duration::from_millis(ms.unwrap_or(0)));
    }
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

/// Every file under `dir` but `state.lock`: relative path to size and SHA-256.
fn snapshot(dir: &Path) -> BTreeMap<PathBuf, (u64, String)> {
    let mut files = BTreeMap::new();
    let mut pending = vec![dir.to_path_buf()];
    while let Some(next) = pending.pop() {
        for entry in fs::read_dir(&next).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                pending.push(path);
            } else if path != dir.join("state.lock") {
                let bytes = fs::read(&path).unwrap();
                files.insert(
                    path.strip_prefix(dir).unwrap().to_path_buf(),
                    (bytes.len() as u64, sha256::hex(&bytes)),
                );
            }
        }
    }
    files
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

#[test]
fn lock_taken_after_state_preparation_denies_and_blocks_and_records_nothing() {
    let (write, stop) = (
        Fixture::new("connection-write"),
        Fixture::new("connection-stop"),
    );
    thread::scope(|s| {
        for (f, payload, refused) in [
            (&write, write.write(), denied as fn(&Value) -> &str),
            (&stop, stop.stop(), blocked),
        ] {
            s.spawn(move || {
                f.task_start();
                let path = f.stub_path();
                let root = f.state_root();
                let lock = root.join("state.lock");
                let held = f.root.join("held");
                let before = snapshot(&root);
                assert!(!before.is_empty());
                // The hook's last git before observe_connection is the candidate's
                // `rev-parse HEAD`, after prepare_state and the ownership checks released the
                // lock; it returns only once another process holds the lock.
                let (decision, elapsed) = f.hook(
                    payload,
                    &[
                        ("PATH", path.as_os_str()),
                        ("STUB_GIT_HOLD_AT_HEAD", lock.as_os_str()),
                        ("STUB_GIT_HELD", held.as_os_str()),
                    ],
                );
                assert!(held.exists(), "{decision}");
                assert!(refused(&decision).contains("state is busy"), "{decision}");
                assert!(elapsed < Duration::from_secs(25), "{elapsed:?}");
                // Wait out the holder, then nothing may have been written.
                let file = OpenOptions::new()
                    .read(true)
                    .write(true)
                    .open(&lock)
                    .unwrap();
                file.lock().unwrap();
                drop(file);
                assert_eq!(snapshot(&root), before);
            });
        }
    });
}

/// Runs one hook whose lock is taken by another process just before `observe_connection` (and
/// whose candidate `rev-parse HEAD` then sleeps `head_sleep_ms`). Asserts that nothing under the
/// state dir or the project changed: no journal or `events.jsonl` line, no receipt.
fn busy_at_connection(
    name: &str,
    payload: fn(&Fixture) -> Value,
    head_sleep_ms: &str,
) -> (Value, Duration) {
    let f = Fixture::new(name);
    f.task_start();
    let path = f.stub_path();
    let root = f.state_root();
    let lock = root.join("state.lock");
    let held = f.root.join("held");
    let before = (snapshot(&root), snapshot(&f.project));
    assert!(!before.0.is_empty());
    let (decision, elapsed) = f.hook(
        payload(&f),
        &[
            ("PATH", path.as_os_str()),
            ("STUB_GIT_HOLD_AT_HEAD", lock.as_os_str()),
            ("STUB_GIT_HELD", held.as_os_str()),
            ("STUB_GIT_SLEEP_AT_HEAD_MS", head_sleep_ms.as_ref()),
        ],
    );
    assert!(held.exists(), "{decision}");
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(&lock)
        .unwrap();
    file.lock().unwrap();
    drop(file);
    assert_eq!(
        (snapshot(&root), snapshot(&f.project)),
        before,
        "{decision}"
    );
    (decision, elapsed)
}

fn subagent_stop(f: &Fixture) -> Value {
    json!({"cwd":f.project,"session_id":"session-1","hook_event_name":"SubagentStop","agent_id":"agent-1","stop_hook_active":false})
}

#[test]
fn busy_lock_stop_and_subagentstop_block_with_no_record() {
    thread::scope(|s| {
        for (name, payload) in [
            ("busy-stop", Fixture::stop as fn(&Fixture) -> Value),
            ("busy-subagent-stop", subagent_stop),
        ] {
            s.spawn(move || {
                let (decision, elapsed) = busy_at_connection(name, payload, "0");
                assert!(blocked(&decision).contains("state is busy"), "{decision}");
                assert!(elapsed < Duration::from_secs(25), "{elapsed:?}");
            });
        }
    });
}

#[test]
fn busy_lock_after_git_timeout_at_stop_blocks_with_no_record() {
    // The candidate's `rev-parse HEAD` is cut off at 5 s; then the connection lock is busy.
    let (decision, elapsed) = busy_at_connection("busy-git-stop", Fixture::stop, "600000");
    assert!(blocked(&decision).contains("state is busy"), "{decision}");
    assert!(elapsed < Duration::from_secs(25), "{elapsed:?}");
}

#[test]
fn busy_lock_other_events_fail_open_unchanged() {
    fn post_tool_use(f: &Fixture) -> Value {
        json!({"cwd":f.project,"session_id":"session-1","hook_event_name":"PostToolUse","tool_name":"Write","tool_input":{"file_path":"src/lib.rs","content":"x"},"tool_response":{"success":true}})
    }
    fn session_start(f: &Fixture) -> Value {
        json!({"cwd":f.project,"session_id":"session-1","hook_event_name":"SessionStart","source":"startup"})
    }
    thread::scope(|s| {
        for (name, payload) in [
            ("busy-post-tool-use", post_tool_use as fn(&Fixture) -> Value),
            ("busy-session-start", session_start),
        ] {
            s.spawn(move || {
                let (decision, elapsed) = busy_at_connection(name, payload, "0");
                assert_eq!(decision.as_object().map(|o| o.len()), Some(1), "{decision}");
                let message = decision["systemMessage"].as_str().unwrap_or_default();
                assert!(message.starts_with("GATE adapter error: "), "{decision}");
                assert!(message.contains("state is busy"), "{decision}");
                assert!(elapsed < Duration::from_secs(25), "{elapsed:?}");
            });
        }
    });
}

/// Runs `command` to completion; fails the test if it does not answer within `bound`.
fn bounded(mut command: Command, bound: Duration) -> (std::process::Output, Duration) {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let started = Instant::now();
    let mut child = command.spawn().unwrap();
    while child.try_wait().unwrap().is_none() {
        if started.elapsed() > bound {
            let _ = child.kill();
            let _ = child.wait();
            panic!("{command:?} did not answer within {bound:?}");
        }
        thread::sleep(Duration::from_millis(20));
    }
    let elapsed = started.elapsed();
    (child.wait_with_output().unwrap(), elapsed)
}

#[test]
fn busy_lock_verdict_and_status_exit_1_with_no_record() {
    let f = Fixture::new("busy-cli");
    f.task_start();
    let root = f.state_root();
    let run = |command: &str| {
        let mut c = Command::new(BIN);
        c.env("SUPERVISOR_STATE_DIR", &f.state)
            .args([command, "--project"])
            .arg(&f.project);
        bounded(c, Duration::from_secs(15))
    };
    // Unlocked, status succeeds and verdict does not fail with 1.
    assert!(run("status").0.status.success());
    assert_ne!(run("verdict").0.status.code(), Some(1));
    let before = (snapshot(&root), snapshot(&f.project));
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .open(root.join("state.lock"))
        .unwrap();
    lock.lock().unwrap();
    for (command, reason) in [
        ("verdict", "state is busy"),
        ("status", "status could not resolve project state safely"),
    ] {
        let (output, elapsed) = run(command);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert_eq!(output.status.code(), Some(1), "{command}: {stderr}");
        assert!(output.stdout.is_empty(), "{command}");
        assert!(stderr.contains(reason), "{command}: {stderr}");
        assert!(elapsed < Duration::from_secs(13), "{command}: {elapsed:?}");
    }
    drop(lock);
    assert_eq!((snapshot(&root), snapshot(&f.project)), before);
}
