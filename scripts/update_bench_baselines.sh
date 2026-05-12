#!/usr/bin/env bash
# Usage: ./scripts/update_bench_baselines.sh
# Runs critical benches on current checkout, writes bench-dashboard/baselines.json
# Run manually on main after a perf improvement to move the baseline forward.
set -euo pipefail

CRITERION_DIR="target/criterion"
OUT="bench-dashboard/baselines.json"

echo "Compiling benches..."
cargo bench --workspace --no-run 2>/dev/null

echo "Running benches..."
cargo bench --bench rrf_neon     -p synapse-core    2>/dev/null || true
cargo bench --bench i8_dot       -p synapse-kernel  2>/dev/null || true
cargo bench --bench f16_dot      -p synapse-kernel  2>/dev/null || true
cargo bench --bench i8_neon      -p synapse-colbert 2>/dev/null || true
cargo bench --bench bmp_vs_naive -p synapse-splade  2>/dev/null || true

echo "Extracting means → $OUT ..."
python3 - "$CRITERION_DIR" "$OUT" <<'PY'
import json, pathlib, sys

crit = pathlib.Path(sys.argv[1])
out  = pathlib.Path(sys.argv[2])

results = {}
for est in crit.rglob("new/estimates.json"):
    key = "/".join(est.parts[len(crit.parts):-2])
    try:
        data = json.loads(est.read_text())
        results[key] = data["mean"]["point_estimate"]
    except Exception:
        pass

out.parent.mkdir(parents=True, exist_ok=True)
out.write_text(json.dumps(results, indent=2, sort_keys=True))
print(f"Wrote {len(results)} entries to {out}")
PY

echo "Done. Commit bench-dashboard/baselines.json to main."
