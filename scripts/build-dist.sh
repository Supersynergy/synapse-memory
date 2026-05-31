#!/usr/bin/env sh
# build-dist.sh — build release synapse-mcp binaries for distribution + SHA256SUMS.
#
#   sh scripts/build-dist.sh [out_dir]
#
# Builds the host target always; cross-builds the Linux musl targets too when
# cargo-zigbuild + zig are available. Output: <out>/synapse-mcp-<target> + SHA256SUMS.
# A maintainer attaches these to a GitHub release; install.sh downloads + verifies them.
set -eu

OUT="${1:-dist}"
HERE=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
cd "${HERE}/.."
mkdir -p "$OUT"

HOST=$(rustc -vV | sed -n 's/host: //p')
# Host target by default. Cross targets (Linux gnu) are produced by the
# release-ctxos CI workflow on native runners; override locally with e.g.
#   TARGETS="aarch64-apple-darwin x86_64-apple-darwin" sh scripts/build-dist.sh
TARGETS="${TARGETS:-$HOST}"

built=0
for t in $TARGETS; do
  printf '› building %s\n' "$t"
  rustup target add "$t" >/dev/null 2>&1 || true
  if [ "$t" = "$HOST" ]; then
    cargo build --release -p synapse-mcp --target "$t" || { printf '! %s failed, skipping\n' "$t" >&2; continue; }
  else
    cargo zigbuild --release -p synapse-mcp --target "$t" || { printf '! %s failed (cross), skipping\n' "$t" >&2; continue; }
  fi
  cp "target/${t}/release/synapse-mcp" "${OUT}/synapse-mcp-${t}"
  built=$((built + 1))
done
[ "$built" -gt 0 ] || { printf '✗ no targets built\n' >&2; exit 1; }

( cd "$OUT"
  if command -v sha256sum >/dev/null 2>&1; then sha256sum synapse-mcp-* > SHA256SUMS
  else shasum -a 256 synapse-mcp-* > SHA256SUMS; fi )

printf '› dist ready: %s\n' "$OUT"
ls -1 "$OUT"
