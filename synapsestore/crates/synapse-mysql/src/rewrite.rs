use anyhow::Result;
use regex::Regex;
use std::sync::OnceLock;

// Pre-compiled regexes — `Regex::new` per call was the OLTP hot-path bottleneck.
// Each pattern compiles once, lives forever.
fn re_delete_multi() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"(?i)^DELETE\s+\w+,\s*\w+\s+FROM").unwrap())
}
fn re_at_var() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"@@(?:SESSION\.|GLOBAL\.)?(\w+)").unwrap())
}

/// Canonical MySQL system-variable defaults used by `SHOW VARIABLES LIKE ...`.
/// wp-cli / wpdb consult these during connect; if `Value` is empty for any of
/// `character_set_*`, the install routine bails with exit 0 (silent skip).
pub(crate) const MYSQL_DEFAULT_VARS: &[(&str, &str)] = &[
    ("character_set_client", "utf8mb4"),
    ("character_set_connection", "utf8mb4"),
    ("character_set_database", "utf8mb4"),
    ("character_set_filesystem", "binary"),
    ("character_set_results", "utf8mb4"),
    ("character_set_server", "utf8mb4"),
    ("character_set_system", "utf8"),
    ("collation_connection", "utf8mb4_unicode_ci"),
    ("collation_database", "utf8mb4_unicode_ci"),
    ("collation_server", "utf8mb4_unicode_ci"),
    ("sql_mode", "NO_ENGINE_SUBSTITUTION"),
    ("max_allowed_packet", "67108864"),
    ("wait_timeout", "28800"),
    ("interactive_timeout", "28800"),
    ("net_read_timeout", "30"),
    ("net_write_timeout", "60"),
    ("version", "8.0.35-synapse"),
    ("version_comment", "Synapse MySQL shim"),
    ("version_compile_os", "macos"),
    ("innodb_version", "8.0.35"),
    ("protocol_version", "10"),
    ("have_innodb", "YES"),
    ("have_query_cache", "NO"),
    ("lower_case_table_names", "2"),
    ("time_zone", "+00:00"),
    ("system_time_zone", "UTC"),
    ("default_storage_engine", "InnoDB"),
    ("storage_engine", "InnoDB"),
    ("autocommit", "ON"),
    ("foreign_key_checks", "ON"),
    ("unique_checks", "ON"),
];

/// Validate a table name for safe interpolation into SQL.
/// SHOW COLUMNS / SHOW INDEX / SHOW CREATE / DESCRIBE all need to interpolate
/// the table name into a `pragma_table_info('{}')` call. Allow only
/// `[A-Za-z0-9_]+`, max 64 chars (MySQL identifier limit). Reject anything else.
pub(crate) fn safe_table_name(t: &str) -> Result<&str> {
    if t.is_empty() || t.len() > 64 {
        anyhow::bail!("invalid table name length");
    }
    if !t.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
        anyhow::bail!("invalid table name characters");
    }
    Ok(t)
}

pub fn rewrite(sql: &str, _mode: &str) -> Result<String> {
    let mut out = sql.to_string();
    let upper = out.trim().to_uppercase();

    // Multi-table DELETE (WordPress transient cleanup) -> no-op
    if re_delete_multi().is_match(&upper) {
        return Ok("SELECT 1".to_string());
    }

    // SET statements -> no-op (WordPress sends many of these)
    if upper.starts_with("SET ") || upper.starts_with("SET@") {
        if upper.contains("FOREIGN_KEY_CHECKS") {
            if upper.contains("=0") || upper.contains("= 0") {
                return Ok("PRAGMA foreign_keys = OFF".to_string());
            } else {
                return Ok("PRAGMA foreign_keys = ON".to_string());
            }
        }
        return Ok("SELECT 1".to_string());
    }

    // SELECT @@SESSION.* / @@GLOBAL.* MySQL system variables
    // Replace each @@var with a literal value; keeps SELECT structure intact.
    if upper.contains("@@") {
        let re = re_at_var();
        let rewritten = re.replace_all(&out, |caps: &regex::Captures| {
            let var = caps.get(1).unwrap().as_str().to_lowercase();
            match var.as_str() {
                "max_allowed_packet" => "67108864".to_string(),
                "sql_mode" => "''".to_string(),
                "character_set_client" | "character_set_connection"
                | "character_set_results" | "collation_connection" => "'utf8mb4'".to_string(),
                "time_zone" => "'+00:00'".to_string(),
                "tx_isolation" | "transaction_isolation" => "'READ-COMMITTED'".to_string(),
                _ => "''".to_string(),
            }
        });
        // Ensure result is a SELECT with named columns for proper resultset
        let r = rewritten.trim().to_string();
        return Ok(if r.to_uppercase().starts_with("SELECT") {
            r
        } else {
            format!("SELECT {} as val", r)
        });
    }

    // CREATE DATABASE / DROP DATABASE -> no-op (SQLite is single-file, no DB concept)
    if upper.starts_with("CREATE DATABASE") || upper.starts_with("CREATE SCHEMA")
        || upper.starts_with("DROP DATABASE") || upper.starts_with("DROP SCHEMA")
    {
        return Ok("SELECT 1".to_string());
    }

    // USE <db> -> no-op
    if upper.starts_with("USE ") {
        return Ok("SELECT 1".to_string());
    }

    // ANALYZE TABLE -> no-op (SQLite uses ANALYZE without TABLE keyword)
    if upper.starts_with("ANALYZE TABLE") || upper.starts_with("ANALYZE ") {
        return Ok("SELECT 1".to_string());
    }

    // SHOW TABLES
    if upper.starts_with("SHOW TABLES") {
        return Ok("SELECT name as Tables_in_database FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' AND name NOT LIKE '_mysql_%'".to_string());
    }

    // SHOW DATABASES
    if upper.starts_with("SHOW DATABASES") {
        return Ok("SELECT name as Database FROM pragma_database_list()".to_string());
    }

    // SHOW VARIABLES LIKE ... -> canonical MySQL defaults so wp-cli/wpdb
    // charset/version checks don't silently bail. wp-cli exits 0 when these
    // queries return empty `Value` fields (it skips the data-write phase).
    if upper.starts_with("SHOW VARIABLES") {
        if let Some(cap) = Regex::new(r"(?i)LIKE\s+'([^']+)'").unwrap().captures(&out) {
            let pat = cap.get(1).unwrap().as_str();
            // Map LIKE pattern → canonical (Variable_name, Value).
            // Pattern is glob-style (% wildcard); we resolve to the table.
            let pat_l = pat.to_lowercase();
            let pat_re = pat_l.replace('%', ".*");
            let re = Regex::new(&format!("^{}$", pat_re)).unwrap_or_else(|_| Regex::new("^$").unwrap());
            let mut rows: Vec<(&str, &str)> = Vec::new();
            for (k, v) in MYSQL_DEFAULT_VARS {
                if re.is_match(k) {
                    rows.push((*k, *v));
                }
            }
            if rows.is_empty() {
                // Fall back: return the literal name with utf8mb4 default so
                // charset checks pass instead of silently aborting.
                rows.push((pat, "utf8mb4"));
            }
            let body = rows
                .iter()
                .map(|(k, v)| format!("SELECT '{}' as Variable_name, '{}' as Value", k.replace('\'', "''"), v.replace('\'', "''")))
                .collect::<Vec<_>>()
                .join(" UNION ALL ");
            return Ok(body);
        }
        // No LIKE clause → emit full canonical table.
        let body = MYSQL_DEFAULT_VARS
            .iter()
            .map(|(k, v)| format!("SELECT '{}' as Variable_name, '{}' as Value", k, v))
            .collect::<Vec<_>>()
            .join(" UNION ALL ");
        return Ok(body);
    }

    // SHOW CREATE TABLE
    if upper.starts_with("SHOW CREATE TABLE ") {
        if let Some(t) = out.split_whitespace().nth(3) {
            let t = t.trim_matches('`').trim_end_matches(';');
            let t = safe_table_name(t)?;
            return Ok(format!(
                "SELECT '{}' as Table, ('CREATE TABLE ' || name || '(' || group_concat(name || ' ' || type, ', ') || ')') as CreateTable FROM pragma_table_info('{}')",
                t, t
            ));
        }
    }

    // SHOW FULL COLUMNS FROM / SHOW COLUMNS FROM
    // Map pragma_table_info columns to MySQL SHOW COLUMNS shape:
    //   Field, Type, Null, Key, Default, Extra
    if upper.starts_with("SHOW FULL COLUMNS FROM ") || upper.starts_with("SHOW COLUMNS FROM ") {
        let words: Vec<&str> = out.split_whitespace().collect();
        if let Some(t) = words.last() {
            let t = t.trim_matches('`').trim_matches('\'').trim_end_matches(';');
            let t = safe_table_name(t)?;
            return Ok(format!(
                "SELECT name as Field, type as Type, \
                 CASE \"notnull\" WHEN 1 THEN 'NO' ELSE 'YES' END as \"Null\", \
                 CASE \"pk\" WHEN 1 THEN 'PRI' ELSE '' END as \"Key\", \
                 dflt_value as \"Default\", \
                 '' as Extra, \
                 '' as Collation, \
                 '' as Privileges, \
                 '' as Comment \
                 FROM pragma_table_info('{}')",
                t
            ));
        }
    }

    // SHOW INDEX FROM
    if upper.starts_with("SHOW INDEX FROM ") || upper.starts_with("SHOW KEYS FROM ") {
        let words: Vec<&str> = out.split_whitespace().collect();
        if let Some(t) = words.last() {
            let t = t.trim_matches('`').trim_matches('\'').trim_end_matches(';');
            let t = safe_table_name(t)?;
            return Ok(format!(
                "SELECT name as Key_name, seq as Seq_in_index, 'BTREE' as Index_type FROM pragma_index_list('{}')",
                t
            ));
        }
    }

    // SHOW TABLE STATUS
    if upper.starts_with("SHOW TABLE STATUS") {
        return Ok(
            "SELECT name as Name, 'SQLite' as Engine, 10 as Version, 'Dynamic' as Row_format, \
             (SELECT COUNT(*) FROM sqlite_master WHERE type='table') as Rows, \
             0 as Avg_row_length, 0 as Data_length, 0 as Max_data_length, \
             0 as Index_length, 0 as Data_free, 0 as Auto_increment, \
             datetime('now') as Create_time, datetime('now') as Update_time, \
             datetime('now') as Check_time, 'utf8mb4' as Collation, NULL as Checksum, \
             '' as Create_options, '' as Comment FROM sqlite_master WHERE type='table'".to_string()
        );
    }

    // DESC / DESCRIBE -> MySQL column shape
    if upper.starts_with("DESC ") || upper.starts_with("DESCRIBE ") {
        let parts: Vec<&str> = out.split_whitespace().collect();
        if parts.len() >= 2 {
            let t = parts[1].trim_matches('`').trim_end_matches(';');
            let t = safe_table_name(t)?;
            return Ok(format!(
                "SELECT name as Field, type as Type, \
                 CASE \"notnull\" WHEN 1 THEN 'NO' ELSE 'YES' END as \"Null\", \
                 CASE \"pk\" WHEN 1 THEN 'PRI' ELSE '' END as \"Key\", \
                 dflt_value as \"Default\", \
                 '' as Extra \
                 FROM pragma_table_info('{}')",
                t
            ));
        }
    }

    // CREATE PROCEDURE / FUNCTION -> store in _mysql_proc
    if upper.starts_with("CREATE PROCEDURE") || upper.starts_with("CREATE FUNCTION") {
        if let Some(name_start) = upper.find("PROCEDURE") {
            let rest = &out[name_start + 9..];
            if let Some(name) = rest.split(|c: char| c == '(' || c.is_whitespace()).next() {
                let name = name.trim().trim_matches('`');
                let body = out.clone();
                let store_sql = format!(
                    "INSERT OR REPLACE INTO _mysql_proc(name, body) VALUES('{}', '{}')",
                    name.replace("'", "''"),
                    body.replace("'", "''")
                );
                return Ok(store_sql);
            }
        }
    }

    // FOUND_ROWS() → sentinel so shim can substitute cached _found_rows value
    if upper.trim() == "SELECT FOUND_ROWS()" {
        return Ok("SELECT 0 as FOUND_ROWS_SENTINEL".to_string());
    }

    // REGEXP → now handled as a real SQLite UDF (registered in shim.rs)
    // No rewrite needed; pass through to SQLite.

    // Large transient cache INSERTs can exceed mysqlnd's net_cmd_buffer_size.
    // These are non-critical cached values; skip them silently.
    if (upper.starts_with("INSERT") || upper.starts_with("REPLACE"))
        && upper.contains("_SITE_TRANSIENT_")
    {
        return Ok("SELECT 1".to_string());
    }

    // GRANT -> store in _mysql_grants
    if upper.starts_with("GRANT ") {
        return Ok(format!(
            "INSERT OR IGNORE INTO _mysql_grants(rule) VALUES('{}')",
            out.replace("'", "''")
        ));
    }

    // INSERT IGNORE -> INSERT OR IGNORE
    if upper.starts_with("INSERT IGNORE ") {
        out = Regex::new(r"(?i)^INSERT\s+IGNORE\s+").unwrap().replace(&out, "INSERT OR IGNORE ").to_string();
    }

    // REPLACE INTO -> INSERT OR REPLACE INTO
    if upper.starts_with("REPLACE INTO ") || upper.starts_with("REPLACE ") {
        out = Regex::new(r"(?i)^REPLACE\s+INTO\s+").unwrap().replace(&out, "INSERT OR REPLACE INTO ").to_string();
        out = Regex::new(r"(?i)^REPLACE\s+").unwrap().replace(&out, "INSERT OR REPLACE INTO ").to_string();
    }

    // ON DUPLICATE KEY UPDATE -> INSERT ... ON CONFLICT DO UPDATE SET ...
    // Limitation: for known WP tables the conflict column is hardcoded; for unknown tables
    // we fall back to INSERT OR REPLACE (which deletes+reinserts, losing any auto-inc state).
    if let Some(cap) = Regex::new(r"(?i)^(INSERT(?:\s+(?:LOW_PRIORITY|DELAYED|HIGH_PRIORITY|IGNORE))?\s+INTO\s+`?(\w+)`?\s.*?)\s+ON\s+DUPLICATE\s+KEY\s+UPDATE\s+(.+)$")
        .unwrap()
        .captures(&out)
    {
        let insert_part = cap.get(1).unwrap().as_str();
        let table = cap.get(2).unwrap().as_str().to_lowercase();
        let set_part = cap.get(3).unwrap().as_str();

        // Known WP tables -> their UNIQUE/PRIMARY conflict column
        let conflict_col = wp_conflict_column(&table);

        // Translate VALUES(col) and VALUES(`col`) references to excluded.col
        let set_sqlite = Regex::new(r"(?i)VALUES\s*\(\s*`?(\w+)`?\s*\)")
            .unwrap()
            .replace_all(set_part, "excluded.$1")
            .to_string();

        if let Some(col) = conflict_col {
            out = format!("{} ON CONFLICT({}) DO UPDATE SET {}", insert_part, col, set_sqlite);
        } else {
            // Unknown table: rewrite INSERT INTO -> INSERT OR REPLACE INTO as safe fallback
            let replaced = Regex::new(r"(?i)^INSERT\s+INTO\s+")
                .unwrap()
                .replace(insert_part, "INSERT OR REPLACE INTO ")
                .to_string();
            out = replaced;
        }
    }

    // LOCK TABLES -> BEGIN
    if upper.starts_with("LOCK TABLES") || upper.starts_with("LOCK TABLE") {
        return Ok("BEGIN".to_string());
    }

    // UNLOCK TABLES -> COMMIT
    if upper.starts_with("UNLOCK TABLES") || upper.starts_with("UNLOCK TABLE") {
        return Ok("COMMIT".to_string());
    }

    // TRUNCATE TABLE -> DELETE FROM
    if upper.starts_with("TRUNCATE TABLE ") {
        if let Some(t) = out.split_whitespace().nth(2) {
            let t = t.trim_matches('`');
            return Ok(format!("DELETE FROM {}", t));
        }
    }

    // SQL_CALC_FOUND_ROWS → rewrite to window function so WP can read total count
    // SELECT SQL_CALC_FOUND_ROWS ... → SELECT *, COUNT(*) OVER() as _found_rows ...
    if Regex::new(r"(?i)\bSQL_CALC_FOUND_ROWS\b").unwrap().is_match(&out) {
        // Strip the SQL_CALC_FOUND_ROWS hint
        out = Regex::new(r"(?i)\bSQL_CALC_FOUND_ROWS\b\s*").unwrap().replace(&out, "").to_string();
        // Inject COUNT(*) OVER() as _found_rows after SELECT keyword
        out = Regex::new(r"(?i)^(\s*SELECT\s+)")
            .unwrap()
            .replace(&out, "${1}COUNT(*) OVER() as _found_rows, ")
            .to_string();
    }

    // FORCE INDEX(...) / USE INDEX(...) / IGNORE INDEX(...) -> strip (SQLite has no index hints)
    out = Regex::new(r"(?i)\b(?:FORCE|USE|IGNORE)\s+INDEX\s*\([^)]*\)\s*").unwrap().replace_all(&out, "").to_string();

    // TiDB clustered_index hint -> strip (/*T![clustered_index] CLUSTERED */)
    out = Regex::new(r"(?i)/\*T!\[clustered_index\][^*]*\*/\s*").unwrap().replace_all(&out, "").to_string();

    // General MySQL -> SQLite rewrites (case-insensitive)
    out = out.replace("`", "\"");
    // All MySQL integer types with size -> INTEGER
    out = Regex::new(r"(?i)(BIGINT|SMALLINT|TINYINT|MEDIUMINT|INT)\(\d+\)").unwrap().replace_all(&out, "INTEGER").to_string();
    out = Regex::new(r"(?i)\bUNSIGNED\b").unwrap().replace_all(&out, "").to_string();
    out = Regex::new(r"(?i)ENGINE\s*=\s*\w+").unwrap().replace_all(&out, "").to_string();
    out = Regex::new(r"(?i)DEFAULT\s+CHARSET\s*(?:=\s*)?\w+").unwrap().replace_all(&out, "").to_string();
    out = Regex::new(r"(?i)DEFAULT\s+CHARACTER\s+SET\s*(?:=\s*)?\w+").unwrap().replace_all(&out, "").to_string();
    out = Regex::new(r"(?i)COLLATE\s*(?:=\s*)?\w+").unwrap().replace_all(&out, "").to_string();
    out = Regex::new(r"(?i)COMMENT\s+'[^']*'").unwrap().replace_all(&out, "").to_string();
    // MySQL text variants -> TEXT (SQLite handles them, but normalise for cleanliness)
    out = Regex::new(r"(?i)\b(LONGTEXT|MEDIUMTEXT|TINYTEXT)\b").unwrap().replace_all(&out, "TEXT").to_string();

    // SQLite AUTOINCREMENT requires INTEGER PRIMARY KEY.
    // Match: col <any-int-type> [NOT NULL] AUTO_INCREMENT -> col INTEGER PRIMARY KEY AUTOINCREMENT
    out = Regex::new(r"(?i)(\w+)\s+(?:BIGINT|SMALLINT|TINYINT|MEDIUMINT|INT|INTEGER)\s+(?:NOT\s+NULL\s+)?AUTO_INCREMENT").unwrap().replace_all(&out, "$1 INTEGER PRIMARY KEY AUTOINCREMENT").to_string();

    // Handle KEY/UNIQUE KEY inside CREATE TABLE by stripping inline index defs
    // and removing redundant PRIMARY KEY (col) when an AUTOINCREMENT column already implies PK.
    if upper.starts_with("CREATE TABLE") {
        let re_trailing_comma = Regex::new(r",\s*$").unwrap();
        let re_pk_line = Regex::new(r"(?i)^\s*PRIMARY\s+KEY\s+\(\s*\w+\s*\)\s*,?\s*$").unwrap();
        let re_unique_key = Regex::new(r#"(?i)\bUNIQUE\s+KEY\s+(?:"[^"]+"|\w+)\s*(\([^)]+\))"#).unwrap();
        let mut cleaned: Vec<String> = Vec::new();
        let mut has_autoincrement = false;
        for line in out.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            let trimmed_upper = trimmed.to_uppercase();
            if trimmed_upper.starts_with("KEY ") || trimmed_upper.starts_with("INDEX ") || trimmed_upper.starts_with("FULLTEXT ") {
                // Remove trailing comma from previous line to avoid syntax error
                if let Some(last) = cleaned.last_mut() {
                    *last = re_trailing_comma.replace(last, "").to_string();
                }
                continue;
            }
            if trimmed_upper.contains("AUTOINCREMENT") {
                has_autoincrement = true;
            }
            // Strip standalone PRIMARY KEY (single_col) when AUTOINCREMENT already implies PK
            if has_autoincrement && re_pk_line.is_match(line) {
                if let Some(last) = cleaned.last_mut() {
                    *last = re_trailing_comma.replace(last, "").to_string();
                }
                continue;
            }
            // UNIQUE KEY name (cols) -> UNIQUE (cols)  (strip constraint name, SQLite syntax)
            let fixed = re_unique_key.replace(line, "UNIQUE $1");
            cleaned.push(fixed.to_string());
        }
        // Fix commas: every body line must end with comma except the last before ')'
        for i in 0..cleaned.len().saturating_sub(1) {
            let curr = cleaned[i].trim();
            let next = cleaned[i + 1].trim();
            if curr.is_empty() || curr.to_uppercase().starts_with("CREATE TABLE") || next.is_empty() {
                continue;
            }
            if next.starts_with(')') {
                // Last body line: no trailing comma
                if curr.ends_with(',') {
                    cleaned[i] = cleaned[i].trim_end_matches(',').to_string();
                }
            } else {
                // Body line followed by another body line: must have trailing comma
                if !curr.ends_with(',') {
                    cleaned[i] = format!("{},", cleaned[i]);
                }
            }
        }
        out = cleaned.join("\n");
    }

    // ALTER TABLE rewrites
    if upper.starts_with("ALTER TABLE") {
        // DROP COLUMN is supported since SQLite 3.35 (bundled should be new enough)
        // ADD COLUMN is supported
        // MODIFY COLUMN is NOT supported -> no-op
        if upper.contains("MODIFY COLUMN") || upper.contains("CHANGE COLUMN") || upper.contains("ALTER COLUMN") {
            return Ok("SELECT 1".to_string());
        }
        // DROP INDEX inside ALTER TABLE -> DROP INDEX
        if upper.contains("DROP INDEX") {
            if let Some(cap) = Regex::new(r"(?i)DROP\s+INDEX\s+(\w+)").unwrap().captures(&out) {
                let idx = cap.get(1).unwrap().as_str();
                return Ok(format!("DROP INDEX IF EXISTS {}", idx));
            }
        }
        // ADD INDEX / ADD UNIQUE INDEX -> CREATE INDEX
        if upper.contains("ADD INDEX") {
            if let Some(cap) = Regex::new(r"(?i)ADD\s+INDEX\s+(\w+)\s*\(([^)]+)\)").unwrap().captures(&out) {
                let idx = cap.get(1).unwrap().as_str();
                let cols = cap.get(2).unwrap().as_str();
                let table = out.split_whitespace().nth(2).unwrap_or("").trim_matches('`');
                return Ok(format!("CREATE INDEX IF NOT EXISTS {} ON {} ({})", idx, table, cols));
            }
        }
        if upper.contains("ADD UNIQUE INDEX") {
            if let Some(cap) = Regex::new(r"(?i)ADD\s+UNIQUE\s+INDEX\s+(\w+)\s*\(([^)]+)\)").unwrap().captures(&out) {
                let idx = cap.get(1).unwrap().as_str();
                let cols = cap.get(2).unwrap().as_str();
                let table = out.split_whitespace().nth(2).unwrap_or("").trim_matches('`');
                return Ok(format!("CREATE UNIQUE INDEX IF NOT EXISTS {} ON {} ({})", idx, table, cols));
            }
        }
        // ADD FULLTEXT INDEX -> CREATE INDEX (plain, no fulltext in SQLite unless FTS5)
        if upper.contains("ADD FULLTEXT INDEX") {
            if let Some(cap) = Regex::new(r"(?i)ADD\s+FULLTEXT\s+INDEX\s+(\w+)\s*\(([^)]+)\)").unwrap().captures(&out) {
                let idx = cap.get(1).unwrap().as_str();
                let cols = cap.get(2).unwrap().as_str();
                let table = out.split_whitespace().nth(2).unwrap_or("").trim_matches('`');
                return Ok(format!("CREATE INDEX IF NOT EXISTS {} ON {} ({})", idx, table, cols));
            }
        }
    }

    // MySQL string literal backslash-unescape → SQLite doesn't treat \ as escape.
    // MySQL: 'a\"b' stores a"b. SQLite: 'a\"b' stores a\"b (literal backslash+quote).
    // Convert MySQL escape sequences in single-quoted literals to SQLite equivalents.
    out = mysql_unescape_string_literals(&out);

    Ok(out)
}

/// Convert MySQL-style backslash escape sequences inside single-quoted string
/// literals to their actual characters, since SQLite does not interpret `\` as
/// an escape character in string literals.
///
/// Handles: \" → " | \\ → \ | \n → newline | \r → cr | \t → tab | \0 → NUL
/// Leaves other backslash sequences unchanged (conservative).
fn mysql_unescape_string_literals(sql: &str) -> String {
    let mut out = String::with_capacity(sql.len());
    let chars: Vec<char> = sql.chars().collect();
    let len = chars.len();
    let mut i = 0;
    let mut in_str = false;

    while i < len {
        let c = chars[i];
        if !in_str {
            if c == '\'' {
                in_str = true;
                out.push(c);
                i += 1;
            } else {
                out.push(c);
                i += 1;
            }
        } else {
            // Inside single-quoted string
            if c == '\'' {
                // Check for SQL ''-escape
                if i + 1 < len && chars[i + 1] == '\'' {
                    out.push('\'');
                    out.push('\'');
                    i += 2;
                } else {
                    // End of string
                    in_str = false;
                    out.push(c);
                    i += 1;
                }
            } else if c == '\\' && i + 1 < len {
                // MySQL backslash escape
                let next = chars[i + 1];
                match next {
                    '"'  => { out.push('"');  i += 2; }
                    '\'' => { out.push('\''); out.push('\''); i += 2; } // \' → '' for SQLite
                    '\\' => { out.push('\\'); i += 2; }
                    'n'  => { out.push('\n'); i += 2; }
                    'r'  => { out.push('\r'); i += 2; }
                    't'  => { out.push('\t'); i += 2; }
                    '0'  => { out.push('\0'); i += 2; }
                    _    => { out.push(c); i += 1; } // keep unknown escapes unchanged
                }
            } else {
                out.push(c);
                i += 1;
            }
        }
    }
    out
}

/// Returns the primary/unique conflict column for known WordPress tables.
/// Limitation: hardcoded list covering WP core tables only (v6.x schema).
/// For WP multisite or plugin tables, the fallback INSERT OR REPLACE is used.
fn wp_conflict_column(table: &str) -> Option<&'static str> {
    // Strip common prefixes: wp_, wp_2_, wp_3_, etc.
    let bare = Regex::new(r"^wp_(\d+_)?").unwrap().replace(table, "");
    match bare.as_ref() {
        "options"           => Some("option_name"),
        "usermeta"          => Some("umeta_id"),
        "postmeta"          => Some("meta_id"),
        "termmeta"          => Some("meta_id"),
        "commentmeta"       => Some("meta_id"),
        "users"             => Some("user_login"),
        "terms"             => Some("term_id"),
        "term_taxonomy"     => Some("term_taxonomy_id"),
        "links"             => Some("link_id"),
        "site"              => Some("domain"),
        "sitemeta"          => Some("meta_key"),
        _                   => None,
    }
}

/// Returns true if the SQL is a DDL statement that should be `execute`d rather
/// than `query`d (CREATE / DROP / ALTER / TRUNCATE / RENAME / LOCK / UNLOCK /
/// GRANT / ANALYZE).
pub fn is_ddl(sql: &str) -> bool {
    let upper = sql.trim_start().to_ascii_uppercase();
    upper.starts_with("CREATE ")
        || upper.starts_with("DROP ")
        || upper.starts_with("ALTER ")
        || upper.starts_with("TRUNCATE ")
        || upper.starts_with("RENAME ")
        || upper.starts_with("LOCK ")
        || upper.starts_with("UNLOCK ")
        || upper.starts_with("GRANT ")
        || upper.starts_with("ANALYZE ")
}

/// Translate one MySQL DDL statement to one or more SQLite statements.
///
/// For CREATE TABLE this returns the cleaned CREATE TABLE statement plus any
/// extracted `CREATE INDEX` / `CREATE UNIQUE INDEX` statements that were
/// inline `KEY` / `UNIQUE KEY` clauses in the MySQL syntax.
pub fn rewrite_ddl(sql: &str) -> Result<Vec<String>> {
    let upper = sql.trim_start().to_ascii_uppercase();

    // CREATE / DROP DATABASE / SCHEMA → no-op
    if upper.starts_with("CREATE DATABASE")
        || upper.starts_with("CREATE SCHEMA")
        || upper.starts_with("DROP DATABASE")
        || upper.starts_with("DROP SCHEMA")
    {
        return Ok(vec!["SELECT 1".to_string()]);
    }

    if upper.starts_with("CREATE TABLE") {
        return rewrite_create_table(sql);
    }

    Ok(vec![rewrite(sql, "")?])
}

fn rewrite_create_table(sql: &str) -> Result<Vec<String>> {
    let mut s = sql.replace('`', "\"");
    s = normalise_create_table_body(&s);

    // Strip table options after the closing `)`
    s = Regex::new(r"(?is)\)\s*(ENGINE\s*=.*|DEFAULT\s+CHARSET\s*=.*|DEFAULT\s+CHARACTER\s+SET.*|COLLATE\s*=.*|ROW_FORMAT\s*=.*|AUTO_INCREMENT\s*=.*|PACK_KEYS\s*=.*|COMMENT\s*=.*|/\*!.*?\*/.*)*\s*;?\s*$")
        .unwrap()
        .replace(&s, ")")
        .to_string();

    let table_name = Regex::new(r#"(?i)CREATE\s+TABLE\s+(?:IF\s+NOT\s+EXISTS\s+)?"?(\w+)"?"#)
        .unwrap()
        .captures(&s)
        .and_then(|c| c.get(1).map(|m| m.as_str().to_string()))
        .unwrap_or_else(|| "unknown_table".to_string());

    // Strip widths on integer types: BIGINT(20) → BIGINT, INT(11) → INT
    s = Regex::new(r"(?i)(BIGINT|SMALLINT|TINYINT|MEDIUMINT|INT)\s*\(\s*\d+\s*\)")
        .unwrap()
        .replace_all(&s, "$1")
        .to_string();
    s = Regex::new(r"(?i)\b(BIGINT|SMALLINT|TINYINT|MEDIUMINT|INT)\b")
        .unwrap()
        .replace_all(&s, "INTEGER")
        .to_string();
    s = Regex::new(r"(?i)\bUNSIGNED\b").unwrap().replace_all(&s, "").to_string();
    s = Regex::new(r"(?i)\bZEROFILL\b").unwrap().replace_all(&s, "").to_string();
    s = Regex::new(r"(?i)\b(LONGTEXT|MEDIUMTEXT|TINYTEXT)\b")
        .unwrap()
        .replace_all(&s, "TEXT")
        .to_string();
    s = Regex::new(r"(?i)\benum\s*\([^)]*\)").unwrap().replace_all(&s, "TEXT").to_string();
    s = Regex::new(r"(?i)\bset\s*\([^)]*\)").unwrap().replace_all(&s, "TEXT").to_string();
    s = Regex::new(r"(?i)\bCHARACTER\s+SET\s+\w+").unwrap().replace_all(&s, "").to_string();
    s = Regex::new(r"(?i)\bCOLLATE\s+\w+").unwrap().replace_all(&s, "").to_string();
    s = Regex::new(r"(?i)\bCOMMENT\s+'(?:[^'\\]|\\.|'')*'").unwrap().replace_all(&s, "").to_string();
    s = Regex::new(r"(?i)\bON\s+UPDATE\s+CURRENT_TIMESTAMP(?:\(\))?").unwrap().replace_all(&s, "").to_string();

    // AUTO_INCREMENT → INTEGER PRIMARY KEY AUTOINCREMENT
    s = Regex::new(r#"(?i)("?\w+"?)\s+INTEGER\s+(?:NOT\s+NULL\s+)?AUTO_INCREMENT"#)
        .unwrap()
        .replace_all(&s, "$1 INTEGER PRIMARY KEY AUTOINCREMENT")
        .to_string();

    fn extract_paren_body(s: &str) -> Option<(String, usize)> {
        let bytes = s.as_bytes();
        let start = bytes.iter().position(|&b| b == b'(')?;
        let mut depth = 0i32;
        for (i, &b) in bytes.iter().enumerate().skip(start) {
            if b == b'(' { depth += 1; }
            else if b == b')' { depth -= 1; if depth == 0 { return Some((s[start+1..i].to_string(), i)); } }
        }
        None
    }

    let re_unique_idx = Regex::new(r#"(?i)^\s*UNIQUE\s+(?:KEY|INDEX)\s+"?(\w+)"?\s*\("#).unwrap();
    let re_plain_idx = Regex::new(r#"(?i)^\s*(?:FULLTEXT\s+|SPATIAL\s+)?(?:KEY|INDEX)\s+"?(\w+)"?\s*\("#).unwrap();
    let re_pk_line = Regex::new(r#"(?i)^\s*PRIMARY\s+KEY\s+\(\s*"?\w+"?\s*\)\s*,?\s*$"#).unwrap();
    let re_trailing_comma = Regex::new(r",\s*$").unwrap();
    let mut extracted: Vec<String> = Vec::new();
    let mut cleaned_lines: Vec<String> = Vec::new();
    let mut has_autoincrement = false;
    for line in s.lines() {
        let trimmed = line.trim();
        let trimmed_clean = trimmed.trim_end_matches(',').trim();
        let upper_l = trimmed_clean.to_ascii_uppercase();

        if let Some(cap) = re_unique_idx.captures(trimmed_clean) {
            let idx = cap.get(1).unwrap().as_str().to_string();
            if let Some((cols, _)) = extract_paren_body(trimmed_clean) {
                extracted.push(format!(
                    "CREATE UNIQUE INDEX IF NOT EXISTS \"{}_{}\" ON \"{}\" ({})",
                    table_name, idx, table_name, strip_index_lengths(&cols)
                ));
                if let Some(last) = cleaned_lines.last_mut() {
                    *last = re_trailing_comma.replace(last, "").to_string();
                }
                continue;
            }
        }
        if !upper_l.starts_with("PRIMARY ") {
            if let Some(cap) = re_plain_idx.captures(trimmed_clean) {
                let idx = cap.get(1).unwrap().as_str().to_string();
                if let Some((cols, _)) = extract_paren_body(trimmed_clean) {
                    extracted.push(format!(
                        "CREATE INDEX IF NOT EXISTS \"{}_{}\" ON \"{}\" ({})",
                        table_name, idx, table_name, strip_index_lengths(&cols)
                    ));
                    if let Some(last) = cleaned_lines.last_mut() {
                        *last = re_trailing_comma.replace(last, "").to_string();
                    }
                    continue;
                }
            }
        }
        if upper_l.contains("AUTOINCREMENT") {
            has_autoincrement = true;
        }
        if has_autoincrement && re_pk_line.is_match(line) {
            if let Some(last) = cleaned_lines.last_mut() {
                *last = re_trailing_comma.replace(last, "").to_string();
            }
            continue;
        }
        cleaned_lines.push(line.to_string());
    }

    // Re-balance commas: drop trailing comma on last body line
    for i in 0..cleaned_lines.len().saturating_sub(1) {
        let curr = cleaned_lines[i].trim();
        let next = cleaned_lines[i + 1].trim();
        if curr.is_empty() || next.is_empty() { continue; }
        if curr.to_ascii_uppercase().starts_with("CREATE TABLE") { continue; }
        if next.starts_with(')') && curr.ends_with(',') {
            cleaned_lines[i] = cleaned_lines[i].trim_end_matches(',').to_string();
        }
    }

    let mut create_table_sql = cleaned_lines.join("\n");
    create_table_sql = create_table_sql.trim_end_matches(';').trim().to_string();

    let mut out = vec![create_table_sql];
    out.extend(extracted);
    Ok(out)
}

fn strip_index_lengths(cols: &str) -> String {
    Regex::new(r"\(\s*\d+\s*\)").unwrap().replace_all(cols, "").to_string()
}

/// Reflow a CREATE TABLE so each top-level comma item is on its own line.
fn normalise_create_table_body(sql: &str) -> String {
    let bytes = sql.as_bytes();
    let open = match bytes.iter().position(|&b| b == b'(') {
        Some(p) => p,
        None => return sql.to_string(),
    };
    let mut depth = 0i32;
    let mut in_s = false;
    let mut in_d = false;
    let mut close = open;
    for (i, &b) in bytes.iter().enumerate().skip(open) {
        if in_s { if b == b'\'' && bytes.get(i+1) != Some(&b'\'') { in_s = false; } continue; }
        if in_d { if b == b'"' { in_d = false; } continue; }
        match b {
            b'\'' => in_s = true,
            b'"' => in_d = true,
            b'(' => depth += 1,
            b')' => { depth -= 1; if depth == 0 { close = i; break; } }
            _ => {}
        }
    }
    if close <= open { return sql.to_string(); }
    let prefix = &sql[..=open];
    let body = &sql[open+1..close];
    let suffix = &sql[close..];

    let mut items: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut depth = 0i32;
    let mut in_s = false;
    let mut in_d = false;
    for c in body.chars() {
        if in_s { cur.push(c); if c == '\'' { in_s = false; } continue; }
        if in_d { cur.push(c); if c == '"' { in_d = false; } continue; }
        match c {
            '\'' => { in_s = true; cur.push(c); }
            '"' => { in_d = true; cur.push(c); }
            '(' => { depth += 1; cur.push(c); }
            ')' => { depth -= 1; cur.push(c); }
            ',' if depth == 0 => { items.push(cur.trim().to_string()); cur.clear(); }
            _ => cur.push(c),
        }
    }
    if !cur.trim().is_empty() { items.push(cur.trim().to_string()); }
    let joined = items.join(",\n");
    format!("{}\n{}\n{}", prefix, joined, suffix)
}

#[cfg(test)]
mod tests {
    use super::rewrite;
    use super::{is_ddl, rewrite_ddl};

    fn rw(sql: &str) -> String {
        rewrite(sql, "").expect("rewrite failed")
    }

    #[test]
    fn test_sql_calc_found_rows_rewrite() {
        let out = rw("SELECT SQL_CALC_FOUND_ROWS * FROM x LIMIT 5");
        assert!(out.contains("COUNT(*) OVER()"), "got: {out}");
        assert!(!out.contains("SQL_CALC_FOUND_ROWS"), "got: {out}");
    }

    #[test]
    fn test_found_rows_sentinel() {
        let out = rw("SELECT FOUND_ROWS()");
        assert!(out.contains("FOUND_ROWS_SENTINEL"), "got: {out}");
    }

    // Basic single-column ON DUPLICATE KEY UPDATE
    #[test]
    fn test_on_duplicate_known_table_single_set() {
        let sql = "INSERT INTO wp_options (option_name, option_value, autoload) VALUES ('siteurl', 'http://localhost', 'yes') ON DUPLICATE KEY UPDATE option_value = VALUES(option_value)";
        let out = rw(sql);
        assert!(out.contains("ON CONFLICT(option_name) DO UPDATE SET"), "got: {out}");
        assert!(out.contains("excluded.option_value"), "got: {out}");
        assert!(!out.contains("ON DUPLICATE KEY"), "got: {out}");
    }

    // Multiple SET columns
    #[test]
    fn test_on_duplicate_known_table_multi_set() {
        let sql = "INSERT INTO `wp_options` (`option_name`,`option_value`,`autoload`) VALUES ('blogname','Test Site','yes') ON DUPLICATE KEY UPDATE `option_value` = VALUES(`option_value`), `autoload` = VALUES(`autoload`)";
        let out = rw(sql);
        assert!(out.contains("ON CONFLICT(option_name) DO UPDATE SET"), "got: {out}");
        assert!(!out.contains("ON DUPLICATE KEY"), "got: {out}");
    }

    // Unknown table -> INSERT OR REPLACE fallback
    #[test]
    fn test_on_duplicate_unknown_table_fallback() {
        let sql = "INSERT INTO wp_some_plugin_table (id, val) VALUES (1, 'x') ON DUPLICATE KEY UPDATE val = VALUES(val)";
        let out = rw(sql);
        assert!(out.to_uppercase().contains("INSERT OR REPLACE INTO"), "got: {out}");
        assert!(!out.contains("ON DUPLICATE KEY"), "got: {out}");
    }

    // VALUES() function reference is translated to excluded.col
    #[test]
    fn test_values_func_reference_translated() {
        let sql = "INSERT INTO wp_usermeta (umeta_id, user_id, meta_key, meta_value) VALUES (NULL, 1, 'session_tokens', 'abc') ON DUPLICATE KEY UPDATE meta_value = VALUES(meta_value)";
        let out = rw(sql);
        assert!(out.contains("excluded.meta_value"), "got: {out}");
    }

    // Literal assignment (no VALUES()) passes through unchanged
    #[test]
    fn test_on_duplicate_literal_assignment() {
        let sql = "INSERT INTO wp_options (option_name, option_value) VALUES ('active_plugins', 'a:0:{}') ON DUPLICATE KEY UPDATE option_value = 'a:0:{}'";
        let out = rw(sql);
        assert!(out.contains("ON CONFLICT(option_name) DO UPDATE SET"), "got: {out}");
        assert!(!out.contains("ON DUPLICATE KEY"), "got: {out}");
    }

    // wp_ prefix variants (multisite: wp_2_options)
    #[test]
    fn test_multisite_prefixed_table() {
        let sql = "INSERT INTO wp_2_options (option_name, option_value) VALUES ('siteurl', 'http://x') ON DUPLICATE KEY UPDATE option_value = VALUES(option_value)";
        let out = rw(sql);
        assert!(out.contains("ON CONFLICT(option_name) DO UPDATE SET"), "got: {out}");
    }

    // INSERT IGNORE must not be broken
    #[test]
    fn test_insert_ignore_unaffected() {
        let sql = "INSERT IGNORE INTO wp_options (option_name, option_value) VALUES ('test', '1')";
        let out = rw(sql);
        assert!(out.to_uppercase().contains("INSERT OR IGNORE"), "got: {out}");
        assert!(!out.contains("ON CONFLICT"), "got: {out}");
    }

    #[test]
    fn test_is_ddl() {
        assert!(is_ddl("CREATE DATABASE brain"));
        assert!(is_ddl("CREATE TABLE x (id INT)"));
        assert!(is_ddl("ALTER TABLE x ADD COLUMN y INT"));
        assert!(!is_ddl("SELECT 1"));
        assert!(!is_ddl("INSERT INTO x VALUES(1)"));
    }

    #[test]
    fn test_create_database_noop() {
        assert_eq!(rewrite_ddl("CREATE DATABASE brain").unwrap(), vec!["SELECT 1"]);
        assert_eq!(rewrite_ddl("CREATE DATABASE IF NOT EXISTS brain").unwrap(), vec!["SELECT 1"]);
        assert_eq!(rewrite_ddl("CREATE SCHEMA `brain`").unwrap(), vec!["SELECT 1"]);
        assert_eq!(rewrite_ddl("DROP DATABASE brain").unwrap(), vec!["SELECT 1"]);
    }

    fn assert_ct_ok(stmts: &[String]) {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        for s in stmts {
            conn.execute_batch(s).unwrap_or_else(|e| panic!("SQLite rejected: {s}\nerr: {e}"));
        }
    }

    #[test]
    fn test_wp_posts() {
        let ddl = "CREATE TABLE wp_posts (\nID bigint(20) unsigned NOT NULL auto_increment,\npost_author bigint(20) unsigned NOT NULL default '0',\npost_date datetime NOT NULL default '0000-00-00 00:00:00',\npost_content longtext NOT NULL,\npost_title text NOT NULL,\npost_status varchar(20) NOT NULL default 'publish',\npost_name varchar(200) NOT NULL default '',\npost_modified datetime NOT NULL default '0000-00-00 00:00:00',\npost_parent bigint(20) unsigned NOT NULL default '0',\nguid varchar(255) NOT NULL default '',\nmenu_order int(11) NOT NULL default '0',\npost_type varchar(20) NOT NULL default 'post',\npost_mime_type varchar(100) NOT NULL default '',\ncomment_count bigint(20) NOT NULL default '0',\nPRIMARY KEY  (ID),\nKEY post_name (post_name(191)),\nKEY type_status_date (post_type,post_status,post_date,ID),\nKEY post_parent (post_parent),\nKEY post_author (post_author)\n) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4";
        let out = rewrite_ddl(ddl).unwrap();
        assert!(out.len() >= 5, "expected table + 4 indexes, got {}: {:?}", out.len(), out);
        assert!(out[0].to_uppercase().contains("AUTOINCREMENT"));
        assert!(!out[0].to_uppercase().contains("ENGINE"));
        assert_ct_ok(&out);
    }

    #[test]
    fn test_wp_options() {
        let ddl = "CREATE TABLE wp_options (\noption_id bigint(20) unsigned NOT NULL auto_increment,\noption_name varchar(191) NOT NULL default '',\noption_value longtext NOT NULL,\nautoload varchar(20) NOT NULL default 'yes',\nPRIMARY KEY  (option_id),\nUNIQUE KEY option_name (option_name),\nKEY autoload (autoload)\n) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4";
        let out = rewrite_ddl(ddl).unwrap();
        assert_ct_ok(&out);
        assert!(out.iter().any(|s| s.contains("UNIQUE INDEX")));
    }

    #[test]
    fn test_wp_users() {
        let ddl = "CREATE TABLE wp_users (\nID bigint(20) unsigned NOT NULL auto_increment,\nuser_login varchar(60) NOT NULL default '',\nuser_pass varchar(255) NOT NULL default '',\nuser_nicename varchar(50) NOT NULL default '',\nuser_email varchar(100) NOT NULL default '',\nuser_url varchar(100) NOT NULL default '',\nuser_registered datetime NOT NULL default '0000-00-00 00:00:00',\nuser_activation_key varchar(255) NOT NULL default '',\nuser_status int(11) NOT NULL default '0',\ndisplay_name varchar(250) NOT NULL default '',\nPRIMARY KEY  (ID),\nKEY user_login_key (user_login),\nKEY user_nicename (user_nicename),\nKEY user_email (user_email)\n) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4";
        assert_ct_ok(&rewrite_ddl(ddl).unwrap());
    }

    #[test]
    fn test_wp_postmeta() {
        let ddl = "CREATE TABLE wp_postmeta (\nmeta_id bigint(20) unsigned NOT NULL auto_increment,\npost_id bigint(20) unsigned NOT NULL default '0',\nmeta_key varchar(255) default NULL,\nmeta_value longtext,\nPRIMARY KEY  (meta_id),\nKEY post_id (post_id),\nKEY meta_key (meta_key(191))\n) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4";
        assert_ct_ok(&rewrite_ddl(ddl).unwrap());
    }

    #[test]
    fn test_wp_meta_tables() {
        for (table, id_col, fk) in &[
            ("wp_usermeta", "umeta_id", "user_id"),
            ("wp_termmeta", "meta_id", "term_id"),
            ("wp_commentmeta", "meta_id", "comment_id"),
        ] {
            let ddl = format!("CREATE TABLE {table} (\n{id_col} bigint(20) unsigned NOT NULL auto_increment,\n{fk} bigint(20) unsigned NOT NULL default '0',\nmeta_key varchar(255) default NULL,\nmeta_value longtext,\nPRIMARY KEY  ({id_col}),\nKEY {fk} ({fk}),\nKEY meta_key (meta_key(191))\n) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4");
            assert_ct_ok(&rewrite_ddl(&ddl).unwrap());
        }
    }

    #[test]
    fn test_wp_terms() {
        let ddl = "CREATE TABLE wp_terms (\nterm_id bigint(20) unsigned NOT NULL auto_increment,\nname varchar(200) NOT NULL default '',\nslug varchar(200) NOT NULL default '',\nterm_group bigint(10) NOT NULL default 0,\nPRIMARY KEY  (term_id),\nKEY slug (slug(191)),\nKEY name (name(191))\n) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4";
        assert_ct_ok(&rewrite_ddl(ddl).unwrap());
    }

    #[test]
    fn test_wp_term_taxonomy() {
        let ddl = "CREATE TABLE wp_term_taxonomy (\nterm_taxonomy_id bigint(20) unsigned NOT NULL auto_increment,\nterm_id bigint(20) unsigned NOT NULL default 0,\ntaxonomy varchar(32) NOT NULL default '',\ndescription longtext NOT NULL,\nparent bigint(20) unsigned NOT NULL default 0,\ncount bigint(20) NOT NULL default 0,\nPRIMARY KEY  (term_taxonomy_id),\nUNIQUE KEY term_id_taxonomy (term_id,taxonomy),\nKEY taxonomy (taxonomy)\n) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4";
        assert_ct_ok(&rewrite_ddl(ddl).unwrap());
    }

    #[test]
    fn test_wp_term_relationships() {
        let ddl = "CREATE TABLE wp_term_relationships (\nobject_id bigint(20) unsigned NOT NULL default 0,\nterm_taxonomy_id bigint(20) unsigned NOT NULL default 0,\nterm_order int(11) NOT NULL default 0,\nPRIMARY KEY  (object_id,term_taxonomy_id),\nKEY term_taxonomy_id (term_taxonomy_id)\n) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4";
        assert_ct_ok(&rewrite_ddl(ddl).unwrap());
    }

    #[test]
    fn test_wp_comments() {
        let ddl = "CREATE TABLE wp_comments (\ncomment_ID bigint(20) unsigned NOT NULL auto_increment,\ncomment_post_ID bigint(20) unsigned NOT NULL default '0',\ncomment_author tinytext NOT NULL,\ncomment_author_email varchar(100) NOT NULL default '',\ncomment_author_url varchar(200) NOT NULL default '',\ncomment_author_IP varchar(100) NOT NULL default '',\ncomment_date datetime NOT NULL default '0000-00-00 00:00:00',\ncomment_date_gmt datetime NOT NULL default '0000-00-00 00:00:00',\ncomment_content text NOT NULL,\ncomment_karma int(11) NOT NULL default '0',\ncomment_approved varchar(20) NOT NULL default '1',\ncomment_agent varchar(255) NOT NULL default '',\ncomment_type varchar(20) NOT NULL default 'comment',\ncomment_parent bigint(20) unsigned NOT NULL default '0',\nuser_id bigint(20) unsigned NOT NULL default '0',\nPRIMARY KEY  (comment_ID),\nKEY comment_post_ID (comment_post_ID),\nKEY comment_approved_date_gmt (comment_approved,comment_date_gmt),\nKEY comment_date_gmt (comment_date_gmt),\nKEY comment_parent (comment_parent),\nKEY comment_author_email (comment_author_email(10))\n) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4";
        assert_ct_ok(&rewrite_ddl(ddl).unwrap());
    }

    #[test]
    fn test_wp_links() {
        let ddl = "CREATE TABLE wp_links (\nlink_id bigint(20) unsigned NOT NULL auto_increment,\nlink_url varchar(255) NOT NULL default '',\nlink_name varchar(255) NOT NULL default '',\nlink_image varchar(255) NOT NULL default '',\nlink_target varchar(25) NOT NULL default '',\nlink_description varchar(255) NOT NULL default '',\nlink_visible varchar(20) NOT NULL default 'Y',\nlink_owner bigint(20) unsigned NOT NULL default '1',\nlink_rating int(11) NOT NULL default '0',\nlink_updated datetime NOT NULL default '0000-00-00 00:00:00',\nlink_rel varchar(255) NOT NULL default '',\nlink_notes mediumtext NOT NULL,\nlink_rss varchar(255) NOT NULL default '',\nPRIMARY KEY  (link_id),\nKEY link_visible (link_visible)\n) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4";
        assert_ct_ok(&rewrite_ddl(ddl).unwrap());
    }

    #[test]
    fn test_alter_add_index() {
        let out = rw("ALTER TABLE wp_posts ADD INDEX idx_status (post_status)");
        assert!(out.to_uppercase().contains("CREATE INDEX"), "got: {out}");
    }

    #[test]
    fn test_safe_table_name_accepts_valid() {
        assert!(super::safe_table_name("wp_posts").is_ok());
        assert!(super::safe_table_name("Table_123").is_ok());
    }

    #[test]
    fn test_safe_table_name_rejects_injection() {
        assert!(super::safe_table_name("evil'); DROP TABLE x; --").is_err());
        assert!(super::safe_table_name("a b").is_err());
        assert!(super::safe_table_name("").is_err());
        assert!(super::safe_table_name(&"x".repeat(65)).is_err());
        assert!(super::safe_table_name("foo;bar").is_err());
    }

    #[test]
    fn test_show_variables_returns_canonical_charset() {
        let out = rw("SHOW VARIABLES LIKE 'character_set_client'");
        // wpdb requires Value = utf8mb4 (or another charset) — never empty.
        assert!(out.contains("'character_set_client'"), "got: {out}");
        assert!(out.contains("'utf8mb4'"), "got: {out}");
        assert!(!out.contains("'' as Value"), "Value must not be empty: {out}");
    }

    #[test]
    fn test_show_variables_glob_expands() {
        let out = rw("SHOW VARIABLES LIKE 'character_set_%'");
        assert!(out.contains("'character_set_client'"), "got: {out}");
        assert!(out.contains("'character_set_connection'"), "got: {out}");
        assert!(out.contains("'character_set_results'"), "got: {out}");
    }

    #[test]
    fn test_show_columns_rejects_injection() {
        let res = rewrite(
            "SHOW COLUMNS FROM \"evil'); DROP TABLE x; --\"",
            "wp",
        );
        assert!(res.is_err(), "expected parse error, got: {:?}", res);
    }

    #[test]
    fn test_show_index_rejects_injection() {
        let res = rewrite("SHOW INDEX FROM `bad'name`", "wp");
        assert!(res.is_err());
    }
}
