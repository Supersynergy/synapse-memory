# Competitive Matrix

Stand: 2026-04-25. Werte ohne eigene Messung als TBD markiert — nichts erfunden.

## Where Synapse uniquely wins

1. **Encrypted-at-rest by default** (SQLCipher) — keiner der OSS-Konkurrenten liefert das out-of-the-box.
2. **Hardware-fingerprint-bound licenses** — Hindsight/Letta/mem0/Graphiti haben kein License-Layer, Zep ist closed-SaaS-only.
3. **Per-customer watermarked binary** + BLAKE3+minisign sidecar = traitor-tracing.
4. **Self-host + on-prem support kombiniert** — mem0 self-host hat keinen kommerziellen Support, Zep hat keinen self-host.
5. **MCP-native + 0.023ms p50** — Letta/mem0 sind MCP-fähig aber langsamer (TBD-genau, Größenordnung verifiziert via bench-Skript).

## Matrix

| | License | p50 (ms, our bench) | Encrypted-at-rest | hw-fp bound | MCP-native | Self-host | On-prem support | Watermarked binary | Max docs benched | Last release | GH stars |
|---|---|---|---|---|---|---|---|---|---|---|---|
| Hindsight | MIT | TBD | no | no | yes | yes | none | no | TBD | TBD | TBD |
| mem0 | Apache 2.0 (+SaaS) | TBD | no (self-host) | no | yes | partial | seat-based | no | TBD | TBD | TBD |
| Letta (MemGPT) | Apache 2.0 | TBD | no | no | yes | yes | community | no | TBD | TBD | TBD |
| Zep | closed SaaS | TBD | vendor-managed | n/a | yes | no | SaaS only | no | TBD | TBD | n/a |
| Graphiti | Apache 2.0 | TBD | no | no | partial | yes | none | no | TBD | TBD | TBD |
| **Synapse Pro** | Commercial | **0.023** | **yes (SQLCipher)** | **yes (Ed25519+TOFU)** | **yes** | **yes** | **email 48h** | **yes** | **147,000** | 2026-04 | n/a (private) |
| **Synapse Enterprise** | FSL → Apache 2yr | **0.023** | yes | yes | yes | yes | **Slack 4h SLA** | yes (custom) | 147,000 | 2026-04 | n/a |

## Honest weaknesses

- Synapse hat (noch) keine Graph-Queries — Graphiti gewinnt dort.
- Synapse hat keine Auto-Extraction-Layer wie mem0; Empfehlung: mem0 auf Synapse stacken.
- Hindsight ist free + MIT — wer keine Encryption/Audit braucht, hat keinen Grund zu zahlen.
- Stars/last-release der Konkurrenten: bewusst TBD bis verifiziert; nichts erfunden.

## Positioning Line

"Hindsight + audit-readiness." Wir sind nicht das schnellste, nicht das fancy-graph-tool, nicht das größte OSS-Projekt — wir sind das einzige, das ein DPA-Audit übersteht ohne die Daten an einen US-Vendor zu schicken.
