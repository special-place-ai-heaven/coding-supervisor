//! `supervisor level` (show or set how strictly the gate judges) and `supervisor status`.

use std::fs;
use std::path::PathBuf;

use supervisor_claude_code::level::{self, Level};
use supervisor_claude_code::location::{resolve_state_dir, state_dir_path};
use supervisor_claude_code::status;

use crate::{Failure, Flags, parse_flags};

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok()
}

/// `--home`, else the user config dir from the environment (see `level::config_dir`).
fn config_dir(flags: &Flags) -> Option<PathBuf> {
    flags
        .value("--home")
        .map(PathBuf::from)
        .or_else(|| level::config_dir(&env))
}

/// The project's state directory, located without creating it.
fn state_dir(flags: &Flags) -> Result<PathBuf, Failure> {
    Ok(state_dir_path(&flags.project()?, &env).0)
}

pub fn run(args: &[String]) -> Result<(), Failure> {
    let (word, rest) = match args.split_first() {
        Some((word, rest)) if !word.starts_with("--") => (Some(word.as_str()), rest),
        _ => (None, args),
    };
    let flags = parse_flags(rest, &["--project", "--home"], &[])?;
    let Some(word) = word else {
        let (level, source) = level::effective(&state_dir(&flags)?, config_dir(&flags).as_deref())
            .map_err(Failure::Error)?;
        println!("level: {} ({source})", level.name());
        return Ok(());
    };
    let level = Level::parse(word).ok_or_else(|| {
        Failure::Usage(format!(
            "unknown level: {word}; expected off, lite, full or ultra"
        ))
    })?;
    let (dir, scope) = if flags.value("--project").is_some() {
        (resolve_state_dir(&flags.project()?, &env).0, "project")
    } else {
        let dir = config_dir(&flags).ok_or_else(|| {
            Failure::Error(
                "no user config directory: set SUPERVISOR_CONFIG_DIR or pass --home".to_owned(),
            )
        })?;
        (dir, "global")
    };
    let path = dir.join("level");
    fs::create_dir_all(&dir)
        .and_then(|()| fs::write(&path, format!("{}\n", level.name())))
        .map_err(|e| Failure::Error(format!("{}: {e}", path.display())))?;
    println!(
        "level {} written ({scope}): {}",
        level.name(),
        path.display()
    );
    Ok(())
}

pub fn status(args: &[String]) -> Result<(), Failure> {
    let flags = parse_flags(args, &["--project", "--home"], &[])?;
    let project = flags.project()?;
    let text = status::render(&state_dir(&flags)?, config_dir(&flags).as_deref())
        .map_err(Failure::Error)?;
    println!("project: {}\n{text}", project.display());
    Ok(())
}
