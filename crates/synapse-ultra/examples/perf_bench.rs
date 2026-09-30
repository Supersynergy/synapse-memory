use synapse_ultra::events::{Event, EventKind, ingest_events};
use synapse_ultra::graph::{EdgeInput, upsert_edge, upsert_node, why};
use synapse_ultra::{Ultra, UltraError};

fn bench_ingest_batch(n: usize) -> Result<(), UltraError> {
    let u = Ultra::open_memory()?;
    u.migrate()?;
    let batch: Vec<Event> = (0..n)
        .map(|i| Event {
            ts: 1000 + i as i64,
            session_id: Some("s1".into()),
            agent: "claude".into(),
            kind: EventKind::Message.as_str().to_string(),
            uri: Some(format!("file:{i}")),
            content: Some(format!("content-{i}")),
            meta: None,
        })
        .collect();
    let start = std::time::Instant::now();
    let inserted = u.with_conn(|c| ingest_events(c, &batch))?;
    let elapsed = start.elapsed();
    let per = elapsed.as_micros() as f64 / inserted as f64;
    println!(
        "ingest_events(n={n}): {inserted} rows in {:?} ({per:.2}µs/row)",
        elapsed
    );
    Ok(())
}

fn bench_chain(n: i64) -> Result<(), UltraError> {
    let u = Ultra::open_memory()?;
    u.migrate()?;
    let ts = 0;
    u.with_conn(|c| {
        for i in 0..n {
            upsert_node(c, &format!("n{i}"), "decision", None, ts + i)?;
            if i > 0 {
                upsert_edge(
                    c,
                    &EdgeInput::new(&format!("n{}", i - 1), &format!("n{i}"), "caused", ts + i),
                )?;
            }
        }
        Ok::<(), UltraError>(())
    })?;
    let start = std::time::Instant::now();
    let steps = u.with_conn(|c| why(c, &format!("n{}", n - 1), 20))?;
    let elapsed = start.elapsed();
    println!(
        "why(chain n={n}, depth=20): {} steps in {:?}",
        steps.len(),
        elapsed
    );
    Ok(())
}

fn bench_cycle(n: i64) -> Result<(), UltraError> {
    let u = Ultra::open_memory()?;
    u.migrate()?;
    let ts = 0;
    u.with_conn(|c| {
        for i in 0..n {
            upsert_node(c, &format!("n{i}"), "d", None, ts + i)?;
        }
        for i in 0..n {
            upsert_edge(
                c,
                &EdgeInput::new(
                    &format!("n{i}"),
                    &format!("n{}", (i + 1) % n),
                    "caused",
                    ts + i,
                ),
            )?;
        }
        Ok::<(), UltraError>(())
    })?;
    let start = std::time::Instant::now();
    let steps = u.with_conn(|c| why(c, "n0", 100))?;
    let elapsed = start.elapsed();
    println!(
        "why(cycle n={n}, depth=100): {} steps in {:?}",
        steps.len(),
        elapsed
    );
    Ok(())
}

fn main() -> Result<(), UltraError> {
    bench_ingest_batch(1_000)?;
    bench_ingest_batch(10_000)?;
    bench_chain(10_000)?;
    bench_chain(50_000)?;
    bench_cycle(1_000)?;
    bench_cycle(10_000)?;
    Ok(())
}
