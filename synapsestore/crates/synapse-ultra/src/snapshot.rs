use std::fs::File;
use std::io::Write;
use std::path::Path;
use std::{fs, mem};

use memmap2::Mmap;
use ndarray::Array2;
use rusqlite::{Connection, OpenFlags, ffi::sqlite3_auto_extension};

use crate::error::{Result, UltraError};

type SqliteAutoExtensionFn = unsafe extern "C" fn(
    *mut rusqlite::ffi::sqlite3,
    *mut *mut i8,
    *const rusqlite::ffi::sqlite3_api_routines,
) -> i32;

pub const MAGIC: u64 = 0x534E55_4C543200; // "SNULT2\0"
pub const HEADER_BYTES: usize = 64; // magic(8)+version(2)+dim(2)+n_rows(4)+mtime(8)+flags(4)+hash(32)+pad(4)
pub const EMBED_DIM: usize = 384;
pub const SNAPSHOT_VERSION: u16 = 2;

/// flags bit: 0 = f32, 1 = f16
pub const FLAG_F16: u32 = 0x01;

#[derive(Debug, Clone)]
pub struct SnapshotHeader {
    pub magic: u64,
    pub version: u16,
    pub dim: u16,
    pub n_rows: u32,
    pub mtime: u64,
    pub flags: u32,
    pub hash: [u8; 32],
}

impl SnapshotHeader {
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(HEADER_BYTES);
        buf.extend_from_slice(&self.magic.to_le_bytes());
        buf.extend_from_slice(&self.version.to_le_bytes());
        buf.extend_from_slice(&self.dim.to_le_bytes());
        buf.extend_from_slice(&self.n_rows.to_le_bytes());
        buf.extend_from_slice(&self.mtime.to_le_bytes());
        buf.extend_from_slice(&self.flags.to_le_bytes());
        buf.extend_from_slice(&self.hash);
        buf.extend_from_slice(&[0u8; 4]); // pad
        buf
    }

    pub fn from_bytes(b: &[u8]) -> Option<Self> {
        if b.len() < HEADER_BYTES {
            return None;
        }
        Some(Self {
            magic: u64::from_le_bytes(b[0..8].try_into().ok()?),
            version: u16::from_le_bytes(b[8..10].try_into().ok()?),
            dim: u16::from_le_bytes(b[10..12].try_into().ok()?),
            n_rows: u32::from_le_bytes(b[12..16].try_into().ok()?),
            mtime: u64::from_le_bytes(b[16..24].try_into().ok()?),
            flags: u32::from_le_bytes(b[24..28].try_into().ok()?),
            hash: b[28..60].try_into().ok()?,
        })
    }

    pub fn is_f16(&self) -> bool {
        self.flags & FLAG_F16 != 0
    }
}

/// f16 as u16 LE storage
#[inline]
fn f32_to_f16_bits(x: f32) -> u16 {
    half::f16::from_f32(x).to_bits()
}

#[inline]
fn f16_bits_to_f32(b: u16) -> f32 {
    half::f16::from_bits(b).to_f32()
}

pub struct Snapshot {
    pub ids: Vec<i64>,
    /// Pre-normalized f32 rows, shape (n, 384). Used for T1-strict.
    pub matrix_f32: Array2<f32>,
    /// Pre-normalized f16 rows as u16 LE, len = n*384. Used for T1'.
    pub matrix_f16: Vec<u16>,
}

pub fn brain_db_mtime(brain_path: &Path) -> u64 {
    fs::metadata(brain_path)
        .and_then(|m| m.modified())
        .map(|t| {
            t.duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs()
        })
        .unwrap_or(0)
}

pub fn load_mmap(snap_path: &Path, brain_mtime: u64) -> Option<Snapshot> {
    let file = File::open(snap_path).ok()?;
    let mmap = unsafe { Mmap::map(&file).ok()? };
    let header = SnapshotHeader::from_bytes(&mmap)?;
    if header.magic != MAGIC {
        tracing::debug!("snapshot magic mismatch");
        return None;
    }
    if header.version != SNAPSHOT_VERSION {
        tracing::info!(
            "snapshot version {} != {}, rebuilding",
            header.version,
            SNAPSHOT_VERSION
        );
        return None;
    }
    if header.mtime < brain_mtime {
        tracing::info!("snapshot stale, rebuilding");
        return None;
    }
    let n = header.n_rows as usize;
    let dim = header.dim as usize;
    let bytes_per_elem: usize = if header.is_f16() { 2 } else { 4 };
    let data_len = n * dim * bytes_per_elem;
    if mmap.len() < HEADER_BYTES + n * 8 + data_len {
        return None;
    }
    let ids: Vec<i64> = (0..n)
        .map(|i| {
            let s = HEADER_BYTES + i * 8;
            i64::from_le_bytes(mmap[s..s + 8].try_into().unwrap())
        })
        .collect();
    let data_start = HEADER_BYTES + n * 8;
    let data = &mmap[data_start..data_start + data_len];

    let (matrix_f32, matrix_f16) = if header.is_f16() {
        // decode f16 → f32, also keep f16 u16 array
        let f16u: Vec<u16> = data
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes(c.try_into().unwrap()))
            .collect();
        let f32v: Vec<f32> = f16u.iter().map(|&b| f16_bits_to_f32(b)).collect();
        let m = Array2::from_shape_vec((n, dim), f32v).ok()?;
        (m, f16u)
    } else {
        let f32v: Vec<f32> = data
            .chunks_exact(4)
            .map(|c| f32::from_le_bytes(c.try_into().unwrap()))
            .collect();
        let f16u: Vec<u16> = f32v.iter().map(|&x| f32_to_f16_bits(x)).collect();
        let m = Array2::from_shape_vec((n, dim), f32v).ok()?;
        (m, f16u)
    };

    Some(Snapshot {
        ids,
        matrix_f32,
        matrix_f16,
    })
}

pub fn rebuild(brain_path: &Path, snap_path: &Path) -> Result<Snapshot> {
    tracing::info!("rebuilding snapshot from {:?}", brain_path);

    unsafe {
        sqlite3_auto_extension(Some(mem::transmute::<*const (), SqliteAutoExtensionFn>(
            sqlite_vec::sqlite3_vec_init as *const (),
        )));
    }

    let conn = Connection::open_with_flags(
        brain_path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;

    let mut stmt = conn.prepare("SELECT id, embedding FROM docs_vec ORDER BY id ASC")?;
    let mut ids: Vec<i64> = Vec::new();
    let mut all_f32: Vec<f32> = Vec::new();

    let rows = stmt.query_map([], |row| {
        let id: i64 = row.get(0)?;
        let blob: Vec<u8> = row.get(1)?;
        Ok((id, blob))
    })?;

    for row in rows {
        let (id, blob) = row?;
        if blob.len() != EMBED_DIM * 4 {
            tracing::warn!("id={} unexpected blob len {}, skipping", id, blob.len());
            continue;
        }
        ids.push(id);
        all_f32.extend(
            blob.chunks_exact(4)
                .map(|c| f32::from_le_bytes(c.try_into().unwrap())),
        );
    }

    let n = ids.len();
    tracing::info!("loaded {} embeddings", n);

    let mut matrix_f32 = Array2::from_shape_vec((n, EMBED_DIM), all_f32)
        .map_err(|e| UltraError::SnapshotCorrupt(e.to_string()))?;

    normalize_rows(&mut matrix_f32);

    // Build f16 version
    let matrix_f16: Vec<u16> = matrix_f32.iter().map(|&x| f32_to_f16_bits(x)).collect();

    let mtime = brain_db_mtime(brain_path);
    let raw_f16_bytes: &[u8] = u16_as_u8(&matrix_f16);
    let hash = *blake3::hash(raw_f16_bytes).as_bytes();

    let header = SnapshotHeader {
        magic: MAGIC,
        version: SNAPSHOT_VERSION,
        dim: EMBED_DIM as u16,
        n_rows: n as u32,
        mtime,
        flags: FLAG_F16,
        hash,
    };

    let mut f = File::create(snap_path)?;
    f.write_all(&header.to_bytes())?;
    for &id in &ids {
        f.write_all(&id.to_le_bytes())?;
    }
    f.write_all(raw_f16_bytes)?;
    f.flush()?;

    tracing::info!("snapshot written ({} rows, f16)", n);
    Ok(Snapshot {
        ids,
        matrix_f32,
        matrix_f16,
    })
}

pub fn normalize_rows(matrix: &mut Array2<f32>) {
    for mut row in matrix.rows_mut() {
        let norm = row.iter().map(|x| x * x).sum::<f32>().sqrt();
        if norm > 1e-9 {
            row.iter_mut().for_each(|x| *x /= norm);
        }
    }
}

pub fn normalize_vec(v: &mut [f32]) {
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 1e-9 {
        v.iter_mut().for_each(|x| *x /= norm);
    }
}

fn u16_as_u8(s: &[u16]) -> &[u8] {
    unsafe { std::slice::from_raw_parts(s.as_ptr() as *const u8, s.len() * 2) }
}
