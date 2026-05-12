#!/usr/bin/env node
const { spawnSync } = require("node:child_process");
const path = require("node:path");
const bin = path.join(__dirname, process.platform === "win32" ? "synx.exe" : "synx");
const r = spawnSync(bin, process.argv.slice(2), { stdio: "inherit" });
process.exit(r.status ?? 1);
