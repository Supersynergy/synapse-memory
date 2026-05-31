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
tmp="$(mktemp -d "${TMPDIR:-/tmp}/synapse-context-os.XXXXXX")"
trap 'rm -rf "$tmp"' EXIT

db="$tmp/brain.db"
project="$tmp/project"
home="$tmp/home"
host_cargo_home="${CARGO_HOME:-$HOME/.cargo}"
host_rustup_home="${RUSTUP_HOME:-$HOME/.rustup}"
mkdir -p "$project" "$home"

cat >"$project/README.md" <<'EOF'
# Context OS Smoke Project

Small public fixture used by the release verifier.
EOF

cat >"$project/Cargo.toml" <<'EOF'
[package]
name = "context-os-smoke"
version = "0.1.0"
edition = "2021"

[dependencies]
serde = "1"
EOF

run_synx() {
  if [ -n "${SYNX_BIN:-}" ]; then
    "$SYNX_BIN" "$@"
  else
    (cd "$repo_root" && cargo run -q -p synapse-cli -- "$@")
  fi
}

require_contains() {
  haystack="$1"
  needle="$2"
  label="$3"
  if ! printf '%s' "$haystack" | grep -F -- "$needle" >/dev/null; then
    echo "FAIL: expected $label to contain: $needle" >&2
    echo "$haystack" >&2
    exit 1
  fi
}

echo "1/12 script syntax"
bash -n "$script_dir/install.sh" "$script_dir/service.sh" "$script_dir/package.sh" "$script_dir/verify.sh"

echo "2/12 service dry install"
HOME="$home" SYNAPSE_SERVICE_OS=Darwin SYNAPSE_SERVICE_DRY_RUN=1 SYNAPSED_BIN="$tmp/bin/synapsed" SYNAPSE_DB="$db" "$script_dir/service.sh" install >/dev/null
HOME="$home" SYNAPSE_SERVICE_OS=Linux SYNAPSE_SERVICE_DRY_RUN=1 SYNAPSED_BIN="$tmp/bin/synapsed" SYNAPSE_DB="$db" "$script_dir/service.sh" install >/dev/null
mac_plist="$home/Library/LaunchAgents/com.synapse.context-os.plist"
linux_unit="$home/.config/systemd/user/synapse-context-os.service"
test -f "$mac_plist"
test -f "$linux_unit"
mac_plist_text="$(sed -n '1,220p' "$mac_plist")"
linux_unit_text="$(sed -n '1,220p' "$linux_unit")"
require_contains "$mac_plist_text" "com.synapse.context-os" "macOS launchd plist"
require_contains "$mac_plist_text" "--lazy-embed" "macOS launchd plist"
require_contains "$linux_unit_text" "ExecStart=$tmp/bin/synapsed --file $db --sock /tmp/synapse.sock --lazy-embed" "Linux systemd unit"
require_contains "$linux_unit_text" "WantedBy=default.target" "Linux systemd unit"

echo "3/12 package dry run"
package_listing="$(SYNAPSE_PACKAGE_DRY_RUN=1 SYNAPSE_RELEASE_OUT="$tmp/dist" "$script_dir/package.sh")"
require_contains "$package_listing" "Cargo.toml" "package listing"
require_contains "$package_listing" "crates/synapse-cli/Cargo.toml" "package listing"
require_contains "$package_listing" "release/context-os/install.sh" "package listing"
fake_bin="$tmp/fake-bin"
mkdir -p "$fake_bin"
for fake in synx synapsed synapse-mcp; do
  printf '#!/usr/bin/env sh\nexit 0\n' >"$fake_bin/$fake"
  chmod +x "$fake_bin/$fake"
done
binary_listing="$(
  SYNAPSE_PACKAGE_DRY_RUN=1 \
    SYNAPSE_PACKAGE_INCLUDE_BIN=1 \
    SYNAPSE_RELEASE_TARGET=test-os-test-arch \
    SYNAPSE_BIN_DIR="$fake_bin" \
    SYNAPSE_RELEASE_OUT="$tmp/dist" \
    "$script_dir/package.sh"
)"
require_contains "$binary_listing" "synapse-context-os-1.0.1-rc.1-test-os-test-arch" "binary package listing"
require_contains "$binary_listing" "BINARY_PACKAGE.md" "binary package listing"
require_contains "$binary_listing" "bin/synx" "binary package listing"
if SYNAPSE_PACKAGE_DRY_RUN=1 \
  SYNAPSE_PACKAGE_INCLUDE_BIN=1 \
  SYNAPSE_RELEASE_TARGET=test-os-test-arch \
  SYNAPSE_BIN_DIR="$tmp/missing-bin" \
  SYNAPSE_RELEASE_OUT="$tmp/dist" \
  "$script_dir/package.sh" >/dev/null 2>&1; then
  echo "FAIL: binary package succeeded without required binaries" >&2
  exit 1
fi

echo "4/12 init"
run_synx -f "$db" init >/dev/null

echo "5/12 remember typed memory"
remember_out="$(run_synx -f "$db" remember --kind decision --title decision/context-os-smoke --no-embed "Synapse Context OS release smoke memory with cited context and feedback.")"
require_contains "$remember_out" "ok remembered" "remember output"

echo "6/12 put public fact"
doc_id="$(run_synx -f "$db" put --title "known-fact: context-os-smoke" --kind fact --source release-smoke --no-embed --text "Freshness-sensitive agent work must use version-aware context before relying on cached memory.")"
case "$doc_id" in
  ''|*[!0-9]*) echo "FAIL: put did not return numeric doc id: $doc_id" >&2; exit 1 ;;
esac

echo "7/12 context pack with route/id/feedback"
context_json="$(run_synx -f "$db" context "context os release smoke freshness feedback" --mode coding --json)"
require_contains "$context_json" '"context_id"' "context json"
require_contains "$context_json" '"route"' "context json"
require_contains "$context_json" '"reward_hint"' "context json"
context_id="$(printf '%s\n' "$context_json" | sed -n 's/.*"context_id": "\([^"]*\)".*/\1/p' | head -1)"
first_id="$(printf '%s\n' "$context_json" | sed -n 's/.*"id": \([0-9][0-9]*\).*/\1/p' | head -1)"
if [ -z "$context_id" ] || [ -z "$first_id" ]; then
  echo "FAIL: could not extract context_id/doc_id" >&2
  echo "$context_json" >&2
  exit 1
fi

echo "8/12 feedback learning path"
feedback_out="$(run_synx -f "$db" feedback "context:$context_id" "$first_id")"
require_contains "$feedback_out" "ok feedback recorded" "feedback output"
learn_out="$(run_synx -f "$db" learn status)"
require_contains "$learn_out" "feedback_entries=1" "learn status"

echo "9/12 prime clean project"
prime_json="$(run_synx -f "$db" prime "$project" --json)"
require_contains "$prime_json" '"project": "project"' "prime json"
require_contains "$prime_json" '"fresh_command"' "prime json"
require_contains "$prime_json" '"context_command"' "prime json"

echo "10/12 fresh-context offline mode"
fresh_out="$(run_synx -f "$db" fresh-context --cwd "$project" --prompt "latest serde api changes" --no-registry)"
require_contains "$fresh_out" "fresh_context" "fresh-context output"

echo "11/12 doctor, safe fix, db verify"
doctor_json="$(run_synx -f "$db" doctor --json)"
require_contains "$doctor_json" '"quick_check": "ok"' "doctor json"
require_contains "$doctor_json" '"private_source_hits": 0' "doctor json"
require_contains "$doctor_json" '"stale_or_generated_source_hits": 0' "doctor json"
require_contains "$doctor_json" '"backup_age_seconds": null' "doctor json"
run_synx -f "$db" doctor --fix >/dev/null

verify_out="$(run_synx -f "$db" db-verify)"
require_contains "$verify_out" "ok verify" "db-verify output"

echo "12/12 package install smoke"
if [ "${SYNAPSE_VERIFY_INSTALL:-0}" = "1" ]; then
  SYNAPSE_RELEASE_OUT="$tmp/dist" "$script_dir/package.sh" >/dev/null
  tar -xzf "$tmp/dist/synapse-context-os-1.0.1-rc.1.tar.gz" -C "$tmp"
  install_home="$tmp/install-home"
  install_prefix="$tmp/install-prefix"
  install_db="$tmp/install-brain.db"
  mkdir -p "$install_home"
  HOME="$install_home" \
    CARGO_HOME="$host_cargo_home" \
    RUSTUP_HOME="$host_rustup_home" \
    SYNAPSE_PREFIX="$install_prefix" \
    SYNAPSE_DB="$install_db" \
    SYNAPSE_BUILD_PROFILE="${SYNAPSE_VERIFY_BUILD_PROFILE:-dev}" \
    CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$repo_root/target}" \
    "$tmp/synapse-context-os-1.0.1-rc.1/install.sh" >/dev/null
  test -x "$install_prefix/bin/synx"
  "$install_prefix/bin/synx" -f "$install_db" doctor --json >/dev/null
else
  echo "skip package install smoke (set SYNAPSE_VERIFY_INSTALL=1)"
fi

echo "PASS context-os release smoke"
