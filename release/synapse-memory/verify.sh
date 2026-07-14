#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd "$script_dir/../.." && pwd)"
bin="${SYNX_BIN:-$repo_root/target/debug/synx}"

fail() {
  echo "FAIL $*" >&2
  exit 1
}

require_text() {
  case "$1" in
    *"$2"*) ;;
    *) fail "$3 missing: $2" ;;
  esac
}

echo "1/15 script syntax"
bash -n "$script_dir/audit.sh" "$script_dir/licenses.sh" "$script_dir/install.sh" "$script_dir/uninstall.sh" "$script_dir/package.sh" "$script_dir/verify.sh" "$script_dir/fresh-snapshot.sh"
if command -v pwsh >/dev/null 2>&1; then
  pwsh -NoProfile -Command "[scriptblock]::Create((Get-Content -Raw '$script_dir/install.ps1')) | Out-Null; [scriptblock]::Create((Get-Content -Raw '$script_dir/uninstall.ps1')) | Out-Null; [scriptblock]::Create((Get-Content -Raw '$script_dir/package.ps1')) | Out-Null"
fi

release_version="$(tr -d '[:space:]' <"$script_dir/VERSION")"
workspace_version="$(awk '
  $0 == "[workspace.package]" { in_package = 1; next }
  /^\[/ { in_package = 0 }
  in_package && $1 == "version" { gsub(/[\"[:space:]]/, "", $3); print $3; exit }
' "$repo_root/Cargo.toml")"
sh_default="$(sed -n 's/^default_version="\([^"]*\)"/\1/p' "$script_dir/install.sh")"
ps_default="$(sed -n 's/^[$]DefaultVersion = "\([^"]*\)"/\1/p' "$script_dir/install.ps1")"
[ -n "$release_version" ] || fail "VERSION is empty"
[ "$release_version" = "$workspace_version" ] || fail "VERSION ($release_version) != workspace version ($workspace_version)"
[ "$release_version" = "$sh_default" ] || fail "VERSION ($release_version) != install.sh default ($sh_default)"
[ "$release_version" = "$ps_default" ] || fail "VERSION ($release_version) != install.ps1 default ($ps_default)"
[ -f "$repo_root/Cargo.lock" ] || fail "Cargo.lock missing"
if git -C "$repo_root" check-ignore -q Cargo.lock; then
  fail "Cargo.lock is ignored"
fi

echo "2/15 locked dependency fetch"
# cargo-about resolves the full locked workspace metadata even though this
# release only ships the feature-resolved synapse-cli closure. Fetch first so
# the subsequent offline license proof is deterministic on fresh runners.
cargo fetch --locked --manifest-path "$repo_root/Cargo.toml"

echo "3/15 portable dependency licenses"
"$script_dir/licenses.sh" check

echo "4/15 portable dependency policy"
cargo deny --manifest-path "$repo_root/crates/synapse-cli/Cargo.toml" \
  --locked --no-default-features --exclude-dev \
  -t aarch64-apple-darwin \
  -t x86_64-apple-darwin \
  -t aarch64-unknown-linux-musl \
  -t x86_64-unknown-linux-musl \
  -t aarch64-pc-windows-msvc \
  -t x86_64-pc-windows-msvc \
  check --config "$script_dir/deny.toml" -D warnings

echo "5/15 portable RustSec closure"
"$script_dir/audit.sh"

echo "6/15 native portable binary"
[ -x "$bin" ] || fail "binary is not executable: $bin"
[ "$(dd if="$bin" bs=1 count=2 2>/dev/null || true)" != '#!' ] || fail "binary is a script wrapper"
"$bin" --version

tmp="$(mktemp -d "${TMPDIR:-/tmp}/synapse-memory-verify.XXXXXX")"
trap 'rm -rf "$tmp"' EXIT
home="$tmp/home"
db="$home/.synapse/brain.db"
mkdir -p "$home" "$tmp/project"

echo "7/15 init and typed memory"
HOME="$home" "$bin" -f "$db" init >/dev/null
remember_out="$(HOME="$home" "$bin" -f "$db" remember --kind decision "Portable Synapse ships one verified Rust binary.")"
require_text "$remember_out" "ok remembered" "remember"

echo "8/15 bounded cited context"
context_json="$(HOME="$home" "$bin" -f "$db" context "verified Rust binary" --mode coding --json)"
require_text "$context_json" '"context_id"' "context"
require_text "$context_json" '"route": "lexical"' "portable context route"
context_id="$(printf '%s\n' "$context_json" | sed -n 's/.*"context_id": "\([^"]*\)".*/\1/p' | head -1)"
doc_id="$(printf '%s\n' "$context_json" | sed -n 's/.*"id": \([0-9][0-9]*\).*/\1/p' | head -1)"
[ -n "$context_id" ] && [ -n "$doc_id" ] || fail "context ids missing"

echo "9/15 feedback loop"
HOME="$home" "$bin" -f "$db" feedback "context:$context_id" "$doc_id" >/dev/null
learn_out="$(HOME="$home" "$bin" -f "$db" learn status)"
require_text "$learn_out" "feedback_entries=1" "learn status"

echo "10/15 offline freshness and doctor"
printf '[package]\nname = "portable-smoke"\nversion = "0.1.0"\n\n[dependencies]\nserde = "1"\n' >"$tmp/project/Cargo.toml"
fresh_out="$(HOME="$home" "$bin" -f "$db" fresh-context --cwd "$tmp/project" --prompt "current local dependencies" --no-registry)"
require_text "$fresh_out" "fresh_context" "fresh context"
doctor_json="$(HOME="$home" "$bin" -f "$db" doctor --json)"
require_text "$doctor_json" '"quick_check": "ok"' "doctor"
require_text "$doctor_json" '"semantic_enabled": false' "portable doctor profile"

echo "11/15 backup restore integrity"
pack="$tmp/brain.synx"
restored="$tmp/restored.db"
HOME="$home" "$bin" -f "$db" backup "$pack" >/dev/null
HOME="$home" "$bin" -f "$restored" db-restore "$pack" >/dev/null
verify_out="$(HOME="$home" "$bin" -f "$restored" db-verify)"
require_text "$verify_out" "1 docs clean" "restored db verify"

echo "12/15 package native archive"
target="$(rustc -vV | sed -n 's/^host: //p')"
dist="$tmp/dist"
if SYNAPSE_BIN="$script_dir/install.sh" SYNAPSE_TARGET="$target" SYNAPSE_RELEASE_OUT="$dist" SYNAPSE_ALLOW_DIRTY=1 "$script_dir/package.sh" >/dev/null 2>&1; then
  fail "package accepted a script wrapper"
fi
SYNAPSE_BIN="$bin" SYNAPSE_TARGET="$target" SYNAPSE_RELEASE_OUT="$dist" SYNAPSE_ALLOW_DIRTY=1 "$script_dir/package.sh" >/dev/null
asset="$dist/synapse-memory-$target.tar.gz"
[ -f "$asset" ] && [ -f "$asset.sha256" ] || fail "package assets missing"
archive_list="$(tar -tzf "$asset")"
require_text "$archive_list" "LICENSES/FSL-1.1-ALv2.txt" "FSL license payload"
require_text "$archive_list" "LICENSES/MIT.txt" "MIT license payload"
require_text "$archive_list" "THIRD-PARTY-LICENSES.html" "dependency license payload"
require_text "$archive_list" "NOTICE" "notice payload"
require_text "$archive_list" "ATTRIBUTIONS.md" "attribution payload"

echo "13/15 install from checksummed local release"
install_home="$tmp/install-home"
install_prefix="$install_home/.local"
install_db="$install_home/.synapse/brain.db"
mkdir -p "$install_prefix/bin"
printf '#!/bin/sh\necho private-wrapper\n' >"$install_prefix/bin/synx"
chmod 0755 "$install_prefix/bin/synx"
HOME="$install_home" SYNAPSE_TEST_TARGET="$target" SYNAPSE_RELEASE_BASE="file://$dist" SYNAPSE_PREFIX="$install_prefix" SYNAPSE_DB="$install_db" "$script_dir/install.sh" >/dev/null
installed="$install_prefix/bin/synx"
[ -x "$installed" ] || fail "installed binary missing"
[ "$(dd if="$installed" bs=1 count=2 2>/dev/null || true)" != '#!' ] || fail "installed script wrapper"
[ "$(dd if="$installed.previous" bs=1 count=2 2>/dev/null || true)" = '#!' ] || fail "installer did not preserve the replaced wrapper"
HOME="$install_home" "$installed" -f "$install_db" remember --kind fact "Local archive install passed." >/dev/null
installed_context="$(HOME="$install_home" "$installed" -f "$install_db" context "archive install" --json)"
require_text "$installed_context" '"route": "lexical"' "installed context"

bad_dist="$tmp/bad-dist"
cp -R "$dist" "$bad_dist"
printf '%064d  %s\n' 0 "$(basename "$asset")" >"$bad_dist/$(basename "$asset").sha256"
if HOME="$tmp/bad-home" SYNAPSE_TEST_TARGET="$target" SYNAPSE_RELEASE_BASE="file://$bad_dist" SYNAPSE_PREFIX="$tmp/bad-prefix" SYNAPSE_DB="$tmp/bad.db" "$script_dir/install.sh" >/dev/null 2>&1; then
  fail "installer accepted a bad checksum"
fi

link_dist="$tmp/link-dist"
mkdir -p "$link_dist"
link_asset="$link_dist/$(basename "$asset")"
python3 - "$asset" "$link_asset" "$target" <<'PY'
import pathlib
import sys
import tarfile

source = pathlib.Path(sys.argv[1])
destination = pathlib.Path(sys.argv[2])
target = sys.argv[3]
binary = f"synapse-memory-{target}/synx"
with tarfile.open(source, "r:gz") as incoming, tarfile.open(destination, "w:gz") as outgoing:
    for member in incoming.getmembers():
        if member.name == binary:
            link = tarfile.TarInfo(member.name)
            link.type = tarfile.SYMTYPE
            link.linkname = "/bin/sh"
            link.mode = 0o777
            outgoing.addfile(link)
            continue
        payload = incoming.extractfile(member) if member.isfile() else None
        outgoing.addfile(member, payload)
PY
if command -v sha256sum >/dev/null 2>&1; then
  (cd "$link_dist" && sha256sum "$(basename "$link_asset")" >"$(basename "$link_asset").sha256")
else
  (cd "$link_dist" && shasum -a 256 "$(basename "$link_asset")" >"$(basename "$link_asset").sha256")
fi
if HOME="$tmp/link-home" SYNAPSE_TEST_TARGET="$target" SYNAPSE_RELEASE_BASE="file://$link_dist" SYNAPSE_PREFIX="$tmp/link-prefix" SYNAPSE_DB="$tmp/link.db" "$script_dir/install.sh" >/dev/null 2>&1; then
  fail "installer accepted a checksummed symlink binary"
fi

echo "14/15 safe uninstall preserves memory"
HOME="$install_home" SYNAPSE_PREFIX="$install_prefix" SYNAPSE_DB="$install_db" "$script_dir/uninstall.sh" >/dev/null
[ ! -e "$installed" ] || fail "uninstall left binary"
[ -f "$install_db" ] || fail "uninstall removed user memory"

echo "15/15 Codex disconnect recovery adapter"
python3 "$repo_root/integrations/codex/hooks/test_checkpoint.py"

echo "PASS synapse-memory portable release"
