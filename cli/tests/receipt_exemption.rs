//! WP7 (gate W7.6): `supervisor receipt required` decides from the event, the label timeline,
//! the changed paths and the base commit's lists, never from a branch name. Real binary.

use std::fs;
use std::path::PathBuf;
use std::process::Command;

use serde_json::{Value, json};

const BIN: &str = env!("CARGO_BIN_EXE_supervisor");
const BANNED: [&str; 3] = ["missing", "not found", "no such file"];
const OWNER: &str = "the-owner";
const RELEASE_FILES: [&str; 3] = [
    ".github/.release-please-manifest.json",
    "CHANGELOG.md",
    "Cargo.toml",
];

struct Case {
    dir: PathBuf,
}
impl Drop for Case {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

fn case(name: &str) -> Case {
    let dir = std::env::temp_dir().join(format!("sup-wp7-required-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    Case { dir }
}

fn pr(base: &str, head: &str, author: &str) -> Value {
    json!({"pull_request":{"base":{"ref":base},"head":{"ref":head},"user":{"login":author},
        "labels":[]},"repository":{"owner":{"login":"the-org"}}})
}

fn labeled(event: &str, actor: &str) -> Value {
    json!({"event":event,"label":{"name":"receipt-exempt"},"actor":{"login":actor}})
}

/// Runs `receipt required` with the given inputs; `timeline` as JSON lines.
fn required(
    c: &Case,
    event: &Value,
    changed: Option<&[&str]>,
    timeline: Option<&[Value]>,
    extra: &[&str],
) -> (bool, String) {
    let write = |name: &str, text: String| {
        let path = c.dir.join(name);
        fs::write(&path, text).unwrap();
        path
    };
    let mut cmd = Command::new(BIN);
    cmd.args(["receipt", "required", "--event"])
        .arg(write("event.json", event.to_string()))
        .arg("--docs-paths")
        .arg(write("docs-paths.txt", "# docs\r\ndocs/**\r\nREADME.md\r\n".into()))
        .arg("--release-config")
        .arg(write(
            "release-please-config.json",
            json!({"packages":{".":{"changelog-path":"CHANGELOG.md",
                "extra-files":[{"type":"toml","path":"Cargo.toml","jsonpath":"$.package.version"}]}}})
            .to_string(),
        ))
        .args(["--owner", OWNER]);
    if let Some(changed) = changed {
        cmd.arg("--changed")
            .arg(write("changed.txt", changed.join("\n") + "\n"));
    }
    if let Some(timeline) = timeline {
        let text: Vec<String> = timeline.iter().map(Value::to_string).collect();
        cmd.arg("--labels")
            .arg(write("timeline.jsonl", text.join("\n")));
    }
    let out = cmd.args(extra).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout).to_lowercase();
    for word in BANNED {
        assert!(!stdout.contains(word), "banned word {word:?}: {stdout}");
    }
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    (
        v["required"].as_bool().unwrap(),
        v["reason"].as_str().unwrap().to_owned(),
    )
}

#[test]
fn a_code_change_to_main_needs_a_receipt() {
    let c = case("code");
    let (req, why) = required(
        &c,
        &pr("main", "feat/x", "someone"),
        Some(&["src/lib.rs"]),
        Some(&[]),
        &[],
    );
    assert!(req, "{why}");
}

/// (a): documentation only, read from the base commit's list; one code path breaks it.
#[test]
fn a_docs_only_change_is_exempt_and_a_code_path_beside_it_is_not() {
    let c = case("docs");
    let event = pr("main", "docs/x", "someone");
    let (req, why) = required(&c, &event, Some(&["docs/a/b.md", "README.md"]), None, &[]);
    assert!(!req, "{why}");
    assert!(why.contains("documentation"), "{why}");
    let (req, _) = required(&c, &event, Some(&["docs/a.md", "src/lib.rs"]), None, &[]);
    assert!(req);
    // No changed-path list: no path-based exemption.
    let (req, _) = required(&c, &event, None, None, &[]);
    assert!(req);
}

/// (b): branch, author and files must all hold; the branch name alone never exempts.
#[test]
fn release_please_is_exempt_only_with_its_author_and_release_files() {
    let c = case("release");
    let branch = "release-please--branches--main--components--x";
    let (req, why) = required(
        &c,
        &pr("main", branch, OWNER),
        Some(&RELEASE_FILES),
        None,
        &[],
    );
    assert!(!req, "{why}");
    assert!(why.contains("release-please"), "{why}");
    // Same branch name, another author.
    let (req, _) = required(
        &c,
        &pr("main", branch, "intruder"),
        Some(&RELEASE_FILES),
        None,
        &[],
    );
    assert!(req);
    // Same branch and author, a source file beside the release files.
    let (req, _) = required(
        &c,
        &pr("main", branch, OWNER),
        Some(&["CHANGELOG.md", "src/lib.rs"]),
        None,
        &[],
    );
    assert!(req);
    // An explicit release author other than the owner.
    let (req, _) = required(
        &c,
        &pr("main", branch, "release-bot"),
        Some(&RELEASE_FILES),
        None,
        &["--release-author", "release-bot"],
    );
    assert!(!req);
}

#[test]
fn branch_names_alone_never_exempt() {
    let c = case("branch");
    for head in [
        "receipt-exempt",
        "docs/only",
        "release-please--branches--main",
        "dependabot/x",
    ] {
        let (req, why) = required(
            &c,
            &pr("main", head, "someone"),
            Some(&["src/lib.rs"]),
            Some(&[]),
            &[],
        );
        assert!(req, "{head}: {why}");
    }
}

/// (c): the label counts only when the owner (or a listed maintainer) applied it last.
#[test]
fn the_exempt_label_counts_only_from_the_owner_or_a_maintainer() {
    let c = case("label");
    let event = pr("main", "feat/x", "someone");
    let code = Some(&["src/lib.rs"][..]);
    let (req, why) = required(&c, &event, code, Some(&[labeled("labeled", OWNER)]), &[]);
    assert!(!req, "{why}");
    assert!(why.contains(OWNER), "{why}");
    // Applied by someone else.
    let (req, _) = required(
        &c,
        &event,
        code,
        Some(&[labeled("labeled", "someone")]),
        &[],
    );
    assert!(req);
    // A maintainer listed explicitly.
    let (req, _) = required(
        &c,
        &event,
        code,
        Some(&[labeled("labeled", "helper")]),
        &["--maintainers", "a, helper"],
    );
    assert!(!req);
    // Removed again after the owner applied it.
    let (req, _) = required(
        &c,
        &event,
        code,
        Some(&[labeled("labeled", OWNER), labeled("unlabeled", OWNER)]),
        &[],
    );
    assert!(req);
    // Owner applied it, someone else re-applied it last.
    let (req, _) = required(
        &c,
        &event,
        code,
        Some(&[
            labeled("labeled", OWNER),
            labeled("unlabeled", "x"),
            labeled("labeled", "x"),
        ]),
        &[],
    );
    assert!(req);
    // The label on the event with no timeline entry does not exempt.
    let mut tagged = event.clone();
    tagged["pull_request"]["labels"] = json!([{"name":"receipt-exempt"}]);
    let (req, _) = required(&c, &tagged, code, Some(&[]), &[]);
    assert!(req);
    let (req, _) = required(&c, &tagged, code, None, &[]);
    assert!(req);
    // The default owner is the event's repository owner (an organisation never labels).
    let out = Command::new(BIN)
        .args(["receipt", "required", "--event"])
        .arg({
            let p = c.dir.join("org-event.json");
            fs::write(&p, event.to_string()).unwrap();
            p
        })
        .arg("--labels")
        .arg({
            let p = c.dir.join("org-timeline.jsonl");
            fs::write(&p, labeled("labeled", "the-org").to_string()).unwrap();
            p
        })
        .output()
        .unwrap();
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["required"], false, "{v}");
}

#[test]
fn a_push_a_tag_the_release_call_and_another_base_need_nothing() {
    let c = case("other");
    for (event, word) in [
        (
            json!({"ref":"refs/heads/main","before":"a","after":"b"}),
            "push to main",
        ),
        (json!({"ref":"refs/tags/v1.2.3"}), "tag"),
        (
            json!({"workflow":"release.yml","ref":"refs/tags/v1.2.3"}),
            "tag",
        ),
        (json!({"inputs":{}}), "not a pull request"),
    ] {
        let (req, why) = required(&c, &event, Some(&["src/lib.rs"]), None, &[]);
        assert!(!req, "{event}: {why}");
        assert!(why.contains(word), "{why}");
    }
    let (req, why) = required(
        &c,
        &pr("release/1.x", "feat/x", "someone"),
        Some(&["src/lib.rs"]),
        None,
        &[],
    );
    assert!(!req, "{why}");
}

/// A pull request to main whose base cannot be read fails closed; an unreadable event is an
/// error (exit 1), never a pass.
#[test]
fn unreadable_pull_request_data_needs_a_receipt() {
    let c = case("closed");
    let (req, _) = required(
        &c,
        &json!({"pull_request":{"head":{"ref":"x"}}}),
        Some(&["README.md"]),
        None,
        &[],
    );
    assert!(req);
    let bad = c.dir.join("bad.json");
    fs::write(&bad, "{not json").unwrap();
    let out = Command::new(BIN)
        .args(["receipt", "required", "--event"])
        .arg(&bad)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    assert!(out.stdout.is_empty());
}
