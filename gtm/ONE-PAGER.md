# Synapse: the agent memory layer that survives an audit

## Problem

- mem0, Zep & Co. routen Embeddings + Roh-Prompts an Vendor-Clouds — DPA-Albtraum für regulierte Branchen.
- Closed-SaaS-Memory ist nicht air-gap-fähig; on-prem ist entweder unmöglich oder teuer-custom.
- OSS-Alternativen (Hindsight, Letta) speichern unverschlüsselt, kein License-Layer, kein Audit-Trail.

## Solution

- **SQLCipher-sealed `brain.db`** — AES-256, key aus Hardware-Fingerprint + License-JWT abgeleitet.
- **Ed25519 JWT-Licenses** mit hw-fp lock und 30d offline grace; License-Server selbst-hostbar.
- **0.023ms p50 hybrid query** auf 147k docs (sqlite-vec + FTS5 + recency-rerank, in-process Rust).

## Proof

- Bench: 3rd overall vs FAISS/FTS5/Chroma/LanceDB on 147k Wikipedia chunks. ~13× Chroma, ~62× LanceDB. (M4 Max, repro in `bench/`.)
- Per-customer-watermarked Binary mit BLAKE3 + minisign sidecar — traitor-tracing built-in.
- MCP-native: drop-in für Claude Code, Cursor, Continue.

> "Customer quote — placeholder" — <Customer Logo Strip Placeholder>

## CTA

Demo buchen: **true@supersynergy.de** · 30 Min · wir bringen Bench + Architektur-Review mit.
