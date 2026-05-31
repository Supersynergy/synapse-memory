#!/usr/bin/env sh
# install.sh — worldwide one-liner for the Synapse Context-OS.
#
#   curl -fsSL https://raw.githubusercontent.com/supersynergy/synapse/main/scripts/install.sh | sh
#
# Order of preference (first that works wins):
#   1. download a prebuilt synapse-mcp binary for your OS/arch from the GitHub release
#      (sha256-verified),
#   2. `cargo install --git` (any machine with cargo),
#   3. build from a local checkout (if run inside the repo).
# Then it re-signs on macOS and registers the server into every agent CLI you have.
#
# Env:
#   SYNAPSE_RELEASE_BASE  base URL for assets (default: latest GitHub release; file:// ok for testing)
#   SYNAPSE_REPO          git repo for the cargo-install fallback
#   PREFIX                install dir (default ~/.local/bin)
#   SYNAPSE_SKIP_REGISTER set to 1 to install the binary only (no CLI registration)
set -eu

REPO="${SYNAPSE_REPO:-https://github.com/supersynergy/synapse}"
RELEASE_BASE="${SYNAPSE_RELEASE_BASE:-${REPO}/releases/latest/download}"
PREFIX="${PREFIX:-$HOME/.local/bin}"
BIN_NAME=synapse-mcp
HERE=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)

log()  { printf '\033[36m›\033[0m %s\n' "$*"; }
warn() { printf '\033[33m!\033[0m %s\n' "$*" >&2; }
die()  { printf '\033[31m✗\033[0m %s\n' "$*" >&2; exit 1; }

SKIP_REG="${SYNAPSE_SKIP_REGISTER:-0}"
for a in "$@"; do
  case "$a" in --no-register) SKIP_REG=1 ;; esac
done

detect_target() {
  os=$(uname -s); arch=$(uname -m)
  case "$os" in
    Darwin) case "$arch" in
      arm64|aarch64) echo "aarch64-apple-darwin" ;;
      x86_64)        echo "x86_64-apple-darwin" ;;
      *) return 1 ;; esac ;;
    Linux) case "$arch" in
      aarch64|arm64) echo "aarch64-unknown-linux-musl" ;;
      x86_64|amd64)  echo "x86_64-unknown-linux-musl" ;;
      *) return 1 ;; esac ;;
    *) return 1 ;;
  esac
}

sha256_of() {
  if command -v sha256sum >/dev/null 2>&1; then sha256sum "$1" | awk '{print $1}'
  elif command -v shasum   >/dev/null 2>&1; then shasum -a 256 "$1" | awk '{print $1}'
  else echo ""; fi
}

fetch() { # fetch <url> <dest> -> 0 on success
  if command -v curl >/dev/null 2>&1; then curl -fsSL "$1" -o "$2" 2>/dev/null
  elif command -v wget >/dev/null 2>&1; then wget -qO "$2" "$1" 2>/dev/null
  else return 1; fi
}

# 1. prebuilt binary for this platform, sha256-verified.
try_prebuilt() {
  target=$(detect_target) || { warn "unsupported OS/arch for prebuilt"; return 1; }
  asset="${BIN_NAME}-${target}"
  tmp=$(mktemp -d)
  log "fetching $asset"
  fetch "${RELEASE_BASE}/${asset}" "${tmp}/${BIN_NAME}" || { warn "no prebuilt at ${RELEASE_BASE}"; return 1; }
  if fetch "${RELEASE_BASE}/SHA256SUMS" "${tmp}/SHA256SUMS"; then
    want=$(grep " ${asset}\$" "${tmp}/SHA256SUMS" 2>/dev/null | awk '{print $1}' | head -1)
    if [ -n "$want" ]; then
      got=$(sha256_of "${tmp}/${BIN_NAME}")
      [ "$want" = "$got" ] || die "checksum mismatch for $asset (want $want got $got)"
      log "sha256 verified"
    fi
  else
    warn "no SHA256SUMS — skipping checksum (set SYNAPSE_RELEASE_BASE to a signed release)"
  fi
  chmod +x "${tmp}/${BIN_NAME}"
  RESOLVED_BIN="${tmp}/${BIN_NAME}"
  return 0
}

# 2. cargo install --git.
try_cargo() {
  command -v cargo >/dev/null 2>&1 || return 1
  log "cargo install --git $REPO $BIN_NAME"
  cargo install --git "$REPO" "$BIN_NAME" --root "${PREFIX%/bin}" 2>/dev/null || return 1
  RESOLVED_BIN="${PREFIX}/${BIN_NAME}"
  return 0
}

# 3. local checkout build.
try_source() {
  [ -f "${HERE}/../Cargo.toml" ] && command -v cargo >/dev/null 2>&1 || return 1
  log "building from checkout (release)"
  ( cd "${HERE}/.." && cargo build --release -p "$BIN_NAME" ) || return 1
  RESOLVED_BIN="${HERE}/../target/release/${BIN_NAME}"
  return 0
}

RESOLVED_BIN=""
try_prebuilt || try_cargo || try_source || die "could not obtain $BIN_NAME (no prebuilt, no cargo, no checkout)"

# install to PREFIX (unless cargo already put it there)
mkdir -p "$PREFIX"
DEST="${PREFIX}/${BIN_NAME}"
abs_bin=$(cd "$(dirname "$RESOLVED_BIN")" && pwd)/$(basename "$RESOLVED_BIN")
if [ "$abs_bin" != "$DEST" ]; then
  cp -f "$RESOLVED_BIN" "$DEST"
fi
chmod +x "$DEST"
log "installed $DEST"

# macOS (Apple Silicon) SIGKILLs a Mach-O whose signature the copy invalidated — re-sign ad-hoc.
if [ "$(uname -s)" = "Darwin" ] && command -v codesign >/dev/null 2>&1; then
  codesign --force --sign - "$DEST" >/dev/null 2>&1 || true
fi

# smoke test: the binary must answer initialize
if printf '{"jsonrpc":"2.0","id":1,"method":"initialize"}\n' | "$DEST" 2>/dev/null | grep -q '"serverInfo"'; then
  log "binary OK (MCP initialize responded)"
else
  warn "binary did not respond to initialize — check it runs: $DEST"
fi

# register into every CLI present
if [ "$SKIP_REG" = "1" ]; then
  log "skipping CLI registration (--no-register)"
elif [ -f "${HERE}/install-ctxos.sh" ]; then
  SYNAPSE_MCP_BIN="$DEST" sh "${HERE}/install-ctxos.sh" install --all
  SYNAPSE_MCP_BIN="$DEST" sh "${HERE}/install-ctxos.sh" doctor
else
  warn "install-ctxos.sh not found; register manually (see docs/CTXOS.md)"
fi

log "done — ask any agent CLI to call context_pack."
