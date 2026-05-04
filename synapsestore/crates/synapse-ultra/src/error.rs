use thiserror::Error;

#[derive(Debug, Error)]
pub enum UltraError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("anyhow: {0}")]
    Anyhow(#[from] anyhow::Error),
    #[error("embed: {0}")]
    Embed(String),
    #[error("snapshot corrupt: {0}")]
    SnapshotCorrupt(String),
}

pub type Result<T, E = UltraError> = std::result::Result<T, E>;
