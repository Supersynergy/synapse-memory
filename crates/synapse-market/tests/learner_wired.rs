use synapse_market::learn::{FtrlLearner, OnlineLearner};

fn rand_f32(seed: &mut u64) -> f32 {
    *seed ^= *seed << 13;
    *seed ^= *seed >> 7;
    *seed ^= *seed << 17;
    ((*seed as i32) as f32) / i32::MAX as f32
}

fn label(x: &[f32]) -> f32 {
    if x[0] + x[1] >= 0.0 {
        1.0
    } else {
        0.0
    }
}

#[test]
fn ftrl_trains_500_samples_80pct_accuracy() {
    let mut learner = FtrlLearner::new(4);
    let mut seed = 98765u64;
    for _ in 0..500 {
        let x: Vec<f32> = (0..4).map(|_| rand_f32(&mut seed)).collect();
        learner.update(&x, label(&x));
    }
    let mut correct = 0usize;
    let mut eval_seed = 11111u64;
    for _ in 0..500 {
        let x: Vec<f32> = (0..4).map(|_| rand_f32(&mut eval_seed)).collect();
        let p = learner.predict(&x);
        if (p >= 0.5) == (label(&x) >= 0.5) {
            correct += 1;
        }
    }
    let acc = correct as f64 / 500.0;
    assert!(acc >= 0.80, "accuracy {acc:.3} < 0.80");
}
