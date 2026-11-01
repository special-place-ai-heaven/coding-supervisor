#!/usr/bin/env node
"use strict";

// The plugin's hook for every event: runs `supervisor hook` with the payload on stdin and passes
// its stdout through byte for byte. `supervisor` is looked up in the absolute PATH entries only,
// so a file an agent writes in the project directory is never run in its place. When the gate
// cannot answer, PreToolUse, Stop, SubagentStop and any unrecognised payload are refused with
// exit 2 and the reason on stderr; UserPromptSubmit, PostToolUse and PostToolUseFailure go ahead
// with a systemMessage. A Stop or SubagentStop that Claude Code already continued once
// (stop_hook_active) goes ahead with a NOT certified systemMessage, so a broken gate cannot loop
// the session. The reason never says a file is missing: Claude Code treats exit 2 with such text
// on Stop as non-blocking.

const childProcess = require("child_process");
const fs = require("fs");
const path = require("path");

const TIMEOUT_MS = 20000;
const FAIL_OPEN = ["UserPromptSubmit", "PostToolUse", "PostToolUseFailure"];
const STOPS = ["Stop", "SubagentStop"];
const UNSTARTABLE = "the supervisor command could not be started from PATH; install the gate with npm install -g coding-supervisor";

function message(systemMessage) {
  return { code: 0, stdout: `${JSON.stringify({ systemMessage })}\n`, stderr: "" };
}

function failure(payload, why) {
  const event = payload && payload.hook_event_name;
  if (FAIL_OPEN.includes(event)) return message(`coding-supervisor: ${event} went ahead unjudged: ${why}.`);
  if (STOPS.includes(event) && payload.stop_hook_active === true) {
    return message(`coding-supervisor: WARNING, this ${event} went ahead NOT certified: ${why}. Claude Code is already continuing from a refused stop, so this one is let through to end the loop; the task has no receipt.`);
  }
  const what = event || "this hook event";
  return { code: 2, stdout: "", stderr: `coding-supervisor: ${what} refused, the gate fails closed: ${why}.\n` };
}

function parse(input) {
  try {
    const payload = JSON.parse(input);
    const name = payload && payload.hook_event_name;
    return typeof name === "string" && name ? payload : null;
  } catch (_err) {
    return null;
  }
}

function isObject(text) {
  try {
    const value = JSON.parse(text);
    return value !== null && typeof value === "object" && !Array.isArray(value);
  } catch (_err) {
    return false;
  }
}

function executable(file) {
  try {
    fs.accessSync(file, fs.constants.X_OK);
    return fs.statSync(file).isFile();
  } catch (_err) {
    return false;
  }
}

// Empty and relative PATH entries name the current directory, so they are skipped. On Windows
// each entry is tried with every PATHEXT extension, in order, as cmd.exe would.
function resolve(name) {
  const extensions = process.platform === "win32" ? (process.env.PATHEXT || ".COM;.EXE;.BAT;.CMD").split(";").filter(Boolean) : [""];
  for (const dir of (process.env.PATH || "").split(path.delimiter)) {
    if (!path.isAbsolute(dir)) continue;
    for (const extension of extensions) {
      const file = path.join(dir, name + extension);
      if (executable(file)) return file;
    }
  }
  return null;
}

// Starts `supervisor hook` and collects its stdout. On timeout the whole process tree is killed
// and its pipes are let go, because npm's launcher runs the binary as a child that shares them.
function spawn(file, input) {
  // npm installs `supervisor` as a .cmd shim on Windows, which only cmd.exe can start; Node runs
  // it as `cmd.exe /d /s /c "<command>"`, here with the absolute path quoted.
  const shim = /\.(cmd|bat)$/i.test(file);
  return new Promise((resolve) => {
    const child = childProcess.spawn(shim ? `"${file}" hook` : file, shim ? [] : ["hook"], {
      stdio: ["pipe", "pipe", "ignore"],
      shell: shim,
      windowsHide: true,
      // Its own process group, so a timeout can kill the launcher and the binary together.
      detached: process.platform !== "win32",
      // Keeps cmd.exe, including the shim's own bare `node`, from running programs in the cwd.
      env: { ...process.env, NoDefaultCurrentDirectoryInExePath: "1" },
    });
    const stdout = [];
    const timer = setTimeout(() => {
      killTree(child.pid);
      child.stdout.destroy();
      child.unref();
      resolve({ timedOut: true });
    }, TIMEOUT_MS);
    child.on("error", (error) => {
      clearTimeout(timer);
      resolve({ error });
    });
    child.on("close", (status, signal) => {
      clearTimeout(timer);
      resolve({ status, signal, stdout: Buffer.concat(stdout) });
    });
    child.stdout.on("data", (chunk) => stdout.push(chunk));
    child.stdin.on("error", () => {});
    child.stdin.end(input);
  });
}

function killTree(pid) {
  try {
    if (process.platform === "win32") {
      const taskkill = path.join(process.env.SystemRoot || "C:\\Windows", "System32", "taskkill.exe");
      childProcess.spawnSync(taskkill, ["/pid", String(pid), "/t", "/f"], { stdio: "ignore", windowsHide: true });
    } else {
      process.kill(-pid, "SIGKILL");
    }
  } catch (_err) {
    // The tree has already exited.
  }
}

async function hook(input, payload) {
  const file = resolve("supervisor");
  if (!file) return failure(payload, UNSTARTABLE);
  const run = await spawn(file, input);
  if (run.timedOut) return failure(payload, `supervisor gave no answer within ${TIMEOUT_MS / 1000} s`);
  if (run.error) return failure(payload, UNSTARTABLE);
  if (run.signal) return failure(payload, `supervisor was killed by ${run.signal}`);
  if (run.status !== 0) return failure(payload, `supervisor exited with ${run.status}`);
  const text = run.stdout.toString("utf8");
  if (text.trim() && !isObject(text)) return failure(payload, "supervisor printed output that is not one JSON object");
  return { code: 0, stdout: run.stdout, stderr: "" };
}

async function main() {
  let payload = null;
  try {
    const input = fs.readFileSync(0);
    payload = parse(input.toString("utf8"));
    return payload ? await hook(input, payload) : failure(null, "stdin is not a hook payload with a hook_event_name");
  } catch (_err) {
    return failure(payload, "the plugin hook itself failed");
  }
}

main().then((result) => {
  process.stdout.write(result.stdout);
  process.stderr.write(result.stderr);
  process.exitCode = result.code;
});
