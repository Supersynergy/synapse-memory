#!/usr/bin/env sh
set -eu

url="https://raw.githubusercontent.com/Supersynergy/synapse-agent-memory/main/release/synapse-agent-memory/install.sh"
tmp=$(mktemp "${TMPDIR:-/tmp}/synapse-agent-memory-forward.XXXXXX")
trap 'rm -f "$tmp"' EXIT HUP INT TERM

if command -v curl >/dev/null 2>&1; then
  curl -fsSL "$url" -o "$tmp"
elif command -v wget >/dev/null 2>&1; then
  wget -qO "$tmp" "$url"
else
  printf '%s\n' "error: curl or wget is required" >&2
  exit 1
fi

sh "$tmp" "$@"
