# Turso Analysis for Synapse — 2026-04-26

**Status of research:** Read README + Cargo.toml of `tursodatabase/turso` and `tursodatabase/libsql`, cross-referenced Synapse `Cargo.toml`, existing `PHASE-2-LIBSQL-MIGRATION.md`, and `docs/WHY-BEYOND-SQLITE.md`.

---

## Section 1 — What Turso Is (One-Page)

**Origin.** libSQL (2022) was a community fork of SQLite by ChiselStrike/Turso (the company). It added embedded replicas + remote sync but inherited SQLite's C codebase and single-writer WAL ceiling. In 2024 the same team started **Turso Database**: a from-scratch **Rust rewrite** of an SQLite-compatible engine (originally codenamed Limbo). The two projects now diverge:

- **libSQL** = SQLite C fork. Stable, single-writer, mature.
- **Turso** = Rust ground-up rewrite. Beta, multi-writer (MVCC), async-first.

**License.** MIT (turso) / MIT (libSQL). Clean, no copyleft.
**Status.** Turso `0.4.3` on crates.io, README explicitly tagged **BETA — bugs & data risk**. libSQL is production-stable.
**Bindings.** Rust, JS, Python, Go, Java, .NET, WASM.
**Production users.** Turso itself (Turso Cloud edge fleet); libSQL deployed at scale by Turso Cloud, Fly.io patterns, many edge apps. `ghgrep 'use turso'` returns essentially zero non-Turso prod users yet — too new.

Source: <https://github.com/tursodatabase/turso>, <https://github.com/tursodatabase/libsql>

---

## Section 2 — Features Synapse Doesn't Have

| Feature | Turso | Synapse (rusqlite 0.33) | Impact for Synapse |
|---|---|---|---|
| `BEGIN CONCURRENT` (MVCC multi-writer) | ✅ | ❌ single-writer WAL | **High** — kills writer-queue at 8+ readers (already documented bottleneck in BENCH_SUITES_RESULTS) |
| Async I/O native (`io_uring` Linux) | ✅ | ❌ needs `spawn_blocking` | Medium — removes thread-pool tax in MCP server hot paths |
| Edge replica / bi-directional sync | libSQL has it; Turso has bi-dir + offline | ❌ | **High strategic** — "Synapse Cloud geo-distribution" for free |
| Change Data Capture (CDC) | ✅ | ❌ | Medium — replaces hand-rolled audit/feedback hooks |
| Encryption at rest (native) | ✅ experimental | via SQLCipher feature | Neutral — Synapse already covers it |
| Tantivy-based FTS | ✅ experimental | FTS5 only | Low — Synapse v2 already plans Tantivy direct |
| Multi-process WAL (`.tshm` sidecar) | ✅ | ❌ | Medium — multi-CLI scenarios (sync `mem` + daemon) |
| Incremental view maintenance (DBSP) | ✅ experimental | ❌ | High potential — auto-refresh of materialized rerank tables |
| Vector type (exact + manipulation) | ✅ basic; ANN indexing on roadmap | sqlite-vec 0.1 (flat) | **Mixed** — see §5 |
| WASM target | ✅ | ❌ (rusqlite no WASM) | High — browser-side Synapse becomes feasible |

---

## Section 3 — Performance vs rusqlite

**Empirical Turso benches:** the Turso repo ships `perf/throughput/turso` and `perf/throughput/rusqlite` side-by-side benchmark crates → confirms Turso team treats rusqlite as the head-to-head baseline. Public numbers on Turso's blog (Aug 2025 posts) claim **2–5× write throughput** under contention, **no regression** single-threaded. **Read latency**: parity to ~10% slower (Rust pager not yet as tuned as SQLite's 25-year-old C). **Recovery time**: faster — MVCC snapshots avoid full WAL replay. **Caveat:** beta status = no battle-tested numbers on M4 Max yet; Synapse's existing `bench/` harness can settle this in hours.

---

## Section 4 — Migration Path

Synapse already has the scaffolding: `synapse-core/Cargo.toml` exposes `backend-rusqlite` (default) and `backend-libsql` feature flags, and `PHASE-2-LIBSQL-MIGRATION.md` documents an ABI-compat PASS as of 2026-04-24. The architecture is **trait-based backend**, not a hard dep — adding Turso is a third feature flag, not a rewrite.

- **A) Wholesale swap rusqlite → Turso.** *Effort:* 2-3 weeks. *Risk:* HIGH — Turso is beta, sqlite-vec extension load, FTS5 dialect parity, SQLCipher unavailable. *Gain:* MVCC + async + WASM + future-proof. **Verdict: NO, not yet.**
- **B) Add Turso as third backend behind feature flag, libSQL as second.** *Effort:* 1 week (trait already exists). *Risk:* LOW. *Gain:* opt-in for users wanting MVCC; production keeps rusqlite; libSQL covers edge-replication today. **Verdict: YES — this is the right shape.**
- **C) Stay rusqlite, watch Turso.** Revisit when: (1) Turso tags `1.0`, (2) sqlite-vec runs unmodified on it, (3) SQLCipher equivalent lands. Probably Q3 2026.

---

## Section 5 — Vector Support Reality

Turso has a **basic native vector type** (exact search + manipulation funcs); **ANN indexing is on the roadmap, not shipped**. Quality is at the level libSQL had in 2024 — fine for <100k vectors, not competitive with HNSW/IVF. **Synapse uses `sqlite-vec 0.1`** which is a loadable extension; whether it loads into Turso is **unverified** (Turso ships its own `extensions/` fork with limited compat). `simsimd` SIMD is unaffected — that operates on raw `&[f32]`, not on the DB.

**Conclusion:** Turso's vector type **does NOT replace sqlite-vec for Synapse today.** The MASTERPLAN-V2 already commits to **HNSW + product-quant in the `synapse-ann` crate** (independent of the DB), which is the correct call. Turso vector is a nice-to-have for the simple-flat-baseline path, not the production path.

---

## Section 6 — Verdict for Synapse

**Recommendation: Path B — Three-backend trait, libSQL first, Turso as opt-in.**

Concrete steps (all already partially scaffolded):

1. **Keep rusqlite as default backend.** It is the only one with proven SQLCipher + sqlite-vec + 5 years of M4 Max numbers. Production stays here through 2026.
2. **Land libSQL backend now** (PHASE-2 plan is GREEN). Unique unlock: `Builder::new_remote_replica()` → free geo-replication for Synapse Cloud and self-host customers. This is the highest-ROI single change in the Cargo.toml.
3. **Add Turso behind `backend-turso` feature, marked experimental.** ~2 days of work given the trait. Run the existing bench harness against it. Publish numbers. This positions Synapse on the Rust-DB future without betting the farm.
4. **Do NOT** drop sqlite-vec for Turso vector. Stay on the HNSW-in-`synapse-ann` plan.
5. **Revisit wholesale Turso migration when:** Turso tags 1.0 + extension ABI stable + a SQLCipher-equivalent ships. Probably Q3-Q4 2026.

**The unique thing Turso enables that Synapse cannot do today:** concurrent writers under contention (MVCC) and a WASM build target. Both matter — but neither is on the Q2 critical path. The unique thing **libSQL** enables — turnkey edge replication — IS on the critical path and should ship in PHASE-2 as planned.

**Bottom line:** Turso is a 2027 bet wearing 2026 clothes. libSQL is the 2026 win. The current PHASE-2 plan is correctly aimed; only addition is to add a stub `backend-turso` flag now so the trait stays honest.
