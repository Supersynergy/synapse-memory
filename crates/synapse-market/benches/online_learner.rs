use criterion::{black_box, criterion_group, criterion_main, Criterion};
use synapse_market::learn::{OnlineLearner, FtrlLearner};

fn rand_f32(seed: &mut u64) -> f32 {
    *seed ^= *seed << 13;
    *seed ^= *seed >> 7;
    *seed ^= *seed << 17;
    ((*seed as i32) as f32) / i32::MAX as f32
}

fn bench_update(c: &mut Criterion) {
    let mut learner = FtrlLearner::new(16);
    let mut seed = 999u64;
    let features: Vec<f32> = (0..16).map(|_| rand_f32(&mut seed)).collect();
    c.bench_function("ftrl_update_d16", |b| {
        b.iter(|| {
            learner.update(black_box(&features), black_box(1.0))
        })
    });
}

fn bench_predict(c: &mut Criterion) {
    let mut learner = FtrlLearner::new(16);
    let mut seed = 42u64;
    // Pre-train
    for _ in 0..1000 {
        let x: Vec<f32> = (0..16).map(|_| rand_f32(&mut seed)).collect();
        learner.update(&x, if x[0] > 0.0 { 1.0 } else { 0.0 });
    }
    let features: Vec<f32> = (0..16).map(|_| rand_f32(&mut seed)).collect();
    c.bench_function("ftrl_predict_d16", |b| {
        b.iter(|| {
            learner.predict(black_box(&features))
        })
    });
}

fn bench_100k_updates(c: &mut Criterion) {
    c.bench_function("ftrl_100k_updates_wall", |b| {
        b.iter(|| {
            let mut learner = FtrlLearner::new(16);
            let mut seed = 12345u64;
            for _ in 0..100_000 {
                let x: Vec<f32> = (0..16).map(|_| rand_f32(&mut seed)).collect();
                let y = if x[0] + x[1] > 0.0 { 1.0 } else { 0.0 };
                black_box(learner.update(&x, y));
            }
        })
    });
}

criterion_group!(benches, bench_update, bench_predict, bench_100k_updates);
criterion_main!(benches);
