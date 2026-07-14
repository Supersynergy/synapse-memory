#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd "$script_dir/../.." && pwd)"
mode="${1:-check}"
report="$script_dir/THIRD-PARTY-LICENSES.html"
tmp="$(mktemp -d "${TMPDIR:-/tmp}/synapse-memory-licenses.XXXXXX")"
trap 'rm -rf "$tmp"' EXIT

case "$mode" in
  check|generate) ;;
  *) printf 'usage: %s [check|generate]\n' "$0" >&2; exit 2 ;;
esac

for tool in cargo jq rg comm; do
  command -v "$tool" >/dev/null 2>&1 || {
    printf 'error: required license tool missing: %s\n' "$tool" >&2
    exit 2
  }
done
if [ "$(cargo about --version 2>/dev/null)" != "cargo-about 0.9.0" ]; then
  printf 'error: cargo-about 0.9.0 required; install with --locked --features cli\n' >&2
  exit 2
fi

targets=(
  aarch64-apple-darwin
  x86_64-apple-darwin
  aarch64-unknown-linux-musl
  x86_64-unknown-linux-musl
  aarch64-pc-windows-msvc
  x86_64-pc-windows-msvc
)

cd "$repo_root"
for target in "${targets[@]}"; do
  CARGO_TERM_COLOR=never cargo tree \
    --locked \
    -p synapse-cli \
    --no-default-features \
    --target "$target" \
    --prefix none \
    --format '{p}' 2>/dev/null
done \
  | awk 'NF >= 2 && $2 ~ /^v/ { version=$2; sub(/^v/, "", version); print $1 "@" version }' \
  | sort -u >"$tmp/closure"

common=(
  --locked
  --offline
  --fail
  --manifest-path crates/synapse-cli/Cargo.toml
  --no-default-features
  --config release/synapse-memory/about.toml
)
cargo about generate "${common[@]}" --format json --output-file "$tmp/about.json"
cargo about generate "${common[@]}" release/synapse-memory/about.hbs --output-file "$tmp/report.html"
# cargo-about preserves trailing spaces inside embedded license text. Normalize
# the generated artifact here so `git diff --check` and `check` agree.
perl -pi -e 's/\r$//; s/[ \t]+$//' "$tmp/report.html"

jq -r '[.licenses[].used_by[].crate | (.name + "@" + .version)] | unique[]' \
  "$tmp/about.json" | sort -u >"$tmp/reported"
comm -23 "$tmp/closure" "$tmp/reported" >"$tmp/missing"
if [ -s "$tmp/missing" ]; then
  printf 'FAIL dependency licenses missing for release closure:\n' >&2
  sed -n '1,200p' "$tmp/missing" >&2
  exit 1
fi
if jq -r '.overview[].id' "$tmp/about.json" \
  | rg -i '^(AGPL|GPL-[123]|SSPL|BUSL)' >/dev/null; then
  printf 'FAIL copyleft or source-available dependency license in portable report\n' >&2
  exit 1
fi

if [ "$mode" = generate ]; then
  install -m 0644 "$tmp/report.html" "$report"
elif ! cmp -s "$tmp/report.html" "$report"; then
  printf 'FAIL dependency license report is stale; run %s generate\n' "$0" >&2
  exit 1
fi

printf 'PASS portable license closure packages=%s report_packages=%s\n' \
  "$(wc -l <"$tmp/closure" | tr -d ' ')" \
  "$(wc -l <"$tmp/reported" | tr -d ' ')"
