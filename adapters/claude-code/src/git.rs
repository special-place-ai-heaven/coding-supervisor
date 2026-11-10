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
    // The gate's own fallback state directory inside the project is not content, and neither
    // is what the project's CI treats as documentation only (G28).
    let docs = crate::location::worktree_root(cwd)
        .map(|top| ci_docs_only(&top))
        .unwrap_or_default();
    tree_without(cwd, state_dir, &docs)
}

/// WP5: the tree of the whole worktree content, documentation included, for reviews: a
/// reviewer may have judged the README. Leaves out only the gate's own fallback state
/// directory, and never reads the documentation list.
pub fn content_tree(cwd: &Path, state_dir: &Path) -> Option<String> {
    tree_without(cwd, state_dir, &[])
}

/// The worktree tree with the `docs` globs left out (none for [`content_tree`]).
fn tree_without(cwd: &Path, state_dir: &Path, docs: &[String]) -> Option<String> {
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
        // Racy git: an entry no older than the index file is re-read, because a same-size edit
        // in that second leaves its stat unchanged. A copy dated now (Linux) would make git
        // trust that entry and miss the edit, so the copy keeps the real index's time.
        let time = std::fs::metadata(&real).and_then(|m| m.modified()).ok()?;
        std::fs::File::options()
            .write(true)
            .open(&scratch)
            .and_then(|f| f.set_modified(time))
            .ok()?;
    }
    let env = [("GIT_INDEX_FILE", scratch.as_os_str())];
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

/// G28: what the project's CI treats as documentation only, as git glob patterns. The lines of
/// `.github/docs-paths.txt` when that file exists (one list that CI reads too; blank lines and
/// `#` comments skipped), otherwise the `paths-ignore` entries of `.github/workflows/ci.yml`.
/// Empty when neither names a path.
fn ci_docs_only(top: &Path) -> Vec<String> {
    if let Ok(text) = std::fs::read_to_string(top.join(".github/docs-paths.txt")) {
        // A negated or absolute entry is not a plain path; leave it counted.
        return text
            .lines()
            .map(str::trim)
            .filter(|p| !p.is_empty() && !p.starts_with(['#', '!', '/']))
            .map(str::to_owned)
            .collect();
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::process::Command;

    struct Dir(PathBuf);
    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn git(dir: &Path, args: &[&str], env: &[(&str, &Path)]) -> String {
        let out = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(["-c", "user.name=t", "-c", "user.email=t@example.invalid"])
            .args(args)
            .envs(env.iter().copied())
            .output()
            .unwrap();
        assert!(out.status.success(), "git {args:?}: {out:?}");
        String::from_utf8_lossy(&out.stdout).trim().to_owned()
    }

    fn set_time(path: &Path, time: std::time::SystemTime) {
        std::fs::File::options()
            .write(true)
            .open(path)
            .and_then(|f| f.set_modified(time))
            .unwrap();
    }

    /// Racy git: an edit made in the same second as the index, with the same size, still matches
    /// the index entry's stat. Git notices it only because the entry is not older than the index
    /// file itself, so the scratch copy must keep the index's time. The race is built directly:
    /// the edited file and the index both get the time the entry recorded (ctime cannot be set,
    /// so `core.trustctime` is off, as git's own same-second case behaves).
    #[test]
    fn the_tree_includes_an_edit_whose_stat_still_matches_the_index() {
        let dir =
            Dir(std::env::temp_dir().join(format!("supervisor-racy-index-{}", std::process::id())));
        let _ = std::fs::remove_dir_all(&dir.0);
        let (repo, state) = (dir.0.join("repo"), dir.0.join("state"));
        std::fs::create_dir_all(&repo).unwrap();
        std::fs::create_dir_all(&state).unwrap();
        git(&repo, &["init", "-q"], &[]);
        git(&repo, &["config", "core.trustctime", "false"], &[]);
        let file = repo.join("a.rs");
        std::fs::write(&file, "a1\n").unwrap();
        // An hour back, so the scratch copy made now is in a later second than the entry, as a
        // hook call after the edit would be (within the same second git re-reads anyway).
        let recorded = std::time::SystemTime::now() - std::time::Duration::from_secs(3600);
        set_time(&file, recorded);
        git(&repo, &["add", "a.rs"], &[]);
        git(&repo, &["commit", "-q", "-m", "base"], &[]);
        std::fs::write(&file, "a2\n").unwrap();
        set_time(&file, recorded);
        set_time(&repo.join(".git/index"), recorded);

        // What a commit of everything would record: a fresh index has no stat cache to trust.
        let fresh = dir.0.join("fresh-index");
        git(&repo, &["add", "-A"], &[("GIT_INDEX_FILE", &fresh)]);
        let want = git(&repo, &["write-tree"], &[("GIT_INDEX_FILE", &fresh)]);
        let head = git(&repo, &["rev-parse", "HEAD^{tree}"], &[]);
        assert_ne!(want, head, "the edit must change the tree");

        assert_eq!(worktree_tree(&repo, &state).as_deref(), Some(want.as_str()));
    }
}
