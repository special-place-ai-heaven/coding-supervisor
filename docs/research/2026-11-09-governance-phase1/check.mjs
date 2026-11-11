#!/usr/bin/env node
// Oracles for GATES.md in this folder. Each subcommand prints its marker only after every
// assertion passed and exits 1 otherwise. Needs node and, for the GitHub subcommands, gh.
import { execFileSync } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";

const REPO = "special-place-ai-heaven/coding-supervisor";
const CI_JOBS = [
  "Decide whether to run",
  "Linux: format, lint, test, core dependency check",
  "Windows (hosted): test",
];
const HEAVY = CI_JOBS.slice(1);

function fail(message) {
  console.error("check failed: " + message);
  process.exit(1);
}
function run(cmd, args) {
  try {
    return execFileSync(cmd, args, { encoding: "utf8", stdio: ["ignore", "pipe", "pipe"] });
  } catch (e) {
    fail(cmd + " " + args.join(" ") + " exited " + e.status + ": " + String(e.stderr).trim());
  }
}
function mergedPr(branch) {
  const prs = JSON.parse(run("gh", ["pr", "list", "-R", REPO, "--state", "merged", "--head", branch,
    "--json", "number,baseRefName,mergedAt"]));
  const pr = prs.find((p) => p.baseRefName === "main" && p.mergedAt);
  if (!pr) fail("no pull request from " + branch + " was merged into main");
  return pr;
}
function checks(pr) {
  return JSON.parse(run("gh", ["pr", "checks", String(pr), "-R", REPO, "--json", "name,bucket"]));
}

const [sub, arg, arg2] = process.argv.slice(2);
switch (sub) {
  case "merged": {
    if (!arg) fail("usage: merged <branch>");
    const pr = mergedPr(arg);
    // The merge must be in the checkout the other gates run against.
    run("git", ["fetch", "--quiet", "origin", "main"]);
    const head = run("git", ["rev-parse", "HEAD"]).trim();
    const tip = run("git", ["rev-parse", "origin/main"]).trim();
    if (head !== tip) fail("checkout HEAD " + head + " is not origin/main " + tip + "; pull first");
    console.log("merged into main: " + arg + " #" + pr.number);
    break;
  }
  case "docs-only-ci": {
    // A docs-only PR gets every CI job reported, the light job passing and the heavy ones skipped.
    if (!arg) fail("usage: docs-only-ci <branch>");
    const pr = mergedPr(arg);
    const files = JSON.parse(run("gh", ["pr", "view", String(pr.number), "-R", REPO, "--json", "files"])).files;
    const code = files.map((f) => f.path).filter((p) => !/^docs\/|^(README|CHANGELOG|CLAUDE|HANDOVER)\.md$|^LICENSE$/.test(p));
    if (files.length === 0 || code.length) fail("PR #" + pr.number + " is not docs-only: " + code.join(", "));
    const got = checks(pr.number);
    for (const name of CI_JOBS) {
      const c = got.find((x) => x.name === name);
      if (!c) fail("PR #" + pr.number + " has no check named " + JSON.stringify(name));
      const want = HEAVY.includes(name) ? "skipping" : "pass";
      if (c.bucket !== want) fail(name + " is " + c.bucket + ", expected " + want);
    }
    console.log("docs-only PR #" + pr.number + " reported all CI jobs, heavy ones skipped");
    break;
  }
  case "ci-passed": {
    // The named job ran and passed on the merged PR of <branch>.
    if (!arg || !arg2) fail("usage: ci-passed <branch> <job name>");
    const pr = mergedPr(arg);
    const c = checks(pr.number).find((x) => x.name === arg2);
    if (!c || c.bucket !== "pass") fail("job " + JSON.stringify(arg2) + " on PR #" + pr.number + " is " + (c ? c.bucket : "absent"));
    console.log("job passed on PR #" + pr.number + ": " + arg2);
    break;
  }
  case "receipt-passed": {
    // The job passed on the merged PR of <branch>, the PR adds a bundle under
    // supervisor-receipts/, and it does not carry the receipt-exempt label.
    if (!arg || !arg2) fail("usage: receipt-passed <branch> <job name>");
    const pr = mergedPr(arg);
    const view = JSON.parse(run("gh", ["pr", "view", String(pr.number), "-R", REPO, "--json", "files,labels"]));
    if (view.labels.some((l) => l.name === "receipt-exempt")) fail("PR #" + pr.number + " carries the receipt-exempt label");
    if (!view.files.some((f) => /^supervisor-receipts\/[^/]+\.json$/.test(f.path))) fail("PR #" + pr.number + " adds no receipt bundle");
    const c = checks(pr.number).find((x) => x.name === arg2);
    if (!c || c.bucket !== "pass") fail("job " + JSON.stringify(arg2) + " on PR #" + pr.number + " is " + (c ? c.bucket : "absent"));
    console.log("job passed with a receipt bundle on PR #" + pr.number + ": " + arg2);
    break;
  }
  case "ci-failed": {
    // A pull request run from <branch> had the job fail. Read from the branch's own workflow
    // runs: `gh pr checks` shows the newest result per job name for the head commit, and that
    // can be a later run of another pull request on the same commit.
    if (!arg || !arg2) fail("usage: ci-failed <branch> <job name>");
    const runs = JSON.parse(run("gh", ["api", "repos/" + REPO + "/actions/runs?event=pull_request&per_page=100&branch=" + encodeURIComponent(arg)])).workflow_runs;
    if (!runs.length) fail("no pull request run from " + arg);
    for (const r of runs) {
      const jobs = JSON.parse(run("gh", ["api", "repos/" + REPO + "/actions/runs/" + r.id + "/jobs?filter=all&per_page=100"])).jobs;
      const job = jobs.find((x) => x.name === arg2 && x.conclusion === "failure");
      if (job) {
        console.log("job failed on PR run " + r.id + " from " + arg + ": " + arg2);
        process.exit(0);
      }
    }
    fail("job " + JSON.stringify(arg2) + " did not fail on any pull request run from " + arg);
    break;
  }
  case "core-no-deps": {
    // Same rule as ci.yml: cargo tree prints exactly the crate's own line.
    const lines = run("cargo", ["tree", "-p", "coding-supervisor", "--edges", "normal,build,dev"]).trim().split(/\r?\n/);
    if (lines.length !== 1) fail("coding-supervisor has dependencies:\n" + lines.join("\n"));
    console.log("core crate has no dependencies");
    break;
  }
  case "stale-comment-fixed": {
    // Positive control: this fails on e25a0d3, where the stale wording is present.
    const text = readFileSync("src/lib.rs", "utf8");
    if (!text.includes("pub forbidden_effects")) fail("src/lib.rs no longer declares forbidden_effects; recheck the gate");
    if (text.includes("Not enforced by the gate")) fail("src/lib.rs still says forbidden_effects is not enforced");
    console.log("forbidden_effects comment no longer claims it is unenforced");
    break;
  }
  case "matrix": {
    // RESULTS.md: one table row per spec §9 case 1..20 with setup, action, expected, actual,
    // exit code, evidence. Exit code is an integer, or N/A with a reason in the actual column.
    const file = arg || "docs/research/2026-11-09-governance-phase1/RESULTS.md";
    if (!existsSync(file)) fail(file + " does not exist");
    const rows = readFileSync(file, "utf8").split(/\r?\n/)
      .filter((l) => /^\|\s*\d+\s*\|/.test(l))
      .map((l) => l.split("|").slice(1, -1).map((c) => c.trim()));
    const seen = new Set();
    for (const [n, setup, action, expected, actual, exit, evidence] of rows) {
      const id = Number(n);
      if (id < 1 || id > 20 || seen.has(id)) fail("case " + n + " is out of range or repeated");
      seen.add(id);
      if (![setup, action, expected, actual, evidence].every((c) => c && c !== "pending")) fail("case " + n + " has an empty column");
      if (/^N\/A$/i.test(exit)) {
        if (!/server/i.test(actual)) fail("case " + n + " is N/A without naming the server reason");
        continue;
      }
      if (!/^-?\d+$/.test(exit)) fail("case " + n + " exit code is not an integer: " + exit);
      // Evidence must be committed, so the path is repository-relative.
      const path = evidence.replace(/`/g, "").split(/[\s#]/)[0];
      if (/^[A-Za-z]:|^[\\/]|\.\./.test(path)) fail("case " + n + " evidence path is not repository-relative: " + path);
      if (!existsSync(path)) fail("case " + n + " evidence path does not exist: " + path);
    }
    if (seen.size !== 20) fail("matrix has " + seen.size + " of 20 cases");
    console.log("matrix: 20 cases recorded with exit codes or a server N/A reason");
    break;
  }
  default:
    fail("unknown subcommand " + sub);
}
