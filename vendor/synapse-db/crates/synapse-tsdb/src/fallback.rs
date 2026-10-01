//! Minimal columnar TSDB — no Arrow dep.
//! Layout: in-memory columns + optional zstd flush to `<dir>/<YYYY-MM-DD>/<HH>.bin`.
//!
//! Format per shard file (little-endian):
//!   [u64 rows][ts: i64×N][value: f64×N][metric_len: u32×N][metric_bytes: concat][labels_json_len: u32×N][labels_json: concat]

use std::{
    collections::HashMap,
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::Result;
use chrono::{TimeZone, Utc};
use serde::{Deserialize, Serialize};

type ShardGroups = HashMap<(String, u32), Vec<usize>>;

/// One time-series record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Row {
    /// Unix timestamp in milliseconds.
    pub ts: i64,
    pub metric: String,
    pub labels: HashMap<String, String>,
    pub value: f64,
}

/// Aggregation operation for [`TsdbStore::aggregate`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AggOp {
    Avg,
    Sum,
    Max,
    Min,
    Count,
}

/// Columnar in-process time-series store.
///
/// Internally keeps an unsorted write-buffer. `flush()` (or drop) writes
/// zstd-compressed shard files under `<dir>/<YYYY-MM-DD>/<HH>.bin`.
pub struct TsdbStore {
    dir: PathBuf,
    buf_ts: Vec<i64>,
    buf_metric: Vec<String>,
    buf_labels: Vec<HashMap<String, String>>,
    buf_value: Vec<f64>,
    /// Auto-flush threshold (rows in write-buffer).
    flush_threshold: usize,
}

impl TsdbStore {
    /// Open (or create) a store rooted at `dir`.
    pub fn open(dir: impl AsRef<Path>) -> Result<Self> {
        let dir = dir.as_ref().to_path_buf();
        fs::create_dir_all(&dir)?;
        Ok(Self {
            dir,
            buf_ts: Vec::with_capacity(65_536),
            buf_metric: Vec::with_capacity(65_536),
            buf_labels: Vec::with_capacity(65_536),
            buf_value: Vec::with_capacity(65_536),
            flush_threshold: 1_000_000,
        })
    }

    /// Set auto-flush threshold (default: 1_000_000 rows).
    pub fn with_flush_threshold(mut self, n: usize) -> Self {
        self.flush_threshold = n;
        self
    }

    /// Append a single row.
    #[inline]
    pub fn append_row(&mut self, row: Row) -> Result<()> {
        self.buf_ts.push(row.ts);
        self.buf_metric.push(row.metric);
        self.buf_labels.push(row.labels);
        self.buf_value.push(row.value);
        if self.buf_ts.len() >= self.flush_threshold {
            self.flush()?;
        }
        Ok(())
    }

    /// Append a batch of rows (columnar slices).
    pub fn append(
        &mut self,
        ts: &[i64],
        metrics: &[&str],
        labels: &[HashMap<String, String>],
        values: &[f64],
    ) -> Result<()> {
        assert_eq!(ts.len(), metrics.len());
        assert_eq!(ts.len(), values.len());
        assert_eq!(ts.len(), labels.len());
        self.buf_ts.extend_from_slice(ts);
        self.buf_metric
            .extend(metrics.iter().map(|s| s.to_string()));
        self.buf_labels.extend_from_slice(labels);
        self.buf_value.extend_from_slice(values);
        if self.buf_ts.len() >= self.flush_threshold {
            self.flush()?;
        }
        Ok(())
    }

    /// Query rows where `metric == metric` and `from <= ts <= to` (ms).
    /// Returns sorted by ts.
    pub fn query_range(&self, metric: &str, from: i64, to: i64) -> Result<Vec<Row>> {
        // Collect from write-buffer.
        let mut rows: Vec<Row> = self
            .buf_ts
            .iter()
            .enumerate()
            .filter(|(i, ts)| **ts >= from && **ts <= to && self.buf_metric[*i] == metric)
            .map(|(i, &ts)| Row {
                ts,
                metric: self.buf_metric[i].clone(),
                labels: self.buf_labels[i].clone(),
                value: self.buf_value[i],
            })
            .collect();

        // Collect from disk shards in the date range.
        let shards = self.shards_in_range(from, to)?;
        for shard in shards {
            let disk = self.read_shard(&shard)?;
            for r in disk {
                if r.metric == metric && r.ts >= from && r.ts <= to {
                    rows.push(r);
                }
            }
        }

        rows.sort_unstable_by_key(|r| r.ts);
        Ok(rows)
    }

    /// Aggregate `metric` in time-windows of `window` ms.
    /// Returns rows with ts = window-start, value = agg result.
    pub fn aggregate(&self, metric: &str, agg: AggOp, window: Duration) -> Result<Vec<Row>> {
        let window_ms = window.as_millis() as i64;
        // Collect all matching rows from buffer + disk (full scan).
        let mut all: Vec<(i64, f64)> = self
            .buf_ts
            .iter()
            .enumerate()
            .filter(|(i, _)| self.buf_metric[*i] == metric)
            .map(|(i, &ts)| (ts, self.buf_value[i]))
            .collect();

        let shards = self.all_shards()?;
        for shard in shards {
            let disk = self.read_shard(&shard)?;
            for r in disk {
                if r.metric == metric {
                    all.push((r.ts, r.value));
                }
            }
        }

        if all.is_empty() {
            return Ok(vec![]);
        }

        all.sort_unstable_by_key(|&(ts, _)| ts);

        // Group into windows.
        let mut result: Vec<Row> = Vec::new();
        let mut i = 0;
        while i < all.len() {
            let win_start = (all[i].0 / window_ms) * window_ms;
            let win_end = win_start + window_ms;
            let mut vals: Vec<f64> = Vec::new();
            while i < all.len() && all[i].0 < win_end {
                vals.push(all[i].1);
                i += 1;
            }
            let v = match agg {
                AggOp::Avg => vals.iter().sum::<f64>() / vals.len() as f64,
                AggOp::Sum => vals.iter().sum(),
                AggOp::Max => vals.iter().cloned().fold(f64::NEG_INFINITY, f64::max),
                AggOp::Min => vals.iter().cloned().fold(f64::INFINITY, f64::min),
                AggOp::Count => vals.len() as f64,
            };
            result.push(Row {
                ts: win_start,
                metric: metric.to_string(),
                labels: HashMap::new(),
                value: v,
            });
        }
        Ok(result)
    }

    /// Flush write-buffer to disk shards.
    pub fn flush(&mut self) -> Result<()> {
        if self.buf_ts.is_empty() {
            return Ok(());
        }
        // Group rows by (date, hour).
        let mut groups: ShardGroups = HashMap::new();
        for (i, &ts) in self.buf_ts.iter().enumerate() {
            let dt = Utc.timestamp_millis_opt(ts).single().unwrap_or_default();
            let date = dt.format("%Y-%m-%d").to_string();
            let hour = dt.hour();
            groups.entry((date, hour)).or_default().push(i);
        }

        for ((date, hour), indices) in &groups {
            let shard_dir = self.dir.join(date);
            fs::create_dir_all(&shard_dir)?;
            let shard_path = shard_dir.join(format!("{:02}.bin", hour));

            // Read existing shard if present, merge, re-write.
            let mut existing = if shard_path.exists() {
                self.read_shard(&shard_path)?
            } else {
                vec![]
            };

            for &i in indices {
                existing.push(Row {
                    ts: self.buf_ts[i],
                    metric: self.buf_metric[i].clone(),
                    labels: self.buf_labels[i].clone(),
                    value: self.buf_value[i],
                });
            }

            write_shard(&shard_path, &existing)?;
        }

        self.buf_ts.clear();
        self.buf_metric.clear();
        self.buf_labels.clear();
        self.buf_value.clear();
        Ok(())
    }

    // ── internal helpers ──────────────────────────────────────────────────────

    fn shards_in_range(&self, from: i64, to: i64) -> Result<Vec<PathBuf>> {
        let from_dt = Utc.timestamp_millis_opt(from).single().unwrap_or_default();
        let to_dt = Utc.timestamp_millis_opt(to).single().unwrap_or_default();
        let mut paths = vec![];
        // Iterate day directories.
        if !self.dir.exists() {
            return Ok(paths);
        }
        for entry in fs::read_dir(&self.dir)?.flatten() {
            let p = entry.path();
            if !p.is_dir() {
                continue;
            }
            let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
            // Quick filter: date string comparison works lexicographically.
            let from_date = from_dt.format("%Y-%m-%d").to_string();
            let to_date = to_dt.format("%Y-%m-%d").to_string();
            if name < from_date.as_str() || name > to_date.as_str() {
                continue;
            }
            for h_entry in fs::read_dir(&p)?.flatten() {
                let hp = h_entry.path();
                if hp.extension().and_then(|e| e.to_str()) == Some("bin") {
                    paths.push(hp);
                }
            }
        }
        Ok(paths)
    }

    fn all_shards(&self) -> Result<Vec<PathBuf>> {
        let mut paths = vec![];
        if !self.dir.exists() {
            return Ok(paths);
        }
        for entry in fs::read_dir(&self.dir)?.flatten() {
            let p = entry.path();
            if p.is_dir() {
                for h in fs::read_dir(&p)?.flatten() {
                    let hp = h.path();
                    if hp.extension().and_then(|e| e.to_str()) == Some("bin") {
                        paths.push(hp);
                    }
                }
            }
        }
        Ok(paths)
    }

    fn read_shard(&self, path: &Path) -> Result<Vec<Row>> {
        let compressed = fs::read(path)?;
        let raw = zstd::decode_all(io::Cursor::new(&compressed))?;
        let rows: Vec<Row> = serde_json::from_slice(&raw)?;
        Ok(rows)
    }
}

impl Drop for TsdbStore {
    fn drop(&mut self) {
        let _ = self.flush();
    }
}

fn write_shard(path: &Path, rows: &[Row]) -> Result<()> {
    let json = serde_json::to_vec(rows)?;
    let mut enc = zstd::Encoder::new(Vec::new(), 3)?;
    enc.write_all(&json)?;
    let compressed = enc.finish()?;
    fs::write(path, &compressed)?;
    Ok(())
}

// Needed for chrono's hour() method.
use chrono::Timelike;

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn make_store() -> (TsdbStore, TempDir) {
        let dir = TempDir::new().unwrap();
        let store = TsdbStore::open(dir.path()).unwrap();
        (store, dir)
    }

    #[test]
    fn test_append_and_query() {
        let (mut store, _dir) = make_store();
        let base_ts = 1_700_000_000_000i64; // ms
        for i in 0..100 {
            store
                .append_row(Row {
                    ts: base_ts + i * 1000,
                    metric: "cpu".to_string(),
                    labels: HashMap::new(),
                    value: i as f64,
                })
                .unwrap();
        }
        let rows = store.query_range("cpu", base_ts, base_ts + 50_000).unwrap();
        assert_eq!(rows.len(), 51);
        assert!(rows.windows(2).all(|w| w[0].ts <= w[1].ts));
    }

    #[test]
    fn test_aggregate_avg() {
        let (mut store, _dir) = make_store();
        let base_ts = 1_700_000_000_000i64;
        // 4 rows in 2 windows of 5s each
        for (i, v) in [(0i64, 10f64), (1000, 20.0), (6000, 30.0), (7000, 40.0)] {
            store
                .append_row(Row {
                    ts: base_ts + i,
                    metric: "lat".to_string(),
                    labels: HashMap::new(),
                    value: v,
                })
                .unwrap();
        }
        let result = store
            .aggregate("lat", AggOp::Avg, Duration::from_secs(5))
            .unwrap();
        assert_eq!(result.len(), 2);
        assert!((result[0].value - 15.0).abs() < 1e-9);
        assert!((result[1].value - 35.0).abs() < 1e-9);
    }

    #[test]
    fn test_flush_and_reload() {
        let dir = TempDir::new().unwrap();
        let base_ts = 1_700_000_000_000i64;
        {
            let mut store = TsdbStore::open(dir.path()).unwrap();
            for i in 0..1000 {
                store
                    .append_row(Row {
                        ts: base_ts + i * 1000,
                        metric: "mem".to_string(),
                        labels: HashMap::new(),
                        value: i as f64,
                    })
                    .unwrap();
            }
            // drop → flush
        }
        let store2 = TsdbStore::open(dir.path()).unwrap();
        let rows = store2
            .query_range("mem", base_ts, base_ts + 999_000)
            .unwrap();
        assert_eq!(rows.len(), 1000);
    }

    #[test]
    fn test_10k_insert_throughput() {
        let (mut store, _dir) = make_store();
        let base_ts = 1_700_000_000_000i64;
        let n = 10_000;
        let start = std::time::Instant::now();
        for i in 0..n {
            store
                .append_row(Row {
                    ts: base_ts + i * 100,
                    metric: "tput".to_string(),
                    labels: HashMap::new(),
                    value: i as f64,
                })
                .unwrap();
        }
        let elapsed = start.elapsed();
        let per_sec = n as f64 / elapsed.as_secs_f64();
        eprintln!("10k inserts: {:.0} rows/s ({:?} total)", per_sec, elapsed);
        assert!(per_sec > 500_000.0, "expected >500K/s, got {:.0}", per_sec);

        let rows = store
            .query_range("tput", base_ts, base_ts + n * 100)
            .unwrap();
        assert_eq!(rows.len() as i64, n);
    }
}
