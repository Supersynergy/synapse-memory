#!/usr/bin/env sh
# install-ctxos.sh — register the Synapse Context-OS MCP server into every
# AI coding CLI you have (Claude Code, Codex, Gemini CLI). One server, three configs.
#
# Usage:
#   sh install-ctxos.sh [install|doctor|uninstall] [--all|--claude|--codex|--gemini] [--dry-run]
#
# Env:
#   SYNAPSE_MCP_BIN   path to the synapse-mcp binary (else autodetected / built)
#   SYNAPSE_SOCK      daemon socket (default /tmp/synapse.sock — clap default, omitted when default)
#   SCOPE             user|project for Claude/Gemini (default: user)
set -eu

NAME=synapse
DEFAULT_SOCK=/tmp/synapse.sock
SOCK="${SYNAPSE_SOCK:-$DEFAULT_SOCK}"
SCOPE="${SCOPE:-user}"
MODE=install
WANT_CLAUDE=0
WANT_CODEX=0
WANT_GEMINI=0
EXPLICIT=0
DRY=0

for arg in "$@"; do
  case "$arg" in
    install|doctor|uninstall) MODE="$arg" ;;
    --all) WANT_CLAUDE=1; WANT_CODEX=1; WANT_GEMINI=1; EXPLICIT=1 ;;
    --claude) WANT_CLAUDE=1; EXPLICIT=1 ;;
    --codex) WANT_CODEX=1; EXPLICIT=1 ;;
    --gemini) WANT_GEMINI=1; EXPLICIT=1 ;;
    --dry-run) DRY=1 ;;
    -h|--help) sed -n '2,13p' "$0"; exit 0 ;;
    *) echo "unknown arg: $arg" >&2; exit 2 ;;
  esac
done

# Default: target every CLI that is installed.
if [ "$EXPLICIT" -eq 0 ]; then
  command -v claude >/dev/null 2>&1 && WANT_CLAUDE=1
  command -v codex  >/dev/null 2>&1 && WANT_CODEX=1
  command -v gemini >/dev/null 2>&1 && WANT_GEMINI=1
fi

say() { printf '%s\n' "$*"; }
run() {
  if [ "$DRY" -eq 1 ]; then say "DRY: $*"; else "$@"; fi
}

find_bin() {
  if [ -n "${SYNAPSE_MCP_BIN:-}" ] && [ -x "$SYNAPSE_MCP_BIN" ]; then
    printf '%s' "$SYNAPSE_MCP_BIN"; return 0
  fi
  p=$(command -v synapse-mcp 2>/dev/null || true)
  if [ -n "$p" ]; then printf '%s' "$p"; return 0; fi
  for c in "$HOME/.local/bin/synapse-mcp" \
           "./target/release/synapse-mcp" \
           "./target/debug/synapse-mcp"; do
    [ -x "$c" ] && { printf '%s' "$c"; return 0; }
  done
  return 1
}

# Build the server-launch arg list. Socket is omitted when it equals the binary's
# default, which avoids any CLI arg-parsing ambiguity for the common case.
sock_args() {
  [ "$SOCK" != "$DEFAULT_SOCK" ] && printf -- '-s %s' "$SOCK"
}

doctor() {
  say "── Synapse Context-OS doctor ──"
  BIN=$(find_bin || true)
  if [ -n "$BIN" ]; then say "✓ server binary: $BIN"; else say "✗ synapse-mcp binary not found (build: cargo build --release -p synapse-mcp)"; fi
  if [ -S "$SOCK" ]; then say "✓ daemon socket: $SOCK"; else say "⚠ daemon socket $SOCK not found (start synapsed; ctxos falls back to read-only brain)"; fi
  reg() { # reg <cli-label> <list-cmd...>
    label=$1; shift
    # merge stderr: some CLIs print the server list (or warnings) to stderr.
    if "$@" 2>&1 | grep -q "^[^A-Za-z]*${NAME}\b\|${NAME}:"; then
      say "✓ $label: $NAME registered"
    else
      say "· $label: $NAME not registered"
    fi
  }
  [ "$WANT_CLAUDE" -eq 1 ] && reg "Claude Code" claude mcp list
  [ "$WANT_CODEX"  -eq 1 ] && reg "Codex"       codex  mcp list
  [ "$WANT_GEMINI" -eq 1 ] && reg "Gemini CLI"  gemini mcp list
  return 0
}

uninstall() {
  [ "$WANT_CLAUDE" -eq 1 ] && run claude mcp remove "$NAME" 2>/dev/null || true
  [ "$WANT_CODEX"  -eq 1 ] && run codex  mcp remove "$NAME" 2>/dev/null || true
  [ "$WANT_GEMINI" -eq 1 ] && run gemini mcp remove "$NAME" 2>/dev/null || true
  say "✓ uninstalled $NAME from selected CLIs"
}

install() {
  BIN=$(find_bin || true)
  if [ -z "$BIN" ]; then
    if command -v cargo >/dev/null 2>&1; then
      say "building synapse-mcp (release)…"
      run cargo build --release -p synapse-mcp
      BIN=$(find_bin || true)
    fi
  fi
  [ -n "$BIN" ] || { say "✗ cannot find or build synapse-mcp"; exit 1; }
  say "server: $BIN  (socket: $SOCK)"
  # shellcheck disable=SC2046
  set -- $(sock_args)   # expands to '-s <sock>' or nothing

  if [ "$WANT_CLAUDE" -eq 1 ]; then
    run claude mcp remove "$NAME" >/dev/null 2>&1 || true
    run claude mcp add -s "$SCOPE" "$NAME" -- "$BIN" "$@"
    say "✓ Claude Code"
  fi
  if [ "$WANT_CODEX" -eq 1 ]; then
    run codex mcp remove "$NAME" >/dev/null 2>&1 || true
    run codex mcp add "$NAME" -- "$BIN" "$@"
    say "✓ Codex"
  fi
  if [ "$WANT_GEMINI" -eq 1 ]; then
    run gemini mcp remove "$NAME" >/dev/null 2>&1 || true
    if [ "$#" -gt 0 ]; then
      run gemini mcp add -s "$SCOPE" --trust "$NAME" "$BIN" "$@"
    else
      run gemini mcp add -s "$SCOPE" --trust "$NAME" "$BIN"
    fi
    say "✓ Gemini CLI"
  fi
  say ""
  say "Done. Verify: sh $0 doctor"
  say "Then in any CLI ask it to call context_pack — it returns budget-bounded verbatim STATE."
}

case "$MODE" in
  install)   install ;;
  doctor)    doctor ;;
  uninstall) uninstall ;;
esac
