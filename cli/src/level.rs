//! `supervisor level`: show or set how strictly the gate judges.

use std::path::PathBuf;

use supervisor_claude_code::level::{self, Level};
use supervisor_claude_code::location::{resolve_state_dir_checked, resolve_state_dir_read_only};
use supervisor_claude_code::transaction::atomic_write;

use crate::{Failure, parse_flags};

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok()
}

pub fn run(args: &[String]) -> Result<(), Failure> {
    // `status` shows the mode like the bare command (M3: the plugin's /supervisor status).
    let (word, rest) = match args.split_first() {
        Some((word, rest)) if word == "status" => (None, rest),
        Some((word, rest)) if !word.starts_with("--") => (Some(word.as_str()), rest),
        _ => (None, args),
    };
    let flags = parse_flags(rest, &["--project"], &["--global"])?;
    let config_dir = level::config_dir(&env);
    let Some(word) = word else {
        if flags.switch("--global") {
            return Err(Failure::Usage(
                "--global applies only when setting a level".to_owned(),
            ));
        }
        let (root, _) =
            resolve_state_dir_read_only(&flags.project()?, &env).map_err(Failure::Error)?;
        let (level, source) =
            level::effective(&root, config_dir.as_deref()).map_err(Failure::Error)?;
        println!("level: {} ({source})", level.name());
        return Ok(());
    };
    let level = Level::parse(word).ok_or_else(|| {
        Failure::Usage(format!(
            "unknown level: {word}; expected off, audit, enforce or strict (lite, full and ultra still work)"
        ))
    })?;
    let (dir, scope): (PathBuf, &str) = if flags.switch("--global") {
        let dir = config_dir.ok_or_else(|| {
            Failure::Error("no user config directory: set SUPERVISOR_CONFIG_DIR".to_owned())
        })?;
        (dir, "global")
    } else {
        let (root, _) =
            resolve_state_dir_checked(&flags.project()?, &env).map_err(Failure::Error)?;
        (root, "project")
    };
    let path = dir.join("level");
    atomic_write(&path, format!("{}\n", level.name()).as_bytes()).map_err(Failure::Error)?;
    println!(
        "level {} written ({scope}): {}",
        level.name(),
        path.display()
    );
    Ok(())
}
