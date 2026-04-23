#!/usr/bin/env bash
# UC2: Multi-agent CRDT-like merge via snap/restore
SYNAPSE=~/projects/synapse/target/release/synapse

for i in 0 1 2; do
  $SYNAPSE -f /tmp/synapse-eval/agent${i}.db put --text "shared knowledge all agents" --title shared --no-embed
  $SYNAPSE -f /tmp/synapse-eval/agent${i}.db put --text "agent${i} private memory lang${i}" --title agent${i} --no-embed
done

$SYNAPSE -f /tmp/synapse-eval/agent0.db snap /tmp/synapse-eval/agent0.brainpack
# NOTE: restore replaces db — not a merge. agent1 data is lost.
$SYNAPSE -f /tmp/synapse-eval/merged.db restore /tmp/synapse-eval/agent0.brainpack

echo "=== merged stats ==="
$SYNAPSE -f /tmp/synapse-eval/merged.db stats
echo "=== rust search (agent0 data) ==="
$SYNAPSE -f /tmp/synapse-eval/merged.db find lang0
