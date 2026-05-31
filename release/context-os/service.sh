#!/usr/bin/env bash
set -euo pipefail

cmd="${1:-install}"
prefix="${SYNAPSE_PREFIX:-$HOME/.local}"
db="${SYNAPSE_DB:-$HOME/.synapse/brain.db}"
sock="${SYNAPSE_SOCK:-/tmp/synapse.sock}"
metrics_addr="${SYNAPSE_METRICS_ADDR:-127.0.0.1:9090}"
live_addr="${SYNAPSE_LIVE_ADDR:-127.0.0.1:9091}"
bin="${SYNAPSED_BIN:-$prefix/bin/synapsed}"
dry_run="${SYNAPSE_SERVICE_DRY_RUN:-0}"
service_os="${SYNAPSE_SERVICE_OS:-$(uname -s)}"
label="com.synapse.context-os"

xml_escape() {
  printf '%s' "$1" \
    | sed -e 's/&/\&amp;/g' -e 's/</\&lt;/g' -e 's/>/\&gt;/g' -e 's/"/\&quot;/g'
}

require_bin() {
  if [ ! -x "$bin" ]; then
    if [ "$dry_run" = "1" ]; then
      echo "dry-run: synapsed not present at $bin; rendering service file only"
      return
    fi
    echo "error: synapsed not executable at $bin; run release/context-os/install.sh first" >&2
    exit 1
  fi
}

install_macos() {
  require_bin
  plist="$HOME/Library/LaunchAgents/$label.plist"
  log_dir="$HOME/Library/Logs/synapse"
  mkdir -p "$(dirname "$plist")" "$log_dir" "$(dirname "$db")"
  cat >"$plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN"
 "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>$label</string>
  <key>ProgramArguments</key>
  <array>
    <string>$(xml_escape "$bin")</string>
    <string>--file</string><string>$(xml_escape "$db")</string>
    <string>--sock</string><string>$(xml_escape "$sock")</string>
    <string>--lazy-embed</string>
    <string>--metrics-addr</string><string>$(xml_escape "$metrics_addr")</string>
    <string>--live-addr</string><string>$(xml_escape "$live_addr")</string>
  </array>
  <key>RunAtLoad</key><true/>
  <key>KeepAlive</key><true/>
  <key>StandardOutPath</key><string>$(xml_escape "$log_dir/synapsed.out.log")</string>
  <key>StandardErrorPath</key><string>$(xml_escape "$log_dir/synapsed.err.log")</string>
  <key>EnvironmentVariables</key>
  <dict>
    <key>SYNAPSE_METRICS_ADDR</key><string>$(xml_escape "$metrics_addr")</string>
    <key>SYNAPSE_LIVE_ADDR</key><string>$(xml_escape "$live_addr")</string>
  </dict>
</dict>
</plist>
EOF
  echo "wrote $plist"
  if [ "$dry_run" = "1" ]; then
    echo "dry-run: not loading launchd service"
    return
  fi
  launchctl bootout "gui/$UID" "$plist" >/dev/null 2>&1 || true
  launchctl bootstrap "gui/$UID" "$plist"
  launchctl kickstart -k "gui/$UID/$label"
  echo "loaded launchd service $label"
}

install_linux() {
  require_bin
  if [ "$dry_run" != "1" ] && ! command -v systemctl >/dev/null 2>&1; then
    echo "error: systemctl not found; run synapsed manually or install systemd user services" >&2
    exit 1
  fi
  unit_dir="$HOME/.config/systemd/user"
  unit="$unit_dir/synapse-context-os.service"
  mkdir -p "$unit_dir" "$(dirname "$db")"
  cat >"$unit" <<EOF
[Unit]
Description=Synapse Context OS daemon
After=network.target

[Service]
Type=simple
ExecStart=$bin --file $db --sock $sock --lazy-embed --metrics-addr $metrics_addr --live-addr $live_addr
Restart=on-failure
RestartSec=2
Environment=SYNAPSE_METRICS_ADDR=$metrics_addr
Environment=SYNAPSE_LIVE_ADDR=$live_addr

[Install]
WantedBy=default.target
EOF
  echo "wrote $unit"
  if [ "$dry_run" = "1" ]; then
    echo "dry-run: not enabling systemd user service"
    return
  fi
  systemctl --user daemon-reload
  systemctl --user enable --now synapse-context-os.service
  echo "enabled systemd user service synapse-context-os.service"
}

uninstall_macos() {
  plist="$HOME/Library/LaunchAgents/$label.plist"
  launchctl bootout "gui/$UID" "$plist" >/dev/null 2>&1 || true
  rm -f "$plist"
  echo "removed $plist"
}

uninstall_linux() {
  unit="$HOME/.config/systemd/user/synapse-context-os.service"
  if command -v systemctl >/dev/null 2>&1; then
    systemctl --user disable --now synapse-context-os.service >/dev/null 2>&1 || true
    systemctl --user daemon-reload >/dev/null 2>&1 || true
  fi
  rm -f "$unit"
  echo "removed $unit"
}

status_macos() {
  launchctl print "gui/$UID/$label" 2>/dev/null || {
    echo "not loaded: $label"
    return 1
  }
}

status_linux() {
  systemctl --user status synapse-context-os.service --no-pager
}

case "$service_os" in
  Darwin)
    case "$cmd" in
      install) install_macos ;;
      uninstall) uninstall_macos ;;
      status) status_macos ;;
      print) SYNAPSE_SERVICE_DRY_RUN=1 dry_run=1 install_macos ;;
      *) echo "usage: $0 install|uninstall|status|print" >&2; exit 2 ;;
    esac
    ;;
  Linux)
    case "$cmd" in
      install) install_linux ;;
      uninstall) uninstall_linux ;;
      status) status_linux ;;
      print) SYNAPSE_SERVICE_DRY_RUN=1 dry_run=1 install_linux ;;
      *) echo "usage: $0 install|uninstall|status|print" >&2; exit 2 ;;
    esac
    ;;
  *)
    echo "error: unsupported OS $(uname -s); run synapsed manually" >&2
    exit 1
    ;;
esac
