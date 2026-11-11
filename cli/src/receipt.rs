//! WP6: `supervisor receipt sign|verify` and the signature line of `supervisor status`.
//!
//! The signature lives beside the task in `signature.json`, outside gate.json and outside the
//! replay digest, so signing never changes a gate decision. It covers a fixed text statement
//! (see [`crate::signing::Statement`]) recomputed from the stored state on every read.

use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use ed25519_dalek::{SigningKey, VerifyingKey};
use serde_json::{Value, json};
use supervisor_claude_code::adapter::now;
use supervisor_claude_code::lifecycle;
use supervisor_claude_code::location::{canonical_project, resolve_state_dir_read_only};
use supervisor_claude_code::state::{Stored, load_gate_head, read_contract};
use supervisor_claude_code::transaction::{atomic_write, read_lock};

use crate::signing::{self, KeyStore, Statement};
use crate::{Failure, Flags};

const SIDECAR: &str = "signature.json";
const KEYS_FILE: &str = "supervisor-keys.json";

pub fn run(action: &str, flags: &Flags) -> Result<(), Failure> {
    let project = PathBuf::from(canonical_project(&flags.project()?));
    let env = |key: &str| std::env::var(key).ok();
    let (root, _) = resolve_state_dir_read_only(&project, &env)
        .map_err(|e| Failure::Error(format!("receipt: project state unusable: {e}")))?;
    // `sign` reads the key store before taking the lock, so a slow store never makes a hook
    // wait. The unlocked look only decides whether there is anything to sign; any error there
    // is left to the read under the lock, which alone decides.
    let key = (action == "sign" && !matches!(current(&root), Ok(Err(_))))
        .then(|| load_key(&env, &project));
    // Held for the rest of the command: a hook transaction, which rewrites every task file it
    // staged, cannot interleave with the read and the sidecar write.
    let _lock = read_lock(&root)
        .map_err(|e| Failure::Error(format!("receipt: project state not readable: {e}")))?;
    let current = current(&root).map_err(|e| Failure::Error(format!("receipt: {e}")))?;
    match action {
        "sign" => {
            println!("{}", sign(current, key));
            Ok(())
        }
        "verify" => {
            let keys = flags
                .value("--keys")
                .map_or_else(|| project.join(KEYS_FILE), PathBuf::from);
            let answer = verify(current, &keys);
            println!("{answer}");
            if answer["signature"] == "valid" {
                Ok(())
            } else {
                Err(Failure::Unmet)
            }
        }
        _ => Err(Failure::Usage("receipt requires sign or verify".to_owned())),
    }
}

/// The active task's folder and the statement its stored state yields now. `Ok(Err(reason))`:
/// there is nothing to sign. `Err`: the state could not be read or its chain is broken.
fn current(root: &Path) -> Result<Result<(PathBuf, Statement), &'static str>, String> {
    let Some(task) = lifecycle::active_task(root)? else {
        return Ok(Err("there is no active task"));
    };
    let (contract, digest) = read_contract(&task.dir)?;
    if contract.is_none() {
        return Err("the active task has no contract".to_owned());
    }
    match load_gate_head(&task.dir, &digest)? {
        (Stored::Valid(gate), Some(head)) => Ok(Ok((
            task.dir,
            Statement::new(
                task.task_id,
                digest,
                head,
                gate.checkpoint.candidate.as_ref(),
            ),
        ))),
        (Stored::Absent, _) => Ok(Err("the active task has no stored gate state yet")),
        (Stored::Tampered(what), _) => Err(format!("task checkpoint tampered ({what})")),
        (Stored::Valid(_), None) => Err("task checkpoint tampered (checkpoint)".to_owned()),
    }
}

fn unsigned(reason: &str) -> Value {
    json!({"signature":"unsigned","reason":reason})
}

/// The key store and its key, created at the first sign.
pub(crate) fn load_key(
    env: &dyn Fn(&str) -> Option<String>,
    project: &Path,
) -> Result<(KeyStore, SigningKey), String> {
    let store = KeyStore::select(env, project)?;
    let key = store.signing_key()?;
    Ok((store, key))
}

/// Never fakes a signature: any key store problem is `unsigned` with the reason. `key` is
/// `None` when the unlocked look found nothing to sign.
fn sign(
    current: Result<(PathBuf, Statement), &'static str>,
    key: Option<Result<(KeyStore, SigningKey), String>>,
) -> Value {
    let (dir, statement) = match current {
        Ok(current) => current,
        Err(reason) => return unsigned(reason),
    };
    let bytes = match statement.bytes() {
        Ok(bytes) => bytes,
        Err(reason) => return unsigned(&reason),
    };
    let (store, key) = match key {
        Some(Ok(key)) => key,
        Some(Err(reason)) => return unsigned(&reason),
        // The task appeared between the two looks; sign again.
        None => return unsigned("the active task appeared while signing; run sign again"),
    };
    let public = key.verifying_key();
    let key_id = signing::key_id(&public);
    let mut sidecar = statement_json(&statement);
    sidecar["key_id"] = key_id.clone().into();
    sidecar["signature"] = signing::sign(&key, &bytes).into();
    sidecar["storage"] = store.label().into();
    sidecar["signed_at"] = crate::verdict::rfc3339_millis(now()).into();
    if atomic_write(&dir.join(SIDECAR), format!("{sidecar}\n").as_bytes()).is_err() {
        return unsigned("the signature file could not be written beside the task");
    }
    let public = signing::hex(public.as_bytes());
    json!({
        "signature":"signed","key_id":key_id,"public_key":public,"storage":store.label(),
        "task_id":statement.task,
        // Add this to supervisor-keys.json at the repository root; nothing here edits it.
        "keys_entry":{"key_id":key_id,"public_key":public,"revoked_at":null},
    })
}

fn statement_json(statement: &Statement) -> Value {
    json!({
        "statement":signing::STATEMENT_TAG,"task":statement.task,
        "contract_sha256":statement.contract_sha256,"checkpoint_hash":statement.checkpoint_hash,
        "candidate":statement.candidate,
    })
}

/// The sidecar, if one exists; `Err` when it cannot be read or does not parse.
fn read_sidecar(dir: &Path) -> Result<Option<Value>, &'static str> {
    match fs::read_to_string(dir.join(SIDECAR)) {
        Ok(text) => serde_json::from_str(&text)
            .map(Some)
            .map_err(|_| "the signature file does not parse"),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(None),
        Err(_) => Err("the signature file could not be read"),
    }
}

/// Whether the sidecar was made for exactly the statement the stored state yields now.
fn matches(sidecar: &Value, statement: &Statement) -> bool {
    let expected = statement_json(statement);
    expected
        .as_object()
        .is_some_and(|fields| fields.iter().all(|(k, v)| sidecar.get(k) == Some(v)))
}

fn verify(current: Result<(PathBuf, Statement), &'static str>, keys: &Path) -> Value {
    let invalid = |reason: &str| json!({"signature":"invalid","reason":reason});
    let (dir, statement) = match current {
        Ok(current) => current,
        Err(reason) => return unsigned(reason),
    };
    let sidecar = match read_sidecar(&dir) {
        Ok(Some(sidecar)) => sidecar,
        Ok(None) => return unsigned("the active task has no signature"),
        Err(reason) => return invalid(reason),
    };
    if !matches(&sidecar, &statement) {
        return invalid("the stored state changed after signing");
    }
    // Key ids are hex: compared without regard to letter case.
    let Some(key_id) = sidecar["key_id"].as_str().map(str::to_ascii_lowercase) else {
        return invalid("the signature file names no key id");
    };
    let key_id = key_id.as_str();
    let public = match listed_key(keys, key_id) {
        Ok(public) => public,
        Err(reason) => return invalid(reason),
    };
    // Checked over the statement recomputed from the stored state, never the sidecar's copy.
    let checked = statement.bytes().and_then(|bytes| {
        signing::verify(
            &public,
            &bytes,
            sidecar["signature"].as_str().unwrap_or_default(),
        )
    });
    match checked {
        Ok(()) => json!({"signature":"valid","key_id":key_id,"task_id":statement.task}),
        Err(reason) => invalid(&reason),
    }
}

/// The public key the keys file lists for `key_id` (lowercase hex), or why it cannot be used:
/// absent or unparsable file, not listed, revoked by any entry, malformed, or an id that does not
/// match its key.
pub(crate) fn listed_key(keys: &Path, key_id: &str) -> Result<VerifyingKey, &'static str> {
    let listed = match fs::read_to_string(keys) {
        Ok(text) => serde_json::from_str::<Value>(&text),
        Err(e) if e.kind() == ErrorKind::NotFound => return Err("the keys file is absent"),
        Err(_) => return Err("the keys file could not be read"),
    };
    let listed = listed.map_err(|_| "the keys file does not parse")?;
    let entries = listed["keys"]
        .as_array()
        .ok_or("the keys file has no keys list")?;
    let listed: Vec<&Value> = entries
        .iter()
        .filter(|e| e["key_id"].as_str().map(str::to_ascii_lowercase).as_deref() == Some(key_id))
        .collect();
    let entry = listed
        .first()
        .ok_or("the signing key is not listed in the keys file")?;
    // Any entry revoking the id counts, so listing the key again cannot undo a revocation.
    if listed.iter().any(|e| !e["revoked_at"].is_null()) {
        return Err("the signing key is revoked");
    }
    let public = entry["public_key"]
        .as_str()
        .and_then(signing::unhex::<32>)
        .and_then(|bytes| VerifyingKey::from_bytes(&bytes).ok())
        .ok_or("the listed public key is not a valid 64-hex Ed25519 key")?;
    if signing::key_id(&public) != key_id {
        return Err("the listed key id does not match its public key");
    }
    Ok(public)
}

/// For `supervisor status`: compares the sidecar with the stored state; never reads a key
/// store or checks the signature (that is `receipt verify`), so a matching sidecar is
/// `signed_not_verified`. A sidecar without a 128-hex signature or a key id is `unknown`.
pub fn status(root: &Path) -> Value {
    let (dir, statement) = match current(root) {
        Ok(Ok(current)) => current,
        Ok(Err(_)) => return json!({"state":"unsigned"}),
        Err(_) => return json!({"state":"unknown"}),
    };
    let sidecar = match read_sidecar(&dir) {
        Ok(None) => return json!({"state":"unsigned"}),
        Ok(Some(sidecar)) => sidecar,
        Err(_) => return json!({"state":"unknown"}),
    };
    let signature = sidecar["signature"].as_str().and_then(signing::unhex::<64>);
    match (signature, sidecar["key_id"].as_str()) {
        (Some(_), Some(key_id)) if matches(&sidecar, &statement) => json!({
            "state":"signed_not_verified","key_id":key_id,
            "storage":sidecar["storage"].as_str().unwrap_or("unknown"),
        }),
        (Some(_), Some(_)) => json!({"state":"stale"}),
        _ => json!({"state":"unknown"}),
    }
}
