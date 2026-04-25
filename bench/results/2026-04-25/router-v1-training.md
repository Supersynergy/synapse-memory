# Router v1 Training Report — 2026-04-25

## Data

| Metric | Value |
|--------|-------|
| query_logs rows | 491 |
| Unique query hashes | 17 |
| Training samples (post-label) | 17 |
| Data generation method | Synthetic bench (8–16 repeated queries × 20 iter) |

## Per-Mode Latency Stats

| Mode   | n   | Mean (µs) | Median (µs) | p95 (µs) |
|--------|-----|-----------|-------------|----------|
| Lex    | 320 | 2,721     | 2,001       | 6,890    |
| Vec    | 80  | 87,538    | 69,691      | 205,663  |
| Hybrid | 96  | 104,367   | 77,448      | 209,993  |

Lex is **32–52× faster** than Vec/Hybrid on this corpus (88 docs). Latency gap will narrow at scale (FTS5 degrades, HNSW stays flat).

## Model

- Algorithm: CatBoostClassifier (iterations=200, depth=4, lr=0.1, CPU)
- Features used: `query_len`, `hour_of_day`
- Classes learned: `lex`, `hybrid` (vec absent from labels — always loses to lex on small corpus)

## 5-Fold CV Accuracy

| Result | Value |
|--------|-------|
| Mean accuracy | ~1.000 (4/5 folds) |
| Note | 1 fold failed — only 1 "hybrid" sample, can't stratify reliably |

**Effective accuracy is misleading**: the model learned "always predict Lex" because Lex wins on every query with a tiny corpus. This is not a classifier failure — it's a data reality.

## Feature Importances

| Feature | Importance |
|---------|------------|
| query_len | 100% |
| hour_of_day | 0% |

## Verdict: NOT production-ready yet

**Data blockers:**
1. Only 17 unique queries — synthetic repeats of the same 16 strings
2. Vec never beats Lex on 88-doc corpus — no class diversity
3. Missing 10/12 PHASE-2B features (IDF, embedding norm/entropy, EMA hit rates, cache ratio)

**What's needed before production routing:**
- 10k+ real diverse queries with 10k+ docs indexed
- Vec/Hybrid queries embedded (embed_query=True) at ingestion time
- EMA feedback loop wired (prev_hit_rate_lex/vec columns)
- Minimum 3-class balance: target ≥15% each of Lex/Vec/Hybrid wins

**Path forward:**
1. Ingest real content (synapse_ingestor or doc dump) → 10k+ docs
2. Run real query traffic for 48h with all 3 modes
3. Retrain with full 12-feature set per PHASE-2B spec
4. Shadow-mode gate: >68% queries improve, median uplift >10%

Model saved: `~/.synapse/models/router_v1.cbm` (serves as baseline, not deployed).
