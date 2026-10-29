//! How strictly the gate judges: `off`, `lite`, `full` (default) or `ultra`. A per-project
//! `level` file in the state directory beats the global one in the user config directory.

use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

/// Ordered from least to most strict.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    /// Nothing is judged or written; only the state-path guard answers.
    Off,
    /// Judged as `full`, never blocked.
    Lite,
    Full,
    /// `full` plus full-tier criteria, every stop after a change, and out-of-scope writes refused.
    Ultra,
}

const NAMES: [(Level, &str); 4] = [
    (Level::Off, "off"),
    (Level::Lite, "lite"),
    (Level::Full, "full"),
    (Level::Ultra, "ultra"),
];

impl Level {
    pub fn parse(word: &str) -> Option<Self> {
        NAMES.iter().find(|(_, n)| *n == word).map(|(l, _)| *l)
    }
    pub fn name(self) -> &'static str {
        NAMES
            .iter()
            .find(|(l, _)| *l == self)
            .map_or("full", |(_, n)| n)
    }
}

/// `SUPERVISOR_CONFIG_DIR`, else `%APPDATA%\coding-supervisor` on Windows, else
/// `$XDG_CONFIG_HOME/coding-supervisor`, else `$HOME/.config/coding-supervisor`.
pub fn config_dir(env_lookup: &dyn Fn(&str) -> Option<String>) -> Option<PathBuf> {
    let var = |name: &str| env_lookup(name).filter(|v| !v.is_empty());
    if let Some(dir) = var("SUPERVISOR_CONFIG_DIR") {
        return Some(PathBuf::from(dir));
    }
    let app_data = if cfg!(windows) { var("APPDATA") } else { None };
    app_data
        .map(PathBuf::from)
        .or_else(|| var("XDG_CONFIG_HOME").map(PathBuf::from))
        .or_else(|| var("HOME").map(|h| PathBuf::from(h).join(".config")))
        .map(|base| base.join("coding-supervisor"))
}

/// The level named in `<dir>/level`; `None` when the file does not exist.
pub fn read(dir: &Path) -> Result<Option<Level>, String> {
    let path = dir.join("level");
    match fs::read_to_string(&path) {
        Ok(text) => Level::parse(text.trim()).map(Some).ok_or_else(|| {
            format!(
                "{}: unknown level {:?}; expected off, lite, full or ultra",
                path.display(),
                text.trim()
            )
        }),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("{}: {e}", path.display())),
    }
}

/// The effective level and where it came from: `project`, `global` or `default`.
pub fn effective(
    state_dir: &Path,
    config_dir: Option<&Path>,
) -> Result<(Level, &'static str), String> {
    if let Some(level) = read(state_dir)? {
        return Ok((level, "project"));
    }
    if let Some(dir) = config_dir
        && let Some(level) = read(dir)?
    {
        return Ok((level, "global"));
    }
    Ok((Level::Full, "default"))
}
