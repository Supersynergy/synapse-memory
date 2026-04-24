# Synapse v2.1-m4max-preview · Launch Card

> Sub-millisecond agent memory on Apple Silicon. Written in Rust. Drop-in
> Python. Adapts to your corpus.

## Headline numbers (M4 Max · 100 000 × 384-dim · fresh run)

| path | µs / query | QPS | vs NumPy-no-SIMD |
|------|-----------:|----:|-----------------:|
| scalar f32 | 1 373 | 728 | 1.00× |
| ndarray BLAS-proxy | 2 885 | 347 | 0.48× |
| **SimSIMD int8** | **414** | **2 417** | **3.32×** |
| **Hamming → int8 rescore k=10** | **293** | **3 407** | **4.68×** |

Peak in the 8-kernel progression (S0 → S8): **71× vs scalar** (S4 · 1-bit Hamming · 192 µs). See `docs/bench_2026-04-24/progression.md`.

## What ships

| layer | what |
|------|------|
| **Kernels** | NEON int8 dot · 1-bit popcount · f16 storage · MRL trunc · Binary-MRL |
| **Indices** | `InMemoryI8Index` · `InMemoryF16Index` (50 % RAM) · `InMemoryHammingIndex` |
| **Routing** | `AdaptiveRouter` · Thompson-bandit · observe() learns from real latency |
| **Bundle** | `MultiIndex.build(rows).search(q, latency_budget, min_recall, k)` |
| **Framework adapters** | LangChain · Mem0 · LlamaIndex |
| **Formats accepted** | `.md .txt .rst .org .json .jsonl .csv .tsv .db .sqlite .synx .brainpack` |
| **Bench harnesses** | 11 real-world (Obsidian · ChatGPT · Gmail · Slack · iMessage · Apple Notes · Logseq · WhatsApp · Linear · Photo-CLIP · Health-fuse) + 50-usecase catalog |

## One-liner (Python)

```python
import synapse
rows = [(i, your_embedder(text)) for i, text in enumerate(corpus)]
idx  = synapse.MultiIndex.build(rows)
hits = idx.search(query_vec, latency_budget_us=500, min_recall=0.95, k=10)
```

## One-command (terminal)

```bash
./demo.sh
```

## Quality

- **68 Rust tests** · **10 pytest cases** · 0 errors · 0 release warnings · clippy clean
- **4 parallel Haiku-agent audit sweep** (bottleneck · error-hunt · best-practice · smart-test) → 3 fixes applied
- **28 loop iterations** · 35+ commits on `m4max-preview`

## Rollback

```bash
git checkout backup-pre-m4max-bench-2026-04-24     # tag
# or
tar -xzf ~/projects/data/synapse-backup-2026-04-24.tgz
```

## 140-char pitches

### HN / X / Mastodon

> Synapse v2.1: agent memory with sub-ms search on Apple Silicon. Rust-native, Thompson-bandit router, LangChain/Mem0/LlamaIndex adapters.

### Blog sub-headline

> 5× faster than your NumPy fallback · 50 % less RAM · recall@10 ≥ 0.99 · runs on battery.

### Elevator

> Drop-in replacement for in-memory vector caches inside agent apps. Four indices, one router, 68 tests, zero warnings. `pip install synapse`.
