//! Explicit task activation, rollover and recovery. Installation never activates a task.

use std::fs;
use std::path::Path;

use serde_json::Value;
use supervisor_claude_code::{lifecycle, location};

use crate::{Failure, Flags};

pub fn run(command: &str, flags: &Flags) -> Result<(), Failure> {
    let project = flags.project()?;
    let (root, _) = location::resolve_state_dir_checked(&project, &|name| std::env::var(name).ok())
        .map_err(Failure::Error)?;
    let needed = |name| {
        flags
            .value(name)
            .ok_or_else(|| Failure::Usage(format!("{command} requires {name}")))
    };
    let outcome = match command {
        "start" | "next" => {
            let path = Path::new(needed("--contract")?);
            let text = fs::read_to_string(path)
                .map_err(|e| Failure::Error(format!("{}: {e}", path.display())))?;
            let contract: Value = serde_json::from_str(&text)
                .map_err(|_| Failure::Error("contract is not valid JSON".to_owned()))?;
            let writer = supervisor_claude_code::state::Writer::task_start(Some(&project));
            let (next, requested, expected) = (
                command == "next",
                flags.value("--disposition"),
                flags.value("--task"),
            );
            match flags.value("--worktree") {
                Some(worktree) => {
                    let worktree = std::path::absolute(worktree)
                        .map_err(|e| Failure::Error(format!("{worktree}: {e}")))?;
                    lifecycle::start_in_worktree(
                        &root, &writer, &contract, next, requested, expected, &worktree,
                    )
                }
                None => lifecycle::start_as(&root, &writer, &contract, next, requested, expected),
            }
        }
        "cleanup" => supervisor_claude_code::worktree::cleanup(&root, &project, needed("--task")?),
        "resume" => lifecycle::resume(&root, needed("--task")?),
        "deactivate" => {
            lifecycle::deactivate(&root, needed("--task")?, flags.value("--disposition"))
        }
        "rebind" => lifecycle::rebind(&root, needed("--task")?, needed("--session")?),
        "migrate" => lifecycle::migrate(&root, Path::new(needed("--legacy")?)),
        "reset-rounds" => lifecycle::reset_rounds(&root, needed("--task")?, needed("--reason")?),
        "cancel" => {
            supervisor_claude_code::autonomy::cancel(&root, needed("--task")?, needed("--reason")?)
        }
        "add-check" => supervisor_claude_code::autonomy::add_check(
            &root,
            &project,
            needed("--criterion")?,
            needed("--command")?,
            needed("--reason")?,
        ),
        "amend-floor" => supervisor_claude_code::autonomy::amend_floor(
            &root,
            &project,
            needed("--criterion")?,
            needed("--command")?,
            needed("--reason")?,
        ),
        _ => return Err(Failure::Usage(format!("unknown task command: {command}"))),
    }
    .map_err(Failure::Error)?;
    println!("{outcome}");
    Ok(())
}
