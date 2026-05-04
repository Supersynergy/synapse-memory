#!/bin/bash
set -e

ARCH=$(uname -m)
OS=$(uname -s)
VERSION="0.1.0"
BASE="https://github.com/Supersynergy/synapse/releases/latest/download"

if [ "$OS" = "Darwin" ]; then
  TARBALL="synapse-mysql-aarch64-apple-darwin.tar.gz"
  [ "$ARCH" = "x86_64" ] && TARBALL="synapse-mysql-x86_64-apple-darwin.tar.gz"
else
  TARBALL="synapse-mysql-x86_64-unknown-linux-gnu.tar.gz"
fi

curl -fsSL "$BASE/$TARBALL" | tar -xz -C /usr/local/bin/
mkdir -p /var/lib/synapse

if [ "$OS" = "Linux" ]; then
  # Install systemd unit
  curl -fsSL "$BASE/synapse-mysqld.service" -o /etc/systemd/system/synapse-mysqld.service
  systemctl daemon-reload
  systemctl enable --now synapse-mysqld
elif [ "$OS" = "Darwin" ]; then
  PLIST="/Library/LaunchDaemons/com.supersynergy.synapse-mysql.plist"
  cat > "$PLIST" <<PLIST_EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>com.supersynergy.synapse-mysql</string>
  <key>ProgramArguments</key>
  <array>
    <string>/usr/local/bin/synapse-mysql-async</string>
    <string>-f</string><string>/var/lib/synapse/default.db</string>
    <string>-b</string><string>0.0.0.0:3306</string>
  </array>
  <key>RunAtLoad</key><true/>
  <key>KeepAlive</key><true/>
</dict>
</plist>
PLIST_EOF
  launchctl load "$PLIST"
fi

echo "Synapse MySQL drop-in listening on :3306"
echo ""
echo "Default credentials: user=root  password=synapse  host=127.0.0.1"
echo ""
echo "WordPress: wp config set DB_HOST 127.0.0.1"
echo "Drupal:    \$databases['default']['default']['host'] = '127.0.0.1';"
echo "Joomla:    config.php: \$host = '127.0.0.1';"
