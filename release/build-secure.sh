#!/usr/bin/env bash
# build-secure.sh — Synapse hardened multi-target build with per-customer watermark.
#
# Required env:
#   CUSTOMER_ID         opaque short id, e.g. "cust_a1b2c3"
#   SIGNING_KEY_PATH    path to ed25519 private key (raw 32B or PEM)
#   APPLE_IDENTITY      (mac only) "Developer ID Application: ..."
# Optional:
#   TARGETS             space-separated rust target triples
#   OUT_DIR             output dir (default release/dist)
#
# Idempotent. Exits non-zero on any failure.

set -Eeuo pipefail
IFS=$'\n\t'

: "${CUSTOMER_ID:?CUSTOMER_ID required}"
: "${SIGNING_KEY_PATH:?SIGNING_KEY_PATH required}"

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
OUT_DIR="${OUT_DIR:-$ROOT/release/dist/$CUSTOMER_ID}"
PROFILE="release-secure"
BIN_NAME="${BIN_NAME:-synapse}"

DEFAULT_TARGETS="aarch64-apple-darwin x86_64-unknown-linux-gnu aarch64-unknown-linux-gnu"
TARGETS="${TARGETS:-$DEFAULT_TARGETS}"

mkdir -p "$OUT_DIR"
cd "$ROOT"

log() { printf '[build-secure] %s\n' "$*" >&2; }
fail() { printf '[build-secure][FATAL] %s\n' "$*" >&2; exit 1; }

# Tools
command -v cargo          >/dev/null || fail "cargo missing"
command -v cargo-zigbuild >/dev/null 2>&1 || fail "install: cargo install --locked cargo-zigbuild && brew install zig"
command -v sha256sum >/dev/null 2>&1 || SHA256="shasum -a 256"
SHA256="${SHA256:-sha256sum}"
command -v rsign2   >/dev/null || command -v minisign >/dev/null || \
  command -v signify >/dev/null || fail "need rsign2/minisign/signify for ed25519"

SIGNER=""
for c in rsign2 minisign signify; do command -v $c >/dev/null && SIGNER=$c && break; done

# Step 1: clean to guarantee deterministic output
log "cargo clean (profile=$PROFILE)"
cargo clean --profile "$PROFILE" || true

# Step 2: ensure targets installed
for t in $TARGETS; do
  rustup target add "$t" >/dev/null 2>&1 || log "rustup target add $t skipped"
done

# Step 3: per-target build
for TARGET in $TARGETS; do
  log "building $BIN_NAME for $TARGET"
  CARGO_CMD="cargo build"
  case "$TARGET" in
    *linux-gnu) CARGO_CMD="cargo zigbuild" ;;
  esac
  RUSTFLAGS="-C link-arg=-s -D warnings --remap-path-prefix=$HOME=/build" \
    SYNAPSE_CUSTOMER_ID="$CUSTOMER_ID" \
    $CARGO_CMD --profile "$PROFILE" --target "$TARGET" --bin "$BIN_NAME" \
    || fail "cargo build failed for $TARGET"

  SRC="target/$TARGET/$PROFILE/$BIN_NAME"
  [ -f "$SRC" ] || fail "binary not produced at $SRC"

  STAGE="$OUT_DIR/$TARGET"
  mkdir -p "$STAGE"
  DST="$STAGE/$BIN_NAME"
  cp "$SRC" "$DST"

  # Step 4: strip (extra belt-and-braces; profile already strips)
  case "$TARGET" in
    *apple-darwin) strip -x "$DST" || true ;;
    *linux-gnu)    strip --strip-all "$DST" || true ;;
  esac

  # Step 5: bake watermark via objcopy --add-section (linux) or otool/codesign resource (mac)
  WM="$STAGE/.watermark.bin"
  printf 'SYNAPSE-WM\0%s\0%s\0%s\n' \
    "$CUSTOMER_ID" \
    "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
    "$(git -C "$ROOT" rev-parse --short HEAD 2>/dev/null || echo nogit)" > "$WM"

  case "$TARGET" in
    *linux-gnu)
      command -v objcopy >/dev/null || fail "objcopy required for linux watermark"
      objcopy --add-section .synwm="$WM" --set-section-flags .synwm=noload,readonly \
        "$DST" "$DST.tmp" && mv "$DST.tmp" "$DST"
      ;;
    *apple-darwin)
      # mach-o: append as __TEXT,__synwm via ld is build-time only; embed as xattr + extra resource file
      xattr -w com.synapse.watermark "$(base64 < "$WM" | tr -d '\n')" "$DST" || true
      cp "$WM" "$STAGE/watermark.txt"
      ;;
  esac

  # Step 6: codesign (mac)
  if [[ "$TARGET" == *apple-darwin && -n "${APPLE_IDENTITY:-}" ]]; then
    log "codesign $DST"
    codesign --force --options runtime --timestamp \
      --sign "$APPLE_IDENTITY" "$DST" || fail "codesign failed"
    codesign --verify --strict --verbose=2 "$DST" || fail "codesign verify failed"
  fi

  # Step 7: sha256 + ed25519 sign
  ( cd "$STAGE" && $SHA256 "$BIN_NAME" > "$BIN_NAME.sha256" )
  log "ed25519 sign via $SIGNER"
  case "$SIGNER" in
    rsign2)   rsign2 sign -s "$SIGNING_KEY_PATH" -x "$DST.sig" "$DST" ;;
    minisign) minisign -Sm "$DST" -s "$SIGNING_KEY_PATH" -x "$DST.sig" ;;
    signify)  signify -S -s "$SIGNING_KEY_PATH" -m "$DST" -x "$DST.sig" ;;
  esac

  log "OK $TARGET -> $DST"
done

# Step 8: manifest
MAN="$OUT_DIR/manifest.json"
{
  printf '{\n  "customer_id": "%s",\n' "$CUSTOMER_ID"
  printf '  "built_at": "%s",\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  printf '  "git": "%s",\n' "$(git -C "$ROOT" rev-parse HEAD 2>/dev/null || echo nogit)"
  printf '  "targets": ['
  first=1
  for t in $TARGETS; do
    [ $first -eq 1 ] || printf ','
    first=0
    printf '\n    {"triple":"%s","sha256":"%s"}' \
      "$t" "$($SHA256 "$OUT_DIR/$t/$BIN_NAME" | awk '{print $1}')"
  done
  printf '\n  ]\n}\n'
} > "$MAN"

log "manifest -> $MAN"
log "DONE"
