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

/// When the event's pull request was last updated (a push), and a time after it.
const EVENT_AT: &str = "2026-11-11T10:00:00Z";
const AFTER: &str = "2026-11-11T10:30:00Z";
const RELEASE_BRANCH: &str = "release-please--branches--main--components--coding-supervisor";

fn pr(base: &str, head: &str, author: &str) -> Value {
    json!({"pull_request":{"base":{"ref":base},"head":{"ref":head},"user":{"login":author},
        "labels":[],"updated_at":EVENT_AT},"repository":{"owner":{"login":"the-org"}}})
}

/// A `receipt-exempt` timeline event after the event's update time.
fn labeled(event: &str, actor: &str) -> Value {
    labeled_at(event, actor, AFTER, "receipt-exempt")
}

fn labeled_at(event: &str, actor: &str, at: &str, label: &str) -> Value {
    json!({"event":event,"label":{"name":label},"actor":{"login":actor},"created_at":at})
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
    // `extra` first: the first occurrence of a flag wins, so a test can replace a default.
    cmd.args(["receipt", "required", "--event"])
        .arg(write("event.json", event.to_string()))
        .args(extra)
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
    let out = cmd.output().unwrap();
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
    let branch = RELEASE_BRANCH;
    // A branch that only starts with the release prefix does not count.
    let (req, why) = required(
        &c,
        &pr(
            "main",
            "release-please--branches--main--components--x",
            OWNER,
        ),
        Some(&RELEASE_FILES),
        None,
        &[],
    );
    assert!(req, "{why}");
    let (req, why) = required(
        &c,
        &pr(
            "main",
            "release-please--branches--main--components--x",
            OWNER,
        ),
        Some(&RELEASE_FILES),
        None,
        &[
            "--release-branch",
            "release-please--branches--main--components--x",
        ],
    );
    assert!(!req, "{why}");
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

/// The repository's own release-please config and the ten files release PR #92 changed
/// (`gh pr view 92 --json files`, 2026-11-11): one file short and every release PR would stall.
#[test]
fn this_repositorys_release_pull_request_is_exempt_under_its_own_config() {
    let c = case("release-real");
    let config = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../.github/release-please-config.json"
    );
    let files = [
        ".github/.release-please-manifest.json",
        "CHANGELOG.md",
        "Cargo.lock",
        "Cargo.toml",
        "adapters/claude-code/Cargo.toml",
        "adapters/mcp/Cargo.toml",
        "cli/Cargo.toml",
        "npm/package.json",
        "npm/platforms/linux-x64/package.json",
        "npm/platforms/windows-x64/package.json",
    ];
    let event = pr("main", RELEASE_BRANCH, OWNER);
    let (req, why) = required(
        &c,
        &event,
        Some(&files),
        None,
        &["--release-config", config],
    );
    assert!(!req, "{why}");
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

/// The label counts only when applied after the event's pull request was last updated, so a
/// push after labelling needs the label again; with no update time it never exempts.
#[test]
fn the_exempt_label_must_postdate_the_last_push() {
    let c = case("label-time");
    let event = pr("main", "feat/x", "someone");
    let code = Some(&["src/lib.rs"][..]);
    let before = labeled_at("labeled", OWNER, "2026-11-11T09:59:59Z", "receipt-exempt");
    let (req, why) = required(&c, &event, code, Some(&[before]), &[]);
    assert!(req, "{why}");
    let (req, why) = required(&c, &event, code, Some(&[labeled("labeled", OWNER)]), &[]);
    assert!(!req, "{why}");
    let mut timeless = event.clone();
    timeless["pull_request"]
        .as_object_mut()
        .unwrap()
        .remove("updated_at");
    let (req, why) = required(&c, &timeless, code, Some(&[labeled("labeled", OWNER)]), &[]);
    assert!(req, "{why}");
    // A labelling event with no time of its own does not exempt either.
    let mut untimed = labeled("labeled", OWNER);
    untimed.as_object_mut().unwrap().remove("created_at");
    let (req, why) = required(&c, &event, code, Some(&[untimed]), &[]);
    assert!(req, "{why}");
}

/// Events for other labels neither exempt nor undo the exemption label.
#[test]
fn events_for_other_labels_do_not_count() {
    let c = case("label-other");
    let event = pr("main", "feat/x", "someone");
    let code = Some(&["src/lib.rs"][..]);
    let other = labeled_at("labeled", OWNER, AFTER, "bug");
    let (req, _) = required(&c, &event, code, Some(&[other]), &[]);
    assert!(req);
    let undo_other = labeled_at("unlabeled", OWNER, AFTER, "bug");
    let (req, why) = required(
        &c,
        &event,
        code,
        Some(&[labeled("labeled", OWNER), undo_other]),
        &[],
    );
    assert!(!req, "{why}");
}

/// Changed paths keep their spaces: only a trailing carriage return is removed.
#[test]
fn a_changed_path_with_a_trailing_space_is_not_documentation() {
    let c = case("space");
    let event = pr("main", "docs/x", "someone");
    let (req, why) = required(&c, &event, Some(&["README.md "]), None, &[]);
    assert!(req, "{why}");
    let crlf = c.dir.join("changed-crlf.txt");
    fs::write(&crlf, "README.md\r\ndocs/a.md\r\n").unwrap();
    let (req, why) = required(
        &c,
        &event,
        None,
        None,
        &["--changed", crlf.to_str().unwrap()],
    );
    assert!(!req, "{why}");
}

/// A push to main and a tag (which is also what release.yml's call carries) need nothing; any
/// other event shape needs a receipt.
#[test]
fn a_push_to_main_and_a_tag_need_nothing_and_other_shapes_need_a_receipt() {
    let c = case("other");
    for (event, word) in [
        (
            json!({"ref":"refs/heads/main","before":"a","after":"b"}),
            "push to main",
        ),
        (
            json!({"ref":"refs/tags/v1.2.3","before":"a","after":"b"}),
            "tag",
        ),
    ] {
        let (req, why) = required(&c, &event, Some(&["src/lib.rs"]), None, &[]);
        assert!(!req, "{event}: {why}");
        assert!(why.contains(word), "{why}");
    }
    for event in [
        json!({"inputs":{}}),
        json!({"ref":"refs/heads/feature","before":"a","after":"b"}),
        json!({"ref":"refs/heads/main"}),
        json!({"pull_request":"not an object"}),
    ] {
        let (req, why) = required(&c, &event, Some(&["README.md"]), None, &[]);
        assert!(req, "{event}: {why}");
    }
}

/// Every pull request needs a receipt whatever its base: a base changed to main later starts
/// no new run, so the check from the other base would stay green on the head commit.
#[test]
fn a_pull_request_to_another_base_needs_a_receipt_too() {
    let c = case("base");
    let (req, why) = required(
        &c,
        &pr("release/1.x", "feat/x", "someone"),
        Some(&["src/lib.rs"]),
        None,
        &[],
    );
    assert!(req, "{why}");
    let (req, why) = required(
        &c,
        &pr("release/1.x", "docs/x", "someone"),
        Some(&["README.md"]),
        None,
        &[],
    );
    assert!(!req, "{why}");
}

/// A pull request whose base cannot be read still needs one; an unreadable event is an error
/// (exit 1), never a pass.
#[test]
fn unreadable_pull_request_data_needs_a_receipt() {
    let c = case("closed");
    let (req, _) = required(
        &c,
        &json!({"pull_request":{"head":{"ref":"x"}}}),
        Some(&["src/lib.rs"]),
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
