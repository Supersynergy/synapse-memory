//! Arrow/Parquet columnar backend (feature = "tsdb").
//!
//! Schema:
//!   ts:     Timestamp(Millisecond, UTC) NOT NULL
//!   metric: Utf8 NOT NULL
//!   labels: Utf8 (JSON-encoded map)
//!   value:  Float64

use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use anyhow::{Context, Result};
use arrow::{
    array::{
        Array, Float64Array, Float64Builder, Int64Array, Int64Builder, StringArray, StringBuilder,
    },
    datatypes::{DataType, Field, Schema, SchemaRef},
    record_batch::RecordBatch,
};
use chrono::{TimeZone, Utc};
use parquet::{
    arrow::{ArrowWriter, arrow_reader::ParquetRecordBatchReaderBuilder},
    basic::Compression,
    file::properties::WriterProperties,
};

pub use crate::fallback::AggOp;

type ShardGroups = HashMap<(String, u32), Vec<usize>>;

fn tsdb_schema() -> SchemaRef {
    Arc::new(Schema::new(vec![
        Field::new("ts", DataType::Int64, false),
        Field::new("metric", DataType::Utf8, false),
        Field::new("labels", DataType::Utf8, true),
        Field::new("value", DataType::Float64, true),
    ]))
}

/// Arrow/Parquet columnar time-series store.
pub struct TsdbStore {
    dir: PathBuf,
    schema: SchemaRef,
    // Write-buffer columns
    buf_ts: Vec<i64>,
    buf_metric: Vec<String>,
    buf_labels: Vec<String>,
    buf_value: Vec<f64>,
    flush_threshold: usize,
}

impl TsdbStore {
    pub fn open(dir: impl AsRef<Path>) -> Result<Self> {
        let dir = dir.as_ref().to_path_buf();
        fs::create_dir_all(&dir)?;
        Ok(Self {
            dir,
            schema: tsdb_schema(),
            buf_ts: Vec::with_capacity(65_536),
            buf_metric: Vec::with_capacity(65_536),
            buf_labels: Vec::with_capacity(65_536),
            buf_value: Vec::with_capacity(65_536),
            flush_threshold: 1_000_000,
        })
    }

    pub fn with_flush_threshold(mut self, n: usize) -> Self {
        self.flush_threshold = n;
        self
    }

    /// Append a [`RecordBatch`] matching the tsdb schema.
    pub fn append(&mut self, batch: RecordBatch) -> Result<()> {
        let rows = batch.num_rows();
        let ts_col = batch
            .column(0)
            .as_any()
            .downcast_ref::<Int64Array>()
            .context("ts column must be Int64")?;
        let metric_col = batch
            .column(1)
            .as_any()
            .downcast_ref::<StringArray>()
            .context("metric column must be Utf8")?;
        let labels_col = batch
            .column(2)
            .as_any()
            .downcast_ref::<StringArray>()
            .context("labels column must be Utf8")?;
        let value_col = batch
            .column(3)
            .as_any()
            .downcast_ref::<Float64Array>()
            .context("value column must be Float64")?;

        for i in 0..rows {
            self.buf_ts.push(ts_col.value(i));
            self.buf_metric.push(metric_col.value(i).to_string());
            self.buf_labels.push(if labels_col.is_null(i) {
                "{}".to_string()
            } else {
                labels_col.value(i).to_string()
            });
            self.buf_value.push(if value_col.is_null(i) {
                f64::NAN
            } else {
                value_col.value(i)
            });
        }
        if self.buf_ts.len() >= self.flush_threshold {
            self.flush()?;
        }
        Ok(())
    }

    /// Append rows from plain slices (convenience wrapper).
    pub fn append_rows(
        &mut self,
        ts: &[i64],
        metrics: &[&str],
        labels: &[HashMap<String, String>],
        values: &[f64],
    ) -> Result<()> {
        let n = ts.len();
        assert_eq!(n, metrics.len());
        assert_eq!(n, values.len());
        assert_eq!(n, labels.len());

        let mut ts_b = Int64Builder::new();
        let mut m_b = StringBuilder::new();
        let mut l_b = StringBuilder::new();
        let mut v_b = Float64Builder::new();

        for i in 0..n {
            ts_b.append_value(ts[i]);
            m_b.append_value(metrics[i]);
            l_b.append_value(serde_json::to_string(&labels[i]).unwrap_or_default());
            v_b.append_value(values[i]);
        }

        let batch = RecordBatch::try_new(
            self.schema.clone(),
            vec![
                Arc::new(ts_b.finish()),
                Arc::new(m_b.finish()),
                Arc::new(l_b.finish()),
                Arc::new(v_b.finish()),
            ],
        )?;
        self.append(batch)
    }

    /// Query rows where `metric == metric` and `from <= ts <= to` (ms).
    pub fn query_range(&self, metric: &str, from: i64, to: i64) -> Result<RecordBatch> {
        let mut ts_b = Int64Builder::new();
        let mut m_b = StringBuilder::new();
        let mut l_b = StringBuilder::new();
        let mut v_b = Float64Builder::new();

        // Buffer scan
        for (i, &ts) in self.buf_ts.iter().enumerate() {
            if ts >= from && ts <= to && self.buf_metric[i] == metric {
                ts_b.append_value(ts);
                m_b.append_value(&self.buf_metric[i]);
                l_b.append_value(&self.buf_labels[i]);
                v_b.append_value(self.buf_value[i]);
            }
        }

        // Disk scan
        for shard in self.shards_in_range(from, to)? {
            let file = fs::File::open(&shard)?;
            let builder = ParquetRecordBatchReaderBuilder::try_new(file)?;
            let reader = builder.build()?;
            for batch in reader {
                let batch = batch?;
                let ts_col = batch
                    .column(0)
                    .as_any()
                    .downcast_ref::<Int64Array>()
                    .unwrap();
                let m_col = batch
                    .column(1)
                    .as_any()
                    .downcast_ref::<StringArray>()
                    .unwrap();
                let l_col = batch
                    .column(2)
                    .as_any()
                    .downcast_ref::<StringArray>()
                    .unwrap();
                let v_col = batch
                    .column(3)
                    .as_any()
                    .downcast_ref::<Float64Array>()
                    .unwrap();
                for i in 0..batch.num_rows() {
                    let ts = ts_col.value(i);
                    if ts >= from && ts <= to && m_col.value(i) == metric {
                        ts_b.append_value(ts);
                        m_b.append_value(m_col.value(i));
                        l_b.append_value(if l_col.is_null(i) {
                            "{}"
                        } else {
                            l_col.value(i)
                        });
                        v_b.append_value(if v_col.is_null(i) {
                            f64::NAN
                        } else {
                            v_col.value(i)
                        });
                    }
                }
            }
        }

        Ok(RecordBatch::try_new(
            self.schema.clone(),
            vec![
                Arc::new(ts_b.finish()),
                Arc::new(m_b.finish()),
                Arc::new(l_b.finish()),
                Arc::new(v_b.finish()),
            ],
        )?)
    }

    /// Time-windowed aggregation. Returns RecordBatch with ts=window_start, value=agg.
    pub fn aggregate(&self, metric: &str, agg: AggOp, window: Duration) -> Result<RecordBatch> {
        let window_ms = window.as_millis() as i64;
        let mut pairs: Vec<(i64, f64)> = self
            .buf_ts
            .iter()
            .enumerate()
            .filter(|(i, _)| self.buf_metric[*i] == metric)
            .map(|(i, &ts)| (ts, self.buf_value[i]))
            .collect();

        for shard in self.all_shards()? {
            let file = fs::File::open(&shard)?;
            let builder = ParquetRecordBatchReaderBuilder::try_new(file)?;
            let reader = builder.build()?;
            for batch in reader {
                let batch = batch?;
                let ts_col = batch
                    .column(0)
                    .as_any()
                    .downcast_ref::<Int64Array>()
                    .unwrap();
                let m_col = batch
                    .column(1)
                    .as_any()
                    .downcast_ref::<StringArray>()
                    .unwrap();
                let v_col = batch
                    .column(3)
                    .as_any()
                    .downcast_ref::<Float64Array>()
                    .unwrap();
                for i in 0..batch.num_rows() {
                    if m_col.value(i) == metric {
                        pairs.push((ts_col.value(i), v_col.value(i)));
                    }
                }
            }
        }

        pairs.sort_unstable_by_key(|&(ts, _)| ts);

        let mut ts_b = Int64Builder::new();
        let mut m_b = StringBuilder::new();
        let mut l_b = StringBuilder::new();
        let mut v_b = Float64Builder::new();

        let mut i = 0;
        while i < pairs.len() {
            let win_start = (pairs[i].0 / window_ms) * window_ms;
            let win_end = win_start + window_ms;
            let mut vals: Vec<f64> = Vec::new();
            while i < pairs.len() && pairs[i].0 < win_end {
                vals.push(pairs[i].1);
                i += 1;
            }
            let v = match agg {
                AggOp::Avg => vals.iter().sum::<f64>() / vals.len() as f64,
                AggOp::Sum => vals.iter().sum(),
                AggOp::Max => vals.iter().cloned().fold(f64::NEG_INFINITY, f64::max),
                AggOp::Min => vals.iter().cloned().fold(f64::INFINITY, f64::min),
                AggOp::Count => vals.len() as f64,
            };
            ts_b.append_value(win_start);
            m_b.append_value(metric);
            l_b.append_value("{}");
            v_b.append_value(v);
        }

        Ok(RecordBatch::try_new(
            self.schema.clone(),
            vec![
                Arc::new(ts_b.finish()),
                Arc::new(m_b.finish()),
                Arc::new(l_b.finish()),
                Arc::new(v_b.finish()),
            ],
        )?)
    }

    /// Flush write-buffer to Parquet shards on disk.
    pub fn flush(&mut self) -> Result<()> {
        if self.buf_ts.is_empty() {
            return Ok(());
        }
        use chrono::Timelike;

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
            let shard_path = shard_dir.join(format!("{:02}.parquet", hour));

            // Read existing shard rows if present.
            let mut existing_ts: Vec<i64> = vec![];
            let mut existing_m: Vec<String> = vec![];
            let mut existing_l: Vec<String> = vec![];
            let mut existing_v: Vec<f64> = vec![];

            if shard_path.exists() {
                let file = fs::File::open(&shard_path)?;
                let builder = ParquetRecordBatchReaderBuilder::try_new(file)?;
                let reader = builder.build()?;
                for batch in reader {
                    let batch = batch?;
                    let tc = batch
                        .column(0)
                        .as_any()
                        .downcast_ref::<Int64Array>()
                        .unwrap();
                    let mc = batch
                        .column(1)
                        .as_any()
                        .downcast_ref::<StringArray>()
                        .unwrap();
                    let lc = batch
                        .column(2)
                        .as_any()
                        .downcast_ref::<StringArray>()
                        .unwrap();
                    let vc = batch
                        .column(3)
                        .as_any()
                        .downcast_ref::<Float64Array>()
                        .unwrap();
                    for i in 0..batch.num_rows() {
                        existing_ts.push(tc.value(i));
                        existing_m.push(mc.value(i).to_string());
                        existing_l.push(if lc.is_null(i) {
                            "{}".to_string()
                        } else {
                            lc.value(i).to_string()
                        });
                        existing_v.push(if vc.is_null(i) { f64::NAN } else { vc.value(i) });
                    }
                }
            }

            // Append new rows.
            for &i in indices {
                existing_ts.push(self.buf_ts[i]);
                existing_m.push(self.buf_metric[i].clone());
                existing_l.push(self.buf_labels[i].clone());
                existing_v.push(self.buf_value[i]);
            }

            // Write merged shard.
            let mut ts_b = Int64Builder::new();
            let mut m_b = StringBuilder::new();
            let mut l_b = StringBuilder::new();
            let mut v_b = Float64Builder::new();
            for i in 0..existing_ts.len() {
                ts_b.append_value(existing_ts[i]);
                m_b.append_value(&existing_m[i]);
                l_b.append_value(&existing_l[i]);
                v_b.append_value(existing_v[i]);
            }
            let batch = RecordBatch::try_new(
                self.schema.clone(),
                vec![
                    Arc::new(ts_b.finish()),
                    Arc::new(m_b.finish()),
                    Arc::new(l_b.finish()),
                    Arc::new(v_b.finish()),
                ],
            )?;

            let props = WriterProperties::builder()
                .set_compression(Compression::SNAPPY)
                .build();
            let file = fs::File::create(&shard_path)?;
            let mut writer = ArrowWriter::try_new(file, self.schema.clone(), Some(props))?;
            writer.write(&batch)?;
            writer.close()?;
        }

        self.buf_ts.clear();
        self.buf_metric.clear();
        self.buf_labels.clear();
        self.buf_value.clear();
        Ok(())
    }

    fn shards_in_range(&self, from: i64, to: i64) -> Result<Vec<PathBuf>> {
        let from_dt = Utc.timestamp_millis_opt(from).single().unwrap_or_default();
        let to_dt = Utc.timestamp_millis_opt(to).single().unwrap_or_default();
        let from_date = from_dt.format("%Y-%m-%d").to_string();
        let to_date = to_dt.format("%Y-%m-%d").to_string();
        let mut paths = vec![];
        if !self.dir.exists() {
            return Ok(paths);
        }
        for entry in fs::read_dir(&self.dir)?.flatten() {
            let p = entry.path();
            if !p.is_dir() {
                continue;
            }
            let name = p
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("")
                .to_string();
            if name < from_date || name > to_date {
                continue;
            }
            for h in fs::read_dir(&p)?.flatten() {
                let hp = h.path();
                if hp.extension().and_then(|e| e.to_str()) == Some("parquet") {
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
                    if hp.extension().and_then(|e| e.to_str()) == Some("parquet") {
                        paths.push(hp);
                    }
                }
            }
        }
        Ok(paths)
    }
}

impl Drop for TsdbStore {
    fn drop(&mut self) {
        let _ = self.flush();
    }
}
