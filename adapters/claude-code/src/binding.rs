//! G5: a task bound to a linked worktree of the session's repository. The binding is the
//! `worktree` field of the task's `autonomy.json` (an agent's start), else of its `task.json`
//! (WP3: a person's `task start --worktree`); a task without either keeps the hook's own cwd.

use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::candidate::{CandidateProvider, GitCandidate};
use crate::location::{canonical_project, common_git_dir, linked_main_checkout, normalize};

/// The worktree root `task_dir`'s task is bound to, in the form `canonical_project` yields.
pub fn bound_root(task_dir: &Path) -> Option<String> {
    ["autonomy.json", "task.json"].into_iter().find_map(|file| {
        let text = std::fs::read_to_string(task_dir.join(file)).ok()?;
        let metadata: Value = serde_json::from_str(&text).ok()?;
        metadata.get("worktree")?.as_str().map(str::to_owned)
    })
}

/// The root a run or edit of the task counts in: its bound worktree, else `project`'s root.
pub(crate) fn execution_root(task_dir: &Path, project: &Path) -> String {
    bound_root(task_dir).unwrap_or_else(|| canonical_project(project))
}

/// Validate a requested `worktree` of a `start` or `revise`. `None` means no binding: absent, or
/// the project itself. A path that does not exist yet is accepted and checked when it appears.
pub fn resolve(project: &Path, requested: &str) -> Result<Option<String>, String> {
    let path = Path::new(requested);
    let path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        project.join(path)
    };
    if normalize(&path) == canonical_project(project) {
        return Ok(None);
    }
    if !path.exists() {
        return Ok(Some(normalize(&path)));
    }
    same_repository(&path, project)?;
    Ok(Some(canonical_project(&path)))
}

fn same_repository(worktree: &Path, project: &Path) -> Result<(), String> {
    match (common_git_dir(worktree), common_git_dir(project)) {
        (Some(a), Some(b)) if a == b => Ok(()),
        _ => Err(format!(
            "{} is not a worktree of this repository",
            worktree.display()
        )),
    }
}

/// A candidate that cannot be observed, with the reason the gate shows.
struct NotObservable(String);
impl CandidateProvider for NotObservable {
    fn observe(&self) -> Result<coding_supervisor::Candidate, String> {
        Err(self.0.clone())
    }
}

/// Observes the bound worktree of `task_dir`'s task, else `project`.
pub fn provider(task_dir: Option<&Path>, project: &Path) -> Box<dyn CandidateProvider> {
    let Some(bound) = task_dir.and_then(bound_root) else {
        return Box::new(GitCandidate {
            cwd: project.to_path_buf(),
        });
    };
    let bound = PathBuf::from(bound);
    if !bound.exists() {
        return Box::new(NotObservable(format!(
            "the task is bound to worktree {}, which does not exist yet; create it with git worktree add",
            bound.display()
        )));
    }
    match same_repository(&bound, project) {
        Ok(()) => Box::new(GitCandidate { cwd: bound }),
        Err(why) => Box::new(NotObservable(why)),
    }
}

/// [`provider`] for the active task under the state root `root`.
pub fn provider_for_root(root: &Path, project: &Path) -> Box<dyn CandidateProvider> {
    let active = crate::lifecycle::active_task(root).ok().flatten();
    provider(active.as_ref().map(|a| a.dir.as_path()), project)
}

/// The main checkout whose state root holds this session's task when `cwd` lies in the linked
/// worktree that task is bound to; `None` otherwise (unbound worktrees stay with G4).
pub fn redirect(cwd: &Path, session: &str) -> Option<PathBuf> {
    // File reads only: this runs on every hook call, before any git.
    let worktree = crate::location::worktree_root(cwd)?;
    let main = linked_main_checkout(&worktree)?;
    let lookup = |name: &str| std::env::var(name).ok();
    let (root, _) = crate::location::resolve_state_dir_read_only(&main, &lookup).ok()?;
    let active = crate::lifecycle::active_task(&root).ok().flatten()?;
    (bound_root(&active.dir)? == normalize(&worktree)
        && crate::autonomy::owns_session(&active.dir, session) == Ok(true))
    .then_some(main)
}

/// The checkout `root`'s project.json names, when `cwd` is a linked worktree of it (a redirected
/// hook call); else `cwd`. The state identity check compares against this.
pub(crate) fn state_project(root: &Path, cwd: &Path) -> PathBuf {
    let named = std::fs::read_to_string(root.join("project.json"))
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .and_then(|v| v["canonical_worktree"].as_str().map(str::to_owned));
    let Some(worktree) = crate::location::worktree_root(cwd) else {
        return cwd.to_path_buf();
    };
    match (named, linked_main_checkout(&worktree)) {
        (Some(named), Some(main)) if normalize(&main) == named && normalize(&worktree) != named => {
            main
        }
        _ => cwd.to_path_buf(),
    }
}
