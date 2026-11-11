//! Candidate identity: HEAD plus the dirty-tree digest of the entire physical worktree.

use std::fmt;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use coding_supervisor::{Candidate, CandidateId};

use crate::sha256;

pub trait CandidateProvider {
    /// The current candidate, or why it could not be observed.
    fn observe(&self) -> Result<Candidate, String>;
}

pub struct GitCandidate {
    pub cwd: PathBuf,
}

/// Why git could not name the candidate; displays as the underlying message.
pub enum Unobserved {
    /// git could not be started.
    GitUnavailable(String),
    /// git ran and failed: not a repository, unborn HEAD, no answer within the deadline, ...
    GitFailed(String),
    /// A changed file could not be read for the digest.
    WorktreeUnreadable(String),
}

impl fmt::Display for Unobserved {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::GitUnavailable(why) | Self::GitFailed(why) | Self::WorktreeUnreadable(why) => {
                f.write_str(why)
            }
        }
    }
}

impl CandidateProvider for GitCandidate {
    fn observe(&self) -> Result<Candidate, String> {
        self.observe_git().map_err(|e| e.to_string())
    }
}

impl GitCandidate {
    /// [`CandidateProvider::observe`], with the failure kept apart by cause.
    pub fn observe_git(&self) -> Result<Candidate, Unobserved> {
        Self {
            cwd: PathBuf::from(
                crate::location::checked_project(&self.cwd).map_err(Unobserved::GitFailed)?,
            ),
        }
        .observe_worktree()
    }

    fn observe_worktree(&self) -> Result<Candidate, Unobserved> {
        let (base_commit, files) = self.dirty()?;
        let mut listing = String::new();
        for (path, hash) in &files {
            listing.push_str(path);
            listing.push('\n');
            listing.push_str(hash);
            listing.push('\n');
        }
        Ok(Candidate {
            id: CandidateId {
                base_commit,
                dirty_digest: sha256::hex(listing.as_bytes()),
            },
            provenance: "git".to_owned(),
            changed_paths: files.into_iter().map(|(path, _)| path).collect(),
        })
    }

    /// HEAD, and every path that differs from it (or is untracked) with the hash of its content
    /// (`deleted` for a removed one), sorted by path.
    fn dirty(&self) -> Result<(String, Vec<(String, String)>), Unobserved> {
        // One git for both: HEAD on the first line, the subdirectory prefix (empty at the root) on
        // the second. Scope and exclusions are worktree-relative, even from a subdirectory.
        let answer = self.git(&["rev-parse", "HEAD", "--show-prefix"])?;
        let mut lines = answer.lines();
        let base_commit = lines.next().unwrap_or_default().trim().to_owned();
        let prefix = lines.next().unwrap_or_default().trim().to_owned();
        let status = self.git(&[
            "status",
            "--porcelain=v1",
            "-z",
            "--untracked-files=all",
            "--",
            ".",
        ])?;
        let mut paths = Vec::new();
        let mut fields = status.split('\0');
        while let Some(entry) = fields.next() {
            let (Some(code), Some(path)) = (entry.get(..2), entry.get(3..)) else {
                continue;
            };
            if code.contains(['R', 'C'])
                && let Some(from) = fields.next()
            {
                paths.push(from.to_owned());
            }
            paths.push(path.to_owned());
        }
        let mut changed_paths: Vec<String> = paths
            .iter()
            .map(|p| p.strip_prefix(prefix.as_str()).unwrap_or(p).to_owned())
            .filter(|p| !p.is_empty() && !p.starts_with(".supervisor/"))
            .collect();
        changed_paths.sort();
        changed_paths.dedup();
        let mut files = Vec::new();
        for path in changed_paths {
            let hash = content_hash(&self.cwd.join(&path))
                .map_err(|e| Unobserved::WorktreeUnreadable(format!("cannot read {path}: {e}")))?;
            files.push((path, hash));
        }
        Ok((base_commit, files))
    }
}

/// The hash a candidate lists for a file: of its bytes, or `deleted` when it is gone.
fn content_hash(file: &Path) -> std::io::Result<String> {
    match std::fs::read(file) {
        Ok(bytes) => Ok(sha256::hex(&bytes)),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok("deleted".to_owned()),
        Err(e) => Err(e),
    }
}

/// G48: the file a task start writes into the task's folder.
pub(crate) const BASELINE: &str = "baseline.json";

/// G48: what was already changed in `project` when a task starts there: its root and each such
/// file with its content hash. `None` outside a repository or when git does not answer; the
/// task then has no baseline and every changed file counts as its own, as before.
pub(crate) fn baseline(project: &Path) -> Option<String> {
    let root = crate::location::checked_project(project).ok()?;
    let (_, files) = GitCandidate {
        cwd: PathBuf::from(&root),
    }
    .dirty()
    .ok()?;
    let files: serde_json::Map<String, serde_json::Value> = files
        .into_iter()
        .map(|(path, hash)| (path, hash.into()))
        .collect();
    Some(format!(
        "{}\n",
        serde_json::json!({"schema":1, "root":root, "files":files})
    ))
}

/// G48: `candidate` without the changed paths the task inherited: those its baseline lists whose
/// content is still what it was at the start. A file the task changed further, deleted or first
/// touched stays. Only the path list is trimmed (it feeds the scope check); the candidate's
/// identity still covers every changed file. With no baseline, one that does not parse, or one
/// taken for another root than `root`, nothing is removed.
pub(crate) fn without_inherited(
    task_dir: &Path,
    root: &str,
    mut candidate: Candidate,
) -> Candidate {
    let Some(baseline) = std::fs::read_to_string(task_dir.join(BASELINE))
        .ok()
        .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
    else {
        return candidate;
    };
    let Some(files) = baseline["files"]
        .as_object()
        .filter(|_| baseline["root"].as_str() == Some(root))
    else {
        return candidate;
    };
    candidate.changed_paths.retain(|path| {
        files
            .get(path)
            .and_then(|was| was.as_str())
            .is_none_or(|was| {
                content_hash(&Path::new(root).join(path)).map_or(true, |now| now != was)
            })
    });
    candidate
}

impl GitCandidate {
    fn git(&self, args: &[&str]) -> Result<String, Unobserved> {
        let out = crate::git::run(&self.cwd, args).map_err(|e| match e {
            crate::git::Failure::Unavailable(why) => Unobserved::GitUnavailable(why),
            crate::git::Failure::TimedOut(why) => Unobserved::GitFailed(why),
        })?;
        if out.status.success() {
            Ok(String::from_utf8_lossy(&out.stdout).into_owned())
        } else {
            Err(Unobserved::GitFailed(format!(
                "git {} failed: {}",
                args.join(" "),
                String::from_utf8_lossy(&out.stderr).trim()
            )))
        }
    }
}
