# Show HN Draft

**Title:** Show HN: Synapse — 0.023ms hybrid memory DB in Rust, source-available under FSL

**Pre-launch checklist:** post `LICENSE-FAQ.md` (gtm/LICENSE-FAQ.md) at repo root + link from first paragraph of submission body. Top comment will be license framing — pre-empt it.

**URL:** https://github.com/supersynergy/synapse (placeholder)

**Body:**

Synapse is an agent-memory layer in Rust. We built it because mem0/Zep want our embeddings on their cloud and Hindsight (MIT) is great but ships nothing for production: no encryption, no licensing, no audit story.

What it is:
- Single-file `brain.db` (SQLite + sqlite-vec + FTS5), MCP-native
- Hybrid retrieval (keyword + vector + recency), p50 0.023ms on 147k docs (M4 Max, in-process)
- Internal benchmark vs FAISS, FTS5, Chroma, LanceDB: 3rd overall, ~13× Chroma, ~62× LanceDB. FAISS and pure FTS5 still win their respective single-modality cases. Repro script in `bench/`.
- Encrypted at rest (SQLCipher), per-customer watermarked binaries, Ed25519 JWT licenses bound to hardware fingerprint with 30d offline grace. License server is Rust+axum, ~600 LoC, ships as Docker for Enterprise.
- Demo gif: <placeholder>

Pricing rationale: Dev tier is free cloud (10k docs, rate-limited). Pro is €49/mo self-host. Enterprise is FSL source-available (2-year OSS sunset, à la Sentry/Sourcegraph). We tried pure-OSS-with-support but found it doesn't fund the security work this needs.

Not affiliated with any other "Synapse". Roadmap: graph queries (Graphiti-style), Letta-compatible MCP shapes, Linux/Windows binaries, hosted Pro.

Feedback wanted on: license model, what we're missing vs Letta/mem0 in your stack, and if the bench is something you'd actually re-run.

---

## Anticipated comments + answers

**1. "Why not just OSS like Hindsight?"**
We will OSS the core under FSL (Apache after 2yr). The encryption + license-server bits stay commercial because that's what funds maintenance. Hindsight is great for hobby; once you sign a DPA you need encrypted-at-rest + audit, which is out-of-scope there.

**2. "License question — is FSL really open?"**
FSL = source-available, you can read/fork/run/modify, just can't resell as a competing hosted service for 2 years. Same model as Sentry, Sourcegraph, CockroachDB. After 2 years each release auto-converts to Apache 2.0. fsl.software has the FAQ.

**3. "Rust vs Go for a memory DB?"**
SQLite C-bindings, zero-copy mmap, sqlite-vec is C — Rust gave us safe FFI without GC pauses on the hot path. Go was tested; lost 2-3× on p99 due to GC.

**4. "vs mem0?"**
mem0 is broader (extraction layer, multi-backend). Synapse is narrower and faster: one storage, one query path, encrypted, on-prem. If you want mem0's auto-summarization, run mem0 on top of Synapse — we expose the standard MCP shapes.

**5. "Can I reproduce the bench?"**
`cargo run -p synapse-bench -- --dataset wiki147k`. Uses public Wikipedia chunks. We publish raw CSVs; PRs welcome if your hardware shows different.
