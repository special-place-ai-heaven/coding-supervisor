//! WP7: receipt bundles that travel with the code. `supervisor receipt export` writes
//! `supervisor-receipts/<task id>.json`; `supervisor receipt verify --bundle` checks one without
//! any supervisor state (CI has none).
//!
//! A bundle is a JSON object. Its canonical body is the object without its `signature` key,
//! rendered by `chain::canonical` (object keys sorted, no whitespace) from the parsed value, so
//! line endings or indentation of the committed file do not matter. The signature covers the
//! fixed text statement [`BundleStatement`], which names the body's SHA-256.
//!
//! What is exported, field by field: no receipt output text, no review finding text (a count
//! and a SHA-256), no reviewer model, no absolute `cwd` (relative to the project), and the
//! contract with `approval.by` (an OS user name) replaced by a label and that name's SHA-256.
//! The contract's goal, criterion checks and commands are exported: CI runs those commands and
//! checks that scope. The bundled contract is canonical JSON of the stored one after that one
//! replacement, so `exported_contract_sha256` is the SHA-256 of the exported text; the stored file's own
//! digest (`stored_contract_sha256`) and the gate's chain head are signed but cannot be
//! rechecked without the local state, whose chain also covers receipt `cwd` values and review
//! finding text that the bundle leaves out.

use std::fs;
use std::path::{Path, PathBuf};

use coding_supervisor::{
    CandidateId, CriterionId, ReviewDecision, ReviewProvenance, ReviewRecord, TaskState,
};
use serde_json::{Value, json};
use supervisor_claude_code::adapter::{now, receipts_observed};
use supervisor_claude_code::candidate::GitCandidate;
use supervisor_claude_code::chain::canonical;
use supervisor_claude_code::git::{self, RECEIPTS_DIR};
use supervisor_claude_code::lifecycle;
use supervisor_claude_code::location::{canonical_project, resolve_state_dir_read_only};
use supervisor_claude_code::sha256;
use supervisor_claude_code::state::{self, Stored, load_gate_head, read_contract};
use supervisor_claude_code::transaction::{atomic_write, read_lock};

use crate::signing::{self, BUNDLE_TAG, BundleStatement};
use crate::verdict::{self, Require};
use crate::{Failure, Flags};

/// The `bundle_version` this binary writes and reads.
const BUNDLE_VERSION: u64 = 1;
const KEYS_FILE: &str = "supervisor-keys.json";
/// Replaces `approval.by` in the exported contract, followed by ` sha256:<hex of the name>`.
const REDACTED_USER: &str = "redacted-os-user";

/// Printed with exit 3 when export writes nothing.
fn refused(reason: &str) -> Failure {
    println!("{}", json!({"export":"refused","reason":reason}));
    Failure::Unmet
}

pub fn export(flags: &Flags) -> Result<(), Failure> {
    let project = PathBuf::from(canonical_project(&flags.project()?));
    let env = |key: &str| std::env::var(key).ok();
    let (root, _) = resolve_state_dir_read_only(&project, &env)
        .map_err(|e| Failure::Error(format!("receipt export: project state unusable: {e}")))?;
    // As `sign`: the key store is read before the lock, so a slow store never makes a hook wait.
    let key = crate::receipt::load_key(&env, &project);
    let _lock = read_lock(&root)
        .map_err(|e| Failure::Error(format!("receipt export: project state not readable: {e}")))?;
    let (bundle, task_id) = build(&project, &root)?;
    let (store, key) = key.map_err(|reason| refused(&reason))?;
    let body_sha256 = sha256::hex(canonical(&bundle).as_bytes());
    let statement = statement_of(&bundle, &body_sha256).map_err(refused)?;
    let bytes = statement.bytes().map_err(|reason| refused(&reason))?;
    let public = key.verifying_key();
    let key_id = signing::key_id(&public);
    let mut bundle = bundle;
    bundle["signature"] = json!({
        "statement":BUNDLE_TAG,"body_sha256":body_sha256,"key_id":key_id,
        "storage":store.label(),"signature":signing::sign(&key, &bytes),
    });
    let relative = format!("{RECEIPTS_DIR}/{task_id}.json");
    let text = serde_json::to_string_pretty(&bundle)
        .map_err(|_| Failure::Error("receipt export: the bundle could not be rendered".into()))?;
    let path = project.join(&relative);
    if atomic_write(&path, format!("{text}\n").as_bytes()).is_err() {
        // atomic_write's temporary file; left behind it would fail CI's folder check.
        let _ = fs::remove_file(path.with_extension("supervisor-tmp"));
        return Err(refused("the bundle could not be written into the project"));
    }
    let public = signing::hex(public.as_bytes());
    println!(
        "{}",
        json!({"export":"written","bundle":relative,"task_id":task_id,"key_id":key_id,
            "storage":store.label(),"tree":bundle["tree"],
            "keys_entry":{"key_id":key_id,"public_key":public,"revoked_at":null}})
    );
    Ok(())
}

/// The unsigned bundle for the active task, refused unless its stored state verifies (chain),
/// is `completed`, and `supervisor verdict` passes under `observed` on the same candidate.
fn build(project: &Path, root: &Path) -> Result<(Value, String), Failure> {
    let error = |what: &str| Failure::Error(format!("receipt export: {what}"));
    let (verdict, code, _) = verdict::evaluate(project, root, Require::Observed, None)?;
    if code != 0 {
        let why = verdict["reason"].as_str().unwrap_or("unknown");
        return Err(refused(&format!(
            "supervisor verdict fails ({why}); a bundle needs a pass on hook-observed receipts"
        )));
    }
    let task = lifecycle::active_task(root)
        .map_err(|_| error("the active task pointer is invalid"))?
        .ok_or_else(|| refused("there is no active task"))?;
    if !task
        .task_id
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.')
    {
        return Err(refused("the task id is not usable as a file name"));
    }
    let (contract, stored_digest) =
        read_contract(&task.dir).map_err(|_| error("the active contract could not be read"))?;
    let contract = contract.ok_or_else(|| error("the active task has no contract"))?;
    let (stored, head) = load_gate_head(&task.dir, &stored_digest)
        .map_err(|_| error("the task checkpoint could not be read"))?;
    let (Stored::Valid(gate), Some(chain_head)) = (stored, head) else {
        return Err(error("the task checkpoint is tampered or absent"));
    };
    if gate.checkpoint.state != TaskState::Completed {
        return Err(refused(&format!(
            "the active task is {}, not completed",
            state::state_name(gate.checkpoint.state)
        )));
    }
    let candidate = gate
        .checkpoint
        .candidate
        .clone()
        .ok_or_else(|| refused("the gate recorded no candidate"))?;
    let live = CandidateId {
        base_commit: verdict["head"]["sha"]
            .as_str()
            .unwrap_or_default()
            .to_owned(),
        dirty_digest: verdict["head"]["dirty_digest"]
            .as_str()
            .unwrap_or_default()
            .to_owned(),
    };
    // The completion is bound to its candidate. A plain commit afterwards moves the receipts
    // (same tree) but not the completed checkpoint, so export comes before that commit.
    // shortcut: no export after a commit of the completed work; compare the checkpoint
    // candidate's content tree instead if that flow is needed.
    if live != candidate {
        return Err(refused(
            "the worktree is not the candidate the task completed on; export right after completion, before committing the work",
        ));
    }
    let base = contract
        .base_sha
        .clone()
        .ok_or_else(|| refused("the contract names no base commit (base_sha)"))?;
    if !git::is_commit_id(&base) {
        return Err(refused("the contract's base_sha is not a commit id"));
    }
    let roots = git::root_commits(project).map_err(refused)?;
    let docs = git::docs_list_at(project, &base);
    let scratch = std::env::temp_dir();
    let tree = git::bundle_tree(project, &scratch, None, &docs)
        .map_err(|why| refused(&format!("git could not compute the worktree tree: {why}")))?;
    let base_tree = git::bundle_tree(project, &scratch, Some(&base), &docs).map_err(|why| {
        refused(&format!(
            "git could not read the contract's base commit: {why}"
        ))
    })?;
    let changed = git::changed_between(project, &base_tree, &tree)
        .ok_or_else(|| refused("git could not list the changed paths"))?;
    // The tree must be the candidate's: nothing may have changed while it was computed.
    let again = (GitCandidate {
        cwd: project.to_path_buf(),
    })
    .observe_git()
    .ok()
    .map(|c| c.id);
    if again.as_ref() != Some(&candidate) {
        return Err(refused("the worktree changed during export; export again"));
    }
    let text = fs::read_to_string(task.dir.join("contract.json"))
        .map_err(|_| error("the active contract could not be read"))?;
    if sha256::hex(text.as_bytes()) != stored_digest {
        return Err(error("the active contract changed while it was read"));
    }
    let contract_text =
        exported_contract(&text).ok_or_else(|| error("the active contract could not be read"))?;
    let observed = receipts_observed(&task.dir, &task.task_id, &gate.receipts)
        .map_err(|_| error("the run journal could not be read"))?;
    let project_text = project.to_string_lossy();
    let receipts: Vec<Value> = gate
        .receipts
        .iter()
        .zip(observed)
        // Hook-observed only: a caller-asserted receipt's `output_ref` is the caller's own text.
        .filter(|(r, observed)| {
            r.candidate == candidate && label(verdict::provenance_code(r, *observed)) == "observed"
        })
        .map(|(r, observed)| {
            json!({
                "criterion":r.criterion.0,"command":r.command,
                "cwd":relative_cwd(&r.cwd, &project_text),"exit_code":r.exit_code,
                "claim_supported":r.claim_supported,"candidate":state::candidate_json(&r.candidate),
                "started_at":r.started_at,"finished_at":r.finished_at,
                "provenance":label(verdict::provenance_code(r, observed)),"output_ref":r.output_ref,
            })
        })
        .collect();
    let reviews: Vec<Value> = gate
        .reviews
        .iter()
        .map(|s| &s.review)
        .filter(|r| r.candidate == candidate)
        .map(|r| {
            json!({
                "reviewer":r.reviewer,"decision":state::decision_name(r.decision),"at":r.at,
                "provenance":state::provenance_name(r.provenance),
                "candidate":state::candidate_json(&r.candidate),"finding_count":r.findings.len(),
                "findings_sha256":sha256::hex(canonical(&json!(r.findings)).as_bytes()),
            })
        })
        .collect();
    let bundle = json!({
        "bundle_version":BUNDLE_VERSION,"repository":{"root_commits":roots},
        "task_id":task.task_id,"contract":contract_text,
        "exported_contract_sha256":sha256::hex(contract_text.as_bytes()),
        "stored_contract_sha256":stored_digest,"gate_state":"completed","chain_head":chain_head,
        "candidate":state::candidate_json(&candidate),"tree":tree,"base_sha":base,
        "changed_paths":changed,"receipts":receipts,"reviews":reviews,
        "exported_at":verdict::rfc3339_millis(now()),
    });
    Ok((bundle, task.task_id))
}

/// The stored contract as canonical JSON with `approval.by` replaced by [`REDACTED_USER`] and
/// the name's SHA-256 (unsalted: a guessable name can be found by trying names).
fn exported_contract(text: &str) -> Option<String> {
    let mut value: Value = serde_json::from_str(text).ok()?;
    if let Some(by) = value.pointer_mut("/approval/by")
        && let Some(name) = by.as_str()
    {
        *by = format!("{REDACTED_USER} sha256:{}", sha256::hex(name.as_bytes())).into();
    }
    Some(canonical(&value))
}

/// The provenance label a bundle carries for a verdict provenance code.
fn label(code: &str) -> &'static str {
    match code {
        "hook" | "tc" => "observed",
        "mcp" => "caller-asserted",
        _ => "unverified",
    }
}

/// `cwd` relative to the project root (`.` for the root itself), with `/` separators, or
/// `outside-project`: an absolute path names this machine's user and folders.
fn relative_cwd(cwd: &str, project: &str) -> String {
    let norm = |s: &str| s.replace('\\', "/").trim_end_matches('/').to_owned();
    let (cwd, project) = (norm(cwd), norm(project));
    let same = |a: &str, b: &str| {
        if cfg!(windows) {
            a.eq_ignore_ascii_case(b)
        } else {
            a == b
        }
    };
    if same(&cwd, &project) {
        return ".".to_owned();
    }
    match (cwd.get(..project.len()), cwd.get(project.len()..)) {
        (Some(head), Some(rest)) if same(head, &project) && rest.starts_with('/') => {
            rest[1..].to_owned()
        }
        _ => "outside-project".to_owned(),
    }
}

/// The bundle without its `signature` key: what `body_sha256` covers.
fn body(bundle: &Value) -> Value {
    let mut body = bundle.clone();
    if let Some(fields) = body.as_object_mut() {
        fields.remove("signature");
    }
    body
}

/// The statement a bundle's signature covers, from the bundle's own fields.
fn statement_of(bundle: &Value, body_sha256: &str) -> Result<BundleStatement, &'static str> {
    let text = |key: &str| {
        bundle[key]
            .as_str()
            .map(str::to_owned)
            .ok_or("the bundle lacks a field its statement needs")
    };
    let roots: Option<Vec<&str>> = bundle["repository"]["root_commits"]
        .as_array()
        .map(|a| a.iter().filter_map(Value::as_str).collect());
    let candidate = &bundle["candidate"];
    Ok(BundleStatement {
        repository: roots
            .filter(|r| !r.is_empty())
            .ok_or("the bundle names no repository")?
            .join(","),
        task: text("task_id")?,
        exported_contract_sha256: text("exported_contract_sha256")?,
        gate_state: text("gate_state")?,
        chain_head: text("chain_head")?,
        candidate: match (
            candidate["base_commit"].as_str(),
            candidate["dirty_digest"].as_str(),
        ) {
            (Some(base), Some(digest)) => format!("{base}:{digest}"),
            _ => return Err("the bundle names no candidate"),
        },
        tree: text("tree")?,
        body_sha256: body_sha256.to_owned(),
    })
}

/// A file that parses as a bundle of the version this binary reads.
fn read_bundle(path: &Path) -> Result<Value, &'static str> {
    let text = fs::read_to_string(path).map_err(|_| "the bundle file could not be read")?;
    let bundle: Value = serde_json::from_str(&text).map_err(|_| "the bundle does not parse")?;
    if bundle["bundle_version"].as_u64() != Some(BUNDLE_VERSION) {
        return Err("the bundle version is not one this supervisor reads (1)");
    }
    Ok(bundle)
}

/// Every entry of `supervisor-receipts/` must be a plain file that parses as a bundle: the
/// folder is outside every check, so nothing else may hide there.
fn folder_holds_only_bundles(project: &Path) -> Result<(), String> {
    let dir = project.join(RECEIPTS_DIR);
    let Ok(entries) = fs::read_dir(&dir) else {
        return Ok(());
    };
    for entry in entries {
        let entry = entry.map_err(|_| format!("{RECEIPTS_DIR}/ could not be listed"))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let plain = entry.file_type().is_ok_and(|t| t.is_file());
        if !plain || read_bundle(&entry.path()).is_err() {
            return Err(format!(
                "{RECEIPTS_DIR}/{name} is not a receipt bundle; only bundles may live there"
            ));
        }
    }
    Ok(())
}

pub fn verify(flags: &Flags) -> Result<(), Failure> {
    let project = PathBuf::from(canonical_project(&flags.project()?));
    let bundle = PathBuf::from(flags.value("--bundle").unwrap_or_default());
    let keys = flags
        .value("--keys")
        .map_or_else(|| project.join(KEYS_FILE), PathBuf::from);
    let answer = check(&project, &bundle, &keys, flags.value("--base"))
        .unwrap_or_else(|reason| json!({"bundle":"invalid","reason":reason}));
    println!("{answer}");
    if answer["bundle"] == "valid" {
        Ok(())
    } else {
        Err(Failure::Unmet)
    }
}

/// One receipt as the bundle carries it, for the evidence rule.
struct Bundled {
    criterion: CriterionId,
    command: String,
    exit_code: Option<i64>,
    claim_supported: bool,
    candidate: Option<CandidateId>,
    observed: bool,
}

fn candidate_of(v: &Value) -> Option<CandidateId> {
    Some(CandidateId {
        base_commit: v["base_commit"].as_str()?.to_owned(),
        dirty_digest: v["dirty_digest"].as_str()?.to_owned(),
    })
}

/// Every check of `receipt verify --bundle`, in order; `Err` is the `invalid` reason.
fn check(project: &Path, path: &Path, keys: &Path, base: Option<&str>) -> Result<Value, String> {
    let bundle = read_bundle(path)?;
    folder_holds_only_bundles(project)?;
    let signature = &bundle["signature"];
    let body_sha256 = sha256::hex(canonical(&body(&bundle)).as_bytes());
    if signature["body_sha256"].as_str() != Some(body_sha256.as_str()) {
        return Err("the bundle changed after it was signed".into());
    }
    let key_id = signature["key_id"]
        .as_str()
        .map(str::to_ascii_lowercase)
        .ok_or("the bundle names no signing key")?;
    let public = crate::receipt::listed_key(keys, &key_id)?;
    let statement = statement_of(&bundle, &body_sha256)?.bytes()?;
    signing::verify(
        &public,
        &statement,
        signature["signature"].as_str().unwrap_or_default(),
    )?;
    // A bundle is named after its task, so a copy cannot pose as another task's receipt.
    let task_file = format!("{}.json", bundle["task_id"].as_str().unwrap_or_default());
    if path.file_name().and_then(|n| n.to_str()) != Some(task_file.as_str()) {
        return Err("the bundle's file name is not its task id followed by .json".into());
    }
    if bundle["gate_state"] != "completed" {
        return Err("the bundled task is not completed".into());
    }
    let contract_text = bundle["contract"]
        .as_str()
        .ok_or("the bundle holds no contract")?;
    if bundle["exported_contract_sha256"].as_str()
        != Some(sha256::hex(contract_text.as_bytes()).as_str())
    {
        return Err("the bundled contract does not match its SHA-256".into());
    }
    // The loader's own error text is not shown: it is not written for this output.
    let contract = serde_json::from_str::<Value>(contract_text)
        .ok()
        .and_then(|v| state::contract_from(&v).ok())
        .ok_or("the bundled contract does not load")?;
    if !contract.criteria.iter().any(|c| c.mandatory) {
        return Err("the contract has no mandatory criteria (no_mandatory_criteria)".into());
    }
    let candidate = candidate_of(&bundle["candidate"]).ok_or("the bundle names no candidate")?;
    let receipts: Vec<Bundled> = bundle["receipts"]
        .as_array()
        .ok_or("the bundle holds no receipts list")?
        .iter()
        .map(|r| Bundled {
            criterion: CriterionId(r["criterion"].as_str().unwrap_or_default().to_owned()),
            command: r["command"].as_str().unwrap_or_default().to_owned(),
            exit_code: r["exit_code"].as_i64(),
            claim_supported: r["claim_supported"] == true,
            candidate: candidate_of(&r["candidate"]),
            observed: r["provenance"] == "observed",
        })
        .collect();
    let mut acceptance = Vec::new();
    for criterion in contract.criteria.iter().filter(|c| c.mandatory) {
        // As `supervisor verdict`: the latest observed receipt on the candidate decides.
        let latest = receipts.iter().rev().find(|r| {
            r.criterion == criterion.id && r.observed && r.candidate.as_ref() == Some(&candidate)
        });
        match latest {
            Some(r)
                if r.exit_code == Some(0) && r.claim_supported && criterion.accepts(&r.command) =>
            {
                acceptance.push(json!({"criterion":criterion.id.0,"command":r.command}));
            }
            _ => {
                return Err(format!(
                    "mandatory criterion {} has no observed zero-exit receipt citing its command on the bundle's candidate",
                    criterion.id.0
                ));
            }
        }
    }
    let reviews = bundled_reviews(&bundle)?;
    if coding_supervisor::review_met(&contract.review, &reviews, &candidate).is_err() {
        return Err("the contract's review rule is not met by the bundled reviews".into());
    }
    let roots = git::root_commits(project)?;
    if bundle["repository"]["root_commits"] != json!(roots) {
        return Err("the bundle was made in another repository".into());
    }
    let base = base
        .map(str::to_owned)
        .or_else(|| contract.base_sha.clone())
        .ok_or("the contract names no base commit and no --base was given")?;
    // Only a full commit id reaches git, never an option or a revision expression.
    // The contract's own base_sha too, even when --base replaces it.
    let bundled_base = bundle["base_sha"].as_str().unwrap_or_default();
    let contract_base_ok = contract.base_sha.as_deref().is_none_or(git::is_commit_id);
    if !git::is_commit_id(&base) || !git::is_commit_id(bundled_base) || !contract_base_ok {
        return Err("the base is not a commit id (40 or 64 hex characters)".into());
    }
    let docs = git::docs_list_at(project, &base);
    let scratch = std::env::temp_dir();
    let tree = git::bundle_tree(project, &scratch, Some("HEAD"), &docs)
        .map_err(|why| format!("git could not compute the tree of HEAD: {why}"))?;
    if bundle["tree"].as_str() != Some(tree.as_str()) {
        return Err("the bundle's tree is not the project's tree: the receipt is stale".into());
    }
    let base_tree = git::bundle_tree(project, &scratch, Some(&base), &docs)
        .map_err(|why| format!("git could not read the base commit: {why}"))?;
    let changed = git::changed_between(project, &base_tree, &tree)
        .ok_or("git could not list the changed paths")?;
    let outside: Vec<&str> = changed
        .iter()
        .filter(|p| {
            !contract
                .scope
                .iter()
                .any(|s| coding_supervisor::scope_contains(s, p))
        })
        .map(String::as_str)
        .collect();
    if !outside.is_empty() {
        return Err(format!(
            "changed paths outside the contract's scope: {}",
            outside.join(", ")
        ));
    }
    if bundle["changed_paths"] != json!(changed) {
        return Err("the bundle's changed paths differ from what git lists".into());
    }
    Ok(
        json!({"bundle":"valid","task_id":bundle["task_id"],"key_id":key_id,"tree":tree,
        "base":base,"acceptance":acceptance}),
    )
}

fn bundled_reviews(bundle: &Value) -> Result<Vec<ReviewRecord>, String> {
    let Some(list) = bundle["reviews"].as_array() else {
        return Err("the bundle holds no reviews list".into());
    };
    list.iter()
        .map(|r| {
            let decision = match r["decision"].as_str() {
                Some("approved") => ReviewDecision::Approved,
                Some("rejected") => ReviewDecision::Rejected,
                _ => return Err("a bundled review has no decision".to_owned()),
            };
            let provenance = match r["provenance"].as_str() {
                Some("observed") => ReviewProvenance::Observed,
                _ => ReviewProvenance::CallerAsserted,
            };
            let count = r["finding_count"].as_u64().unwrap_or(0);
            Ok(ReviewRecord {
                reviewer: r["reviewer"].as_str().unwrap_or_default().to_owned(),
                model: None,
                decision,
                // The rule reads only the number of findings; their text stays local.
                findings: vec![String::new(); usize::try_from(count).unwrap_or(0).min(100)],
                at: r["at"].as_u64().unwrap_or(0),
                candidate: candidate_of(&r["candidate"])
                    .ok_or("a bundled review names no candidate")?,
                provenance,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The canonical body is stable: key order and whitespace of the file do not change it, the
    /// signature key is left out, and any value change does.
    #[test]
    fn the_canonical_body_ignores_layout_and_the_signature() {
        let a: Value = serde_json::from_str(
            "{\"b\":[1,{\"y\":2,\"x\":\"é\"}],\"a\":null,\"signature\":{\"k\":1}}",
        )
        .unwrap();
        let b: Value = serde_json::from_str(
            "{\r\n  \"signature\": {\"k\": 2},\r\n  \"a\": null,\r\n  \"b\": [ 1, { \"x\": \"é\", \"y\": 2 } ]\r\n}",
        )
        .unwrap();
        assert_eq!(
            canonical(&body(&a)),
            "{\"a\":null,\"b\":[1,{\"x\":\"é\",\"y\":2}]}"
        );
        assert_eq!(canonical(&body(&a)), canonical(&body(&b)));
        let mut c = b.clone();
        c["b"][0] = 2.into();
        assert_ne!(canonical(&body(&a)), canonical(&body(&c)));
    }

    #[test]
    fn cwd_is_exported_relative_to_the_project() {
        assert_eq!(relative_cwd("/p/q", "/p/q"), ".");
        assert_eq!(relative_cwd("/p/q/src", "/p/q/"), "src");
        assert_eq!(relative_cwd("/p/qr", "/p/q"), "outside-project");
        assert_eq!(relative_cwd("/home/user", "/p/q"), "outside-project");
        assert_eq!(relative_cwd(r"C:\p\q\sub", "C:/p/q"), "sub");
    }

    #[test]
    fn only_approval_by_is_replaced_in_the_exported_contract() {
        let text = r#"{"goal":"g","approval":{"by":"someone","at":1,"via":"task-start"}}"#;
        let out = exported_contract(text).unwrap();
        assert!(!out.contains("someone"), "{out}");
        assert!(out.contains(&format!("{REDACTED_USER} sha256:")), "{out}");
        assert!(out.contains("\"goal\":\"g\""), "{out}");
        let none = r#"{"approval":{"by":null,"at":1,"via":"agent-proposal"}}"#;
        assert_eq!(
            exported_contract(none).unwrap(),
            canonical(&serde_json::from_str(none).unwrap())
        );
    }
}
