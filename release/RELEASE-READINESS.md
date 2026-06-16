# Synapse — Release Readiness (ready for BOTH: private + public)

One workspace, two products:
- **private** — the full 56-crate engine Maxim runs locally (unchanged, battle-tested: auto-capture hooks, LongMemEval R@5 0.640, PageRank `ground`).
- **public** — a clean MIT subset (`grounddb`) emitted on demand by `release/oss-bundle.sh`, verified by `release/scrub-gate.sh`.

## How to emit the public bundle
```bash
release/oss-bundle.sh --check     # parse manifest, list public-mit crates (no copy)
release/oss-bundle.sh             # → release/dist/grounddb-oss/  (then scrub-gate runs)
release/scrub-gate.sh <dir>       # standalone safety scan (fail-closed)
```
Tiers live in `release/oss-manifest.toml`. Default-deny: any crate not listed `public-mit` never ships.

## License coherence (audit-corrected — mostly fine)
The audit resolved the apparent contradiction:
- **`synapse-engine` `license = "MIT"` is CORRECT.** The tracked source is only an open
  RRF C-ABI (`engine/src/{abi,rrf,lib}.rs`, ~209 LOC, no secrets). The *proprietary* thing
  is the compiled `libsynapse_engine` artifact, not this crate's source. → no Cargo fix
  needed; add one README line clarifying "source MIT, compiled artifact proprietary" to
  avoid confusion.
- **`synapse-core` = FSL-1.1-Apache** — declared correctly (workspace override). Matches LICENSE-CORE.md.
- `synapse-license` — keep source-available but flag (Ed25519/ChaCha20 license-enforcement logic).

No material license/content contradiction once the source-vs-artifact distinction is clear.
The bundle ships only `public-mit` crates regardless, so the boundary is safe either way.

## Pre-public cleanup (from audit)
- 🔴 **`core` coredump (4.96 MB) was tracked in git HEAD.** Now: deleted from disk + added to
  `.gitignore`. Index/history removal is blocked by a local git quirk on files named `core`
  → owner runs `git filter-repo --path core --invert-paths` (history rewrite = owner decision)
  before any full-repo public push. The OSS bundle never includes repo-root files, so it does
  not leak there.
- 🟡 `true@supersynergy.de` in ~10 `Cargo.toml` authors + private absolute paths in some docs/bench-results — cosmetic; swap to a role address if desired before public.

## Tier map (see oss-manifest.toml; public-mit pending audit verification)
- **proprietary** (moat, never public): engine, license, kernel, quant, ann, spann, market, raft, cluster, tier.
- **fsl-core** (source-available, non-commercial): core.
- **public-mit** (candidates, audit-verified for zero secrets): mcp, cli, graph, fts, fusion, rerank, extract, temporal, migrate, obs.

## Known coupling risk (audit confirms)
The `public-mit` crates may depend on `synapse-core` (FSL). If so, the public bundle
needs those dependencies either (a) decoupled behind a trait, or (b) the minimal core
surface they need re-licensed/re-implemented MIT. This is the main engineering gap
between "manifest lists them" and "bundle compiles standalone." Tracked as the next slice.

## Optimization findings
See the audit report (build/test health, the one recall-path bottleneck, safe wins).
Filled in when the read-only audit completes; safe high-confidence wins applied,
correctness-touching items flagged DO-NOT-AUTO for owner review.
