# synapse-py

PyO3 bindings for [`synapse-core`](../synapse-core).

## Install (dev)

```bash
pipx install maturin
cd crates/synapse-py
maturin develop --release --features simsimd
pytest tests/ -v
```

## What's exposed

| Python API | Rust backend | feature |
|---|---|---|
| `synapse.cos_f32(a, b)` | `simsimd_kernels::cos_f32` | `simsimd` |
| `synapse.dot_i8(a, b)` | `simsimd_kernels::dot_i8` | `simsimd` |
| `synapse.hamming_b8(q, db)` | `simsimd_kernels::hamming_b8` | `simsimd` |
| `synapse.truncate_row(v, k)` | `matryoshka::truncate_row` | — |
| `synapse.MultiIndex.build(rows).search(q, latency_budget_us, min_recall, k)` | `turbo::multi_index` — **one-call bundle** | `simsimd` |
| `synapse.I8Index.build(rows).search(q, k)` | `turbo::inmem_i8_index` | `simsimd` |
| `synapse.F16Index.build(rows).search(q, k)` — 50 % RAM | `turbo::inmem_f16_index` | `simsimd` |
| `synapse.HammingIndex.build(rows).search(q, k)` | `turbo::inmem_hamming_index` | `simsimd` |
| `synapse.rerank(ham, i8, q, k, candidates)` | two-stage pipeline | `simsimd` |
| `synapse.AdaptiveRouter()` | Thompson-bandit strategy picker | `turbo` |
| `synapse.Brain(path)` | `synapse-core Store` wrapper | — |

## One-liner end-to-end

```python
import synapse
rows = [(i, your_embedder(text)) for i, text in enumerate(corpus)]
idx  = synapse.MultiIndex.build(rows)
hits = idx.search(query_vec, latency_budget_us=500, min_recall=0.95, k=10)
```

The router dispatches to the best backend per query (int8 / f16 / hamming)
based on corpus size + latency budget + recall floor, then learns from
observed latency via `idx.observe(...)` in production.

## Framework adapters

See `examples/`:

| File | Integrates with |
|------|-----------------|
| `langchain_adapter.py` | LangChain `VectorStore` + `Embeddings` |
| `mem0_adapter.py` | Mem0 `VectorStoreBase` |
| `llamaindex_adapter.py` | LlamaIndex `BasePydanticVectorStore` |

## Benchmarks

See [`docs/bench_2026-04-24/progression.md`](../../docs/bench_2026-04-24/progression.md)
for the 8-step SIMSIMD progression (53×–71× vs scalar on M4 Max).
