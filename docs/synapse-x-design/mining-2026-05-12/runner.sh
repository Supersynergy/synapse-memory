#!/usr/bin/env bash
# 20 ghmax mining queries — 5min cap each via timeout
set +e
cd "$(dirname "$0")"

FLAGS=(--mine 500 --quality-rerank --lang rust --stars-min 30 --pushed ">2025-01-01" --format json --no-ingest)

run() {
  local out=$1; shift
  local q=$1
  if [[ -s "$out" ]]; then
    echo "[skip] $out exists"
    return
  fi
  echo "[run] $out :: $q"
  timeout 300 ghmax "${FLAGS[@]}" "$q" > "$out" 2> "${out%.json}.err"
  local rc=$?
  echo "[done] $out rc=$rc bytes=$(wc -c <"$out" 2>/dev/null || echo 0)"
}

export -f run
export FLAGS

case "$1" in
  q01) run q01.json "embedded columnar database mmap" ;;
  q02) run q02.json "adaptive radix tree ART" ;;
  q03) run q03.json "learned index RMI FITing" ;;
  q04) run q04.json "cranelift JIT filter compile" ;;
  q05) run q05.json "bloom filter cuckoo quotient SIMD" ;;
  q06) run q06.json "RaBitQ vector quantization" ;;
  q07) run q07.json "online learner streaming FTRL" ;;
  q08) run q08.json "Lance Vortex Beacon columnar" ;;
  q09) run q09.json "io_uring fixed file batch read" ;;
  q10) run q10.json "tick database order book delta" ;;
  q11) run q11.json "Apple silicon AMX accelerate cblas" ;;
  q12) run q12.json "MLX zero copy mmap rust" ;;
  q13) run q13.json "MVCC append log snapshot" ;;
  q14) run q14.json "wide simd portable f32x8 NEON" ;;
  q15) run q15.json "MLflow tabular online prediction" ;;
  q16) run q16.json "Welford tdigest online stats" ;;
  q17) run q17.json "BtrBlocks FastPFOR pcodec integer compress" ;;
  q18) run q18.json "Hilbert zorder space filling curve" ;;
  q19) run q19.json "conformal prediction streaming" ;;
  q20) run q20.json "DataFusion plan cache adaptive" ;;
esac
