#!/usr/bin/env node
"use strict";

// Plugin wrapper: runs the native `supervisor` binary that `npm install -g coding-supervisor`
// installed, with this process's arguments, stdin, stdout and stderr. It never blocks on its own
// failure: a missing binary is exit 0 with no decision, announced once.
//   supervisor-hook.js hook            -> supervisor hook          (every plugin hook)
//   supervisor-hook.js slash [word]    -> supervisor level [word] | supervisor status  (/supervisor)

const childProcess = require("child_process");
const fs = require("fs");
const os = require("os");
const path = require("path");

const MISSING_HOOK =
  '{"systemMessage":"coding-supervisor: the gate is not active; run npm install -g coding-supervisor"}';
const MISSING_SLASH = "coding-supervisor: the supervisor binary was not found; run npm install -g coding-supervisor";
const USAGE = "usage: /supervisor off|lite|full|ultra|status";
const LEVELS = ["off", "lite", "full", "ultra"];

// `/supervisor` words to binary arguments; null for anything else.
function slashArgs(words) {
  if (words.length === 0) return ["level"];
  if (words.length > 1) return null;
  if (words[0] === "status") return ["status"];
  return LEVELS.includes(words[0]) ? ["level", words[0]] : null;
}

function createWrapper(overrides = {}) {
  const env = overrides.env || process.env;
  const platform = overrides.platform || process.platform;
  const fsMod = overrides.fs || fs;
  const spawnSync = overrides.spawnSync || childProcess.spawnSync;
  const load = overrides.load || require;
  const tmpdir = overrides.tmpdir || os.tmpdir();
  const print = overrides.print || ((text) => process.stdout.write(`${text}\n`));
  const dataDir = env.CLAUDE_PLUGIN_DATA || null;

  // SUPERVISOR_BIN, then the path cached in the plugin data directory, then the globally
  // installed npm package's own resolver. null when none works.
  // shortcut: without a cache hit every call asks `npm root -g` again (about a second), so a
  // user who never installs the package pays that on each hook call; cache a miss if it matters.
  function resolve() {
    if (env.SUPERVISOR_BIN) return env.SUPERVISOR_BIN;
    const cache = dataDir && path.join(dataDir, "binary-path");
    if (cache && fsMod.existsSync(cache)) {
      const cached = fsMod.readFileSync(cache, "utf8").trim();
      if (cached && fsMod.existsSync(cached)) return cached;
    }
    const npm = spawnSync("npm", ["root", "-g"], {
      encoding: "utf8",
      shell: platform === "win32", // npm is npm.cmd there
      timeout: 20000,
    });
    const root = npm.status === 0 && npm.stdout ? npm.stdout.trim() : "";
    if (!root) return null;
    const { resolveBinary } = load(path.join(root, "coding-supervisor", "lib", "resolve-binary.js"));
    const found = resolveBinary({ binary: "supervisor" });
    if (found.reason !== "ok") return null;
    if (cache) {
      try {
        fsMod.mkdirSync(dataDir, { recursive: true });
        fsMod.writeFileSync(cache, found.binaryPath);
      } catch (_err) {
        // An unwritable cache only costs the next call another lookup.
      }
    }
    return found.binaryPath;
  }

  // True once per marker file; false when the marker already exists or cannot be written.
  function firstMiss() {
    const dir = dataDir || tmpdir;
    const marker = path.join(dir, "gate-not-active-notified");
    try {
      if (fsMod.existsSync(marker)) return false;
      fsMod.mkdirSync(dir, { recursive: true });
      fsMod.writeFileSync(marker, "");
      return true;
    } catch (_err) {
      return false;
    }
  }

  function main(argv) {
    const slash = argv[0] === "slash";
    let args = argv;
    if (slash) {
      args = slashArgs(argv.slice(1));
      if (!args) {
        print(USAGE);
        return 0;
      }
    }
    const binary = resolve();
    const result = binary && spawnSync(binary, args, { stdio: "inherit", env, shell: false });
    if (result && !result.error) return result.status ?? 1;
    // A person who typed /supervisor is told every time; a hook only the first time.
    if (slash) print(MISSING_SLASH);
    else if (firstMiss()) print(MISSING_HOOK);
    return 0;
  }

  return { main };
}

module.exports = { createWrapper, slashArgs };

if (require.main === module) {
  let code = 0;
  try {
    code = createWrapper().main(process.argv.slice(2));
  } catch (_err) {
    // Never block the agent because the wrapper itself failed.
  }
  process.exit(code);
}
