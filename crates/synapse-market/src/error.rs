use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("market: {0}")]
    Market(String),
    #[error("pattern: {0}")]
    Other(String),
}

pub type Result<T, E = Error> = std::result::Result<T, E>;
