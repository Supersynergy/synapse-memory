#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
cd "$ROOT"

cargo check -p synapsed --bin synapsed --bin synx-fast
make smoke-fast

echo "agentdb-local verify PASS"
