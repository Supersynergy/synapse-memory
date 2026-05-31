default: check-all

check: check-all

test:
    cargo nextest run --workspace

# Retrain the Context-OS learned noise classifier from the brain + feedback signal.
# Writes ~/.synapse/ctxos_noise_model.json, applied natively by synapse-mcp (no runtime Python).
ctxos-train:
    uv run tools/ctxos/train_noise_model.py

lint:
    cargo clippy --workspace -- -D warnings

fmt:
    cargo fmt

audit:
    cargo audit

deny:
    cargo deny check

cov:
    cargo llvm-cov --workspace --html

bench:
    bash bench/e2e_smoke.sh

shear:
    cargo shear --fix

outdated:
    cargo outdated --workspace

flame REC:
    cargo flamegraph --bin {{REC}}

mutants MOD:
    cargo mutants --package {{MOD}}

mutants-ci:
    cargo mutants --package synapse-core --timeout 60 --jobs 2 \
        -- crates/synapse-core/src/db.rs \
           crates/synapse-core/src/crdt.rs \
           crates/synapse-core/src/sign.rs

msrv:
    cargo msrv find

bloat:
    cargo bloat --release --crates

release VER:
    cargo release {{VER}} --execute --no-publish

turbo-daemon:
    python3 tools/turbo/synapse_turbo.py daemon

bench-micro:
    cd bench/comprehensive && ~/.venvs/synapse-bench/bin/python3 micro.py

check-all: test lint fmt audit deny
