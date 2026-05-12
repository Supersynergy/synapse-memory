#!/bin/bash
set -e
# Requires an active venv: uv venv .venv && source .venv/bin/activate
uv pip install -e ./synapse_vectorbt
uv pip install -e ./synapse_qlib
uv pip install -e ./synapse_nautilus
echo "all 3 adapters installed"
