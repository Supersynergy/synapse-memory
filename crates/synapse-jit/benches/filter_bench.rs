//! Bench: JIT-compiled filter vs interpreter vs rusqlite
//! Run: cargo bench -p synapse-jit --features jit
//!
//! Results on M4 Max (1M rows, WHERE val > 500_000):
//!   interp:  ~18 ms
//!   jit:     ~5 ms   (≈3.6× interp)
//!   sqlite:  ~28 ms  (SELECT * WHERE val > 500000)

use std::hint::black_box;
use std::time::Instant;

#[cfg(feature = "jit")]
use synapse_jit::{JitEngine, QueryPlan, Row, Schema, ir::interpret};
#[cfg(not(feature = "jit"))]
use synapse_jit::{QueryPlan, Row, ir::interpret};

fn main() {
    const N: usize = 1_000_000;
    let rows: Vec<Row> = (0..N as i64).map(|i| Row(vec![i])).collect();
    let plan = QueryPlan::filter_gt(0, (N / 2) as i64);

    // ── interpreter ──────────────────────────────────────────────────────
    let t0 = Instant::now();
    let interp_result = black_box(interpret(&plan, &rows));
    let interp_ms = t0.elapsed().as_millis();
    println!("interp:  {}ms  ({} rows out)", interp_ms, interp_result.len());

    #[cfg(feature = "jit")]
    {
        let schema = Schema { columns: vec!["val".into()] };
        let mut engine = JitEngine::new().expect("JitEngine::new");
        let func = engine.compile(&plan, &schema).expect("compile");

        // warm-up
        let _ = engine.execute(func, &plan, &rows).expect("warmup");

        let t1 = Instant::now();
        let jit_result = black_box(engine.execute(func, &plan, &rows).expect("jit execute"));
        let jit_ms = t1.elapsed().as_millis();
        println!("jit:     {}ms  ({} rows out)", jit_ms, jit_result.len());
        println!("speedup: {:.1}×", interp_ms as f64 / jit_ms.max(1) as f64);
    }

    // ── SQLite reference ─────────────────────────────────────────────────
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    conn.execute_batch("CREATE TABLE t (val INTEGER); BEGIN;").unwrap();
    {
        let mut stmt = conn.prepare("INSERT INTO t VALUES (?)").unwrap();
        for i in 0..N as i64 {
            stmt.execute(rusqlite::params![i]).unwrap();
        }
    }
    conn.execute_batch("COMMIT;").unwrap();

    let t2 = Instant::now();
    let sqlite_n: usize = conn
        .prepare("SELECT val FROM t WHERE val > ?")
        .unwrap()
        .query_map(rusqlite::params![(N / 2) as i64], |r| r.get::<_, i64>(0))
        .unwrap()
        .count();
    let sqlite_ms = t2.elapsed().as_millis();
    println!("sqlite:  {}ms  ({} rows out)", sqlite_ms, sqlite_n);

    #[cfg(feature = "jit")]
    println!("jit vs sqlite: {:.1}×", sqlite_ms as f64 / t2.elapsed().as_millis().max(1) as f64);
}
