# Synapse — Roadmap 2026-Q2 → Q4

**Date**: 2026-05-04
**Inputs**: `docs/COMPETITIVE-MATRIX.md` (Top-50), `docs/SPEC-VS-REALITY-2026-05-04.md` (gap audit), `RESULTS.md`, `KNOWN-ISSUES.md`.
**Effort scale**: S = ≤ 2 days · M = 1–2 weeks · L = 3–6 weeks.
**Impact axes**: Recall (LongMemEval R@5) · Perf (latency / throughput) · Feature (capability matrix).

---

## Q2 2026 (next 8 weeks) — Close R@5 gap to 0.85, harden stubs

Theme: make SPEC truthful + ship the recall pipeline.

| # | Item | Effort | Impact (R/P/F) | Notes |
|---|------|:------:|:--------------:|-------|
| 1 | Wire synapse-rerank ONNX cross-encoder into LongMemEval runner | M | R+0.30–0.40 | KNOWN-ISSUES P1 fix; trait + identity already exist (110 LOC). Default model: ms-marco-MiniLM-L-6-v2 |
| 2 | BM25 pre-filter + HyDE query expansion | M | R+0.10–0.15 | SPEC P2 step. Tantivy already in deps; HyDE prompt template + cache |
| 3 | Temporal boost in fusion (synapse-temporal parse_date_range -> score-multiplier) | S | R+0.02–0.05 | Re-uses existing 171 LOC parser |
| 4 | synapse-ann real HNSW (not stub) — finalize UsearchIndex behind default features + add IVF-PQ (PR-A2) | L | P+ scale, F+ | KNOWN-ISSUES stub. Get to 10M-vec local |
| 5 | synapse-wal real WAL (replace 18-LOC stub) — append/replay/truncate, crash-recovery test | L | F+ durability | UC19 crash recovery <60s @ 100M |
| 6 | synapse-seg real segments (replace 18-LOC stub) — fjall-backed L0–L3 size-tiered LSM | L | P+ ingest, F+ | Plan §2.2 |
| 7 | Fix synapse-license test mutex poison — per-test state | S | (test stability) | KNOWN-ISSUES Low-severity |
| 8 | Replace Python adapter FFI loop with synapsed RPC in eval pipeline | M | P+ ~4000× at 76k chunks | RESULTS.md "Honest Gaps": 200ms -> 51us |
| 9 | Add missing bench rows: single-embed ms, 4-thread insert ops/s, saturation curve | S | (truthful SPEC) | SPEC-VS-REALITY items A.2, A.5 |
| 10 | Reconcile config defaults (mmap 256MB->1GB, journal/sync workload-conditional) | S | (doc) | SPEC-VS-REALITY §D drift |
| 11 | Re-scope or extend synapse-temporal to match SPEC bitemporal claim | M | F+ | SPEC drift item B.10 |
| 12 | synapse-engine planner/cache layer or SPEC rewrite | M | P+ or doc | SPEC drift item B.2 |

Q2 exit criteria:
- LongMemEval R@5 >= 0.70 (P1+P2 combined)
- 0 stub crates in [workspace] active list (or removed)
- SPEC-VS-REALITY green-rate >= 75%
- Python adapter p50 < 5ms at 76k chunks

---

## Q3 2026 — Multi-device + audit-grade

Theme: own the CRDT + signing + federate axis no competitor has.

| # | Item | Effort | Impact | Notes |
|---|------|:------:|:------:|-------|
| 13 | Surface CRDT merge ops in CLI + MCP + Python (synx merge, synapse_merge) | M | F++ | crdt.rs (101) + federate.rs (462) impls exist; need user-facing API |
| 14 | ed25519 author-signing per-Drawer + verification in synx find | M | F++ | sign.rs (84 LOC) ready; wire into write/read paths |
| 15 | Audit-trail export (signed-chain JSON + replay tool) | M | F+ compliance | Differentiates from Pinecone/Qdrant/Weaviate |
| 16 | Multi-device sync demo: laptop + phone (via synapse-py mobile wheel) | L | F+ | Showcase use case |
| 17 | LongMemEval R@5 -> 0.85 (close gap with KG entity extract + space_evolve) | M | R+0.05–0.10 | SPEC P3 step |
| 18 | Cross-encoder cascade reranker tuning (CascadeReranker exists at synapse-rerank/cascade.rs) | S | R+0.02 | Two-stage cheap->expensive |
| 19 | Synapse-as-MCP-skill packaging (single cargo install synapse-mcp, register in Claude/Cursor) | S | F+ adoption | MCP ecosystem play |
| 20 | Bench shootout vs Zep + Letta + mem0 (KG memory tier) | M | (positioning) | Update RESULTS.md with KG-memory competitor row |

Q3 exit criteria:
- All 8 capability columns green in COMPETITIVE-MATRIX (CRDT + Sign user-facing, not just impl)
- LongMemEval R@5 >= 0.85 hit
- Beat Zep/Letta on either query latency OR R@5 in published shootout

---

## Q4 2026 — Optional distributed mode (close last gap vs Qdrant)

Theme: selectively close the one gap users actually demand: multi-node sharding for >10M-vec teams.

| # | Item | Effort | Impact | Notes |
|---|------|:------:|:------:|-------|
| 21 | gRPC peer-sync protocol on top of synapse-wal | L | F+ | Optional feature flag, NOT default. Stays embedded-first |
| 22 | Consistent-hash shard router (re-use synapse-seg key range) | L | P+ scale | Builds on Q2 #6 |
| 23 | Read-replica failover (raft-lite or simple primary+follower) | L | F+ | Audit-grade replication |
| 24 | Dashboard / GUI (read-only Tauri app) | M | (ecosystem) | Closes "no GUI" loss vs pgvector ecosystem |
| 25 | Postgres FDW or sidecar adapter (pg_synapse) | M | F+ adoption | Lets pgvector users adopt Synapse without leaving Postgres |
| 26 | DiskANN-style backend behind synapse-ann trait | L | P+ scale 100M+ | Closes ANN-scale loss row B.5 / D.5 of competitive matrix |

Q4 exit criteria:
- Distributed mode passes 3-node 10M-vec consistency test (optional feature)
- Synapse appears in >= 1 community comparison vs Qdrant/Milvus

---

## Continuous (every sprint)

| Cadence | Item |
|---------|------|
| Weekly | MemPalace shootout re-run; check no regression on insert/query/R@5 |
| Weekly | cargo bench -p bench-space-vs-chroma (FTS criterion) — alert on >10% drift |
| Monthly | Auto-tune sweep re-run; re-derive BEST_CONFIG.json per current corpus shape |
| Monthly | ghgrep "ai memory mcp" + "agent memory framework" — refresh COMPETITIVE-MATRIX |
| Per-PR | SPEC-VS-REALITY audit: any SPEC change must update green/yellow/red status |
| Per-release | Update KNOWN-ISSUES.md; never let stubs leak into "1.0 stable" claims |

---

## Top-3 priorities (ranked by impact / effort)

1. Q2 #1 — Wire synapse-rerank into LongMemEval (M, R+0.30–0.40). Single biggest recall lift; code already exists. Do this first.
2. Q2 #8 — Replace Python FFI loop with synapsed RPC (M, P+ ~4000× at 76k chunks). Closes the most embarrassing perf number.
3. Q3 #13–#15 — Surface CRDT + signing user-facing (M+M+M). The single defensible moat against the entire row 2–50 — and ed25519 + yrs impls already exist; just needs API surface.
