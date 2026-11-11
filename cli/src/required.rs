//! WP7: `supervisor receipt required`: whether a CI event needs a receipt bundle, decided only
//! from data CI passes in (the event payload, the pull request's label timeline from the API,
//! the changed paths, and lists read from the base commit). Branch names opt nothing in or out.
//!
//! Every pull request needs a receipt, whatever its base, unless one exemption holds:
//! (0) CI found its head commit already on the default branch (`--head-on-default-branch`,
//! passed only when `git merge-base --is-ancestor <head> <tip>` exits 0), so it brings nothing;
//! (a) every changed path is on the documentation list; (b) it is release-please's version-only
//! pull request: head branch exactly the release branch, opened by the release author, and every
//! changed path one of the files release-please writes; (c) its last `receipt-exempt`
//! labelling in the timeline (not undone by a later unlabelling) was by the owner or a listed
//! maintainer, after the pull request was last updated. The base does not matter because
//! changing a pull request's base to `main` starts no new run: a pass given for another base
//! would stay on the head commit. A push to `main` and a tag (which is also what the release
//! workflow's call carries) need nothing; any other event shape needs a receipt. Data that
//! cannot be read gives no exemption.

use std::fs;

use serde_json::{Value, json};

use crate::{Failure, Flags};

const EXEMPT_LABEL: &str = "receipt-exempt";
/// The head branch this repository's release-please opens (`--release-branch` for another).
const RELEASE_BRANCH: &str = "release-please--branches--main--components--coding-supervisor";
/// release-please's manifest, at its default place or under `.github/` (this repository's).
const RELEASE_MANIFESTS: [&str; 2] = [
    ".release-please-manifest.json",
    ".github/.release-please-manifest.json",
];

pub fn run(flags: &Flags) -> Result<(), Failure> {
    let read = |flag: &str| flags.value(flag).and_then(|p| fs::read_to_string(p).ok());
    let event_path = flags
        .value("--event")
        .ok_or_else(|| Failure::Usage("receipt required needs --event <file>".to_owned()))?;
    let event: Value = fs::read_to_string(event_path)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .ok_or_else(|| {
            Failure::Error("receipt required: the event file could not be read as JSON".into())
        })?;
    let owner = flags.value("--owner").map(str::to_owned).or_else(|| {
        event["repository"]["owner"]["login"]
            .as_str()
            .map(str::to_owned)
    });
    let mut deciders: Vec<String> = owner.clone().into_iter().collect();
    deciders.extend(
        flags
            .value("--maintainers")
            .unwrap_or_default()
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned),
    );
    let input = Input {
        event: &event,
        changed: read("--changed").map(|t| changed_lines(&t)),
        docs: read("--docs-paths")
            .map(|t| docs_list(&t))
            .unwrap_or_default(),
        release_files: read("--release-config")
            .and_then(|t| serde_json::from_str::<Value>(&t).ok())
            .map(|c| release_files(&c))
            .unwrap_or_default(),
        timeline: read("--labels").map(|t| timeline(&t)),
        deciders,
        release_author: flags.value("--release-author").map(str::to_owned).or(owner),
        release_branch: flags.value("--release-branch").unwrap_or(RELEASE_BRANCH),
        head_on_default: flags.switch("--head-on-default-branch"),
    };
    let (required, reason) = decide(&input);
    println!("{}", json!({"required":required,"reason":reason}));
    Ok(())
}

struct Input<'a> {
    event: &'a Value,
    /// `None` when CI passed no readable list: no path-based exemption then.
    changed: Option<Vec<String>>,
    docs: Vec<String>,
    release_files: Vec<String>,
    /// `None` when CI passed no readable timeline: no label exemption then.
    timeline: Option<Vec<Value>>,
    /// The owner and the maintainers: who may apply the exemption label.
    deciders: Vec<String>,
    release_author: Option<String>,
    release_branch: &'a str,
    /// CI saw the head commit already on the default branch's tip.
    head_on_default: bool,
}

fn decide(input: &Input) -> (bool, String) {
    let event = input.event;
    let Some(pr) = event.get("pull_request").filter(|p| p.is_object()) else {
        // A push carries `before` and `after`; release.yml's call carries its tag push.
        let push = event.get("pull_request").is_none()
            && event["before"].is_string()
            && event["after"].is_string();
        return match event["ref"].as_str() {
            Some(r) if push && r.starts_with("refs/tags/") => {
                (false, "a tag needs no receipt".into())
            }
            Some("refs/heads/main") if push => (false, "a push to main needs no receipt".into()),
            _ => (
                true,
                "an event this rule does not know needs a receipt bundle".into(),
            ),
        };
    };
    if input.head_on_default {
        return (
            false,
            "the head commit is already on the default branch: nothing to receipt".into(),
        );
    }
    let changed = input.changed.as_deref().filter(|c| !c.is_empty());
    if let Some(changed) = changed
        && !input.docs.is_empty()
        && changed
            .iter()
            .all(|p| input.docs.iter().any(|d| glob(d, p)))
    {
        return (
            false,
            "documentation only: every changed path is on the documentation list".into(),
        );
    }
    let head = pr["head"]["ref"].as_str().unwrap_or_default();
    let author = pr["user"]["login"].as_str();
    if head == input.release_branch
        && author.is_some()
        && author == input.release_author.as_deref()
        && let Some(changed) = changed
        && changed.iter().all(|p| input.release_files.contains(p))
    {
        return (
            false,
            "release-please's version-only pull request: only release files changed".into(),
        );
    }
    if let Some(actor) = input
        .timeline
        .as_deref()
        .and_then(|t| exempt_label_actor(t, pr["updated_at"].as_str()?))
        && input.deciders.iter().any(|d| d == &actor)
    {
        return (false, format!("labelled {EXEMPT_LABEL} by {actor}"));
    }
    (true, "a pull request needs a receipt bundle".into())
}

/// Who applied the `receipt-exempt` label that is still on, from the timeline in order: the
/// last labelling of it, unless an unlabelling follows, and only when it was applied after
/// `updated_at`, the pull request's last update in the event (a later push needs a new label).
fn exempt_label_actor(timeline: &[Value], updated_at: &str) -> Option<String> {
    let last = timeline.iter().rev().find(|e| {
        matches!(e["event"].as_str(), Some("labeled" | "unlabeled"))
            && e["label"]["name"] == EXEMPT_LABEL
    })?;
    let at = last["created_at"].as_str()?;
    (last["event"] == "labeled" && utc_time(at)? > utc_time(updated_at)?)
        .then(|| last["actor"]["login"].as_str().map(str::to_owned))
        .flatten()
}

/// GitHub's `YYYY-MM-DDTHH:MM:SSZ`, which then compares in time order as text; any other form
/// is `None`, so it never exempts.
fn utc_time(text: &str) -> Option<&str> {
    let b = text.as_bytes();
    let digits = [0, 1, 2, 3, 5, 6, 8, 9, 11, 12, 14, 15, 17, 18];
    let marks = [
        (4, b'-'),
        (7, b'-'),
        (10, b'T'),
        (13, b':'),
        (16, b':'),
        (19, b'Z'),
    ];
    let shaped = b.len() == 20
        && digits.iter().all(|&i| b[i].is_ascii_digit())
        && marks.iter().all(|&(i, c)| b[i] == c);
    shaped.then_some(text)
}

/// Timeline events: one JSON object per line (`gh api --paginate ... --jq '.[] | @json'`), or
/// one JSON array (pages of arrays are flattened).
fn timeline(text: &str) -> Vec<Value> {
    let flatten = |v: Value| match v {
        Value::Array(items) => items
            .into_iter()
            .flat_map(|i| match i {
                Value::Array(inner) => inner,
                other => vec![other],
            })
            .collect(),
        other => vec![other],
    };
    match serde_json::from_str::<Value>(text) {
        Ok(v) => flatten(v),
        Err(_) => text
            .lines()
            .filter_map(|l| serde_json::from_str::<Value>(l).ok())
            .flat_map(flatten)
            .collect(),
    }
}

/// Changed paths, one per line, kept exactly as written except a trailing carriage return.
fn changed_lines(text: &str) -> Vec<String> {
    text.split('\n')
        .map(|l| l.strip_suffix('\r').unwrap_or(l))
        .filter(|l| !l.is_empty())
        .map(str::to_owned)
        .collect()
}

fn lines(text: &str) -> Vec<String> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(str::to_owned)
        .collect()
}

/// `.github/docs-paths.txt` as the gate reads it: blank lines, `#` comments and negated or
/// absolute entries skipped.
fn docs_list(text: &str) -> Vec<String> {
    lines(text)
        .into_iter()
        .filter(|p| !p.starts_with(['#', '!', '/']))
        .collect()
}

/// The files release-please writes: each package's changelog and `extra-files`, plus its
/// manifest. Version files a release type updates on its own must be listed in `extra-files`
/// to count.
fn release_files(config: &Value) -> Vec<String> {
    let mut files: Vec<String> = RELEASE_MANIFESTS.iter().map(|s| (*s).to_owned()).collect();
    for (dir, package) in config["packages"].as_object().into_iter().flatten() {
        let join = |file: &str| match dir.trim_matches('/') {
            "" | "." => file.to_owned(),
            dir => format!("{dir}/{file}"),
        };
        files.push(join(
            package["changelog-path"].as_str().unwrap_or("CHANGELOG.md"),
        ));
        for extra in package["extra-files"].as_array().into_iter().flatten() {
            if let Some(path) = extra.as_str().or_else(|| extra["path"].as_str()) {
                files.push(join(path));
            }
        }
    }
    files
}

/// A documentation-list entry against a path: an exact path, or `dir/**` for everything under
/// `dir/`.
/// shortcut: any other glob never matches (so it never exempts); add a fuller matcher if the
/// list ever needs one.
fn glob(pattern: &str, path: &str) -> bool {
    match pattern.strip_suffix("/**") {
        Some(dir) if !dir.contains(['*', '?', '[']) => path
            .strip_prefix(dir)
            .is_some_and(|rest| rest.starts_with('/')),
        Some(_) => false,
        None => !pattern.contains(['*', '?', '[']) && pattern == path,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_docs_matcher_takes_exact_paths_and_folder_globs_only() {
        assert!(glob("docs/**", "docs/a/b.md"));
        assert!(!glob("docs/**", "docs"));
        assert!(!glob("docs/**", "docsx/a.md"));
        assert!(glob("README.md", "README.md"));
        assert!(!glob("README.md", "src/README.md"));
        assert!(!glob("*.md", "README.md"));
        assert!(!glob("d*/**", "docs/a.md"));
    }
}
