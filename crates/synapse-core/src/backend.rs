use async_trait::async_trait;

#[async_trait]
pub trait SqliteBackend: Send + Sync {
    async fn execute(&self, sql: &str) -> Result<u64, String>;
    async fn query_one_row(&self, sql: &str) -> Result<Option<Vec<String>>, String>;
}

// ── Rusqlite backend (default) ────────────────────────────────────────────────

#[cfg(feature = "backend-rusqlite")]
pub struct RusqliteBackend {
    path: String,
}

#[cfg(feature = "backend-rusqlite")]
impl RusqliteBackend {
    pub fn new(path: impl Into<String>) -> Self {
        Self { path: path.into() }
    }
}

#[cfg(feature = "backend-rusqlite")]
#[async_trait]
impl SqliteBackend for RusqliteBackend {
    async fn execute(&self, sql: &str) -> Result<u64, String> {
        let conn = rusqlite::Connection::open(&self.path).map_err(|e| e.to_string())?;
        let rows = conn.execute(sql, []).map_err(|e| e.to_string())?;
        Ok(rows as u64)
    }

    async fn query_one_row(&self, sql: &str) -> Result<Option<Vec<String>>, String> {
        let conn = rusqlite::Connection::open(&self.path).map_err(|e| e.to_string())?;
        let mut stmt = conn.prepare(sql).map_err(|e| e.to_string())?;
        let col_count = stmt.column_count();
        let mut rows = stmt.query([]).map_err(|e| e.to_string())?;
        match rows.next().map_err(|e| e.to_string())? {
            None => Ok(None),
            Some(row) => {
                let vals = (0..col_count)
                    .map(|i| row.get::<_, rusqlite::types::Value>(i))
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|e| e.to_string())?;
                let strs = vals
                    .into_iter()
                    .map(|v| match v {
                        rusqlite::types::Value::Text(s) => s,
                        rusqlite::types::Value::Integer(n) => n.to_string(),
                        rusqlite::types::Value::Real(f) => f.to_string(),
                        rusqlite::types::Value::Blob(_) => "<blob>".into(),
                        rusqlite::types::Value::Null => "NULL".into(),
                    })
                    .collect();
                Ok(Some(strs))
            }
        }
    }
}

// ── libSQL backend (opt-in) ───────────────────────────────────────────────────

#[cfg(feature = "backend-libsql")]
pub struct LibsqlBackend {
    path: String,
}

#[cfg(feature = "backend-libsql")]
impl LibsqlBackend {
    pub fn new(path: impl Into<String>) -> Self {
        Self { path: path.into() }
    }

    async fn conn(&self) -> Result<libsql::Connection, String> {
        // Register extensions BEFORE opening the connection (ABI-verified 2026-04-24).
        unsafe {
            libsql::ffi::sqlite3_auto_extension(Some(std::mem::transmute::<
                *const (),
                unsafe extern "C" fn(
                    *mut libsql::ffi::sqlite3,
                    *mut *const i8,
                    *const libsql::ffi::sqlite3_api_routines,
                ) -> i32,
            >(
                sqlite_vec::sqlite3_vec_init as *const ()
            )));
        }
        let db = libsql::Builder::new_local(&self.path)
            .build()
            .await
            .map_err(|e| e.to_string())?;
        db.connect().map_err(|e| e.to_string())
    }
}

#[cfg(feature = "backend-libsql")]
#[async_trait]
impl SqliteBackend for LibsqlBackend {
    async fn execute(&self, sql: &str) -> Result<u64, String> {
        let conn = self.conn().await?;
        let rows = conn.execute(sql, ()).await.map_err(|e| e.to_string())?;
        Ok(rows)
    }

    async fn query_one_row(&self, sql: &str) -> Result<Option<Vec<String>>, String> {
        let conn = self.conn().await?;
        let mut rows = conn.query(sql, ()).await.map_err(|e| e.to_string())?;
        match rows.next().await.map_err(|e| e.to_string())? {
            None => Ok(None),
            Some(row) => {
                let col_count = row.column_count();
                let strs = (0..col_count)
                    .map(|i| {
                        row.get_value(i)
                            .map(|v| match v {
                                libsql::Value::Text(s) => s,
                                libsql::Value::Integer(n) => n.to_string(),
                                libsql::Value::Real(f) => f.to_string(),
                                libsql::Value::Blob(_) => "<blob>".into(),
                                libsql::Value::Null => "NULL".into(),
                            })
                            .map_err(|e| e.to_string())
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(Some(strs))
            }
        }
    }
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature = "backend-rusqlite")]
    #[tokio::test]
    async fn rusqlite_select_one() {
        let dir = tempfile::tempdir().unwrap();
        let backend = RusqliteBackend::new(dir.path().join("t.db").to_str().unwrap());
        let row = backend.query_one_row("SELECT 1").await.unwrap().unwrap();
        assert_eq!(row[0], "1");
    }

    // Run in isolation: `cargo test -p synapse-core --no-default-features
    // --features backend-libsql libsql_select_one` (libsql+rusqlite share one SQLite
    // init slot; mixing them in one process triggers the SQLITE_CONFIG_SERIALIZED assert).
    #[cfg(feature = "backend-libsql")]
    #[tokio::test(flavor = "current_thread")]
    #[ignore = "must run in isolation without backend-rusqlite (dual-SQLite init conflict)"]
    async fn libsql_select_one() {
        let dir = tempfile::tempdir().unwrap();
        let backend = LibsqlBackend::new(dir.path().join("t.db").to_str().unwrap());
        let row = backend.query_one_row("SELECT 1").await.unwrap().unwrap();
        assert_eq!(row[0], "1");
    }
}
