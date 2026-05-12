/// Append-only OHLCV series backed by mmap pages + a .smx.idx index file.
///
/// Index file format: line-delimited text — each line: `ts_min ts_max page_offset`
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::ops::Range;

use crate::store::mmap::MmapFile;
use crate::store::page::{encode_page, decode_page, Bar, MAX_ROWS};
use crate::router::{Plan, QueryKey, QueryKind, PlanCache};

pub struct Series {
    mmap: MmapFile,
    idx_path: PathBuf,
    /// In-memory index: (ts_min, ts_max, page_idx)
    index: Vec<(i64, i64, usize)>,
    /// Pending bars not yet flushed to a page
    pending: Vec<Bar>,
}

impl Series {
    /// Open or create a series file at `path` (e.g. `data/AAPL.smx`).
    /// Index lives at `path` + ".idx".
    pub fn open<P: AsRef<Path>>(path: P) -> std::io::Result<Self> {
        let path = path.as_ref();
        let idx_path = PathBuf::from(format!("{}.idx", path.display()));
        let mmap = MmapFile::open(path)?;
        let index = Self::load_index(&idx_path)?;
        Ok(Self { mmap, idx_path, index, pending: Vec::new() })
    }

    fn load_index(idx_path: &Path) -> std::io::Result<Vec<(i64, i64, usize)>> {
        if !idx_path.exists() {
            return Ok(Vec::new());
        }
        let f = File::open(idx_path)?;
        let reader = BufReader::new(f);
        let mut index = Vec::new();
        for line in reader.lines() {
            let line = line?;
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() == 3 {
                let ts_min: i64 = parts[0].parse().unwrap_or(0);
                let ts_max: i64 = parts[1].parse().unwrap_or(0);
                let page_idx: usize = parts[2].parse().unwrap_or(0);
                index.push((ts_min, ts_max, page_idx));
            }
        }
        Ok(index)
    }

    fn append_index_entry(&mut self, ts_min: i64, ts_max: i64, page_idx: usize) -> std::io::Result<()> {
        let mut f = OpenOptions::new().append(true).create(true).open(&self.idx_path)?;
        writeln!(f, "{} {} {}", ts_min, ts_max, page_idx)?;
        self.index.push((ts_min, ts_max, page_idx));
        Ok(())
    }

    /// Append bars. Flushes complete pages (MAX_ROWS) immediately.
    pub fn append(&mut self, bars: &[Bar]) -> std::io::Result<()> {
        self.pending.extend_from_slice(bars);
        while self.pending.len() >= MAX_ROWS {
            let chunk: Vec<Bar> = self.pending.drain(..MAX_ROWS).collect();
            self.flush_chunk(&chunk)?;
        }
        Ok(())
    }

    fn flush_chunk(&mut self, chunk: &[Bar]) -> std::io::Result<()> {
        let page = encode_page(chunk);
        let page_idx = self.mmap.append_page(&page)?;
        let ts_min = chunk.iter().map(|b| b.ts).min().unwrap();
        let ts_max = chunk.iter().map(|b| b.ts).max().unwrap();
        self.append_index_entry(ts_min, ts_max, page_idx)?;
        Ok(())
    }

    /// Flush remaining pending bars (partial page).
    pub fn flush_pending(&mut self) -> std::io::Result<()> {
        if !self.pending.is_empty() {
            let chunk: Vec<Bar> = self.pending.drain(..).collect();
            self.flush_chunk(&chunk)?;
        }
        Ok(())
    }

    /// Close — flush pending + fsync.
    pub fn close(mut self) -> std::io::Result<()> {
        self.flush_pending()?;
        self.mmap.sync()
    }

    /// Fetch all bars in timestamp range [start, end).
    pub fn range(&mut self, range: Range<i64>) -> std::io::Result<Vec<Bar>> {
        // Flush pending first so they're visible
        self.flush_pending()?;

        let mut result = Vec::new();
        for &(ts_min, ts_max, page_idx) in &self.index {
            // Skip pages entirely outside range
            if ts_max < range.start || ts_min >= range.end {
                continue;
            }
            let page = self.mmap.read_page(page_idx)?;
            let (_hdr, bars) = decode_page(&page);
            for bar in bars {
                if bar.ts >= range.start && bar.ts < range.end {
                    result.push(bar);
                }
            }
        }
        result.sort_unstable_by_key(|b| b.ts);
        Ok(result)
    }

    /// Total bars (approximate — counts flushed only).
    pub fn flushed_page_count(&self) -> usize {
        self.index.len()
    }

    /// Routed range — uses PlanCache to pick execution strategy, records latency.
    pub fn range_routed(&mut self, range: std::ops::Range<i64>, cache: &mut PlanCache) -> std::io::Result<Vec<Bar>> {
        let range_bars = ((range.end - range.start) / 900).max(0) as usize;
        let n_pages = self.index.len();
        let key = QueryKey {
            kind: QueryKind::CandleRange,
            range_bars,
            n_pages,
            has_filter: false,
        };
        let candidates = if range_bars < 500 {
            vec![Plan::MmapScanSkipped, Plan::MmapScanFull]
        } else {
            vec![Plan::MmapScanFull, Plan::MmapScanSkipped]
        };
        let plan = cache.choose(&key, &candidates);
        let t0 = std::time::Instant::now();
        let result = self.range(range)?;
        let elapsed_us = t0.elapsed().as_micros() as u64;
        cache.record(key, plan, elapsed_us);
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn make_bars(n: usize, base_ts: i64) -> Vec<Bar> {
        (0..n)
            .map(|i| Bar {
                ts: base_ts + i as i64 * 900,
                open: 100.0,
                high: 101.0,
                low: 99.0,
                close: 100.5,
                volume: 1000.0,
            })
            .collect()
    }

    #[test]
    fn append_and_range() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("test.smx");
        let mut s = Series::open(&path).unwrap();
        let base = 1_700_000_000i64;
        let bars = make_bars(2880, base);
        s.append(&bars).unwrap();
        s.flush_pending().unwrap();
        let result = s.range(base..base + 2880 * 900).unwrap();
        assert_eq!(result.len(), 2880);
    }

    #[test]
    fn range_filter() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("test2.smx");
        let mut s = Series::open(&path).unwrap();
        let base = 1_700_000_000i64;
        let bars = make_bars(3000, base);
        s.append(&bars).unwrap();
        s.flush_pending().unwrap();
        // Only first 100 bars
        let end = base + 100 * 900;
        let result = s.range(base..end).unwrap();
        assert_eq!(result.len(), 100);
    }
}
