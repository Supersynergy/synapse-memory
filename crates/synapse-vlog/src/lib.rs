use std::{
    collections::BTreeMap,
    fs::{File, OpenOptions},
    io::{BufWriter, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};

use memmap2::Mmap;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum VlogError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("key not found")]
    NotFound,
    #[error("corrupt entry at offset {0}")]
    Corrupt(u64),
}

pub type Result<T> = std::result::Result<T, VlogError>;
pub type Key = Vec<u8>;

/// Position + length of a value in the value-log.
#[derive(Clone, Copy, Debug)]
pub struct VlogPtr {
    pub offset: u64,
    pub len: u32,
}

/// WiscKey-pattern store: BTreeMap keys + append-only value log.
///
/// Format of vlog.bin records:
///   [4-byte little-endian value_len][value_bytes...]
pub struct VlogStore {
    keys: BTreeMap<Key, VlogPtr>,
    vlog: BufWriter<File>,
    vlog_pos: u64,
    vlog_path: PathBuf,
    /// Cached mmap — rebuilt on flush(). None = dirty (unflushed writes pending).
    mmap: Option<Mmap>,
}

impl VlogStore {
    /// Open (or create) a store rooted at `dir`.
    pub fn open(dir: impl AsRef<Path>) -> Result<Self> {
        let dir = dir.as_ref();
        std::fs::create_dir_all(dir)?;
        let vlog_path = dir.join("vlog.bin");
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .read(true)
            .open(&vlog_path)?;
        let vlog_pos = file.metadata()?.len();
        let mut store = VlogStore {
            keys: BTreeMap::new(),
            vlog: BufWriter::with_capacity(256 * 1024, file),
            vlog_pos,
            vlog_path,
            mmap: None,
        };

        // Replay existing vlog to rebuild in-mem key index.
        if vlog_pos > 0 {
            store.replay()?;
            let f2 = File::open(&store.vlog_path)?;
            store.mmap = Some(unsafe { Mmap::map(&f2)? });
        }

        Ok(store)
    }

    fn replay(&mut self) -> Result<()> {
        // Re-open for sequential read.
        use std::io::Read;
        let mut f = File::open(&self.vlog_path)?;
        let mut offset = 0u64;
        loop {
            let mut len_buf = [0u8; 4];
            match f.read_exact(&mut len_buf) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
                Err(e) => return Err(e.into()),
            }
            let klen = u32::from_le_bytes(len_buf) as usize;
            let mut key_buf = vec![0u8; klen];
            f.read_exact(&mut key_buf)?;
            let mut vlen_buf = [0u8; 4];
            f.read_exact(&mut vlen_buf)?;
            let vlen = u32::from_le_bytes(vlen_buf);
            let val_offset = offset + 4 + klen as u64 + 4;
            self.keys.insert(
                key_buf,
                VlogPtr {
                    offset: val_offset,
                    len: vlen,
                },
            );
            // Seek past value.
            f.seek(SeekFrom::Current(vlen as i64))?;
            offset = val_offset + vlen as u64;
        }
        Ok(())
    }

    /// Append value to vlog, store pointer in BTreeMap.
    ///
    /// Record layout: [4 klen][key][4 vlen][value]
    pub fn put(&mut self, k: Key, v: &[u8]) -> Result<()> {
        let klen = k.len() as u32;
        let vlen = v.len() as u32;
        self.vlog.write_all(&klen.to_le_bytes())?;
        self.vlog.write_all(&k)?;
        self.vlog.write_all(&vlen.to_le_bytes())?;
        let val_offset = self.vlog_pos + 4 + k.len() as u64 + 4;
        self.vlog.write_all(v)?;
        self.vlog_pos = val_offset + v.len() as u64;
        self.keys.insert(k, VlogPtr { offset: val_offset, len: vlen });
        self.mmap = None; // invalidate cache on write
        Ok(())
    }

    /// Lookup key → mmap-read value slice.
    /// Call flush() first to ensure all writes are visible.
    pub fn get(&self, k: &Key) -> Result<Option<Vec<u8>>> {
        let ptr = match self.keys.get(k) {
            Some(p) => *p,
            None => return Ok(None),
        };
        let mmap = match &self.mmap {
            Some(m) => m,
            None => {
                // mmap not cached — do a fresh one-shot open (slow path, use flush() to cache)
                let f = File::open(&self.vlog_path)?;
                let m = unsafe { Mmap::map(&f)? };
                let start = ptr.offset as usize;
                let end = start + ptr.len as usize;
                if end > m.len() {
                    return Err(VlogError::Corrupt(ptr.offset));
                }
                return Ok(Some(m[start..end].to_vec()));
            }
        };
        let start = ptr.offset as usize;
        let end = start + ptr.len as usize;
        if end > mmap.len() {
            return Err(VlogError::Corrupt(ptr.offset));
        }
        Ok(Some(mmap[start..end].to_vec()))
    }

    /// fsync vlog + rebuild mmap cache.
    pub fn flush(&mut self) -> Result<()> {
        self.vlog.flush()?;
        self.vlog.get_ref().sync_all()?;
        // Rebuild mmap so subsequent get() calls hit the cache.
        let f = File::open(&self.vlog_path)?;
        if f.metadata()?.len() > 0 {
            self.mmap = Some(unsafe { Mmap::map(&f)? });
        }
        Ok(())
    }

    /// Number of keys in the in-mem index.
    pub fn len(&self) -> usize {
        self.keys.len()
    }

    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn put_get_roundtrip() {
        let dir = tempdir().unwrap();
        let mut store = VlogStore::open(dir.path()).unwrap();
        store.put(b"hello".to_vec(), b"world").unwrap();
        store.put(b"foo".to_vec(), b"bar").unwrap();
        store.flush().unwrap();
        assert_eq!(store.get(&b"hello".to_vec()).unwrap(), Some(b"world".to_vec()));
        assert_eq!(store.get(&b"foo".to_vec()).unwrap(), Some(b"bar".to_vec()));
        assert_eq!(store.get(&b"missing".to_vec()).unwrap(), None);
    }

    #[test]
    fn overwrite_returns_latest() {
        let dir = tempdir().unwrap();
        let mut store = VlogStore::open(dir.path()).unwrap();
        store.put(b"k".to_vec(), b"v1").unwrap();
        store.put(b"k".to_vec(), b"v2").unwrap();
        store.flush().unwrap();
        assert_eq!(store.get(&b"k".to_vec()).unwrap(), Some(b"v2".to_vec()));
    }

    #[test]
    fn reopen_replay() {
        let dir = tempdir().unwrap();
        {
            let mut store = VlogStore::open(dir.path()).unwrap();
            store.put(b"persistent".to_vec(), b"data").unwrap();
            store.flush().unwrap();
        }
        let mut store2 = VlogStore::open(dir.path()).unwrap();
        store2.flush().unwrap();
        assert_eq!(
            store2.get(&b"persistent".to_vec()).unwrap(),
            Some(b"data".to_vec())
        );
    }

    #[test]
    fn large_value() {
        let dir = tempdir().unwrap();
        let mut store = VlogStore::open(dir.path()).unwrap();
        let big = vec![0xabu8; 1_000_000];
        store.put(b"big".to_vec(), &big).unwrap();
        store.flush().unwrap();
        assert_eq!(store.get(&b"big".to_vec()).unwrap().unwrap().len(), 1_000_000);
    }
}
