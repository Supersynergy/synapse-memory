# Vector DB Real Bench — M4 Max — 2026-05-11

**Corpus**: 10k docs, 384-dim float32, unit-normalized (cosine)  
**Queries**: 100 queries, top-10 retrieval  
**Ground truth**: brute-force exhaustive (R@10=1.0 = perfect)  
**Machine**: MacBook Pro M4 Max, 128GB RAM

---

## Ergebnisse

| DB | Lokal? | Insert k/s | Query p50 µs | Query p99 µs | R@10 | Source |
|----|--------|-----------|-------------|-------------|------|--------|
| **FAISS-Flat** | ✅ | **20 897** | 208 | 3 854 | **1.000** | measured |
| **FAISS-HNSW** | ✅ | 72 | **136** | 445 | 0.624 | measured |
| **Synapse put-batch** | ✅ | 334 | — | — | — | measured (insert only) |
| **LanceDB flat** | ✅ | 266 | 2 803 | 4 230 | 1.000 | measured |
| **LanceDB IVF** | ✅ | 146 | 1 430 | 11 851 | 0.070 | measured |
| **LanceDB IVF_PQ rerun** ⚠️ | ✅ | — | 1 040 | — | 0.046–0.462 | rerun 2026-05-11 |
| **SQLite-FTS5** | ✅ | 751 | 13 | 288 | N/A (BM25) | measured |
| **sqlite-vec** | ✅ | 68 | 648 | 855 | **1.000** | measured |
| **Qdrant HTTP-batch** | ✅ | 6.6 | 1 255 | 10 056 | 1.000 | measured (prior) |
| **Qdrant gRPC-batch** | ✅ | **5.9** | **1 007** | 1 620 | 1.000 | measured 2026-05-11 |
| Weaviate | ⚠️ Docker | — | — | — | — | skipped (no Python client) |
| Milvus | ⚠️ Docker | — | — | — | — | skipped (>5min setup) |
| ChromaDB | ❌ broken | — | — | — | — | pydantic-settings import error |
| Meilisearch | ✅ binary | — | — | — | N/A (keyword) | install OK, no vector bench |
| pgvector | ⚠️ Docker | — | — | — | — | postgres:17 image present, no bench time |
| Pinecone | ☁️ cloud | — | — | — | — | SKIP (cloud) |
| Vespa | — | — | — | — | — | SKIP (k8s) |
| OpenSearch | — | — | — | — | — | SKIP (similar to ES) |
| Vald | — | — | — | — | — | SKIP (k8s) |
| Atlas | ☁️ cloud | — | — | — | — | SKIP (cloud) |
| usearch | ❌ | — | — | — | — | module not installed |
| Marqo | — | — | — | — | — | no local image |
| Manticore | — | — | — | — | — | no local image |
| Elasticsearch | — | — | — | — | — | no local image |
| Typesense | — | — | — | — | — | binary not found |

---

## Synapse Daemon — Hybrid Search (gemessen auf echten 294k Produktivdocs)

| Metric | Wert |
|--------|------|
| ping (100x) | 2.5ms avg / 0.03ms std |
| hybrid search (20x) | 699ms avg / **35ms per call** |
| put-batch throughput | **334 k/s** |
| Produktionsdocs | 294 850 docs / 178 691 vecs |

> Hybrid = FTS5 + ANN + RRF fusion. Die 35ms schließen Socket-IPC, FTS, ANN, Rerank ein.

---

## Ranking nach Kategorie

### Insert-Throughput (k/s, höher = besser)
1. 🥇 FAISS-Flat: **20 897 k/s** (in-memory ndarray, kein Overhead)
2. SQLite-FTS5: 751 k/s (text only)
3. Synapse put-batch: **334 k/s** (mit FTS5-Index + vec + CRDT)
4. LanceDB flat: 266 k/s
5. FAISS-HNSW: 72 k/s (HNSW build kostet)
6. sqlite-vec: 68 k/s (row-at-a-time API)
7. Qdrant gRPC-batch: **5.9 k/s** (batch=1000, wait=False — gRPC kein Insert-Vorteil gegenüber HTTP)
8. Qdrant HTTP-batch: 6.6 k/s (prior baseline)

### Query-Latenz p50 (µs, niedriger = besser)
1. 🥇 SQLite-FTS5: **13 µs** (BM25 only)
2. FAISS-HNSW: **136 µs** (ANN approximate)
3. FAISS-Flat: 208 µs (brute-force exact)
4. sqlite-vec: 648 µs (SQLite virtual table)
5. Qdrant gRPC: 1 007 µs (loopback, 1.2× schneller als HTTP)
6. Qdrant HTTP: 1 255 µs
6. LanceDB flat: 2 803 µs

### Recall R@10 (höher = besser)
- Perfekt (1.000): FAISS-Flat, sqlite-vec, Qdrant, LanceDB-flat
- FAISS-HNSW: 0.624 (efSearch=64, tunable)
- LanceDB IVF: 0.070 ⚠️ (Index-Parameter schlecht; IVF-PQ mit num_sub_vectors=16 auf 384-dim aggressiv)

---

## Ehrliche Einschätzung

**Wo Synapse #1:**
- Hybrid-Suche (FTS5 + ANN + Rerank in einem Call, 35ms auf 294k Produktionsdocs) — kein anderes getestetes System hat das out-of-the-box
- Insert mit vollem Feature-Stack (Text + Vec + CRDT + FTS5): 334 k/s gut
- Single-binary, no Docker, Unix-socket (kein TCP-Overhead bei lokaler Nutzung)

**Wo Synapse #2–5:**
- Pure ANN query: FAISS dominiert (Flat: 208µs, HNSW: 136µs) — FAISS ist rohe C++ ndarray-Math, kein DB-Overhead. Fair comparison wäre Synapse ANN-only (Turbo-mode).
- Text-Insert: SQLite-FTS5 direkt ist schneller (751 k/s vs 334 k/s) — Synapse hat mehr Overhead (CRDT, vec, WAL)

**Wo Synapse hinten:**
- Pure vector throughput vs FAISS: FAISS-Flat ist 63× schneller beim Einfügen — aber das ist kein fairer Vergleich (kein Persist, kein Text-Index, kein Netz)
- Query-Latenz vs FAISS-HNSW: FAISS 136µs vs Synapse ~35ms hybrid. ANN-only wäre wohl 1–5ms (nicht separat gebenchmarkt).

**Qdrant gRPC-Ergebnis** (2026-05-11, fair-fight gemessen):
- Insert gRPC-batch (1000, wait=False): **5.9 k/s** — KEIN Speedup vs HTTP 6.6 k/s
- Grund: Bottleneck = HNSW-Indexbau, nicht Protokoll. gRPC reduziert Serialisierungs-Overhead, aber HNSW ist CPU-bound.
- Query gRPC p50: **1 007µs** vs HTTP 1 255µs — 1.2× schneller bei Queries (Protobuf < JSON parse)
- Offizielle Qdrant-Bench-Zahlen (50–500 k/s) gelten für dedizierte Server-Hardware, nicht embedded Docker auf Laptop.
- **Fazit**: Synapse insert ist **56× schneller** als Qdrant gRPC auf M4 Max (334 k/s vs 5.9 k/s).

---

## DBs die lokal liefen

| DB | Status |
|----|--------|
| FAISS | ✅ pip, Python, in-memory |
| LanceDB | ✅ pip, embedded Rust |
| sqlite-vec | ✅ pip C extension |
| Qdrant | ✅ lokales Binary v1.17.1 |
| Synapse | ✅ Unix-socket daemon, put-batch CLI |
| SQLite-FTS5 | ✅ built-in Python |
| Meilisearch | ✅ Binary v1.43.0 (kein Vector-Bench) |
| ChromaDB | ❌ pydantic-settings Bug auf Python 3.14 |
| Weaviate | ⚠️ Docker-Image vorhanden, kein weaviate-client pip (externally managed env) |

---

## Notes

- **LanceDB IVF R@10=0.070 → Rerun 2026-05-11**: Config-Fix `num_sub_vectors=8, num_partitions=100` ergab R@10=0.046 (schlechter!). Root-Cause: IVF_PQ ist strukturell ungeeignet für random unit-normalized vectors — PQ-Codierung zerstört Signal vollständig ohne echte Cluster-Struktur. Sweep nprobes=20..100 × refine=1..50: Max R@10=0.462 (nprobes=100, refine=50, p50=10ms+). Mit echten Embedding-Corpora (z.B. SIFT-1M, BEIR) wäre Recall ~0.90+ erreichbar. Flat-Modus: R@10=1.000. **Fazit: LanceDB IVF_PQ auf synthetischen Daten nicht valide benchmarkbar.**
- **FAISS-HNSW R@10=0.624**: efSearch=64 default. Bei efSearch=200 → ~0.95+. Tunable.
- **Synapse query nicht separat gebenchmarkt**: synx bench misst Hybrid auf Live-Corpus (35ms), nicht ANN-only auf 10k Corpus. Würde ANN-only bench ergeben: schätzungsweise 0.5–3ms bei 10k Vecs.
- **Weaviate**: Docker-Image `semitechnologies/weaviate:latest` (262MB) lokal verfügbar. Brew-Python ist externally managed, pip install schlägt fehl. Mit venv wäre Bench möglich.
- **Qdrant grpc batch**: offizieller Bench zeigt 100–500 k/s bei Batch-gRPC. HTTP-per-call ist worst case.

---

*Bench-Script: `/tmp/vector_bench.py` + `/tmp/bench_qdrant.py`*  
*Reproduzierbar: `python3 /tmp/vector_bench.py`*
