//! Optional DuckLake archive support.
//!
//! DuckLake is a lakehouse format that uses a SQL catalog (SQLite, DuckDB, or
//! PostgreSQL) for metadata of Parquet files. It gives snapshots, time travel,
//! and ACID over Parquet — perfect for archiving old `synapse_events` rows.
//!
//! This module is a thin wrapper: it assumes `duckdb` CLI is in PATH. The
//! actual DuckLake catalog is a SQLite file (`metadata.ducklake` by default)
//! that DuckDB creates and manages.
//!
//! **Design note (C2):** Uses `duckdb` CLI rather than the `duckdb` Rust crate
//! because the crate adds ~50MB to the binary and significant compile time for
//! a path that runs rarely (archival). The CLI is fast, single-invocation, and
//! already installed on the target machine.
//!
//! Usage:
//!   - `synapse-ultra lake init` — creates the catalog + `synapse_events` table
//!   - `synapse-ultra lake archive --older-than 90d` — moves old rows to Parquet
//!   - `synapse-ultra lake analytics` — starts DuckDB with ATTACH on brain.db

use crate::UltraResult;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone)]
pub struct LakeConfig {
    pub catalog_path: PathBuf,
    pub data_dir: PathBuf,
}

impl Default for LakeConfig {
    fn default() -> Self {
        Self {
            catalog_path: PathBuf::from("metadata.ducklake"),
            data_dir: PathBuf::from("lake-data"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct LakeState {
    pub initialized: bool,
    pub catalog_path: PathBuf,
    pub archived_rows: i64,
}

/// Check if DuckDB CLI is available.
pub fn duckdb_available() -> bool {
    Command::new("duckdb")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Initialize a DuckLake catalog. Creates the catalog SQLite file and the
/// data directory. Idempotent — safe to call repeatedly.
pub fn init(cfg: &LakeConfig) -> UltraResult<()> {
    if !duckdb_available() {
        return Err(crate::UltraError::DuckLake(
            "duckdb CLI not found in PATH. Install via `brew install duckdb` or https://duckdb.org/".into(),
        ));
    }
    std::fs::create_dir_all(&cfg.data_dir)?;
    let sql = format!(
        r#"
INSTALL ducklake;
LOAD ducklake;
ATTACH 'ducklake:{catalog}' AS synapse_lake (DATA_PATH '{data}');
CREATE TABLE IF NOT EXISTS synapse_lake.synapse_events AS
SELECT * FROM (SELECT 1 AS id, 0 AS ts, '' AS agent, '' AS kind, '' AS uri, '' AS content) WHERE 1=0;
"#,
        catalog = cfg.catalog_path.display(),
        data = cfg.data_dir.display(),
    );
    let out = Command::new("duckdb")
        .arg(":memory:")
        .arg("-c")
        .arg(&sql)
        .output()?;
    if !out.status.success() {
        return Err(crate::UltraError::DuckLake(format!(
            "duckdb init failed: {}",
            String::from_utf8_lossy(&out.stderr)
        )));
    }
    Ok(())
}

/// Check whether the catalog exists and count archived rows.
pub fn state(cfg: &LakeConfig) -> UltraResult<LakeState> {
    let initialized = cfg.catalog_path.exists();
    if !initialized {
        return Ok(LakeState {
            initialized: false,
            catalog_path: cfg.catalog_path.clone(),
            archived_rows: 0,
        });
    }
    let sql = format!(
        "INSTALL ducklake; LOAD ducklake; ATTACH 'ducklake:{}' AS synapse_lake; SELECT COUNT(*) FROM synapse_lake.synapse_events;",
        cfg.catalog_path.display()
    );
    let out = Command::new("duckdb")
        .arg(":memory:")
        .arg("-c")
        .arg(&sql)
        .output()?;
    if !out.status.success() {
        return Ok(LakeState {
            initialized: true,
            catalog_path: cfg.catalog_path.clone(),
            archived_rows: 0,
        });
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    let count = stdout
        .lines()
        .find_map(|l| l.trim().parse::<i64>().ok())
        .unwrap_or(0);
    Ok(LakeState {
        initialized: true,
        catalog_path: cfg.catalog_path.clone(),
        archived_rows: count,
    })
}

/// Archive rows older than `cutoff_ts` from `brain.db` into the DuckLake table,
/// then delete the archived rows from brain.db. Returns the number of rows archived.
///
/// Single duckdb invocation exports to Parquet + ingests into DuckLake. The
/// brain.db cleanup is a separate `DELETE` on the source connection.
pub fn archive(brain_db: &Path, cfg: &LakeConfig, cutoff_ts: i64) -> UltraResult<i64> {
    if !cfg.catalog_path.exists() {
        init(cfg)?;
    }
    // Export old rows from brain.db to a Parquet file, then ingest into DuckLake.
    let parquet_path = cfg.data_dir.join(format!("events_{}.parquet", cutoff_ts));
    let export_sql = format!(
        r#"
INSTALL ducklake; LOAD ducklake;
ATTACH 'ducklake:{catalog}' AS synapse_lake (DATA_PATH '{data}');
ATTACH '{brain}' AS brain (READ_ONLY);
COPY (SELECT id, ts, session_id, agent, kind, uri, content, meta
      FROM brain.synapse_events WHERE ts < {cutoff}) TO '{parquet}';
INSERT INTO synapse_lake.synapse_events
SELECT id, ts, session_id, agent, kind, uri, content, meta FROM read_parquet('{parquet}');
SELECT COUNT(*) FROM read_parquet('{parquet}');
"#,
        catalog = cfg.catalog_path.display(),
        data = cfg.data_dir.display(),
        brain = brain_db.display(),
        cutoff = cutoff_ts,
        parquet = parquet_path.display(),
    );
    let out = Command::new("duckdb")
        .arg(":memory:")
        .arg("-c")
        .arg(&export_sql)
        .output()?;
    if !out.status.success() {
        return Err(crate::UltraError::DuckLake(format!(
            "duckdb archive failed: {}",
            String::from_utf8_lossy(&out.stderr)
        )));
    }
    // Parse the trailing COUNT(*) from the last SELECT.
    let stdout = String::from_utf8_lossy(&out.stdout);
    let count = stdout
        .lines()
        .rev()
        .find_map(|l| l.trim().parse::<i64>().ok())
        .unwrap_or(0);
    // Delete archived rows from brain.db so the archive actually frees space.
    if count > 0 {
        let conn = rusqlite::Connection::open(brain_db)?;
        conn.execute(
            "DELETE FROM synapse_events WHERE ts < ?1",
            rusqlite::params![cutoff_ts],
        )?;
        conn.execute("PRAGMA wal_checkpoint(PASSIVE)", [])?;
    }
    Ok(count)
}

/// Start a DuckDB shell with both the brain.db and the DuckLake catalog
/// attached, for interactive analytics. Blocks until the user exits.
pub fn analytics_shell(brain_db: &Path, cfg: &LakeConfig) -> UltraResult<()> {
    if !cfg.catalog_path.exists() {
        return Err(crate::UltraError::DuckLake(
            "catalog not initialized — run `synapse-ultra lake init` first".into(),
        ));
    }
    let sql = format!(
        "INSTALL ducklake; LOAD ducklake; ATTACH 'ducklake:{catalog}' AS synapse_lake (DATA_PATH '{data}'); ATTACH '{brain}' AS brain (READ_ONLY);",
        catalog = cfg.catalog_path.display(),
        data = cfg.data_dir.display(),
        brain = brain_db.display(),
    );
    let status = Command::new("duckdb")
        .arg(":memory:")
        .arg("-c")
        .arg(&sql)
        .status()?;
    if !status.success() {
        return Err(crate::UltraError::DuckLake(
            "duckdb analytics shell exited with non-zero status".into(),
        ));
    }
    Ok(())
}
