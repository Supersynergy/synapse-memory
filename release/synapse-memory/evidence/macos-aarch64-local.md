# Synapse Memory local footprint

Generated `2026-07-13T21:24:29+00:00` on `Darwin 24.5.0 arm64` with
`synapse 1.0.1-rc.1`. CLI process startup is included in every latency.

| Metric | Result |
|---|---:|
| Binary | 3.19 MiB |
| Local copy install | 2.17 ms |
| Init | 7.39 ms |
| Remember p50 / p95 | 5.37 / 6.01 ms |
| First cited context | 6.95 ms |
| Warm context p50 / p95 | 5.78 / 6.56 ms |
| Peak RSS, one context | 4.61 MiB |
| SQLite bytes after 100 records | 204800 B |

Binary SHA-256: `d5f477570545afa4aaf663c3ae9edbf2dc16970e4af27935446191c1f4179645`.

Scope: local lexical portable build. No model, provider, network, daemon, or
competitor service. This is footprint evidence, not a cross-product recall claim.
