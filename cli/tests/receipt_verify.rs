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
    f.edit_bundle(&bundle, |b| b["contract_sha256"] = "0".repeat(64).into());
    invalid(f.verify(&bundle, &keys, &[]), "does not match its SHA-256");
    // The contract text edited instead, the hash left: the same check.
    f.edit_bundle(&bundle, |b| {
        let text = b["contract"].as_str().unwrap().replace("src/", "");
        b["contract"] = text.into();
    });
    invalid(f.verify(&bundle, &keys, &[]), "does not match its SHA-256");
}

/// An out-of-scope file committed during the task is outside the candidate's dirty paths, so
/// the task completes; the manifest from the base commit still shows it.
#[test]
fn an_out_of_scope_path_in_the_recomputed_manifest_is_invalid() {
    let f = Fixture::new("scope");
    f.start(json!({}));
    fs::create_dir_all(f.project.join("other")).unwrap();
    fs::write(f.project.join("other").join("x.txt"), "x\n").unwrap();
    f.commit("outside the scope");
    fs::write(f.project.join("src").join("lib.rs"), "// two\n").unwrap();
    f.complete("ok");
    let (bundle, _, keys) = f.exported();
    f.commit("work");
    invalid(
        f.verify(&bundle, &keys, &[]),
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
    // An explicit --tree that differs: stale too.
    let head_tree = f.git(&["rev-parse", "HEAD^{tree}"]);
    invalid(f.verify(&bundle, &keys, &["--tree", &head_tree]), "stale");
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
    ] {
        // Each edit alone, on the original bundle.
        fs::write(&bundle, &original).unwrap();
        f.edit_bundle(&bundle, |b| b["receipts"][0][field] = value.clone());
        invalid(f.verify(&bundle, &keys, &[]), "mandatory criterion C1");
    }
    f.edit_bundle(&bundle, |b| b["receipts"] = json!([]));
    invalid(f.verify(&bundle, &keys, &[]), "mandatory criterion C1");
}

#[test]
fn a_bundle_from_another_repository_is_invalid() {
    let f = Fixture::new("repo");
    let (bundle, keys) = f.ready();
    let other = Fixture::new("repo-other");
    invalid(other.verify(&bundle, &keys, &[]), "another repository");
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
