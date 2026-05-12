# Synapse-X — Backend Testing & Deep-Analysis Discipline

> Extends SYNAPSE-X-DESIGN.md. Covers: test pyramid, deep-analysis primitives, regression gates, ML-discipline. Without this, 100× claims are vibes.

## 1. Test Pyramid

```
                  L8 Antifragile (kill-mid-write, gossip-resync)
              L7 Bench-as-Test (criterion, gates, M4 runner)
           L6 Cross-Language ABI (Rust↔Py↔TS↔MCP byte-equal)
         L5 Soak (24h ingest+query, no leak, p99 ≤ budget)
       L4 Snapshot/Replay (bagger-corpus, git-bisect-ready)
     L3 Differential vs DuckDB+SQLite (must agree ≤1e-6)
   L2 Property + Fuzz (proptest, arbitrary, cargo-fuzz)
 L1 Unit (per-encoder, per-decoder, per-Welford)
```

Each PR runs L1-L4 + ML-L1-L5. L5-L8 nightly. L7 weekly + before-release.

## 2. Crate Layout (test infra)

```
crates/synapse-market/
├── tests/
│   ├── integration_diff.rs   # L3 differential
│   ├── proptest_pages.rs     # L2
│   ├── snapshot_bagger.rs    # L4
│   └── abi_cross_lang.rs     # L6
├── fuzz/
│   ├── fuzz_targets/
│   │   ├── page_decode.rs
│   │   ├── tick_ingest.rs
│   │   └── book_delta.rs
├── benches/
│   ├── w1_candle_range.rs
│   ├── tick_replay.rs
│   ├── ann_pagelocal.rs
│   ├── corr_matrix.rs
│   └── mixed_workload.rs
└── chaos/
    ├── kill_midwrite.sh
    ├── io_fault.sh
    └── cluster_failover.sh
```

## 3. Differential Test Pattern (the CRUCIAL one)

```rust
#[test]
fn candle_range_matches_duckdb_and_sqlite() {
    let fixture = load_fixture("bagger_220_15m.smx"); // also generate matching duckdb+sqlite
    
    let mut rng = StdRng::seed_from_u64(42);
    for _ in 0..1000 {
        let ticker = pick_ticker(&mut rng);
        let (start, end) = pick_range(&mut rng);
        
        let smx_bars = mkt.candles(ticker, "15m").range(start..end).collect();
        let ddb_bars = duckdb_q!("SELECT * FROM candles WHERE ticker=? AND ts BETWEEN ? AND ?", ticker, start, end);
        let sqlite_bars = sqlite_q!("SELECT * FROM candles WHERE ticker=? AND ts BETWEEN ? AND ?", ticker, start, end);
        
        assert_eq!(smx_bars.len(), ddb_bars.len(), "row-count diff");
        for (a, b) in smx_bars.iter().zip(ddb_bars.iter()) {
            assert!((a.close - b.close).abs() < 1e-6, "close mismatch ticker={ticker} ts={}", a.ts);
            // similar for o/h/l/v
        }
        assert_eq!(smx_bars, sqlite_bars, "sqlite mismatch ticker={ticker}");
    }
}
```

→ catches every dtype-loss / off-by-one / ts-rounding bug. **Non-negotiable.**

## 4. Property Tests (proptest)

```rust
proptest! {
    #[test]
    fn page_roundtrip(rows in vec(arb_candle(), 1..50_000)) {
        let mut p = Page::new();
        for r in &rows { p.append(r); }
        let bytes = p.serialize();
        let p2 = Page::deserialize(&bytes).unwrap();
        let decoded: Vec<_> = p2.iter().collect();
        prop_assert_eq!(rows, decoded);
    }
    
    #[test]
    fn hilbert_zorder_preserves_locality(pts in vec((0..1_000_000_i64, 0.01..1000.0_f32), 2..1000)) {
        let zorder_distances: Vec<_> = pts.windows(2).map(|w| hilbert(w[0]) - hilbert(w[1])).collect();
        let euclid_distances: Vec<_> = pts.windows(2).map(|w| euclid(w[0], w[1])).collect();
        let correlation = spearman(&zorder_distances, &euclid_distances);
        prop_assert!(correlation > 0.7, "Hilbert lost locality, corr={correlation}");
    }
    
    #[test]
    fn online_ftrl_matches_batch_after_n_steps(samples in vec(arb_labeled(), 100..1000)) {
        let mut online = FtrlFfm::new();
        for s in &samples { online.update(s); }
        let batch_w = batch_lr_fit(&samples);
        let cosine = cosine_sim(online.weights(), batch_w);
        prop_assert!(cosine > 0.9, "online drift, cos={cosine}");
    }
}
```

## 5. Fuzz Targets

```rust
// fuzz/fuzz_targets/page_decode.rs
#![no_main]
use libfuzzer_sys::fuzz_target;
fuzz_target!(|data: &[u8]| {
    let _ = synapse_market::Page::deserialize(data);  // must never panic
});
```

→ Cargo-fuzz 24h-soak before any release.

## 6. Bench-as-Test (acceptance-gate)

`benches/gates.rs`:
```rust
fn assert_gate<F: FnMut()>(name: &str, target_p50_us: u64, mut f: F) {
    let mut times = vec![];
    for _ in 0..1000 { let t = Instant::now(); f(); times.push(t.elapsed().as_nanos() as u64); }
    times.sort();
    let p50 = times[500] / 1000;
    println!("{name}: p50={p50}us target={target_p50_us}us");
    assert!(p50 <= target_p50_us, "GATE FAIL: {name} p50={p50}us > {target_p50_us}us");
}

fn main() {
    assert_gate("candle_range_60d", 500, || { mkt.candles("RGTI","15m").range(60d).collect(); });
    assert_gate("similar_top5", 500, || { mkt.signal("RGTI",id).similar(5); });
    assert_gate("corr_matrix_220", 6_000, || { mkt.correlation_matrix(&tickers, 60.days()); });
    assert_gate("ingest_50k", 20_000, || { mkt.candles("RGTI","tick").append(&batch_50k); });
}
```

PR fails if any gate trips. Hard gate.

## 7. Soak Test (nightly cron)

```bash
# chaos/soak_24h.sh
cargo run --release --bin soak -- \
    --ingest-rate 50_000/sec \
    --query-mix 80/15/5 \
    --duration 24h \
    --memory-limit 4G \
    --fd-limit 100 \
    --assert-p99-le 5ms
```

Failure-modes catched:
- memory leak (heaptrack monitor)
- fd leak (lsof poll)
- compaction-stall blocks readers
- WAL-rotation glitch

## 8. Cross-Language ABI Test

```rust
#[test]
fn rust_py_ts_mcp_agree() {
    let query = ("RGTI", "15m", t_start, t_end);
    let rust_result = synapse_market::query(query);
    let py_result   = pyo3_query(query);     // via spawn python -c "..."
    let ts_result   = bun_ffi_query(query);  // via spawn bun -e "..."
    let mcp_result  = mcp_tool_call("smx_candles", query);
    
    assert_eq!(blake3(&rust_result), blake3(&py_result));
    assert_eq!(blake3(&py_result),   blake3(&ts_result));
    assert_eq!(blake3(&ts_result),   blake3(&mcp_result));
}
```

→ Forces dtype/precision/ordering parity. Catches if any binding accidentally rounds.

## 9. ML Test Discipline (separate suite)

| Test | What | Gate |
|---|---|---|
| **conformal-coverage** | nominal 80% → empirical | 78-82% on 1k holdout |
| **online↔batch-parity** | FTRL N updates vs batch-train N | cosine ≥ 0.9 |
| **leakage-audit** | randomize y, train, check AUC | AUC drop to 0.5±0.05 |
| **drift-monitor** | feature KL-div day-to-day | alert if >0.2 |
| **two-failure-rule** | wrong on same setup 2× | auto-quarantine model |
| **causal-validity** | parallel-trends pre-treatment | p>0.05 |
| **prompt-regression** | embedding-model change → eval-set | scores ±2% |
| **shap-stability** | top-10 feat-imp Jaccard | ≥0.7 |
| **adversarial-robust** | ±1σ feature noise 5% rows | prediction-flip ≤10% |
| **deterministic-seed** | same seed → same output | bit-equal across runs |

Lives in `tests/ml_discipline.rs`. Runs in nightly CI. Failed tests block release-tag (not PR-merge — would be too strict for exploration).

## 10. Deep-Analysis Primitives (the killer query layer)

Test these specifically because they don't exist in SQL/Parquet/DuckDB:

### A. Tick-Replay-to-Time
```rust
mkt.symbol("RGTI").replay_to(t).book_state()  
// = full L2 book at exact tick t
// gate: <10µs p50, <50µs p99
```

### B. Causal-Effect "what-if-no-news"
```rust
mkt.symbol("RGTI")
   .causal_effect(treatment: news_event_id, horizon: 30.min)
   .doubly_robust_streaming()
// gate: <100ms on 1M ticks
```

### C. Similar-Setup-Search
```rust
mkt.signal_universe.find_similar(current_setup, n: 20)
   .filter(|s| s.outcome.was_winner())
   .group_by(|s| s.pattern)
// "find me 20 historical winners that looked like RGTI today"
// gate: <5ms on 100k past signals
```

### D. Online Drift-Trajectory
```rust
mkt.symbol("RGTI").feature("micro_ofi").drift_trajectory(30.days)
// KL-div per day vs 90d baseline
// gate: <50ms on 8M ticks
```

### E. Conformal-Backtest
```rust
mkt.strategy_eval(my_strat, last_2y)
   .with_conformal(0.80)
   .nominal_vs_empirical_coverage()
// gate: <2s on 2y minute-bars × 220 tickers
```

Each gets dedicated test + bench. No regression > 10% of last-release floor.

## 11. Test Data Strategy

- **Synthetic**: parameterized generators (volatility-regime, news-cluster, halt-period)
- **Replay-corpus**: bagger.db exported to `.smx` fixtures (committed in `tests/data/`, gitlfs if >5MB)
- **Public-tick**: TAQ samples or Polygon free-tier dumps (license-checked)
- **Adversarial**: hand-crafted edge-cases (DST-shift, leap-second, zero-volume tick, halt-resume)

Each test categorizes the data-set it uses → bench-history tags by corpus.

## 12. CI Pipeline

```yaml
on: pull_request
jobs:
  fast:    # ≤5min
    - cargo nextest run -p synapse-market --no-fail-fast
    - cargo clippy -p synapse-market -- -D warnings (new code only)
    - L3 differential (small corpus)
  medium:  # ≤15min
    - L4 snapshot replay
    - L7 bench-gates
    - L6 cross-language abi
  slow:    # nightly
    - L5 soak (24h)
    - L2 fuzz (1h cargo-fuzz)
    - ML discipline full
    - L8 chaos
```

PR can merge with `fast + medium` green. Release-tag needs `slow` green from last 7 nights.

## 13. Bench History DB

Every CI bench-run writes one row to `bench-history.db` (SQLite, gitlfs):

```sql
CREATE TABLE bench_runs(
  ts INTEGER, commit_sha TEXT, bench_name TEXT,
  p50_us REAL, p95_us REAL, p99_us REAL, mean_us REAL,
  corpus_tag TEXT, machine_tag TEXT, notes TEXT
);
```

Trend-chart in `docs/bench-trends.html` auto-rendered. Regression alert if rolling-7d p50 > 1.1× of release-floor.

## 14. Hebelwort applied to Testing

- **compounding > one-shot**: bench-history accumulates, each commit improves baseline
- **zero-friction > clever**: differential-test is brain-dead-simple, catches everything
- **antifragile**: chaos tests get harder over time, system gets more robust
- **moat > feature**: nobody else tests cross-language byte-parity → only we trust the data shape
- **default-alive**: every bench has a gate; can't ship broken
- **10x not 10%**: regression gate is fail-on-1.1×, not fail-on-2× (no slow-creep)

## 15. Honesty Gate (the Truth-Test)

If any of these silently regress without alerting → testing is theater:
- p50 latency on critical path
- Memory footprint per page
- Compression ratio achieved
- ML AUC on held-out set
- Conformal coverage empirical
- Cross-language result-hash

All 6 → bench-history dashboard with red/green per-day. Slack/Telegram-alert if any goes red.

---

**TL;DR**: testing isn't a checklist, it's an antifragile system. Differential-vs-DuckDB+SQLite catches 80% of bugs. Bench-as-gate prevents perf-rot. Cross-lang byte-parity prevents binding-drift. ML-discipline catches model-rot. Nightly soak catches resource-leak. Everything else is bonus.
