#!/usr/bin/env node
"use strict";

// The plugin's hook for every event: runs `supervisor hook` from PATH with the payload on stdin
// and passes its stdout through byte for byte. When the gate cannot answer, PreToolUse, Stop,
// SubagentStop and any unrecognised payload are refused with exit 2 and the reason on stderr;
// UserPromptSubmit, PostToolUse and PostToolUseFailure go ahead with a systemMessage. The reason
// never says a file is missing: Claude Code treats exit 2 with such text on Stop as non-blocking.

const childProcess = require("child_process");
const fs = require("fs");

const TIMEOUT_MS = 20000;
const FAIL_OPEN = ["UserPromptSubmit", "PostToolUse", "PostToolUseFailure"];

function failure(event, why) {
  if (FAIL_OPEN.includes(event)) {
    const systemMessage = `coding-supervisor: ${event} went ahead unjudged: ${why}.`;
    return { code: 0, stdout: `${JSON.stringify({ systemMessage })}\n`, stderr: "" };
  }
  const what = event || "this hook event";
  return { code: 2, stdout: "", stderr: `coding-supervisor: ${what} refused, the gate fails closed: ${why}.\n` };
}

function eventName(input) {
  try {
    const name = JSON.parse(input).hook_event_name;
    return typeof name === "string" && name ? name : null;
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

function hook(input) {
  const event = eventName(input.toString("utf8"));
  if (!event) return failure(null, "stdin is not a hook payload with a hook_event_name");
  const run = childProcess.spawnSync("supervisor", ["hook"], {
    input,
    timeout: TIMEOUT_MS,
    // npm installs `supervisor` as a .cmd shim on Windows, which only a shell can start.
    shell: process.platform === "win32",
    windowsHide: true,
  });
  if (run.error && run.error.code === "ETIMEDOUT") return failure(event, `supervisor gave no answer within ${TIMEOUT_MS / 1000} s`);
  if (run.error) return failure(event, "the supervisor command could not be started from PATH; install the gate with npm install -g coding-supervisor");
  if (run.signal) return failure(event, `supervisor was killed by ${run.signal}`);
  if (run.status !== 0) return failure(event, `supervisor exited with ${run.status}`);
  const text = run.stdout.toString("utf8");
  if (text.trim() && !isObject(text)) return failure(event, "supervisor printed output that is not one JSON object");
  return { code: 0, stdout: run.stdout, stderr: "" };
}

let result;
try {
  result = hook(fs.readFileSync(0));
} catch (_err) {
  result = failure(null, "the plugin hook itself failed");
}
process.stdout.write(result.stdout);
process.stderr.write(result.stderr);
process.exitCode = result.code;
