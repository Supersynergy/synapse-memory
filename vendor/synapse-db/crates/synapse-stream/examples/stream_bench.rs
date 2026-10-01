use serde_json::json;
use std::time::Instant;
use synapse_stream::pubsub::Hub;
use synapse_stream::{CdcReader, Op};
use tempfile::TempDir;

fn main() {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();

    rt.block_on(async {
        // ── pub/sub broadcast latency ─────────────────────────────────────
        let hub = Hub::new(100_000);
        let mut rx = hub.subscribe();

        let dummy_ev = synapse_stream::ChangeEvent {
            table: "t".into(),
            op: Op::Insert,
            ts: 0,
            row: json!({"v": 1}),
        };

        const FANOUT: usize = 100_000;
        let t0 = Instant::now();
        for i in 0..FANOUT {
            let mut ev = dummy_ev.clone();
            ev.ts = i as i64;
            hub.publish(ev);
        }
        let publish_us = t0.elapsed().as_micros();

        let mut received = 0usize;
        while received < FANOUT {
            let _ = rx.recv().await.unwrap();
            received += 1;
        }
        let total_us = t0.elapsed().as_micros();

        println!("pubsub_publish_100k_us={}", publish_us);
        println!("pubsub_recv_100k_us={}", total_us);
        println!("pubsub_per_msg_ns={}", (publish_us * 1000) / FANOUT as u128);

        // ── CDC emit throughput (trigger-based on-disk) ───────────────────
        let dir = TempDir::new().unwrap();
        let db = dir.path().join("bench.db");
        // init table
        CdcReader::new(&db).unwrap();

        let t1 = Instant::now();
        for i in 0u64..100_000 {
            CdcReader::emit_direct(&db, Op::Insert, "bench", json!({"id": i})).unwrap();
        }
        let cdc_ms = t1.elapsed().as_millis();
        println!("cdc_emit_100k_ms={}", cdc_ms);
        println!(
            "cdc_emit_per_s={}",
            (100_000u64 * 1000) / cdc_ms.max(1) as u64
        );
    });
}
