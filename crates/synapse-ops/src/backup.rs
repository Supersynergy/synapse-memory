//! Backup tool — sqlite-style .db file copy with WAL checkpoint.
//!
//! Use case: production backup without stopping daemon.

use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub enum BackupTarget {
    LocalFile(PathBuf),
    // Future: S3, GCS, Azure
}

#[derive(Debug, thiserror::Error)]
pub enum BackupError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("source not found: {0}")]
    SourceNotFound(String),
}

pub struct Backup;

impl Backup {
    /// Snapshot a SQLite/libsql database file to target.
    /// Caller must `PRAGMA wal_checkpoint(FULL)` first if active writers.
    pub fn snapshot(src: &Path, target: BackupTarget) -> Result<u64, BackupError> {
        if !src.exists() {
            return Err(BackupError::SourceNotFound(src.display().to_string()));
        }
        match target {
            BackupTarget::LocalFile(dst) => {
                if let Some(parent) = dst.parent()
                    && !parent.as_os_str().is_empty()
                {
                    std::fs::create_dir_all(parent)?;
                }
                let bytes = std::fs::copy(src, &dst)?;
                Ok(bytes)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn snapshot_to_local_file() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("source.db");
        std::fs::write(&src, b"FAKEDBCONTENT").unwrap();
        let dst = dir.path().join("backup/snapshot.db");
        let n = Backup::snapshot(&src, BackupTarget::LocalFile(dst.clone())).unwrap();
        assert_eq!(n, 13);
        assert_eq!(std::fs::read(&dst).unwrap(), b"FAKEDBCONTENT");
    }
    #[test]
    fn missing_source_errors() {
        let r = Backup::snapshot(
            Path::new("/nonexistent/x.db"),
            BackupTarget::LocalFile(PathBuf::from("/tmp/bak.db")),
        );
        assert!(matches!(r, Err(BackupError::SourceNotFound(_))));
    }
}
