# Multi-Hop Entity Graph Traversal

## Why
1-hop entity expansion in `Store::recall` leaves multi-hop LongMemEval queries
("what did Alice say after the Q1 review") under-served. Push to BFS depth ≤3
with attenuation `0.6^hop` per added list weight in RRF fusion.

## What
- `synapse_core::sota::multi_hop_neighbors(conn, seeds, max_hops, per_hop_cap)`:
  BFS over `memory_edges`, returns `(memory_id, hop_distance)` excluding seeds.
- `RecallParams::max_hops` (default 2). When `entity_expand=true`, fusion adds
  one RRF list per hop level with `list_weight = 0.6.powi(hop)`.
- Bounded: `per_hop_cap=64`, `max_hops≤3`, prepared statement cached → <2 ms
  for ≤200 seed ids on M-series.

## Tests
- `multi_hop_neighbors` returns empty on no seeds / `max_hops=0`.
- `recall` with `max_hops=2` surfaces 2-hop docs absent from base lex+vec.

## Source mining
- Pattern: `petgraph::visit::Bfs` (re-implemented inline to avoid the dep).
- Attenuation factor: langchain `MultiHopRetriever` default 0.6.
