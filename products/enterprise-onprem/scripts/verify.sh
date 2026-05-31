#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
cd "$ROOT"

test -s products/enterprise-onprem/files/Dockerfile
test -s products/enterprise-onprem/files/docs/enterprise/SYNAPSE_ENTERPRISE_SECURITY.md
test -s products/enterprise-onprem/files/docs/enterprise/SYNAPSE_ENTERPRISE_SECURITY.pdf

if [[ "${RUN_DOCKER:-0}" == "1" ]]; then
  docker build -t synapse-agentdb-enterprise:local .
fi

echo "enterprise-onprem verify PASS"
