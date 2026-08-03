use anyhow::{Context, Result, anyhow};
use rusqlite::{Connection, OpenFlags};
use serde::Serialize;
use std::fs::Metadata;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub const DEFAULT_MMAP_BYTES: u64 = 256 * 1024 * 1024;
pub const DEFAULT_CACHE_KIB: i64 = 32 * 1024;
pub const DEFAULT_IDLE_MS: u64 = 300_000;

#[derive(Debug, Clone, Copy, Serialize)]
pub struct ReaderBudget {
    pub mmap_bytes: u64,
    pub cache_kib: i64,
    pub idle_ms: u64,
}

impl ReaderBudget {
    pub fn new(mmap_bytes: u64, cache_kib: i64, idle_ms: u64) -> Result<Self> {
        if mmap_bytes > 1024 * 1024 * 1024 {
            return Err(anyhow!(
                "sql reader mmap must be between 0 and 1073741824 bytes"
            ));
        }
        if !(1..=65_536).contains(&cache_kib) {
            return Err(anyhow!("sql reader cache must be between 1 and 65536 KiB"));
        }
        if !(30_000..=3_600_000).contains(&idle_ms) {
            return Err(anyhow!(
                "sql reader idle timeout must be between 30000 and 3600000 ms"
            ));
        }
        Ok(Self {
            mmap_bytes,
            cache_kib,
            idle_ms,
        })
    }

    pub fn reaper_period(self) -> Duration {
        Duration::from_millis((self.idle_ms / 10).clamp(1_000, 30_000))
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ReaderRegistrySnapshot {
    pub connection_open: bool,
    pub active_statements: usize,
    pub opens: u64,
    pub idle_reaps: u64,
    pub last_operation: Option<String>,
    pub last_statement_ms: Option<u128>,
    pub idle_ms: Option<u128>,
    pub budget: ReaderBudget,
}

pub struct SqlReader {
    path: PathBuf,
    budget: ReaderBudget,
    connection: Option<Connection>,
    last_used: Option<Instant>,
    active_statements: usize,
    opens: u64,
    idle_reaps: u64,
    last_operation: Option<String>,
    last_statement_ms: Option<u128>,
}

impl SqlReader {
    pub fn new(path: PathBuf, budget: ReaderBudget) -> Self {
        Self {
            path,
            budget,
            connection: None,
            last_used: None,
            active_statements: 0,
            opens: 0,
            idle_reaps: 0,
            last_operation: None,
            last_statement_ms: None,
        }
    }

    fn open_if_needed(&mut self) -> Result<()> {
        if self.connection.is_some() {
            return Ok(());
        }
        let conn = Connection::open_with_flags(
            &self.path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI,
        )
        .with_context(|| format!("open read-only SQL reader {}", self.path.display()))?;
        conn.pragma_update(None, "query_only", "ON")?;
        conn.pragma_update(None, "mmap_size", self.budget.mmap_bytes as i64)?;
        conn.pragma_update(None, "cache_size", -self.budget.cache_kib)?;
        conn.pragma_update(None, "temp_store", 2_i64)?;
        self.connection = Some(conn);
        self.opens += 1;
        Ok(())
    }

    pub fn with_connection<T>(
        &mut self,
        operation: &str,
        action: impl FnOnce(&Connection) -> Result<T>,
    ) -> Result<T> {
        self.reap_if_idle();
        self.open_if_needed()?;
        self.active_statements += 1;
        self.last_operation = Some(operation.to_owned());
        let started = Instant::now();
        let result = action(self.connection.as_ref().expect("reader opened"));
        self.active_statements -= 1;
        self.last_statement_ms = Some(started.elapsed().as_millis());
        self.last_used = Some(Instant::now());
        result
    }

    pub fn with_connection_string<T>(
        &mut self,
        operation: &str,
        action: impl FnOnce(&Connection) -> std::result::Result<T, String>,
    ) -> std::result::Result<T, String> {
        self.with_connection(operation, |conn| {
            action(conn).map_err(|error| anyhow!(error))
        })
        .map_err(|error| error.to_string())
    }

    pub fn reap_if_idle(&mut self) -> bool {
        self.reap_if_idle_at(Instant::now())
    }

    pub fn reap_if_idle_at(&mut self, now: Instant) -> bool {
        let idle = self.last_used.is_some_and(|used| {
            now.duration_since(used) >= Duration::from_millis(self.budget.idle_ms)
        });
        if self.active_statements == 0 && idle && self.connection.take().is_some() {
            self.last_used = None;
            self.idle_reaps += 1;
            return true;
        }
        false
    }

    pub fn snapshot(&self) -> ReaderRegistrySnapshot {
        ReaderRegistrySnapshot {
            connection_open: self.connection.is_some(),
            active_statements: self.active_statements,
            opens: self.opens,
            idle_reaps: self.idle_reaps,
            last_operation: self.last_operation.clone(),
            last_statement_ms: self.last_statement_ms,
            idle_ms: self.last_used.map(|used| used.elapsed().as_millis()),
            budget: self.budget,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct FileObservation {
    pub kind: &'static str,
    pub exists: bool,
    pub bytes: Option<u64>,
    pub modified_unix_ms: Option<u128>,
}

#[derive(Debug, Serialize)]
pub struct WalObservation {
    pub db: FileObservation,
    pub wal: FileObservation,
    pub shm: FileObservation,
    pub journal_mode: String,
    pub synchronous: i64,
    pub wal_autocheckpoint: i64,
    pub page_size: i64,
    pub data_version: i64,
    pub classification: &'static str,
    pub note: &'static str,
}

fn file_observation(kind: &'static str, path: PathBuf) -> FileObservation {
    let metadata: Option<Metadata> = std::fs::metadata(&path).ok();
    let modified_unix_ms = metadata.as_ref().and_then(|m| {
        m.modified()
            .ok()
            .and_then(|time: SystemTime| time.duration_since(UNIX_EPOCH).ok())
            .map(|duration| duration.as_millis())
    });
    FileObservation {
        kind,
        exists: metadata.is_some(),
        bytes: metadata.as_ref().map(Metadata::len),
        modified_unix_ms,
    }
}

/// Read-only observation. It intentionally never invokes a checkpoint pragma.
pub fn observe(path: &Path) -> Result<WalObservation> {
    let conn = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI,
    )
    .with_context(|| format!("open read-only WAL observation {}", path.display()))?;
    let journal_mode: String = conn.query_row("PRAGMA journal_mode", [], |row| row.get(0))?;
    let synchronous: i64 = conn.query_row("PRAGMA synchronous", [], |row| row.get(0))?;
    let wal_autocheckpoint: i64 =
        conn.query_row("PRAGMA wal_autocheckpoint", [], |row| row.get(0))?;
    let page_size: i64 = conn.query_row("PRAGMA page_size", [], |row| row.get(0))?;
    let data_version: i64 = conn.query_row("PRAGMA data_version", [], |row| row.get(0))?;
    let wal_path = PathBuf::from(format!("{}-wal", path.display()));
    let shm_path = PathBuf::from(format!("{}-shm", path.display()));
    Ok(WalObservation {
        db: file_observation("db", path.to_path_buf()),
        wal: file_observation("wal", wal_path),
        shm: file_observation("shm", shm_path),
        journal_mode,
        synchronous,
        wal_autocheckpoint,
        page_size,
        data_version,
        classification: "unknown",
        note: "read-only observation; retained WAL size is not a write-rate or blocker diagnosis",
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, SystemTime};

    fn temp_db(label: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "synapse-reader-{label}-{}-{nanos}.db",
            std::process::id()
        ))
    }

    #[test]
    fn budget_rejects_out_of_range_values() {
        assert!(ReaderBudget::new(1024 * 1024 * 1024 + 1, 1, 30_000).is_err());
        assert!(ReaderBudget::new(0, 0, 30_000).is_err());
        assert!(ReaderBudget::new(0, 1, 29_999).is_err());
    }

    #[test]
    fn reader_reaps_after_idle_without_a_statement() {
        let path = temp_db("reap");
        let _ = std::fs::remove_file(&path);
        Connection::open(&path)
            .unwrap()
            .execute_batch("CREATE TABLE t (id INTEGER)")
            .unwrap();
        let mut reader = SqlReader::new(path.clone(), ReaderBudget::new(0, 1, 30_000).unwrap());
        reader
            .with_connection("test", |conn| {
                let _: i64 = conn.query_row("SELECT COUNT(*) FROM t", [], |row| row.get(0))?;
                Ok(())
            })
            .unwrap();
        let now = Instant::now();
        reader.last_used = Some(now - Duration::from_secs(31));
        assert!(reader.reap_if_idle_at(now));
        assert!(!reader.snapshot().connection_open);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn query_only_reader_can_attach_a_read_only_tenant() {
        let primary = temp_db("primary");
        let tenant = temp_db("tenant");
        Connection::open(&primary)
            .unwrap()
            .execute_batch("CREATE TABLE primary_docs (id INTEGER)")
            .unwrap();
        Connection::open(&tenant)
            .unwrap()
            .execute_batch("CREATE TABLE docs (id INTEGER); INSERT INTO docs VALUES (7)")
            .unwrap();
        let tenant_uri = format!("file:{}?mode=ro", tenant.display());
        let mut reader = SqlReader::new(primary.clone(), ReaderBudget::new(0, 1, 30_000).unwrap());
        let value: i64 = reader
            .with_connection("attach_tenant", |conn| {
                conn.execute("ATTACH DATABASE ?1 AS tenant", [&tenant_uri])?;
                Ok(conn.query_row("SELECT id FROM tenant.docs", [], |row| row.get(0))?)
            })
            .unwrap();
        assert_eq!(value, 7);
        drop(reader);
        let _ = std::fs::remove_file(primary);
        let _ = std::fs::remove_file(tenant);
    }

    #[test]
    fn observation_is_structured_and_does_not_expose_a_path() {
        let path = temp_db("observe");
        Connection::open(&path)
            .unwrap()
            .execute_batch("CREATE TABLE docs (id INTEGER)")
            .unwrap();
        let encoded = serde_json::to_value(observe(&path).unwrap()).unwrap();
        assert_eq!(encoded["db"]["kind"], "db");
        assert!(encoded.get("path").is_none());
        assert_eq!(encoded["classification"], "unknown");
        let _ = std::fs::remove_file(path);
    }
}
