#!/usr/bin/env bash
set -euo pipefail

cd "$(git rev-parse --show-toplevel)"

FAILED=0
run() {
    echo "==> $*"
    "$@" || { echo "FAILED: $*"; FAILED=1; }
}

run cargo update --dry-run
run cargo audit
run cargo semver-checks check-release
run cargo deny check
run cargo nextest run --workspace
run cargo bloat --release --crates 2>&1 | head -20
run bash bench/e2e_smoke.sh

if [ $FAILED -ne 0 ]; then
    echo ""
    echo "Pre-release checks FAILED — do not tag."
    exit 1
fi

echo ""
echo "All pre-release checks passed. Safe to tag."
