//! WP6 (gate W6.2): `supervisor receipt sign|verify` with the real binary. Every test selects
//! the file key store through `SUPERVISOR_SIGNING_KEY_FILE` (or, on unix, a temporary
//! `XDG_CONFIG_HOME`), so no test touches the Windows Credential Manager entry.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use coding_supervisor::{CandidateId, Checkpoint, CriterionId, TaskState, VerificationReceipt};
use serde_json::{Value, json};
use supervisor_claude_code::lifecycle;
use supervisor_claude_code::location::resolve_state_dir_checked;
use supervisor_claude_code::state::{Gate, read_contract, save_gate};

const BIN: &str = env!("CARGO_BIN_EXE_supervisor");
const BANNED: [&str; 3] = ["missing", "not found", "no such file"];

struct Fixture {
    root: PathBuf,
    project: PathBuf,
    state: PathBuf,
    key: PathBuf,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
impl Fixture {
    fn new(name: &str) -> Self {
        let root =
            std::env::temp_dir().join(format!("sup-cli-receipt-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("project")).unwrap();
        Self {
            project: root.join("project"),
            state: root.join("state"),
            key: root.join("keys").join("signing-key"),
            root,
        }
    }
    /// The binary with this fixture's project state, no user configuration, and the file key
    /// store at `self.key`.
    fn command(&self, args: &[&str]) -> Command {
        let mut cmd = Command::new(BIN);
        cmd.args(args)
            .arg("--project")
            .arg(&self.project)
            .env("HOME", &self.root)
            .env("SUPERVISOR_CONFIG_DIR", self.root.join("config"))
            .env("SUPERVISOR_STATE_DIR", &self.state)
            .env("SUPERVISOR_SIGNING_KEY_FILE", &self.key)
            .env("GIT_CEILING_DIRECTORIES", &self.root);
        cmd
    }
    fn run(&self, args: &[&str]) -> Output {
        self.command(args).output().unwrap()
    }
    fn state_root(&self) -> PathBuf {
        let state = self.state.to_string_lossy().into_owned();
        resolve_state_dir_checked(&self.project, &|key| {
            (key == "SUPERVISOR_STATE_DIR").then(|| state.clone())
        })
        .unwrap()
        .0
    }
    /// An active task whose gate holds `receipts` passing receipts for C1 (sealed by the
    /// adapter's own writer, so the chain verifies).
    fn task(&self) -> PathBuf {
        let contract = json!({"id":"receipt-fixture","goal":"exercise receipt signing",
            "criteria":[{"id":"C1","check":"candidate_passes_reported_test","mandatory":true,"command":"fixture-check"}],
            "scope":[],"forbidden_effects":[],"max_repair_rounds":1});
        let root = self.state_root();
        lifecycle::start_as(
            &root,
            &supervisor_claude_code::state::Writer::task_start(None),
            &contract,
            false,
            None,
            None,
        )
        .unwrap();
        let dir = lifecycle::active_task(&root).unwrap().unwrap().dir;
        self.store_receipts(&dir, 1);
        dir
    }
    /// Rewrite the gate through `save_gate` with `count` receipts: a supported state change.
    fn store_receipts(&self, dir: &Path, count: u64) {
        let (_, digest) = read_contract(dir).unwrap();
        let candidate = CandidateId {
            base_commit: "c0ffee".to_owned(),
            dirty_digest: "d1".to_owned(),
        };
        let mut gate = Gate::new(
            "receipt-fixture".to_owned(),
            Checkpoint {
                state: TaskState::Verifying,
                candidate: Some(candidate.clone()),
                open_findings: Vec::new(),
                consumed_repair_rounds: 0,
                next_action: "none".to_owned(),
            },
        );
        gate.receipts = (0..count)
            .map(|i| VerificationReceipt {
                criterion: CriterionId("C1".to_owned()),
                candidate: candidate.clone(),
                command: "fixture-check".to_owned(),
                cwd: self.project.to_string_lossy().into_owned(),
                exit_code: Some(0),
                output_ref: format!("mcp-asserted:{i}"),
                claim_supported: true,
                started_at: 1 + i,
                finished_at: 2 + i,
            })
            .collect();
        save_gate(dir, &gate, &digest).unwrap();
    }
    fn keys_file(&self, entries: Value) -> PathBuf {
        let path = self.root.join("supervisor-keys.json");
        fs::write(&path, json!({ "keys": entries }).to_string()).unwrap();
        path
    }
    /// Sign, assert it signed, and return the printed answer.
    fn sign(&self) -> Value {
        let out = self.run(&["receipt", "sign"]);
        let (code, value) = parse(&out);
        assert_eq!(code, 0, "sign: {value}");
        assert_eq!(value["signature"], "signed", "sign: {value}");
        value
    }
    fn verify(&self, keys: &Path) -> (i32, Value) {
        parse(&self.run(&["receipt", "verify", "--keys", keys.to_str().unwrap()]))
    }
    fn status_signature(&self) -> Value {
        let (code, value) = parse(&self.run(&["status", "--json"]));
        assert_eq!(code, 0, "status: {value}");
        value["task"]["signature"].clone()
    }
}

fn parse(out: &Output) -> (i32, Value) {
    let stderr = String::from_utf8_lossy(&out.stderr);
    let stdout = String::from_utf8_lossy(&out.stdout);
    for word in BANNED {
        assert!(
            !stdout.to_lowercase().contains(word) && !stderr.to_lowercase().contains(word),
            "banned word {word:?}: {stdout} / {stderr}"
        );
    }
    let value = serde_json::from_slice(&out.stdout)
        .unwrap_or_else(|e| panic!("JSON ({e}): {stdout} / {stderr}"));
    (out.status.code().unwrap(), value)
}

/// The keys-file entry `sign` printed, with `revoked_at` replaced.
fn entry(signed: &Value, revoked_at: Value) -> Value {
    json!({"key_id":signed["key_id"],"public_key":signed["public_key"],"revoked_at":revoked_at})
}

fn sidecar(dir: &Path) -> Value {
    serde_json::from_str(&fs::read_to_string(dir.join("signature.json")).unwrap()).unwrap()
}

/// Sign, then verify against the printed key: valid. Fails if sign or verify were removed.
#[test]
fn sign_then_verify_with_the_printed_key_is_valid() {
    let f = Fixture::new("valid");
    let dir = f.task();
    let signed = f.sign();
    assert_eq!(signed["storage"], "file-fallback");
    assert_eq!(signed["keys_entry"], entry(&signed, Value::Null));
    let stored = sidecar(&dir);
    assert_eq!(stored["key_id"], signed["key_id"]);
    assert_eq!(stored["storage"], "file-fallback");
    let keys = f.keys_file(json!([entry(&signed, Value::Null)]));
    let (code, value) = f.verify(&keys);
    assert_eq!((code, &value["signature"]), (0, &json!("valid")), "{value}");
    // The default keys file is <project>/supervisor-keys.json.
    fs::copy(&keys, f.project.join("supervisor-keys.json")).unwrap();
    let (code, value) = parse(&f.run(&["receipt", "verify"]));
    assert_eq!((code, &value["signature"]), (0, &json!("valid")), "{value}");
}

/// One changed byte of the stored signature: invalid. Fails if verify skipped the signature.
#[test]
fn a_changed_signature_byte_is_invalid() {
    let f = Fixture::new("byte");
    let dir = f.task();
    let signed = f.sign();
    let keys = f.keys_file(json!([entry(&signed, Value::Null)]));
    let mut stored = sidecar(&dir);
    let signature = stored["signature"].as_str().unwrap().to_owned();
    let flipped = if signature.starts_with('0') { "1" } else { "0" };
    stored["signature"] = format!("{flipped}{}", &signature[1..]).into();
    fs::write(dir.join("signature.json"), stored.to_string()).unwrap();
    let (code, value) = f.verify(&keys);
    assert_eq!(
        (code, &value["signature"]),
        (3, &json!("invalid")),
        "{value}"
    );
}

/// A new receipt stored after signing: verify says invalid and status says stale. Fails if the
/// statement did not bind the chain head.
#[test]
fn a_state_change_after_signing_is_invalid_and_stale() {
    let f = Fixture::new("stale");
    let dir = f.task();
    let signed = f.sign();
    let keys = f.keys_file(json!([entry(&signed, Value::Null)]));
    assert_eq!(f.status_signature()["state"], "signed");
    f.store_receipts(&dir, 2);
    let (code, value) = f.verify(&keys);
    assert_eq!(
        (code, &value["signature"]),
        (3, &json!("invalid")),
        "{value}"
    );
    assert_eq!(f.status_signature()["state"], "stale");
}

/// A revoked key fails whatever any time says. Fails if `revoked_at` were ignored.
#[test]
fn a_revoked_key_is_invalid() {
    let f = Fixture::new("revoked");
    f.task();
    let signed = f.sign();
    let keys = f.keys_file(json!([entry(&signed, json!("2026-11-10T00:00:00Z"))]));
    let (code, value) = f.verify(&keys);
    assert_eq!(
        (code, &value["signature"]),
        (3, &json!("invalid")),
        "{value}"
    );
    assert!(
        value["reason"].as_str().unwrap().contains("revoked"),
        "{value}"
    );
}

/// Key id not listed, keys file absent, keys file not JSON: each invalid, never valid.
#[test]
fn an_unlisted_key_or_an_unusable_keys_file_is_invalid() {
    let f = Fixture::new("keys");
    f.task();
    let signed = f.sign();
    let other = json!({"key_id":"0123456789abcdef","public_key":"00".repeat(32),"revoked_at":null});
    let unlisted = f.keys_file(json!([other]));
    let absent = f.root.join("absent-keys.json");
    let not_json = f.root.join("not-json.json");
    fs::write(&not_json, "{keys").unwrap();
    for keys in [unlisted, absent, not_json] {
        let (code, value) = f.verify(&keys);
        assert_eq!(
            (code, &value["signature"]),
            (3, &json!("invalid")),
            "{value}"
        );
    }
    // Positive control: the same project verifies with the right entry.
    let (code, _) = f.verify(&f.keys_file(json!([entry(&signed, Value::Null)])));
    assert_eq!(code, 0);
}

/// No sidecar: unsigned, exit 3, never valid. Fails if verify read an absent sidecar as valid.
#[test]
fn no_sidecar_is_unsigned_exit_three() {
    let f = Fixture::new("unsigned");
    f.task();
    let keys = f.keys_file(json!([]));
    let (code, value) = f.verify(&keys);
    assert_eq!(
        (code, &value["signature"]),
        (3, &json!("unsigned")),
        "{value}"
    );
    assert_eq!(f.status_signature()["state"], "unsigned");
}

/// A key store that cannot be written (the key path's parent is a file): unsigned with a
/// reason, exit 0, and no sidecar. Fails if sign faked a signature or failed hard.
#[test]
fn an_unusable_key_store_signs_nothing() {
    let mut f = Fixture::new("unwritable");
    let dir = f.task();
    let blocker = f.root.join("a-file");
    fs::write(&blocker, "not a directory").unwrap();
    f.key = blocker.join("signing-key");
    let (code, value) = parse(&f.run(&["receipt", "sign"]));
    assert_eq!(
        (code, &value["signature"]),
        (0, &json!("unsigned")),
        "{value}"
    );
    assert!(value["reason"].as_str().is_some_and(|r| !r.is_empty()));
    assert!(!dir.join("signature.json").exists());
}

/// The key is created once and reused; another key file is another key.
#[test]
fn signing_reuses_the_key_and_another_file_is_another_key() {
    let mut f = Fixture::new("reuse");
    f.task();
    let first = f.sign();
    let second = f.sign();
    assert_eq!(first["key_id"], second["key_id"]);
    f.key = f.root.join("keys").join("other-key");
    let third = f.sign();
    assert_ne!(first["key_id"], third["key_id"]);
}

/// The private key's hex never reaches stdout, stderr or the sidecar.
#[test]
fn the_private_key_never_leaves_the_key_file() {
    let f = Fixture::new("secret");
    let dir = f.task();
    let out = f.run(&["receipt", "sign"]);
    assert!(out.status.success());
    let secret = fs::read_to_string(&f.key).unwrap().trim().to_owned();
    assert_eq!(secret.len(), 64, "the key file holds 32 bytes as hex");
    let sidecar = fs::read_to_string(dir.join("signature.json")).unwrap();
    let keys = f.keys_file(json!([]));
    let verify = f.run(&["receipt", "verify", "--keys", keys.to_str().unwrap()]);
    let status = f.run(&["status"]);
    for text in [
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        String::from_utf8_lossy(&verify.stdout).into_owned(),
        String::from_utf8_lossy(&verify.stderr).into_owned(),
        String::from_utf8_lossy(&status.stdout).into_owned(),
        sidecar,
    ] {
        assert!(
            !text.to_lowercase().contains(&secret),
            "private key leaked: {text}"
        );
    }
}

/// Status names the signing key and store, then stale, and the human text carries the line.
#[test]
fn status_shows_signed_with_key_and_storage() {
    let f = Fixture::new("status");
    f.task();
    let signed = f.sign();
    let signature = f.status_signature();
    assert_eq!(signature["state"], "signed");
    assert_eq!(signature["key_id"], signed["key_id"]);
    assert_eq!(signature["storage"], "file-fallback");
    let human = String::from_utf8(f.run(&["status"]).stdout).unwrap();
    let line = format!(
        "signature: signed {} (file-fallback)",
        signed["key_id"].as_str().unwrap()
    );
    assert!(human.lines().any(|l| l == line), "{human}");
}

/// A project that never signs: verdict's output is byte-identical with and without the key
/// override variable. A guard against a behaviour change, not a test of the feature itself.
#[test]
fn verdict_is_unchanged_by_the_signing_variable() {
    let f = Fixture::new("verdict");
    f.task();
    let with = f.run(&["verdict", "--json"]);
    let without = f
        .command(&["verdict", "--json"])
        .env_remove("SUPERVISOR_SIGNING_KEY_FILE")
        .output()
        .unwrap();
    assert_eq!(with.status.code(), without.status.code());
    assert_eq!(with.stdout, without.stdout);
    assert!(!with.stdout.is_empty());
    assert!(!f.key.exists(), "verdict must not create a key");
}

#[cfg(unix)]
mod unix {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    /// The default location is the per-user config directory, created 0600 at the first sign.
    #[test]
    fn the_default_key_file_is_created_owner_only() {
        let f = Fixture::new("xdg");
        f.task();
        let config = f.root.join("xdg");
        let out = f
            .command(&["receipt", "sign"])
            .env_remove("SUPERVISOR_SIGNING_KEY_FILE")
            .env("XDG_CONFIG_HOME", &config)
            .output()
            .unwrap();
        let (code, value) = parse(&out);
        assert_eq!(
            (code, &value["signature"]),
            (0, &json!("signed")),
            "{value}"
        );
        let path = config.join("coding-supervisor").join("signing-key");
        let mode = fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    /// A key file readable by group or others is refused: unsigned with a reason.
    #[test]
    fn a_key_file_open_to_others_is_refused() {
        let f = Fixture::new("mode");
        let dir = f.task();
        f.sign();
        fs::set_permissions(&f.key, fs::Permissions::from_mode(0o644)).unwrap();
        fs::remove_file(dir.join("signature.json")).unwrap();
        let (code, value) = parse(&f.run(&["receipt", "sign"]));
        assert_eq!(
            (code, &value["signature"]),
            (0, &json!("unsigned")),
            "{value}"
        );
        assert!(value["reason"].as_str().is_some_and(|r| !r.is_empty()));
        assert!(!dir.join("signature.json").exists());
    }
}
