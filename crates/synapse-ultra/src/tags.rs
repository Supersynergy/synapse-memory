//! Tag system for Synapse Ultra.
//!
//! Additive layer on top of synapse_events / docs. Provides:
//!   - `tags` — canonical tag dictionary with color + description
//!   - `doc_tags` — many-to-many association between tags and docs/events
//!   - `tag_rules` — auto-tagging rules (keyword → tag) applied on ingest
//!
//! Design goals:
//!   - Idempotent migration (safe to run repeatedly)
//!   - No modification to existing synapse-core schema
//!   - Bulk-tagging, export/import (JSON, CSV), cleanup, merge
//!   - Auto-tag rules fire on event insert via trigger

use crate::UltraResult;
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};

/// A tag definition row.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tag {
    pub id: i64,
    pub name: String,
    pub color: Option<String>,
    pub description: Option<String>,
    pub created_ts: i64,
}

/// A doc/event ↔ tag association.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocTag {
    pub id: i64,
    pub doc_id: i64,
    pub tag_id: i64,
    pub tag_name: String,
    pub source: String,
    pub ts: i64,
}

/// An auto-tag rule: if `keyword` matches in content, apply `tag_name`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TagRule {
    pub id: i64,
    pub keyword: String,
    pub tag_name: String,
    pub enabled: bool,
    pub created_ts: i64,
}

/// Run the idempotent tag-schema migration. Additive only.
pub fn migrate(conn: &Connection) -> UltraResult<()> {
    conn.execute_batch(
        r#"
CREATE TABLE IF NOT EXISTS tags (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    name        TEXT UNIQUE NOT NULL,
    color       TEXT,
    description TEXT,
    created_ts  INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS doc_tags (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    doc_id      INTEGER NOT NULL,
    tag_id      INTEGER NOT NULL,
    source      TEXT NOT NULL DEFAULT 'manual',
    ts          INTEGER NOT NULL,
    UNIQUE (doc_id, tag_id),
    FOREIGN KEY (tag_id) REFERENCES tags(id) ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS idx_doc_tags_doc ON doc_tags(doc_id);
CREATE INDEX IF NOT EXISTS idx_doc_tags_tag ON doc_tags(tag_id);

CREATE TABLE IF NOT EXISTS tag_rules (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    keyword     TEXT NOT NULL,
    tag_name    TEXT NOT NULL,
    enabled     INTEGER NOT NULL DEFAULT 1,
    created_ts  INTEGER NOT NULL,
    UNIQUE (keyword, tag_name)
);
CREATE INDEX IF NOT EXISTS idx_tag_rules_enabled ON tag_rules(enabled);
"#,
    )?;
    Ok(())
}

/// Create a tag. Idempotent — returns existing id if name already present.
pub fn create_tag(
    conn: &Connection,
    name: &str,
    color: Option<&str>,
    description: Option<&str>,
) -> UltraResult<i64> {
    let now = chrono::Utc::now().timestamp();
    conn.execute(
        "INSERT OR IGNORE INTO tags (name, color, description, created_ts) VALUES (?1, ?2, ?3, ?4)",
        params![name, color, description, now],
    )?;
    let id: i64 = conn.query_row(
        "SELECT id FROM tags WHERE name = ?1",
        params![name],
        |row| row.get(0),
    )?;
    Ok(id)
}

/// List all tags.
pub fn list_tags(conn: &Connection) -> UltraResult<Vec<Tag>> {
    let mut stmt =
        conn.prepare("SELECT id, name, color, description, created_ts FROM tags ORDER BY name")?;
    let rows = stmt.query_map([], |row| {
        Ok(Tag {
            id: row.get(0)?,
            name: row.get(1)?,
            color: row.get(2)?,
            description: row.get(3)?,
            created_ts: row.get(4)?,
        })
    })?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r?);
    }
    Ok(out)
}

/// Tag a doc/event. Idempotent. `source` = "manual" | "auto" | "import".
pub fn tag_doc(conn: &Connection, doc_id: i64, tag_name: &str, source: &str) -> UltraResult<()> {
    let now = chrono::Utc::now().timestamp();
    let tag_id = create_tag(conn, tag_name, None, None)?;
    conn.execute(
        "INSERT OR IGNORE INTO doc_tags (doc_id, tag_id, source, ts) VALUES (?1, ?2, ?3, ?4)",
        params![doc_id, tag_id, source, now],
    )?;
    Ok(())
}

/// Bulk-tag multiple docs with one tag.
pub fn bulk_tag(
    conn: &Connection,
    doc_ids: &[i64],
    tag_name: &str,
    source: &str,
) -> UltraResult<usize> {
    let mut count = 0;
    for doc_id in doc_ids {
        tag_doc(conn, *doc_id, tag_name, source)?;
        count += 1;
    }
    Ok(count)
}

/// Remove a tag from a doc.
pub fn untag_doc(conn: &Connection, doc_id: i64, tag_name: &str) -> UltraResult<()> {
    conn.execute(
        "DELETE FROM doc_tags WHERE doc_id = ?1 AND tag_id = (SELECT id FROM tags WHERE name = ?2)",
        params![doc_id, tag_name],
    )?;
    Ok(())
}

/// List all tags applied to a doc.
pub fn tags_for_doc(conn: &Connection, doc_id: i64) -> UltraResult<Vec<DocTag>> {
    let mut stmt = conn.prepare(
        "SELECT dt.id, dt.doc_id, dt.tag_id, t.name, dt.source, dt.ts
         FROM doc_tags dt JOIN tags t ON t.id = dt.tag_id
         WHERE dt.doc_id = ?1 ORDER BY t.name",
    )?;
    let rows = stmt.query_map(params![doc_id], |row| {
        Ok(DocTag {
            id: row.get(0)?,
            doc_id: row.get(1)?,
            tag_id: row.get(2)?,
            tag_name: row.get(3)?,
            source: row.get(4)?,
            ts: row.get(5)?,
        })
    })?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r?);
    }
    Ok(out)
}

/// List all docs with a given tag.
pub fn docs_for_tag(conn: &Connection, tag_name: &str) -> UltraResult<Vec<DocTag>> {
    let mut stmt = conn.prepare(
        "SELECT dt.id, dt.doc_id, dt.tag_id, t.name, dt.source, dt.ts
         FROM doc_tags dt JOIN tags t ON t.id = dt.tag_id
         WHERE t.name = ?1 ORDER BY dt.ts DESC",
    )?;
    let rows = stmt.query_map(params![tag_name], |row| {
        Ok(DocTag {
            id: row.get(0)?,
            doc_id: row.get(1)?,
            tag_id: row.get(2)?,
            tag_name: row.get(3)?,
            source: row.get(4)?,
            ts: row.get(5)?,
        })
    })?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r?);
    }
    Ok(out)
}

/// Add an auto-tag rule.
pub fn add_rule(conn: &Connection, keyword: &str, tag_name: &str) -> UltraResult<i64> {
    let now = chrono::Utc::now().timestamp();
    create_tag(conn, tag_name, None, None)?;
    conn.execute(
        "INSERT OR IGNORE INTO tag_rules (keyword, tag_name, enabled, created_ts) VALUES (?1, ?2, 1, ?3)",
        params![keyword, tag_name, now],
    )?;
    let id: i64 = conn.query_row(
        "SELECT id FROM tag_rules WHERE keyword = ?1 AND tag_name = ?2",
        params![keyword, tag_name],
        |row| row.get(0),
    )?;
    Ok(id)
}

/// List all auto-tag rules.
pub fn list_rules(conn: &Connection) -> UltraResult<Vec<TagRule>> {
    let mut stmt = conn.prepare(
        "SELECT id, keyword, tag_name, enabled, created_ts FROM tag_rules ORDER BY keyword",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok(TagRule {
            id: row.get(0)?,
            keyword: row.get(1)?,
            tag_name: row.get(2)?,
            enabled: row.get::<_, i64>(3)? != 0,
            created_ts: row.get(4)?,
        })
    })?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r?);
    }
    Ok(out)
}

/// Apply auto-tag rules to a single event/doc. Returns tags applied.
pub fn apply_rules(conn: &Connection, doc_id: i64, content: &str) -> UltraResult<Vec<String>> {
    let rules = list_rules(conn)?;
    let mut applied = Vec::new();
    for rule in rules.iter().filter(|r| r.enabled) {
        if content
            .to_lowercase()
            .contains(&rule.keyword.to_lowercase())
        {
            tag_doc(conn, doc_id, &rule.tag_name, "auto")?;
            applied.push(rule.tag_name.clone());
        }
    }
    Ok(applied)
}

/// Merge two tags: all doc_tags pointing to `from_id` are repointed to `to_id`,
/// then `from_id` is deleted.
pub fn merge_tags(conn: &Connection, from_name: &str, into_name: &str) -> UltraResult<usize> {
    let from_id: Option<i64> = conn
        .query_row(
            "SELECT id FROM tags WHERE name = ?1",
            params![from_name],
            |row| row.get(0),
        )
        .ok();
    let Some(from_id) = from_id else {
        return Ok(0);
    };
    let to_id = create_tag(conn, into_name, None, None)?;
    let moved = conn.execute(
        "UPDATE OR IGNORE doc_tags SET tag_id = ?1 WHERE tag_id = ?2 AND doc_id NOT IN (
            SELECT doc_id FROM doc_tags WHERE tag_id = ?1
        )",
        params![to_id, from_id],
    )?;
    conn.execute("DELETE FROM doc_tags WHERE tag_id = ?1", params![from_id])?;
    conn.execute("DELETE FROM tags WHERE id = ?1", params![from_id])?;
    Ok(moved)
}

/// Cleanup: delete tags with no associations.
pub fn cleanup_orphans(conn: &Connection) -> UltraResult<usize> {
    let n = conn.execute(
        "DELETE FROM tags WHERE id NOT IN (SELECT DISTINCT tag_id FROM doc_tags)",
        [],
    )?;
    Ok(n)
}

/// Export all tags + associations as JSON-serializable struct.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TagExport {
    pub tags: Vec<Tag>,
    pub doc_tags: Vec<DocTag>,
    pub rules: Vec<TagRule>,
}

pub fn export(conn: &Connection) -> UltraResult<TagExport> {
    Ok(TagExport {
        tags: list_tags(conn)?,
        doc_tags: {
            let mut stmt = conn.prepare(
                "SELECT dt.id, dt.doc_id, dt.tag_id, t.name, dt.source, dt.ts
                 FROM doc_tags dt JOIN tags t ON t.id = dt.tag_id
                 ORDER BY dt.ts",
            )?;
            let rows = stmt.query_map([], |row| {
                Ok(DocTag {
                    id: row.get(0)?,
                    doc_id: row.get(1)?,
                    tag_id: row.get(2)?,
                    tag_name: row.get(3)?,
                    source: row.get(4)?,
                    ts: row.get(5)?,
                })
            })?;
            let mut out = Vec::new();
            for r in rows {
                out.push(r?);
            }
            out
        },
        rules: list_rules(conn)?,
    })
}

/// Import tags + associations from an export. Idempotent.
pub fn import(conn: &Connection, data: &TagExport) -> UltraResult<usize> {
    let mut count = 0;
    for tag in &data.tags {
        create_tag(
            conn,
            &tag.name,
            tag.color.as_deref(),
            tag.description.as_deref(),
        )?;
        count += 1;
    }
    for rule in &data.rules {
        add_rule(conn, &rule.keyword, &rule.tag_name)?;
        if !rule.enabled {
            conn.execute(
                "UPDATE tag_rules SET enabled = 0 WHERE keyword = ?1 AND tag_name = ?2",
                params![rule.keyword, rule.tag_name],
            )?;
        }
        count += 1;
    }
    for dt in &data.doc_tags {
        tag_doc(conn, dt.doc_id, &dt.tag_name, &dt.source)?;
        count += 1;
    }
    Ok(count)
}

/// Tag statistics for `synapse-ultra tags stats`.
#[derive(Debug, Clone, Serialize)]
pub struct TagStats {
    pub total_tags: i64,
    pub total_associations: i64,
    pub total_rules: i64,
    pub top_tags: Vec<(String, i64)>,
}

pub fn stats(conn: &Connection) -> UltraResult<TagStats> {
    let (total_tags, total_associations, total_rules) = conn.query_row(
        "SELECT (SELECT COUNT(*) FROM tags), (SELECT COUNT(*) FROM doc_tags), (SELECT COUNT(*) FROM tag_rules)",
        [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    let mut stmt = conn.prepare(
        "SELECT t.name, COUNT(dt.id) AS n FROM tags t
         LEFT JOIN doc_tags dt ON dt.tag_id = t.id
         GROUP BY t.id ORDER BY n DESC LIMIT 10",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
    })?;
    let mut top_tags = Vec::new();
    for r in rows {
        top_tags.push(r?);
    }
    Ok(TagStats {
        total_tags,
        total_associations,
        total_rules,
        top_tags,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Ultra;

    fn mem() -> UltraResult<Ultra> {
        let u = Ultra::open_memory()?;
        u.migrate()?;
        let conn = u.pool.get().expect("pool");
        migrate(&conn)?;
        u.pool.put(conn);
        Ok(u)
    }

    #[test]
    fn create_and_list_tags() -> UltraResult<()> {
        let u = mem()?;
        u.with_conn(|c| {
            create_tag(c, "rust", Some("#FF5733"), Some("Rust lang"))?;
            create_tag(c, "memory", None, None)?;
            let tags = list_tags(c)?;
            assert_eq!(tags.len(), 2);
            assert_eq!(tags[0].name, "memory");
            Ok(())
        })
    }

    #[test]
    fn tag_and_untag_doc() -> UltraResult<()> {
        let u = mem()?;
        u.with_conn(|c| {
            tag_doc(c, 1, "rust", "manual")?;
            tag_doc(c, 1, "memory", "auto")?;
            let tags = tags_for_doc(c, 1)?;
            assert_eq!(tags.len(), 2);
            untag_doc(c, 1, "rust")?;
            let tags = tags_for_doc(c, 1)?;
            assert_eq!(tags.len(), 1);
            Ok(())
        })
    }

    #[test]
    fn bulk_tag_works() -> UltraResult<()> {
        let u = mem()?;
        u.with_conn(|c| {
            bulk_tag(c, &[1, 2, 3, 4, 5], "decision", "manual")?;
            let docs = docs_for_tag(c, "decision")?;
            assert_eq!(docs.len(), 5);
            Ok(())
        })
    }

    #[test]
    fn auto_rules_apply() -> UltraResult<()> {
        let u = mem()?;
        u.with_conn(|c| {
            add_rule(c, "refactor", "refactoring")?;
            add_rule(c, "bug", "bugfix")?;
            let applied = apply_rules(c, 42, "refactor the parser, fix bug #123")?;
            assert_eq!(applied.len(), 2);
            let tags = tags_for_doc(c, 42)?;
            assert_eq!(tags.len(), 2);
            Ok(())
        })
    }

    #[test]
    fn merge_tags_repoints() -> UltraResult<()> {
        let u = mem()?;
        u.with_conn(|c| {
            tag_doc(c, 1, "rust-lang", "manual")?;
            tag_doc(c, 2, "rust-lang", "manual")?;
            tag_doc(c, 3, "rust", "manual")?;
            let moved = merge_tags(c, "rust-lang", "rust")?;
            assert_eq!(moved, 2);
            let docs = docs_for_tag(c, "rust")?;
            assert_eq!(docs.len(), 3);
            let docs_old = docs_for_tag(c, "rust-lang")?;
            assert_eq!(docs_old.len(), 0);
            Ok(())
        })
    }

    #[test]
    fn cleanup_orphan_tags() -> UltraResult<()> {
        let u = mem()?;
        u.with_conn(|c| {
            create_tag(c, "lonely", None, None)?;
            tag_doc(c, 1, "used", "manual")?;
            let n = cleanup_orphans(c)?;
            assert_eq!(n, 1);
            let tags = list_tags(c)?;
            assert_eq!(tags.len(), 1);
            assert_eq!(tags[0].name, "used");
            Ok(())
        })
    }

    #[test]
    fn export_import_roundtrip() -> UltraResult<()> {
        let u = mem()?;
        u.with_conn(|c| {
            tag_doc(c, 1, "rust", "manual")?;
            tag_doc(c, 2, "memory", "auto")?;
            add_rule(c, "refactor", "refactoring")?;
            let export_data = export(c)?;
            assert_eq!(export_data.tags.len(), 3); // rust + memory + refactoring (from rule)
            assert_eq!(export_data.doc_tags.len(), 2);
            assert_eq!(export_data.rules.len(), 1);
            // Import into fresh schema
            let u2 = Ultra::open_memory()?;
            u2.migrate()?;
            u2.with_conn(|c2| {
                migrate(c2)?;
                let n = import(c2, &export_data)?;
                assert_eq!(n, 6); // 3 tags + 1 rule + 2 associations
                let tags = list_tags(c2)?;
                assert_eq!(tags.len(), 3);
                Ok(())
            })
        })
    }

    #[test]
    fn tag_stats() -> UltraResult<()> {
        let u = mem()?;
        u.with_conn(|c| {
            tag_doc(c, 1, "rust", "manual")?;
            tag_doc(c, 2, "rust", "manual")?;
            tag_doc(c, 3, "rust", "manual")?;
            tag_doc(c, 1, "memory", "manual")?;
            add_rule(c, "refactor", "refactoring")?;
            let s = stats(c)?;
            assert_eq!(s.total_tags, 3); // rust + memory + refactoring
            assert_eq!(s.total_associations, 4);
            assert_eq!(s.total_rules, 1);
            assert_eq!(s.top_tags[0].0, "rust");
            assert_eq!(s.top_tags[0].1, 3);
            Ok(())
        })
    }
}
