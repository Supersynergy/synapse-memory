#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
if [ -f "$script_dir/Cargo.toml" ]; then
  repo_root="$script_dir"
elif [ -f "$script_dir/../../Cargo.toml" ]; then
  repo_root="$(cd "$script_dir/../.." && pwd)"
else
  echo "error: cannot find Synapse Cargo.toml near $script_dir" >&2
  exit 1
fi
prefix="${SYNAPSE_PREFIX:-$HOME/.local}"
db="${SYNAPSE_DB:-$HOME/.synapse/brain.db}"
build_profile="${SYNAPSE_BUILD_PROFILE:-release}"

mkdir -p "$prefix/bin" "$(dirname "$db")"

cd "$repo_root"

if ! command -v cargo >/dev/null 2>&1; then
  echo "error: cargo not found; install Rust via rustup first" >&2
  exit 1
fi

echo "building Synapse Context OS binaries..."
case "$build_profile" in
  release)
    cargo build --release -p synapse-cli -p synapsed -p synapse-mcp --bins
    bin_dir="${CARGO_TARGET_DIR:-$repo_root/target}/release"
    ;;
  dev|debug)
    cargo build -p synapse-cli -p synapsed -p synapse-mcp --bins
    bin_dir="${CARGO_TARGET_DIR:-$repo_root/target}/debug"
    ;;
  *)
    cargo build --profile "$build_profile" -p synapse-cli -p synapsed -p synapse-mcp --bins
    bin_dir="${CARGO_TARGET_DIR:-$repo_root/target}/$build_profile"
    ;;
esac

install -m 0755 "$bin_dir/synx" "$prefix/bin/synx"
if [ -x "$bin_dir/synapsed" ]; then
  install -m 0755 "$bin_dir/synapsed" "$prefix/bin/synapsed"
fi
if [ -x "$bin_dir/synapse-mcp" ]; then
  install -m 0755 "$bin_dir/synapse-mcp" "$prefix/bin/synapse-mcp"
fi

"$prefix/bin/synx" -f "$db" init
"$prefix/bin/synx" -f "$db" doctor --json >/dev/null

cat <<EOF
Synapse Context OS installed.

Binaries: $prefix/bin
Brain DB:  $db

Add this to PATH if needed:
  export PATH="$prefix/bin:\$PATH"

Try:
  synx -f "$db" prime .
  synx -f "$db" remember --kind decision "Use Synapse context packs before major agent work."
  synx -f "$db" context "current task" --mode coding
  synx -f "$db" fresh-context --cwd . --prompt "latest package API changes"
EOF
