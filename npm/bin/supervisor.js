#!/usr/bin/env node
"use strict";

const { createLauncher } = require("./launcher.js");

try {
  process.exit(createLauncher().main(process.argv.slice(2)));
} catch (err) {
  console.error(err.message);
  process.exit(1);
}
