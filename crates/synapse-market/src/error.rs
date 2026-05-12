use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("market: {0}")]
    Market(String),
}

pub type Result<T, E = Error> = std::result::Result<T, E>;
