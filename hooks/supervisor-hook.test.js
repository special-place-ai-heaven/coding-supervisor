"use strict";

// Runs the plugin hook as Claude Code does (node + script, payload on stdin) against a stub
// `supervisor` placed alone on PATH.

const assert = require("node:assert/strict");
const childProcess = require("node:child_process");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const test = require("node:test");

const ROOT = path.join(__dirname, "..");
const HOOK = path.join(__dirname, "supervisor-hook.js");
const EVENTS = ["UserPromptSubmit", "PreToolUse", "PostToolUse", "PostToolUseFailure", "SubagentStop", "Stop"];

const dirs = [];
test.after(() => dirs.forEach((dir) => fs.rmSync(dir, { recursive: true, force: true })));

// A PATH directory holding a `supervisor` that runs `body` as a node script; no stub when body is null.
function bin(body) {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "sup-plugin-"));
  dirs.push(dir);
  if (body !== null) {
    fs.writeFileSync(path.join(dir, "stub.js"), body);
    if (process.platform === "win32") {
      fs.writeFileSync(path.join(dir, "supervisor.cmd"), `@"${process.execPath}" "%~dp0stub.js" %*\r\n`);
    } else {
      fs.writeFileSync(path.join(dir, "supervisor"), `#!${process.execPath}\nrequire("./stub.js");\n`, { mode: 0o755 });
    }
  }
  return dir;
}

// Runs the hook with `searchPath` as PATH, from `cwd`, with extra node `flags`.
function run(searchPath, input, { cwd = ROOT, flags = [] } = {}) {
  return new Promise((resolve) => {
    const child = childProcess.execFile(
      process.execPath,
      [...flags, HOOK],
      { cwd, env: { ...process.env, PATH: searchPath }, encoding: "buffer" },
      (_err, stdout, stderr) => resolve({ code: child.exitCode, stdout, stderr: stderr.toString() }),
    );
    child.stdin.end(input);
  });
}

const payload = (event, fields) => JSON.stringify({ hook_event_name: event, cwd: ROOT, ...fields });

// Each failure mode: PreToolUse, Stop and SubagentStop are refused with exit 2 and a reason on
// stderr; UserPromptSubmit and the PostToolUse events go ahead with a visible systemMessage. A
// Stop or SubagentStop with stop_hook_active goes ahead with a NOT certified systemMessage.
async function assertFailure(dir, reason, options) {
  const [results, again] = await Promise.all([
    Promise.all(EVENTS.map((event) => run(dir, payload(event), options))),
    Promise.all(["Stop", "SubagentStop"].map((event) => run(dir, payload(event, { stop_hook_active: true }), options))),
  ]);
  ["Stop", "SubagentStop"].forEach((event, i) => {
    const { code, stdout, stderr } = again[i];
    assert.equal(code, 0, `${event} with stop_hook_active: ${stderr}`);
    const { systemMessage } = JSON.parse(stdout.toString());
    assert.match(systemMessage, new RegExp(`^coding-supervisor: WARNING, this ${event} went ahead NOT certified: ${reason}`));
  });
  EVENTS.forEach((event, i) => {
    const { code, stdout, stderr } = results[i];
    if (["PreToolUse", "Stop", "SubagentStop"].includes(event)) {
      assert.equal(code, 2, `${event}: ${stderr}`);
      assert.equal(stdout.length, 0, event);
      assert.match(stderr, new RegExp(`^coding-supervisor: ${event} refused, the gate fails closed: ${reason}`));
    } else {
      assert.equal(code, 0, `${event}: ${stderr}`);
      const { systemMessage } = JSON.parse(stdout.toString());
      assert.match(systemMessage, new RegExp(`^coding-supervisor: ${event} went ahead unjudged: ${reason}`));
    }
    // Claude Code treats exit 2 on Stop as non-blocking when stderr says a file is missing.
    assert.doesNotMatch(stderr + stdout.toString(), /no such file|ENOENT|not found|missing/i, event);
  });
}

test("a supervisor that cannot be started", () =>
  assertFailure(bin(null), "the supervisor command could not be started from PATH"));

test("a supervisor that exits non-zero", () =>
  assertFailure(bin("process.exit(101);"), "supervisor exited with 101"));

test("a supervisor killed by a signal", { skip: process.platform === "win32" && "no POSIX signals" }, () =>
  assertFailure(bin('process.kill(process.pid, "SIGKILL");'), "supervisor was killed by SIGKILL"));

test("a supervisor that prints something other than one JSON object", async () => {
  await assertFailure(bin('process.stdout.write("allow\\n");'), "supervisor printed output that is not one JSON object");
  await assertFailure(bin('process.stdout.write("[{}]\\n");'), "supervisor printed output that is not one JSON object");
});

test("a supervisor that does not answer in 20 s", { timeout: 60000 }, () =>
  assertFailure(bin("setTimeout(() => {}, 60000);"), "supervisor gave no answer within 20 s"));

test("a crash inside the hook", () => {
  const crash = path.join(bin(null), "crash.js");
  fs.writeFileSync(crash, 'require("child_process").spawnSync = () => { throw new Error("crash"); };');
  return assertFailure(bin(""), "the plugin hook itself failed", { flags: ["--require", crash] });
});

// A planted supervisor records that it ran; PATH lookups that mean the current directory skip it.
test("a supervisor in the working directory or in a relative PATH entry is never run", async () => {
  const planted = 'require("fs").writeFileSync(require("path").join(__dirname, "ran"), "");';
  const cwd = bin(planted);
  fs.renameSync(bin(planted), path.join(cwd, "relative"));
  const relative = ["", ".", "relative"].join(path.delimiter);
  await assertFailure(relative, "the supervisor command could not be started from PATH", { cwd });

  const answer = bin('process.stdout.write("{}\\n");');
  const { code, stdout } = await run([relative, answer].join(path.delimiter), payload("PreToolUse"), { cwd });
  assert.deepEqual([code, stdout.toString()], [0, "{}\n"]);
  assert.deepEqual([cwd, path.join(cwd, "relative")].filter((dir) => fs.existsSync(path.join(dir, "ran"))), []);
});

// npm's supervisor.cmd shim starts a bare `node`, which cmd.exe would otherwise look up in the cwd first.
test("the Windows shim's node is never taken from the working directory", { skip: process.platform !== "win32" && "cmd.exe only" }, async () => {
  const shim = bin('process.stdout.write("{}\\n");');
  fs.writeFileSync(path.join(shim, "supervisor.cmd"), '@node "%~dp0stub.js" %*\r\n');
  const cwd = bin(null);
  fs.writeFileSync(path.join(cwd, "node.cmd"), '@echo.> "%~dp0ran"\r\n');
  const { code, stdout } = await run([shim, path.dirname(process.execPath)].join(path.delimiter), payload("PreToolUse"), { cwd });
  assert.deepEqual([code, stdout.toString(), fs.existsSync(path.join(cwd, "ran"))], [0, "{}\n", false]);
});

test("empty or unparsable stdin is refused without running supervisor", async () => {
  const dir = bin('require("fs").writeFileSync(require("path").join(__dirname, "ran"), "");');
  for (const input of ["", "not json", "{}"]) {
    const { code, stdout, stderr } = await run(dir, input);
    assert.equal(code, 2, JSON.stringify(input));
    assert.equal(stdout.length, 0);
    assert.match(stderr, /^coding-supervisor: this hook event refused, the gate fails closed: stdin is not a hook payload/);
  }
  assert.equal(fs.existsSync(path.join(dir, "ran")), false);
});

// The hook runs `supervisor hook` and nothing else, so it has no way to change the level.
test("supervisor hook gets the payload unchanged and its answer passes through byte for byte", async () => {
  const echo = bin(`
    const input = require("fs").readFileSync(0, "utf8");
    process.stdout.write(JSON.stringify({ argv: process.argv.slice(2), systemMessage: input }) + "\\n");
  `);
  const input = payload("PreToolUse");
  const { code, stdout } = await run(echo, input);
  assert.equal(code, 0);
  assert.equal(stdout.toString(), `${JSON.stringify({ argv: ["hook"], systemMessage: input })}\n`);

  const silent = await run(bin(""), payload("Stop"));
  assert.deepEqual([silent.code, silent.stdout.length, silent.stderr], [0, 0, ""]);
});

// The plugin adds hooks only: no MCP server, command, agent, skill, executable or setting, so it
// offers the agent no tool or path to change the gate (for example its level).
test("the plugin ships only its hooks, on every event install registers, with no matcher", () => {
  const manifest = JSON.parse(fs.readFileSync(path.join(ROOT, ".claude-plugin", "plugin.json"), "utf8"));
  assert.deepEqual(Object.keys(manifest).sort(), ["author", "description", "license", "name", "repository"]);
  const components = [".mcp.json", ".lsp.json", "commands", "agents", "bin", "output-styles", "workflows", "themes", "monitors", "settings.json", "SKILL.md"];
  assert.deepEqual(components.filter((p) => fs.existsSync(path.join(ROOT, p))), []);
  const skills = fs.readdirSync(path.join(ROOT, "skills"), { withFileTypes: true });
  assert.deepEqual(skills.filter((e) => fs.existsSync(path.join(ROOT, "skills", e.name, "SKILL.md"))), []);

  const { hooks } = JSON.parse(fs.readFileSync(path.join(__dirname, "hooks.json"), "utf8"));
  assert.deepEqual(Object.keys(hooks).sort(), [...EVENTS].sort());
  for (const event of EVENTS) {
    const handler = { type: "command", command: "node", args: ["${CLAUDE_PLUGIN_ROOT}/hooks/supervisor-hook.js"], timeout: 30 };
    if (event === "PreToolUse") handler.onFailure = "block";
    assert.deepEqual(hooks[event], [{ hooks: [handler] }], event);
  }
});
