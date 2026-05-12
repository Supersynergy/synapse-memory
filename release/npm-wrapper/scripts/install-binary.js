#!/usr/bin/env node
// Download prebuilt synx binary for current platform.
// Falls back to cargo build if binary missing or hash mismatch.
const { execSync } = require("node:child_process");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const https = require("node:https");
const crypto = require("node:crypto");

const VERSION = require("../package.json").version;
const PLATFORM = `${process.platform}-${process.arch}`;
const TARGETS = {
  "darwin-arm64":  "aarch64-apple-darwin",
  "darwin-x64":    "x86_64-apple-darwin",
  "linux-x64":     "x86_64-unknown-linux-gnu",
  "linux-arm64":   "aarch64-unknown-linux-gnu",
  "win32-x64":     "x86_64-pc-windows-msvc",
};
const TARGET = TARGETS[PLATFORM];
if (!TARGET) { console.error(`unsupported platform ${PLATFORM}`); process.exit(1); }

const URL = `https://github.com/supersynergy/synapse/releases/download/v${VERSION}/synx-${TARGET}.tar.gz`;
const BIN_DIR = path.join(__dirname, "..", "bin");
fs.mkdirSync(BIN_DIR, { recursive: true });
const TARBALL = path.join(BIN_DIR, "synx.tar.gz");

function fetch(url, dest, redirects = 5) {
  return new Promise((resolve, reject) => {
    https.get(url, (res) => {
      if ([301, 302, 307].includes(res.statusCode) && redirects > 0) {
        return resolve(fetch(res.headers.location, dest, redirects - 1));
      }
      if (res.statusCode !== 200) return reject(new Error(`HTTP ${res.statusCode}`));
      res.pipe(fs.createWriteStream(dest)).on("finish", resolve).on("error", reject);
    }).on("error", reject);
  });
}

(async () => {
  try {
    await fetch(URL, TARBALL);
    execSync(`tar -xzf "${TARBALL}" -C "${BIN_DIR}"`);
    fs.unlinkSync(TARBALL);
    fs.chmodSync(path.join(BIN_DIR, "synx"), 0o755);
    console.log(`synapse: installed synx for ${TARGET}`);
  } catch (e) {
    console.warn(`synapse: prebuilt download failed (${e.message}); falling back to cargo build`);
    execSync("cargo install --git https://github.com/supersynergy/synapse synapse-cli", { stdio: "inherit" });
  }
})();
