# Known issues

Gaps between what Synapse Memory does today and what the docs or a first-time
user might expect. Work is tracked in `bd` (see `.beads/`); the 2026-09-30
audit findings are in `docs/reviews/2026-09-30-audit-welle-plan.md`.

## Platform / transport

- **Windows daemon transport is new.** `synapsed` binds TCP `127.0.0.1:9477`
  (loopback only; `--allow-remote` refuses non-loopback binds) instead of a
  unix socket. `synapse-mcp` reaches it via `-s 127.0.0.1:9477` or
  `SYNAPSE_SOCK`. The wire protocol and token auth are identical to the unix
  path, but the TCP transport has seen far less mileage.
- The unix default `/tmp/synapse.sock` is shared machine-wide. For a private
  brain on a multi-user box, pick a per-user socket, e.g.
  `synapsed -s ~/.synapse/synapse.sock -f ~/.synapse/brain.db`.
- **Windows ARM64 is not supported yet.** `usearch` (via the vendored ANN
  stack) pulls `numkong`, whose C feature probes use ARM NEON types that
  MSVC's `arm_neon.h` does not provide (`float16x8_t`, `bfloat16x8_t`).
  `cargo check` fails in that dependency on `aarch64-pc-windows-msvc`
  upstream — the CI leg stays enabled but marked `allow-failure`. Windows
  on x64 works.

## Performance hot path

Verified in the 2026-09-30 audit (file:line refs in the review doc):

- `Store::put` runs a `SELECT COUNT(*)` per insert — ingest cost grows with
  corpus size.
- `NdArraySearch::add_row` rebuilds the full matrix per write and scans the id
  list linearly.
- The Tantivy leg commits per `put` and again per lexical search.
- Search hydration is N+1 in the lexical and hybrid paths.
- A single `PlMutex<Store>` serializes daemon ops; `put_batch` holds it for the
  whole batch.
- `Store::open` runs synchronously before the socket binds, so daemon startup
  time scales with brain size.
- Two perf asserts (`rabitq-latency`, `put_batch` throughput) are flaky under
  machine load — red gate, green correctness.

## Product surface

- `cargo build -p synapse-cli --no-default-features` (the portable release
  profile) does not currently compile: `crates/synapse-cli/src/main.rs`
  imports `synapse_core::embed::Embedder` unconditionally. Build with default
  features until the cfg-gating lands.
- `remember --supersedes` is referenced in `release/` docs but the flag does
  not exist on the CLI. Supersession today lives at the MCP `context_remember`
  layer (`supersedes` arg) and in the sota pipeline's `superseded_by` column.
- MCP `memory_save` writes docs with `embed: false` — those memories are
  invisible to vector and hybrid recall until re-embedded.
- The MCP `context_pack` LRU cache is not invalidated on writes; a repeated
  identical query can return a pack built before the latest `put`.
- `crates/synapse-core` (FSL-1.1-ALv2) and the vendored MIT copy under
  `vendor/synapse-db/crates/synapse-core` are divergent twins; product crates
  build the vendored one. `crates/synapse-engine` is similar (source-available
  in-tree, MIT vendored).
- Datalog (`graph-datalog` feature in the vendored `synapse-graph`) is
  quadratic past ~100 facts; the supported graph path is the CTE-based
  `why` / `graph_expand` in `synapse-ultra` and the MCP tools.

## Security notes worth knowing

- Daemon auth is on by default via a generated `auth.token` (0600) next to the
  brain DB, but if the token file cannot be written the daemon logs a warning
  and accepts unauthenticated ops. Check `~/.synapse/auth.token` exists.
- The `Sql` op is read-only via a SQLite authorizer; `ATTACH` is allowed only
  for `~/.synapse/tenants/*.db` read-only URIs.
- CRDT federation trusts the self-signed keys carried in peer messages —
  sync only with peers you control.
- `.synx`/`.brainpack` import is a parse path; prefer `synx db-verify` after
  restoring packs from untrusted sources.
