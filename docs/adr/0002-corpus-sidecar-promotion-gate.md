# ADR 0002: Corpus Sidecar With Promotion Gate

## Status

Accepted.

## Context

Synapse durable memory is optimized for verified facts, decisions, and context
packs. Raw knowledge feeds are different: RSS posts, YouTube transcripts, PDFs,
and web pages can be useful for retrieval before they are trustworthy enough to
become durable agent memory.

Dumping raw corpus content directly into the main memory path increases false
recall risk and makes stale or low-confidence material look equivalent to
verified decisions.

## Decision

Add a `synapse_corpus_*` sidecar schema inside the same SQLite brain file:

- `synapse_corpus_sources` records raw source streams.
- `synapse_corpus_documents` records deduplicated source items.
- `synapse_corpus_chunks` plus FTS5 stores retrievable passages.
- `synapse_corpus_vec` optionally stores chunk embeddings for vector retrieval.
- `synapse_corpus_promotions` gates raw chunks before they can become durable
  facts or decisions.

Corpus retrieval uses the same direction as the main memory system: FTS5/BM25
and optional vector candidates are fused with RRF. A small eval helper reports
Recall@5, MRR, and false-recall rate so retrieval changes can be measured before
promotion policy changes.

## Consequences

Raw material becomes searchable without polluting durable memory.

Promotion remains explicit: queued chunks are invisible to ready-promotions until
verified. A caller can then convert verified promotions into regular `remember`
or `put` entries with existing metadata/signing flows.

The first implementation supports manual text ingest and CLI evaluation. Feed
fetchers for RSS, YouTube, PDF, and web URLs can build on the same schema without
changing the durable memory contract.
