#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd "$script_dir/../.." && pwd)"
closure="$(mktemp "${TMPDIR:-/tmp}/synapse-agent-memory-closure.XXXXXX")"
audit_json="$(mktemp "${TMPDIR:-/tmp}/synapse-agent-memory-audit.XXXXXX")"
findings="$(mktemp "${TMPDIR:-/tmp}/synapse-agent-memory-findings.XXXXXX")"
trap 'rm -f "$closure" "$audit_json" "$findings"' EXIT

for tool in cargo jq grep; do
  command -v "$tool" >/dev/null 2>&1 || {
    printf 'error: required audit tool missing: %s\n' "$tool" >&2
    exit 2
  }
done
cargo audit --version >/dev/null 2>&1 || {
  printf 'error: cargo-audit is required\n' >&2
  exit 2
}

cd "$repo_root"
targets=(
  aarch64-apple-darwin
  x86_64-apple-darwin
  aarch64-unknown-linux-musl
  x86_64-unknown-linux-musl
  aarch64-pc-windows-msvc
  x86_64-pc-windows-msvc
)
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
  | sort -u >"$closure"

# cargo-audit exits non-zero for advisories anywhere in the monorepo lockfile.
# The release artifact is a narrower, feature-resolved graph, so intersect exact
# package versions with that graph and fail on every vulnerability or warning in it.
audit_cwd="${TMPDIR:-/tmp}"
(cd "$audit_cwd" && cargo audit --file "$repo_root/Cargo.lock" --json) \
  >"$audit_json" 2>/dev/null || true
jq -e . "$audit_json" >/dev/null
jq -r '
  [.vulnerabilities.list[], (.warnings | to_entries[] | .value[])]
  | .[]
  | [.package.name, .package.version, .advisory.id,
     (.advisory.informational // "vulnerability"), .advisory.title]
  | @tsv
' "$audit_json" \
  | while IFS=$'\t' read -r name version id kind title; do
      if grep -Fqx "$name@$version" "$closure"; then
        printf '%s\t%s@%s\t%s\t%s\n' "$id" "$name" "$version" "$kind" "$title"
      fi
    done >"$findings"

# Transitive advisories we cannot upgrade away today (pinned by deps), with
# the puller and why the risk is accepted. Format: "<ADVISORY-ID>\t<name>@<ver>"
# — keep this list minimal and justified, mirroring deny.toml ignores.
allowed=(
  # lru@0.12.5 ← tantivy 0.22 (synapse-fts). pop()/IterMut paths unsound;
  # tantivy's searcher cache uses neither. Upgrade pending tantivy release.
  $'RUSTSEC-2026-0253\tlru@0.12.5'
  $'RUSTSEC-2026-0002\tlru@0.12.5'
  # instant@0.1.13 ← measure_time ← tantivy. Timing helper, unmaintained.
  $'RUSTSEC-2024-0384\tinstant@0.1.13'
  # paste@1.0.15 (proc-macro) ← simba ← nalgebra ← statrs (synapse-learn).
  $'RUSTSEC-2024-0436\tpaste@1.0.15'
  # proc-macro-error2@2.0.1 (proc-macro) ← i18n-embed-fl ← age ← synapse-core.
  $'RUSTSEC-2026-0173\tproc-macro-error2@2.0.1'
  # smallstr@0.3.1 ← yrs (CRDT store, synapse-core). Unmaintained.
  $'RUSTSEC-2026-0215\tsmallstr@0.3.1'
)
if [ "${#allowed[@]}" -gt 0 ]; then
  printf '%s\n' "${allowed[@]}" >"$closure.allowed"
  grep -vFf "$closure.allowed" "$findings" >"$findings.real" || true
  mv "$findings.real" "$findings"
fi

if [ -s "$findings" ]; then
  printf 'FAIL RustSec findings in portable dependency closure:\n' >&2
  sed -n '1,200p' "$findings" >&2
  exit 1
fi

printf 'PASS portable RustSec closure packages=%s findings=0\n' "$(wc -l <"$closure" | tr -d ' ')"
