.PHONY: bench-agent-memory demo-agent-memory enterprise-pdf docker-build install-local smoke-fast

SHELL := /bin/bash
PYTHON ?= python3
SYNAPSE_SOCK ?= /tmp/synapse-agent-memory-bench.sock

bench-agent-memory:
	@./bench/recall_bakeoff/run_public.sh

demo-agent-memory:
	@./demo/three-session-bugfix/run.sh

enterprise-pdf:
	@/Users/master/.cache/codex-runtimes/codex-primary-runtime/dependencies/python/bin/python3 docs/enterprise/build_pdf.py

docker-build:
	@docker build -t synapse-agentdb:local .

install-local:
	@./install.sh --local

smoke-fast:
	@./scripts/smoke_fast.sh
