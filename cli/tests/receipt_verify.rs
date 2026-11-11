//! WP7 (gate W7.2): `supervisor receipt export` and `receipt verify --bundle` with the real
//! binary. Each task reaches `completed` through real hook calls; the key is a file in the
//! temporary directory. Edited bundles are re-signed with that key so each check is reached on
//! its own.

mod support;

use std::fs;

use serde_json::{Value, json};
use support::*;

fn invalid(result: (i32, Value), word: &str) {
    let (code, value) = result;
    assert_eq!((code, &value["bundle"]), (3, &json!("invalid")), "{value}");
    let reason = value["reason"].as_str().unwrap();
    assert!(reason.contains(word), "{word:?} not in {reason:?}");
}

#[test]
fn an_exported_bundle_verifies_and_names_the_acceptance_commands() {
    let f = Fixture::new("valid");
    let (bundle, keys) = f.ready();
    assert!(bundle.starts_with(f.project.join("supervisor-receipts")));
    let (code, value) = f.verify(&bundle, &keys, &[]);
    assert_eq!((code, &value["bundle"]), (0, &json!("valid")), "{value}");
    assert_eq!(
        value["acceptance"],
        json!([{"criterion":"C1","command":"cargo test -p fixture"}])
    );
    let text: Value = serde_json::from_str(&fs::read_to_string(&bundle).unwrap()).unwrap();
    assert_eq!(text["changed_paths"], json!(["src/lib.rs"]));
    assert_eq!(text["receipts"][0]["provenance"], "observed");
    assert_eq!(text["receipts"][0]["cwd"], ".");
    // The default keys file is <project>/supervisor-keys.json (uncommitted here; the tree is
    // HEAD's).
    fs::copy(&keys, f.project.join("supervisor-keys.json")).unwrap();
    let out = f
        .command(BIN)
        .args(["receipt", "verify", "--bundle"])
        .arg(&bundle)
        .arg("--project")
        .arg(&f.project)
        .output()
        .unwrap();
    let (code, value) = parse(&out);
    assert_eq!((code, &value["bundle"]), (0, &json!("valid")), "{value}");
}

/// A documentation file changed beside the code (as almost every pull request does) leaves the
/// bundle valid: the base commit's documentation is dropped from the scratch index even though
/// it differs from both HEAD and the worktree.
#[test]
fn a_bundle_verifies_when_documentation_changed_beside_the_code() {
    let f = Fixture::new("docs-beside");
    f.start(json!({"scope":["src/", "README.md"]}));
    fs::write(f.project.join("src").join("lib.rs"), "// two
").unwrap();
    fs::write(f.project.join("README.md"), "two
").unwrap();
    f.complete("ok");
    let (bundle, _, keys) = f.exported();
    f.commit("work, its documentation and its receipt");
    let (code, value) = f.verify(&bundle, &keys, &[]);
    assert_eq!((code, &value["bundle"]), (0, &json!("valid")), "{value}");
}

#[test]
fn a_changed_signature_is_invalid() {
    let f = Fixture::new("signature");
    let (bundle, keys) = f.ready();
    let mut v: Value = serde_json::from_str(&fs::read_to_string(&bundle).unwrap()).unwrap();
    let sig = v["signature"]["signature"].as_str().unwrap().to_owned();
    let flipped = if sig.starts_with('0') { "1" } else { "0" };
    v["signature"]["signature"] = format!("{flipped}{}", &sig[1..]).into();
    fs::write(&bundle, v.to_string()).unwrap();
    invalid(f.verify(&bundle, &keys, &[]), "signature does not match");
}

#[test]
fn a_changed_bundle_byte_is_invalid() {
    let f = Fixture::new("byte");
    let (bundle, keys) = f.ready();
    let text = fs::read_to_string(&bundle).unwrap();
    let edited = text.replacen("\"exit_code\": 0", "\"exit_code\": 1", 1);
    assert_ne!(text, edited);
    fs::write(&bundle, edited).unwrap();
    invalid(f.verify(&bundle, &keys, &[]), "changed after it was signed");
}

#[test]
fn a_wrong_contract_hash_is_invalid() {
    let f = Fixture::new("contract");
    let (bundle, keys) = f.ready();
    f.edit_bundle(&bundle, |b| {
        b["exported_contract_sha256"] = "0".repeat(64).into();
    });
    invalid(f.verify(&bundle, &keys, &[]), "does not match its SHA-256");
    // The contract text edited instead, the hash left: the same check.
    f.edit_bundle(&bundle, |b| {
        let text = b["contract"].as_str().unwrap().replace("src/", "");
        b["contract"] = text.into();
    });
    invalid(f.verify(&bundle, &keys, &[]), "does not match its SHA-256");
}

/// Export runs verify's scope check on the contract's `base_sha` and refuses, naming the paths, writing
/// nothing. Two ways the local gate lets such a path through: a file already changed when the
/// task started (G48 does not charge it to the task), and one committed during the task (outside
/// the candidate's dirty paths).
#[test]
fn export_refuses_a_changed_path_outside_the_scope() {
    let refused_for = |f: &Fixture, path: &str| {
        let (code, value) = f.export();
        assert_eq!((code, &value["export"]), (3, &json!("refused")), "{value}");
        let reason = value["reason"].as_str().unwrap();
        assert!(
            reason.contains(&format!("outside the contract's scope: {path}")),
            "{value}"
        );
        assert!(!f.project.join("supervisor-receipts").exists());
    };
    let f = Fixture::new("scope-dirty");
    fs::create_dir_all(f.project.join("other")).unwrap();
    fs::write(f.project.join("other").join("x.txt"), "x\n").unwrap();
    f.start(json!({}));
    fs::write(f.project.join("src").join("lib.rs"), "// two\n").unwrap();
    f.complete("ok");
    refused_for(&f, "other/x.txt");
    let f = Fixture::new("scope-committed");
    f.start(json!({}));
    fs::create_dir_all(f.project.join("other")).unwrap();
    fs::write(f.project.join("other").join("y.txt"), "y\n").unwrap();
    f.commit("outside the scope");
    fs::write(f.project.join("src").join("lib.rs"), "// two\n").unwrap();
    f.complete("ok");
    refused_for(&f, "other/y.txt");
}

/// Verify's own scope check: CI's base (the merge base) can be older than the contract's, and a
/// path changed between the two is outside the scope.
#[test]
fn an_out_of_scope_path_in_the_recomputed_manifest_is_invalid() {
    let f = Fixture::new("scope");
    let first = f.head();
    fs::create_dir_all(f.project.join("other")).unwrap();
    fs::write(f.project.join("other").join("x.txt"), "x\n").unwrap();
    f.commit("outside the scope, before the task");
    let (bundle, keys) = f.ready();
    invalid(
        f.verify(&bundle, &keys, &["--base", &first]),
        "outside the contract's scope: other/x.txt",
    );
    // A bundle claiming a manifest git does not list: refused too.
    let f = Fixture::new("manifest");
    let (bundle, keys) = f.ready();
    f.edit_bundle(&bundle, |b| b["changed_paths"] = json!([]));
    invalid(f.verify(&bundle, &keys, &[]), "changed paths differ");
}

#[test]
fn a_stale_tree_is_invalid() {
    let f = Fixture::new("stale");
    let (bundle, keys) = f.ready();
    fs::write(f.project.join("src").join("lib.rs"), "// three\n").unwrap();
    f.commit("more work");
    invalid(f.verify(&bundle, &keys, &[]), "stale");
}

/// `--base` and the contract's `base_sha` reach git only as a 40- or 64-hex commit id.
#[test]
fn a_base_that_is_not_a_commit_id_is_invalid() {
    let f = Fixture::new("base-id");
    let (bundle, keys) = f.ready();
    for base in ["--output=x", "main", "HEAD~1", "abc"] {
        invalid(
            f.verify(&bundle, &keys, &["--base", base]),
            "not a commit id",
        );
    }
    let original = fs::read_to_string(&bundle).unwrap();
    let head = f.head();
    f.edit_contract(&bundle, |text| {
        let v: Value = serde_json::from_str(text).unwrap();
        let base = v["base_sha"].as_str().unwrap().to_owned();
        text.replace(&base, "--output=x")
    });
    invalid(f.verify(&bundle, &keys, &[]), "not a commit id");
    // The contract's base_sha is checked even when --base replaces it.
    invalid(
        f.verify(&bundle, &keys, &["--base", &head]),
        "not a commit id",
    );
    fs::write(&bundle, &original).unwrap();
    f.edit_bundle(&bundle, |b| b["base_sha"] = "-x".into());
    invalid(f.verify(&bundle, &keys, &[]), "not a commit id");
    // Positive control: a real commit id as --base is read by git (here HEAD, so the bundle's
    // changed paths differ).
    fs::write(&bundle, &original).unwrap();
    invalid(
        f.verify(&bundle, &keys, &["--base", &head]),
        "changed paths differ",
    );
}

/// A contract with no mandatory criteria proves nothing: export and verify both refuse it.
#[test]
fn a_contract_without_mandatory_criteria_is_refused() {
    let f = Fixture::new("no-mandatory");
    let (bundle, keys) = f.ready();
    f.edit_contract(&bundle, |text| {
        text.replace("\"mandatory\":true", "\"mandatory\":false")
    });
    invalid(f.verify(&bundle, &keys, &[]), "no_mandatory_criteria");
    let g = Fixture::new("no-mandatory-export");
    g.start(
        json!({"criteria":[{"id":"C1","check":"candidate_passes_reported_test",
        "mandatory":false,"command":"cargo test -p fixture"}]}),
    );
    g.run_check("cargo test -p fixture", 0, "ok");
    g.stop("Done.");
    let (code, value) = g.export();
    assert_eq!((code, &value["export"]), (3, &json!("refused")), "{value}");
    assert!(
        value["reason"]
            .as_str()
            .unwrap()
            .contains("no_mandatory_criteria"),
        "{value}"
    );
}

#[test]
fn an_unknown_or_revoked_key_is_invalid() {
    let f = Fixture::new("keys");
    let (bundle, keys) = f.ready();
    let listed: Value = serde_json::from_str(&fs::read_to_string(&keys).unwrap()).unwrap();
    let entry = listed["keys"][0].clone();
    let other = f.root.join("other-keys.json");
    fs::write(&other, json!({"keys":[]}).to_string()).unwrap();
    invalid(f.verify(&bundle, &other, &[]), "not listed");
    let mut revoked = entry.clone();
    revoked["revoked_at"] = "2026-11-11T00:00:00Z".into();
    fs::write(&other, json!({"keys":[entry, revoked]}).to_string()).unwrap();
    invalid(f.verify(&bundle, &other, &[]), "revoked");
    invalid(
        f.verify(&bundle, &f.root.join("absent-keys.json"), &[]),
        "keys file is absent",
    );
}

#[test]
fn a_task_state_other_than_completed_is_invalid() {
    let f = Fixture::new("state");
    let (bundle, keys) = f.ready();
    f.edit_bundle(&bundle, |b| b["gate_state"] = "verifying".into());
    invalid(f.verify(&bundle, &keys, &[]), "not completed");
}

#[test]
fn a_mandatory_criterion_without_an_observed_zero_exit_receipt_is_invalid() {
    let f = Fixture::new("criterion");
    let (bundle, keys) = f.ready();
    let original = fs::read_to_string(&bundle).unwrap();
    for (field, value) in [
        ("provenance", json!("caller-asserted")),
        ("exit_code", json!(1)),
        ("command", json!("cargo test -p something-else")),
        ("candidate", json!({"base_commit":"0","dirty_digest":"0"})),
        ("claim_supported", json!(false)),
    ] {
        // Each edit alone, on the original bundle.
        fs::write(&bundle, &original).unwrap();
        f.edit_bundle(&bundle, |b| b["receipts"][0][field] = value.clone());
        invalid(f.verify(&bundle, &keys, &[]), "mandatory criterion C1");
    }
    f.edit_bundle(&bundle, |b| b["receipts"] = json!([]));
    invalid(f.verify(&bundle, &keys, &[]), "mandatory criterion C1");
    // The latest observed receipt decides: a pass followed by a fail does not verify.
    fs::write(&bundle, &original).unwrap();
    f.edit_bundle(&bundle, |b| {
        let mut later = b["receipts"][0].clone();
        later["exit_code"] = json!(1);
        b["receipts"].as_array_mut().unwrap().push(later);
    });
    invalid(f.verify(&bundle, &keys, &[]), "mandatory criterion C1");
}

/// Receipts recorded through MCP `receipt_record` carry caller text in `output_ref`; only
/// hook-observed receipts are exported.
#[test]
fn caller_asserted_receipts_are_not_exported() {
    let f = Fixture::new("asserted");
    let task = f.start(json!({}));
    fs::write(f.project.join("src").join("lib.rs"), "// two\n").unwrap();
    f.mcp("candidate_observe", json!({"task_id":task}));
    f.mcp(
        "receipt_record",
        json!({"task_id":task,"event_id":"asserted-1","criterion":"C1",
            "command":"cargo test -p fixture","cwd":".","exit_code":0,
            "output_ref":"asserted-output-marker-3b7f","claim_supported":true}),
    );
    f.complete("ok");
    // Positive control: the gate stored the caller's text.
    assert!(
        walk(&f.state)
            .iter()
            .filter(|p| p.ends_with("gate.json"))
            .any(|p| fs::read_to_string(p)
                .unwrap()
                .contains("asserted-output-marker-3b7f"))
    );
    let (_, text, _) = f.exported();
    assert!(!text.contains("asserted-output-marker-3b7f"), "{text}");
    let bundle: Value = serde_json::from_str(&text).unwrap();
    let receipts = bundle["receipts"].as_array().unwrap();
    assert!(!receipts.is_empty());
    assert!(
        receipts.iter().all(|r| r["provenance"] == "observed"),
        "{text}"
    );
}

#[test]
fn a_bundle_from_another_repository_is_invalid() {
    let f = Fixture::new("repo");
    let (bundle, keys) = f.ready();
    let other = Fixture::new_with("repo-other", "another first commit\n");
    let root = |x: &Fixture| x.git(&["rev-list", "--max-parents=0", "HEAD"]);
    assert_ne!(root(&f), root(&other));
    invalid(other.verify(&bundle, &keys, &[]), "another repository");
}

/// A shallow clone's oldest commit is not the root: verify refuses rather than compare it.
#[test]
fn a_shallow_clone_is_invalid() {
    let f = Fixture::new("shallow");
    let (bundle, keys) = f.ready();
    let clone = f.root.join("shallow-clone");
    // --no-local: a plain local clone would ignore --depth.
    f.git_in(
        &f.root,
        &[
            "clone",
            "-q",
            "--no-local",
            "--depth",
            "1",
            f.project.to_str().unwrap(),
            clone.to_str().unwrap(),
        ],
    );
    assert_eq!(
        f.git_in(&clone, &["rev-parse", "--is-shallow-repository"]),
        "true"
    );
    let cloned = clone.join(bundle.strip_prefix(&f.project).unwrap());
    invalid(f.verify_in(&clone, &cloned, &keys, &[]), "shallow");
}

#[test]
fn a_file_in_the_receipts_folder_that_is_not_a_bundle_is_invalid() {
    let f = Fixture::new("folder");
    let (bundle, keys) = f.ready();
    fs::write(
        f.project.join("supervisor-receipts").join("notes.txt"),
        "hidden\n",
    )
    .unwrap();
    invalid(
        f.verify(&bundle, &keys, &[]),
        "supervisor-receipts/notes.txt is not a receipt bundle",
    );
    fs::remove_file(f.project.join("supervisor-receipts").join("notes.txt")).unwrap();
    fs::write(
        f.project.join("supervisor-receipts").join("x.json"),
        "{\"bundle_version\":9}",
    )
    .unwrap();
    invalid(
        f.verify(&bundle, &keys, &[]),
        "x.json is not a receipt bundle",
    );
}

/// A bundle under any name but its task id's: refused, so a copy cannot pose as another.
#[test]
fn a_bundle_under_another_file_name_is_invalid() {
    let f = Fixture::new("name");
    let (bundle, keys) = f.ready();
    let copy = f
        .project
        .join("supervisor-receipts")
        .join("someone-else.json");
    fs::copy(&bundle, &copy).unwrap();
    invalid(f.verify(&copy, &keys, &[]), "file name");
}

/// A failed write leaves no temporary file in the folder (CI would refuse it).
#[test]
fn a_failed_export_leaves_nothing_in_the_folder() {
    let f = Fixture::new("tmp");
    let task = f.start(json!({}));
    fs::write(f.project.join("src").join("lib.rs"), "// two\n").unwrap();
    f.complete("ok");
    let folder = f.project.join("supervisor-receipts");
    // The bundle's own path is a folder, so the final rename fails.
    fs::create_dir_all(folder.join(format!("{task}.json"))).unwrap();
    let (code, value) = f.export();
    assert_eq!((code, &value["export"]), (3, &json!("refused")), "{value}");
    // The write itself failed, not an earlier check.
    assert!(
        value["reason"]
            .as_str()
            .unwrap()
            .contains("could not be written"),
        "{value}"
    );
    let left: Vec<String> = fs::read_dir(&folder)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(left, vec![format!("{task}.json")], "{left:?}");
}

#[test]
fn an_unmet_review_rule_is_invalid() {
    let f = Fixture::new("review");
    f.start(json!({"review":{"required":true,"independent":true}}));
    fs::write(f.project.join("src").join("lib.rs"), "// two\n").unwrap();
    f.run_check("cargo test -p fixture", 0, "ok");
    f.review("rev1", "REVIEW: APPROVED");
    let stop = f.stop("Done.");
    assert_ne!(stop["decision"], "block", "{stop}");
    let (bundle, _, keys) = f.exported();
    f.commit("work");
    let (code, value) = f.verify(&bundle, &keys, &[]);
    assert_eq!(code, 0, "{value}");
    f.edit_bundle(&bundle, |b| b["reviews"] = json!([]));
    invalid(f.verify(&bundle, &keys, &[]), "review rule");
    f.edit_bundle(&bundle, |b| {
        b["reviews"] = json!([{"reviewer":"main","decision":"approved","at":1,
            "provenance":"caller-asserted","candidate":b["candidate"].clone(),"finding_count":0}]);
    });
    invalid(f.verify(&bundle, &keys, &[]), "review rule");
    // A bundled rejection beside the observed approval: refused.
    f.edit_bundle(&bundle, |b| {
        b["reviews"] = json!([
            {"reviewer":"rev1","decision":"approved","at":1,"provenance":"observed",
                "candidate":b["candidate"].clone(),"finding_count":0},
            {"reviewer":"rev2","decision":"rejected","at":2,"provenance":"observed",
                "candidate":b["candidate"].clone(),"finding_count":1}]);
    });
    invalid(f.verify(&bundle, &keys, &[]), "review rule");
}

/// Export refuses, writing nothing, while the task is not completed or the verdict fails.
#[test]
fn export_refuses_an_unfinished_task() {
    let f = Fixture::new("unfinished");
    f.start(json!({}));
    let (code, value) = f.export();
    assert_eq!((code, &value["export"]), (3, &json!("refused")), "{value}");
    // A Stop refused for a path outside the scope: the receipt is recorded and the verdict, which
    // does not judge scope, passes; the stored task is not completed.
    fs::write(f.project.join("outside.txt"), "x\n").unwrap();
    f.run_check("cargo test -p fixture", 0, "ok");
    assert_eq!(f.stop("Done.")["decision"], "block");
    let (code, verdict) = parse(&f.run(&["verdict", "--json"]));
    assert_eq!(code, 0, "{verdict}");
    let (code, value) = f.export();
    assert_eq!((code, &value["export"]), (3, &json!("refused")), "{value}");
    assert!(
        value["reason"].as_str().unwrap().contains("not completed"),
        "{value}"
    );
    assert!(!f.project.join("supervisor-receipts").exists());
    // Completed, then edited: the receipt is stale and the verdict fails.
    fs::remove_file(f.project.join("outside.txt")).unwrap();
    f.complete("ok");
    fs::write(f.project.join("src").join("lib.rs"), "// edited after\n").unwrap();
    let (code, value) = f.export();
    assert_eq!((code, &value["export"]), (3, &json!("refused")), "{value}");
    assert!(
        value["reason"].as_str().unwrap().contains("verdict fails"),
        "{value}"
    );
    assert!(!f.project.join("supervisor-receipts").exists());
}

/// Text that is not meant to be exported: review finding text, receipt output, the absolute
/// project path and the OS user name. None of it may reach the bundle.
#[test]
fn free_text_that_is_not_exported_never_reaches_the_bundle() {
    let f = Fixture::new("markers");
    f.start(json!({"review":{"required":true,"independent":false}}));
    fs::write(f.project.join("src").join("lib.rs"), "// two\n").unwrap();
    f.run_check("cargo test -p fixture", 0, "receipt-output-marker-91c2");
    f.review(
        "rev1",
        "REVIEW: APPROVED\nFINDING: finding-text-marker-5d0e",
    );
    let stop = f.stop("Done.");
    assert_ne!(stop["decision"], "block", "{stop}");
    let (_, text, _) = f.exported();
    let project = f.project.to_string_lossy().into_owned();
    for marker in [
        // The fixture's own folder name, whatever the path spelling.
        "sup-wp7-markers",
        "receipt-output-marker-91c2",
        "finding-text-marker-5d0e",
        USER_MARKER,
        project.as_str(),
        &project.replace('\\', "/"),
        &project.replace('\\', "\\\\"),
    ] {
        assert!(
            !text.contains(marker),
            "{marker:?} reached the bundle:\n{text}"
        );
    }
    let bundle: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(bundle["reviews"][0]["finding_count"], 1, "{text}");
    assert!(
        bundle["contract"]
            .as_str()
            .unwrap()
            .contains("redacted-os-user sha256:"),
        "{text}"
    );
    // Positive control: the OS user was recorded in the stored contract the bundle redacted.
    let stored: Vec<String> = walk(&f.state)
        .into_iter()
        .filter(|p| p.ends_with("contract.json"))
        .map(|p| fs::read_to_string(p).unwrap())
        .collect();
    assert!(stored.iter().any(|t| t.contains(USER_MARKER)), "{stored:?}");
}

fn walk(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    for entry in fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = entry.path();
        if path.is_dir() {
            out.extend(walk(&path));
        } else {
            out.push(path);
        }
    }
    out
}
