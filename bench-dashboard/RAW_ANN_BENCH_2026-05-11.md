# RAW ANN BENCH 2026-05-11

N=10000 DIM=384 K=10 Q=100  |  M4 Max  |  in-memory only

| Backend | p50 µs | p99 µs | R@10 |
|---------|--------|--------|------|
| FAISS-Flat (exact) | 208 | 286 | 0.9 |
| FAISS-HNSW | 98 | 157 | 0.9 |
| usearch-HNSW (synapse) | 77 | 104 | 0.942 |

usearch build: 1168ms

Parity claim: usearch R@10=0.942 < 0.95
