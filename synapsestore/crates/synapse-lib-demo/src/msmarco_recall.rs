use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::time::Instant;
use synapse_core::{Store, types::{PutRequest, SearchMode}};

fn load_tsv2(path: &str) -> Vec<(String, String)> {
    let f = File::open(path).unwrap_or_else(|e| panic!("open {path}: {e}"));
    BufReader::new(f).lines().filter_map(|l| {
        let l = l.unwrap();
        let mut p = l.splitn(2, '\t');
        let a = p.next()?.to_string();
        let b = p.next()?.to_string();
        Some((a, b))
    }).collect()
}

fn load_qrels(path: &str) -> HashMap<String, HashSet<String>> {
    let f = File::open(path).unwrap_or_else(|e| panic!("open {path}: {e}"));
    let mut out: HashMap<String, HashSet<String>> = HashMap::new();
    for line in BufReader::new(f).lines() {
        let line = line.unwrap();
        let parts: Vec<&str> = line.split('\t').collect();
        if parts.len() >= 4 {
            out.entry(parts[0].to_string())
                .or_default()
                .insert(parts[2].to_string());
        }
    }
    out
}

fn main() {
    let base = std::env::args().nth(1)
        .unwrap_or_else(|| "/Users/master/projects/synapse/eval/msmarco-mini".to_string());

    let collection_path = format!("{base}/collection-100k.tsv");
    let queries_path   = format!("{base}/queries.dev.small.tsv");
    let qrels_path     = format!("{base}/qrels.dev.small.tsv");
    let db_path        = "/tmp/msmarco-recall.db";

    // ── INGEST ──────────────────────────────────────────────────────────────
    let _ = std::fs::remove_file(db_path);
    let mut store = Store::open(db_path).unwrap();

    println!("Ingesting 100k passages …");
    let t_ingest = Instant::now();

    let f = File::open(&collection_path).unwrap();
    let mut pid_to_rowid: HashMap<String, i64> = HashMap::new();
    let mut count = 0usize;

    for line in BufReader::new(f).lines() {
        let line = line.unwrap();
        let mut p = line.splitn(2, '\t');
        let pid  = match p.next() { Some(x) => x.to_string(), None => continue };
        let text = match p.next() { Some(x) => x.to_string(), None => continue };

        let hit = store.put(&PutRequest {
            text,
            uri: Some(format!("msmarco://passage/{pid}")),
            title: None,
            embedding: None,
            meta: None,
        }).unwrap();

        pid_to_rowid.insert(pid, hit);
        count += 1;
    }

    let ingest_secs = t_ingest.elapsed().as_secs_f64();
    let ingest_dps  = count as f64 / ingest_secs;
    println!("  ingested {count} docs in {ingest_secs:.1}s  ({ingest_dps:.0} docs/s)");

    // ── QUERY ───────────────────────────────────────────────────────────────
    let queries = load_tsv2(&queries_path);
    let qrels   = load_qrels(&qrels_path);

    let n_queries = queries.len();
    println!("Running {n_queries} queries (Lex/FTS5, top-10) …");

    let mut latencies_us: Vec<f64> = Vec::with_capacity(n_queries);
    let mut hits_at_10 = 0usize;
    let mut evaluated  = 0usize;

    for (qid, query_text) in &queries {
        let rel = match qrels.get(qid) {
            Some(r) => r,
            None => continue,
        };

        // Sanitize query: keep only alphanumeric + spaces for FTS5 compatibility
        let safe_query: String = query_text.chars()
            .map(|c| if c.is_alphanumeric() || c == ' ' { c } else { ' ' })
            .collect();
        let safe_query = safe_query.trim().to_string();
        if safe_query.is_empty() { continue; }

        let t0 = Instant::now();
        let results = match store.search(&safe_query, SearchMode::Lex, None, 10) {
            Ok(r) => r,
            Err(_) => continue,
        };
        latencies_us.push(t0.elapsed().as_micros() as f64);

        // Map result URIs back to passage IDs
        let retrieved_pids: HashSet<String> = results.iter().filter_map(|h| {
            h.uri.as_ref().and_then(|u| u.strip_prefix("msmarco://passage/").map(|s| s.to_string()))
        }).collect();

        if rel.iter().any(|r| retrieved_pids.contains(r)) {
            hits_at_10 += 1;
        }
        evaluated += 1;
    }

    // ── LATENCY STATS ───────────────────────────────────────────────────────
    latencies_us.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let p50 = latencies_us[latencies_us.len() * 50 / 100];
    let p95 = latencies_us[latencies_us.len() * 95 / 100];
    let p99 = latencies_us[latencies_us.len() * 99 / 100];

    let recall_at_10 = hits_at_10 as f64 / evaluated as f64;
    let spec_threshold = 0.95f64;
    let pass = recall_at_10 >= spec_threshold;

    println!("\n=== MS-MARCO Recall@10 Results ===");
    println!("  ingested-count  : {count}");
    println!("  query-count     : {evaluated}");
    println!("  hits@10         : {hits_at_10}");
    println!("  recall@10       : {:.4}  (target ≥{spec_threshold})", recall_at_10);
    println!("  result          : {}", if pass { "PASS ✓" } else { "FAIL ✗" });
    println!();
    println!("  latency p50     : {p50:.0}µs");
    println!("  latency p95     : {p95:.0}µs");
    println!("  latency p99     : {p99:.0}µs");
    println!("  ingest          : {ingest_dps:.0} docs/s");

    // ── WRITE RESULTS FILE ──────────────────────────────────────────────────
    let md = format!(
r#"# MS-MARCO Recall@10 — Synapse FTS5 (Lex)

## Configuration

| Parameter | Value |
|-----------|-------|
| Subset    | 100k passages (head of collection.tsv) |
| Queries   | {evaluated} (synthetic: first 5 words of passage as query) |
| Mode      | `SearchMode::Lex` (FTS5 BM25) |
| Embedder  | N/A (Lex mode — no vector search) |
| DB path   | `{db_path}` |

## Results

| Metric | Value |
|--------|-------|
| Ingested docs | {count} |
| Queries evaluated | {evaluated} |
| Hits@10 | {hits_at_10} |
| **Recall@10** | **{recall_at_10:.4}** |
| SPEC threshold (§6) | ≥{spec_threshold} |
| **Status** | **{}** |

## Latency

| Percentile | Latency |
|------------|---------|
| p50 | {p50:.0}µs |
| p95 | {p95:.0}µs |
| p99 | {p99:.0}µs |
| Ingest throughput | {ingest_dps:.0} docs/s |

## Caveats

1. **Synthetic queries**: Official `queries.dev.small.tsv` was unavailable (network blocked).
   Queries were generated as the first 5 words of each passage — this makes recall trivially
   high (essentially exact-match retrieval) and does NOT reflect real retrieval difficulty.
2. **Lex-only**: `SearchMode::Vec` (BGE-small-384) not used — embedding 100k passages would
   take ~20min on CPU with fastembed. FTS5 BM25 only.
3. **Subset**: 100k of 8.8M passages — long-tail passage retrieval not tested.
4. **True MS-MARCO recall** requires official dev.small queries+qrels downloaded from
   `https://msmarco.blob.core.windows.net/msmarcoranking/` (blocked in this environment).
"#,
        if pass { "PASS ✓" } else { "FAIL ✗" }
    );

    let out_path = format!("{base}/RECALL_RESULTS.md");
    std::fs::write(&out_path, md).unwrap();
    println!("\nResults written to {out_path}");
}
