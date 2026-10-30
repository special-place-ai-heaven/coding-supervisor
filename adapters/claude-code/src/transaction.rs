//! Serialized, recoverable adapter transactions. The core has no filesystem dependency.

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Component, Path, PathBuf};

use serde_json::{Value, json};

/// OS locks are released when the process dies; the persistent lock file is never removed.
pub struct Transaction {
    root: PathBuf,
    _lock: File,
}

pub fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let parent = path.parent().ok_or("state path has no parent")?;
    fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    let tmp = path.with_extension("supervisor-tmp");
    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(&tmp)
        .map_err(|e| format!("{}: {e}", tmp.display()))?;
    file.write_all(bytes)
        .and_then(|()| file.sync_all())
        .map_err(|e| format!("{}: {e}", tmp.display()))?;
    drop(file);
    fs::rename(&tmp, path).map_err(|e| format!("{}: {e}", path.display()))?;
    #[cfg(unix)]
    File::open(parent)
        .and_then(|f| f.sync_all())
        .map_err(|e| format!("{}: {e}", parent.display()))?;
    Ok(())
}

fn relative(path: &str) -> Result<&Path, String> {
    let path = Path::new(path);
    if path.as_os_str().is_empty()
        || path
            .components()
            .any(|c| !matches!(c, Component::Normal(_)))
    {
        return Err("invalid transaction destination".to_owned());
    }
    Ok(path)
}

impl Transaction {
    pub fn open(root: &Path) -> Result<Self, String> {
        fs::create_dir_all(root).map_err(|e| format!("{}: {e}", root.display()))?;
        let lock_path = root.join("state.lock");
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&lock_path)
            .map_err(|e| format!("{}: {e}", lock_path.display()))?;
        lock.lock()
            .map_err(|e| format!("{}: {e}", lock_path.display()))?;
        let tx = Self {
            root: root.to_path_buf(),
            _lock: lock,
        };
        tx.recover()?;
        Ok(tx)
    }

    fn pointer_digest(&self) -> Result<String, String> {
        match fs::read(self.root.join("active.json")) {
            Ok(bytes) => Ok(crate::sha256::hex(&bytes)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(crate::sha256::hex(b"")),
            Err(e) => Err(e.to_string()),
        }
    }

    fn install(&self, writes: &Value) -> Result<(), String> {
        for write in writes.as_array().ok_or("invalid transaction journal")? {
            let name = write
                .get("path")
                .and_then(Value::as_str)
                .ok_or("invalid transaction path")?;
            let text = write
                .get("text")
                .and_then(Value::as_str)
                .ok_or("invalid transaction content")?;
            let destination = self.root.join(relative(name)?);
            if name.starts_with("archives/") || name.starts_with("archives\\") {
                match fs::read(&destination) {
                    Ok(existing) if existing != text.as_bytes() => {
                        return Err(
                            "immutable archive already exists with different content".to_owned()
                        );
                    }
                    Ok(_) => continue,
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(e) => return Err(e.to_string()),
                }
            }
            atomic_write(&destination, text.as_bytes())?;
        }
        Ok(())
    }

    /// Replay is safe after any prefix of writes was installed. No journal is silently discarded.
    pub fn recover(&self) -> Result<(), String> {
        let path = self.root.join("transaction.json");
        let text = match fs::read_to_string(&path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(format!("{}: {e}", path.display())),
        };
        let journal: Value =
            serde_json::from_str(&text).map_err(|_| "invalid transaction journal".to_owned())?;
        let pointer = self.pointer_digest()?;
        if journal.get("before_active").and_then(Value::as_str) != Some(pointer.as_str())
            && journal.get("after_active").and_then(Value::as_str) != Some(pointer.as_str())
        {
            return Err(
                "transaction belongs to an obsolete active task; recovery refused".to_owned(),
            );
        }
        self.install(
            journal
                .get("writes")
                .ok_or("transaction write set missing")?,
        )?;
        fs::remove_file(&path).map_err(|e| format!("{}: {e}", path.display()))
    }

    pub fn commit(&self, writes: Vec<(PathBuf, String)>) -> Result<(), String> {
        let before_active = self.pointer_digest()?;
        let mut after_active = before_active.clone();
        let mut entries = Vec::new();
        for (path, text) in writes {
            let name = path.to_str().ok_or("non-UTF-8 transaction path")?;
            relative(name)?;
            if path == Path::new("active.json") {
                after_active = crate::sha256::hex(text.as_bytes());
            }
            entries.push(json!({"path": name, "text": text}));
        }
        let journal = json!({"schema":1,"before_active":before_active,"after_active":after_active,"writes":entries});
        atomic_write(
            &self.root.join("transaction.json"),
            format!("{journal}\n").as_bytes(),
        )?;
        self.recover()
    }

    pub fn root(&self) -> &Path {
        &self.root
    }
}

/// Only task data files are staged; lock files and the transaction journal never enter a task.
pub fn copy_task(source: &Path, destination: &Path) -> Result<(), String> {
    fs::create_dir_all(destination).map_err(|e| format!("{}: {e}", destination.display()))?;
    if !source.exists() {
        return Ok(());
    }
    for entry in fs::read_dir(source).map_err(|e| format!("{}: {e}", source.display()))? {
        let entry = entry.map_err(|e| e.to_string())?;
        if !entry.file_type().map_err(|e| e.to_string())?.is_file() {
            continue;
        }
        let name = entry.file_name();
        let name_text = name.to_string_lossy();
        if name_text.ends_with(".json") || name_text.ends_with(".jsonl") {
            if matches!(
                name_text.as_ref(),
                "transaction.json"
                    | "active.json"
                    | "sessions.json"
                    | "project.json"
                    | "policy.json"
                    | "connection.json"
            ) {
                continue;
            }
            fs::copy(entry.path(), destination.join(name)).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

pub fn task_writes(stage: &Path, prefix: &Path) -> Result<Vec<(PathBuf, String)>, String> {
    let mut writes = Vec::new();
    for entry in fs::read_dir(stage).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        if entry.file_type().map_err(|e| e.to_string())?.is_file() {
            let text = fs::read_to_string(entry.path()).map_err(|e| e.to_string())?;
            writes.push((prefix.join(entry.file_name()), text));
        }
    }
    writes.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(writes)
}
