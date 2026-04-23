use anyhow::Result;
use regex::Regex;

pub fn rewrite(sql: &str, _mode: &str) -> Result<String> {
    let mut out = sql.to_string();
    let upper = out.trim().to_uppercase();

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

    // General MySQL -> SQLite rewrites
    out = out.replace("`", "\"");
    out = Regex::new(r"INT\(\d+\)").unwrap().replace_all(&out, "INTEGER").to_string();
    out = Regex::new(r"\bUNSIGNED\b").unwrap().replace_all(&out, "").to_string();
    out = Regex::new(r"\bAUTO_INCREMENT\b").unwrap().replace_all(&out, "AUTOINCREMENT").to_string();
    out = Regex::new(r"ENGINE\s*=\s*\w+").unwrap().replace_all(&out, "").to_string();
    out = Regex::new(r"DEFAULT\s+CHARSET\s*=\s*\w+").unwrap().replace_all(&out, "").to_string();
    out = Regex::new(r"COLLATE\s*=\s*\w+").unwrap().replace_all(&out, "").to_string();
    out = Regex::new(r"COMMENT\s+'[^']*'").unwrap().replace_all(&out, "").to_string();

    // Handle KEY/UNIQUE KEY inside CREATE TABLE by stripping inline index defs
    // SQLite ignores them if placed after column defs, but `KEY idx (col)` causes error.
    // We strip lines containing standalone KEY definitions inside CREATE TABLE.
    if upper.starts_with("CREATE TABLE") {
        let mut cleaned = Vec::new();
        for line in out.lines() {
            let trimmed = line.trim().to_uppercase();
            if trimmed.starts_with("KEY ") || trimmed.starts_with("INDEX ") || trimmed.starts_with("FULLTEXT ") {
                continue;
            }
            // UNIQUE KEY -> UNIQUE
            let fixed = Regex::new(r"(?i)\bUNIQUE\s+KEY\b").unwrap().replace(line, "UNIQUE");
            cleaned.push(fixed.to_string());
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
