//! Every git the adapter runs: non-interactive, without optional locks or fsmonitor, and
//! answered within a deadline so a hook never waits on git without bound.

use std::io::Read;
use std::path::Path;
use std::process::{Command, Output, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{Duration, Instant};

const DEADLINE: Duration = Duration::from_secs(5);

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
    let timed_out =
        || Failure::TimedOut(format!("{what}: no answer within {} s", DEADLINE.as_secs()));
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
    // The gate's own fallback state directory inside the project is not content, and neither
    // is what the project's CI treats as documentation only (G28).
    let docs = crate::location::worktree_root(cwd)
        .map(|top| ci_docs_only(&top))
        .unwrap_or_default();
    let mut add = vec!["add", "-A", "--", ":/", ":(top,exclude).supervisor/"];
    let excludes: Vec<String> = docs
        .iter()
        .map(|p| format!(":(top,exclude,glob){p}"))
        .collect();
    add.extend(excludes.iter().map(String::as_str));
    // Tracked and committed documentation leaves the scratch index too, so a committed docs
    // edit does not change the tree either.
    let includes: Vec<String> = docs.iter().map(|p| format!(":(top,glob){p}")).collect();
    let mut remove = vec!["rm", "--cached", "-r", "-q", "--ignore-unmatch", "--"];
    remove.extend(includes.iter().map(String::as_str));
    let tree = ok(&add, &env)
        .and_then(|out| {
            if includes.is_empty() {
                Some(out)
            } else {
                ok(&remove, &env)
            }
        })
        .and_then(|_| ok(&["write-tree"], &env))
        .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_owned())
        .filter(|id| !id.is_empty());
    let _ = std::fs::remove_file(&scratch);
    tree
}

/// G28: the `paths-ignore` entries of the project's `.github/workflows/ci.yml` (what its CI
/// treats as documentation only), as git glob patterns. Empty when the file or list is absent.
fn ci_docs_only(top: &Path) -> Vec<String> {
    let Ok(text) = std::fs::read_to_string(top.join(".github/workflows/ci.yml")) else {
        return Vec::new();
    };
    let mut found: Vec<String> = Vec::new();
    let mut list_indent = None;
    for line in text.lines() {
        let item = line.trim_start();
        let indent = line.len() - item.len();
        if item.starts_with("paths-ignore:") {
            list_indent = Some(indent);
            continue;
        }
        let Some(key_indent) = list_indent else {
            continue;
        };
        if item.is_empty() || item.starts_with('#') {
            continue;
        }
        match item.strip_prefix("- ") {
            Some(entry) if indent > key_indent => {
                let entry = entry.trim().trim_matches(|c| c == '"' || c == '\'');
                // A negated or absolute entry is not a plain path; leave it counted.
                if !entry.is_empty()
                    && !entry.starts_with('!')
                    && !entry.starts_with('/')
                    && !found.iter().any(|f| f == entry)
                {
                    found.push(entry.to_owned());
                }
            }
            _ => list_indent = None,
        }
    }
    found
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
