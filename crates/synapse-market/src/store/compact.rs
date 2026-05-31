/// Cold-tier compaction: zstd-19 compressed pages stored in a `.csm` file.
///
/// Format:
///   [0..64]   CsmHeader (magic, page_count, reserved)
///   [64..)    directory: page_count × CsmEntry { u32 offset, u32 zstd_len, i64 ts_min, i64 ts_max }
///   then:     concat of zstd-compressed page blobs
///
/// After directory: each entry's offset is relative to start-of-file (absolute).
use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use anyhow::{Result, bail};
use lru::LruCache;
use std::num::NonZeroUsize;
use zstd::stream::{decode_all, encode_all};

use crate::store::page::{Bar, HEADER_SIZE, PAGE_SIZE, decode_page};

const CSM_MAGIC: u64 = 0x534D_5843_4F4C_4400; // "SMXCOLD\0"
const CSM_HEADER_SIZE: usize = 64;
const CSM_ENTRY_SIZE: usize = 24; // u32 offset + u32 zstd_len + i64 ts_min + i64 ts_max
const ZSTD_LEVEL: i32 = 19;
pub const COLD_CACHE_CAPACITY: usize = 50;

#[repr(C)]
struct CsmFileHeader {
    magic: u64,
    page_count: u32,
    _pad: [u8; 52],
}

impl CsmFileHeader {
    fn to_bytes(&self) -> [u8; CSM_HEADER_SIZE] {
        let mut b = [0u8; CSM_HEADER_SIZE];
        b[0..8].copy_from_slice(&self.magic.to_le_bytes());
        b[8..12].copy_from_slice(&self.page_count.to_le_bytes());
        b
    }

    fn from_bytes(b: &[u8; CSM_HEADER_SIZE]) -> Result<Self> {
        let magic = u64::from_le_bytes(b[0..8].try_into().unwrap());
        if magic != CSM_MAGIC {
            bail!("invalid csm magic");
        }
        let page_count = u32::from_le_bytes(b[8..12].try_into().unwrap());
        Ok(Self {
            magic,
            page_count,
            _pad: [0u8; 52],
        })
    }
}

#[derive(Debug, Clone, Copy)]
pub struct CsmEntry {
    pub offset: u32,
    pub zstd_len: u32,
    pub ts_min: i64,
    pub ts_max: i64,
}

impl CsmEntry {
    fn to_bytes(self) -> [u8; CSM_ENTRY_SIZE] {
        let mut b = [0u8; CSM_ENTRY_SIZE];
        b[0..4].copy_from_slice(&self.offset.to_le_bytes());
        b[4..8].copy_from_slice(&self.zstd_len.to_le_bytes());
        b[8..16].copy_from_slice(&self.ts_min.to_le_bytes());
        b[16..24].copy_from_slice(&self.ts_max.to_le_bytes());
        b
    }

    fn from_bytes(b: &[u8; CSM_ENTRY_SIZE]) -> Self {
        let offset = u32::from_le_bytes(b[0..4].try_into().unwrap());
        let zstd_len = u32::from_le_bytes(b[4..8].try_into().unwrap());
        let ts_min = i64::from_le_bytes(b[8..16].try_into().unwrap());
        let ts_max = i64::from_le_bytes(b[16..24].try_into().unwrap());
        Self {
            offset,
            zstd_len,
            ts_min,
            ts_max,
        }
    }
}

pub struct ColdTier {
    pub base_path: PathBuf,
    entries: Vec<CsmEntry>,
    cache: LruCache<u32, Vec<Bar>>,
}

impl ColdTier {
    /// Create a new cold-tier file from hot pages (Vec<Vec<u8>> raw page buffers).
    /// `hot_pages` — each element is a PAGE_SIZE raw page.
    pub fn create(base_path: &Path, hot_pages: &[Vec<u8>]) -> Result<()> {
        let n = hot_pages.len();
        // Compress all pages first to compute offsets
        let mut compressed: Vec<Vec<u8>> = Vec::with_capacity(n);
        for raw in hot_pages {
            let c = encode_all(raw.as_slice(), ZSTD_LEVEL)?;
            compressed.push(c);
        }

        // Directory starts at byte CSM_HEADER_SIZE
        // Data starts after directory
        let dir_size = n * CSM_ENTRY_SIZE;
        let data_start = CSM_HEADER_SIZE + dir_size;

        let mut entries: Vec<CsmEntry> = Vec::with_capacity(n);
        let mut cur_offset = data_start as u32;
        for (i, c) in compressed.iter().enumerate() {
            let raw = &hot_pages[i];
            // ts_min, ts_max from page header
            let (ts_min, ts_max) = page_ts_range(raw);
            entries.push(CsmEntry {
                offset: cur_offset,
                zstd_len: c.len() as u32,
                ts_min,
                ts_max,
            });
            cur_offset += c.len() as u32;
        }

        let hdr = CsmFileHeader {
            magic: CSM_MAGIC,
            page_count: n as u32,
            _pad: [0u8; 52],
        };
        let mut f = File::create(base_path)?;
        f.write_all(&hdr.to_bytes())?;
        for e in &entries {
            f.write_all(&e.to_bytes())?;
        }
        for c in &compressed {
            f.write_all(c)?;
        }
        f.flush()?;
        Ok(())
    }

    /// Open existing cold-tier file.
    pub fn open(base_path: &Path) -> Result<Self> {
        let mut f = File::open(base_path)?;
        let mut hdr_buf = [0u8; CSM_HEADER_SIZE];
        f.read_exact(&mut hdr_buf)?;
        let hdr = CsmFileHeader::from_bytes(&hdr_buf)?;
        let n = hdr.page_count as usize;
        let mut entries = Vec::with_capacity(n);
        for _ in 0..n {
            let mut eb = [0u8; CSM_ENTRY_SIZE];
            f.read_exact(&mut eb)?;
            entries.push(CsmEntry::from_bytes(&eb));
        }
        Ok(Self {
            base_path: base_path.to_path_buf(),
            entries,
            cache: LruCache::new(NonZeroUsize::new(COLD_CACHE_CAPACITY).unwrap()),
        })
    }

    /// Read page by index, returning decoded bars. Uses LRU decompress-cache.
    pub fn read_page(&mut self, idx: u32) -> Result<Vec<Bar>> {
        if let Some(cached) = self.cache.get(&idx) {
            return Ok(cached.clone());
        }
        let entry = self
            .entries
            .get(idx as usize)
            .ok_or_else(|| anyhow::anyhow!("cold page idx {} out of range", idx))?;
        let raw_page = self.decompress_entry(entry)?;
        let (_hdr, bars) = decode_page(&raw_page);
        self.cache.put(idx, bars.clone());
        Ok(bars)
    }

    fn decompress_entry(&self, entry: &CsmEntry) -> Result<Vec<u8>> {
        let mut f = File::open(&self.base_path)?;
        use std::io::Seek;
        f.seek(std::io::SeekFrom::Start(entry.offset as u64))?;
        let mut buf = vec![0u8; entry.zstd_len as usize];
        f.read_exact(&mut buf)?;
        let decompressed = decode_all(buf.as_slice())?;
        if decompressed.len() < HEADER_SIZE {
            bail!("decompressed page too small");
        }
        Ok(decompressed)
    }

    pub fn entries(&self) -> &[CsmEntry] {
        &self.entries
    }

    pub fn page_count(&self) -> usize {
        self.entries.len()
    }

    /// Cache stats: (hits, misses) — not tracked separately; instead expose cache len.
    pub fn cache_len(&self) -> usize {
        self.cache.len()
    }
}

fn page_ts_range(raw: &[u8]) -> (i64, i64) {
    use crate::store::page::PageHeader;
    let hdr_bytes: [u8; HEADER_SIZE] = raw[..HEADER_SIZE].try_into().unwrap();
    let hdr = PageHeader::from_bytes(&hdr_bytes);
    (hdr.ts_min, hdr.ts_max)
}

/// Stats about a cold-tier file without opening full bars.
pub struct ColdStats {
    pub page_count: usize,
    pub total_compressed_bytes: u64,
    pub hot_bytes: u64, // page_count × PAGE_SIZE
}

impl ColdStats {
    pub fn compression_ratio(&self) -> f64 {
        if self.total_compressed_bytes == 0 {
            return 0.0;
        }
        self.hot_bytes as f64 / self.total_compressed_bytes as f64
    }
}

pub fn cold_stats(base_path: &Path) -> Result<ColdStats> {
    let cold = ColdTier::open(base_path)?;
    let total_compressed_bytes: u64 = cold.entries().iter().map(|e| e.zstd_len as u64).sum();
    let hot_bytes = cold.page_count() as u64 * PAGE_SIZE as u64;
    Ok(ColdStats {
        page_count: cold.page_count(),
        total_compressed_bytes,
        hot_bytes,
    })
}
