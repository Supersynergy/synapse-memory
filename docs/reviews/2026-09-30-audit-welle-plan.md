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

---

# Ergebnis-Konsolidierung (2026-09-30, post-Welle)

## Gate-Status

| Gate | Stand |
|------|-------|
| `just check` (layering + fmt + clippy -D warnings + cargo check) | **grün** (Commit c90afbc) |
| `cargo nextest --workspace` | **472 passed, 0 failed, 2 skipped** |
| `cargo clippy --all-targets --all-features` | rot durch `import-parquet` (fix committed, synapse-memory-ajs) + Rest-Drift |

Gate-Lauf entlarvte 3 echte Build-Bugs, die vorher nie kompilierten: `synapse-router` tempfile-Dep (P1, -2fg), `synapse-cli` import-parquet (P1, -ajs), `fts-tantivy` cfg-Name (P3, -4to). Lehre: `--all-features` und lib-Targets waren dauerhaft ungetestet — CI-Matrix-Lücke ist Meta-Finding.

## Panel A — Performance & Architektur (Top-10, verifiziert)

Kern-Befund: **Produktion läuft über `vendor/synapse-db/crates/synapse-core`; die Member-Kopie `crates/synapse-core` ist ein unreferenzierter divergenter Zwilling** (~100 Zeilen Drift, z. B. fehlt der Dedup-Block aus vendor db.rs:1539–1549). Gates bauen+testen toten Code — `check-layering.py` sieht das nicht.

| Sev | Befund | Beleg | bd |
|-----|--------|-------|-----|
| P0 | `put()` = `SELECT COUNT(*)` pro Insert → O(n²) Ingest | vendor db.rs:684 | -bwk |
| P0 | `NdArraySearch::add_row` = Vollmatrix-Concat + `ids.contains` O(n) | ndarray_search.rs:362; synapsed main.rs:1001 | -yxy |
| P1 | tantivy `commit()` pro put **und** pro `search_lex` + `MAX(id)` + json-Schreiben je Suche | vendor db.rs:785, 1523 | -kqu |
| P1 | N+1-Hydration in `search_lex` und `search_hybrid_hippo` | vendor db.rs:1555; crates db.rs:2030 | -c0c |
| P1 | `log_query`-INSERT je Suche unter globalem Store-Mutex | synapsed main.rs:1046 | -423 |
| P1 | Ein `PlMutex<Store>` serialisiert alles; `put_batch` hält Lock über Batch | synapsed main.rs:91, 632 | -4qs |
| P1 | `Store::open` synchron vor Socket-Bind (migrate + tantivy-Warmstart aller Docs + ANN-Tail) | synapsed main.rs:264; db.rs:517 | -w4i |
| P1 | Divergente synapse-core-Zwillinge + Layer-Guard-Blindspot | synapsed Cargo.toml:24 | -cpd |
| P1 | Flaky Perf-Asserts: Wallclock-Floors + Ground-Truth im Timer | crates db.rs:2436; rabitq_index.rs:289 | -gk6 |
| P2 | `wal_autocheckpoint=0` ohne Checkpoint-Mechanismus → WAL unbounded | db.rs:342 | -sbr |

Bonus-Verifikationen (doc 319166, -dn8): Cache-Atomizität **real** (MCP PACK_CACHE nie invalidiert, -hab) · Path-Handling **real** (SnapMerge unsanitisiert, -o0i) · CLI/MCP-Parity **real** (`memory_save` embed:false → unsichtbar für Vec/Hybrid, -2ej) · `queue.jsonl`-Writeback nicht im Code gefunden (unverified).

## Panel B — Security & Enterprise (Auszug)

Sauber: FTS5-Tokenizer-Escaping, Snap-Path-Confining, Tenant-Validierung, CSV-Escape, Idempotent-Delete, License-JWT + Grace-Cache.

Lücken (alle bd-getrackt): Socket ohne Permissions/Auth-default-off (-cx4) · SnapMerge-Pfade (-o0i) · Raw-SQL ATTACH-Bypass + kein Timeout (-4oi) · Compliance-Export gegen falsche Spalten `docs.agent`/`docs.content` (-c1c) · Audit/RBAC/Provenance nicht in Prod-Pfade gewired (-5dm) · Secrets/Brain-Dateien ohne 0600 (-lwn) · LiveQuery-WS ohne Auth/Origin (-uiz) · Vendor-Provenance fehlt (-ug6) · Brainpack-Parser unbounded + Signatur optional (-5ep) · DoS-Flächen: Frames/Metadaten/Titel/Imports (-gqd) · Federation vertraut Self-Signed-Keys aus der Message (-j1u) · Provenance-Chain stoppt an Root (-ymy) · `debug_assert` für Vec-Länge + wiederholte Auto-Extension-Registrierung (-87y) · Non-root Cargo-Profile ignoriert (-77x).

## Donor-Recherche (ghmax)

Überwiegend Rauschen; ein verwertbares Muster: **Aleph** `hybrid_search_notes` liefert pro Leg Candidate-Counts + `SearchAdvisory` ("vector leg didn't participate" ≠ "fand nichts"). Für Synapse: Hybrid-Response könnte `legs: {fts: n, vec: n, fused: rrf}` liefern — hilft Agenten, Fallbacks zu unterscheiden. → Follow-up-Issue.

## Roadmap (Wellen)

- **W1 (P0, diese Session)**: Gate grün ✓ · Doc-Drift ✓ · Issues angelegt ✓
- **W2 (P0/P1 Security)**: Socket-Härtung+Auth-default (-cx4), SnapMerge-Sanitize (-o0i), SQL-Authorizer+Timeout (-4oi), 0600-Permissions (-lwn), LiveQuery-Auth (-uiz). Blocker für jede Enterprise-Story.
- **W3 (P0/P1 Perf)**: COUNT(*)-Ingest (-bwk), add_row-Matrix (-yxy), tantivy-Commit-Batching (-kqu), N+1-Hydration (-c0c), Store-Concurrency (-4qs/-423). Kunde merkt: Ingest 100× schneller.
- **W4 (P1 Wiring)**: Audit/RBAC/Compliance in Daemon-Mutationen (-5dm, -c1c), MCP embed:false (-2ej), Pack-Cache-Invalidierung (-hab), WAL-Checkpoint (-sbr), Startup-async (-w4i).
- **W5 (P2)**: Federation-Trust (-j1u), Brainpack-Härtung (-5ep), Provenance-Chain (-ymy), Vendor-Provenance (-ug6), DoS-Bounds (-gqd), Twin-Auflösung (-cpd), Hybrid-SearchAdvisory.
- **W6**: Load-Benchmarks mit stabilen Thresholds (-gk6), CI-Matrix für --all-features + Lib-Targets.
