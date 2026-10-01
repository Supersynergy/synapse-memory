# Verifikation der 4 Alt-Audit-Hypothesen (bd synapse-memory-dn8)

Datum: 2026-10-01 · Stand: main @ 45fd5e5 · Quelle: Audit-Hypothesen aus brain doc 319166.
Methode: Code-Inspektion der konkreten Stellen, keine Lasttests.

## 1. Cache-Atomizität — **CONFIRMED (bounded)**

`crates/synapsed/src/main.rs` `get_cached_hits` (~Z. 1720): Fingerprint
(mtime+len von `brain.db` UND `brain.db-wal`) wird gelesen, dann unter
`db_seen`-Lock verglichen und der Cache ggf. komplett gecleart.

- **Korrekt:** Fingerprint-Clear ist atomar ggü. anderen Lesern; lokale
  Writes clearen den Cache nach dem Store-Op (Put/Delete/Merge:
  Z. 830/838/1327/1375) — kein stale-serve nach eigenem Write.
- **Residual (bounded):** TOCTOU-Fenster zwischen `db_fingerprint()` und
  Cache-Read — ein externer Write, der genau dazwischen landet, kann eine
  veraltete Antwort liefern. Fenster = 2 `stat`-Syscalls; selbstheilend
  beim nächsten Read, da WAL-mtime sich ändert. Kein Datenverlust, nur
  kurzzeitig stale Suche — für eine Memory-DB akzeptabel; bei Bedarf
  später über `PRAGMA data_version` statt mtime härten (echte
  Commit-Erkennung statt FS-Heuristik).

## 2. Path-Handling — **CONFIRMED (minor)**

- `synx -f "~/x.db"` expandiert die Tilde **nicht** — ein quoteter
  Tilde-Pfad legt ein wörtliches `~/`-Verzeichnis relativ zum CWD an.
  Nur `synapse-extract` hat `expand_tilde`; CLI/Daemon/MCP nicht.
  Follow-up: kleine Expansion im CLI `-f`-Resolver oder dokumentieren.
- Default-Pfad `~/.synapse/brain.db` via `dirs_next::home_dir()` ist
  korrekt inkl. Windows (`%USERPROFILE%`); Legacy-Warnung vorhanden.
- Nicht-UTF8-Pfade: `PathBuf`-End-to-End, kein `to_str().unwrap()` auf
  User-Pfaden gefunden — ok.

## 3. Source-Validation — **RESOLVED durch bd -5ep/-j1u**

- Brainpack-Import: keine Netz-Fetches (kein SSRF); Parser seit -5ep
  längen-geprüft, zstd/decrypt gebondet, signierte Packs nur via
  `import_signed` mit Pflicht-Verifizierung. Unsigned Packs werden
  angenommen — gewolltes Format, Integrität via Sig optional.
- `source_uri` in Provenance ist reines Metadaten-Feld, wird nie
  dereferenziert — keine Injection-Fläche.
- Federation: seit -j1u werden Updates/Sync-Requests von unbekannten
  Keys rejected (TrustStore), SyncStep1 leakt keine Diffs mehr an
  Unauthentisierte.

## 4. CLI/MCP-Parity — **MOSTLY RESOLVED**

- Embed-Default: `memory_save`/`put` in MCP jetzt `embed=true` wie
  `synx put` (bd -2ej). Interne Feedback-Writes (agent-feedback,
  ctx-feedback) sind bewusst `embed=false` — Telemetrie, kein
  Suchkorpus: korrekt.
- Restliche Parity-Lücke = direkte brain.db-Zugriffe in MCP
  (bd -3qw, in Arbeit) — danach kanalisiert alles durch den Daemon.

## Fazit

Vier Hypothesen: 1× confirmed-bounded (dokumentiert, kein Fix nötig),
1× confirmed-minor (Tilde-Expansion → Follow-up-Issue), 1× resolved,
1× mostly-resolved (Rest = -3qw).
