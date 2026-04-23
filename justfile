default: check-all

test:
    cargo nextest run --workspace

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

check-all: test lint fmt audit deny
