//! Candidate identity: HEAD plus the dirty-tree digest of the entire physical worktree.

use std::fmt;
use std::io::ErrorKind;
use std::path::PathBuf;

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
        let base_commit = self.git(&["rev-parse", "HEAD"])?.trim().to_owned();
        // Scope and exclusions are worktree-relative for hooks and MCP, even from a subdirectory.
        let prefix = self.git(&["rev-parse", "--show-prefix"])?.trim().to_owned();
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
        let mut listing = String::new();
        for path in &changed_paths {
            let hash = match std::fs::read(self.cwd.join(path)) {
                Ok(bytes) => sha256::hex(&bytes),
                Err(e) if e.kind() == ErrorKind::NotFound => "deleted".to_owned(),
                Err(e) => {
                    return Err(Unobserved::WorktreeUnreadable(format!(
                        "cannot read {path}: {e}"
                    )));
                }
            };
            listing.push_str(path);
            listing.push('\n');
            listing.push_str(&hash);
            listing.push('\n');
        }
        Ok(Candidate {
            id: CandidateId {
                base_commit,
                dirty_digest: sha256::hex(listing.as_bytes()),
            },
            provenance: "git".to_owned(),
            changed_paths,
        })
    }
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
