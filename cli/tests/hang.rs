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

/// G26: these tests measure wall-clock bounds with sleeping stub gits and 30 s lock holders;
/// run side by side they starve each other past their bounds (each passes alone), so they run
/// one at a time (`.config/nextest.toml` does the same under nextest). A panic poisons the
/// mutex; the next test still runs.
fn serial() -> std::sync::MutexGuard<'static, ()> {
    static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());
    SERIAL.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}
/// A hook whose every git is cut off at the 5 s deadline answers by then (about 20 s measured).
const GIT_BOUND: Duration = Duration::from_secs(30);
/// A hook that waits out the 10 s lock deadline once answers by then.
const LOCK_BOUND: Duration = Duration::from_secs(15);
/// A busy connection lock after a git timeout: 5 s plus the 10 s lock deadline.
const BUSY_BOUND: Duration = Duration::from_secs(25);

/// A `git` that records whether `STUB_GIT_PROBE` is exclusively locked, sleeps
/// `STUB_GIT_SLEEP_MS` and then fails as outside a repository. On `rev-parse HEAD ...` (the
/// candidate's HEAD lookup), with
/// `STUB_GIT_HOLD_AT_HEAD` set, it first starts a copy of itself that holds that lock for 30 s
/// and touches `STUB_GIT_HELD` once it has it, then sleeps `STUB_GIT_SLEEP_AT_HEAD_MS`.
/// With `STUB_GIT_PIDS` it appends its pid there. With `STUB_GIT_ORPHAN` it leaves behind a
/// detached copy of itself that keeps its stdout and stderr open until the file named by
/// `STUB_GIT_ORPHAN` exists, then exits at once. Every sleep ends early once the stub's own
/// executable is gone, so a fixture's removal also ends any stub left running.
/// Compiled, so Windows finds it.
const STUB: &str = r#"
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};
/// Sleeps `ms`, or less once the executable is gone or `until` exists.
fn nap(ms: u64, until: Option<&Path>) {
    let me = std::env::current_exe().unwrap();
    let started = Instant::now();
    while started.elapsed() < Duration::from_millis(ms)
        && me.exists()
        && !until.is_some_and(Path::exists)
    {
        std::thread::sleep(Duration::from_millis(20));
    }
}
fn main() {
    if let Ok(lock) = std::env::var("STUB_GIT_HOLDER") {
        let file = std::fs::OpenOptions::new().read(true).write(true).open(lock).unwrap();
        file.lock().unwrap();
        std::fs::write(std::env::var("STUB_GIT_HELD").unwrap(), "").unwrap();
        nap(30_000, None);
        return;
    }
    if let Ok(release) = std::env::var("STUB_GIT_ORPHAN_CHILD") {
        nap(120_000, Some(Path::new(&release)));
        return;
    }
    if let Ok(pids) = std::env::var("STUB_GIT_PIDS") {
        let mut pids = std::fs::OpenOptions::new().create(true).append(true).open(pids).unwrap();
        writeln!(pids, "{}", std::process::id()).unwrap();
    }
    if let Ok(release) = std::env::var("STUB_GIT_ORPHAN") {
        let mut orphan = Command::new(std::env::current_exe().unwrap());
        orphan.env("STUB_GIT_ORPHAN_CHILD", release).stdin(Stdio::null());
        #[cfg(unix)]
        std::os::unix::process::CommandExt::process_group(&mut orphan, 0);
        orphan.spawn().unwrap();
        std::process::exit(128);
    }
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Ok(lock) = std::env::var("STUB_GIT_HOLD_AT_HEAD")
        && args.windows(2).any(|w| w[0] == "rev-parse" && w[1] == "HEAD")
    {
        Command::new(std::env::current_exe().unwrap())
            .env("STUB_GIT_HOLDER", lock)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let held = std::env::var("STUB_GIT_HELD").unwrap();
        while !Path::new(&held).exists() {
            std::thread::sleep(Duration::from_millis(10));
        }
        let ms = std::env::var("STUB_GIT_SLEEP_AT_HEAD_MS").ok().and_then(|v| v.parse().ok());
        nap(ms.unwrap_or(0), None);
    }
    if let (Ok(probe), Ok(log)) = (std::env::var("STUB_GIT_PROBE"), std::env::var("STUB_GIT_LOG")) {
        let held = std::fs::File::open(&probe)
            .is_ok_and(|f| matches!(f.try_lock_shared(), Err(std::fs::TryLockError::WouldBlock)));
        let mut log = std::fs::OpenOptions::new().create(true).append(true).open(log).unwrap();
        log.write_all(if held { b"held\n" } else { b"free\n" }).unwrap();
    }
    let ms = std::env::var("STUB_GIT_SLEEP_MS").ok().and_then(|v| v.parse().ok()).unwrap_or(0);
    nap(ms, None);
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
    /// One hook call's decision. Fails the test, killing the hook, once it has run for
    /// `bound` without answering.
    fn hook(&self, payload: Value, env: &[(&str, &std::ffi::OsStr)], bound: Duration) -> Value {
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
            if started.elapsed() > bound {
                let _ = child.kill();
                let _ = child.wait();
                panic!("hook gave no answer within its {bound:?} bound: {payload}");
            }
            thread::sleep(Duration::from_millis(20));
        }
        let output = child.wait_with_output().unwrap();
        assert!(output.status.success());
        let text = String::from_utf8_lossy(&output.stdout);
        if text.trim().is_empty() {
            Value::Null
        } else {
            serde_json::from_str(&text).unwrap()
        }
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
    let _serial = serial();
    let f = Fixture::new("git");
    f.task_start();
    let path = f.stub_path();
    let env = [
        ("PATH", path.as_os_str()),
        ("STUB_GIT_SLEEP_MS", "600000".as_ref()),
    ];
    let (write, stop) = thread::scope(|s| {
        let write = s.spawn(|| f.hook(f.write(), &env, GIT_BOUND));
        let stop = s.spawn(|| f.hook(f.stop(), &env, GIT_BOUND));
        (write.join().unwrap(), stop.join().unwrap())
    });
    for (decision, reason) in [(&write, denied(&write)), (&stop, blocked(&stop))] {
        assert!(reason.contains("no answer within 5 s"), "{decision}");
    }
}

#[test]
fn held_state_lock_denies_and_blocks_within_the_bound() {
    let _serial = serial();
    let f = Fixture::new("lock");
    f.task_start();
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .open(f.state_root().join("state.lock"))
        .unwrap();
    lock.lock().unwrap();
    let (write, stop) = thread::scope(|s| {
        let write = s.spawn(|| f.hook(f.write(), &[], LOCK_BOUND));
        let stop = s.spawn(|| f.hook(f.stop(), &[], LOCK_BOUND));
        (write.join().unwrap(), stop.join().unwrap())
    });
    drop(lock);
    for (decision, reason) in [(&write, denied(&write)), (&stop, blocked(&stop))] {
        assert!(reason.contains("state is busy"), "{decision}");
    }
}

#[test]
fn hook_never_runs_git_under_the_state_lock_and_parallel_hooks_do_not_queue_on_it() {
    let _serial = serial();
    let f = Fixture::new("parallel");
    f.task_start();
    let path = f.stub_path();
    let probe = f.state_root().join("state.lock");
    let log = f.root.join("git.log");
    let read = |n: usize| json!({"cwd":f.project,"session_id":format!("session-{n}"),"hook_event_name":"PreToolUse","tool_name":"Read","tool_input":{"file_path":"src/lib.rs"}});
    // Alone, so a held lock can only be this hook's own.
    let decision = f.hook(
        read(0),
        &[
            ("PATH", path.as_os_str()),
            ("STUB_GIT_PROBE", probe.as_os_str()),
            ("STUB_GIT_LOG", log.as_os_str()),
        ],
        LOCK_BOUND,
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
            s.spawn(move || assert_eq!(f.hook(read(n), env, LOCK_BOUND), Value::Null));
        }
    });
    let elapsed = started.elapsed();
    assert!(elapsed < Duration::from_secs(15), "{elapsed:?}");
}

#[test]
fn lock_taken_after_state_preparation_denies_and_blocks_and_records_nothing() {
    let _serial = serial();
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
                let decision = f.hook(
                    payload,
                    &[
                        ("PATH", path.as_os_str()),
                        ("STUB_GIT_HOLD_AT_HEAD", lock.as_os_str()),
                        ("STUB_GIT_HELD", held.as_os_str()),
                    ],
                    BUSY_BOUND,
                );
                assert!(held.exists(), "{decision}");
                assert!(refused(&decision).contains("state is busy"), "{decision}");
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
fn busy_at_connection(name: &str, payload: fn(&Fixture) -> Value, head_sleep_ms: &str) -> Value {
    let f = Fixture::new(name);
    f.task_start();
    let path = f.stub_path();
    let root = f.state_root();
    let lock = root.join("state.lock");
    let held = f.root.join("held");
    let before = (snapshot(&root), snapshot(&f.project));
    assert!(!before.0.is_empty());
    let decision = f.hook(
        payload(&f),
        &[
            ("PATH", path.as_os_str()),
            ("STUB_GIT_HOLD_AT_HEAD", lock.as_os_str()),
            ("STUB_GIT_HELD", held.as_os_str()),
            ("STUB_GIT_SLEEP_AT_HEAD_MS", head_sleep_ms.as_ref()),
        ],
        BUSY_BOUND,
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
    decision
}

fn subagent_stop(f: &Fixture) -> Value {
    json!({"cwd":f.project,"session_id":"session-1","hook_event_name":"SubagentStop","agent_id":"agent-1","stop_hook_active":false})
}

#[test]
fn busy_lock_stop_and_subagentstop_block_with_no_record() {
    let _serial = serial();
    thread::scope(|s| {
        for (name, payload) in [
            ("busy-stop", Fixture::stop as fn(&Fixture) -> Value),
            ("busy-subagent-stop", subagent_stop),
        ] {
            s.spawn(move || {
                let decision = busy_at_connection(name, payload, "0");
                assert!(blocked(&decision).contains("state is busy"), "{decision}");
            });
        }
    });
}

#[test]
fn busy_lock_after_git_timeout_at_stop_blocks_with_no_record() {
    let _serial = serial();
    // The candidate's `rev-parse HEAD` is cut off at 5 s; then the connection lock is busy.
    let decision = busy_at_connection("busy-git-stop", Fixture::stop, "600000");
    assert!(blocked(&decision).contains("state is busy"), "{decision}");
}

#[test]
fn busy_lock_other_events_fail_open_unchanged() {
    let _serial = serial();
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
                let decision = busy_at_connection(name, payload, "0");
                assert_eq!(decision.as_object().map(|o| o.len()), Some(1), "{decision}");
                let message = decision["systemMessage"].as_str().unwrap_or_default();
                assert!(message.starts_with("GATE adapter error: "), "{decision}");
                assert!(message.contains("state is busy"), "{decision}");
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
    let _serial = serial();
    let f = Fixture::new("busy-cli");
    f.task_start();
    let root = f.state_root();
    // G26: the code waits up to its 10 s lock deadline; allow twice that, since a loaded
    // machine (nextest runs test files in parallel) adds seconds that are not a hang.
    let bound = Duration::from_secs(2 * 10);
    let run = |command: &str| {
        let mut c = Command::new(BIN);
        c.env("SUPERVISOR_STATE_DIR", &f.state)
            .args([command, "--project"])
            .arg(&f.project);
        bounded(c, bound + Duration::from_secs(5))
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
        assert!(elapsed < bound, "{command}: {elapsed:?}");
    }
    drop(lock);
    assert_eq!((snapshot(&root), snapshot(&f.project)), before);
}

/// Makes every orphan the stub left behind exit, once the test is done with them.
struct Release(PathBuf);
impl Drop for Release {
    fn drop(&mut self) {
        let _ = fs::write(&self.0, "");
    }
}

#[test]
fn git_whose_orphan_keeps_its_pipes_open_denies_and_blocks_within_the_bound() {
    let _serial = serial();
    let f = Fixture::new("orphan");
    f.task_start();
    let path = f.stub_path();
    // Each stub exits at once, but a detached child of it holds the inherited stdout and
    // stderr open for up to 120 s; collection must stop at the git deadline anyway.
    let release = Release(f.root.join("release"));
    let env = [
        ("PATH", path.as_os_str()),
        ("STUB_GIT_ORPHAN", release.0.as_os_str()),
    ];
    let (write, stop) = thread::scope(|s| {
        let write = s.spawn(|| f.hook(f.write(), &env, GIT_BOUND));
        let stop = s.spawn(|| f.hook(f.stop(), &env, GIT_BOUND));
        (write.join().unwrap(), stop.join().unwrap())
    });
    for (decision, reason) in [(&write, denied(&write)), (&stop, blocked(&stop))] {
        assert!(reason.contains("no answer within 5 s"), "{decision}");
    }
    drop(release);
}

/// While the hook keeps running after a git timeout, the git it killed is already reaped:
/// by the time it starts its next git, the first is no zombie.
#[cfg(target_os = "linux")]
#[test]
fn timed_out_git_is_reaped_before_the_next_git_starts() {
    let _serial = serial();
    let f = Fixture::new("reap");
    f.task_start();
    let path = f.stub_path();
    let pids = f.root.join("pids");
    let env = [
        ("PATH", path.as_os_str()),
        ("STUB_GIT_SLEEP_MS", "600000".as_ref()),
        ("STUB_GIT_PIDS", pids.as_os_str()),
    ];
    thread::scope(|s| {
        let hook = s.spawn(|| f.hook(f.write(), &env, GIT_BOUND));
        let started = Instant::now();
        let first = loop {
            let text = fs::read_to_string(&pids).unwrap_or_default();
            let mut lines = text.lines();
            if let (Some(first), Some(_)) = (lines.next(), lines.next()) {
                break first.to_owned();
            }
            assert!(
                started.elapsed() < Duration::from_secs(15),
                "no second git: {text}"
            );
            thread::sleep(Duration::from_millis(20));
        };
        // `/proc/<pid>/stat` is `pid (comm) state …`; gone means reaped.
        if let Ok(stat) = fs::read_to_string(format!("/proc/{first}/stat")) {
            let state = stat.rsplit_once(") ").map(|(_, rest)| &rest[..1]);
            assert_ne!(state, Some("Z"), "git {first} left unreaped: {stat}");
        }
        denied(&hook.join().unwrap());
    });
}
