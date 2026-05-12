/// Append-only OHLCV series backed by mmap pages + a .smx.idx index file.
///
/// Index file format: line-delimited text — each line: `ts_min ts_max page_offset`
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::ops::Range;

use crate::store::mmap::MmapFile;
use crate::store::page::{encode_page, decode_page, Bar, MAX_ROWS};

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
        self.range_filter(range, None)
    }

    /// Fetch bars in timestamp range AND optional price range (close ∈ price_range).
    /// Returns (bars, pages_scanned, pages_skipped).
    pub fn range_filter_stats(
        &mut self,
        ts_range: Range<i64>,
        price_range: Option<Range<f32>>,
    ) -> std::io::Result<(Vec<Bar>, usize, usize)> {
        self.flush_pending()?;

        let mut result = Vec::new();
        let mut scanned = 0usize;
        let mut skipped = 0usize;

        for &(ts_min, ts_max, page_idx) in &self.index {
            if ts_max < ts_range.start || ts_min >= ts_range.end {
                skipped += 1;
                continue;
            }
            scanned += 1;
            let page = self.mmap.read_page(page_idx)?;
            let (_hdr, bars) = decode_page(&page);
            for bar in bars {
                if bar.ts >= ts_range.start && bar.ts < ts_range.end {
                    if let Some(ref pr) = price_range {
                        if bar.close < pr.start || bar.close >= pr.end {
                            continue;
                        }
                    }
                    result.push(bar);
                }
            }
        }
        result.sort_unstable_by_key(|b| b.ts);
        Ok((result, scanned, skipped))
    }

    /// Fetch bars in timestamp range AND optional price range.
    pub fn range_filter(
        &mut self,
        ts_range: Range<i64>,
        price_range: Option<Range<f32>>,
    ) -> std::io::Result<Vec<Bar>> {
        let (bars, _, _) = self.range_filter_stats(ts_range, price_range)?;
        Ok(bars)
    }

    /// Fetch only requested columns from pages in ts_range.
    /// Returns bars with only the requested columns populated (others = 0).
    /// `cols`: subset of ["open","high","low","close","volume"] — ts always returned.
    pub fn range_with_columns(
        &mut self,
        ts_range: Range<i64>,
        cols: &[&str],
    ) -> std::io::Result<Vec<Bar>> {
        self.flush_pending()?;

        let want_open   = cols.contains(&"open");
        let want_high   = cols.contains(&"high");
        let want_low    = cols.contains(&"low");
        let want_close  = cols.contains(&"close");
        let want_volume = cols.contains(&"volume");

        let mut result = Vec::new();
        for &(ts_min, ts_max, page_idx) in &self.index {
            if ts_max < ts_range.start || ts_min >= ts_range.end {
                continue;
            }
            let page = self.mmap.read_page(page_idx)?;
            let (hdr, bars) = decode_page(&page);
            let _ = hdr;
            for bar in bars {
                if bar.ts >= ts_range.start && bar.ts < ts_range.end {
                    result.push(Bar {
                        ts:     bar.ts,
                        open:   if want_open   { bar.open   } else { 0.0 },
                        high:   if want_high   { bar.high   } else { 0.0 },
                        low:    if want_low    { bar.low    } else { 0.0 },
                        close:  if want_close  { bar.close  } else { 0.0 },
                        volume: if want_volume { bar.volume } else { 0.0 },
                    });
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
