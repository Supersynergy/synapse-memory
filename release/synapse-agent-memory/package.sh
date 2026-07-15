#!/usr/bin/env sh
set -eu

script_dir=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
repo_root=$(CDPATH='' cd -- "$script_dir/../.." && pwd)
out_dir="${SYNAPSE_RELEASE_OUT:-$repo_root/release/dist-agent-memory}"
target="${SYNAPSE_TARGET:-$(rustc -vV | sed -n 's/^host: //p')}"
allow_dirty="${SYNAPSE_ALLOW_DIRTY:-0}"
dry_run="${SYNAPSE_PACKAGE_DRY_RUN:-0}"

die() {
  printf 'error: %s\n' "$*" >&2
  exit 1
}

[ -n "$target" ] || die "cannot determine Rust target; set SYNAPSE_TARGET"
case "$target" in
  *-pc-windows-*) exe=synx.exe; ext=zip ;;
  *) exe=synx; ext=tar.gz ;;
esac

if [ "$allow_dirty" != 1 ] && [ -n "$(git -C "$repo_root" status --porcelain)" ]; then
  die "refusing to package a dirty worktree; commit intended release changes or set SYNAPSE_ALLOW_DIRTY=1 for a local smoke"
fi

bin="${SYNAPSE_BIN:-}"
if [ -z "$bin" ]; then
  if [ "$ext" = zip ]; then
    die "Windows archives are built by package.ps1 or require SYNAPSE_BIN"
  fi
  cargo build \
    --manifest-path "$repo_root/Cargo.toml" \
    --locked \
    --profile release-hardened \
    --target "$target" \
    -p synapse-cli \
    --no-default-features
  bin="$repo_root/target/$target/release-hardened/$exe"
fi
[ -f "$bin" ] || die "Rust binary not found: $bin"

first_two=$(dd if="$bin" bs=1 count=2 2>/dev/null || true)
[ "$first_two" != '#!' ] || die "refusing script wrapper: $bin"
magic=$(od -An -tx1 -N4 "$bin" | tr -d ' \n')
case "$magic" in
  7f454c46|cffaedfe|feedfacf|cafebabe|bebafeca|4d5a*) ;;
  *) die "unrecognized native executable magic $magic: $bin" ;;
esac

version=$($bin --version | awk 'NF { print $NF; exit }')
[ -n "$version" ] || die "binary did not report a version"
commit=$(git -C "$repo_root" rev-parse --short=12 HEAD)
dirty=false
[ -z "$(git -C "$repo_root" status --porcelain)" ] || dirty=true

tmp=$(mktemp -d "${TMPDIR:-/tmp}/synapse-agent-memory-package.XXXXXX")
trap 'rm -rf "$tmp"' EXIT HUP INT TERM
root="synapse-agent-memory-$target"
stage="$tmp/$root"
mkdir -p "$stage/LICENSES"
cp "$bin" "$stage/$exe"
chmod 0755 "$stage/$exe"
cp "$script_dir/README.md" "$stage/README.md"
cp "$script_dir/THIRD-PARTY-LICENSES.html" "$stage/THIRD-PARTY-LICENSES.html"
cp "$repo_root/NOTICE" "$stage/NOTICE"
cp "$repo_root/ATTRIBUTIONS.md" "$stage/ATTRIBUTIONS.md"
cp "$repo_root/LICENSES/MIT.txt" "$stage/LICENSES/MIT.txt"
cp "$repo_root/LICENSES/FSL-1.1-ALv2.txt" "$stage/LICENSES/FSL-1.1-ALv2.txt"
cat >"$stage/BUILD-INFO.json" <<EOF
{
  "product": "synapse-agent-memory",
  "binary": "$exe",
  "version": "$version",
  "target": "$target",
  "profile": "portable",
  "semantic_embeddings": false,
  "network": false,
  "age_encryption": false,
  "pdf_ingest": false,
  "sharding": false,
  "proprietary_engine": false,
  "first_party_licenses": ["FSL-1.1-ALv2", "MIT"],
  "git_commit": "$commit",
  "dirty": $dirty
}
EOF

if [ "$dry_run" = 1 ]; then
  printf 'package=%s\nbinary=%s\nversion=%s\ntarget=%s\n' "$stage" "$bin" "$version" "$target"
  exit 0
fi

mkdir -p "$out_dir"
asset="synapse-agent-memory-$target.$ext"
archive="$out_dir/$asset"
if [ "$ext" = tar.gz ]; then
  (cd "$tmp" && tar -czf "$archive" "$root")
else
  die "use package.ps1 for Windows zip assets"
fi

if command -v sha256sum >/dev/null 2>&1; then
  (cd "$out_dir" && sha256sum "$asset" >"$asset.sha256")
elif command -v shasum >/dev/null 2>&1; then
  (cd "$out_dir" && shasum -a 256 "$asset" >"$asset.sha256")
else
  die "sha256sum or shasum is required"
fi

printf 'archive=%s\nchecksum=%s.sha256\n' "$archive" "$archive"
