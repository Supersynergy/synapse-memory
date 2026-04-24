#!/usr/bin/env bash
# run.sh — 10 WP DB use-cases, 3 backends in parallel, median of 5 runs
# Outputs: results.csv + prints markdown table
set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CSV="$SCRIPT_DIR/results.csv"
RUNS=5

BACKENDS=("MySQL8" "Percona8" "Synapse-MySQL")
PORTS=(18081 18082 18083)
CONTAINERS=(wp3_wp_a wp3_wp_b wp3_wp_c)

log() { echo "[$(date +%H:%M:%S)] $*" >&2; }

# ── helpers ──────────────────────────────────────────────────────────────────

# median of N floats (ms)
median() {
  printf '%s\n' "$@" | sort -n | awk '
    {a[NR]=$1} END {
      if (NR%2==1) print a[int(NR/2)+1]
      else printf "%.1f\n", (a[NR/2]+a[NR/2+1])/2
    }'
}

# curl TTFB in ms
curl_ttfb() {
  local url=$1
  curl -so /dev/null -w "%{time_starttransfer}" "$url" 2>/dev/null \
    | awk '{printf "%.0f", $1*1000}'
}

# curl total time in ms
curl_total() {
  local url=$1
  curl -so /dev/null -w "%{time_total}" "$url" 2>/dev/null \
    | awk '{printf "%.0f", $1*1000}'
}

# wp-cli exec time in ms
wp_time() {
  local container=$1; shift
  local t0 t1
  t0=$(date +%s%3N)
  docker exec "$container" wp --allow-root "$@" >/dev/null 2>&1
  local rc=$?
  t1=$(date +%s%3N)
  if [[ $rc -ne 0 ]]; then echo "BLOCKED"; return; fi
  echo $((t1 - t0))
}

# Check if a container is reachable
is_up() {
  local port=$1
  curl -sf --max-time 3 "http://localhost:${port}/" >/dev/null 2>&1
}

# ── init CSV ─────────────────────────────────────────────────────────────────
echo "uc,backend,run,value_ms" > "$CSV"

record() {
  local uc=$1 backend=$2 run=$3 val=$4
  echo "${uc},${backend},${run},${val}" >> "$CSV"
}

# ── pre-flight ───────────────────────────────────────────────────────────────
log "Pre-flight checks..."
STATUS=("UP" "UP" "UP")
for i in 0 1 2; do
  if ! is_up "${PORTS[$i]}"; then
    STATUS[$i]="DOWN"
    log "WARN: ${BACKENDS[$i]} (:${PORTS[$i]}) not reachable — skipping"
  fi
done

# ── per-UC parallel runner ────────────────────────────────────────────────────

run_uc_parallel() {
  local uc_name=$1
  # Each backend runs in a subshell; results go to temp files
  local tmp_dir
  tmp_dir=$(mktemp -d)
  local pids=()

  for i in 0 1 2; do
    (
      [[ "${STATUS[$i]}" == "DOWN" ]] && { echo "DOWN"; exit; }
      run_uc_for_backend "$uc_name" $i
    ) > "$tmp_dir/$i" &
    pids+=($!)
  done

  for i in 0 1 2; do wait "${pids[$i]}" 2>/dev/null || true; done

  # Read results
  declare -a vals
  for i in 0 1 2; do
    vals[$i]=$(cat "$tmp_dir/$i" 2>/dev/null || echo "ERR")
  done
  rm -rf "$tmp_dir"

  # Print row
  printf "| %-32s | %12s | %12s | %16s |\n" \
    "$uc_name" "${vals[0]}" "${vals[1]}" "${vals[2]}"

  # Record CSV
  for i in 0 1 2; do
    if [[ "${vals[$i]}" =~ ^[0-9] ]]; then
      record "$uc_name" "${BACKENDS[$i]}" "median" "${vals[$i]}"
    else
      record "$uc_name" "${BACKENDS[$i]}" "median" "${vals[$i]}"
    fi
  done
}

run_uc_for_backend() {
  local uc=$1
  local i=$2
  local port="${PORTS[$i]}"
  local container="${CONTAINERS[$i]}"
  local base="http://localhost:${port}"
  local samples=()

  case "$uc" in

    "UC1_cold_homepage_ttfb")
      for _ in $(seq 1 $RUNS); do
        # Simulate cold: clear WP object cache via wp-cli
        docker exec "$container" wp --allow-root cache flush >/dev/null 2>&1 || true
        samples+=("$(curl_ttfb "$base/")")
      done
      median "${samples[@]}"
      ;;

    "UC2_warm_homepage_median")
      # Prime cache
      curl -so /dev/null "$base/" 2>/dev/null
      for _ in $(seq 1 $RUNS); do
        samples+=("$(curl_total "$base/")")
      done
      median "${samples[@]}"
      ;;

    "UC3_posts_list_REST")
      for _ in $(seq 1 $RUNS); do
        samples+=("$(curl_total "$base/wp-json/wp/v2/posts?per_page=50")")
      done
      median "${samples[@]}"
      ;;

    "UC4_single_post_fetch")
      for _ in $(seq 1 $RUNS); do
        samples+=("$(curl_total "$base/?p=1")")
      done
      median "${samples[@]}"
      ;;

    "UC5_search_query")
      for _ in $(seq 1 $RUNS); do
        samples+=("$(curl_total "$base/?s=lorem")")
      done
      median "${samples[@]}"
      ;;

    "UC6_wp_option_list")
      for _ in $(seq 1 $RUNS); do
        samples+=("$(wp_time "$container" option list --format=count)")
      done
      # If first sample is BLOCKED, return immediately
      [[ "${samples[0]}" == "BLOCKED" ]] && { echo "BLOCKED"; return; }
      median "${samples[@]}"
      ;;

    "UC7_insert_post")
      local t0 t1
      t0=$(date +%s%3N)
      for _ in $(seq 1 $RUNS); do
        v=$(wp_time "$container" post create \
          --post_title="BenchInsert_$(date +%s%N)" \
          --post_status=publish --post_type=post)
        [[ "$v" == "BLOCKED" ]] && { echo "BLOCKED"; return; }
        samples+=("$v")
      done
      median "${samples[@]}"
      ;;

    "UC8_insert_100_comments")
      # Get post ID 1
      local pid
      pid=$(docker exec "$container" wp --allow-root post list \
        --post_type=post --post_status=publish --format=ids \
        --posts_per_page=1 2>/dev/null | head -1)
      [[ -z "$pid" ]] && { echo "BLOCKED(no post)"; return; }
      local t0 t1
      t0=$(date +%s%3N)
      for n in $(seq 1 100); do
        docker exec "$container" wp --allow-root comment create \
          --comment_post_ID="$pid" \
          --comment_author="BenchUser${n}" \
          --comment_content="Comment ${n} from bench run" \
          --comment_approved=1 \
          --quiet 2>/dev/null || { echo "BLOCKED"; return; }
      done
      t1=$(date +%s%3N)
      echo $((t1 - t0))
      ;;

    "UC9_update_post_meta_100x")
      local pid
      pid=$(docker exec "$container" wp --allow-root post list \
        --post_type=post --post_status=publish --format=ids \
        --posts_per_page=1 2>/dev/null | head -1)
      [[ -z "$pid" ]] && { echo "BLOCKED(no post)"; return; }
      local t0 t1
      t0=$(date +%s%3N)
      for n in $(seq 1 100); do
        docker exec "$container" wp --allow-root post meta update \
          "$pid" "bench_key_${n}" "val_${n}" --quiet 2>/dev/null \
          || { echo "BLOCKED"; return; }
      done
      t1=$(date +%s%3N)
      echo $((t1 - t0))
      ;;

    "UC10_concurrent_reads_ab")
      if ! command -v ab &>/dev/null; then
        echo "SKIP(ab not installed)"
        return
      fi
      # ab -c 8 -n 200, report median from ab output
      local result
      result=$(ab -c 8 -n 200 -q "$base/" 2>/dev/null \
        | awk '/^50%/ {printf "%.0f", $2}')
      echo "${result:-ERR}"
      ;;

  esac
}

# ── main ─────────────────────────────────────────────────────────────────────
echo ""
echo "## WP 3-Way Benchmark — $(date '+%Y-%m-%d %H:%M')"
echo ""
printf "| %-32s | %12s | %12s | %16s |\n" \
  "Use Case" "MySQL8(ms)" "Percona8(ms)" "Synapse-MySQL(ms)"
printf "|%s|%s|%s|%s|\n" \
  "$(printf '%0.s-' {1..34})" "$(printf '%0.s-' {1..14})" \
  "$(printf '%0.s-' {1..14})" "$(printf '%0.s-' {1..18})"

UCS=(
  "UC1_cold_homepage_ttfb"
  "UC2_warm_homepage_median"
  "UC3_posts_list_REST"
  "UC4_single_post_fetch"
  "UC5_search_query"
  "UC6_wp_option_list"
  "UC7_insert_post"
  "UC8_insert_100_comments"
  "UC9_update_post_meta_100x"
  "UC10_concurrent_reads_ab"
)

for uc in "${UCS[@]}"; do
  log "Running $uc..."
  run_uc_parallel "$uc"
done

echo ""
log "Done. Results: $CSV"
