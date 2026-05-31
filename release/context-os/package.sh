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
out_dir="${SYNAPSE_RELEASE_OUT:-$repo_root/release/dist}"
name="${SYNAPSE_RELEASE_NAME:-synapse-context-os}"
version="${SYNAPSE_RELEASE_VERSION:-1.0.1-rc.1}"
dry_run="${SYNAPSE_PACKAGE_DRY_RUN:-0}"
include_bin="${SYNAPSE_PACKAGE_INCLUDE_BIN:-0}"
bin_dir="${SYNAPSE_BIN_DIR:-$repo_root/target/release}"

detect_target_label() {
  os="$(uname -s | tr '[:upper:]' '[:lower:]')"
  arch="$(uname -m | tr '[:upper:]' '[:lower:]')"
  case "$os" in
    darwin) os="macos" ;;
  esac
  case "$arch" in
    arm64) arch="aarch64" ;;
    amd64) arch="x86_64" ;;
  esac
  printf '%s-%s' "$os" "$arch"
}

target_label="${SYNAPSE_RELEASE_TARGET:-$(detect_target_label)}"
package_id="$name-$version"
package_kind="source"
if [ "$include_bin" = "1" ]; then
  package_kind="binary"
  if [ -z "$target_label" ]; then
    echo "error: binary package requires SYNAPSE_RELEASE_TARGET or detectable OS/arch" >&2
    exit 1
  fi
  package_id="$name-$version-$target_label"
fi

stage="$(mktemp -d "${TMPDIR:-/tmp}/synapse-package.XXXXXX")"
trap 'rm -rf "$stage"' EXIT
pkg="$stage/$package_id"
mkdir -p "$pkg/sample" "$pkg/release/context-os/sample"

cat >"$pkg/Cargo.toml" <<EOF
[workspace]
resolver = "2"
members = [
    "crates/synapse-kernel",
    "crates/synapse-ann",
    "crates/synapse-engine",
    "crates/synapse-quant",
    "crates/synapse-fts",
    "crates/synapse-graph",
    "crates/synapse-spann",
    "crates/synapse-obs",
    "crates/synapse-tsdb",
    "crates/synapse-core",
    "crates/synapse-learn",
    "crates/synapse-rerank",
    "crates/synapse-license",
    "crates/synapse-market",
    "crates/synapsed",
    "crates/synapse-mcp",
    "crates/synapse-cli",
]
EOF
awk 'BEGIN { skip = 1 } /^\[workspace.metadata\]/ { skip = 0 } !skip { print }' "$repo_root/Cargo.toml" >>"$pkg/Cargo.toml"

install -m 0644 "$repo_root/Cargo.lock" "$pkg/Cargo.lock"
install -m 0644 "$repo_root/rust-toolchain.toml" "$pkg/rust-toolchain.toml"
install -m 0644 "$repo_root/LICENSE" "$pkg/LICENSE"
install -m 0644 "$script_dir/README.md" "$pkg/README.md"
install -m 0644 "$script_dir/MANIFEST.md" "$pkg/MANIFEST.md"
install -m 0644 "$script_dir/CHECKLIST.md" "$pkg/CHECKLIST.md"
install -m 0644 "$script_dir/RELEASE_NOTES.md" "$pkg/RELEASE_NOTES.md"
install -m 0644 "$script_dir/VERIFICATION.md" "$pkg/VERIFICATION.md"
install -m 0755 "$script_dir/install.sh" "$pkg/install.sh"
install -m 0755 "$script_dir/service.sh" "$pkg/service.sh"
install -m 0755 "$script_dir/verify.sh" "$pkg/verify.sh"
install -m 0755 "$script_dir/package.sh" "$pkg/package.sh"
install -m 0644 "$script_dir/sample/seed.jsonl" "$pkg/sample/seed.jsonl"

install -m 0644 "$script_dir/README.md" "$pkg/release/context-os/README.md"
install -m 0644 "$script_dir/MANIFEST.md" "$pkg/release/context-os/MANIFEST.md"
install -m 0644 "$script_dir/CHECKLIST.md" "$pkg/release/context-os/CHECKLIST.md"
install -m 0644 "$script_dir/RELEASE_NOTES.md" "$pkg/release/context-os/RELEASE_NOTES.md"
install -m 0644 "$script_dir/VERIFICATION.md" "$pkg/release/context-os/VERIFICATION.md"
install -m 0755 "$script_dir/install.sh" "$pkg/release/context-os/install.sh"
install -m 0755 "$script_dir/service.sh" "$pkg/release/context-os/service.sh"
install -m 0755 "$script_dir/verify.sh" "$pkg/release/context-os/verify.sh"
install -m 0755 "$script_dir/package.sh" "$pkg/release/context-os/package.sh"
install -m 0644 "$script_dir/sample/seed.jsonl" "$pkg/release/context-os/sample/seed.jsonl"

mkdir -p "$pkg/python/migrations"
install -m 0644 "$repo_root/python/migrations/add_rerank_log.sql" "$pkg/python/migrations/add_rerank_log.sql"

copy_tree() {
  rel="$1"
  mkdir -p "$pkg/$(dirname "$rel")"
  cp -R "$repo_root/$rel" "$pkg/$rel"
}

for crate in \
  synapse-kernel \
  synapse-ann \
  synapse-engine \
  synapse-quant \
  synapse-fts \
  synapse-graph \
  synapse-spann \
  synapse-obs \
  synapse-tsdb \
  synapse-core \
  synapse-learn \
  synapse-rerank \
  synapse-license \
  synapse-market \
  synapsed \
  synapse-mcp \
  synapse-cli
do
  copy_tree "crates/$crate"
done

find "$pkg" \( -name target -o -name .git -o -name .cargo -o -name .synapse -o -name .claude -o -name .codex -o -name __pycache__ -o -name .pytest_cache \) -prune -exec rm -rf {} +
find "$pkg" -name '*.pyc' -delete
find "$pkg" \( -name '*.rlib' -o -name '*.dylib' -o -name '*.so' -o -name '*.dll' -o -name '*.a' -o -name '*.o' \) -delete
rm -f "$pkg/crates/synapse-mcp/README.md"

maintainer_user="master"
maintainer_home="/Users/${maintainer_user}"
maintainer_home_re="${maintainer_home}|/home/${maintainer_user}"

sanitize_private_paths() {
  file="$1"
  if [ -f "$file" ]; then
    tmp_file="$file.sanitized"
    sed \
      -e "s#${maintainer_home}/projects/synapse#./synapse#g" \
      -e "s#${maintainer_home}/.synapse/brain.db#~/.synapse/brain.db#g" \
      "$file" >"$tmp_file"
    mv "$tmp_file" "$file"
  fi
}

sanitize_private_paths "$pkg/crates/synapse-ann/examples/raw_ann_microbench.rs"
sanitize_private_paths "$pkg/crates/synapse-core/examples/bench_ndarray_real.rs"

market_manifest="$pkg/crates/synapse-market/Cargo.toml"
if [ -f "$market_manifest" ]; then
  market_manifest_tmp="$market_manifest.release"
  awk '
    /opensrv-mysql =/ { next }
    /smx-mysql =/ { next }
    $0 == "[[bin]]" {
      getline name_line
      if (name_line == "name = \"smx_mysql_shim\"") {
        getline
        getline
        next
      }
      print
      print name_line
      next
    }
    { print }
  ' "$market_manifest" >"$market_manifest_tmp"
  mv "$market_manifest_tmp" "$market_manifest"
fi

if [ "$include_bin" = "1" ]; then
  mkdir -p "$pkg/bin"
  for bin in synx synapsed synapse-mcp; do
    if [ ! -x "$bin_dir/$bin" ]; then
      echo "error: binary package requires executable $bin_dir/$bin" >&2
      echo "hint: build first or set SYNAPSE_BIN_DIR=/path/to/release/bin" >&2
      exit 1
    fi
    install -m 0755 "$bin_dir/$bin" "$pkg/bin/$bin"
  done
  cat >"$pkg/BINARY_PACKAGE.md" <<EOF
# Binary Package

Kind: $package_kind
Target: $target_label
Binaries:
- bin/synx
- bin/synapsed
- bin/synapse-mcp

This package is target-specific. Use the default source package for portable
Mac/Linux installs.
EOF
fi

if rg -n "$maintainer_home_re" "$pkg" >/tmp/synapse-package-forbidden.$$ 2>/dev/null; then
  echo "error: package contains forbidden local/private references:" >&2
  cat /tmp/synapse-package-forbidden.$$ >&2
  rm -f /tmp/synapse-package-forbidden.$$
  exit 1
fi
rm -f /tmp/synapse-package-forbidden.$$

if find "$pkg" \( -name 'brain.db' -o -name '*.db-wal' -o -name '*.db-shm' -o -name '.emb-cache' -o -name 'node_modules' -o -name 'file-history' -o -name '.claude' -o -name '.codex' \) | grep . >/tmp/synapse-package-forbidden-files.$$; then
  echo "error: package contains forbidden local data files:" >&2
  cat /tmp/synapse-package-forbidden-files.$$ >&2
  rm -f /tmp/synapse-package-forbidden-files.$$
  exit 1
fi
rm -f /tmp/synapse-package-forbidden-files.$$

(cd "$stage" && find "$package_id" -type f | sort >"$pkg/FILES.txt")
if command -v shasum >/dev/null 2>&1; then
  (cd "$stage" && find "$package_id" -type f -print0 | sort -z | xargs -0 shasum -a 256 >"$pkg/SHA256SUMS")
elif command -v sha256sum >/dev/null 2>&1; then
  (cd "$stage" && find "$package_id" -type f -print0 | sort -z | xargs -0 sha256sum >"$pkg/SHA256SUMS")
fi

if [ "$dry_run" = "1" ]; then
  echo "dry-run package staged at $pkg"
  find "$pkg" -maxdepth 3 -type f | sort
  exit 0
fi

mkdir -p "$out_dir"
tarball="$out_dir/$package_id.tar.gz"
(cd "$stage" && tar -czf "$tarball" "$package_id")
if command -v shasum >/dev/null 2>&1; then
  shasum -a 256 "$tarball" >"$tarball.sha256"
elif command -v sha256sum >/dev/null 2>&1; then
  sha256sum "$tarball" >"$tarball.sha256"
fi
echo "wrote $tarball"
