const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");

const PKG_ROOT = path.resolve(__dirname, "..");

test("root npm package is passive and uses platform optional dependencies", () => {
  const pkg = JSON.parse(fs.readFileSync(path.join(PKG_ROOT, "package.json"), "utf8"));
  const scripts = pkg.scripts || {};

  assert.equal(scripts.preinstall, undefined);
  assert.equal(scripts.install, undefined);
  assert.equal(scripts.postinstall, undefined);
  assert.deepEqual(pkg.optionalDependencies, {
    "coding-supervisor-linux-x64": pkg.version,
    "coding-supervisor-windows-x64": pkg.version,
  });
  assert.ok(pkg.files.includes("lib/"));
  assert.equal(pkg.files.includes("scripts/"), false);
});

test("launcher contains no installer, downloader, or auto-init path", () => {
  const src = fs.readFileSync(path.join(PKG_ROOT, "bin", "launcher.js"), "utf8");

  assert.doesNotMatch(src, /runInstaller/);
  assert.doesNotMatch(src, /runAutoInit/);
  assert.doesNotMatch(src, /installScriptPath/);
  assert.doesNotMatch(src, /execFileSync/);
  assert.doesNotMatch(src, /scripts[\\/]install/);
});

test("legacy npm install downloader is not shipped", () => {
  assert.equal(fs.existsSync(path.join(PKG_ROOT, "scripts", "install.js")), false);
});
