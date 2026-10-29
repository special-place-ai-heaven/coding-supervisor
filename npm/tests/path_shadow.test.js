const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");

const {
  createLauncher,
  whichAllSupervisor,
  isOwnShim,
  classifyShadow,
} = require("../bin/launcher.js");

// Create a temp dir that auto-cleans after the test, and return its path.
function tmpDir(t, prefix) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), prefix));
  t.after(() => fs.rmSync(root, { recursive: true, force: true }));
  return root;
}

// Write a fake `supervisor` (or named) file inside `dir` and return its path.
function writeFakeBinary(dir, name = "supervisor", contents = "fake") {
  fs.mkdirSync(dir, { recursive: true });
  const file = path.join(dir, name);
  fs.writeFileSync(file, contents);
  return file;
}

test("whichAllSupervisor returns existing PATH entries in PATH order", (t) => {
  const root = tmpDir(t, "supervisor-which-");
  const dirA = path.join(root, "a");
  const dirB = path.join(root, "b");
  const dirEmpty = path.join(root, "empty");
  fs.mkdirSync(dirEmpty, { recursive: true });

  const binA = writeFakeBinary(dirA);
  const binB = writeFakeBinary(dirB);

  const env = { PATH: [dirA, dirEmpty, dirB].join(path.delimiter) };
  const found = whichAllSupervisor(env, "linux");

  assert.deepEqual(found, [binA, binB]);
});

test("whichAllSupervisor skips directories with no supervisor", (t) => {
  const root = tmpDir(t, "supervisor-which-skip-");
  const dirA = path.join(root, "a");
  const dirB = path.join(root, "b");
  fs.mkdirSync(dirA, { recursive: true });
  const binB = writeFakeBinary(dirB);

  const env = { PATH: [dirA, dirB].join(path.delimiter) };
  const found = whichAllSupervisor(env, "linux");

  assert.deepEqual(found, [binB]);
});

test("whichAllSupervisor ignores a directory that shares the supervisor name", (t) => {
  const root = tmpDir(t, "supervisor-which-dir-");
  const dir = path.join(root, "a");
  // A *directory* literally named supervisor must not count as an executable.
  fs.mkdirSync(path.join(dir, "supervisor"), { recursive: true });

  const env = { PATH: dir };
  const found = whichAllSupervisor(env, "linux");

  assert.deepEqual(found, []);
});

test("whichAllSupervisor returns [] when PATH is unset", () => {
  assert.deepEqual(whichAllSupervisor({}, "linux"), []);
  assert.deepEqual(whichAllSupervisor({ PATH: "" }, "linux"), []);
});

test("whichAllSupervisor resolves Windows extensions via PATHEXT", (t) => {
  const root = tmpDir(t, "supervisor-which-win-");
  const dir = path.join(root, "bin");
  const cmd = writeFakeBinary(dir, "supervisor.cmd");

  const env = {
    Path: dir,
    PATHEXT: ".COM;.EXE;.BAT;.CMD",
  };
  const found = whichAllSupervisor(env, "win32");

  assert.deepEqual(found, [cmd]);
});

test("whichAllSupervisor prefers the bare name before the extensioned shim within a dir", (t) => {
  const root = tmpDir(t, "supervisor-which-order-");
  const dir = path.join(root, "bin");
  const bare = writeFakeBinary(dir, "supervisor");
  const exe = writeFakeBinary(dir, "supervisor.exe");

  const env = { Path: dir, PATHEXT: ".EXE" };
  const found = whichAllSupervisor(env, "win32");

  // Both exist; the bare candidate is probed first within the directory.
  assert.deepEqual(found, [bare, exe]);
});

test("classifyShadow flags /usr/local as root/system", () => {
  assert.equal(classifyShadow("/usr/local/bin/supervisor", {}, "linux"), "root/system");
  assert.equal(classifyShadow("/usr/bin/supervisor", {}, "linux"), "root/system");
  assert.equal(classifyShadow("/opt/supervisor/bin/supervisor", {}, "linux"), "root/system");
});

test("classifyShadow flags a foreign user prefix", () => {
  assert.equal(
    classifyShadow("/home/other/.npm-global/bin/supervisor", {}, "linux"),
    "foreign-prefix"
  );
});

test("classifyShadow never returns root/system on Windows", () => {
  // The /usr, /opt heuristics are POSIX-only; Windows always falls through.
  assert.equal(classifyShadow("C:\\tools\\supervisor\\supervisor.exe", {}, "win32"), "foreign-prefix");
});

test("isOwnShim recognizes a shim in the npm prefix bin dir as ours", (t) => {
  const root = tmpDir(t, "supervisor-shim-prefix-");
  // Layout: <prefix>/lib/node_modules/coding-supervisor/bin (selfDir) + <prefix>/bin/supervisor (shim)
  const selfDir = path.join(root, "lib", "node_modules", "coding-supervisor", "bin");
  fs.mkdirSync(selfDir, { recursive: true });
  // prefix = dirname(dirname(dirname(selfDir))) = root/lib
  const prefix = path.join(root, "lib");
  const shim = writeFakeBinary(path.join(prefix, "bin"), "supervisor", "#!/bin/sh\nnode launcher\n");

  assert.equal(isOwnShim(shim, selfDir), true);
});

test("isOwnShim recognizes a shim by its launcher reference even from a foreign dir", (t) => {
  const root = tmpDir(t, "supervisor-shim-content-");
  const selfDir = path.join(root, "lib", "node_modules", "coding-supervisor", "bin");
  fs.mkdirSync(selfDir, { recursive: true });
  const shimDir = path.join(root, "somewhere", "else");
  const shim = writeFakeBinary(
    shimDir,
    "supervisor",
    'require("../lib/node_modules/coding-supervisor/bin/launcher.js")'
  );

  assert.equal(isOwnShim(shim, selfDir), true);
});

test("isOwnShim treats a genuinely foreign install as not ours", (t) => {
  const root = tmpDir(t, "supervisor-shim-foreign-");
  const selfDir = path.join(root, "lib", "node_modules", "coding-supervisor", "bin");
  fs.mkdirSync(selfDir, { recursive: true });
  const foreign = writeFakeBinary(path.join(root, "usr", "local", "bin"), "supervisor", "totally different binary");

  assert.equal(isOwnShim(foreign, selfDir), false);
});

test("launcher warns when a different supervisor dir is first on PATH", (t) => {
  const root = tmpDir(t, "supervisor-warn-");
  const shadowDir = path.join(root, "usr", "local", "bin");
  const shadow = writeFakeBinary(shadowDir, "supervisor");
  // selfDir lives in a different prefix entirely.
  const selfDir = path.join(root, "home", "lib", "node_modules", "coding-supervisor", "bin");
  fs.mkdirSync(selfDir, { recursive: true });
  const resolvedBinary = path.join(root, "home", "lib", "node_modules", "coding-supervisor-linux-x64", "bin", "supervisor");

  const errors = [];
  const launcher = createLauncher({
    process: {
      platform: "linux",
      arch: "x64",
      env: { PATH: shadowDir },
    },
    console: { error: (message) => errors.push(message) },
    selfDir,
    resolveBinary() {
      return { reason: "ok", binaryPath: resolvedBinary };
    },
    spawnSync() {
      return { status: 0 };
    },
  });

  assert.equal(launcher.main([]), 0);
  const joined = errors.join("\n");
  assert.match(joined, /different 'supervisor'/);
  assert.match(joined, new RegExp(shadow.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")));
  assert.match(joined, /supervisor install/);
});

test("launcher does NOT warn when its own npm shim is first on PATH", (t) => {
  const root = tmpDir(t, "supervisor-noshim-warn-");
  // Realistic global install: <prefix>/lib/node_modules/coding-supervisor/bin is selfDir,
  // <prefix>/bin/supervisor is the npm shim, and the native binary lives in the
  // sibling platform package.
  const prefix = path.join(root, "prefix");
  const selfDir = path.join(prefix, "lib", "node_modules", "coding-supervisor", "bin");
  fs.mkdirSync(selfDir, { recursive: true });
  const shimDir = path.join(prefix, "lib", "bin");
  const shim = writeFakeBinary(shimDir, "supervisor", "#!/bin/sh\nnode launcher\n");
  const resolvedBinary = path.join(
    prefix,
    "lib",
    "node_modules",
    "coding-supervisor-linux-x64",
    "bin",
    "supervisor"
  );

  const errors = [];
  const launcher = createLauncher({
    process: {
      platform: "linux",
      arch: "x64",
      env: { PATH: shimDir },
    },
    console: { error: (message) => errors.push(message) },
    selfDir,
    resolveBinary() {
      return { reason: "ok", binaryPath: resolvedBinary };
    },
    spawnSync() {
      return { status: 0 };
    },
  });

  assert.equal(launcher.main([]), 0);
  assert.deepEqual(errors, []);
  // Sanity: the shim really is the first PATH entry and a different path than
  // the resolved binary, so the "no warn" result is from shim recognition, not
  // an empty PATH.
  assert.notEqual(path.resolve(shim), path.resolve(resolvedBinary));
});

test("launcher does NOT warn when the resolved native binary is itself first on PATH", (t) => {
  const root = tmpDir(t, "supervisor-self-warn-");
  const binDir = path.join(root, "bin");
  const resolvedBinary = writeFakeBinary(binDir, "supervisor");
  const selfDir = path.join(root, "lib", "node_modules", "coding-supervisor", "bin");
  fs.mkdirSync(selfDir, { recursive: true });

  const errors = [];
  const launcher = createLauncher({
    process: {
      platform: "linux",
      arch: "x64",
      env: { PATH: binDir },
    },
    console: { error: (message) => errors.push(message) },
    selfDir,
    resolveBinary() {
      return { reason: "ok", binaryPath: resolvedBinary };
    },
    spawnSync() {
      return { status: 0 };
    },
  });

  assert.equal(launcher.main([]), 0);
  assert.deepEqual(errors, []);
});

// Build a real npm-global-shaped install under `prefix`: the package's bin dir
// holds supervisor.js, and `<prefix>/bin/supervisor` is a symlink to it (skips the
// test where the OS refuses symlinks).
function linkedInstall(t, prefix) {
  const selfDir = path.join(prefix, "lib", "node_modules", "coding-supervisor", "bin");
  fs.mkdirSync(selfDir, { recursive: true });
  fs.writeFileSync(path.join(selfDir, "supervisor.js"), "// launcher\n");
  const binDir = path.join(prefix, "bin");
  fs.mkdirSync(binDir, { recursive: true });
  const link = path.join(binDir, "supervisor");
  try {
    fs.symlinkSync(path.join(selfDir, "supervisor.js"), link);
  } catch (err) {
    t.skip(`symlinks unavailable: ${err.code}`);
    return null;
  }
  return { selfDir, binDir, link };
}

function runLauncher(selfDir, pathDirs, resolvedBinary) {
  const errors = [];
  const launcher = createLauncher({
    process: { platform: "linux", arch: "x64", env: { PATH: pathDirs.join(path.delimiter) } },
    console: { error: (message) => errors.push(message) },
    selfDir,
    resolveBinary() {
      return { reason: "ok", binaryPath: resolvedBinary };
    },
    spawnSync() {
      return { status: 0 };
    },
  });
  assert.equal(launcher.main([]), 0);
  return errors;
}

test("launcher does NOT flag itself when first on PATH through a symlinked bin", (t) => {
  const root = tmpDir(t, "supervisor-selflink-");
  // The prefix is NOT the npm-configured one and the shim text never names the
  // package, so only the realpath comparison can prove this is the launcher.
  const install = linkedInstall(t, path.join(root, ".npm-global"));
  if (!install) return;
  const resolvedBinary = path.join(root, ".npm-global", "lib", "node_modules", "coding-supervisor-linux-x64", "bin", "supervisor");

  assert.deepEqual(runLauncher(install.selfDir, [install.binDir], resolvedBinary), []);
});

test("launcher reached through a symlinked prefix does not flag itself", (t) => {
  const root = tmpDir(t, "supervisor-prefixlink-");
  const install = linkedInstall(t, path.join(root, "real-prefix"));
  if (!install) return;
  const aliasPrefix = path.join(root, "alias-prefix");
  try {
    fs.symlinkSync(path.join(root, "real-prefix"), aliasPrefix, "dir");
  } catch (err) {
    t.skip(`symlinks unavailable: ${err.code}`);
    return;
  }
  const resolvedBinary = path.join(root, "real-prefix", "lib", "node_modules", "coding-supervisor-linux-x64", "bin", "supervisor");

  assert.deepEqual(runLauncher(install.selfDir, [path.join(aliasPrefix, "bin")], resolvedBinary), []);
});

test("a genuinely different first copy still warns even when this launcher is also on PATH", (t) => {
  const root = tmpDir(t, "supervisor-twocopies-");
  const mine = linkedInstall(t, path.join(root, "mine"));
  if (!mine) return;
  const other = linkedInstall(t, path.join(root, "other"));
  if (!other) return;
  const resolvedBinary = path.join(root, "mine", "lib", "node_modules", "coding-supervisor-linux-x64", "bin", "supervisor");

  const errors = runLauncher(mine.selfDir, [other.binDir, mine.binDir], resolvedBinary);
  const joined = errors.join("\n");
  assert.match(joined, /different 'supervisor'/);
  assert.ok(joined.includes(other.link), joined);
});

test("SUPERVISOR_NO_SHADOW_WARN suppresses the warning entirely", (t) => {
  const root = tmpDir(t, "supervisor-suppress-");
  const shadowDir = path.join(root, "usr", "local", "bin");
  writeFakeBinary(shadowDir, "supervisor");
  const selfDir = path.join(root, "home", "lib", "node_modules", "coding-supervisor", "bin");
  fs.mkdirSync(selfDir, { recursive: true });
  const resolvedBinary = path.join(root, "home", "lib", "node_modules", "coding-supervisor-linux-x64", "bin", "supervisor");

  const errors = [];
  const launcher = createLauncher({
    process: {
      platform: "linux",
      arch: "x64",
      env: { PATH: shadowDir, SUPERVISOR_NO_SHADOW_WARN: "1" },
    },
    console: { error: (message) => errors.push(message) },
    selfDir,
    resolveBinary() {
      return { reason: "ok", binaryPath: resolvedBinary };
    },
    spawnSync() {
      return { status: 0 };
    },
  });

  assert.equal(launcher.main([]), 0);
  assert.deepEqual(errors, []);
});

test("the shadow warning never writes to stdout", (t) => {
  const root = tmpDir(t, "supervisor-stdout-");
  const shadowDir = path.join(root, "usr", "local", "bin");
  writeFakeBinary(shadowDir, "supervisor");
  const selfDir = path.join(root, "home", "lib", "node_modules", "coding-supervisor", "bin");
  fs.mkdirSync(selfDir, { recursive: true });
  const resolvedBinary = path.join(root, "home", "lib", "node_modules", "coding-supervisor-linux-x64", "bin", "supervisor");

  const stdoutChunks = [];
  const originalWrite = process.stdout.write;
  process.stdout.write = (chunk, ...rest) => {
    stdoutChunks.push(String(chunk));
    return originalWrite.call(process.stdout, chunk, ...rest);
  };
  t.after(() => {
    process.stdout.write = originalWrite;
  });

  // Real console.error is used here so it would hit stderr (not captured),
  // proving the warning path does not leak onto stdout.
  const launcher = createLauncher({
    process: {
      platform: "linux",
      arch: "x64",
      env: { PATH: shadowDir },
    },
    selfDir,
    resolveBinary() {
      return { reason: "ok", binaryPath: resolvedBinary };
    },
    spawnSync() {
      return { status: 0 };
    },
  });

  assert.equal(launcher.main([]), 0);
  assert.equal(stdoutChunks.join(""), "");
});

test("a thrown error inside the shadow check does not prevent spawn", (t) => {
  const selfDir = path.join(os.tmpdir(), "supervisor-throw-self");
  const spawnCalls = [];

  // Force whichAllSupervisor to throw by handing it a PATH getter that explodes.
  const explodingEnv = {};
  Object.defineProperty(explodingEnv, "PATH", {
    enumerable: true,
    get() {
      throw new Error("boom: PATH access exploded");
    },
  });

  const errors = [];
  const launcher = createLauncher({
    process: {
      platform: "linux",
      arch: "x64",
      env: explodingEnv,
    },
    console: { error: (message) => errors.push(message) },
    selfDir,
    resolveBinary() {
      return { reason: "ok", binaryPath: "/some/native/supervisor" };
    },
    spawnSync(command, args, options) {
      spawnCalls.push({ command, args, options });
      return { status: 0 };
    },
  });

  // Spawn still happens and the exit status is returned despite the thrown
  // detection error.
  assert.equal(launcher.main(["--version"]), 0);
  assert.equal(spawnCalls.length, 1);
  assert.equal(spawnCalls[0].command, "/some/native/supervisor");
});
