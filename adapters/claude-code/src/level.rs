//! How strictly the gate judges: `off`, `lite`, `full` (default) or `ultra`. A `level` file in
//! the project's state root beats the global one in the user config directory.

use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

/// Ordered from least to most strict.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    /// Nothing is judged or written; only the state guard and the action policy answer.
    Off,
    /// Judged as `full`; refusals are reported instead of blocking.
    Lite,
    #[default]
    Full,
    /// `full`, plus writes outside the task's contract scope are refused before they happen.
    Ultra,
}

impl Level {
    pub fn parse(word: &str) -> Option<Self> {
        match word {
            "off" => Some(Self::Off),
            "lite" => Some(Self::Lite),
            "full" => Some(Self::Full),
            "ultra" => Some(Self::Ultra),
            _ => None,
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Lite => "lite",
            Self::Full => "full",
            Self::Ultra => "ultra",
        }
    }
}

/// The level a hook call runs at.
#[derive(Debug, Default)]
pub struct Setting {
    pub level: Level,
    /// Holds the global `level` file; agents may not write it.
    pub config_dir: Option<PathBuf>,
    /// Why a `level` file could not be used; the call then runs at `full`.
    pub error: Option<String>,
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
    state_root: &Path,
    config_dir: Option<&Path>,
) -> Result<(Level, &'static str), String> {
    if let Some(level) = read(state_root)? {
        return Ok((level, "project"));
    }
    if let Some(dir) = config_dir
        && let Some(level) = read(dir)?
    {
        return Ok((level, "global"));
    }
    Ok((Level::Full, "default"))
}
