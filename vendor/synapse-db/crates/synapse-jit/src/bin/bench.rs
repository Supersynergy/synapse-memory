use std::hint::black_box;
use std::time::Instant;

use synapse_jit::ir::{interpret, interpret_group_by_sum, interpret_hash_join};
use synapse_jit::{QueryPlan, Row};

#[cfg(feature = "jit")]
use synapse_jit::{GroupByJitEngine, HashJoinJitEngine, JitEngine, Schema};

fn main() {
    // ── 1. FILTER bench (1M rows) ────────────────────────────────────────────
    println!("=== FILTER: 1M rows ===");
    const N: usize = 1_000_000;
    let rows: Vec<Row> = (0..N as i64).map(|i| Row(vec![i])).collect();
    let plan = QueryPlan::filter_gt(0, (N / 2) as i64);

    let t0 = Instant::now();
    let interp_result = black_box(interpret(&plan, &rows));
    let interp_us = t0.elapsed().as_micros();
    println!(
        "interp:  {}ms  ({} rows out)",
        interp_us / 1000,
        interp_result.len()
    );

    #[cfg(feature = "jit")]
    {
        let schema = Schema {
            columns: vec!["val".into()],
        };
        let mut engine = JitEngine::new().expect("JitEngine::new");
        let func = engine.compile(&plan, &schema).expect("compile");
        let _ = engine.execute(func, &plan, &rows).expect("warmup");
        let _ = engine.execute(func, &plan, &rows).expect("warmup2");

        let t1 = Instant::now();
        let jit_result = black_box(engine.execute(func, &plan, &rows).expect("jit execute"));
        let jit_us = t1.elapsed().as_micros();
        println!(
            "jit:     {}ms  ({} rows out)",
            jit_us / 1000,
            jit_result.len()
        );
        println!(
            "filter speedup: {:.1}×",
            interp_us as f64 / jit_us.max(1) as f64
        );
    }

    // ── 2. GROUP BY bench (10M rows, 5 distinct keys) ────────────────────────
    println!("\n=== GROUP BY SUM: 10M rows, 5 keys ===");
    const GB_N: usize = 10_000_000;
    const N_KEYS: i64 = 5;
    let gb_rows: Vec<Row> = (0..GB_N as i64)
        .map(|i| Row(vec![i % N_KEYS, i])) // col0=key(0-4), col1=val
        .collect();
    let gb_plan = QueryPlan::group_by_sum(0, 1);

    // interpreter baseline
    let t0 = Instant::now();
    let interp_gb = black_box(interpret_group_by_sum(&gb_plan, &gb_rows));
    let interp_gb_us = t0.elapsed().as_micros();
    println!(
        "interp:  {}ms  ({} groups)",
        interp_gb_us / 1000,
        interp_gb.len()
    );

    #[cfg(feature = "jit")]
    {
        let mut gb_engine = GroupByJitEngine::new().expect("GroupByJitEngine::new");
        let gb_func = gb_engine.compile(&gb_plan).expect("compile gb");
        // warmup
        let _ = gb_engine
            .execute(gb_func, &gb_plan, &gb_rows)
            .expect("warmup");

        let t1 = Instant::now();
        let jit_gb = black_box(
            gb_engine
                .execute(gb_func, &gb_plan, &gb_rows)
                .expect("jit gb"),
        );
        let jit_gb_us = t1.elapsed().as_micros();
        println!("jit:     {}ms  ({} groups)", jit_gb_us / 1000, jit_gb.len());
        println!(
            "group_by speedup: {:.1}×",
            interp_gb_us as f64 / jit_gb_us.max(1) as f64
        );

        // second run for variance
        let t2 = Instant::now();
        let jit_gb2 = black_box(
            gb_engine
                .execute(gb_func, &gb_plan, &gb_rows)
                .expect("jit gb2"),
        );
        let jit_gb2_us = t2.elapsed().as_micros();
        println!(
            "jit run2: {}ms  ({} groups)",
            jit_gb2_us / 1000,
            jit_gb2.len()
        );
        println!(
            "group_by speedup run2: {:.1}×",
            interp_gb_us as f64 / jit_gb2_us.max(1) as f64
        );

        // correctness check
        assert_eq!(jit_gb.len(), interp_gb.len(), "group count mismatch");
        for (jk, ik) in jit_gb.iter().zip(interp_gb.iter()) {
            assert_eq!(jk, ik, "group result mismatch");
        }
        println!("correctness: OK");
    }

    // ── 3. HASH JOIN bench (1M left, 100K right) ────────────────────────────
    println!("\n=== HASH JOIN: 1M left, 100K right ===");
    const HJ_LEFT: usize = 1_000_000;
    const HJ_RIGHT: usize = 100_000;
    let hj_left: Vec<Row> = (0..HJ_LEFT as i64).map(|i| Row(vec![i, i * 2])).collect();
    let hj_right: Vec<Row> = (0..HJ_RIGHT as i64).map(|i| Row(vec![i * 10])).collect();
    let hj_plan = QueryPlan::hash_join(0, 0);

    let t0 = Instant::now();
    let interp_hj = black_box(interpret_hash_join(&hj_plan, &hj_left, &hj_right));
    let interp_hj_us = t0.elapsed().as_micros();
    println!(
        "interp:  {}ms  ({} rows out)",
        interp_hj_us / 1000,
        interp_hj.len()
    );

    #[cfg(feature = "jit")]
    {
        let mut hj_engine = HashJoinJitEngine::new().expect("HashJoinJitEngine::new");
        let hj_func = hj_engine.compile(&hj_plan).expect("compile hj");
        let _ = hj_engine
            .execute(hj_func, &hj_plan, &hj_left, &hj_right)
            .expect("warmup");

        let t1 = Instant::now();
        let jit_hj = black_box(
            hj_engine
                .execute(hj_func, &hj_plan, &hj_left, &hj_right)
                .expect("jit hj"),
        );
        let jit_hj_us = t1.elapsed().as_micros();
        println!(
            "jit:     {}ms  ({} rows out)",
            jit_hj_us / 1000,
            jit_hj.len()
        );
        println!(
            "hash_join speedup: {:.1}×",
            interp_hj_us as f64 / jit_hj_us.max(1) as f64
        );
        println!("correctness: {} == {}", jit_hj.len(), interp_hj.len());
    }

    // ── 4. SQLite GROUP BY reference ─────────────────────────────────────────
    println!("\n=== SQLite GROUP BY reference ===");
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    conn.execute_batch("CREATE TABLE t (k INTEGER, v INTEGER); BEGIN;")
        .unwrap();
    {
        let mut stmt = conn.prepare("INSERT INTO t VALUES (?, ?)").unwrap();
        for i in 0..GB_N as i64 {
            stmt.execute(rusqlite::params![i % N_KEYS, i]).unwrap();
        }
    }
    conn.execute_batch("COMMIT;").unwrap();

    // warmup
    let _: Vec<(i64, i64)> = conn
        .prepare("SELECT k, SUM(v) FROM t GROUP BY k")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .map(|v| v.unwrap())
        .collect();

    let t3 = Instant::now();
    let sqlite_gb: Vec<(i64, i64)> = conn
        .prepare("SELECT k, SUM(v) FROM t GROUP BY k")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .map(|v| v.unwrap())
        .collect();
    let sqlite_us = t3.elapsed().as_micros();
    println!(
        "sqlite:  {}ms  ({} groups)",
        sqlite_us / 1000,
        sqlite_gb.len()
    );

    #[cfg(feature = "jit")]
    println!(
        "jit vs sqlite: {:.1}×",
        sqlite_us as f64 / {
            let mut gb_e = GroupByJitEngine::new().unwrap();
            let gf = gb_e.compile(&gb_plan).unwrap();
            let _ = gb_e.execute(gf, &gb_plan, &gb_rows).unwrap(); // warmup
            let t = Instant::now();
            let _ = black_box(gb_e.execute(gf, &gb_plan, &gb_rows).unwrap());
            t.elapsed().as_micros().max(1)
        } as f64
    );
}
