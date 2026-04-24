# synapse-py

PyO3 bindings for [`synapse-core`](../synapse-core).

## Install (dev)

```bash
pipx install maturin
cd crates/synapse-py
maturin develop --release --features simsimd
python -c "import synapse; print(synapse.__version__); print(synapse.truncate_row([1,2,3,4], 2))"
```

## What's exposed (v0.1 preview)

| Python API | Rust backend | feature |
|---|---|---|
| `synapse.cos_f32(a, b)` | `simsimd_kernels::cos_f32` | `simsimd` |
| `synapse.dot_i8(a, b)` | `simsimd_kernels::dot_i8` | `simsimd` |
| `synapse.hamming_b8(q, db)` | `simsimd_kernels::hamming_b8` | `simsimd` |
| `synapse.truncate_row(v, k)` | `matryoshka::truncate_row` | — |

Future (tracked in `docs/SPEC_V2_M4_MAX_2026-04-24.md` §4 Step I):

- `synapse.Brain(path)` — Store wrapper with `put`, `search_hybrid`.
- `synapse.Embedder(kind)` — MLX / Ollama / fastembed backends.
- LangChain + LlamaIndex + Mem0 adapter sub-packages.
