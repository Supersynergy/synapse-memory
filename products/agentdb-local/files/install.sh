#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PREFIX="${PREFIX:-/usr/local}"
MODE="${1:-}"

usage() {
  cat <<'EOF'
Synapse one-command installer

Usage:
  ./install.sh --local           Build from this checkout and install to PREFIX/bin
  curl -fsSL https://example.com/synapse/install.sh | bash

Environment:
  PREFIX=/opt/homebrew           Install prefix, default /usr/local
  SYNAPSE_NO_HOOKS=1             Skip Claude-Code hook install
EOF
}

if [[ "$MODE" == "-h" || "$MODE" == "--help" ]]; then
  usage
  exit 0
fi

if [[ ! -f "$ROOT/Cargo.toml" ]]; then
  echo "This installer currently expects a local Synapse checkout." >&2
  echo "Clone the repo, then run: ./install.sh --local" >&2
  exit 2
fi

cargo build --release -p synapse-cli --bin synx
cargo build --release -p synapsed --bin synapsed --bin synx-fast

mkdir -p "$PREFIX/bin"
install -m 0755 "$ROOT/target/release/synx" "$PREFIX/bin/synx"
install -m 0755 "$ROOT/target/release/synapsed" "$PREFIX/bin/synapsed"
install -m 0755 "$ROOT/target/release/synx-fast" "$PREFIX/bin/synx-fast"

if [[ "${SYNAPSE_NO_HOOKS:-0}" != "1" && -x "$ROOT/integrations/claude-code/install.sh" ]]; then
  "$ROOT/integrations/claude-code/install.sh" || true
fi

cat <<EOF
Synapse installed:
  $PREFIX/bin/synx
  $PREFIX/bin/synx-fast
  $PREFIX/bin/synapsed

Start daemon:
  synapsed --file ~/.synapse/brain.db --sock /tmp/synapse.sock --lazy-embed

Verify:
  synx-fast ping
  make -C "$ROOT" bench-agent-memory
EOF
