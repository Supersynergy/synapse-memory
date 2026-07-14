//! Concurrent-read bench for `AdaptiveRouter` — validates the `RwLock` vs
//! `Mutex` claim.
//!
//! The `PyAdaptiveRouter` wrapper in `synapse-py` uses `parking_lot::RwLock`
//! so that many Python threads can call `choose()` (read) concurrently without
//! blocking each other. This bench exercises the underlying `AdaptiveRouter`
//! directly under 1 / 4 / 8 concurrent readers to measure real throughput.
//!
//! Run (parking_lot version — current code):
//! ```bash
//! cargo bench -p synapse-core --features turbo \
//!   --bench adaptive_router_concurrent 2>&1 | tee /tmp/bench_parking_lot.txt
//! ```
//!
//! Then (to simulate std::Mutex behaviour) re-run with env var set:
//! ```bash
//! ROUTER_USE_STD_MUTEX=1 cargo bench -p synapse-core --features turbo \
//!   --bench adaptive_router_concurrent 2>&1 | tee /tmp/bench_std_mutex.txt
//! ```
//!
//! The bench achieves the before/after comparison without patching lib.rs by
//! implementing both lock variants here and selecting via env var.

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use std::hint::black_box;
use std::sync::Arc;
use std::thread;
use synapse_core::turbo::adaptive_router::{AdaptiveRouter, QueryHints};

// ── Lock abstractions ──────────────────────────────────────────────────────

/// Thin trait so we can swap std::Mutex vs parking_lot::RwLock at runtime.
trait RouterLock: Send + Sync {
    fn choose(&self, hints: &QueryHints) -> u8;
}

struct StdMutexRouter(std::sync::Mutex<AdaptiveRouter>);
impl RouterLock for StdMutexRouter {
    fn choose(&self, hints: &QueryHints) -> u8 {
        let g = self.0.lock().unwrap();
        g.choose(hints) as u8
    }
}

struct ParkingRwLockRouter(parking_lot::RwLock<AdaptiveRouter>);
impl RouterLock for ParkingRwLockRouter {
    fn choose(&self, hints: &QueryHints) -> u8 {
        let g = self.0.read();
        g.choose(hints) as u8
    }
}

// ── Helpers ────────────────────────────────────────────────────────────────

fn make_router(use_std: bool) -> Arc<dyn RouterLock> {
    if use_std {
        Arc::new(StdMutexRouter(std::sync::Mutex::new(AdaptiveRouter::new())))
    } else {
        Arc::new(ParkingRwLockRouter(parking_lot::RwLock::new(
            AdaptiveRouter::new(),
        )))
    }
}

const HINTS: QueryHints = QueryHints {
    corpus_size: 250_000,
    latency_budget_us: 500,
    min_recall: 0.95,
};

/// Run `iters` total `choose()` calls spread across `threads` threads and
/// return the total number of calls completed (for throughput reporting).
fn run_concurrent(router: &Arc<dyn RouterLock>, threads: usize, iters: u64) -> u64 {
    let per_thread = iters / threads as u64;
    thread::scope(|s| {
        let handles: Vec<_> = (0..threads)
            .map(|_| {
                let r = Arc::clone(router);
                s.spawn(move || {
                    let mut acc: u8 = 0;
                    for _ in 0..per_thread {
                        acc = acc.wrapping_add(r.choose(black_box(&HINTS)));
                    }
                    // prevent dead-code elim
                    black_box(acc);
                    per_thread
                })
            })
            .collect();
        handles.into_iter().map(|h| h.join().unwrap()).sum()
    })
}

// ── Benchmarks ────────────────────────────────────────────────────────────

fn bench_router_concurrent(c: &mut Criterion) {
    let use_std = std::env::var("ROUTER_USE_STD_MUTEX")
        .map(|v| v == "1")
        .unwrap_or(false);
    let label = if use_std {
        "std_mutex"
    } else {
        "parking_lot_rwlock"
    };

    let mut group = c.benchmark_group(format!("adaptive_router/{label}"));

    for threads in [1usize, 4, 8] {
        group.throughput(Throughput::Elements(threads as u64));
        group.bench_with_input(BenchmarkId::new("threads", threads), &threads, |b, &t| {
            let router = make_router(use_std);
            // Warm up the bandit with a few observations so it's non-trivially warm
            {
                let r = &*router;
                for _ in 0..20 {
                    r.choose(&HINTS);
                }
            }
            b.iter_custom(|iters| {
                let start = std::time::Instant::now();
                run_concurrent(&router, t, iters);
                start.elapsed()
            });
        });
    }

    group.finish();
}

criterion_group!(benches, bench_router_concurrent);
criterion_main!(benches);
