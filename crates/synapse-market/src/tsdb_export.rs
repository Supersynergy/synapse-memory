//! Arrow IPC export of OHLCV data via synapse-tsdb.
//!
//! Feature gate: `tsdb-export`
//!
//! Provides `Market::export_arrow(ticker, ts_start, ts_end)` → Arrow RecordBatch
//! (columns: ts i64, open f64, high f64, low f64, close f64, volume f64).

use std::ops::Range;
use std::sync::Arc;

use anyhow::{Context, Result};
use arrow::array::{Float64Builder, Int64Builder};
use arrow::datatypes::{DataType, Field, Schema};
use arrow::ipc::reader::StreamReader;
use arrow::ipc::writer::StreamWriter;
pub use arrow::record_batch::RecordBatch;

use crate::Market;

/// Arrow schema for OHLCV export.
pub fn ohlcv_schema() -> Arc<Schema> {
    Arc::new(Schema::new(vec![
        Field::new("ts", DataType::Int64, false),
        Field::new("open", DataType::Float64, false),
        Field::new("high", DataType::Float64, false),
        Field::new("low", DataType::Float64, false),
        Field::new("close", DataType::Float64, false),
        Field::new("volume", DataType::Float64, false),
    ]))
}

impl Market {
    /// Export OHLCV rows for `ticker` in `ts_range` as an Arrow [`RecordBatch`].
    ///
    /// `ts_range` is `[start, end)` in milliseconds since epoch (same as SQLite schema).
    /// Returns an empty batch (0 rows) if the range is empty.
    pub fn export_arrow(&self, ticker: &str, ts_range: Range<i64>) -> Result<RecordBatch> {
        let rows = crate::ohlcv::fetch_range(&self.conn, ticker, ts_range.start, ts_range.end)
            .context("fetch_range")?;

        let schema = ohlcv_schema();
        let n = rows.len();

        let mut ts_b = Int64Builder::with_capacity(n);
        let mut open_b = Float64Builder::with_capacity(n);
        let mut high_b = Float64Builder::with_capacity(n);
        let mut low_b = Float64Builder::with_capacity(n);
        let mut close_b = Float64Builder::with_capacity(n);
        let mut volume_b = Float64Builder::with_capacity(n);

        for (ts, o, h, l, c, v) in rows {
            ts_b.append_value(ts);
            open_b.append_value(o);
            high_b.append_value(h);
            low_b.append_value(l);
            close_b.append_value(c);
            volume_b.append_value(v);
        }

        RecordBatch::try_new(
            schema,
            vec![
                Arc::new(ts_b.finish()),
                Arc::new(open_b.finish()),
                Arc::new(high_b.finish()),
                Arc::new(low_b.finish()),
                Arc::new(close_b.finish()),
                Arc::new(volume_b.finish()),
            ],
        )
        .context("build RecordBatch")
    }

    /// Serialize a RecordBatch to Arrow IPC stream bytes.
    pub fn arrow_to_ipc(batch: &RecordBatch) -> Result<Vec<u8>> {
        let mut buf = Vec::new();
        {
            let mut writer = StreamWriter::try_new(&mut buf, &batch.schema())
                .context("StreamWriter::try_new")?;
            writer.write(batch).context("write batch")?;
            writer.finish().context("finish")?;
        }
        Ok(buf)
    }

    /// Deserialize Arrow IPC stream bytes back to a RecordBatch.
    pub fn ipc_to_arrow(bytes: &[u8]) -> Result<RecordBatch> {
        let mut reader = StreamReader::try_new(std::io::Cursor::new(bytes), None)
            .context("StreamReader::try_new")?;
        let batch = reader
            .next()
            .context("no batch in IPC stream")?
            .context("IPC read error")?;
        Ok(batch)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arrow::array::{Float64Array, Int64Array};
    use tempfile::NamedTempFile;

    fn sample_rows() -> Vec<crate::OhlcvRow> {
        vec![
            (1_000, 100.0, 105.0, 99.0, 103.0, 1000.0),
            (2_000, 103.0, 108.0, 102.0, 107.0, 1500.0),
            (3_000, 107.0, 110.0, 106.0, 109.0, 2000.0),
        ]
    }

    #[test]
    fn test_export_arrow_roundtrip_ipc() {
        let tmp = NamedTempFile::new().unwrap();
        let market = Market::open(tmp.path()).unwrap();

        let rows = sample_rows();
        crate::ohlcv::ingest(&market.conn, "AAPL", &rows).unwrap();

        let batch = market.export_arrow("AAPL", 0..10_000).unwrap();
        assert_eq!(batch.num_rows(), 3);
        assert_eq!(batch.num_columns(), 6);

        let ts_col = batch
            .column(0)
            .as_any()
            .downcast_ref::<Int64Array>()
            .unwrap();
        assert_eq!(ts_col.value(0), 1_000);
        assert_eq!(ts_col.value(2), 3_000);

        let close_col = batch
            .column(4)
            .as_any()
            .downcast_ref::<Float64Array>()
            .unwrap();
        assert!((close_col.value(0) - 103.0).abs() < 1e-9);

        // IPC roundtrip
        let ipc = Market::arrow_to_ipc(&batch).unwrap();
        assert!(!ipc.is_empty());
        let batch2 = Market::ipc_to_arrow(&ipc).unwrap();
        assert_eq!(batch2.num_rows(), 3);

        let close2 = batch2
            .column(4)
            .as_any()
            .downcast_ref::<Float64Array>()
            .unwrap();
        for i in 0..3 {
            let orig = batch
                .column(4)
                .as_any()
                .downcast_ref::<Float64Array>()
                .unwrap();
            assert!((close2.value(i) - orig.value(i)).abs() < 1e-12);
        }
    }

    #[test]
    fn test_export_arrow_empty_range() {
        let tmp = NamedTempFile::new().unwrap();
        let market = Market::open(tmp.path()).unwrap();

        crate::ohlcv::ingest(&market.conn, "TSLA", &sample_rows()).unwrap();
        let batch = market.export_arrow("TSLA", 10_000..20_000).unwrap();
        assert_eq!(batch.num_rows(), 0);
    }
}
