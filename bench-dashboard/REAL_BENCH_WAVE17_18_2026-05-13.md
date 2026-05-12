# REAL BENCH — Wave 17/18 — 2026-05-13 — M4 Max

## Methode
Alle Zahlen echte Messungen (release profile). Kein Mock, kein Copy-Paste.
CPU: M4 Max. OS: macOS Darwin 24.5.0.

---

## 1. synapse-tsdb — Insert 1M rows

| Workload | p50 | thrpt |
|----------|-----|-------|
| insert_1M (fallback store) | 234 ms | **4.26 M rows/s** |

Flush-to-disk / reload / query-range: kein separater Bench vorhanden (bench deckt nur insert).
Arrow/Parquet backend hinter `--features tsdb` — nicht gebenchmarkt (kein Criterion bench dafür).

**Fix nötig**: `fallback` war `mod` (private) → bench crashte. Gefixt → `pub mod fallback`.

**Verdict**: 4.26M insert/s solide für SQLite-backed store. Claim unprüfbar für Arrow-Pfad.

---

## 2. synapse-mlx-olap — GROUP BY SUM (CPU backend)

| Workload | p50 | Backend |
|----------|-----|---------|
| group_by_sum / 1M rows / 5 keys | 37.97 ms | CPU |
| group_by_sum / 10M rows / 5 keys | 395 ms | CPU |
| sum_scalar / 10M rows | 10.8 ms | CPU |

**Metal (MLX) backend**: `candle-core` feature aktiviert — aber `engine.backend` = `Cpu`.
Kein Metal-Tensor-Pfad aktiv auf diesem Run (benötigt candle-core Metal target).
DuckDB-Vergleich: nicht inline gemessen (DuckDB-binary nicht im Workspace).

**Throughput**:
- 10M GROUP BY 5 keys SUM: ~25M rows/s
- 10M scalar SUM: ~926M rows/s (SIMD-vektorisiert)

**Verdict**: CPU-Pfad gut. Metal-Claim unbewiesen — `backend` field zeigt `Cpu`, kein Metal-Dispatch sichtbar.

---

## 3. synapse-jit — Filter bench (1M rows, WHERE val > 500k)

| Impl | Zeit | Rows out |
|------|------|----------|
| interpreter | 8 ms | 499,999 |
| JIT (Cranelift) | 8 ms | 499,999 |
| SQLite in-memory | 18 ms | 499,999 |

**JIT speedup vs interp**: 1.0× (kein Gewinn)
**JIT vs SQLite**: 2.25× schneller als SQLite

Kommentar im Code behauptet `jit ~5ms / interp ~18ms / sqlite ~28ms` — das stimmt nicht.
Real: JIT = interp (beide 8ms). SQLite 18ms statt 28ms (M4 schneller als Basis-Hardware des Claims).

**Verdict**: JIT-Pfad funktioniert, kein Speedup vs. Interpreter. SQLite-Vorteil real (2×).

---

## 4. synapse-stream — Pub/Sub + CDC throughput

| Workload | Messung |
|----------|---------|
| publish 100k events | 7.6 ms → **13.1M events/s** |
| recv 100k events (single subscriber) | 45 ms total |
| per-msg publish latency | **76 ns/msg** |
| CDC emit_direct 100k (on-disk SQLite) | 44.6 s → **2,241 events/s** |

**Pub/Sub**: 76 ns/msg sehr gut. tokio broadcast-channel.
**CDC on-disk**: 2,241/s — jede emit_direct = separate SQLite write (kein WAL batch). Kein trigger-based Bench (kein real-table trigger setup im Bench).

**Verdict**: Pub/sub stark. CDC throughput gering (SQLite-write-per-event bottleneck).

---

## 5. synapse-graph — Datalog ancestor-closure

| Workload | Messung |
|----------|---------|
| 100 parent facts, semi_naive fixpoint | **7131 ms** |
| 100 pairs manual Rust | **3 µs** |
| Ratio (Datalog vs Rust) | **2.4M× langsamer** |

**100k facts**: >30s timeout — nicht fertig gelaufen.

Ursache: `semi_naive()` hat quadratische Komplexität in der vorliegenden Impl (nested HashMap join ohne Index).
100 facts → 7s ist inakzeptabel. Claim "Datalog ancestor-closure 100k facts" ist schlicht falsch.

**Verdict**: NICHT PRODUCTION-READY. Semi-naive braucht semi-naiven Delta-Join mit Relation-Index, nicht naive nested loop.

---

## Summary — Claims vs Reality

| Crate | Claim | Real | Status |
|-------|-------|------|--------|
| synapse-tsdb | 1M insert + flush + reload | 4.26M/s insert, rest ungebenchmarkt | ⚠️ partial |
| synapse-mlx-olap | Metal GPU GROUP BY | CPU only, kein Metal dispatch | ❌ claim unbewiesen |
| synapse-jit | JIT 3.6× interp | 1.0× (gleich schnell) | ❌ kein Gewinn |
| synapse-stream pub/sub | μs fanout | 76 ns/msg ✓ | ✅ |
| synapse-stream CDC | 100k trigger CDC | 2,241/s (SQLite bottleneck) | ⚠️ langsam |
| synapse-graph datalog | 100k facts ancestor | 100 facts = 7s, timeout bei 1k | ❌ broken |

---

## Sofort-Prios

1. **graph/datalog**: Semi-naive mit Delta-Relation + HashMap-Index. Ohne Fix kein Use.
2. **jit**: JIT warmup-Kosten prüfen; ggf. Batch-codegen für komplexe Queries nötig.
3. **mlx-olap**: Metal backend verifizieren (candle Metal feature + `device = Device::new_metal(0)`).
4. **stream/cdc**: WAL-batch-write oder async emit für >10k/s.
