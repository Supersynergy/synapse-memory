//! `AS OF` time-travel clause — CRDT snapshot query.
//!
//! # Syntax
//! ```sql
//! SELECT * FROM docs AS OF '2026-01-01T00:00:00Z';
//! SELECT * FROM docs AS OF LAMPORT 42;
//! ```
//!
//! TODO: wire parsed snapshot into synapse-core crdt::Op log reader.

use chrono::DateTime;

#[derive(Debug, Clone, PartialEq)]
pub enum Snapshot {
    WallClock(u64), // Unix seconds
    Lamport(u64),
}

/// Parse an `AS OF` clause from a SQL string.
/// Returns `(clean_sql, Some(snapshot))` or `(original, None)`.
pub fn parse_as_of(sql: &str) -> (String, Option<Snapshot>) {
    let upper = sql.to_ascii_uppercase();
    if let Some(pos) = upper.find(" AS OF ") {
        let rest = sql[pos + 7..].trim();

        if let Some(lamport_rest) = rest
            .strip_prefix("LAMPORT ")
            .or_else(|| rest.strip_prefix("lamport "))
        {
            let n_str = lamport_rest
                .split_whitespace()
                .next()
                .unwrap_or("")
                .trim_end_matches(';');
            if let Ok(n) = n_str.parse::<u64>() {
                let clean = sql[..pos].to_owned();
                return (clean, Some(Snapshot::Lamport(n)));
            }
        }

        // Try ISO-8601 timestamp between quotes.
        if let Some(q_start) = rest.find('\'')
            && let Some(q_end) = rest[q_start + 1..].find('\'')
        {
            let ts_str = &rest[q_start + 1..q_start + 1 + q_end];
            if let Ok(dt) = DateTime::parse_from_rfc3339(ts_str) {
                let ts = dt.timestamp() as u64;
                let clean = sql[..pos].to_owned();
                return (clean, Some(Snapshot::WallClock(ts)));
            }
        }
    }
    (sql.to_owned(), None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lamport_parse() {
        let (clean, snap) = parse_as_of("SELECT * FROM docs AS OF LAMPORT 42");
        assert_eq!(snap, Some(Snapshot::Lamport(42)));
        assert_eq!(clean.trim(), "SELECT * FROM docs");
    }

    #[test]
    fn no_as_of() {
        let (clean, snap) = parse_as_of("SELECT * FROM docs");
        assert!(snap.is_none());
        assert_eq!(clean, "SELECT * FROM docs");
    }

    #[test]
    fn wallclock_parses_iso_not_now() {
        let (clean, snap) = parse_as_of("SELECT * FROM docs AS OF '2026-05-12T10:00:00Z'");
        assert_eq!(clean.trim(), "SELECT * FROM docs");
        match snap {
            Some(Snapshot::WallClock(ts)) => {
                // 2026-05-12T10:00:00Z = 1778580000
                assert_eq!(
                    ts, 1778580000,
                    "must parse actual timestamp, not SystemTime::now()"
                );
            }
            other => panic!("expected WallClock, got {:?}", other),
        }
    }
}
