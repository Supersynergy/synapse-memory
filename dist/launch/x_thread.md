# X / Twitter Launch Thread — Synapse

---

**Tweet 1 — Hook**

We built a Rust library that gives you R@10 = 1.000 conformal guarantee on hybrid search — not a tuning knob, a calibrated statistical bound.

Single binary. SQLite inside. No Docker. No cloud.

Thread 🧵

---

**Tweet 2 — Problem**

Every AI agent project hits the same fragmentation wall:

- Qdrant for vectors
- SQLite FTS5 for keyword search
- A graph store for relations
- A CRDT layer for sync

Four processes. Four failure surfaces. Four deployment configs.

---

**Tweet 3 — Solution**

Synapse is one embedded Rust binary that does all four:

vec + BM25 FTS + knowledge-graph triples + CRDT peer sync

One SQLite file. Unix-socket daemon. Ed25519-signed docs. Offline-first.

github.com/Supersynergy/synapse

---

**Tweet 4 — NEON RRF speed**

Reciprocal-rank fusion (merging BM25 + ANN results) is normally a CPU bottleneck.

We rewrote the inner loop with NEON SIMD f32x4 lane-parallel reciprocal arithmetic.

Result: 4.3–5.1× faster RRF on M4 Max vs scalar. Measured with Criterion.

---

**Tweet 5 — BMP 9.7×**

`synapse-fts` uses Block-Max WAND posting lists (BMP) over tantivy.

Early termination on blocks whose max score can't beat the current heap top.

Measured: 9.7× latency drop vs FTS5 cold scan. 18.3× on warm cache.

Same recall. Just faster.

---

**Tweet 6 — Multimodal asset DB**

`synapse-media` indexes video keyframes, audio segments, and image embeddings alongside text — same unified query interface.

Plugs into ComfyUI (image gen) and Remotion (video) for retrieval-augmented asset pipelines.

One schema. No glue code.

---

**Tweet 7 — CRDT cluster scale-out**

`synapse-cluster` uses CRDT gossip for peer sync.

Export a `.synx` brainpack from node A. Import on node B. Docs merge without conflict.

<200ms LAN convergence. No Kafka. No Redis. No coordinator. Just two binaries talking over sockets.

---

**Tweet 8 — Try it**

```
brew tap supersynergy/synapse && brew install synx

synx put --text "your first document"
synx hybrid "your query"
synx stats
```

Or: `npx @supersynergy/synx`

334k inserts/s. 35ms hybrid on 294k docs. R@10=1.000.

Repo + bench scripts: github.com/Supersynergy/synapse

We're looking for testers with large corpora (>1M docs). Drop a comment or open an issue.
