//! Every git the adapter runs: non-interactive, without optional locks or fsmonitor, and
//! answered within a deadline so a hook never waits on git without bound.

use std::io::Read;
use std::path::Path;
use std::process::{Command, Output, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{Duration, Instant};

const DEADLINE: Duration = Duration::from_secs(5);
const HUNG_WINDOW: Duration = Duration::from_secs(60);
static HUNG: std::sync::Mutex<Option<Instant>> = std::sync::Mutex::new(None);

/// Why git gave no exit status to read.
pub enum Failure {
    /// git could not be started or waited for.
    Unavailable(String),
    /// git did not finish within the deadline; the child was killed.
    TimedOut(String),
}

/// `git -C cwd args`, killed and reaped if it is still running after the deadline.
pub fn run(cwd: &Path, args: &[&str]) -> Result<Output, Failure> {
    run_with_env(cwd, args, &[])
}

/// [`run`] with extra environment variables set on the child.
pub fn run_with_env(
    cwd: &Path,
    args: &[&str],
    env: &[(&str, &std::ffi::OsStr)],
) -> Result<Output, Failure> {
    let what = format!("git {}", args.join(" "));
    // Once git has not answered, the next gits of the same process fail at once: a hook makes
    // several, and each waiting its own deadline would exceed the hook's bound.
    // ponytail: process-wide 60 s window; a long-lived MCP server sees git as hung for that long.
    if HUNG
        .lock()
        .ok()
        .and_then(|at| *at)
        .is_some_and(|at| at.elapsed() < HUNG_WINDOW)
    {
        return Err(Failure::TimedOut(format!(
            "{what}: skipped, an earlier git gave no answer within {} s",
            DEADLINE.as_secs()
        )));
    }
    // Test seam: `SUPERVISOR_GIT_TRACE=<file>` appends each git's subcommand (no paths, no
    // output) so a test can pin how many processes a hook call starts.
    if let Ok(trace) = std::env::var("SUPERVISOR_GIT_TRACE") {
        use std::io::Write as _;
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(trace)
        {
            let _ = writeln!(f, "{what}");
        }
    }
    let mut command = Command::new("git");
    command
        .args(["-c", "core.fsmonitor=false", "-C"])
        .arg(cwd)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .envs(env.iter().copied())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let mut child = command
        .spawn()
        .map_err(|e| Failure::Unavailable(format!("{what}: {e}")))?;
    let stdout = drain(child.stdout.take());
    let stderr = drain(child.stderr.take());
    let started = Instant::now();
    let timed_out = || {
        if let Ok(mut at) = HUNG.lock() {
            *at = Some(Instant::now());
        }
        Failure::TimedOut(format!("{what}: no answer within {} s", DEADLINE.as_secs()))
    };
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if started.elapsed() < DEADLINE => thread::sleep(Duration::from_millis(5)),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(timed_out());
            }
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(Failure::Unavailable(format!("{what}: {e}")));
            }
        }
    };
    // A descendant that inherited a pipe can keep it open after git exits.
    let collect = |pipe: Receiver<Vec<u8>>| {
        pipe.recv_timeout(DEADLINE.saturating_sub(started.elapsed()))
            .map_err(|_| timed_out())
    };
    Ok(Output {
        status,
        stdout: collect(stdout)?,
        stderr: collect(stderr)?,
    })
}

/// G9: git's id of the whole worktree content (a tree object), as a commit of everything would
/// record it. Built in a scratch copy of the real index under `state_dir`, so the real index is
/// untouched and git's stat cache is reused. `None` on any failure: callers then keep the exact
/// candidate rule, so a tree that cannot be computed never causes a refusal. Writes unreachable
/// loose objects. The tree is of the worktree whose top level is `cwd`'s.
pub fn worktree_tree(cwd: &Path, state_dir: &Path) -> Option<String> {
    let ok = |args: &[&str], env: &[(&str, &std::ffi::OsStr)]| {
        run_with_env(cwd, args, env)
            .ok()
            .filter(|out| out.status.success())
    };
    let real = ok(&["rev-parse", "--git-path", "index"], &[])?;
    let real = cwd.join(String::from_utf8_lossy(&real.stdout).trim());
    // Per process: parallel hook calls must not share one scratch index.
    let scratch = state_dir.join(format!("scratch-index-{}", std::process::id()));
    let _ = std::fs::remove_file(&scratch);
    // An absent index (nothing staged yet) starts the scratch one empty.
    if real.is_file() {
        std::fs::copy(&real, &scratch).ok()?;
    }
    let env = [("GIT_INDEX_FILE", scratch.as_os_str())];
    // The gate's own fallback state directory inside the project is not content.
    let tree = ok(
        &["add", "-A", "--", ":/", ":(top,exclude).supervisor/"],
        &env,
    )
    .and_then(|_| ok(&["write-tree"], &env))
    .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_owned())
    .filter(|id| !id.is_empty());
    let _ = std::fs::remove_file(&scratch);
    tree
}

fn drain(pipe: Option<impl Read + Send + 'static>) -> Receiver<Vec<u8>> {
    let (sender, receiver) = mpsc::channel();
    if let Some(mut pipe) = pipe {
        thread::spawn(move || {
            let mut bytes = Vec::new();
            let _ = pipe.read_to_end(&mut bytes);
            let _ = sender.send(bytes);
        });
    }
    receiver
}
