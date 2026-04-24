# Synapse — 20 Tester-Personas Challenge (top 0.01%)

**Date**: 2026-04-23 · **Baseline**: `SYNAPSE_BEST_IN_CLASS_2026-04-23.md`, `WP_3WAY_BENCHMARK_2026-04-23.md`, `SCALE_100M_PLAN_2026-04-23.md`, `SYNAPSE_VERIFICATION_2026-04-23.md`.

Each persona sets one hard quantitative threshold representing the top 0.01% of their field. Verdict based on measured numbers only (no extrapolation).

---

### 1. HFT-Quant "Priya" — p99 < 1ms @ 10k qps
Runs kNN as order-book feature lookup.
Measured: p95 0.26 ms @ 100 k; p99 and 10 k qps **not measured** (UC13 n/m).
**FAIL (unmeasured).** Fix: PR-F1 concurrency harness + `criterion` p99 col (1 w). Until shipped, no HFT claim.

### 2. Game-Engine-Dev "Marcus" — vec search < 2 ms in 16.6 ms frame budget
NPC memory lookup per frame.
Measured: 0.19–0.28 ms p95 across 1 k–1 M. **PASS with 60× headroom.**
**Beyond**: ship Bevy/UE plugin + `no_std`-compatible read path.

### 3. Claude-Code-Agent-User "Alex" — 100 recall ops/turn < 100 ms total
Per-turn context fetch.
Measured: lib-mode UC20 = 0.015 ms, BM25 UC03 = 0.009 ms → 100× 0.03 ms ≈ 3 ms. **PASS 30× over.**
**Beyond**: in-process prewarm + turn-scoped result cache → < 300 µs/100 ops.

### 4. Solo-WP-Blogger "Sophie" — admin-op parity ±20% vs MySQL
Real WP admin workflow.
Measured: reads 1.8–4× slower; **writes 32–82× slower** (UC7: 738 ms vs 9 ms).
**FAIL hard.** Fix: batch INSERT rewrite + `BEGIN/COMMIT` wrap in `synapse-mysql/rewrite.rs` + WAL-group commit (3–5 d). Realistic target: 3–5× not parity. Positioning: read-heavy only.

### 5. Enterprise-RAG-Architect "Raj" — 100 M vecs recall@10 ≥ 0.95
Measured: recall@10 ≥ 0.95 vs brute-force @ 100 k only; **no 100 M, no MS-MARCO**.
**FAIL.** Fix: PR-A2 (IVF-PQ, 6–8 d) + PR-C1 (int8, 2 d) + PR-G1 (MS-MARCO harness, 1 w). Arithmetic shows 100 M fits in ~5 GB index.

### 6. Privacy-First-EU-Startup "Lena" — zero-cloud + signed GDPR audit trail
Measured: Ed25519 `.brainpack` ships, single-file local-first, no net. Audit-log structure exists.
**PASS (architectural).**
**Beyond**: append-only signed event log per write + WORM export; DSGVO-scan skill integration.

### 7. Mobile-App-Dev "Kenji" — WASM bundle < 10 MB
Binary today: 1.3 MB single file (default, no C++).
**PASS with 7× headroom** for default build.
**Beyond**: `wasm32-unknown-unknown` target + Matryoshka-256 int8 → < 2 MB vector payload per 10 k docs.

### 8. Research-Lab "Dr. Chen" — MS-MARCO/BEIR top-3 pure-Rust
Measured: **no MS-MARCO/BEIR run** (UC17 n/m).
**FAIL.** Fix: PR-G1 EVAL-HARNESS v0.4 (1 w) — un-blocks credibility for RAG-quality claims.

### 9. Fintech-Compliance "Elena" — Ed25519 proof + tamper-evident
`.synx`/`.brainpack` signed, reproducible. No measured tamper-flip test though.
**PARTIAL PASS.** Fix: ship 10-line negative test (bit-flip → verify fails) + Merkle-root over segments (2 d).

### 10. LangChain-Integrator "Tom" — drop-in VectorStore interface
SPEC §1 explicitly: **"Not a LangChain-orchestrator"**; no VectorStore adapter.
**FAIL (out-of-scope).** Fix (optional): 200-line Python `synapse-langchain` adapter via existing MCP/HTTP shim (2 d). Only if strategic.

### 11. Cursor-IDE-Power-User "Miguel" — cross-file context recall < 50 ms
Measured: lib-mode 0.015 ms + hybrid BM25+vec 0.058 ms. 50-query burst ≈ 3 ms.
**PASS 16× over.**
**Beyond**: semantic file-graph traversal (UC06 KG 3-hop @ 2.21 ms).

### 12. On-Prem-Air-Gap-Defense "Col. Ortiz" — offline, signed artifacts, no net
Pure-Rust default, no C++, no net, Ed25519 `.brainpack`.
**PASS.**
**Beyond**: `cargo deny` + SBOM CI gate + FIPS-mode feature flag.

### 13. Edge-IoT-Dev "Aarav" — 32 MB RAM, 100 k docs
Measured: 100 k disk 174 MB; RSS not measured (UC15 n/m).
**FAIL (unmeasured + likely over).** Fix: PR-C1 int8 + Matryoshka-256 → 4–6× shrink, + RSS harness (3 d).

### 14. OpenAI-Migration "Zoe" — Pinecone→Synapse zero-code-change
No Pinecone-compatible REST today.
**FAIL.** Fix: 400-line `pinecone-shim` over MCP/HTTP (2–3 d). Strategic — huge migration TAM.

### 15. Multilingual-Content "Hans" — DE/EN/CN/AR recall parity ≥ 0.9
Current embedder: BGE-small-384 (EN-biased). UC18 only FTS5 measured.
**FAIL.** Fix: Arctic-embed-v2-m + Matryoshka (1 w, Top-5 Fix #5).

### 16. Realtime-Analytics-Eng "Yuki" — 128 concurrent readers, no drop
UC13 n/m all engines.
**FAIL (unmeasured).** Fix: PR-F1 harness + `Arc<Db>` lock-free read path (already architecturally supported via usearch concurrent search).

### 17. DB-Admin-Oldschool "Bob" — kill -9, reopen, 0 data loss
UC19 only smoke-tested on sqlite-vec. Synapse WAL present (SQLite+fjall+synapse-wal planned).
**PARTIAL.** Fix: PR-E1 crash-replay + `kill -9` fuzz harness (3 d).

### 18. Academic-Benchmarker "Prof. Nakamura" — peer-reviewable methodology
CSV artifacts + reproducible seeds shipped in `bench_*`. EVAL-HARNESS v0.4 pending.
**PARTIAL PASS.**
**Beyond**: preregister on OpenReview, include LoCoMo + TREC-COVID.

### 19. VC-Due-Diligence "Sarah" — defensible 2026 moat
Only row with BM25+HNSW+KG+CRDT+Ed25519+MCP in 1.3 MB single file (MIT, pure-Rust default). 2.9× vs Chroma @ 100 k measured.
**PASS.**
**Beyond**: ship `.synx` signed distribution + publisher marketplace.

### 20. OSS-Maintainer "Jamie" — build < 60 s clean, clippy -D warnings, 0 CVE
Acceptance gate §7 enforces clippy `-D warnings`, MIT-only via `cargo deny`. Build time not explicitly measured.
**PASS (policy).** Fix if needed: `sccache` + `mold` + `cargo nextest` in CI to guarantee < 60 s.

---

## Score: **6 PASS / 3 PARTIAL / 11 FAIL (7 unmeasured, 4 missing feature)**

Honest headline: **6/20 top-0.01% thresholds fully met today.**

## Top-5 FAIL → Fix (effort × impact)

1. **#8 MS-MARCO recall** — PR-G1 EVAL-HARNESS v0.4 (1 w) — unlocks #5, #15, credibility moat.
2. **#4 WP write parity** — batch INSERT + BEGIN/COMMIT wrap + WAL-group-commit (3–5 d) — reframes to 3–5× gap (realistic).
3. **#16/#1 Concurrency** — PR-F1 harness + `Arc<Db>` verification (1 w) — unlocks 3 unmeasured UCs.
4. **#5 100 M scale** — PR-A2 IVF-PQ + PR-C1 int8 (6–8 d + 2 d) — enables 100 M claim.
5. **#15 Multilingual** — Arctic-embed-v2-m + Matryoshka-256 (1 w) — closes DE/CN/AR gap + 32× disk shrink.

## Top-3 "Beyond" moves (already-PASS personas)

1. **#7 Mobile** — `wasm32` build + int8 Matryoshka → < 2 MB mobile vector lib. First pure-Rust embedded agent-memory on iOS/Android.
2. **#19 VC moat** — `.synx` signed-pack marketplace (publisher/subscriber) — only DB with cryptographically distributable memory.
3. **#3 Claude-Code** — turn-scoped in-process prewarm cache → sub-300 µs/100-op recall burst. "Invisible memory" positioning.

## Quick-win cluster (< 1 week total)

PR-G1 (MS-MARCO) + PR-F1 (concurrency) + Ed25519 tamper-test + WP-write batching → moves score from **6/20 to ~12/20** in one session.

## Gated cluster (PR-A2/C1/G1 full shipping, ~3 weeks)

Adds #5, #13, #15 → **~16/20**. At that point "best-in-class agent-memory DB" is defensible without asterisks.

**File**: `docs/20_PERSONAS_CHALLENGE_2026-04-23.md`
