# Architecture — synapse-memory

Local-first **Context OS** for AI agents: give a coding agent the best *relevant*
context before it acts — bounded, cited, freshness-aware, feedback-improved.
One SQLite-backed brain. No Docker, no cloud.

> Core promise: **best context, not biggest context.**

## Layers

Dependency direction points inward only. The default `cargo check --workspace`
builds the L1 + L2 product crates; L0 substrate is pulled in via path-deps from
the vendored foundation. Enforced by `scripts/check-layering.py` (see ADR 0001).

```
L2  interfaces   synapsed · synapse-cli · synapse-mcp        the product surface
        │                                                    socket · synx CLI · MCP bridge
        ▼
L1  domain       synapse-extract · synapse-rerank            chunking · rerank ·
                 synapse-learn · synapse-space               bandit feedback ·
                 synapse-temporal                            namespaces · temporal
        │
        ▼
L0  substrate    vendor/synapse-db                           SQLite + sqlite-vec +
                 (core · kernel · engine · ann · fts · graph) FTS5 + SIMD kernels
```

`synapse-mcp` holds **no** intra-workspace path deps — it speaks to `synapsed`
over the unix socket. Interfaces stay decoupled; keep it that way.

## Product surface (CLI)

`synapse` exposes the SPEC's Context-OS workflow:
`prime · context · remember · feedback · fresh-context · doctor`
(plus low-level `put · find · vec · hybrid · verify · keygen · snap-signed`).

The substrate's vector/FTS/graph power is plumbing behind that surface — not the
first-run promise.

## What is NOT in the default build

Excluded from the default workspace, built on demand (see ADR 0001):

- **substrate**: `vendor/synapse-db` (built transitively by L1/L2 path-deps)
- **bindings**: `synapse-py`, `synapse-js`
- **platform/GPU**: `synapse-metal`, `synapse-embed-gpu`
- **advanced retrieval (research)**: `synapse-colbert`, `synapse-splade`, `synapse-fusion`
- **multimodal/media**: `synapse-multimodal`, `synapse-media`
- **divergent twins** (`synapse-core`, `synapse-engine`, `synapse-ann`,
  `synapse-kernel`, `synapse-quant`, `synapse-fts`, `synapse-graph`,
  `synapse-spann`, `synapse-obs` under `crates/`): a v2.1.0-era code island
  with **zero product dependents** — every product crate path-depends on the
  vendored `vendor/synapse-db` equivalents. The twins stay in tree for
  reference; build them from their own directories. Gates
  (`fmt/clippy/test/layering`) intentionally cover only the vendored
  engine. Convergence → bd `synapse-memory-cpd`.

## Build & verify

```bash
just setup    # rustup show (vendor/synapse-db is absorbed, no submodule)
just check    # layering guard + fmt + clippy + cargo check (fast gate)
just test     # cargo nextest
just ci       # check + test + cargo deny
just build    # release build
```

Decisions of record live in `docs/adr/`.
