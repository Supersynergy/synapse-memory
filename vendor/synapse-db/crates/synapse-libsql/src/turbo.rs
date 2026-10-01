//! TurboLibsqlStore — extreme single-row INSERT optimization.
//!
//! Combines all known SQLite write-speedup pragmas + lock-amortized commits.
//!
//! Applied techniques (mined research/naive_*.md):
//! - `synchronous=OFF` — no fsync (durability tradeoff: tx may be lost on crash)
//! - `journal_mode=WAL` — concurrent readers + single writer
//! - `mmap_size=268435456` (256 MB) — page-cache via OS, fewer syscalls
//! - `temp_store=MEMORY` — sort/group temp in RAM
//! - `cache_size=-262144` (256 MB) — large page cache
//! - `wal_autocheckpoint=10000` — defer checkpoint cost
//! - `locking_mode=EXCLUSIVE` — single-writer fast path (skip lock-byte page)
//! - `page_size=8192` — match common SSD/APFS page boundaries
//!
//! ⚠️ TURBO MODE TRADEOFFS:
//! - `synchronous=OFF`: tx committed but not flushed — last N ms may be lost on power loss
//! - Use for: caches, comments, analytics, transient WP options
//! - DO NOT use for: financial txns, authoritative state

use crate::{Error, QueryResult, Store};
use async_trait::async_trait;
use libsql::{Builder, Connection};
use tokio::sync::Mutex;

pub struct TurboLibsqlStore {
    conn: Mutex<Connection>,
}

impl TurboLibsqlStore {
    pub async fn open_local(path: &str) -> Result<Self, Error> {
        let db = Builder::new_local(path)
            .build()
            .await
            .map_err(|e| Error::Backend(e.to_string()))?;
        let conn = db.connect().map_err(|e| Error::Backend(e.to_string()))?;
        // Apply turbo pragmas — order matters: page_size BEFORE any table create
        let pragmas = [
            "PRAGMA page_size=8192",
            "PRAGMA journal_mode=WAL",
            "PRAGMA synchronous=OFF",
            "PRAGMA temp_store=MEMORY",
            "PRAGMA mmap_size=268435456",
            "PRAGMA cache_size=-262144",
            "PRAGMA wal_autocheckpoint=10000",
            "PRAGMA locking_mode=EXCLUSIVE",
            "PRAGMA busy_timeout=5000",
        ];
        for p in pragmas {
            let _ = conn.execute(p, ()).await;
        }
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    pub async fn checkpoint(&self) -> Result<(), Error> {
        let conn = self.conn.lock().await;
        conn.execute("PRAGMA wal_checkpoint(TRUNCATE)", ())
            .await
            .map(|_| ())
            .map_err(|e| Error::Backend(e.to_string()))
    }
}

#[async_trait]
impl Store for TurboLibsqlStore {
    async fn query(&self, sql: &str) -> Result<QueryResult, Error> {
        let conn = self.conn.lock().await;
        let s = sql.trim_start();
        if s.len() >= 6 && s.as_bytes()[..6].eq_ignore_ascii_case(b"SELECT") {
            let mut rows = conn
                .query(sql, ())
                .await
                .map_err(|e| Error::Backend(e.to_string()))?;
            let mut count = 0u64;
            while rows
                .next()
                .await
                .map_err(|e| Error::Backend(e.to_string()))?
                .is_some()
            {
                count += 1;
            }
            Ok(QueryResult {
                affected: count,
                rows: vec![],
            })
        } else {
            let affected = conn
                .execute(sql, ())
                .await
                .map_err(|e| Error::Backend(e.to_string()))?;
            Ok(QueryResult {
                affected,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn turbo_inserts_work() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("turbo.db");
        let s = TurboLibsqlStore::open_local(path.to_str().unwrap())
            .await
            .unwrap();
        s.exec("CREATE TABLE t (id INTEGER PRIMARY KEY AUTOINCREMENT, v TEXT)")
            .await
            .unwrap();
        for i in 0..10 {
            s.exec(&format!("INSERT INTO t (v) VALUES ('val{i}')"))
                .await
                .unwrap();
        }
        let r = s.query("SELECT * FROM t").await.unwrap();
        assert_eq!(r.affected, 10);
    }
}
