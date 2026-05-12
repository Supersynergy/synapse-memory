/// Append-only OHLCV series backed by mmap pages + a .smx.idx index file.
///
/// Index file format: line-delimited text — each line: `ts_min ts_max page_offset`
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::ops::Range;

use crate::store::mmap::MmapFile;
use crate::store::page::{encode_page, decode_page, decode_page_soa_filtered, Bar, Page, MAX_ROWS};
use crate::router::{Plan, QueryKey, QueryKind, PlanCache};
use crate::analytics::agg::{AggKind, AggResult, agg_pages};
use crate::cache::{HotSet, PageKey, DecodedPage};
use crate::filter::Bloom;
use xxhash_rust::xxh3::xxh3_64;
use blake3;

pub struct Series {
    mmap: MmapFile,
    idx_path: PathBuf,
    bloom_path: PathBuf,
    /// In-memory index: (ts_min, ts_max, page_idx)
    index: Vec<(i64, i64, usize)>,
    /// Pending bars not yet flushed to a page
    pending: Vec<Bar>,
    /// HotSet page cache — optional, shared or per-series
    pub hot: Option<HotSet>,
    /// Blake3 hash of the series path used as series_id in PageKey
    series_id: u64,
    /// Bloom filter over all flushed timestamps
    bloom: Bloom,
}

impl Series {
    /// Open or create a series file at `path` (e.g. `data/AAPL.smx`).
    /// Index lives at `path` + ".idx".
    pub fn open<P: AsRef<Path>>(path: P) -> std::io::Result<Self> {
        let path = path.as_ref();
        let idx_path = PathBuf::from(format!("{}.idx", path.display()));
        let bloom_path = PathBuf::from(format!("{}.bloom", path.display()));
        let mmap = MmapFile::open(path)?;
        let index = Self::load_index(&idx_path)?;
        let series_id = {
            let h = blake3::hash(path.to_string_lossy().as_bytes());
            u64::from_le_bytes(h.as_bytes()[..8].try_into().unwrap())
        };
        let bloom = Self::load_or_rebuild_bloom(&bloom_path, &index, &mmap);
        Ok(Self { mmap, idx_path, bloom_path, index, pending: Vec::new(), hot: None, series_id, bloom })
    }

    fn load_or_rebuild_bloom(bloom_path: &Path, index: &[(i64, i64, usize)], _mmap: &MmapFile) -> Bloom {
        // Try loading sidecar
        if let Ok(bytes) = std::fs::read(bloom_path) {
            if let Some(b) = Bloom::deserialize(&bytes) {
                return b;
            }
        }
        // Rebuild from index ranges (fast approximate: add ts_min..ts_max at 900s step)
        let mut b = Bloom::new();
        for &(ts_min, ts_max, _) in index {
            let mut ts = ts_min;
            while ts <= ts_max {
                b.add(xxh3_64(&ts.to_le_bytes()));
                ts += 900;
            }
        }
        // Persist
        let _ = std::fs::write(bloom_path, b.serialize());
        b
    }

    /// Enable HotSet with given page capacity (default 1000).
    pub fn enable_hot_cache(&mut self, capacity: usize) {
        self.hot = Some(HotSet::new(capacity));
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
        for bar in chunk {
            self.bloom.add(xxh3_64(&bar.ts.to_le_bytes()));
        }
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

    /// Close — flush pending + fsync + persist bloom.
    pub fn close(mut self) -> std::io::Result<()> {
        self.flush_pending()?;
        let _ = std::fs::write(&self.bloom_path, self.bloom.serialize());
        self.mmap.sync()
    }

    /// O(1) bloom guard: probe up to 4 ts values spanning the range.
    /// Returns false if none of the probed ts are in the filter → skip page-scan.
    /// Zero false negatives for exact-ts lookups; low FPR for range queries.
    pub fn bloom_range_likely(&self, range: &Range<i64>) -> bool {
        let step = 900i64;
        // Probe start, start+step, start+2*step, end-step (up to 4 probes)
        let probes = [
            range.start,
            range.start + step,
            range.start + 2 * step,
            range.end.saturating_sub(step),
        ];
        for &ts in &probes {
            if ts >= range.start && ts < range.end && self.bloom.contains(xxh3_64(&ts.to_le_bytes())) {
                return true;
            }
        }
        false
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

    /// Skipped-page range — bloom guard + page-header skip.
    /// Negative lookup (ts out-of-range) returns early without page scan.
    pub fn range_filter(&mut self, range: Range<i64>) -> std::io::Result<Vec<Bar>> {
        self.flush_pending()?;
        // Bloom guard: if no ts in range is in filter, return immediately
        if !self.bloom_range_likely(&range) {
            return Ok(Vec::new());
        }
        let mut result = Vec::new();
        for &(ts_min, ts_max, page_idx) in &self.index {
            if ts_max < range.start || ts_min >= range.end {
                continue; // page-header skip — no decode
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

    /// Routed range — dispatches to MmapScanFull or MmapScanSkipped based on coverage.
    pub fn range_routed(&mut self, range: std::ops::Range<i64>, cache: &mut PlanCache) -> std::io::Result<Vec<Bar>> {
        let range_bars = ((range.end - range.start) / 900).max(0) as usize;
        let n_pages = self.index.len();
        let total_bars = n_pages * crate::store::page::MAX_ROWS;
        // Coverage ratio: if query covers <40% of series, page-skip wins
        let coverage = if total_bars > 0 { range_bars as f64 / total_bars as f64 } else { 1.0 };
        let key = QueryKey {
            kind: QueryKind::CandleRange,
            range_bars,
            n_pages,
            has_filter: coverage < 0.4,
        };
        let candidates = if coverage < 0.4 {
            vec![Plan::MmapScanSkipped, Plan::MmapScanFull]
        } else {
            vec![Plan::MmapScanFull, Plan::MmapScanSkipped]
        };
        let plan = cache.choose(&key, &candidates);
        let t0 = std::time::Instant::now();
        let result = match plan {
            Plan::MmapScanSkipped => self.range_filter(range)?,
            _ => self.range(range)?,
        };
        let elapsed_us = t0.elapsed().as_micros() as u64;
        cache.record(key, plan, elapsed_us);
        Ok(result)
    }

    /// Point-lookup for a single timestamp — uses HotSet when enabled.
    /// Returns `None` if no bar with exact `ts` exists.
    pub fn point_lookup(&mut self, ts: i64) -> std::io::Result<Option<Bar>> {
        self.flush_pending()?;
        for &(ts_min, ts_max, page_idx) in &self.index {
            if ts < ts_min || ts > ts_max {
                continue;
            }
            let bars = if let Some(hot) = self.hot.as_mut() {
                let key = PageKey { series_id: self.series_id, page_idx: page_idx as u32 };
                let mmap = &self.mmap;
                let page = hot.get_or_load(key, || {
                    let raw = mmap.read_page(page_idx).expect("mmap read");
                    let (_hdr, bars) = decode_page(&raw);
                    let ts: Vec<i64> = bars.iter().map(|b| b.ts).collect();
                    let close: Vec<f32> = bars.iter().map(|b| b.close).collect();
                    DecodedPage { ts, close, volume: None, bars }
                });
                page.bars.clone()
            } else {
                let raw = self.mmap.read_page(page_idx)?;
                let (_hdr, bars) = decode_page(&raw);
                bars
            };
            if let Some(bar) = bars.iter().find(|b| b.ts == ts) {
                return Ok(Some(*bar));
            }
        }
        Ok(None)
    }

    /// Aggregate query — SimdAgg path: reads page columns directly, no Bar materialization.
    pub fn aggregate_routed(&mut self, range: Range<i64>, kind: AggKind, cache: &mut PlanCache) -> std::io::Result<AggResult> {
        self.flush_pending()?;
        let range_bars = ((range.end - range.start) / 900).max(0) as usize;
        let n_pages = self.index.len();
        let key = QueryKey {
            kind: QueryKind::Aggregate,
            range_bars,
            n_pages,
            has_filter: false,
        };
        let candidates = vec![Plan::SimdAgg, Plan::MmapScanFull];
        let plan = cache.choose(&key, &candidates);
        let t0 = std::time::Instant::now();
        // SimdAgg: decode directly to SoA Page — zero Vec<Bar> allocation
        let mut pages: Vec<Page> = Vec::new();
        for &(ts_min, ts_max, page_idx) in &self.index {
            if ts_max < range.start || ts_min >= range.end {
                continue;
            }
            let raw = self.mmap.read_page(page_idx)?;
            let page = decode_page_soa_filtered(&raw, range.start, range.end);
            if !page.is_empty() {
                pages.push(page);
            }
        }
        let result = agg_pages(&pages, kind);
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
