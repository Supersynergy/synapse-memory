//! ProxySQL-style statement fingerprinting.
//!
//! Normalizes a SQL string: strips comments, collapses whitespace,
//! replaces literal values with `?`, lowercases keywords.
//! Output is stable across different parameter values → safe cache key.
//!
//! Examples:
//!   `SELECT * FROM t WHERE id = 42`  → `select * from t where id = ?`
//!   `INSERT INTO t VALUES (1,'a',3)` → `insert into t values (?,?,?)`

/// Fingerprint a SQL statement. Returns a normalized string.
/// Cheap: single-pass O(n), no alloc for short queries (stack-smol).
pub fn fingerprint(sql: &str) -> String {
    let mut out = String::with_capacity(sql.len());
    let bytes = sql.as_bytes();
    let len = bytes.len();
    let mut i = 0;
    let mut last_was_space = false;

    while i < len {
        let b = bytes[i];

        // Strip `--` line comments
        if b == b'-' && i + 1 < len && bytes[i + 1] == b'-' {
            while i < len && bytes[i] != b'\n' { i += 1; }
            continue;
        }

        // Strip `/* */` block comments
        if b == b'/' && i + 1 < len && bytes[i + 1] == b'*' {
            i += 2;
            while i + 1 < len && !(bytes[i] == b'*' && bytes[i + 1] == b'/') { i += 1; }
            i += 2;
            continue;
        }

        // Replace string literals 'x' → ?
        if b == b'\'' {
            i += 1;
            while i < len {
                if bytes[i] == b'\'' {
                    if i + 1 < len && bytes[i + 1] == b'\'' { i += 2; continue; }
                    break;
                }
                if bytes[i] == b'\\' { i += 1; }
                i += 1;
            }
            i += 1; // consume closing '
            out.push('?');
            last_was_space = false;
            continue;
        }

        // Replace numeric literals → ?
        if b.is_ascii_digit() {
            // Check that previous char was not alphanumeric (avoid column name digits)
            let prev_ok = out.as_bytes().last()
                .map(|&c| !c.is_ascii_alphanumeric() && c != b'_')
                .unwrap_or(true);
            if prev_ok {
                while i < len && (bytes[i].is_ascii_digit() || bytes[i] == b'.') { i += 1; }
                out.push('?');
                last_was_space = false;
                continue;
            }
        }

        // Collapse whitespace
        if b.is_ascii_whitespace() {
            if !last_was_space && !out.is_empty() {
                out.push(' ');
                last_was_space = true;
            }
            i += 1;
            continue;
        }

        // Lowercase everything else
        out.push(b.to_ascii_lowercase() as char);
        last_was_space = false;
        i += 1;
    }

    out.trim_end().to_owned()
}

/// Classify a fingerprinted query as read-only or write.
/// Used for read/write split (MyDuck pattern).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueryKind {
    Read,
    Write,
    Ddl,
    Other,
}

pub fn classify(fp: &str) -> QueryKind {
    let lead = fp.trim_start();
    if lead.starts_with("select") || lead.starts_with("show") || lead.starts_with("explain") {
        QueryKind::Read
    } else if lead.starts_with("insert") || lead.starts_with("update") || lead.starts_with("delete") || lead.starts_with("replace") {
        QueryKind::Write
    } else if lead.starts_with("create") || lead.starts_with("drop") || lead.starts_with("alter") || lead.starts_with("truncate") {
        QueryKind::Ddl
    } else {
        QueryKind::Other
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic_literal_strip() {
        let fp = fingerprint("SELECT * FROM t WHERE id = 42 AND name = 'alice'");
        assert_eq!(fp, "select * from t where id = ? and name = ?");
    }

    #[test]
    fn comment_strip() {
        let fp = fingerprint("SELECT 1 -- trailing comment");
        assert_eq!(fp, "select ?");
    }

    #[test]
    fn block_comment() {
        let fp = fingerprint("SELECT /* inline */ 1");
        assert_eq!(fp, "select ?");
    }

    #[test]
    fn classify_read() {
        assert_eq!(classify("select * from t"), QueryKind::Read);
    }

    #[test]
    fn classify_write() {
        assert_eq!(classify("insert into t values (?)"), QueryKind::Write);
    }
}
