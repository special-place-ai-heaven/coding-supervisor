#!/usr/bin/env node
// Runs the demonstration of one barrier-matrix case and records it for RESULTS.md.
// Usage: node run-case.mjs <case 1..20> <command> [args...] [+++ <command> [args...]]...
// Writes evidence/case-NN/output.txt (combined stdout and stderr, LF, personal paths replaced)
// and evidence/case-NN/run.txt (each command and its exit code), and exits with the first
// nonzero exit code, else 0. `node run-case.mjs selftest` checks the path scrub.
import { spawnSync } from "node:child_process";
import { closeSync, mkdirSync, openSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { homedir, userInfo } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const worktree = join(here, "..", "..", "..", "..");
const target = "C:\\Target\\coding-supervisor\\main\\target";
// Fixtures: the tests make their throwaway repositories under the temp folder (PLAN row 8).
const fixtures = "C:\\Target\\coding-supervisor\\matrix\\tmp";

function spellings(path) {
  const back = path.replace(/\//g, "\\");
  const forward = back.replace(/\\/g, "/");
  return [back.replace(/\\/g, "\\\\"), back, forward];
}

function scrub(text) {
  const swaps = [[fixtures, "<fixtures>"], [target, "<target>"], [worktree, "<worktree>"], [homedir(), "<home>"]];
  for (const [path, mark] of swaps) {
    for (const s of spellings(path)) {
      text = text.replace(new RegExp(s.replace(/[.*+?^${}()|[\]\\]/g, "\\$&"), "gi"), mark);
    }
  }
  return text.replace(new RegExp(userInfo().username.replace(/[.*+?^${}()|[\]\\]/g, "\\$&"), "gi"), "<user>");
}

function tidy(raw) {
  // Keep the last state of a carriage-return progress line; strip trailing blanks.
  const lines = raw.replace(/\r\n/g, "\n").split("\n").map((l) => l.split("\r").pop().replace(/\s+$/, ""));
  return scrub(lines.join("\n")).replace(/\n+$/, "") + "\n";
}

const [first, ...rest] = process.argv.slice(2);
if (first === "selftest") {
  const home = homedir();
  const got = tidy(`a ${home}\\x\r\nb ${home.replace(/\\/g, "/")}/y \nc 10%\r100%\n\n`);
  if (got !== "a <home>\\x\nb <home>/y\n100%\n") throw new Error("scrub failed: " + JSON.stringify(got));
  console.log("selftest ok");
  process.exit(0);
}
const n = Number(first);
if (!Number.isInteger(n) || n < 1 || n > 20 || rest.length === 0) {
  console.error("usage: node run-case.mjs <case 1..20> <command> [args...] [+++ <command> ...]");
  process.exit(2);
}
const commands = [[]];
for (const arg of rest) arg === "+++" ? commands.push([]) : commands.at(-1).push(arg);

const dir = join(here, "case-" + String(n).padStart(2, "0"));
mkdirSync(dir, { recursive: true });
mkdirSync(fixtures, { recursive: true });
const env = { ...process.env, CARGO_TARGET_DIR: target, TMP: fixtures, TEMP: fixtures, TMPDIR: fixtures };
let output = "";
let run = "";
let exit = 0;
for (const argv of commands) {
  const shown = argv.map((a) => (/\s/.test(a) ? JSON.stringify(a) : a)).join(" ");
  // One file descriptor for both streams keeps their order and has no buffer cap.
  const raw = join(dir, "raw.tmp");
  const fd = openSync(raw, "w");
  const r = spawnSync(argv[0], argv.slice(1), { cwd: worktree, env, stdio: ["ignore", fd, fd] });
  closeSync(fd);
  const code = r.status ?? 1;
  const text = readFileSync(raw, "utf8") + (r.error ? "spawn error: " + r.error.message + "\n" : "");
  rmSync(raw);
  output += "$ " + shown + "\n" + tidy(text) + "[exit " + code + "]\n\n";
  run += "command: " + shown + "\nexit: " + code + "\n";
  if (exit === 0) exit = code;
}
writeFileSync(join(dir, "output.txt"), scrub(output).replace(/\n+$/, "") + "\n");
writeFileSync(join(dir, "run.txt"), scrub(run) + (commands.length > 1 ? "overall exit: " + exit + "\n" : ""));
process.exit(exit);
