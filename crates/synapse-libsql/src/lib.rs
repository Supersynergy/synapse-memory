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

pub use LibsqlError as Error;

#[derive(Debug, Default, Clone)]
pub struct QueryResult {
    pub affected: u64,
    pub rows: Vec<Vec<u8>>,
}

#[async_trait]
pub trait Store: Send + Sync + 'static {
    async fn query(&self, sql: &str) -> Result<QueryResult, Error>;
    async fn exec(&self, sql: &str) -> Result<u64, Error>;
}

#[derive(Debug, thiserror::Error)]
pub enum LibsqlError {
    #[error("not enabled — build with --features libsql-backend")]
    NotEnabled,
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("backend: {0}")]
    Backend(String),
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

#[cfg(feature = "libsql-backend")]
pub mod batched;
#[cfg(feature = "libsql-backend")]
pub mod pool_real;
#[cfg(feature = "libsql-backend")]
pub mod turbo;
#[cfg(feature = "libsql-backend")]
pub use batched::BatchedLibsqlStore;
#[cfg(feature = "libsql-backend")]
pub use pool_real::RealPoolStore;
#[cfg(feature = "libsql-backend")]
pub use turbo::TurboLibsqlStore;

/// Minimal naive LibsqlStore — Mutex<Connection>, no pragmas, no batching.
/// Used as baseline for benches.
#[cfg(feature = "libsql-backend")]
pub struct LibsqlStore {
    conn: tokio::sync::Mutex<libsql::Connection>,
}

#[cfg(feature = "libsql-backend")]
impl LibsqlStore {
    pub async fn open_local(path: &str) -> Result<Self, Error> {
        let db = libsql::Builder::new_local(path)
            .build()
            .await
            .map_err(|e| Error::Backend(e.to_string()))?;
        let conn = db.connect().map_err(|e| Error::Backend(e.to_string()))?;
        Ok(Self {
            conn: tokio::sync::Mutex::new(conn),
        })
    }
}

#[cfg(feature = "libsql-backend")]
#[async_trait]
impl Store for LibsqlStore {
    async fn query(&self, sql: &str) -> Result<QueryResult, Error> {
        let conn = self.conn.lock().await;
        let s = sql.trim_start();
        if s.len() >= 6 && s.as_bytes()[..6].eq_ignore_ascii_case(b"SELECT") {
            let mut rows = conn
                .query(sql, ())
                .await
                .map_err(|e| Error::Backend(e.to_string()))?;
            let mut count = 0u64;
            while let Some(_) = rows
                .next()
                .await
                .map_err(|e| Error::Backend(e.to_string()))?
            {
                count += 1;
            }
            Ok(QueryResult {
                affected: count,
                rows: vec![],
            })
        } else {
            let n = conn
                .execute(sql, ())
                .await
                .map_err(|e| Error::Backend(e.to_string()))?;
            Ok(QueryResult {
                affected: n,
                rows: vec![],
            })
        }
    }
    async fn exec(&self, sql: &str) -> Result<u64, Error> {
        let conn = self.conn.lock().await;
        conn.execute(sql, ())
            .await
            .map_err(|e| Error::Backend(e.to_string()))
    }
}

#[cfg(feature = "libsql-backend")]
pub mod libsql_backend {
    //! Real `libsql` async-WAL backend.
    use super::*;
    use libsql::{Builder, Connection};

    pub struct LibsqlBackend {
        conn: Connection,
    }

    impl LibsqlBackend {
        pub async fn open_local(path: &str) -> Result<Self, LibsqlError> {
            let db = Builder::new_local(path)
                .build()
                .await
                .map_err(|e| LibsqlError::Other(e.to_string()))?;
            let conn = db
                .connect()
                .map_err(|e| LibsqlError::Other(e.to_string()))?;
            Ok(Self { conn })
        }
    }

    #[async_trait]
    impl AsyncWalBackend for LibsqlBackend {
        async fn execute(&self, sql: &str) -> Result<u64, LibsqlError> {
            self.conn
                .execute(sql, ())
                .await
                .map_err(|e| LibsqlError::Other(e.to_string()))
        }
        async fn checkpoint(&self) -> Result<(), LibsqlError> {
            self.conn
                .execute("PRAGMA wal_checkpoint(TRUNCATE)", ())
                .await
                .map(|_| ())
                .map_err(|e| LibsqlError::Other(e.to_string()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature = "libsql-backend")]
    #[tokio::test]
    async fn libsql_local_create_and_insert() {
        use crate::libsql_backend::LibsqlBackend;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.db");
        let b = LibsqlBackend::open_local(path.to_str().unwrap())
            .await
            .unwrap();
        b.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, v TEXT)")
            .await
            .unwrap();
        let n = b
            .execute("INSERT INTO t (v) VALUES ('hello')")
            .await
            .unwrap();
        assert_eq!(n, 1);
    }

    #[tokio::test]
    async fn stub_returns_not_enabled() {
        let b = StubBackend;
        assert!(matches!(
            b.execute("SELECT 1").await,
            Err(LibsqlError::NotEnabled)
        ));
    }
}
