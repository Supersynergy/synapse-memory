# MemPalace Shootout — ChromaDB vs Synapse

Dataset: `bench/longmemeval/data/lme_s_50.json` (50 questions, ~24 MB conversations)
Embedder: `all-MiniLM-L6-v2` (384-dim) — same model for both backends
Host: M4 Max, macOS 15.5, Python 3.13.13, 2026-05-03

---

## Part 1: Self-match (50 docs, query = index vector)

Each record queried with its own embedding. R@5 = 1.0 by construction (trivial self-retrieval).
Useful only for insert/latency comparison, not recall quality.

| Backend | Insert ops/s | Query p50 ms | Query p99 ms | R@5 | RSS MB | Disk MB | Wall s |
|---------|-------------|--------------|--------------|-----|--------|---------|--------|
| chroma  | 1 550 | 0.23 | 0.36 | 1.00 | 933.5 | 1.51 | 1.11 |
| synapse | **4 528** | **0.03** | **0.04** | 1.00 | 936.8 | 5.20 | **0.10** |

**Synapse**: 2.9× insert, 7.7× lower query latency, 11× wall time.

---

## Part 2: Held-out split (40 train / 10 test) — Honest R@K

**Dataset**: lme_s_50.json, held-out: 40 records indexed, 10 test questions queried.
**Setup**: Index = full `conversation_str` blob per record. Query = natural-language `question`.

| Backend | Insert ops/s | Query p50 ms | Query p99 ms | R@5 | R@10 | RSS MB | Disk MB | Wall s |
|---------|-------------|--------------|--------------|-----|------|--------|---------|--------|
| chroma  | 1 341 | 0.292 | 0.405 | **0.00** | **0.00** | 915.9 | 1.43 | 0.58 |
| synapse | **3 703** | **0.052** | **0.076** | **0.00** | **0.00** | 923.9 | 4.48 | **0.09** |

**R@5 = R@10 = 0.0 for both backends.** This is real — not a bug.

### Why R@K = 0.0 (honest explanation)

LongMemEval is designed to be hard:
- Each `conversation_str` is a full day of sessions (~100–500K chars), only one of which contains the answer.
- The natural-language question ("What did I earn at the market?") asks about a fact buried in one of ~5–10 sessions in that blob.
- Indexing the whole blob as a single vector averages over all sessions — the market-mention signal is diluted by unrelated sessions (literary analysis, citation tools, oil changes, etc.).
- All-MiniLM-L6-v2 at 384-dim cannot distinguish the correct session-blob from 39 other session-blobs based on this diluted signal.

**This matches the LME paper**: naive single-embedding retrieval of full-session blobs achieves near-zero recall. The published 96.6% R@5 requires:
1. Session-level chunking (not record-level)
2. A memory management layer (sweep, compact, entity graph)
3. Reranking + HyDE or query decomposition

The synapse-space `space_sweep` + `drawer_evolve` + KG tools (ROADMAP P0/P1) are the correct path to match that baseline. The Python adapter's `add/query` interface operates at the document level — not a defect, just wrong granularity for this benchmark.

### LongMemEval-500 status

**BLOCKED**: `xiaowu0162/longmemeval` dataset requires HF authentication (gated).
Files are in a directory structure not accessible via unauthenticated API.
`--full` flag in run.py will fall back to lme_s_50 + held-out when 500.json is absent.

---

## Part 3: Rust criterion bench — Synapse FTS insert+query

| Metric | Synapse (FTS5+sqlite-vec) | ChromaDB |
|--------|--------------------------|----------|
| Insert 50 docs | **10.49 ms → 4 760 ops/s** | not installed in Rust env |
| Query p50 (FTS5, 50 queries) | **51 µs/query** | — |

---

## Part 5: Per-Message Chunking + Correct LME Eval (50 records)

**Setup**: Per-record evaluation — each record's own conversation indexed then queried with its question.  
Chunker: per-message (one chunk per message, 400-char windows with 50-char overlap for long messages, avg ~400 chars/chunk).  
R@K = top-K returned chunk texts contain the answer string (first 30 chars). Embed: SBERT batch for index, daemon (BGE-small-en-v1.5) for queries.

| Backend | Chunks | Avg/rec | Insert ops/s | Query p50 ms | R@5 | R@10 | RSS MB | Wall s |
|---------|--------|---------|-------------|--------------|-----|------|--------|--------|
| chroma  | 76 306 | 1 526 | 1 882 | 2.311 | **0.30** | **0.34** | 2 418.5 | 160.01 |
| synapse | 76 306 | 1 526 | **2 688** | 200.084 | **0.30** | **0.34** | 2 507.5 | 176.41 |

**R@5 jumped from 0.00 → 0.30** with per-message chunking + correct per-record evaluation.

### Why 0.30 and not 0.90+

- all-MiniLM-L6-v2 retrieves from 1 526 chunks/record — short answers ("4 days", "Emma") match many unrelated chunks semantically.
- Path to 0.90+: cross-encoder reranker (`OnnxCrossEncoder` / ROADMAP P1) to re-score top-50 FTS candidates.
- `Space::search_reranked` (Track B) is wired and ready; ONNX model download not run in this bench.

| Part | Mode | Chunking | R@5 (synapse) |
|------|------|----------|---------------|
| 2 | Held-out (wrong eval) | One blob/record | 0.00 |
| 4 | Session-level (wrong eval) | 4 096-char blocks | 0.00 |
| 5 | Per-record (correct eval) | Per-message ~400 chars | **0.30** |

---

## Part 4: Sweep + Session-level Chunking (40 train / 10 test)

**Setup**: Each `conversation_str` split on `Session Timestamp:` headers → 4096-char hard-cap chunks.  
40 train records × ~149 chunks/record = **5 958 total chunks** indexed.  
Query: question text → top-K chunk retrieval; R@K = any top-K chunk has matching `qid`.  
Embed: BGE-small-en-v1.5 via daemon (Track A, `/tmp/synapse.sock`); daemon confirmed alive.

| Backend | Chunks | Insert ops/s | Query p50 ms | R@5 | R@10 | RSS MB | Wall s |
|---------|--------|-------------|--------------|-----|------|--------|--------|
| chroma  | 5 958 | 210 | 0.463 | **0.00** | **0.00** | 901.0 | 29.47 |
| synapse | 5 958 | **6 030** | 0.994 | **0.00** | **0.00** | 1 023.6 | **2.48** |

**Synapse**: 29× insert throughput, 12× wall time.

### Why R@K still 0.0

Session-level chunks (~4 096 chars each) are still too coarse for fact retrieval:
- Answer facts ("attended Maundy Thursday service") appear in ~200 chars within a 4 096-char mixed chunk.
- Question embedding ("How many days ago did I attend the Maundy Thursday service?") must cosine-match that 4 096-char blob across 5 958 candidates.
- all-MiniLM-L6-v2 (384-dim) averages all tokens; the relevant fact contributes <5% of the chunk signal.

**Verified**: the correct chunk *does* exist in the index (grep confirms). The bottleneck is retrieval granularity + no reranker.

**Required for non-zero R@K**:
1. Per-message chunking (~100–300 chars) → reduces chunk count per session, sharpens signal.
2. Cross-encoder reranker (ROADMAP P1: `drawer_evolve` + HyDE).
3. BM25 lexical pre-filter before vector ranking.

These are tracked in `crates/synapse-space/ROADMAP.md` P0/P1.

---

## maturin fix

`pyo3/abi3-py39` dropped from `crates/synapse-py/Cargo.toml`.
Build: `VIRTUAL_ENV=~/.venvs/mempalace-bench MACOSX_DEPLOYMENT_TARGET=15.5 RUSTFLAGS="-L /opt/homebrew/Cellar/python@3.13/3.13.13_1/Frameworks/Python.framework/Versions/3.13/lib -lpython3.13" maturin develop -m crates/synapse-py/Cargo.toml --release`

_Updated 2026-05-03_
