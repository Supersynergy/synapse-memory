//! Minimal LSM-tree: in-memory L0 (SkipMap) + SSTable stubs for L1+.
//!
//! L0 → SkipMap<Key, Entry>         (in-process, unsorted insert → sorted read)
//! L1+ → SSTable files on disk      (compaction: TODO full impl)
//!
//! L0 flush threshold: 4096 entries (configurable).

use crossbeam_skiplist::SkipMap;
use serde::{Deserialize, Serialize};
use std::ops::Range;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Opaque byte key.
pub type Key = Vec<u8>;

/// A single log-structured entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entry {
    pub key: Key,
    pub value: Vec<u8>,
    /// Monotonically increasing sequence number assigned at insert.
    pub seq: u64,
    /// Tombstone: true = delete marker.
    pub deleted: bool,
}

/// L0 in-memory level backed by a lock-free SkipMap.
pub struct L0 {
    map: Arc<SkipMap<Key, Entry>>,
    count: AtomicUsize,
    pub flush_threshold: usize,
}

impl L0 {
    pub fn new(flush_threshold: usize) -> Self {
        Self {
            map: Arc::new(SkipMap::new()),
            count: AtomicUsize::new(0),
            flush_threshold,
        }
    }

    /// Insert entry. Returns `true` if L0 is full and needs flushing.
    pub fn insert(&self, entry: Entry) -> bool {
        self.map.insert(entry.key.clone(), entry);
        self.count.fetch_add(1, Ordering::Relaxed) + 1 >= self.flush_threshold
    }

    /// Range scan over L0.
    pub fn scan(&self, range: &Range<Key>) -> Vec<Entry> {
        self.map
            .range(range.start.clone()..range.end.clone())
            .map(|e| e.value().clone())
            .collect()
    }

    pub fn len(&self) -> usize {
        self.count.load(Ordering::Relaxed)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Drain all entries (called before flush to SSTable).
    pub fn drain(&self) -> Vec<Entry> {
        let entries: Vec<Entry> = self.map.iter().map(|e| e.value().clone()).collect();
        // Reset: replace inner map is not possible with Arc; entries stay until GC.
        // For now, clear by re-creating is handled at store level via swap.
        self.count.store(0, Ordering::Relaxed);
        entries
    }
}

// ── Bloom Filter ─────────────────────────────────────────────────────────────

/// Bloom filter with 1% FPR at capacity, k=4 hashes (blake3 double-hashing).
///
/// Persisted as `<sstable>.bloom` alongside each SSTable.
pub struct BloomFilter {
    bits: Vec<u64>,
    /// Number of bit-slots (len(bits) * 64).
    m: usize,
}

impl BloomFilter {
    /// Create a new filter sized for `capacity` items at ~1% FPR.
    ///
    /// Formula: m = -n*ln(p) / (ln2)^2  ≈ n * 9.59  for p=0.01
    pub fn new(capacity: usize) -> Self {
        let m = ((capacity as f64 * 9.59).ceil() as usize).max(64);
        let words = m.div_ceil(64);
        Self {
            bits: vec![0u64; words],
            m: words * 64,
        }
    }

    /// Load from persisted bytes (raw little-endian u64 words).
    pub fn from_bytes(data: &[u8]) -> Self {
        let words: Vec<u64> = data
            .chunks_exact(8)
            .map(|c| u64::from_le_bytes(c.try_into().unwrap()))
            .collect();
        let m = words.len() * 64;
        Self { bits: words, m }
    }

    /// Serialize to bytes for persistence.
    pub fn to_bytes(&self) -> Vec<u8> {
        self.bits.iter().flat_map(|w| w.to_le_bytes()).collect()
    }

    fn hashes(&self, key: &[u8]) -> [usize; 4] {
        // Blake3 double-hashing: h1 = first 8 bytes, h2 = next 8 bytes
        let hash = blake3::hash(key);
        let b = hash.as_bytes();
        let h1 = u64::from_le_bytes(b[0..8].try_into().unwrap());
        let h2 = u64::from_le_bytes(b[8..16].try_into().unwrap());
        [
            (h1.wrapping_add(0u64.wrapping_mul(h2)) % self.m as u64) as usize,
            (h1.wrapping_add(1u64.wrapping_mul(h2)) % self.m as u64) as usize,
            (h1.wrapping_add(2u64.wrapping_mul(h2)) % self.m as u64) as usize,
            (h1.wrapping_add(3u64.wrapping_mul(h2)) % self.m as u64) as usize,
        ]
    }

    pub fn add(&mut self, key: &[u8]) {
        for bit in self.hashes(key) {
            self.bits[bit / 64] |= 1u64 << (bit % 64);
        }
    }

    pub fn contains(&self, key: &[u8]) -> bool {
        self.hashes(key)
            .iter()
            .all(|&bit| self.bits[bit / 64] & (1u64 << (bit % 64)) != 0)
    }
}

// ── SSTable ───────────────────────────────────────────────────────────────────

/// SSTable with bloom filter for fast negative lookups.
#[derive(Debug)]
pub struct SSTable {
    pub path: std::path::PathBuf,
    pub min_key: Key,
    pub max_key: Key,
    pub entry_count: usize,
}

impl SSTable {
    /// Write sorted entries + bloom filter to disk.
    ///
    /// Data file: `<path>` (length-prefixed JSON entries)
    /// Bloom file: `<path>.bloom` (raw u64 words)
    pub fn write(path: std::path::PathBuf, entries: &[Entry]) -> crate::error::Result<Self> {
        use std::io::Write;

        // Build bloom filter
        let mut bloom = BloomFilter::new(entries.len().max(1));
        for e in entries {
            bloom.add(&e.key);
        }

        // Write SSTable data
        let mut f = std::fs::File::create(&path)?;
        for e in entries {
            let bytes = serde_json::to_vec(e)
                .map_err(|e| crate::error::IoUringError::AppendFailed(e.to_string()))?;
            let len = (bytes.len() as u32).to_le_bytes();
            f.write_all(&len)?;
            f.write_all(&bytes)?;
        }
        f.sync_all()?;

        // Write bloom file
        let bloom_path = bloom_path_for(&path);
        std::fs::write(&bloom_path, bloom.to_bytes())?;

        Ok(SSTable {
            min_key: entries.first().map(|e| e.key.clone()).unwrap_or_default(),
            max_key: entries.last().map(|e| e.key.clone()).unwrap_or_default(),
            entry_count: entries.len(),
            path,
        })
    }

    /// Load bloom filter for this SSTable (returns None if file missing).
    pub fn load_bloom(&self) -> Option<BloomFilter> {
        let data = std::fs::read(bloom_path_for(&self.path)).ok()?;
        Some(BloomFilter::from_bytes(&data))
    }

    /// Read all entries from this SSTable, optionally filtered by bloom on a key.
    ///
    /// If `bloom_key` is Some and bloom says definitely-not-present, returns empty.
    pub fn read_entries(&self, bloom_key: Option<&[u8]>) -> crate::error::Result<Vec<Entry>> {
        use std::io::Read;

        if let Some(key) = bloom_key
            && let Some(bloom) = self.load_bloom()
            && !bloom.contains(key)
        {
            return Ok(vec![]);
        }

        let mut f = std::fs::File::open(&self.path)?;
        let mut entries = Vec::new();
        let mut len_buf = [0u8; 4];
        loop {
            match f.read_exact(&mut len_buf) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
                Err(e) => return Err(e.into()),
            }
            let len = u32::from_le_bytes(len_buf) as usize;
            let mut buf = vec![0u8; len];
            f.read_exact(&mut buf)?;
            let entry: Entry = serde_json::from_slice(&buf)
                .map_err(|e| crate::error::IoUringError::ReadFailed(e.to_string()))?;
            entries.push(entry);
        }
        Ok(entries)
    }
}

fn bloom_path_for(sst_path: &std::path::Path) -> std::path::PathBuf {
    let mut p = sst_path.to_path_buf();
    let ext = p
        .extension()
        .map(|e| format!("{}.bloom", e.to_string_lossy()))
        .unwrap_or_else(|| "bloom".into());
    p.set_extension(ext);
    p
}
