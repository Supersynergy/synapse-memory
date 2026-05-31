# AgentDB Local Manifest

## Copied Release Files

- `files/install.sh`
- `files/Dockerfile`
- `files/docker-compose.yml`
- `files/scripts/smoke_fast.sh`
- `files/dist/homebrew/*`
- `files/packaging/homebrew/*`

## Canonical Source Files

- `crates/synapsed/src/main.rs`
- `crates/synapsed/src/proto.rs`
- `crates/synapsed/src/bin/synx_fast.rs`
- `crates/synapse-cli/src/main.rs`
- `crates/synapse-core/src/db.rs`
- `install.sh`
- `Dockerfile`
- `scripts/smoke_fast.sh`
- `Makefile`

## Release Commands

```bash
cargo build --release -p synapse-cli --bin synx
cargo build --release -p synapsed --bin synapsed --bin synx-fast
make smoke-fast
docker build -t synapse-agentdb:local .
```

## Key Strings

- `synx-fast doctor`
- `synx-fast find "query" --scope PROJECT`
- `synx-fast context --scope PROJECT "query" --budget 600`
- `printf 'q1\nq2\n' | synx-fast batch hybrid --scope PROJECT --limit 8`
- `SYNAPSE_SOCK=/tmp/synapse.sock`
