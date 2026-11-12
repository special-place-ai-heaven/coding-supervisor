//! WP3: the git worktree a person's `supervisor task start --worktree <path>` creates for the task
//! (branch `supervisor/<task id>` at the contract's base commit), and `supervisor task cleanup`,
//! which removes it again. Nothing here ever forces git: a worktree holding work stays.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::{Value, json};

use crate::location::{
    canonical_project, common_git_dir, current_branch, normalize, worktree_root,
};
use crate::transaction::Transaction;

/// shortcut: a guess for `worktree add` and `worktree remove` on a large checkout; raise it when
/// a real checkout is seen to take longer. Hooks keep git's own five-second deadline.
const DEADLINE: Duration = Duration::from_secs(120);

/// The branch a task's worktree is created on.
pub fn branch(task_id: &str) -> String {
    format!("supervisor/{task_id}")
}

/// git's trimmed stdout on success, else its stderr (or why it gave no answer).
fn git(cwd: &Path, args: &[&str]) -> Result<String, String> {
    match crate::git::run_for(cwd, args, DEADLINE) {
        Ok(out) if out.status.success() => {
            Ok(String::from_utf8_lossy(&out.stdout).trim().to_owned())
        }
        Ok(out) => Err(format!(
            "git {} failed: {}",
            args.first().copied().unwrap_or_default(),
            String::from_utf8_lossy(&out.stderr).trim()
        )),
        Err(crate::git::Failure::Unavailable(why) | crate::git::Failure::TimedOut(why)) => Err(why),
    }
}

/// The checks made before anything is created: `path` does not exist yet and is not inside the
/// project's checkout (a nested worktree would show in its `git status`). The stored form of the
/// path (`location::normalize`).
pub(crate) fn check_request(project: &Path, path: &Path) -> Result<String, String> {
    if fs::symlink_metadata(path).is_ok() {
        return Err(format!(
            "{} already exists; --worktree creates a new worktree, so name a path that does not exist yet",
            path.display()
        ));
    }
    let wanted = normalize(path);
    let checkout = canonical_project(project);
    if wanted == checkout || wanted.starts_with(&format!("{checkout}/")) {
        return Err(format!(
            "{} is inside the main checkout {checkout}; name a path outside it",
            path.display()
        ));
    }
    Ok(wanted)
}

/// `git worktree add -b <branch> <path> <base>`, then the checks of path, branch, base and a
/// clean tree. A failed check removes the new worktree again without force, when git allows.
pub(crate) fn create(
    project: &Path,
    path: &Path,
    stored: &str,
    branch: &str,
    base: &str,
) -> Result<(), String> {
    let target = path.to_string_lossy();
    git(
        project,
        &["worktree", "add", "-q", "-b", branch, &target, base],
    )
    .map_err(|why| format!("the worktree could not be created: {why}"))?;
    verify(project, path, stored, branch, base)
        .and_then(|()| {
            // A session in the new worktree finds a task started from a linked worktree only
            // through this record; the primary checkout it finds from the worktree's `.git`.
            let checkout = canonical_project(project);
            match crate::location::linked_main_checkout(Path::new(&checkout)) {
                Some(_) => crate::location::record_started_from(path, &checkout),
                None => Ok(()),
            }
        })
        .map_err(|why| rollback(project, path, branch, &why))
}

fn verify(
    project: &Path,
    path: &Path,
    stored: &str,
    branch: &str,
    base: &str,
) -> Result<(), String> {
    let top = git(path, &["rev-parse", "--show-toplevel"])?;
    if normalize(Path::new(&top)) != stored {
        return Err(format!(
            "the new worktree's top level is {top}, not {stored}"
        ));
    }
    if common_git_dir(path).is_none() || common_git_dir(path) != common_git_dir(project) {
        return Err("the new worktree is not a worktree of this repository".to_owned());
    }
    if current_branch(path).as_deref() != Some(branch) {
        return Err(format!("the new worktree is not on branch {branch}"));
    }
    let head = git(path, &["rev-parse", "HEAD"])?;
    if head != base {
        return Err(format!(
            "the new worktree is at {head}, not the base commit {base}"
        ));
    }
    let status = git(path, &["status", "--porcelain=v1", "--untracked-files=all"])?;
    if !status.is_empty() {
        return Err(
            "the new worktree is not clean (git status lists changes, for example from a checkout hook)"
                .to_owned(),
        );
    }
    Ok(())
}

/// Undo [`create`] after a failed check or commit: a plain `git worktree remove`, then
/// `git branch -d`. Returns `why` with what was undone or what is left for the person.
pub(crate) fn rollback(project: &Path, path: &Path, branch: &str, why: &str) -> String {
    let target = path.to_string_lossy();
    if let Err(left) = git(project, &["worktree", "remove", &target]) {
        return format!(
            "{why}; the task was not started, and the worktree {target} on branch {branch} is left as it is ({left}): look at it, then remove it with `git worktree remove` and `git branch -d {branch}`"
        );
    }
    match git(project, &["branch", "-d", branch]) {
        Ok(_) => format!("{why}; the task was not started and the new worktree was removed again"),
        Err(left) => format!(
            "{why}; the task was not started and the new worktree was removed, but branch {branch} is left ({left}): delete it with `git branch -d {branch}`"
        ),
    }
}

/// `supervisor task cleanup --task <id>`: remove the worktree task `task_id` was created with,
/// keeping its branch. Refused while the task is active, when the worktree is no longer the
/// task's, while it holds uncommitted or untracked work, when it holds commits past the base
/// without a receipt (`supervisor-receipts/<id>.json` naming the task in its last commit), and
/// while it holds ignored files, which `git worktree remove` would delete, or a file marked
/// assume-unchanged or skip-worktree, whose edits git status hides. Links and junctions
/// git does not track are unlinked first, so `git worktree remove` never deletes through them;
/// their targets must still exist afterwards.
pub fn cleanup(root: &Path, project: &Path, task_id: &str) -> Result<Value, String> {
    crate::lifecycle::safe_id(task_id)?;
    let (path, base) = {
        let _tx = Transaction::open(root)?;
        let dir = root.join("tasks").join(task_id);
        let metadata: Value = fs::read_to_string(dir.join("task.json"))
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .ok_or_else(|| format!("this project's state has no task {task_id}"))?;
        let (contract, _) = crate::state::read_contract(&dir)?;
        let path = metadata["worktree"].as_str().filter(|_| {
            contract
                .as_ref()
                .is_some_and(|c| c.isolation == coding_supervisor::Isolation::Worktree)
        });
        let Some(path) = path.map(str::to_owned) else {
            return Err(format!(
                "only a worktree the supervisor created is cleaned up; task {task_id} has none"
            ));
        };
        if let Some(active) = crate::lifecycle::active_task(root)? {
            if active.task_id == task_id {
                return Err(format!(
                    "task {task_id} is the active task; end it first with `supervisor task deactivate --task {task_id}` (or `supervisor task next`), then clean up"
                ));
            }
            if crate::binding::bound_root(&active.dir).as_deref() == Some(path.as_str()) {
                return Err(format!(
                    "the active task {} works in the same worktree {path}; nothing was touched",
                    active.task_id
                ));
            }
        }
        (path, contract.and_then(|c| c.base_sha))
    };
    let worktree = PathBuf::from(&path);
    let branch = branch(task_id);
    if fs::symlink_metadata(&worktree).is_err() {
        git(project, &["worktree", "prune"])?;
        return Ok(json!({"removed":null,"branch":branch,"links_unlinked":0,
            "note":format!("{path} was already gone; git's record of it was pruned")}));
    }
    let ours = worktree_root(&worktree).is_some_and(|top| normalize(&top) == path)
        && crate::location::linked_main_checkout(&worktree).is_some()
        && common_git_dir(&worktree).is_some()
        && common_git_dir(&worktree) == common_git_dir(project)
        && current_branch(&worktree).as_deref() == Some(branch.as_str());
    if !ours {
        return Err(format!(
            "{path} is no longer this repository's worktree on branch {branch}; nothing was touched"
        ));
    }
    let status = git(
        &worktree,
        &["status", "--porcelain=v1", "--untracked-files=all"],
    )?;
    if !status.is_empty() {
        return Err(format!(
            "{path} holds uncommitted or untracked work; commit it (with its receipt) or remove the worktree yourself; nothing was touched"
        ));
    }
    let head = git(&worktree, &["rev-parse", "HEAD"])?;
    if base.as_deref() != Some(head.as_str()) && !receipt_in_head(&worktree, task_id) {
        return Err(format!(
            "the committed work on {branch} has no receipt ({}/{task_id}.json in its last commit): run `supervisor receipt export` before committing the work, or remove the worktree yourself; nothing was touched",
            crate::git::RECEIPTS_DIR
        ));
    }
    // `-v` tags each path; a lowercase tag (assume-unchanged) or `S` (skip-worktree) marks a file
    // whose edits git status does not show and `git worktree remove` would delete. A marked file
    // absent from the worktree (a sparse checkout) holds nothing to lose.
    let listing = git(&worktree, &["ls-files", "-v", "-z"])?;
    let entries: Vec<(char, &str)> = listing
        .split('\0')
        .filter_map(|entry| {
            let mut chars = entry.chars();
            let tag = chars.next()?;
            Some((tag, chars.as_str().strip_prefix(' ')?))
        })
        .collect();
    let marked: Vec<&str> = entries
        .iter()
        .filter(|(tag, file)| {
            (tag.is_ascii_lowercase() || *tag == 'S') && marked_file_present(&worktree, file)
        })
        .map(|(_, file)| *file)
        .collect();
    if let Some(first) = marked.first() {
        let shown = marked
            .iter()
            .take(3)
            .copied()
            .collect::<Vec<_>>()
            .join(", ");
        let more = if marked.len() > 3 { ", ..." } else { "" };
        return Err(format!(
            "{path} holds {} files marked assume-unchanged or skip-worktree ({shown}{more}), so git status does not show edits to them and removing the worktree would delete them; clear each mark (`git -C \"{path}\" update-index --no-assume-unchanged --no-skip-worktree -- \"{first}\"`) and commit or discard the edits, then run cleanup again; nothing was touched",
            marked.len()
        ));
    }
    let tracked: HashSet<String> = entries.iter().map(|(_, p)| (*p).to_owned()).collect();
    let (links, ignored) = untracked_entries(&worktree, &tracked)?;
    if !ignored.is_empty() {
        let shown = ignored
            .iter()
            .take(3)
            .cloned()
            .collect::<Vec<_>>()
            .join(", ");
        let more = if ignored.len() > 3 { ", ..." } else { "" };
        return Err(format!(
            "{path} holds {} ignored files that removing the worktree would delete ({shown}{more}); delete them or move them out of the worktree, then run cleanup again; nothing was touched",
            ignored.len()
        ));
    }
    for (link, kind) in &links {
        unlink(link, *kind)?;
    }
    let unlinked = links.len();
    let target = worktree.to_string_lossy();
    git(project, &["worktree", "remove", &target]).map_err(|why| {
        format!(
            "{why}; {unlinked} untracked or ignored links were unlinked first (their targets are kept); the worktree is otherwise as it was"
        )
    })?;
    git(project, &["worktree", "prune"])?;
    Ok(json!({"removed":path,"branch":branch,"links_unlinked":unlinked}))
}

/// Whether a marked file named `name` by `git ls-files` may be in `worktree`. Absent only when
/// the lookup says "not found" for a name that decoded: a U+FFFD stands for bytes the lossy
/// decode lost, so the name looked up is not the file's, and any other error proves nothing.
fn marked_file_present(worktree: &Path, name: &str) -> bool {
    name.contains('\u{FFFD}')
        || !matches!(
            fs::symlink_metadata(worktree.join(name)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound
        )
}

/// Whether the worktree's last commit holds a receipt bundle naming `task_id`.
fn receipt_in_head(worktree: &Path, task_id: &str) -> bool {
    let spec = format!("HEAD:{}/{task_id}.json", crate::git::RECEIPTS_DIR);
    git(worktree, &["show", &spec])
        .ok()
        .and_then(|text| serde_json::from_str::<Value>(&text).ok())
        .is_some_and(|bundle| bundle["task_id"] == task_id)
}

/// A link or junction found in a worktree, with its file type.
type Link = (PathBuf, fs::FileType);

/// Every link and junction under `worktree` that git does not track, found without entering any,
/// and every other file git does not track: with a clean status those are ignored files, which
/// `git worktree remove` would delete (relative paths). Only the worktree's own `.git` file is
/// skipped: a `.git` deeper down, link or folder, is looked at like any other entry. A tracked
/// folder entry (a submodule) is not entered.
fn untracked_entries(
    worktree: &Path,
    tracked: &HashSet<String>,
) -> Result<(Vec<Link>, Vec<String>), String> {
    let (mut links, mut ignored) = (Vec::new(), Vec::new());
    let own_git = worktree.join(".git");
    let mut dirs = vec![worktree.to_path_buf()];
    while let Some(dir) = dirs.pop() {
        let entries = fs::read_dir(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        for entry in entries {
            let path = entry.map_err(|e| format!("{}: {e}", dir.display()))?.path();
            if path == own_git {
                continue;
            }
            let kind = fs::symlink_metadata(&path)
                .map_err(|e| format!("{}: {e}", path.display()))?
                .file_type();
            let relative = path
                .strip_prefix(worktree)
                .map(|r| r.to_string_lossy().replace('\\', "/"))
                .unwrap_or_default();
            // A tracked entry is git's: removing a tracked link would make the tree dirty.
            if tracked.contains(&relative) {
                continue;
            }
            if kind.is_symlink() {
                links.push((path, kind));
            } else if kind.is_dir() {
                dirs.push(path);
            } else {
                ignored.push(relative);
            }
        }
    }
    Ok((links, ignored))
}

fn unlink(link: &Path, kind: fs::FileType) -> Result<(), String> {
    let target = fs::read_link(link).ok().map(|t| match link.parent() {
        Some(parent) if t.is_relative() => parent.join(t),
        _ => t,
    });
    let existed = target.as_ref().is_some_and(|t| t.exists());
    #[cfg(windows)]
    let directory = std::os::windows::fs::FileTypeExt::is_symlink_dir(&kind);
    #[cfg(not(windows))]
    let directory = {
        let _ = kind;
        false
    };
    // A directory link or junction is removed as the link itself, never with its contents.
    if directory {
        fs::remove_dir(link)
    } else {
        fs::remove_file(link)
    }
    .map_err(|e| format!("{} could not be unlinked: {e}", link.display()))?;
    if let Some(target) = target.filter(|t| existed && !t.exists()) {
        return Err(format!(
            "after unlinking {}, its target {} is gone; stopped before removing the worktree",
            link.display(),
            target.display()
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// G56: only a lookup that says "not found" makes a marked file absent; a name git gave that
    /// did not decode, or any other lookup error, keeps it present, so cleanup refuses.
    #[test]
    fn a_marked_file_is_absent_only_when_its_lookup_says_not_found() {
        let dir = std::env::temp_dir().join(format!("supervisor-marked-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("kept.txt"), "kept").unwrap();
        assert!(marked_file_present(&dir, "kept.txt"));
        assert!(!marked_file_present(&dir, "gone.txt"));
        // The lossy decode of a name that is not UTF-8: the file on disk has other bytes.
        assert!(marked_file_present(&dir, "bad\u{FFFD}.txt"));
        // A name this file system cannot hold: the lookup fails without saying "not found".
        #[cfg(windows)]
        assert!(marked_file_present(&dir, "a<b.txt"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let locked = dir.join("locked");
            fs::create_dir_all(&locked).unwrap();
            fs::write(locked.join("f.txt"), "kept").unwrap();
            fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();
            let present = marked_file_present(&dir, "locked/f.txt");
            fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();
            assert!(present);
        }
        let _ = fs::remove_dir_all(&dir);
    }
}
