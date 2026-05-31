use criterion::{Criterion, Throughput, criterion_group, criterion_main};
use synapse_market::pattern::fsm::FsmEngine;
use synapse_market::pattern::{Event, Pattern};
use synapse_market::store::page::Bar;

fn bench_pattern_throughput(c: &mut Criterion) {
    const N: usize = 1_000_000;

    let bars: Vec<Bar> = (0..N as i64)
        .map(|i| Bar {
            ts: i * 60,
            open: 100.0,
            high: 101.0,
            low: 99.0,
            close: 100.5,
            volume: if i % 100 == 0 { 500.0 } else { 100.0 },
        })
        .collect();

    let events: Vec<Event> = bars.iter().map(|b| Event::Candle(*b)).collect();

    let mut group = c.benchmark_group("pattern_throughput");
    group.throughput(Throughput::Elements(N as u64));

    group.bench_function("volume_spike_1M", |b| {
        b.iter(|| {
            let mut engine = FsmEngine::new();
            engine.register(Pattern::VolumeSpike {
                multiplier: 2.0,
                window_bars: 20,
            });
            let mut total = 0usize;
            for ev in &events {
                total += engine.on_event("SPY", ev).len();
            }
            total
        });
    });

    group.bench_function("cluster_1M", |b| {
        b.iter(|| {
            let mut engine = FsmEngine::new();
            engine.register(Pattern::InsiderCluster {
                k: 3,
                window_days: 7,
            });
            let buys: Vec<Event> = (0..N as i64)
                .filter(|i| i % 50 == 0)
                .map(|i| Event::InsiderBuy {
                    value_usd: 100_000.0,
                    ts: i * 60,
                    ticker: "SPY".into(),
                })
                .collect();
            let mut total = 0usize;
            for ev in &buys {
                total += engine.on_event("SPY", ev).len();
            }
            total
        });
    });

    group.finish();
}

criterion_group!(benches, bench_pattern_throughput);
criterion_main!(benches);
