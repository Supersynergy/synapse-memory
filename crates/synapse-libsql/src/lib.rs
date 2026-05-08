//! synapse-libsql — async-WAL SQLite-compat backend.
//!
//! Cluster-B of synapse-gap-sprint. Closes the "1 writer at a time" gap
//! that SQLite WAL inherits. libSQL (Turso fork) provides async-WAL with
//! `BEGIN CONCURRENT`-equivalent multi-writer semantics.
//!
//! Reference modules (cloned to ../../synapse-gap-sprint/repos/libsql/):
//! - `libsql-sys/src/wal/sqlite3_wal.rs` — WAL hook impl
//! - `libsql-replication/src/injector/sqlite_injector/` — replication injector
//! - `libsql-wal/src/wal.rs` — async WAL primitives
//!
//! **STATUS**: scaffold. Full Connection wiring TODO.
//! Risk: apsw direct-bypass paths in `synapse-core::synx` need dual-backend.

use async_trait::async_trait;

#[derive(Debug, thiserror::Error)]
pub enum LibsqlError {
    #[error("not enabled — build with --features libsql-backend")]
    NotEnabled,
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("backend: {0}")]
    Other(String),
}

#[async_trait]
pub trait AsyncWalBackend: Send + Sync {
    async fn execute(&self, sql: &str) -> Result<u64, LibsqlError>;
    async fn checkpoint(&self) -> Result<(), LibsqlError>;
}

pub struct StubBackend;

#[async_trait]
impl AsyncWalBackend for StubBackend {
    async fn execute(&self, _sql: &str) -> Result<u64, LibsqlError> {
        Err(LibsqlError::NotEnabled)
    }
    async fn checkpoint(&self) -> Result<(), LibsqlError> {
        Err(LibsqlError::NotEnabled)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn stub_returns_not_enabled() {
        let b = StubBackend;
        assert!(matches!(b.execute("SELECT 1").await, Err(LibsqlError::NotEnabled)));
    }
}
