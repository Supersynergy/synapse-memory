/// mmap manager — open file, get page regions, write regions.
use memmap2::{MmapMut, MmapOptions};
use std::fs::{File, OpenOptions};
use std::path::Path;

use super::page::PAGE_SIZE;

/// Mmap-backed page file. Append-only: grows in PAGE_SIZE chunks.
pub struct MmapFile {
    file: File,
    path: std::path::PathBuf,
}

impl MmapFile {
    pub fn open<P: AsRef<Path>>(path: P) -> std::io::Result<Self> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path.as_ref())?;
        Ok(Self { file, path: path.as_ref().to_path_buf() })
    }

    /// Current number of pages stored.
    pub fn page_count(&self) -> std::io::Result<usize> {
        let len = self.file.metadata()?.len() as usize;
        Ok(len / PAGE_SIZE)
    }

    /// Append a page. Returns the page index (0-based).
    pub fn append_page(&self, page: &[u8]) -> std::io::Result<usize> {
        assert_eq!(page.len(), PAGE_SIZE);
        let current_len = self.file.metadata()?.len() as usize;
        let new_len = current_len + PAGE_SIZE;
        self.file.set_len(new_len as u64)?;

        let mut mmap = unsafe { MmapMut::map_mut(&self.file)? };
        mmap[current_len..new_len].copy_from_slice(page);
        mmap.flush()?;
        Ok(current_len / PAGE_SIZE)
    }

    /// Read page at index. Returns PAGE_SIZE bytes.
    pub fn read_page(&self, page_idx: usize) -> std::io::Result<Vec<u8>> {
        let offset = page_idx * PAGE_SIZE;
        let file_len = self.file.metadata()?.len() as usize;
        if offset + PAGE_SIZE > file_len {
            return Err(std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "page out of range"));
        }
        let mmap = unsafe { MmapOptions::new().offset(offset as u64).len(PAGE_SIZE).map(&self.file)? };
        Ok(mmap.to_vec())
    }

    /// fsync.
    pub fn sync(&self) -> std::io::Result<()> {
        self.file.sync_all()
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    #[test]
    fn append_and_read() {
        let f = NamedTempFile::new().unwrap();
        let mf = MmapFile::open(f.path()).unwrap();
        let mut page = vec![0u8; PAGE_SIZE];
        page[0] = 42;
        page[PAGE_SIZE - 1] = 99;
        let idx = mf.append_page(&page).unwrap();
        assert_eq!(idx, 0);
        let read = mf.read_page(0).unwrap();
        assert_eq!(read[0], 42);
        assert_eq!(read[PAGE_SIZE - 1], 99);
    }

    #[test]
    fn multi_page() {
        let f = NamedTempFile::new().unwrap();
        let mf = MmapFile::open(f.path()).unwrap();
        for i in 0..3u8 {
            let mut page = vec![0u8; PAGE_SIZE];
            page[0] = i;
            mf.append_page(&page).unwrap();
        }
        assert_eq!(mf.page_count().unwrap(), 3);
        for i in 0..3u8 {
            let p = mf.read_page(i as usize).unwrap();
            assert_eq!(p[0], i);
        }
    }
}
