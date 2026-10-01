//! Transaction statement classifier + MVCC isolation for SynapsQL.
//!
//! BEGIN / COMMIT / ROLLBACK pass directly to the SQLite backend as raw SQL.
//! This module provides detection helpers so the MySQL/PG shims can route them
//! correctly (always exec, return OkResponse, never treated as read-only).
//!
//! MVCC extensions (Pattern: maxpert/marmot ReadCoordinator, ClickHouse snapshot tests):
//!   - `BEGIN READ ONLY`  → SQLite `BEGIN DEFERRED` (shared reader, no write intent)
//!   - `BEGIN READ WRITE` → SQLite `BEGIN IMMEDIATE` (prevents writer starvation)
//!   - `BEGIN AS OF TIMESTAMP '<ts>'` → time-travel read via Synapse WAL snapshot
//!   - `SET TRANSACTION ISOLATION LEVEL <level>` → mapped to SQLite BEGIN variant

/// Standard SQL isolation levels mapped to SQLite BEGIN variants.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IsolationLevel {
    /// Each statement sees committed data at statement start (SQLite DEFERRED).
    ReadCommitted,
    /// Entire transaction sees DB as of BEGIN time (SQLite default BEGIN).
    RepeatableRead,
    /// Serializable writes (SQLite IMMEDIATE — blocks concurrent writers).
    Serializable,
}

impl IsolationLevel {
    /// The SQLite BEGIN statement for this level.
    pub fn sqlite_begin(self) -> &'static str {
        match self {
            IsolationLevel::ReadCommitted => "BEGIN DEFERRED",
            IsolationLevel::RepeatableRead => "BEGIN",
            IsolationLevel::Serializable => "BEGIN IMMEDIATE",
        }
    }

    fn from_upper(s: &str) -> Option<Self> {
        match s.trim() {
            "READ COMMITTED" => Some(IsolationLevel::ReadCommitted),
            "REPEATABLE READ" => Some(IsolationLevel::RepeatableRead),
            "SERIALIZABLE" => Some(IsolationLevel::Serializable),
            _ => None,
        }
    }
}

/// Classify `sql` as a transaction control statement.
#[derive(Debug, Clone, PartialEq)]
pub enum TxnStatement {
    /// `BEGIN` — default read/write.
    Begin,
    /// `BEGIN READ ONLY` — shared reader lock, no write intent.
    BeginReadOnly,
    /// `BEGIN READ WRITE` — explicit read/write (maps to BEGIN IMMEDIATE).
    BeginReadWrite,
    /// `BEGIN AS OF TIMESTAMP '<ts>'` — Synapse time-travel snapshot.
    BeginAsOf {
        timestamp: String,
    },
    /// `SET TRANSACTION ISOLATION LEVEL <level>`.
    SetIsolationLevel(IsolationLevel),
    Commit,
    Rollback,
    Savepoint(String),
    ReleaseSavepoint(String),
    RollbackToSavepoint(String),
}

impl TxnStatement {
    /// Map this statement to the SQLite SQL to execute.
    pub fn to_sqlite(&self) -> &'static str {
        match self {
            TxnStatement::Begin => "BEGIN",
            TxnStatement::BeginReadOnly => "BEGIN DEFERRED",
            TxnStatement::BeginReadWrite => "BEGIN IMMEDIATE",
            TxnStatement::BeginAsOf { .. } => "BEGIN DEFERRED",
            TxnStatement::SetIsolationLevel(l) => l.sqlite_begin(),
            TxnStatement::Commit => "COMMIT",
            TxnStatement::Rollback => "ROLLBACK",
            // Savepoints: callers must format manually
            TxnStatement::Savepoint(_) => "SAVEPOINT",
            TxnStatement::ReleaseSavepoint(_) => "RELEASE SAVEPOINT",
            TxnStatement::RollbackToSavepoint(_) => "ROLLBACK TO SAVEPOINT",
        }
    }
}

/// Returns `Some(TxnStatement)` if `sql` is a transaction control statement.
pub fn classify_txn(sql: &str) -> Option<TxnStatement> {
    let s = sql.trim().trim_end_matches(';');
    let upper = s.to_ascii_uppercase();
    let lead: &str = upper.split_whitespace().next().unwrap_or("");

    // SET TRANSACTION ISOLATION LEVEL ...
    if upper.starts_with("SET TRANSACTION ISOLATION LEVEL") {
        let level_str = s["SET TRANSACTION ISOLATION LEVEL".len()..]
            .trim()
            .to_ascii_uppercase();
        return IsolationLevel::from_upper(&level_str).map(TxnStatement::SetIsolationLevel);
    }

    match lead {
        "BEGIN" | "START" => {
            let rest = if lead == "BEGIN" {
                s[5..].trim()
            } else {
                // START TRANSACTION [READ ONLY|READ WRITE]
                s[5..].trim().trim_start_matches("TRANSACTION").trim()
            };
            let rest_upper = rest.to_ascii_uppercase();
            if rest_upper.starts_with("READ ONLY") {
                return Some(TxnStatement::BeginReadOnly);
            }
            if rest_upper.starts_with("READ WRITE") {
                return Some(TxnStatement::BeginReadWrite);
            }
            if let Some(suffix) = rest_upper.strip_prefix("AS OF TIMESTAMP") {
                let ts_raw = rest[rest_upper.find("AS OF TIMESTAMP").unwrap() + 15..].trim();
                let ts = ts_raw.trim_matches(|c| c == '\'' || c == '"').to_owned();
                if !ts.is_empty() {
                    return Some(TxnStatement::BeginAsOf { timestamp: ts });
                }
                let _ = suffix;
            }
            Some(TxnStatement::Begin)
        }
        "COMMIT" | "END" => Some(TxnStatement::Commit),
        "ROLLBACK" => {
            // ROLLBACK TO [SAVEPOINT] name
            if let Some(rest) = upper.strip_prefix("ROLLBACK TO") {
                let name = rest
                    .trim_start()
                    .strip_prefix("SAVEPOINT")
                    .unwrap_or(rest.trim_start())
                    .trim()
                    .to_owned();
                if !name.is_empty() {
                    return Some(TxnStatement::RollbackToSavepoint(
                        s[s.to_ascii_uppercase().len() - name.len()..]
                            .trim()
                            .to_owned(),
                    ));
                }
            }
            Some(TxnStatement::Rollback)
        }
        "SAVEPOINT" => {
            let name = s[9..].trim().to_owned();
            if name.is_empty() {
                None
            } else {
                Some(TxnStatement::Savepoint(name))
            }
        }
        "RELEASE" => {
            let rest = s[7..].trim();
            let name = rest
                .strip_prefix("SAVEPOINT")
                .unwrap_or(rest)
                .trim()
                .to_owned();
            if name.is_empty() {
                None
            } else {
                Some(TxnStatement::ReleaseSavepoint(name))
            }
        }
        _ => None,
    }
}

/// Returns true if `sql` is any transaction control statement.
#[inline]
pub fn is_txn_statement(sql: &str) -> bool {
    classify_txn(sql).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn begin_variants() {
        assert_eq!(classify_txn("BEGIN"), Some(TxnStatement::Begin));
        assert_eq!(classify_txn("begin;"), Some(TxnStatement::Begin));
        assert_eq!(classify_txn("START TRANSACTION"), Some(TxnStatement::Begin));
    }

    #[test]
    fn commit_variants() {
        assert_eq!(classify_txn("COMMIT"), Some(TxnStatement::Commit));
        assert_eq!(classify_txn("commit;"), Some(TxnStatement::Commit));
        assert_eq!(classify_txn("END"), Some(TxnStatement::Commit));
    }

    #[test]
    fn rollback_plain() {
        assert_eq!(classify_txn("ROLLBACK"), Some(TxnStatement::Rollback));
        assert_eq!(classify_txn("rollback;"), Some(TxnStatement::Rollback));
    }

    #[test]
    fn savepoint() {
        assert_eq!(
            classify_txn("SAVEPOINT sp1"),
            Some(TxnStatement::Savepoint("sp1".into()))
        );
    }

    #[test]
    fn non_txn_returns_none() {
        assert_eq!(classify_txn("SELECT 1"), None);
        assert_eq!(classify_txn("INSERT INTO t VALUES (1)"), None);
    }

    #[test]
    fn begin_read_only() {
        assert_eq!(
            classify_txn("BEGIN READ ONLY"),
            Some(TxnStatement::BeginReadOnly)
        );
        assert_eq!(
            classify_txn("begin read only;"),
            Some(TxnStatement::BeginReadOnly)
        );
    }

    #[test]
    fn begin_read_write() {
        assert_eq!(
            classify_txn("BEGIN READ WRITE"),
            Some(TxnStatement::BeginReadWrite)
        );
    }

    #[test]
    fn begin_as_of_timestamp() {
        let d = classify_txn("BEGIN AS OF TIMESTAMP '2026-01-01T00:00:00Z'").unwrap();
        assert!(matches!(d, TxnStatement::BeginAsOf { .. }));
        if let TxnStatement::BeginAsOf { timestamp } = d {
            assert_eq!(timestamp, "2026-01-01T00:00:00Z");
        }
    }

    #[test]
    fn set_isolation_serializable() {
        assert_eq!(
            classify_txn("SET TRANSACTION ISOLATION LEVEL SERIALIZABLE"),
            Some(TxnStatement::SetIsolationLevel(
                IsolationLevel::Serializable
            ))
        );
    }

    #[test]
    fn set_isolation_read_committed() {
        let d = classify_txn("SET TRANSACTION ISOLATION LEVEL READ COMMITTED").unwrap();
        assert_eq!(
            d,
            TxnStatement::SetIsolationLevel(IsolationLevel::ReadCommitted)
        );
        assert_eq!(
            IsolationLevel::ReadCommitted.sqlite_begin(),
            "BEGIN DEFERRED"
        );
    }

    #[test]
    fn sqlite_mapping_read_only() {
        assert_eq!(TxnStatement::BeginReadOnly.to_sqlite(), "BEGIN DEFERRED");
        assert_eq!(TxnStatement::BeginReadWrite.to_sqlite(), "BEGIN IMMEDIATE");
    }

    #[test]
    fn sqlite_mapping_isolation_levels() {
        assert_eq!(
            TxnStatement::SetIsolationLevel(IsolationLevel::Serializable).to_sqlite(),
            "BEGIN IMMEDIATE"
        );
        assert_eq!(
            TxnStatement::SetIsolationLevel(IsolationLevel::RepeatableRead).to_sqlite(),
            "BEGIN"
        );
    }
}
