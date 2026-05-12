/// Append-only OHLCV series backed by mmap pages + a .smx.idx index file.
///
/// Index file format: line-delimited text — each line: `ts_min ts_max page_offset`
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::ops::Range;

use crate::store::mmap::MmapFile;
use crate::store::page::{encode_page, decode_page, decode_page_soa_filtered, Bar, Page, MAX_ROWS};
use crate::store::compact::ColdTier;
use crate::router::{Plan, QueryKey, QueryKind, PlanCache};
use crate::analytics::agg::{AggKind, AggResult, agg_pages};
use crate::cache::{HotSet, PageKey, DecodedPage};
use crate::filter::{Bloom, SeriesXorFilter};
use xxhash_rust::xxh3::xxh3_64;
use blake3;
use crate::learn::OnlineLearner;

pub struct Series {
    mmap: MmapFile,
    idx_path: PathBuf,
    bloom_path: PathBuf,
    /// Named online learners — state persisted in .lnr sidecar.
    learners: std::collections::HashMap<String, Box<dyn OnlineLearner>>,
    /// Path for learner sidecar.
    lnr_path: PathBuf,
    /// In-memory index: (ts_min, ts_max, page_idx)
    index: Vec<(i64, i64, usize)>,
    /// Pending bars not yet flushed to a page
    pending: Vec<Bar>,
    /// HotSet page cache — optional, shared or per-series
    pub hot: Option<HotSet>,
    /// Blake3 hash of the series path used as series_id in PageKey
    series_id: u64,
    /// Bloom filter over all flushed timestamps (used when n_keys < 100_000).
    bloom: Bloom,
    /// Xor-filter built at close for large series (n_keys ≥ 100_000).
    xor_filter: Option<SeriesXorFilter>,
    /// Path for the xor-filter sidecar (.xor).
    xor_path: PathBuf,
    /// All hashed keys accumulated for xor-filter build at close.
    xor_keys: Vec<u64>,
    /// Minimum page count before bloom guard is consulted at query time.
    ///
    /// At low page counts the bloom overhead (hash + bit-test) can exceed the
    /// cost of a straight header-skip scan.  The crossover — measured by
    /// `benches/scale_curve.rs` — is around **100 pages**.  Below this threshold
    /// `range_filter` falls through to the plain header-skip loop.
    ///
    /// Set to `0` to always use bloom; set to `usize::MAX` to disable.
    pub bloom_min_pages: usize,
    /// Cold-tier (zstd-19 compressed) — present after compact_to_cold.
    cold: Option<ColdTier>,
    /// Path for cold-tier file (base_path + ".csm")
    cold_path: PathBuf,
    /// Hot-index entries that have been moved to cold (page_idx values).
    cold_index: Vec<(i64, i64, usize)>, // (ts_min, ts_max, cold_page_idx)
}

impl Series {
    /// Open or create a series file at `path` (e.g. `data/AAPL.smx`).
    /// Index lives at `path` + ".idx".
    pub fn open<P: AsRef<Path>>(path: P) -> std::io::Result<Self> {
        let path = path.as_ref();
        let idx_path = PathBuf::from(format!("{}.idx", path.display()));
        let bloom_path = PathBuf::from(format!("{}.bloom", path.display()));
        let xor_path = PathBuf::from(format!("{}.xor", path.display()));
        let cold_path = PathBuf::from(format!("{}.csm", path.display()));
        let lnr_path = PathBuf::from(format!("{}.lnr", path.display()));
        let mmap = MmapFile::open(path)?;
        let index = Self::load_index(&idx_path)?;
        let cold_index = Self::load_cold_index(&cold_path);
        let series_id = {
            let h = blake3::hash(path.to_string_lossy().as_bytes());
            u64::from_le_bytes(h.as_bytes()[..8].try_into().unwrap())
        };
        let bloom = Self::load_or_rebuild_bloom(&bloom_path, &index, &mmap);
        let xor_filter = if xor_path.exists() {
            std::fs::read(&xor_path).ok().and_then(|b| SeriesXorFilter::deserialize(&b))
        } else {
            None
        };
        let cold = if cold_path.exists() {
            ColdTier::open(&cold_path).ok()
        } else {
            None
        };
        // Bloom is disabled by default (bloom_min_pages = usize::MAX) because the
        // current fixed 128 K-bit filter saturates past ~100 pages (FPR → 100%).
        // Re-enable after auto-scaling bloom bits proportional to n_pages.
        // See BLOOM_SCALE_REPORT.md for full analysis.
        Ok(Self {
            mmap, idx_path, bloom_path, xor_path, cold_path,
            index, pending: Vec::new(), hot: None,
            series_id, bloom, xor_filter, xor_keys: Vec::new(),
            bloom_min_pages: usize::MAX,
            cold, cold_index,
            learners: std::collections::HashMap::new(),
            lnr_path,
        })
    }

    fn load_cold_index(cold_path: &Path) -> Vec<(i64, i64, usize)> {
        let idx_path = PathBuf::from(format!("{}.idx", cold_path.display()));
        if !idx_path.exists() { return Vec::new(); }
        let f = match std::fs::File::open(&idx_path) { Ok(f) => f, Err(_) => return Vec::new() };
        let reader = std::io::BufReader::new(f);
        use std::io::BufRead;
        let mut out = Vec::new();
        for line in reader.lines() {
            let Ok(line) = line else { continue };
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() == 3 {
                let ts_min: i64 = parts[0].parse().unwrap_or(0);
                let ts_max: i64 = parts[1].parse().unwrap_or(0);
                let idx: usize = parts[2].parse().unwrap_or(0);
                out.push((ts_min, ts_max, idx));
            }
        }
        out
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
            let h = xxh3_64(&bar.ts.to_le_bytes());
            self.bloom.add(h);
            self.xor_keys.push(h);
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

    /// Close — flush pending + fsync + persist filter.
    /// For large series (≥100K keys) builds and persists an xor-filter (.xor sidecar).
    /// For small series persists bloom as before.
    pub fn close(mut self) -> std::io::Result<()> {
        self.flush_pending()?;
        const XOR_THRESHOLD: usize = 100_000;
        if self.xor_keys.len() >= XOR_THRESHOLD {
            if let Some(xf) = SeriesXorFilter::build(&self.xor_keys) {
                let _ = std::fs::write(&self.xor_path, xf.serialize());
            }
        } else {
            let _ = std::fs::write(&self.bloom_path, self.bloom.serialize());
        }
        self.mmap.sync()
    }

    /// O(1) xor guard: probe up to 4 ts values using the immutable xor-filter.
    /// Returns false → skip page-scan (negative lookup). Available only after close+reopen.
    pub fn xor_range_likely(&self, range: &Range<i64>) -> bool {
        let xf = match self.xor_filter.as_ref() {
            Some(xf) => xf,
            None => return true, // no xor filter yet → assume present
        };
        let step = 900i64;
        let probes = [
            range.start,
            range.start + step,
            range.start + 2 * step,
            range.end.saturating_sub(step),
        ];
        for &ts in &probes {
            if ts >= range.start && ts < range.end && xf.contains(xxh3_64(&ts.to_le_bytes())) {
                return true;
            }
        }
        false
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
    /// Checks hot mmap first, then cold-tier (decompress on demand).
    pub fn range(&mut self, range: Range<i64>) -> std::io::Result<Vec<Bar>> {
        // Flush pending first so they're visible
        self.flush_pending()?;

        let mut result = Vec::new();

        // Cold tier scan
        let cold_index = std::mem::take(&mut self.cold_index);
        if let Some(cold) = self.cold.as_mut() {
            for &(ts_min, ts_max, cold_idx) in &cold_index {
                if ts_max < range.start || ts_min >= range.end {
                    continue;
                }
                let bars = cold.read_page(cold_idx as u32)
                    .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;
                for bar in bars {
                    if bar.ts >= range.start && bar.ts < range.end {
                        result.push(bar);
                    }
                }
            }
        }
        self.cold_index = cold_index;

        // Hot tier scan
        for &(ts_min, ts_max, page_idx) in &self.index {
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
    ///
    /// Bloom guard is skipped when `index.len() < bloom_min_pages` (default 100) because
    /// below that threshold header-scan is cheaper than the bloom hash overhead.
    pub fn range_filter(&mut self, range: Range<i64>) -> std::io::Result<Vec<Bar>> {
        self.flush_pending()?;
        // Filter guard: prefer xor-filter when available, else bloom above threshold.
        if self.xor_filter.is_some() {
            if !self.xor_range_likely(&range) {
                return Ok(Vec::new());
            }
        } else if self.index.len() >= self.bloom_min_pages && !self.bloom_range_likely(&range) {
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

    /// Move pages older than `age_days` from hot mmap to cold zstd-19 tier.
    /// Idempotent if cold file already exists (appends are not supported; rebuild on repeat call).
    pub fn compact_to_cold(&mut self, age_days: u32) -> anyhow::Result<usize> {
        self.flush_pending()?;
        let cutoff_secs = age_days as i64 * 86400;
        let now_ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(i64::MAX);
        let cutoff = now_ts - cutoff_secs;

        // Collect hot pages older than cutoff
        let mut to_compact: Vec<(i64, i64, usize)> = Vec::new(); // (ts_min, ts_max, page_idx)
        for &(ts_min, ts_max, page_idx) in &self.index {
            if ts_max < cutoff {
                to_compact.push((ts_min, ts_max, page_idx));
            }
        }
        if to_compact.is_empty() {
            return Ok(0);
        }

        // Read raw pages from mmap
        let mut raw_pages: Vec<Vec<u8>> = Vec::with_capacity(to_compact.len());
        for &(_, _, page_idx) in &to_compact {
            let raw = self.mmap.read_page(page_idx)?;
            raw_pages.push(raw);
        }

        // Write cold file
        ColdTier::create(&self.cold_path, &raw_pages)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;

        // Write cold index sidecar
        let cold_idx_path = PathBuf::from(format!("{}.idx", self.cold_path.display()));
        {
            let mut f = std::fs::OpenOptions::new()
                .write(true).create(true).truncate(true)
                .open(&cold_idx_path)?;
            for (i, &(ts_min, ts_max, _)) in to_compact.iter().enumerate() {
                writeln!(f, "{} {} {}", ts_min, ts_max, i)?;
            }
        }

        // Remove compacted pages from hot index
        let compact_set: std::collections::HashSet<usize> = to_compact.iter().map(|&(_, _, idx)| idx).collect();
        self.index.retain(|&(_, _, idx)| !compact_set.contains(&idx));
        // Rebuild hot idx file
        {
            let mut f = std::fs::OpenOptions::new()
                .write(true).create(true).truncate(true)
                .open(&self.idx_path)?;
            for &(ts_min, ts_max, page_idx) in &self.index {
                writeln!(f, "{} {} {}", ts_min, ts_max, page_idx)?;
            }
        }

        // Update cold_index + reload ColdTier
        self.cold_index = to_compact.iter().enumerate().map(|(i, &(ts_min, ts_max, _))| (ts_min, ts_max, i)).collect();
        self.cold = ColdTier::open(&self.cold_path).ok();

        Ok(to_compact.len())
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
    /// Attach a named online learner.
    pub fn attach_learner(&mut self, name: &str, learner: Box<dyn OnlineLearner>) {
        self.learners.insert(name.to_string(), learner);
    }

    /// Feed one labeled sample to the named learner. Returns log-loss.
    pub fn update_learner(&mut self, name: &str, features: &[f32], y: f32) -> anyhow::Result<f32> {
        self.learners.get_mut(name)
            .map(|l| l.update(features, y))
            .ok_or_else(|| anyhow::anyhow!("learner '{}' not found", name))
    }

    /// Predict with the named learner.
    pub fn predict(&self, name: &str, features: &[f32]) -> anyhow::Result<f32> {
        self.learners.get(name)
            .map(|l| l.predict(features))
            .ok_or_else(|| anyhow::anyhow!("learner '{}' not found", name))
    }

    /// Persist all learner states to `.lnr` sidecar.
    /// Format: `[name_len u32 LE][name utf8][bytes_len u32 LE][bytes]*`
    pub fn save_learners(&self) -> anyhow::Result<()> {
        use std::io::Write as _;
        let mut buf = Vec::new();
        for (name, learner) in &self.learners {
            let name_bytes = name.as_bytes();
            let state = learner.serialize();
            buf.extend_from_slice(&(name_bytes.len() as u32).to_le_bytes());
            buf.extend_from_slice(name_bytes);
            buf.extend_from_slice(&(state.len() as u32).to_le_bytes());
            buf.extend_from_slice(&state);
        }
        let mut f = std::fs::File::create(&self.lnr_path)?;
        f.write_all(&buf)?;
        Ok(())
    }

    /// Load learner states from `.lnr` sidecar into attached learners.
    /// Only updates learners already attached (same name + deserializable state).
    pub fn load_learners(&mut self) -> anyhow::Result<()> {
        use crate::learn::ftrl::FtrlLearner;
        let bytes = match std::fs::read(&self.lnr_path) {
            Ok(b) => b,
            Err(_) => return Ok(()), // no sidecar yet
        };
        let mut pos = 0usize;
        while pos + 8 <= bytes.len() {
            let name_len = u32::from_le_bytes(bytes[pos..pos+4].try_into()?) as usize;
            pos += 4;
            if pos + name_len > bytes.len() { break; }
            let name = std::str::from_utf8(&bytes[pos..pos+name_len])?.to_string();
            pos += name_len;
            if pos + 4 > bytes.len() { break; }
            let state_len = u32::from_le_bytes(bytes[pos..pos+4].try_into()?) as usize;
            pos += 4;
            if pos + state_len > bytes.len() { break; }
            let state = &bytes[pos..pos+state_len];
            pos += state_len;
            if let Some(loaded) = FtrlLearner::deserialize_from(state) {
                self.learners.insert(name, Box::new(loaded));
            }
        }
        Ok(())
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
