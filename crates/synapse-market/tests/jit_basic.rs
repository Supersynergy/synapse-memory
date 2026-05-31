use synapse_market::jit::{Col, FilterCache, Op, Predicate};

fn run(p: &Predicate, closes: &[f32], volumes: &[f32]) -> (Vec<u8>, usize) {
    let n = closes.len();
    let ts: Vec<i64> = vec![0i64; n];
    let opens: Vec<f32> = vec![0f32; n];
    let highs: Vec<f32> = vec![0f32; n];
    let lows: Vec<f32> = vec![0f32; n];

    let mut cache = FilterCache::new();
    let compiled = cache.get_or_compile(p).unwrap();
    let mut mask = vec![0u8; n];
    let count = unsafe {
        (compiled.func_ptr)(
            ts.as_ptr(),
            opens.as_ptr(),
            highs.as_ptr(),
            lows.as_ptr(),
            closes.as_ptr(),
            volumes.as_ptr(),
            n,
            mask.as_mut_ptr(),
        )
    };
    (mask, count)
}

#[test]
fn cmp_close_gt_100() {
    let closes = vec![50f32, 150.0, 75.0, 200.0];
    let vols = vec![0f32; 4];
    let p = Predicate::Cmp(Col::Close, Op::Gt, 100.0);
    let (mask, count) = run(&p, &closes, &vols);
    assert_eq!(mask, vec![0, 1, 0, 1]);
    assert_eq!(count, 2);
}

#[test]
fn and_close_gt_100_volume_gt_500() {
    let closes = vec![150f32, 150.0, 50.0, 200.0];
    let vols = vec![600f32, 400.0, 600.0, 600.0];
    let p = Predicate::And(
        Box::new(Predicate::Cmp(Col::Close, Op::Gt, 100.0)),
        Box::new(Predicate::Cmp(Col::Volume, Op::Gt, 500.0)),
    );
    let (mask, count) = run(&p, &closes, &vols);
    // close>100 AND vol>500: [1,0,0,1]
    assert_eq!(mask, vec![1, 0, 0, 1]);
    assert_eq!(count, 2);
}

#[test]
fn or_predicate() {
    let closes = vec![50f32, 150.0, 75.0, 200.0];
    let vols = vec![600f32, 400.0, 400.0, 400.0];
    let p = Predicate::Or(
        Box::new(Predicate::Cmp(Col::Close, Op::Gt, 100.0)),
        Box::new(Predicate::Cmp(Col::Volume, Op::Gt, 500.0)),
    );
    let (mask, count) = run(&p, &closes, &vols);
    // close>100 OR vol>500: [1,1,0,1]
    assert_eq!(mask, vec![1, 1, 0, 1]);
    assert_eq!(count, 3);
}

#[test]
fn not_predicate() {
    let closes = vec![50f32, 150.0, 75.0, 200.0];
    let vols = vec![0f32; 4];
    let p = Predicate::Not(Box::new(Predicate::Cmp(Col::Close, Op::Gt, 100.0)));
    let (mask, count) = run(&p, &closes, &vols);
    assert_eq!(mask, vec![1, 0, 1, 0]);
    assert_eq!(count, 2);
}

#[test]
fn compile_overhead_p50_under_5ms() {
    use std::time::Instant;
    let p = Predicate::Cmp(Col::Close, Op::Gt, 100.0);
    let mut times = Vec::with_capacity(10);
    for _ in 0..10 {
        let t = Instant::now();
        synapse_market::jit::compile::compile(&p).unwrap();
        times.push(t.elapsed().as_millis());
    }
    times.sort_unstable();
    let p50 = times[4];
    assert!(p50 < 50, "compile p50={p50}ms, expected <50ms (doc limit)");
}
