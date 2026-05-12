# @supersynergy/synx

Synapse CLI — 8ms hybrid search, SimSIMD kernels, 113k docs.

## Install

```bash
npm install -g @supersynergy/synx
# or one-shot:
npx @supersynergy/synx --version
```

## Usage

```bash
synx ping
synx stats
synx hybrid "vector search" 8
synx put "my document text" --title "My Doc"
synx bench
```

## Supported platforms

| OS | Arch | Binary |
|----|------|--------|
| macOS | arm64 (M1+) | aarch64-apple-darwin |
| macOS | x64 | x86_64-apple-darwin |
| Linux | x64 | x86_64-unknown-linux-gnu |

## How it works

`postinstall.js` downloads the correct prebuilt binary from GitHub Releases.
The `bin/synx.js` wrapper spawns the native binary with full stdio passthrough.

## Publish to npm (maintainer)

```bash
cd dist/npm
npm publish --access public
```
