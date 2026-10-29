const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");

const { createLauncher } = require("../bin/launcher.js");
const {
  ALLOWED_BINARIES,
  SUPPORTED_TARGETS,
  formatResolveError,
  resolveBinary,
} = require("../lib/resolve-binary.js");

function withPlatformPackage(t, packageName, binaryName) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "supervisor-platform-"));
  t.after(() => fs.rmSync(root, { recursive: true, force: true }));

  const pkgRoot = path.join(root, packageName);
  const binDir = path.join(pkgRoot, "bin");
  fs.mkdirSync(binDir, { recursive: true });
  fs.writeFileSync(path.join(pkgRoot, "package.json"), JSON.stringify({ name: packageName }));
  fs.writeFileSync(path.join(binDir, binaryName), "binary");

  return {
    root,
    packageJsonPath: path.join(pkgRoot, "package.json"),
    binaryPath: path.join(binDir, binaryName),
  };
}

test("resolveBinary maps win32-x64 to the packaged Windows executable", (t) => {
  const fixture = withPlatformPackage(t, "coding-supervisor-windows-x64", "supervisor.exe");
  const result = resolveBinary({
    binary: "supervisor",
    platform: "win32",
    arch: "x64",
    requireResolve(specifier) {
      assert.equal(specifier, "coding-supervisor-windows-x64/package.json");
      return fixture.packageJsonPath;
    },
  });

  assert.equal(result.reason, "ok");
  assert.equal(result.platformPackage, "coding-supervisor-windows-x64");
  assert.equal(result.binaryPath, fixture.binaryPath);
});

test("resolveBinary rejects unsupported platforms without shelling out", () => {
  const result = resolveBinary({
    binary: "supervisor",
    platform: "freebsd",
    arch: "x64",
    requireResolve() {
      throw new Error("requireResolve must not be called");
    },
  });

  assert.equal(result.reason, "unsupported_platform");
  assert.match(formatResolveError(result, { platform: "freebsd", arch: "x64" }), /unsupported platform freebsd-x64/);
});

test("resolveBinary reports missing optional platform package", () => {
  const result = resolveBinary({
    binary: "supervisor",
    platform: "linux",
    arch: "x64",
    requireResolve() {
      throw new Error("not installed");
    },
  });

  assert.equal(result.reason, "platform_package_missing");
  assert.equal(result.platformPackage, "coding-supervisor-linux-x64");
});

test("formatResolveError diagnoses the WSL Windows-prefix trap for a /mnt launcher", () => {
  const result = resolveBinary({
    binary: "supervisor",
    platform: "linux",
    arch: "x64",
    requireResolve() {
      throw new Error("not installed");
    },
  });

  const message = formatResolveError(result, {
    platform: "linux",
    arch: "x64",
    selfPath: "/mnt/c/Users/rakovnik/.npm-global/node_modules/coding-supervisor/lib",
  });

  assert.match(message, /Windows npm install running under linux/);
  assert.match(message, /\/mnt\/c\/Users\/rakovnik/);
  assert.match(message, /shared Windows ~\/\.npmrc 'prefix='/);
  assert.match(message, /npm config set prefix "\$HOME\/\.npm-global"/);
  assert.match(message, /supervisor install --harness/);
});

test("formatResolveError keeps the generic missing-package message off Windows mounts", () => {
  const result = resolveBinary({
    binary: "supervisor",
    platform: "linux",
    arch: "x64",
    requireResolve() {
      throw new Error("not installed");
    },
  });

  const message = formatResolveError(result, {
    platform: "linux",
    arch: "x64",
    selfPath: "/home/rakovnik/.npm-global/lib/node_modules/coding-supervisor/lib",
  });

  assert.match(message, /not installed or missing its binary/);
  assert.doesNotMatch(message, /Windows drive mount/);
});

test("formatResolveError never shows the WSL hint to a real Windows host", () => {
  const result = resolveBinary({
    binary: "supervisor",
    platform: "win32",
    arch: "x64",
    requireResolve() {
      throw new Error("not installed");
    },
  });

  const message = formatResolveError(result, {
    platform: "win32",
    arch: "x64",
    selfPath: "/mnt/c/whatever",
  });

  assert.match(message, /not installed or missing its binary/);
  assert.doesNotMatch(message, /Windows drive mount/);
});

test("resolveBinary rejects invalid binary names", () => {
  const result = resolveBinary({ binary: "other", platform: "linux", arch: "x64" });

  assert.equal(result.reason, "invalid_binary");
  assert.deepEqual(ALLOWED_BINARIES, ["supervisor"]);
  assert.ok(SUPPORTED_TARGETS.some((target) => target.platform === "win32" && target.arch === "x64"));
});

test("launcher spawns the resolved native binary directly", () => {
  const spawnCalls = [];
  const errors = [];
  const launcher = createLauncher({
    process: { platform: "win32", arch: "x64", env: { A: "B" } },
    console: { error: (message) => errors.push(message) },
    resolveBinary() {
      return {
        reason: "ok",
        binaryPath: "C:\\\\tools\\\\supervisor\\\\supervisor.exe",
      };
    },
    spawnSync(command, args, options) {
      spawnCalls.push({ command, args, options });
      return { status: 0 };
    },
  });

  assert.equal(launcher.main(["--version"]), 0);
  assert.deepEqual(spawnCalls, [{
    command: "C:\\\\tools\\\\supervisor\\\\supervisor.exe",
    args: ["--version"],
    options: { stdio: "inherit", env: { A: "B" }, shell: false },
  }]);
  assert.deepEqual(errors, []);
});

test("launcher returns 64 when the platform package is unavailable", () => {
  const errors = [];
  const launcher = createLauncher({
    process: { platform: "linux", arch: "arm64", env: {} },
    console: { error: (message) => errors.push(message) },
    spawnSync() {
      throw new Error("spawnSync must not be called");
    },
  });

  assert.equal(launcher.main([]), 64);
  assert.match(errors.join("\n"), /unsupported platform linux-arm64/);
});

test("launcher maps spawn failures to process-style error codes", () => {
  const errors = [];
  const launcher = createLauncher({
    process: { platform: "linux", arch: "x64", env: {} },
    console: { error: (message) => errors.push(message) },
    resolveBinary() {
      return { reason: "ok", binaryPath: "/usr/local/bin/supervisor" };
    },
    spawnSync() {
      return { error: Object.assign(new Error("permission denied"), { code: "EACCES" }) };
    },
  });

  assert.equal(launcher.main([]), 126);
  assert.match(errors.join("\n"), /failed to spawn/);
});
