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
const EVENTS = ["UserPromptSubmit", "PreToolUse", "PostToolUse", "PostToolUseFailure", "SubagentStop", "Stop", "SessionEnd"];
// The hook gives up on supervisor after 20 s and must answer and exit before Claude Code stops
// waiting for it: the smallest timeout in hooks.json, less a 2 s margin. On Windows the tree kill
// (taskkill /t /f) takes 0.6-1.1 s for eight runs at once on an idle machine and up to 3.8 s when
// every core is busy (measured 2026-11-09), so a bound tighter than Claude Code's own fails on a
// loaded runner while the hook still answers in time.
const ANSWER_MS = (() => {
  const { hooks } = JSON.parse(fs.readFileSync(path.join(__dirname, "hooks.json"), "utf8"));
  const seconds = Object.values(hooks).flatMap((matchers) => matchers.flatMap((m) => m.hooks.map((h) => h.timeout)));
  return Math.min(...seconds) * 1000 - 2000;
})();

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

// Runs the hook with `searchPath` as PATH, from `cwd`, with extra node `flags`. With `limit`, a
// hook still running after `limit` ms is killed and reported `late`; `ms` is spawn to exit.
function run(searchPath, input, { cwd = ROOT, flags = [], limit } = {}) {
  return new Promise((resolve) => {
    const start = Date.now();
    let late = false;
    let timer;
    const child = childProcess.execFile(
      process.execPath,
      [...flags, HOOK],
      { cwd, env: { ...process.env, PATH: searchPath }, encoding: "buffer" },
      (_err, stdout, stderr) => {
        clearTimeout(timer);
        resolve({ code: child.exitCode, stdout, stderr: stderr.toString(), ms: Date.now() - start, late });
      },
    );
    if (limit) timer = setTimeout(() => { late = true; child.kill("SIGKILL"); }, limit);
    child.stdin.end(input);
  });
}

// Kills the processes whose pids a stub appended to `file`, so none outlives its test.
function killPids(file) {
  if (!fs.existsSync(file)) return;
  for (const pid of fs.readFileSync(file, "utf8").trim().split("\n").filter(Boolean).map(Number)) {
    try { process.kill(pid, "SIGKILL"); } catch (_err) { /* already gone */ }
  }
}

// A killed process stays a zombie (state Z) until its new parent reaps it, and signal 0 still
// reaches a zombie. How soon that happens is up to the reaper, not the hook, so on Linux a zombie
// counts as exited. Any other state, including a stopped process, is still running.
const alive = (pid) => {
  try { process.kill(pid, 0); } catch (_err) { return false; }
  if (process.platform !== "linux") return true;
  try {
    const stat = fs.readFileSync(`/proc/${pid}/stat`, "utf8");
    return stat.slice(stat.lastIndexOf(")") + 2, stat.lastIndexOf(")") + 3) !== "Z";
  } catch (_err) {
    return false; // Reaped between the two checks.
  }
};

const payload = (event, fields) => JSON.stringify({ hook_event_name: event, cwd: ROOT, ...fields });

// Each failure mode: PreToolUse, Stop and SubagentStop are refused with exit 2 and a reason on
// stderr; UserPromptSubmit and the PostToolUse events go ahead with a visible systemMessage. A
// Stop or SubagentStop with stop_hook_active goes ahead with a NOT certified systemMessage.
async function assertFailure(dir, reason, options) {
  const [results, again] = await Promise.all([
    Promise.all(EVENTS.map((event) => run(dir, payload(event), options))),
    Promise.all(["Stop", "SubagentStop"].map((event) => run(dir, payload(event, { stop_hook_active: true }), options))),
  ]);
  if (options && options.limit) {
    [...results, ...again].forEach(({ ms, late }, i) =>
      assert.ok(!late && ms < options.limit, `run ${i}: the hook did not answer within ${options.limit} ms (took ${ms} ms${late ? ", killed" : ""})`));
  }
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

// Like npm's launcher, the stub runs the program that hangs as a child sharing its stdout; the
// hook still answers in time, and kills both.
test("a supervisor that does not answer in 20 s", { timeout: 60000 }, async () => {
  const dir = bin(`
    const { spawnSync } = require("child_process");
    spawnSync(process.execPath, ["-e", 'require("fs").appendFileSync(process.argv[1], process.pid + "\\\\n"); setTimeout(() => {}, 120000);', require("path").join(__dirname, "pids")], { stdio: "inherit" });
  `);
  try {
    await assertFailure(dir, "supervisor gave no answer within 20 s", { limit: ANSWER_MS });
    const pids = fs.readFileSync(path.join(dir, "pids"), "utf8").trim().split("\n").map(Number);
    assert.equal(pids.length, EVENTS.length + 2);
    assert.deepEqual(pids.filter(alive), []);
  } finally {
    killPids(path.join(dir, "pids"));
  }
});

// As npm's launcher can on Windows, the stub leaves an orphan outside its process tree that keeps
// the stub's stdout open, then never answers. The tree kill cannot reach the orphan, so the hook
// must let go of the pipe itself and still answer in time. The test kills the orphans afterwards.
test("a supervisor whose orphaned child keeps its stdout open does not hold up the answer", { timeout: 60000 }, async () => {
  const options = '{ detached: true, stdio: ["ignore", "inherit", "ignore"], windowsHide: true }';
  const orphan = 'require("fs").appendFileSync(process.argv[1], process.pid + "\\n"); setTimeout(() => {}, 120000);';
  // Started by a parent that exits at once, so the orphan is in neither the stub's process group
  // nor its Windows process tree.
  const parent = `require("child_process").spawn(process.execPath, ["-e", ${JSON.stringify(orphan)}, process.argv[1]], ${options}).unref();`;
  const dir = bin(`
    require("child_process").spawn(process.execPath, ["-e", ${JSON.stringify(parent)}, require("path").join(__dirname, "pids")], ${options}).unref();
    setInterval(() => {}, 1000);
  `);
  try {
    await assertFailure(dir, "supervisor gave no answer within 20 s", { limit: ANSWER_MS });
    // The orphans are still running, so the hook answered with their pipe still open.
    const pids = fs.readFileSync(path.join(dir, "pids"), "utf8").trim().split("\n").map(Number);
    assert.equal(pids.length, EVENTS.length + 2);
    assert.deepEqual(pids.filter((pid) => !alive(pid)), []);
  } finally {
    killPids(path.join(dir, "pids"));
  }
});

test("a crash inside the hook", () => {
  const crash = path.join(bin(null), "crash.js");
  fs.writeFileSync(crash, 'require("child_process").spawn = () => { throw new Error("crash"); };');
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
test("the plugin ships its hooks on every event install registers, with no matcher, and one user-only command", () => {
  const manifest = JSON.parse(fs.readFileSync(path.join(ROOT, ".claude-plugin", "plugin.json"), "utf8"));
  assert.deepEqual(Object.keys(manifest).sort(), ["author", "description", "license", "name", "repository"]);
  const components = [".mcp.json", ".lsp.json", "agents", "bin", "output-styles", "workflows", "themes", "monitors", "settings.json", "SKILL.md"];
  assert.deepEqual(components.filter((p) => fs.existsSync(path.join(ROOT, p))), []);
  // M3: /supervisor shows or sets the mode; only a person may run it, never the model.
  assert.deepEqual(fs.readdirSync(path.join(ROOT, "commands")), ["supervisor.md"]);
  const command = fs.readFileSync(path.join(ROOT, "commands", "supervisor.md"), "utf8");
  assert.match(command, /^disable-model-invocation: true$/m);
  assert.match(command, /^!`supervisor level \$ARGUMENTS`$/m);
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
