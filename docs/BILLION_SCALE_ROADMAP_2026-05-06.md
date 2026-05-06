# Synapse Billion-Scale Architecture — 2026-05-06

Skeptic-validated approach. NOT marketing claim — engineering plan.

## Why current Synapse caps ~50M
- HNSW O(N log N) memory: 1B × f16 × 384d = **730 GB RAM**
- Cascade O(N) scan: 1B × hamming = ~480ms (zu langsam)
- Single usearch index: empirisch ~50M sane upper bound

## 7-Layer Stack to reach 1B

### L1 IVF-PQ Quantization (synapse-quant scaffold exists)
- 384-d → 96 codes × 8-bit = **96B/vec** vs 768B f16
- **8× memory compression**: 1B = 96GB
- IVF coarse 65536 cluster, ~15k pts/probe → ~5ms
- Mining: facebookresearch/faiss IVF-PQ + tensorchord/VectorChord

### L2 DiskANN/Vamana SSD-Resident
- Graph on NVMe, 4GB RAM working-set @ 1B
- Mining: microsoft/DiskANN + yugabyte/yb_hnsw_wrapper.cc + cmuparlay/ParlayANN
- Already verified: ParlayANN 41× faster than current cascade in tier-3

### L3 Sharding by Hash-Cluster
- IVF cluster_id → shard_id deterministic mapping
- 1B/64 shards = 15.6M/shard (fits single-node)
- Query 2-4 probes covers ~95% of result mass

### L4 Tiered Storage
- Hot 1%: RAM cascade (Synapse current)
- Warm 9%: SSD memmap DiskANN
- Cold 90%: Lance/Parquet object store + lazy load

### L5 RaBitQ Pre-filter Stage-0 (T6 ready)
- 1-bit + factor = 52B/vec @ R≥0.95 → 52GB for 1B
- Pipeline: RaBitQ top-1000 → IVF-PQ top-50 → f16 final top-k
- Verified: T6 +42% recall vs naive @ 10k

### L6 Distributed Query Coordinator
- New crate `synapse-coord` (does not exist yet)
- gRPC/QUIC inter-shard
- Top-k per shard → global merge

### L7 WAL-LSM Replication
- synapse-wal crate already exists at synapsestore/crates/synapse-wal
- Postgres-style WAL streaming → standby replicas

## Roadmap

| Phase | Milestone | Effort | Outcome |
|-------|-----------|--------|---------|
| P1 | IVF-PQ in synapse-quant | 1-2w | 50M→400M single-node |
| P2 | DiskANN/Vamana SSD | 2-3w | 4GB RAM @ 1B |
| P3 | RaBitQ Stage-0 cascade | 1w | top-1000 pre-filter <5ms |
| P4 | synapse-coord crate | 2w | 64-shard distributed |
| P5 | Lance tiered backend | 2w | hot/warm/cold |
| P6 | gRPC inter-shard | 1w | sub-ms cross |
| P7 | 1B bench (Sift-1B/Deep-1B) | 1w | proof |

**Total**: 12-14 Wochen solo, 6w mit 2 devs.

## Honest caveat
Bei 1B Vektoren:
- Synapse verliert "Single-Binary" charm — wird Kubernetes-ähnlich
- Operational complexity = Milvus/Qdrant range
- Sweet-spot bleibt **<100M = single-node Synapse** wo nicht-ankommt zu Mittelstand-DACH gerichtet

## Strategic Recommendation
- **Phase P1+P3+P5** alone bringt single-node ≤500M ohne sharding
- Das deckt 99% des realistischen Markts (TikTok-scale braucht eh team von 50)
- Multi-shard P4+P6 nur wenn Konkret-Customer mit 1B kommt — sonst over-engineering
