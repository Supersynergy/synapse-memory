# Local Dev Setup

## Prerequisites

- Rust stable (see `rust-toolchain.toml`)
- macOS / Linux (Windows: untested)
- `cargo-nextest` for tests: `cargo install cargo-nextest`
- `bacon` for watch mode: `cargo install bacon`

## Build

```bash
# debug (fast iteration)
cargo build

# release (production binary)
cargo build --release -p synapsed

# max-perf single binary (~5 min)
cargo build --profile release-fast -p synapsed
```

## Test

```bash
cargo nextest run                    # all workspace tests
cargo nextest run -p synapse-core    # single crate
cargo test --doc                     # doctests
```

## Run daemon locally

```bash
cargo run -p synapsed -- --socket /tmp/synapse-dev.sock
# in another terminal:
synx --socket /tmp/synapse-dev.sock ping
```

## Bench

```bash
# competitive bench (Python, needs ~/.venvs/synapse-bench)
~/.venvs/synapse-bench/bin/python bench/real_competitors.py

# criterion micro bench
cargo bench -p synapse-kernel
cargo bench -p synapse-core

# WP bench (excluded from default workspace)
cargo bench -p synapse-cms-bench
```

## Lint / Format

```bash
cargo clippy --workspace --all-features -- -D warnings
cargo fmt --check
```

## File Layout

```
crates/                  workspace crates (see ARCHITECTURE.md §2)
synapsestore/crates/     WAL + segment + ultra crates
bench/                   criterion + competitor bench harnesses
docs/                    design docs, dev guides
openspec/                OpenSpec RFC files
scripts/                 release + deploy scripts
sdk/                     Python SDK (synapse-py / PyO3)
integrations/            LangChain / Mem0 / LlamaIndex adapters
```

## Common Pitfalls

| Symptom | Cause | Fix |
|---|---|---|
| `--all-features` build fails | `synapse-embed-gpu` is a standalone workspace | build it separately: `cd crates/synapse-embed-gpu && cargo build` |
| `synapse-edge` not found | Pingora deps have RUSTSEC advisories; crate excluded by default | `cargo build -p synapse-edge` opt-in |
| `bench/wp` build error | mysql driver pulls lru 0.12.5 (RUSTSEC-2026-0002) | `cargo bench -p synapse-cms-bench` explicitly |
| SQLite "database is locked" | Multiple daemon instances on same socket | `rm /tmp/synapse.sock` and restart |
| OOM during fat-LTO build | release-fast uses fat LTO | add `CARGO_BUILD_JOBS=2` or switch to `release` profile |
| `age` decryptor error at runtime | age 0.10 has Decryptor enum→struct API break; snap.rs:294-300 TODO | pending migration; snapshot restore affected only |
