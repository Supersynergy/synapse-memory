#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
RELEASE_BIN="$REPO_ROOT/target/release"
LAUNCH_AGENTS="$HOME/Library/LaunchAgents"
UID_VAL="$(id -u)"

echo "==> Building release binaries..."
cargo build --release -p synapse-extract --features minimax --manifest-path "$REPO_ROOT/Cargo.toml"

echo "==> Installing binaries to /usr/local/bin..."
sudo cp "$RELEASE_BIN/synapse-extract-worker" /usr/local/bin/synapse-extract-worker
sudo cp "$RELEASE_BIN/synapse-lifecycle" /usr/local/bin/synapse-lifecycle
sudo chmod +x /usr/local/bin/synapse-extract-worker /usr/local/bin/synapse-lifecycle

echo "==> Copying plists to $LAUNCH_AGENTS..."
mkdir -p "$LAUNCH_AGENTS"
cp "$SCRIPT_DIR/launchd/com.supersynergy.synapse.extract.plist" "$LAUNCH_AGENTS/"
cp "$SCRIPT_DIR/launchd/com.supersynergy.synapse.lifecycle.plist" "$LAUNCH_AGENTS/"

echo "==> Bootstrapping launch agents..."
for label in com.supersynergy.synapse.extract com.supersynergy.synapse.lifecycle; do
    plist="$LAUNCH_AGENTS/${label}.plist"
    # Bootout first if already loaded (ignore errors)
    launchctl bootout "gui/${UID_VAL}/${label}" 2>/dev/null || true
    launchctl bootstrap "gui/${UID_VAL}" "$plist"
    echo "  bootstrapped: $label"
done

echo "==> Status check..."
launchctl print "gui/${UID_VAL}/com.supersynergy.synapse.extract" | grep -E "state|pid" || true
launchctl print "gui/${UID_VAL}/com.supersynergy.synapse.lifecycle" | grep -E "state|pid" || true

echo "==> Done. Logs: ~/Library/Logs/synapse-{extract,lifecycle}.log"
