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
| `synapse.I8Index.build(rows).search(q, k)` | `turbo::inmem_i8_index` | `simsimd` |
| `synapse.HammingIndex.build(rows).search(q, k)` | `turbo::inmem_hamming_index` | `simsimd` |
| `synapse.rerank(ham, i8, q, k, candidates)` | two-stage pipeline | `simsimd` |
| `synapse.AdaptiveRouter()` | Thompson-bandit strategy picker | `turbo` |
| `synapse.Brain(path)` | `synapse-core Store` wrapper | — |

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
