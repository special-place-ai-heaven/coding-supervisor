//! WP7 (gate W7.4): a bundle committed into `supervisor-receipts/` leaves the candidate identity
//! unchanged, so it verifies at the commit that contains it, also from a fresh clone with no
//! supervisor state; a real change beside it makes it stale.

mod support;

use std::fs;

use serde_json::json;
use supervisor_claude_code::candidate::GitCandidate;
use supervisor_claude_code::git::worktree_tree;
use support::*;

#[test]
fn a_committed_bundle_verifies_at_its_commit_and_from_a_fresh_clone() {
    let f = Fixture::new("in-tree");
    let (bundle, keys) = f.ready();
    let tracked = f.git(&["ls-files", "supervisor-receipts"]);
    assert!(
        tracked.ends_with(".json"),
        "the bundle is committed: {tracked:?}"
    );
    let (code, value) = f.verify(&bundle, &keys, &[]);
    assert_eq!((code, &value["bundle"]), (0, &json!("valid")), "{value}");
    let clone = f.root.join("clone");
    f.git_in(
        &f.root,
        &[
            "clone",
            "-q",
            f.project.to_str().unwrap(),
            clone.to_str().unwrap(),
        ],
    );
    let cloned = clone.join(bundle.strip_prefix(&f.project).unwrap());
    let (code, value) = f.verify_in(&clone, &cloned, &keys, &[]);
    assert_eq!((code, &value["bundle"]), (0, &json!("valid")), "{value}");
}

#[test]
fn writing_and_committing_a_bundle_leave_the_candidate_identity_unchanged() {
    let f = Fixture::new("identity");
    f.start(json!({}));
    fs::write(f.project.join("src").join("lib.rs"), "// two\n").unwrap();
    f.complete("ok");
    let candidate = || {
        GitCandidate {
            cwd: f.project.clone(),
        }
        .observe_git()
        .ok()
        .unwrap()
    };
    let tree = || worktree_tree(&f.project, &f.root).unwrap();
    let (before, tree_before) = (candidate(), tree());
    let _ = f.exported();
    // Written beside the work: the exact candidate, its changed paths and the tree are as before,
    // so `supervisor verdict` still passes.
    let after = candidate();
    assert_eq!(after.id, before.id);
    assert_eq!(after.changed_paths, vec!["src/lib.rs".to_owned()]);
    assert_eq!(tree(), tree_before);
    let (code, verdict) = parse(&f.run(&["verdict", "--json"]));
    assert_eq!(code, 0, "{verdict}");
    // Committed with the work: the tree, which carries the evidence across a commit, is as before,
    // and the hooks still accept the completed task's stop.
    f.commit("work and its receipt");
    assert_eq!(tree(), tree_before);
    let stop = f.stop("Done.");
    assert_ne!(stop["decision"], "block", "{stop}");
}

#[test]
fn a_real_change_beside_the_bundle_makes_it_stale() {
    let f = Fixture::new("beside");
    let (bundle, keys) = f.ready();
    let tree_before = worktree_tree(&f.project, &f.root).unwrap();
    fs::write(f.project.join("src").join("lib.rs"), "// three\n").unwrap();
    assert_ne!(worktree_tree(&f.project, &f.root).unwrap(), tree_before);
    f.commit("another change");
    let (code, value) = f.verify(&bundle, &keys, &[]);
    assert_eq!(code, 3, "{value}");
    assert!(
        value["reason"].as_str().unwrap().contains("stale"),
        "{value}"
    );
}
