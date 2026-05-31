#!/usr/bin/env sh
# install.sh — one-liner bootstrap for the Synapse Context-OS.
#   curl -fsSL https://raw.githubusercontent.com/supersynergy/synapse/main/scripts/install.sh | sh
#
# Builds (or locates) the synapse-mcp server binary, installs it to ~/.local/bin,
# then registers it into every agent CLI you have (Claude Code, Codex, Gemini CLI).
set -eu

REPO="${SYNAPSE_REPO:-https://github.com/supersynergy/synapse}"
PREFIX="${PREFIX:-$HOME/.local/bin}"
HERE=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)

log() { printf '\033[36m›\033[0m %s\n' "$*"; }
die() { printf '\033[31m✗\033[0m %s\n' "$*" >&2; exit 1; }

# 1. Find a built binary, or build it from a checkout.
BIN="$(command -v synapse-mcp 2>/dev/null || true)"
if [ -z "$BIN" ]; then
  for c in "$PREFIX/synapse-mcp" "$HERE/../target/release/synapse-mcp" "$HERE/../target/debug/synapse-mcp"; do
    [ -x "$c" ] && BIN="$c" && break
  done
fi
if [ -z "$BIN" ]; then
  if [ -f "$HERE/../Cargo.toml" ] && command -v cargo >/dev/null 2>&1; then
    log "building synapse-mcp (release)…"
    ( cd "$HERE/.." && cargo build --release -p synapse-mcp )
    BIN="$HERE/../target/release/synapse-mcp"
  else
    die "no synapse-mcp binary and no cargo checkout to build from. Clone $REPO and re-run, or set SYNAPSE_MCP_BIN."
  fi
fi

# 2. Install to PREFIX (skip if it is already the same path).
mkdir -p "$PREFIX"
DEST="$PREFIX/synapse-mcp"
if [ "$(cd "$(dirname "$BIN")" && pwd)/$(basename "$BIN")" != "$DEST" ]; then
  cp -f "$BIN" "$DEST"
  log "installed $DEST"
fi
chmod +x "$DEST"

# macOS (Apple Silicon) SIGKILLs a Mach-O whose signature was invalidated by the copy.
# Re-sign ad-hoc so the binary actually launches from its new path.
if [ "$(uname -s)" = "Darwin" ] && command -v codesign >/dev/null 2>&1; then
  codesign --force --sign - "$DEST" >/dev/null 2>&1 || true
fi

# 3. Register into every CLI present.
SYNAPSE_MCP_BIN="$DEST"
export SYNAPSE_MCP_BIN
if [ -f "$HERE/install-ctxos.sh" ]; then
  sh "$HERE/install-ctxos.sh" install --all
  sh "$HERE/install-ctxos.sh" doctor
else
  log "install-ctxos.sh not found next to this script; register manually (see docs/CTXOS.md)."
fi

log "done — ask any agent CLI to call context_pack."
