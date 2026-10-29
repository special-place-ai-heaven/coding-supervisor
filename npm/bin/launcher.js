#!/usr/bin/env node
"use strict";

const childProcess = require("child_process");
const fs = require("fs");
const path = require("path");
const { resolveBinary, formatResolveError } = require("../lib/resolve-binary.js");

// Best-effort, path-only scan of $PATH for every `supervisor` entry, in PATH
// order. Never spawns anything (this runs on every MCP server start, so it must
// add negligible latency). On Windows it honors PATHEXT (defaulting to the
// usual shim extensions) so the npm `.cmd`/`.exe` shim is found. Returns the
// list of existing candidate paths, deduplicated, in the order PATH would
// resolve them.
function whichAllSupervisor(env, platform) {
  const environ = env || {};
  const rawPath = environ.PATH || environ.Path || "";
  if (!rawPath) return [];

  const isWindows = platform === "win32";
  // Candidate basenames to probe inside each PATH dir. On Windows, PATHEXT
  // decides which extensions are executable; we only care about the ones a
  // supervisor shim would ever use. The bare name covers a shell function / shim
  // without an extension and the WSL-bleed case where a Linux `supervisor` sits
  // on PATH.
  let names = ["supervisor"];
  if (isWindows) {
    const pathext = (environ.PATHEXT || ".COM;.EXE;.BAT;.CMD")
      .split(";")
      .map((ext) => ext.trim().toLowerCase())
      .filter((ext) => ext === ".exe" || ext === ".cmd" || ext === ".bat" || ext === ".com");
    const exts = pathext.length > 0 ? pathext : [".exe", ".cmd"];
    names = ["supervisor", ...exts.map((ext) => `supervisor${ext}`)];
  }

  const found = [];
  const seen = new Set();
  for (const dir of rawPath.split(path.delimiter)) {
    if (!dir) continue;
    for (const name of names) {
      const candidate = path.join(dir, name);
      if (seen.has(candidate)) continue;
      seen.add(candidate);
      try {
        const stat = fs.statSync(candidate);
        if (stat.isFile()) found.push(candidate);
      } catch (_err) {
        // Missing path entry — skip silently.
      }
    }
  }
  return found;
}

// Same file on disk: realpaths equal, so a symlinked npm bin shim
// (`<prefix>/bin/supervisor` -> `.../supervisor/bin/supervisor.js`) matches its target.
// A path that cannot be resolved falls back to its absolute form.
function sameFile(a, b) {
  const real = (p) => {
    try {
      return fs.realpathSync.native(p);
    } catch (_err) {
      return path.resolve(p);
    }
  };
  return real(a) === real(b);
}

// True when `shadowPath` is THIS launcher's own npm bin shim, not a foreign
// install. The thing on PATH is normally the npm-generated shim
// (`supervisor` / `supervisor.cmd`), which lives in the npm prefix `bin` dir that
// is a sibling of this package's install dir (…/node_modules/supervisor). We
// treat a shim as "ours" when it sits in the bin dir adjacent to our package
// root, or when its file contents reference this package's launcher. Either
// proves the first PATH `supervisor` would dispatch back into this very launcher,
// so there is nothing to warn about.
function isOwnShim(shadowPath, selfDir) {
  try {
    // selfDir is …/supervisor/bin (this launcher's directory). The package root
    // is its parent; the install prefix bin dir is a sibling of node_modules.
    const packageRoot = path.dirname(selfDir); // …/supervisor
    const nodeModules = path.dirname(packageRoot); // …/node_modules
    const prefix = path.dirname(nodeModules); // npm prefix root
    const shimDir = path.dirname(path.resolve(shadowPath));

    // npm places global shims in <prefix>/bin (POSIX) or <prefix> (Windows).
    const prefixBinPosix = path.join(prefix, "bin");
    if (shimDir === prefix || shimDir === prefixBinPosix) {
      return true;
    }

    // The shim may itself be the package's own bin file (e.g. local linked
    // install where PATH points directly at bin/supervisor.js's dir).
    if (shimDir === selfDir) {
      return true;
    }

    // Fall back to content inspection: npm shims embed the relative path back
    // to the package launcher. If the shim text references this package, it is
    // ours regardless of where the prefix sits.
    const text = fs.readFileSync(path.resolve(shadowPath), "utf8");
    if (text.includes("node_modules/coding-supervisor/bin/") || text.includes("node_modules\\supervisor\\bin\\")) {
      return true;
    }
  } catch (_err) {
    // Unreadable / non-shim — fall through and treat as foreign so we do not
    // suppress a genuine shadow on a transient read error.
  }
  return false;
}

// Classify the shadowing install so the warning names the kind of copy that wins
// (what `where supervisor` / `which -a supervisor` would list first).
function classifyShadow(shadowPath, env, platform) {
  const environ = env || {};
  if (platform !== "win32") {
    let procVersion = "";
    try {
      procVersion = fs.readFileSync("/proc/version", "utf8").toLowerCase();
    } catch (_err) {
      // Not Linux / unreadable — not WSL.
    }
    const underWsl = /microsoft|wsl/.test(procVersion);
    if (underWsl && /^\/mnt\/[a-z]\//i.test(shadowPath)) {
      return "windows-bleed";
    }
    if (/^\/usr\/local\//.test(shadowPath) || /^\/usr\//.test(shadowPath) || /^\/opt\//.test(shadowPath)) {
      return "root/system";
    }
  }
  return "foreign-prefix";
}

// Best-effort PATH-shadow warning. When the launcher runs but a DIFFERENT
// `supervisor` would win on $PATH, print one or two stderr lines pointing the
// operator at the binary's richer remediation. NEVER blocks or breaks the
// spawn: the entire body is wrapped in try/catch by the caller and any failure
// is swallowed. Output goes to stderr only (stdout is the MCP stdio channel).
function maybeWarnPathShadow(consoleMod, env, platform, resolvedBinaryPath, selfDir) {
  const environ = env || {};
  if (environ.SUPERVISOR_NO_SHADOW_WARN) return;

  const entries = whichAllSupervisor(environ, platform);
  if (entries.length === 0) return;

  const first = entries[0];

  // The launcher already "wins" when the first PATH supervisor IS the resolved
  // native binary, IS this launcher itself (by realpath, so a symlinked bin
  // shim in any prefix counts), or is this launcher's own npm shim that
  // dispatches here.
  if (sameFile(first, resolvedBinaryPath)) return;
  if (sameFile(first, path.join(selfDir, "supervisor.js"))) return;
  if (sameFile(first, path.join(selfDir, "launcher.js"))) return;
  if (isOwnShim(first, selfDir)) return;

  const kind = classifyShadow(first, environ, platform);
  consoleMod.error(
    `supervisor: warning: a different 'supervisor' (${kind}) is first on your PATH: ${first}`
  );
  consoleMod.error(
    "  This launcher is running, but that one shadows it. Put this npm prefix first on PATH or remove that copy, then rerun 'supervisor install'."
  );
}

function createLauncher(overrides = {}) {
  const processMod = overrides.process || process;
  const consoleMod = overrides.console || console;
  const spawnSyncFn = overrides.spawnSync || childProcess.spawnSync;
  const resolveBinaryFn = overrides.resolveBinary || ((opts) => resolveBinary({
    ...opts,
    platform: processMod.platform,
    arch: processMod.arch,
    requireResolve: overrides.requireResolve || require.resolve,
  }));
  const selfDir = overrides.selfDir || __dirname;

  function main(args) {
    const resolved = resolveBinaryFn({ binary: "supervisor" });
    if (resolved.reason !== "ok") {
      consoleMod.error(formatResolveError(resolved, {
        platform: processMod.platform,
        arch: processMod.arch,
      }));
      return 64;
    }

    // Best-effort PATH-shadow warning. Must never block or break the spawn, so
    // swallow any error from the detection path entirely.
    try {
      maybeWarnPathShadow(
        consoleMod,
        processMod.env,
        processMod.platform,
        resolved.binaryPath,
        selfDir
      );
    } catch (_err) {
      // Detection failure must not stop supervisor from running.
    }

    const result = spawnSyncFn(resolved.binaryPath, args, {
      stdio: "inherit",
      env: processMod.env,
      shell: false,
    });
    if (result.error) {
      consoleMod.error(`supervisor: failed to spawn ${resolved.binaryPath}: ${result.error.code || result.error.message}`);
      return result.error.code === "ENOENT" ? 127 : 126;
    }
    return result.status ?? 1;
  }

  return {
    main,
  };
}

module.exports = {
  createLauncher,
  whichAllSupervisor,
  isOwnShim,
  classifyShadow,
};

if (require.main === module) {
  process.exit(createLauncher().main(process.argv.slice(2)));
}
