#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd "$script_dir/../.." && pwd)"
tmp_parent="$(mktemp -d "${TMPDIR:-/tmp}/synapse-memory-fresh.XXXXXX")"
snapshot="$tmp_parent/repo"

cleanup() {
  git -C "$repo_root" worktree remove --force "$snapshot" >/dev/null 2>&1 || true
  rmdir "$tmp_parent" >/dev/null 2>&1 || true
}
trap cleanup EXIT HUP INT TERM

for tool in git rsync cargo; do
  command -v "$tool" >/dev/null 2>&1 || {
    printf 'error: required snapshot tool missing: %s\n' "$tool" >&2
    exit 2
  }
done

git -C "$repo_root" worktree add --detach "$snapshot" HEAD >/dev/null

# Apply every tracked worktree delta to a clean HEAD checkout. This catches
# hidden dependencies on files outside the release folder without committing or
# mutating the user's current worktree.
git -C "$repo_root" diff --name-only -z --diff-filter=ACMRTUXB \
  | rsync -a --from0 --files-from=- "$repo_root/" "$snapshot/"

# New release files are not visible to `git diff` until the owner stages them.
new_paths=(
  .github/workflows/synapse-memory-ci.yml
  ATTRIBUTIONS.md
  Cargo.lock
  CODE_OF_CONDUCT.md
  LICENSES
  NOTICE
  SECURITY.md
  crates/synapse-core/src/corpus.rs
  crates/synapse-learn/src/sampling.rs
  docs/adr/0003-portable-synapse-memory-release.md
  integrations/codex
  release/synapse-memory
)
for path in "${new_paths[@]}"; do
  if [ ! -e "$repo_root/$path" ]; then
    printf 'error: required release path missing: %s\n' "$path" >&2
    exit 1
  fi
  (cd "$repo_root" && rsync -aR "$path" "$snapshot/")
done

cd "$snapshot"
host_target="$(rustc -vV | sed -n 's/^host: //p')"
[ -n "$host_target" ] || {
  printf 'error: cannot determine Rust host target\n' >&2
  exit 1
}
CARGO_TARGET_DIR="$snapshot/target-fresh" cargo build \
  --locked \
  --profile release-hardened \
  --target "$host_target" \
  -p synapse-cli \
  --no-default-features

case "$host_target" in
  *-pc-windows-*) executable=synx.exe ;;
  *) executable=synx ;;
esac
SYNX_BIN="$snapshot/target-fresh/$host_target/release-hardened/$executable" \
  release/synapse-memory/verify.sh

printf 'PASS clean-HEAD snapshot plus release overlay\n'
