# Spec: RAM-first WAL reader budget

## Status, evidence, and proof boundary

This is an implementation specification, not a claim that the daemon is now
faster or that the retained WAL is a current write rate.

Observed read-only at 2026-07-29 17:02 CEST:

- `/Users/master/.synapse/brain.db`: 1,008,357,376 bytes; sibling WAL:
  667,699,592 bytes; SHM: 1,310,720 bytes.
- `lsof` showed only `synapsed` PID 4723 holding the three files. It does not
  identify an SQLite snapshot/end-mark, a checkpoint blocker, or write bytes.
- `crates/synapse-core/src/db.rs:Store::open` sets WAL, `synchronous=NORMAL`,
  1 GiB `mmap_size`, 256 MiB `cache_size`, and `wal_autocheckpoint=0`.
- `crates/synapsed/src/main.rs:State::sql_conn` is one pre-opened read-only
  connection with its own 1 GiB mmap and 256 MiB cache. `State` also has the
  writer `Store` and a startup index-builder read connection. The MCP
  `PackCache` is already a bounded 64-entry LRU; this work must not replace it.

Therefore the likely causes are deliberately **unproven**: disabled automatic
checkpointing, one of Synapse's own readers retaining a snapshot, an external
reader missed by a point-in-time `lsof`, or genuine write accumulation. Do not
add a checkpoint, VACUUM, integrity scan, full DB scan, or cache warmup to a
request path. A retained WAL alone is insufficient evidence for any of them.

## Goal and non-goals

Goal: give `synapsed` a bounded, observable, short-lived read-only SQL reader
so repeated RAM-resident reads remain fast without a long-lived reader silently
pinning WAL frames or reserving an unbounded process-memory budget.

Non-goals:

- no schema, journal-mode, durability, or writer batching change;
- no automatic/manual checkpoint, VACUUM, truncation, or DB deletion in the
  daemon hot path;
- no claim to repair a WAL before the diagnosis gate below passes;
- no change to `synapse-mcp` `PackCache`, ANN index semantics, reranker output,
  or cross-process cache sharing;
- no dependency addition.

Reach ladder: level 2 (architecture/lifecycle), not a micro-tuning of SQLite.

## Required diagnosis gate before any WAL mutation

This gate is a release blocker, not a best-effort log line. Its report is
ephemeral JSON or a caller-chosen output file; it must never be inserted into
`brain.db`.

1. Take two file-stat snapshots at a fixed 60-second interval: DB/WAL/SHM
   inode, size, mtime, PID, elapsed time, RSS, and open FDs from `lsof`.
   Calculate only the observed size delta; label it `wal_size_delta`, never
   `write_rate`.
2. Enumerate every process with the three files open. For `synapsed`, report
   the process-owned reader registry. It contains connection id, operation, age,
   active statement count, and last completed statement duration. `lsof` alone
   cannot prove which reader pins a WAL end-mark.
3. Record read-only facts only: `PRAGMA journal_mode`, `synchronous`,
   `wal_autocheckpoint`, `page_size`, `data_version`, and the currently loaded
   daemon config. Open this probe with `SQLITE_OPEN_READ_ONLY`; do not call any
   `wal_checkpoint` pragma in this phase.
4. Classify exactly one result: `external_reader`, `daemon_reader`,
   `writer_accumulation`, or `unknown`. Multiple open reader FDs without an
   active statement remain `unknown`, not a blocker verdict.
5. Only an explicit maintenance command, after a clean backup and a quiescent
   reader registry, may attempt a PASSIVE checkpoint and record SQLite's
   returned busy/log/checkpoint frame counts. TRUNCATE needs separate operator
   approval. Failure or `busy > 0` is a HALT, never a retry loop.

The implementation must expose this as a diagnostic/maintenance surface,
separate from normal socket requests. It must not execute it at daemon start.

## Architecture and lifecycle

### Exact surfaces

- Modify `crates/synapsed/src/main.rs`: `Cli`, `State::sql_conn`, SQL request
  dispatch, `hydrate_pairs_from_state`, and startup reader construction.
- Add `crates/synapsed/src/wal_reader.rs`: `ReaderBudget`, `ReaderRegistry`,
  `SqlReader`, `SqlReaderLease`, `WalObservation`, and clock-injectable
  `ReaderReaper`.
- Add `crates/synapsed/tests/wal_reader_budget.rs` and
  `crates/synapsed/tests/wal_diagnosis.rs`.
- Add a documented maintenance-only CLI subcommand in
  `crates/synapse-cli/src/main.rs` (for example `Wal Diagnose` / `Wal
  Checkpoint`), with `Checkpoint` refusing unless an explicit confirmation
  flag is present. The daemon never invokes it.

Replace the bare `PlMutex<Option<rusqlite::Connection>>` field with
`PlMutex<SqlReader>`. It owns at most one read-only connection, its immutable
budget, a last-used instant, and the registry. A `SqlReaderLease` is acquired
only around one SQL/hydration operation. It records start/end and statement
count, makes statement scope lexical, and drops every `Statement`/`Rows`
before releasing the lease. No SQL request may return a `Rows`, iterator, or
transaction beyond its lease.

Open lazily on the first SQL/hydration request with
`SQLITE_OPEN_READ_ONLY | SQLITE_OPEN_URI`; set `query_only=ON`, `temp_store=2`,
and the configured mmap/page-cache values. Keep autocommit on; forbid an
explicit `BEGIN`, attach, pragma mutation, and non-read SQL on this connection.
Existing SQL allow-list/limits remain authoritative. A failed reader open or a
read timeout returns the current request error; it must not fall back to a
write-capable connection.

Use a conservative, visible default budget for **this read-only connection**:
256 MiB mmap address window and 32 MiB SQLite page cache. These are caps for
this new reader's configured SQLite allocation/address map, not a false claim
of exact process RSS. The writer's current settings are unchanged. Expose
`--sql-reader-mmap-bytes` (0..1 GiB), `--sql-reader-cache-kib` (1..65536), and
`--sql-reader-idle-ms` (30,000..3,600,000; default 300,000). Reject invalid
values at CLI parsing; no environment variable silently overrides a CLI value.

After every lease, update `last_used`. Before leasing and after a completed
request, if no lease is active and idle age reaches the configured timeout,
drop the connection, clear its registry entry, and log a counter only. Dropping
releases SQLite's read state and bounded cache; the next request opens a new
reader. Do not run `empty_cache` equivalents, write a trace per success, or
force OS page-cache eviction. The startup index builder is separately bounded:
one read-only connection, no shared `SqlReader`, dropped immediately after the
index build succeeds, fails, or is cancelled.

The writer remains the existing single `Store` connection. Writes invalidate
query/cache state exactly as today but must not checkpoint or synchronously
rebuild the reader. Readers may run concurrently only through this one
serialized reader connection; increasing reader count is out of scope until a
measured throughput gain exceeds the resource and WAL gates.

## Durability, recovery, and migration

WAL plus `synchronous=NORMAL` remains the durability contract. A crash before
or during a lease is safe. The reader has no write transaction, so SQLite recovers
the committed WAL on next writer open. A reader-open/configuration failure is a
normal request failure, not a reason to downgrade durability.

Migration is source-compatible and config-default-only. Deploy with defaults,
run diagnosis, then compare the vectors below. Back out by disabling the new
reader lifecycle behind its feature/config gate and restoring the existing
pre-opened reader behaviour; do not touch DB files or perform a compensating
checkpoint. Roll back immediately on oracle failure, any external reader
classification, increased WAL blocker age, or rejected resource vector.

## Terra implementation slices

1. **Probe only.** Add `WalObservation` and the CLI diagnosis output. Exercise
   it against a temp WAL DB and the live DB in read-only mode. No lifecycle
   change and no checkpoint command execution.
2. **Bounded reader.** Introduce `ReaderBudget`/`SqlReaderLease`, lazy open,
   read-only SQL guard, and one reader cap. Preserve byte/value results from
   current SQL and vector hydration tests.
3. **Reap.** Add clock-injected idle drop and registry telemetry. Prove a lease
   never outlives a request and the next lease reopens cleanly.
4. **Maintenance boundary and benchmark.** Add opt-in checkpoint command plus
   two-phase diagnosis test; run the sequential cold/warm/resource runner.
   Ship only if every reject threshold passes.

## Tests, oracle, and proof plan

Identity oracle: on a fixed temporary WAL fixture and fixed search vectors,
old versus new SQL result JSON (ordering, values, snippets, errors) must be
byte-identical; `SearchVec` hydration IDs/snippets must match exactly. Reopen
after forced reader drop and after process kill; committed rows must match the
pre-kill count. The feature must not produce a checkpoint, DB-size rewrite, or
write transaction during ordinary reads.

Required tests:

- `wal_reader_budget.rs`: read-only flags/`query_only`, config range rejection,
  one-connection cap, statement drop before lease release, idle reap/reopen,
  and concurrent callers serialize without a leaked registry entry.
- `wal_diagnosis.rs`: two snapshots calculate a labelled size delta, never call
  a checkpoint in diagnose mode, report an injected external holder as
  `external_reader`, and refuse checkpoint without explicit confirmation.
- Existing `synapsed` request tests: SQL SELECT, ANN `SearchVec` hydration,
  writer invalidation, and a crash/reopen fixture.

Benchmark sequentially, on the same DB copy and fixed query corpus, with
`--warmup 3 --runs 10` (or an equivalent runner that emits raw samples):

1. cold daemon start to first SQL/hydration response;
2. warm repeated SQL/hydration p50/p95/p99;
3. one-reader throughput at fixed concurrency 1, then separately the existing
   normal search path; no multi-reader claim;
4. resource vector: daemon max RSS, reader-registry max active/age, mmap/cache
   config, swap delta, DB/WAL/SHM size deltas, process/FD count, and energy if
   available.

Record before and after in separate JSON artifacts with command, commit, DB
fixture checksum, config, samples, and timestamp. Do not compare the live DB
to a copied fixture as though they were one benchmark.

Reject the slice if any identity/recovery test fails; an ordinary read changes
DB/WAL bytes; more than one lifecycle reader is open; idle reap fails within
idle timeout plus 5 seconds; p95 worsens by more than 10%; max RSS rises by
more than 10% plus 128 MiB; swap increases; a retained-reader age exceeds the
configured idle limit; or diagnosis remains `unknown` while a WAL mutation is
proposed. No speed target is accepted without these before/after samples.

## Terra build capsule (<=700 tokens)

Implement only the four slices above. Start with a read-only WAL observation:
two 60-second stats plus `lsof`, daemon reader leases, and read-only pragmas.
Do not call checkpoint, VACUUM, or integrity scan. Current evidence shows a
1.008 GB DB and 667.7 MB WAL, all held by one `synapsed`; that is not a causal
diagnosis. Replace `State::sql_conn` with one lazily opened `SqlReader` that
has `query_only`, 256 MiB mmap, 32 MiB cache, lexical `SqlReaderLease`, and a
five-minute idle drop. Writer settings and the existing 64-entry MCP
`PackCache` stay unchanged. Add temp-WAL tests for guard, cap=1, reaping,
diagnosis/no-mutation, result identity, and crash recovery. Benchmark cold,
warm, one-reader throughput, RSS/swap/FDs/WAL delta sequentially with ten
runs. Reject if data changes, reader count exceeds one, p95 >10% worse,
RSS >10%+128 MiB, swap rises, reaping fails, or a checkpoint is requested
while diagnosis is unknown. Maintenance checkpoint is an explicit CLI action
after quiescence and backup, never daemon behaviour.
