#!/usr/bin/env bash
# setup.sh — WP-CLI install on all 3 backends (parallel)
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
COMPOSE_FILE="$SCRIPT_DIR/docker-compose.yml"

BACKENDS=(a b c)
PORTS=(18081 18082 18083)
CONTAINERS=(wp3_wp_a wp3_wp_b wp3_wp_c)
TITLES=("MySQL8" "Percona8" "Synapse-MySQL")

log() { echo "[$(date +%H:%M:%S)] $*"; }

# ── 0. Start stacks ──────────────────────────────────────────────────────────
log "Starting all 3 stacks..."
docker compose -f "$COMPOSE_FILE" up -d

# ── 1. Wait for synapse-mysql proxy (if binary present) ──────────────────────
SYNAPSE_BIN="$(cd "$SCRIPT_DIR/../.." && pwd)/target/release/synapse-mysql"
if [[ -x "$SYNAPSE_BIN" ]]; then
  log "Starting synapse-mysql proxy on :3309 → upstream :3308..."
  "$SYNAPSE_BIN" \
    --listen 0.0.0.0:3309 \
    --upstream 127.0.0.1:3308 \
    --user wp --password wp --database wordpress_c \
    > /tmp/synapse-mysql.log 2>&1 &
  SYNAPSE_PID=$!
  echo $SYNAPSE_PID > /tmp/synapse-mysql.pid
  sleep 3
  if ! kill -0 $SYNAPSE_PID 2>/dev/null; then
    log "WARN: synapse-mysql proxy failed to start. WP-C will be BLOCKED."
    rm -f /tmp/synapse-mysql.pid
  fi
else
  log "WARN: synapse-mysql binary not found at $SYNAPSE_BIN. WP-C will be BLOCKED."
fi

# ── 2. Wait for all WP containers ───────────────────────────────────────────
log "Waiting for WP containers to be healthy..."
for i in 0 1 2; do
  container="${CONTAINERS[$i]}"
  port="${PORTS[$i]}"
  log "  Waiting for $container (:${port})..."
  for attempt in $(seq 1 40); do
    if curl -sf "http://localhost:${port}/wp-login.php" >/dev/null 2>&1; then
      log "  $container ready."
      break
    fi
    if [[ $attempt -eq 40 ]]; then
      log "  ERROR: $container never became ready. Aborting."
      exit 1
    fi
    sleep 5
  done
done

# ── 3. WP-CLI install (parallel) ─────────────────────────────────────────────
install_wp() {
  local idx=$1
  local container="${CONTAINERS[$idx]}"
  local port="${PORTS[$idx]}"
  local tag="${TITLES[$idx]}"
  local url="http://localhost:${port}"

  log "[$tag] Running WP-CLI install..."

  # Check if already installed
  if docker exec "$container" wp core is-installed --allow-root 2>/dev/null; then
    log "[$tag] Already installed, skipping."
    return 0
  fi

  docker exec "$container" wp core install \
    --allow-root \
    --url="$url" \
    --title="WP Bench ${tag}" \
    --admin_user=admin \
    --admin_password=adminpass \
    --admin_email="bench@example.com" \
    --skip-email \
    2>&1 | sed "s/^/[$tag] /"

  # Default content: hello world post + sample page already exist after install
  # Add 20 more posts for realistic query load
  log "[$tag] Creating 20 posts..."
  for n in $(seq 1 20); do
    docker exec "$container" wp post create \
      --allow-root \
      --post_title="Bench Post ${n}" \
      --post_content="$(cat /dev/urandom | tr -dc 'a-zA-Z ' | head -c 200 2>/dev/null || echo 'Lorem ipsum dolor sit amet consectetur adipiscing elit sed do eiusmod tempor')" \
      --post_status=publish \
      --post_type=post \
      --quiet 2>/dev/null || true
  done

  log "[$tag] Setup complete."
}

pids=()
for i in 0 1 2; do
  install_wp $i &
  pids+=($!)
done

# Wait and collect exit codes
all_ok=true
for i in 0 1 2; do
  if ! wait "${pids[$i]}"; then
    log "WARN: Setup failed for ${TITLES[$i]}"
    all_ok=false
  fi
done

if $all_ok; then
  log "All 3 backends set up successfully."
else
  log "Some backends failed (see BLOCKED notes above). Continuing with available backends."
fi

log "Verify URLs:"
for i in 0 1 2; do
  echo "  ${TITLES[$i]}: http://localhost:${PORTS[$i]}/"
done
