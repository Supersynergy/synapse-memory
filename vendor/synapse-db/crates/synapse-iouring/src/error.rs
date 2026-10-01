use thiserror::Error;

#[derive(Debug, Error)]
pub enum IoUringError {
    #[error("io_uring not available on this platform (Linux only)")]
    UnsupportedPlatform,

    #[error("io_uring setup failed: {0}")]
    SetupFailed(String),

    #[error("WAL append failed: {0}")]
    AppendFailed(String),

    #[error("read failed: {0}")]
    ReadFailed(String),

    #[error("compaction error: {0}")]
    Compaction(String),

    #[error(transparent)]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, IoUringError>;
