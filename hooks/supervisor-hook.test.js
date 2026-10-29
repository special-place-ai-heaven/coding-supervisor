"use strict";

// Run with: node --test hooks/supervisor-hook.test.js (no real binary, npm or plugin host needed).

const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");

const { createWrapper, slashArgs } = require("./supervisor-hook.js");

const MISSING = '{"systemMessage":"coding-supervisor: the gate is not active; run npm install -g coding-supervisor"}';

function tmpDir(t) {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "supervisor-hook-"));
  t.after(() => fs.rmSync(dir, { recursive: true, force: true }));
  return dir;
}

// A wrapper whose spawn and loader are fakes. `npmRoot` null makes `npm root -g` fail.
function harness(t, { env = {}, npmRoot = null, binaryPath = null, status = 0 } = {}) {
  const calls = [];
  const printed = [];
  const loaded = [];
  const wrapper = createWrapper({
    env,
    platform: "linux",
    tmpdir: tmpDir(t),
    print: (text) => printed.push(text),
    spawnSync(command, args, options) {
      calls.push({ command, args, options });
      if (command === "npm") {
        return npmRoot ? { status: 0, stdout: `${npmRoot}\n` } : { status: 1, stdout: "" };
      }
      return { status };
    },
    load(file) {
      loaded.push(file);
      return {
        resolveBinary: (opts) => {
          assert.deepEqual(opts, { binary: "supervisor" });
          return binaryPath ? { reason: "ok", binaryPath } : { reason: "platform_package_missing" };
        },
      };
    },
  });
  return { wrapper, calls, printed, loaded };
}

test("SUPERVISOR_BIN wins and arguments, stdio and the exit code pass through", (t) => {
  const h = harness(t, { env: { SUPERVISOR_BIN: "/opt/sup", CLAUDE_PLUGIN_DATA: tmpDir(t) }, status: 2 });
  assert.equal(h.wrapper.main(["hook"]), 2);
  assert.equal(h.calls.length, 1);
  assert.equal(h.calls[0].command, "/opt/sup");
  assert.deepEqual(h.calls[0].args, ["hook"]);
  assert.equal(h.calls[0].options.stdio, "inherit");
  assert.deepEqual(h.printed, []);
});

test("a cached path is used, and npm is not asked", (t) => {
  const data = tmpDir(t);
  const bin = path.join(data, "supervisor-bin");
  fs.writeFileSync(bin, "x");
  fs.writeFileSync(path.join(data, "binary-path"), `${bin}\n`);
  const h = harness(t, { env: { CLAUDE_PLUGIN_DATA: data } });
  assert.equal(h.wrapper.main(["hook"]), 0);
  assert.deepEqual(h.calls.map((c) => c.command), [bin]);
  assert.deepEqual(h.loaded, []);
});

test("without a usable cache, npm root -g then the global package's resolver; the result is cached", (t) => {
  const data = tmpDir(t);
  fs.writeFileSync(path.join(data, "binary-path"), path.join(data, "gone")); // stale: file is missing
  const root = path.join(tmpDir(t), "node_modules");
  const bin = path.join(root, "coding-supervisor-linux-x64", "bin", "supervisor");
  const h = harness(t, { env: { CLAUDE_PLUGIN_DATA: data }, npmRoot: root, binaryPath: bin });
  assert.equal(h.wrapper.main(["hook"]), 0);
  assert.deepEqual(h.calls.map((c) => [c.command, c.args]), [["npm", ["root", "-g"]], [bin, ["hook"]]]);
  assert.deepEqual(h.loaded, [path.join(root, "coding-supervisor", "lib", "resolve-binary.js")]);
  assert.equal(fs.readFileSync(path.join(data, "binary-path"), "utf8"), bin);
});

test("a missing binary is exit 0, announced on the first call only", (t) => {
  const data = tmpDir(t);
  const h = harness(t, { env: { CLAUDE_PLUGIN_DATA: data } });
  assert.equal(h.wrapper.main(["hook"]), 0);
  assert.deepEqual(h.printed, [MISSING]);
  assert.equal(h.wrapper.main(["hook"]), 0);
  assert.deepEqual(h.printed, [MISSING]);
  assert.ok(fs.existsSync(path.join(data, "gate-not-active-notified")));
});

test("without CLAUDE_PLUGIN_DATA the marker goes in the temp directory", (t) => {
  const h = harness(t, { npmRoot: "/nowhere", binaryPath: null });
  assert.equal(h.wrapper.main(["hook"]), 0);
  assert.equal(h.wrapper.main(["hook"]), 0);
  assert.deepEqual(h.printed, [MISSING]);
});

test("a binary that cannot be started is treated as missing", (t) => {
  const calls = [];
  const printed = [];
  const wrapper = createWrapper({
    env: { SUPERVISOR_BIN: "/opt/sup", CLAUDE_PLUGIN_DATA: tmpDir(t) },
    print: (text) => printed.push(text),
    spawnSync(command) {
      calls.push(command);
      return { error: Object.assign(new Error("spawn"), { code: "ENOENT" }) };
    },
  });
  assert.equal(wrapper.main(["hook"]), 0);
  assert.deepEqual(printed, [MISSING]);
});

test("slash maps a level to `level <word>`, status to `status`, nothing to `level`", (t) => {
  for (const [words, expected] of [
    [["full"], ["level", "full"]],
    [["off"], ["level", "off"]],
    [["lite"], ["level", "lite"]],
    [["ultra"], ["level", "ultra"]],
    [["status"], ["status"]],
    [[], ["level"]],
  ]) {
    assert.deepEqual(slashArgs(words), expected);
    const h = harness(t, { env: { SUPERVISOR_BIN: "/opt/sup" } });
    assert.equal(h.wrapper.main(["slash", ...words]), 0);
    assert.deepEqual(h.calls.map((c) => [c.command, c.args]), [["/opt/sup", expected]]);
  }
});

test("slash with anything else prints the usage line and starts nothing", (t) => {
  for (const words of [["bogus"], ["full", "extra"], ["status;", "ls"]]) {
    const h = harness(t, { env: { SUPERVISOR_BIN: "/opt/sup" } });
    assert.equal(h.wrapper.main(["slash", ...words]), 0);
    assert.deepEqual(h.printed, ["usage: /supervisor off|lite|full|ultra|status"]);
    assert.deepEqual(h.calls, []);
  }
});

test("slash with no binary says so every time", (t) => {
  const h = harness(t, { env: { CLAUDE_PLUGIN_DATA: tmpDir(t) } });
  assert.equal(h.wrapper.main(["slash", "full"]), 0);
  assert.equal(h.wrapper.main(["slash", "full"]), 0);
  assert.equal(h.printed.length, 2);
  assert.match(h.printed[0], /npm install -g coding-supervisor/);
});
