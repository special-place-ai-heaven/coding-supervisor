"use strict";

const fs = require("fs");
const path = require("path");

// The launcher's own version: a reinstall hint names it, never @latest, so a fix reinstalls the
// version that is running rather than whatever latest points at.
const { version: LAUNCHER_VERSION } = require("../package.json");
const REINSTALL = `npm install -g --prefer-online coding-supervisor@${LAUNCHER_VERSION}`;

const SUPPORTED_TARGETS = Object.freeze([
  Object.freeze({
    platform: "linux",
    arch: "x64",
    pkg: "coding-supervisor-linux-x64",
    monorepoDir: "linux-x64",
    binaryName: "supervisor",
  }),
  Object.freeze({
    platform: "win32",
    arch: "x64",
    pkg: "coding-supervisor-windows-x64",
    monorepoDir: "windows-x64",
    binaryName: "supervisor.exe",
  }),
]);

const ALLOWED_BINARIES = Object.freeze(["supervisor"]);

function findPlatformPackageJson(target, requireResolve) {
  try {
    return requireResolve(`${target.pkg}/package.json`);
  } catch (_err) {
    // Fall through to local development lookup.
  }

  const local = path.join(__dirname, "..", "platforms", target.monorepoDir, "package.json");
  if (fs.existsSync(local)) {
    return local;
  }

  let dir = path.join(__dirname, "..");
  for (let i = 0; i < 6; i += 1) {
    const nested = path.join(dir, "node_modules", target.pkg, "package.json");
    if (fs.existsSync(nested)) {
      return nested;
    }
    const parent = path.dirname(dir);
    if (parent === dir) break;
    dir = parent;
  }

  return null;
}

function resolveBinary(opts) {
  const options = opts || {};
  const binary = options.binary;
  const platform = options.platform || process.platform;
  const arch = options.arch || process.arch;
  const requireResolve = options.requireResolve || require.resolve;

  if (!ALLOWED_BINARIES.includes(binary)) {
    return {
      reason: "invalid_binary",
      platformPackage: null,
      binaryPath: null,
      supportedTargets: SUPPORTED_TARGETS,
    };
  }

  const target = SUPPORTED_TARGETS.find((t) => t.platform === platform && t.arch === arch);
  if (!target) {
    return {
      reason: "unsupported_platform",
      platformPackage: null,
      binaryPath: null,
      supportedTargets: SUPPORTED_TARGETS,
    };
  }

  const pkgJsonPath = findPlatformPackageJson(target, requireResolve);
  if (!pkgJsonPath) {
    return {
      reason: "platform_package_missing",
      platformPackage: target.pkg,
      binaryPath: null,
      supportedTargets: SUPPORTED_TARGETS,
    };
  }

  const binaryPath = path.join(path.dirname(pkgJsonPath), "bin", target.binaryName);
  if (!fs.existsSync(binaryPath)) {
    return {
      reason: "platform_package_missing",
      platformPackage: target.pkg,
      binaryPath: null,
      supportedTargets: SUPPORTED_TARGETS,
    };
  }

  return {
    reason: "ok",
    platformPackage: target.pkg,
    binaryPath,
    supportedTargets: SUPPORTED_TARGETS,
  };
}

// Detect the WSL trap: a non-Windows process executing a supervisor install whose
// own files live under a Windows drive mount (`/mnt/<letter>/...`). That means
// the launcher came from a Windows npm prefix that bled onto the WSL PATH, so it
// only ships the Windows platform package — `npm install -g` from here just
// rewrites the Windows prefix again. The fix is to install into the Linux prefix.
function isWindowsPrefixUnderUnix(platform, selfPath) {
  if (platform === "win32") return false;
  if (typeof selfPath !== "string") return false;
  return /^\/mnt\/[a-z]\//i.test(selfPath);
}

function formatResolveError(result, opts) {
  const options = opts || {};
  const platform = options.platform || process.platform;
  const arch = options.arch || process.arch;
  const selfPath = options.selfPath || __dirname;
  const targets = result.supportedTargets.map((t) => `${t.platform}-${t.arch}`).join(", ");

  if (result.reason === "ok") return null;
  if (result.reason === "unsupported_platform") {
    return `supervisor: unsupported platform ${platform}-${arch}; supported: ${targets}`;
  }
  if (result.reason === "platform_package_missing") {
    if (isWindowsPrefixUnderUnix(platform, selfPath)) {
      return (
        `supervisor: platform package ${result.platformPackage} is missing because this is a Windows ` +
        `npm install running under ${platform}.\n` +
        `  This launcher lives at ${selfPath} — a Windows drive mount, not your ${platform} npm prefix.\n` +
        `  Cause: a shared Windows ~/.npmrc 'prefix=' (e.g. C:\\Users\\<you>\\.npm-global) is bleeding ` +
        `into ${platform}, so 'npm install -g' lands in the Windows prefix and only ships the Windows binary.\n` +
        `  Give ${platform} its own npm prefix, then reinstall:\n` +
        `    npm config set prefix "$HOME/.npm-global"\n` +
        `    export PATH="$HOME/.npm-global/bin:$PATH"   # ahead of any /mnt/* entries; add to ~/.profile to persist\n` +
        `    hash -r && ${REINSTALL}\n` +
        `  Verify with: which supervisor   # expect $HOME/.npm-global/bin/supervisor, not /mnt/...\n` +
        `  Then rerun: supervisor install --harness <name> --project <dir>`
      );
    }
    return (
      `supervisor: platform package ${result.platformPackage} not installed or missing its binary. ` +
      `Reinstall with optional dependencies enabled: ${REINSTALL}`
    );
  }
  if (result.reason === "invalid_binary") {
    return "supervisor: internal error: invalid binary name passed to resolver";
  }
  return `supervisor: unknown resolver state ${result.reason}`;
}

module.exports = {
  ALLOWED_BINARIES,
  SUPPORTED_TARGETS,
  formatResolveError,
  resolveBinary,
};
