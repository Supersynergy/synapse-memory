#!/usr/bin/env sh
set -eu

repo="${SYNAPSE_REPO:-https://github.com/Supersynergy/synapse}"
prefix="${SYNAPSE_PREFIX:-$HOME/.local}"
db="${SYNAPSE_DB:-$HOME/.synapse/brain.db}"
default_version="1.1.0-rc.1"
version=""
dry_run=0

die() {
  printf 'error: %s\n' "$*" >&2
  exit 1
}

while [ "$#" -gt 0 ]; do
  case "$1" in
    --dry-run) dry_run=1 ;;
    --version)
      [ "$#" -ge 2 ] || die "--version needs a value"
      version="$2"
      shift
      ;;
    --prefix)
      [ "$#" -ge 2 ] || die "--prefix needs a value"
      prefix="$2"
      shift
      ;;
    --db)
      [ "$#" -ge 2 ] || die "--db needs a value"
      db="$2"
      shift
      ;;
    -h|--help)
      printf '%s\n' "usage: install.sh [--dry-run] [--version VERSION] [--prefix DIR] [--db FILE]"
      exit 0
      ;;
    *) die "unknown option: $1" ;;
  esac
  shift
done

detect_target() {
  os=$(uname -s)
  arch=$(uname -m)
  case "$os:$arch" in
    Darwin:arm64|Darwin:aarch64) printf '%s' aarch64-apple-darwin ;;
    Darwin:x86_64) printf '%s' x86_64-apple-darwin ;;
    Linux:x86_64|Linux:amd64) printf '%s' x86_64-unknown-linux-musl ;;
    Linux:aarch64|Linux:arm64) printf '%s' aarch64-unknown-linux-musl ;;
    *) return 1 ;;
  esac
}

sha256_file() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{print $1}'
  elif command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$1" | awk '{print $1}'
  else
    return 1
  fi
}

fetch() {
  if command -v curl >/dev/null 2>&1; then
    curl -fsSL "$1" -o "$2"
  elif command -v wget >/dev/null 2>&1; then
    wget -qO "$2" "$1"
  else
    die "curl or wget is required"
  fi
}

# The test-only override lets the release verifier install the just-built host
# archive on CI. Normal installs always use the platform detector below.
target=${SYNAPSE_TEST_TARGET:-}
if [ -z "$target" ]; then
  target=$(detect_target) || die "unsupported platform: $(uname -s)/$(uname -m)"
fi
case "$target" in
  aarch64-apple-darwin|x86_64-apple-darwin|aarch64-unknown-linux-musl|x86_64-unknown-linux-musl|aarch64-pc-windows-msvc|x86_64-pc-windows-msvc|aarch64-unknown-linux-gnu|x86_64-unknown-linux-gnu) ;;
  *) die "unsupported release target: $target" ;;
esac
asset="synapse-memory-${target}.tar.gz"
if [ -n "${SYNAPSE_RELEASE_BASE:-}" ]; then
  base=${SYNAPSE_RELEASE_BASE%/}
elif [ -n "$version" ]; then
  version=${version#synapse-memory-v}
  version=${version#v}
  base="$repo/releases/download/synapse-memory-v$version"
else
  base="$repo/releases/download/synapse-memory-v$default_version"
fi

if [ "$dry_run" = 1 ]; then
  printf 'target=%s\nasset=%s\narchive=%s/%s\nchecksum=%s/%s.sha256\nprefix=%s\ndb=%s\n' \
    "$target" "$asset" "$base" "$asset" "$base" "$asset" "$prefix" "$db"
  exit 0
fi

tmp=$(mktemp -d "${TMPDIR:-/tmp}/synapse-memory-install.XXXXXX")
trap 'rm -rf "$tmp"' EXIT HUP INT TERM
archive="$tmp/$asset"
checksum="$tmp/$asset.sha256"

fetch "$base/$asset" "$archive" || die "release asset unavailable: $base/$asset"
fetch "$base/$asset.sha256" "$checksum" || die "required checksum unavailable: $base/$asset.sha256"

expected=$(awk 'NF { print $1; exit }' "$checksum")
[ -n "$expected" ] || die "empty checksum sidecar"
actual=$(sha256_file "$archive") || die "sha256sum or shasum is required"
[ "$expected" = "$actual" ] || die "checksum mismatch for $asset"

root="synapse-memory-$target"
listing="$tmp/archive-files.txt"
tar -tzf "$archive" >"$listing"
awk -v root="$root/" '
  index($0, root) != 1 || $0 ~ /(^|\/)\.\.($|\/)/ || $0 ~ /^\// { bad=1 }
  END { exit bad }
' "$listing" || die "archive contains a path outside $root/"

expected="$tmp/archive-expected.txt"
{
  printf '%s\n' "$root/"
  printf '%s\n' "$root/ATTRIBUTIONS.md"
  printf '%s\n' "$root/BUILD-INFO.json"
  printf '%s\n' "$root/LICENSES/"
  printf '%s\n' "$root/LICENSES/FSL-1.1-ALv2.txt"
  printf '%s\n' "$root/LICENSES/MIT.txt"
  printf '%s\n' "$root/NOTICE"
  printf '%s\n' "$root/README.md"
  printf '%s\n' "$root/THIRD-PARTY-LICENSES.html"
  printf '%s\n' "$root/synx"
} | LC_ALL=C sort >"$expected"
LC_ALL=C sort "$listing" >"$tmp/archive-actual.txt"
cmp -s "$expected" "$tmp/archive-actual.txt" || die "archive payload does not match the release manifest"

tar -tvzf "$archive" >"$tmp/archive-types.txt"
awk '
  substr($0, 1, 1) != "d" && substr($0, 1, 1) != "-" { bad=1 }
  END { exit bad }
' "$tmp/archive-types.txt" || die "archive contains a link or unsupported entry type"
tar -xzf "$archive" -C "$tmp"

source_bin="$tmp/$root/synx"
[ -f "$source_bin" ] && [ ! -L "$source_bin" ] || die "archive does not contain a regular $root/synx"
if [ "$(dd if="$source_bin" bs=1 count=2 2>/dev/null || true)" = '#!' ]; then
  die "archive contains a script wrapper instead of a native Rust binary"
fi
magic=$(od -An -tx1 -N4 "$source_bin" | tr -d ' \n')
case "$magic" in
  7f454c46|cffaedfe|feedfacf|cafebabe|bebafeca) ;;
  *) die "archive contains an unrecognized executable: $magic" ;;
esac

mkdir -p "$prefix/bin" "$(dirname "$db")"
dest="$prefix/bin/synx"
if [ -e "$dest" ]; then
  cp -p "$dest" "$dest.previous"
fi
tmp_dest="$dest.new.$$"
cp "$source_bin" "$tmp_dest"
chmod 0755 "$tmp_dest"
mv -f "$tmp_dest" "$dest"

if [ "$(uname -s)" = Darwin ] && command -v codesign >/dev/null 2>&1; then
  codesign --force --sign - "$dest" >/dev/null 2>&1 || true
fi

"$dest" -f "$db" init >/dev/null
"$dest" -f "$db" doctor --json >/dev/null

printf 'installed=%s\ndb=%s\nversion=%s\n' "$dest" "$db" "$("$dest" --version)"
case ":${PATH:-}:" in
  *":$prefix/bin:"*) ;;
  *) printf 'next=export PATH="%s/bin:%s"\n' "$prefix" "\$PATH" ;;
esac
