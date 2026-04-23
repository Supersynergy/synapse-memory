use anyhow::Result;
use regex::Regex;

pub fn rewrite(sql: &str, _mode: &str) -> Result<String> {
    let mut out = sql.to_string();
    let upper = out.trim().to_uppercase();

    // Multi-table DELETE (WordPress transient cleanup) -> no-op
    if Regex::new(r"(?i)^DELETE\s+\w+,\s*\w+\s+FROM").unwrap().is_match(&upper) {
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

    // SHOW TABLES
    if upper.starts_with("SHOW TABLES") {
        return Ok("SELECT name as Tables_in_database FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' AND name NOT LIKE '_mysql_%'".to_string());
    }

    // SHOW DATABASES
    if upper.starts_with("SHOW DATABASES") {
        return Ok("SELECT name as Database FROM pragma_database_list()".to_string());
    }

    // SHOW VARIABLES LIKE ... -> dummy (WordPress checks some vars)
    if upper.starts_with("SHOW VARIABLES") {
        if let Some(cap) = Regex::new(r"LIKE\s+'([^']+)'").unwrap().captures(&out) {
            let var = cap.get(1).unwrap().as_str();
            return Ok(format!(
                "SELECT '{}' as Variable_name, '' as Value UNION ALL SELECT 'max_allowed_packet','67108864' UNION ALL SELECT 'sql_mode','NO_ENGINE_SUBSTITUTION'",
                var
            ));
        }
        return Ok("SELECT '' as Variable_name, '' as Value WHERE 0=1".to_string());
    }

    // SHOW CREATE TABLE
    if upper.starts_with("SHOW CREATE TABLE ") {
        if let Some(t) = out.split_whitespace().nth(3) {
            let t = t.trim_matches('`');
            return Ok(format!(
                "SELECT '{}' as Table, ('CREATE TABLE ' || name || '(' || group_concat(name || ' ' || type, ', ') || ')') as CreateTable FROM pragma_table_info('{}')",
                t, t
            ));
        }
    }

    // SHOW FULL COLUMNS FROM / SHOW COLUMNS FROM
    if upper.starts_with("SHOW FULL COLUMNS FROM ") || upper.starts_with("SHOW COLUMNS FROM ") {
        let words: Vec<&str> = out.split_whitespace().collect();
        if let Some(t) = words.last() {
            let t = t.trim_matches('`').trim_matches('\'');
            return Ok(format!("PRAGMA table_info('{}')", t));
        }
    }

    // SHOW INDEX FROM
    if upper.starts_with("SHOW INDEX FROM ") || upper.starts_with("SHOW KEYS FROM ") {
        let words: Vec<&str> = out.split_whitespace().collect();
        if let Some(t) = words.last() {
            let t = t.trim_matches('`').trim_matches('\'');
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

    // DESC / DESCRIBE
    if upper.starts_with("DESC ") || upper.starts_with("DESCRIBE ") {
        let parts: Vec<&str> = out.split_whitespace().collect();
        if parts.len() >= 2 {
            let t = parts[1].trim_matches('`');
            return Ok(format!("PRAGMA table_info('{}')", t));
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

        // Translate VALUES(col) references to excluded.col
        let set_sqlite = Regex::new(r"(?i)VALUES\s*\(\s*(\w+)\s*\)")
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
                    *last = Regex::new(r",\s*$").unwrap().replace(last, "").to_string();
                }
                continue;
            }
            if trimmed_upper.contains("AUTOINCREMENT") {
                has_autoincrement = true;
            }
            // Strip standalone PRIMARY KEY (single_col) when AUTOINCREMENT already implies PK
            if has_autoincrement && Regex::new(r"(?i)^\s*PRIMARY\s+KEY\s+\(\s*\w+\s*\)\s*,?\s*$").unwrap().is_match(line) {
                if let Some(last) = cleaned.last_mut() {
                    *last = Regex::new(r",\s*$").unwrap().replace(last, "").to_string();
                }
                continue;
            }
            // UNIQUE KEY name (cols) -> UNIQUE (cols)  (strip constraint name, SQLite syntax)
            let fixed = Regex::new(r#"(?i)\bUNIQUE\s+KEY\s+(?:"[^"]+"|\w+)\s*(\([^)]+\))"#).unwrap().replace(line, "UNIQUE $1");
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

    Ok(out)
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

#[cfg(test)]
mod tests {
    use super::rewrite;

    fn rw(sql: &str) -> String {
        rewrite(sql, "").expect("rewrite failed")
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
}
