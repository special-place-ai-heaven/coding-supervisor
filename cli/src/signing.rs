//! WP6: the receipt signing key, the statement it signs, and their identifiers.
//!
//! The key is this machine's: an agent running as the same OS user can read it and can point
//! `SUPERVISOR_SIGNING_KEY_FILE` elsewhere, so a signature says "this machine's supervisor signed
//! this state", not that the state is true.

use std::fs;
use std::io::{ErrorKind, Write};
use std::path::{Component, Path, PathBuf};

use coding_supervisor::CandidateId;
use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};

/// First line of every signed statement: the bytes can never be read as another format.
pub const STATEMENT_TAG: &str = "coding-supervisor receipt statement v1";
/// Selects the file key store at this path on every platform (tests and an explicit choice).
pub const KEY_FILE_VARIABLE: &str = "SUPERVISOR_SIGNING_KEY_FILE";

/// What a signature covers: the task, its contract bytes and its verified chain head.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Statement {
    pub task: String,
    pub contract_sha256: String,
    pub checkpoint_hash: String,
    /// `<base_commit>:<dirty_digest>`, or `none` before the gate observed a candidate.
    pub candidate: String,
}

impl Statement {
    pub fn new(
        task: String,
        contract_sha256: String,
        checkpoint_hash: String,
        candidate: Option<&CandidateId>,
    ) -> Self {
        Self {
            task,
            contract_sha256,
            checkpoint_hash,
            candidate: candidate.map_or_else(
                || "none".to_owned(),
                |c| format!("{}:{}", c.base_commit, c.dirty_digest),
            ),
        }
    }

    /// The signed bytes: the tag, then one `name:value` field per line, each ending in LF. A
    /// value holding a line break is refused, so no field can forge another.
    pub fn bytes(&self) -> Result<Vec<u8>, String> {
        lines(
            STATEMENT_TAG,
            &[
                ("task", &self.task),
                ("contract_sha256", &self.contract_sha256),
                ("checkpoint_hash", &self.checkpoint_hash),
                ("candidate", &self.candidate),
            ],
        )
    }
}

/// WP7: first line of a receipt bundle's signed statement.
pub const BUNDLE_TAG: &str = "coding-supervisor receipt bundle v2";

/// What a bundle signature covers. `body_sha256` is SHA-256 of the bundle's canonical body, so
/// every other bundle field is covered too.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BundleStatement {
    /// The root commits, comma-joined.
    pub repository: String,
    pub task: String,
    pub contract_sha256: String,
    pub gate_state: String,
    pub chain_head: String,
    /// `<base_commit>:<dirty_digest>`.
    pub candidate: String,
    pub tree: String,
    pub body_sha256: String,
}

impl BundleStatement {
    pub fn bytes(&self) -> Result<Vec<u8>, String> {
        lines(
            BUNDLE_TAG,
            &[
                ("repository", &self.repository),
                ("task", &self.task),
                ("contract_sha256", &self.contract_sha256),
                ("gate_state", &self.gate_state),
                ("chain_head", &self.chain_head),
                ("candidate", &self.candidate),
                ("tree", &self.tree),
                ("body_sha256", &self.body_sha256),
            ],
        )
    }
}

fn lines(tag: &str, fields: &[(&str, &String)]) -> Result<Vec<u8>, String> {
    let mut text = format!("{tag}\n");
    for (name, value) in fields {
        if value.contains(['\n', '\r']) {
            return Err(format!("the statement field {name} holds a line break"));
        }
        text.push_str(&format!("{name}:{value}\n"));
    }
    Ok(text.into_bytes())
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Lowercase or uppercase hex of exactly `N` bytes.
pub fn unhex<const N: usize>(text: &str) -> Option<[u8; N]> {
    let text = text.as_bytes();
    // `from_str_radix` alone would also take a leading `+`.
    if text.len() != 2 * N || !text.iter().all(u8::is_ascii_hexdigit) {
        return None;
    }
    let mut out = [0u8; N];
    for (byte, pair) in out.iter_mut().zip(text.chunks(2)) {
        *byte = u8::from_str_radix(std::str::from_utf8(pair).ok()?, 16).ok()?;
    }
    Some(out)
}

/// The first 16 hex characters of SHA-256 of the 32-byte public key.
pub fn key_id(public: &VerifyingKey) -> String {
    supervisor_claude_code::sha256::hex(public.as_bytes())[..16].to_owned()
}

pub fn sign(key: &SigningKey, statement: &[u8]) -> String {
    hex(&key.sign(statement).to_bytes())
}

/// `Err` names why, for a verify reason.
pub fn verify(public: &VerifyingKey, statement: &[u8], signature: &str) -> Result<(), String> {
    let bytes = unhex::<64>(signature).ok_or("the signature is not 128 hex characters")?;
    public
        .verify_strict(statement, &Signature::from_bytes(&bytes))
        .map_err(|_| "the signature does not match the statement".to_owned())
}

/// Where the signing key lives. Errors never carry key bytes or OS error texts.
pub enum KeyStore {
    File(PathBuf),
    #[cfg(windows)]
    CredentialManager,
}

impl KeyStore {
    /// The override variable, else the platform's store. An override that is relative or holds
    /// a `..` segment is refused, and so is any key file path, override or default, that
    /// resolves inside `project`, so the key never lands in the tree.
    pub fn select(env: &dyn Fn(&str) -> Option<String>, project: &Path) -> Result<Self, String> {
        let store = match env(KEY_FILE_VARIABLE).filter(|p| !p.is_empty()) {
            None => platform_store(env)?,
            Some(path) => {
                let path = PathBuf::from(path);
                if !path.is_absolute() || path.components().any(|c| c == Component::ParentDir) {
                    return Err(format!(
                        "{KEY_FILE_VARIABLE} must be an absolute path without `..` segments"
                    ));
                }
                Self::File(path)
            }
        };
        match &store {
            Self::File(path) if resolved(path).starts_with(resolved(project)) => Err(format!(
                "the key file {} is inside the project; the key must live outside it",
                path.display()
            )),
            _ => Ok(store),
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::File(_) => "file-fallback",
            #[cfg(windows)]
            Self::CredentialManager => "windows-credential-manager",
        }
    }

    /// The stored key, created from OS randomness when the store holds none yet.
    pub fn signing_key(&self) -> Result<SigningKey, String> {
        let secret = match self {
            Self::File(path) => file_key(path)?,
            #[cfg(windows)]
            Self::CredentialManager => credential_key(CREDENTIAL_TARGET)?,
        };
        Ok(SigningKey::from_bytes(&secret))
    }
}

/// `path` with its deepest existing ancestor resolved (symlinks, junctions, letter case), the
/// rest appended as written.
fn resolved(path: &Path) -> PathBuf {
    let mut rest = Vec::new();
    let mut base = path;
    loop {
        if let Ok(real) = fs::canonicalize(base) {
            return rest.iter().rev().fold(real, |acc, name| acc.join(name));
        }
        match (base.parent(), base.file_name()) {
            (Some(parent), Some(name)) => {
                rest.push(name);
                base = parent;
            }
            _ => return path.to_path_buf(),
        }
    }
}

#[cfg(windows)]
fn platform_store(_env: &dyn Fn(&str) -> Option<String>) -> Result<KeyStore, String> {
    Ok(KeyStore::CredentialManager)
}

/// `$XDG_CONFIG_HOME/coding-supervisor/signing-key`, else `$HOME/.config/...`; `select`
/// refuses it when it resolves inside the project.
#[cfg(unix)]
fn platform_store(env: &dyn Fn(&str) -> Option<String>) -> Result<KeyStore, String> {
    let absolute = |name: &str| env(name).map(PathBuf::from).filter(|p| p.is_absolute());
    let config = absolute("XDG_CONFIG_HOME")
        .or_else(|| absolute("HOME").map(|home| home.join(".config")))
        .ok_or("neither XDG_CONFIG_HOME nor HOME names an absolute directory for the key")?;
    Ok(KeyStore::File(
        config.join("coding-supervisor").join("signing-key"),
    ))
}

#[cfg(not(any(unix, windows)))]
fn platform_store(_env: &dyn Fn(&str) -> Option<String>) -> Result<KeyStore, String> {
    Err("this platform has no signing key store".to_owned())
}

fn fresh_secret() -> Result<[u8; 32], String> {
    let mut secret = [0u8; 32];
    getrandom::fill(&mut secret).map_err(|_| "the OS random source could not be read")?;
    Ok(secret)
}

/// The key file holds the 32-byte secret as 64 hex characters and a newline. On unix it is
/// created 0600 and refused when group or others may access it; Windows gets no mode check.
fn file_key(path: &Path) -> Result<[u8; 32], String> {
    let shown = path.display();
    if let Some(secret) = read_key_file(path)? {
        return Ok(secret);
    }
    if let Some(parent) = path.parent() {
        create_private_dir(parent)
            .map_err(|_| format!("the key directory for {shown} could not be created"))?;
    }
    let secret = fresh_secret()?;
    // Written whole to a temporary file beside it, then linked into place: the key path never
    // exists empty or half-written, and a hard link never replaces a key another sign put there.
    let temp = path.with_file_name(format!(
        ".{}.{}.tmp",
        path.file_name()
            .map_or("signing-key".into(), |n| n.to_string_lossy()),
        hex(&fresh_secret()?[..8])
    ));
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    let written = options.open(&temp).and_then(|mut file| {
        file.write_all(format!("{}\n", hex(&secret)).as_bytes())?;
        file.sync_all()
    });
    let linked = written.and_then(|()| fs::hard_link(&temp, path));
    let _ = fs::remove_file(&temp);
    match linked {
        Ok(()) => Ok(secret),
        Err(e) if e.kind() == ErrorKind::AlreadyExists => read_key_file(path)?
            .ok_or_else(|| format!("the signing key file {shown} could not be read")),
        Err(_) => Err(format!("the signing key file {shown} could not be created")),
    }
}

/// `Ok(None)` when there is no key yet: no file, or an empty one. Since keys are linked into
/// place whole, an empty file was not written by this tool and holds no key that anything was
/// signed with, so it is removed and replaced. Any other content that is not a key is refused
/// and kept: it may be a key in another form, and replacing it would lose it.
fn read_key_file(path: &Path) -> Result<Option<[u8; 32]>, String> {
    let shown = path.display();
    let file = match fs::File::open(path) {
        Ok(file) => file,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(format!("the signing key file {shown} could not be read")),
    };
    if file.metadata().is_ok_and(|m| m.len() == 0) {
        drop(file);
        fs::remove_file(path)
            .map_err(|_| format!("the empty signing key file {shown} could not be replaced"))?;
        return Ok(None);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = file
            .metadata()
            .map_err(|_| format!("the signing key file {shown} could not be read"))?
            .permissions()
            .mode();
        if mode & 0o077 != 0 {
            return Err(format!(
                "the signing key file {shown} grants access to group or others (mode {:o}); it was not used",
                mode & 0o777
            ));
        }
    }
    let mut text = String::new();
    std::io::Read::read_to_string(&mut &file, &mut text)
        .map_err(|_| format!("the signing key file {shown} could not be read"))?;
    unhex::<32>(text.trim())
        .map(Some)
        .ok_or_else(|| format!("the signing key file {shown} does not hold a key"))
}

#[cfg(unix)]
fn create_private_dir(dir: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)
}

#[cfg(not(unix))]
fn create_private_dir(dir: &Path) -> std::io::Result<()> {
    fs::create_dir_all(dir)
}

/// The one generic credential this tool keeps in the Windows Credential Manager.
#[cfg(windows)]
const CREDENTIAL_TARGET: &str = "coding-supervisor/receipt-signing-key";

/// The secret's raw 32 bytes in a local (non-roaming) generic credential.
#[cfg(windows)]
fn credential_key(target: &str) -> Result<[u8; 32], String> {
    let entry = credential_entry(target)?;
    match entry.get_secret() {
        Ok(bytes) => <[u8; 32]>::try_from(bytes.as_slice())
            .map_err(|_| "the Windows Credential Manager entry does not hold a key".to_owned()),
        // shortcut: two concurrent first signs may each store a key and the later one wins; the
        // earlier sidecar then names a key no longer stored and must be signed again.
        Err(keyring_core::Error::NoEntry) => {
            let secret = fresh_secret()?;
            entry.set_secret(&secret).map_err(|_| {
                "the key could not be stored in the Windows Credential Manager".to_owned()
            })?;
            Ok(secret)
        }
        // Error values may carry the stored bytes, so none is ever shown.
        Err(_) => Err("the Windows Credential Manager entry could not be read".to_owned()),
    }
}

#[cfg(windows)]
fn credential_entry(target: &str) -> Result<keyring_core::Entry, String> {
    use keyring_core::api::CredentialStoreApi;
    let unavailable = |_| "the Windows Credential Manager could not be opened".to_owned();
    let store = windows_native_keyring_store::Store::new().map_err(unavailable)?;
    let modifiers = std::collections::HashMap::from([("target", target), ("persistence", "Local")]);
    store
        .build("coding-supervisor", "receipt-signing-key", Some(&modifiers))
        .map_err(unavailable)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The exact bytes for fixed input: changing the layout or the tag fails this test.
    #[test]
    fn statement_bytes_are_fixed() {
        let candidate = CandidateId {
            base_commit: "abc".to_owned(),
            dirty_digest: "def".to_owned(),
        };
        let statement = Statement::new(
            "task-1".to_owned(),
            "c0".to_owned(),
            "h1".to_owned(),
            Some(&candidate),
        );
        assert_eq!(
            String::from_utf8(statement.bytes().unwrap()).unwrap(),
            "coding-supervisor receipt statement v1\ntask:task-1\ncontract_sha256:c0\ncheckpoint_hash:h1\ncandidate:abc:def\n"
        );
        let none = Statement::new("t".to_owned(), "c".to_owned(), "h".to_owned(), None);
        assert!(
            String::from_utf8(none.bytes().unwrap())
                .unwrap()
                .ends_with("\ncandidate:none\n")
        );
    }

    /// WP7: the bundle statement's exact bytes; a line break in a field is refused.
    #[test]
    fn bundle_statement_bytes_are_fixed() {
        let s = |tree: &str| BundleStatement {
            repository: "r1,r2".to_owned(),
            task: "t".to_owned(),
            contract_sha256: "c".to_owned(),
            gate_state: "completed".to_owned(),
            chain_head: "h".to_owned(),
            candidate: "b:d".to_owned(),
            tree: tree.to_owned(),
            body_sha256: "s".to_owned(),
        };
        assert_eq!(
            String::from_utf8(s("x").bytes().unwrap()).unwrap(),
            "coding-supervisor receipt bundle v2\nrepository:r1,r2\ntask:t\ncontract_sha256:c\ngate_state:completed\nchain_head:h\ncandidate:b:d\ntree:x\nbody_sha256:s\n"
        );
        assert!(s("x\nbody_sha256:forged").bytes().is_err());
    }

    /// A line break in any field is refused, so one field cannot forge the next.
    #[test]
    fn a_field_with_a_line_break_is_refused() {
        for (task, head) in [("t\ncandidate:x", "h"), ("t", "h\r")] {
            let statement = Statement::new(task.to_owned(), "c".to_owned(), head.to_owned(), None);
            assert!(statement.bytes().is_err(), "{task:?} {head:?}");
        }
    }

    /// RFC 8032 test 1's public key; the expected id is SHA-256 of its bytes computed outside
    /// this code (Python hashlib), first 16 hex characters.
    #[test]
    fn key_id_is_the_first_16_hex_of_sha256() {
        let public =
            unhex::<32>("d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a")
                .unwrap();
        let public = VerifyingKey::from_bytes(&public).unwrap();
        assert_eq!(key_id(&public), "21fe31dfa154a261");
    }

    /// RFC 8032 test 1: signing the empty message gives the published signature, and it verifies.
    #[test]
    fn signing_matches_rfc_8032_test_1() {
        let secret =
            unhex::<32>("9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60")
                .unwrap();
        let key = SigningKey::from_bytes(&secret);
        let signature = sign(&key, b"");
        assert_eq!(
            signature,
            "e5564300c360ac729086e2cc806e828a84877f1eb8e5d974d873e065224901555fb8821590a33bacc61e39701cf9b46bd25bf5f0595bbe24655141438e7a100b"
        );
        assert!(verify(&key.verifying_key(), b"", &signature).is_ok());
        assert!(verify(&key.verifying_key(), b"x", &signature).is_err());
    }

    #[test]
    fn unhex_takes_exact_lengths_only() {
        assert_eq!(unhex::<2>("0aFf"), Some([0x0a, 0xff]));
        assert_eq!(unhex::<2>("0a"), None);
        assert_eq!(unhex::<1>("zz"), None);
        assert_eq!(unhex::<1>("é"), None);
        assert_eq!(unhex::<1>("+f"), None);
    }

    /// Run by hand only (`--ignored`): stores, reads and deletes a credential under a test-only
    /// target, never the tool's own entry.
    #[cfg(windows)]
    #[test]
    #[ignore = "touches the Windows Credential Manager"]
    fn credential_manager_round_trip_under_a_test_target() {
        let target = "coding-supervisor/receipt-signing-key-test";
        let first = credential_key(target).unwrap();
        let second = credential_key(target).unwrap();
        assert_eq!(first, second);
        credential_entry(target)
            .unwrap()
            .delete_credential()
            .unwrap();
    }
}
