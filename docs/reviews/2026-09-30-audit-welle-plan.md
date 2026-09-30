# Audit-Welle 2026-09-30 — Synapse Memory Experten-Audit

## Strukturregeln (gelten für alle Jobs)

- Repo: `/Users/master/BASE/projects/synapse-memory`, Branch `main`
- Read-only-Audit: **keine Code-Änderungen** in dieser Welle — Findings → bd Issues
- Report-Vertrag: ≤ 400 Wörter + Findings-Tabelle (ID · Severity · Befund · Beleg-Pfad:Zeile · Fix-Vorschlag · Oracle-Idee)
- Keine Fragen — entscheiden, Annahme dokumentieren
- Verboten: rm, git reset, force-push, Schema-Änderungen an brain.db

## Bekannte Vor-Befunde (Synapse-Recall)

- doc 319166: 4 Hypothesen unverifiziert — **Cache-Atomizität, Path-Handling, Source-Validation, CLI/MCP-Parity**
- cli-log 349918: `just check` rot unter Last durch 2 flaky Perf-Asserts (`rabitq-latency`, `put_batch-throughput`) — isoliert grün, Korrektheit ok → **Gate-Kalibrierung** ist das Finding
- cli-log 349918: synx daemon offline → `queue.jsonl` Writeback-Stau (unbekannt ob gefixt)
- CLAUDE.md sagt Branch `split-memory` — tatsächlich `main` → Doc-Drift

## Panel A — Performance & Architektur (Agent: explore)

| ID | Auftrag | Akzeptanz |
|----|---------|-----------|
| A1 | Hot-Path-Audit: put/search/context-Pipeline in synapse-core, -engine, -kernel, -fts, -quant — Allocs, I/O, Locks, N+1-Queries | Top-10 Perf-Befunde mit Pfad:Zeile + geschätzter Kostenklasse (init/CPU/alloc/IO/RTT/lock) |
| A2 | Layer-Verletzungen gegen ARCHITECTURE.md + `just check-layers` Scope | Liste tatsächlicher Verletzungen |
| A3 | Flaky-Perf-Asserts rabitq-latency + put_batch-throughput: Ursache, Kalibrierungs-Fix | Konkreter Fix-Vorschlag mit Datei |
| A4 | Cold-init & Daemon-Pfad: /tmp/synapse.sock :9477, brain.db Fallback — Latenz-Budget | Messbare Empfehlungen |

## Panel B — Security & Enterprise-Robustness (Agent: explore)

| ID | Auftrag | Akzeptanz |
|----|---------|-----------|
| B1 | synapse-crypto (ed25519 sign/verify), synapse-rbac, synapse-audit, synapse-compliance — echte Lücken vs. Theater | Findings mit Severity + Beleg |
| B2 | vendor/synapse-db Provenance + cargo-deny/audit-Abdeckung, unsafe-Blöcke | Liste ungedeckter Risiken |
| B3 | Socket-Sicherheit /tmp/synapse.sock (Auth? Permissions? PID-Hijacking?), PII im Brain (DSGVO), Injection-Flächen (FTS-Queries, Import-Formate) | Findings + Fix-Vorschläge |
| B4 | Enterprise-Robustheit: Fehlerbehandlung, Grenzen (max doc size, memory pressure), Backup/Restore-Konsistenz | Top-5 Robustheits-Findings |

## Panel C — UX, Flows & Kundensimulation (Agent: explore + web)

| ID | Auftrag | Akzeptanz |
|----|---------|-----------|
| C1 | CLI-Ergonomie synx: Konsistenz der ~30 Subcommands, Defaults, Exit-Codes, Fehlermeldungen | Top-UX-Befunde |
| C2 | Docs-vs-Realität: README/SPEC/SYNAPSE-ULTRA vs. tatsächliche Features (verifiziert im Code) | Drift-Liste |
| C3 | Onboarding-Flow `synx onboard` + MCP-Integration + eval/-Harness | Flow-Lücken |
| C4 | Kundensimulation: 3 Personas (Solo-Dev, Team-Lead Enterprise, OSS-Contributor) bewerten vs. mem0/zep/letta — was sagen sie, warum | Persona-Verdicts mit Begründung |

## Nach der Welle (main session)

1. Findings → bd Issues mit Oracle (`--acceptance`/`oracle:`)
2. Top-Fixes implementieren (bounded slice): Doc-Drift, Gate-Kalibrierung falls trivial, schnelle Security-Wins
3. `just check` → Commit
4. Handoff + Synapse-Writeback
