use thiserror::Error;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Error)]
pub enum Error {
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("dim mismatch: expected {expected}, got {got}")]
    DimMismatch { expected: usize, got: usize },
    #[error("not found: {0}")]
    NotFound(String),
    #[error("other: {0}")]
    Other(String),
}
