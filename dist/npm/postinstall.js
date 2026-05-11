"use strict";

const https = require("https");
const fs = require("fs");
const path = require("path");
const { execSync } = require("child_process");
const os = require("os");

const VERSION = require("./package.json").version;
const BASE_URL = `https://github.com/Supersynergy/synapse/releases/download/v${VERSION}`;

const PLATFORM_MAP = {
  "darwin-arm64": "aarch64-apple-darwin",
  "darwin-x64": "x86_64-apple-darwin",
  "linux-x64": "x86_64-unknown-linux-gnu",
};

const key = `${process.platform}-${process.arch}`;
const target = PLATFORM_MAP[key];

if (!target) {
  console.error(`synx postinstall: unsupported platform ${key}, skipping download`);
  process.exit(0);
}

const binDir = path.join(__dirname, "bin", target);
const binaryPath = path.join(binDir, "synx");

if (fs.existsSync(binaryPath)) {
  console.log("synx: binary already present, skipping download");
  process.exit(0);
}

fs.mkdirSync(binDir, { recursive: true });

const tarball = `synx-${target}.tar.gz`;
const url = `${BASE_URL}/${tarball}`;
const tmpFile = path.join(os.tmpdir(), tarball);

console.log(`synx: downloading ${url}`);

function download(url, dest, cb) {
  const file = fs.createWriteStream(dest);
  https.get(url, (res) => {
    if (res.statusCode === 302 || res.statusCode === 301) {
      file.close();
      download(res.headers.location, dest, cb);
      return;
    }
    if (res.statusCode !== 200) {
      cb(new Error(`HTTP ${res.statusCode}`));
      return;
    }
    res.pipe(file);
    file.on("finish", () => file.close(cb));
  }).on("error", cb);
}

download(url, tmpFile, (err) => {
  if (err) {
    console.error(`synx postinstall: download failed — ${err.message}`);
    console.error("Install manually: https://github.com/Supersynergy/synapse/releases");
    process.exit(0); // non-fatal: user can install manually
  }

  try {
    execSync(`tar -xzf ${tmpFile} -C ${binDir} synx`);
    fs.chmodSync(binaryPath, 0o755);
    fs.unlinkSync(tmpFile);
    console.log(`synx: installed to ${binaryPath}`);
  } catch (e) {
    console.error(`synx postinstall: extract failed — ${e.message}`);
    process.exit(0);
  }
});
