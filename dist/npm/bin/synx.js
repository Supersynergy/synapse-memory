#!/usr/bin/env node
"use strict";

const { spawnSync } = require("child_process");
const path = require("path");
const fs = require("fs");

const PLATFORM_MAP = {
  "darwin-arm64": "aarch64-apple-darwin",
  "darwin-x64": "x86_64-apple-darwin",
  "linux-x64": "x86_64-unknown-linux-gnu",
};

const key = `${process.platform}-${process.arch}`;
const target = PLATFORM_MAP[key];

if (!target) {
  process.stderr.write(`synx: unsupported platform ${key}\n`);
  process.exit(1);
}

const binaryPath = path.join(__dirname, target, "synx");

if (!fs.existsSync(binaryPath)) {
  process.stderr.write(
    `synx: binary not found at ${binaryPath}\n` +
    `Run: npm install @supersynergy/synx  (triggers postinstall download)\n`
  );
  process.exit(1);
}

const result = spawnSync(binaryPath, process.argv.slice(2), { stdio: "inherit" });

if (result.error) {
  process.stderr.write(`synx: ${result.error.message}\n`);
  process.exit(1);
}

process.exit(result.status ?? 0);
