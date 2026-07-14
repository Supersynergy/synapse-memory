//! Corpus sidecar for raw knowledge that should not automatically become
//! durable agent memory.

use crate::error::{Error, Result};
use crate::types::EMBED_DIM;
use quick_xml::{Reader, escape::unescape, events::Event};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

type SqliteAutoExtensionFn = unsafe extern "C" fn(
    *mut rusqlite::ffi::sqlite3,
    *mut *mut i8,
    *const rusqlite::ffi::sqlite3_api_routines,
) -> i32;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CorpusSourceKind {
    Rss,
    Youtube,
    Pdf,
    Web,
    Text,
}

#[derive(Debug, Clone)]
pub struct NewCorpusDocument<'a> {
    pub source_kind: CorpusSourceKind,
    pub source_uri: &'a str,
    pub external_id: &'a str,
    pub title: &'a str,
    pub text: &'a str,
    pub published_ts: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CorpusHit {
    pub chunk_id: i64,
    pub document_id: i64,
    pub title: String,
    pub text: String,
    pub score: f64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CorpusSyncSource {
    pub id: i64,
    pub kind: CorpusSourceKind,
    pub uri: String,
    pub title: Option<String>,
    pub sync_interval_secs: i64,
    pub last_sync_ts: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromotionKind {
    Fact,
    Decision,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GoldQuestion {
    pub query: String,
    pub relevant_chunk_ids: Vec<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GoldCandidate {
    pub query: String,
    pub relevant_chunk_ids: Vec<i64>,
    pub title: String,
    pub source_uri: String,
    pub text_preview: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct EvalReport {
    pub recall_at_5: f64,
    pub mrr: f64,
    pub false_recall_rate: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct EvalGateReport {
    pub passed: bool,
    pub baseline: EvalReport,
    pub candidate: EvalReport,
    pub recall_at_5_delta: f64,
    pub mrr_delta: f64,
    pub false_recall_rate_delta: f64,
    pub min_gold: usize,
    pub gold_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvalBootstrap {
    pub candidates: Vec<GoldCandidate>,
    pub gold: Vec<GoldQuestion>,
    pub baseline_rankings: Vec<Vec<i64>>,
    pub baseline: EvalReport,
    pub min_gold: usize,
    pub gold_count: usize,
}

pub trait CorpusReranker {
    fn rerank(&self, query: &str, hits: Vec<CorpusHit>) -> Vec<CorpusHit>;
}

fn source_kind_str(kind: CorpusSourceKind) -> &'static str {
    match kind {
        CorpusSourceKind::Rss => "rss",
        CorpusSourceKind::Youtube => "youtube",
        CorpusSourceKind::Pdf => "pdf",
        CorpusSourceKind::Web => "web",
        CorpusSourceKind::Text => "text",
    }
}

fn source_kind_from_str(raw: &str) -> Result<CorpusSourceKind> {
    match raw {
        "rss" => Ok(CorpusSourceKind::Rss),
        "youtube" => Ok(CorpusSourceKind::Youtube),
        "pdf" => Ok(CorpusSourceKind::Pdf),
        "web" => Ok(CorpusSourceKind::Web),
        "text" => Ok(CorpusSourceKind::Text),
        other => Err(Error::Format(format!(
            "invalid corpus source kind {other:?}"
        ))),
    }
}

fn promotion_kind_str(kind: PromotionKind) -> &'static str {
    match kind {
        PromotionKind::Fact => "fact",
        PromotionKind::Decision => "decision",
    }
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn register_vec_extension() {
    #[allow(clippy::missing_transmute_annotations)]
    unsafe {
        rusqlite::ffi::sqlite3_auto_extension(Some(std::mem::transmute::<
            *const (),
            SqliteAutoExtensionFn,
        >(
            sqlite_vec::sqlite3_vec_init as *const ()
        )));
    }
}

fn ensure_column(conn: &Connection, table: &str, column: &str, definition: &str) -> Result<()> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let columns: Vec<String> = stmt
        .query_map([], |r| r.get::<_, String>(1))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    if !columns.iter().any(|c| c == column) {
        conn.execute(&format!("ALTER TABLE {table} ADD COLUMN {definition}"), [])?;
    }
    Ok(())
}

pub fn corpus_migrate(conn: &Connection) -> Result<()> {
    register_vec_extension();
    conn.execute_batch(&format!(
        r#"
CREATE TABLE IF NOT EXISTS synapse_corpus_sources (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    kind TEXT NOT NULL,
    uri TEXT NOT NULL,
    title TEXT,
    created_ts INTEGER NOT NULL,
    last_sync_ts INTEGER,
    sync_interval_secs INTEGER,
    enabled INTEGER NOT NULL DEFAULT 1,
    UNIQUE(kind, uri)
);

CREATE TABLE IF NOT EXISTS synapse_corpus_documents (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    source_id INTEGER NOT NULL REFERENCES synapse_corpus_sources(id) ON DELETE CASCADE,
    external_id TEXT NOT NULL,
    title TEXT NOT NULL,
    uri TEXT NOT NULL,
    published_ts INTEGER,
    content_hash BLOB NOT NULL,
    created_ts INTEGER NOT NULL,
    UNIQUE(source_id, external_id)
);
CREATE INDEX IF NOT EXISTS idx_synapse_corpus_documents_published
    ON synapse_corpus_documents(COALESCE(published_ts, created_ts), created_ts);

CREATE TABLE IF NOT EXISTS synapse_corpus_chunks (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    document_id INTEGER NOT NULL REFERENCES synapse_corpus_documents(id) ON DELETE CASCADE,
    ordinal INTEGER NOT NULL,
    text TEXT NOT NULL,
    start INTEGER NOT NULL,
    end INTEGER NOT NULL,
    UNIQUE(document_id, ordinal)
);
CREATE INDEX IF NOT EXISTS idx_synapse_corpus_chunks_doc
    ON synapse_corpus_chunks(document_id);

CREATE VIRTUAL TABLE IF NOT EXISTS synapse_corpus_chunks_fts USING fts5(
    text,
    content='synapse_corpus_chunks',
    content_rowid='id',
    tokenize='porter unicode61 remove_diacritics 2'
);

CREATE TRIGGER IF NOT EXISTS synapse_corpus_chunks_ai AFTER INSERT ON synapse_corpus_chunks BEGIN
    INSERT INTO synapse_corpus_chunks_fts(rowid, text) VALUES (new.id, new.text);
END;
CREATE TRIGGER IF NOT EXISTS synapse_corpus_chunks_ad AFTER DELETE ON synapse_corpus_chunks BEGIN
    INSERT INTO synapse_corpus_chunks_fts(synapse_corpus_chunks_fts, rowid, text)
    VALUES('delete', old.id, old.text);
END;
CREATE TRIGGER IF NOT EXISTS synapse_corpus_chunks_au AFTER UPDATE ON synapse_corpus_chunks BEGIN
    INSERT INTO synapse_corpus_chunks_fts(synapse_corpus_chunks_fts, rowid, text)
    VALUES('delete', old.id, old.text);
    INSERT INTO synapse_corpus_chunks_fts(rowid, text) VALUES (new.id, new.text);
END;

CREATE VIRTUAL TABLE IF NOT EXISTS synapse_corpus_vec USING vec0(
    chunk_id INTEGER PRIMARY KEY,
    embedding FLOAT[{dim}]
);

CREATE TABLE IF NOT EXISTS synapse_corpus_promotions (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    chunk_id INTEGER NOT NULL REFERENCES synapse_corpus_chunks(id) ON DELETE CASCADE,
    kind TEXT NOT NULL,
    rationale TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending',
    verifier TEXT,
    created_ts INTEGER NOT NULL,
    verified_ts INTEGER
);
CREATE INDEX IF NOT EXISTS idx_synapse_corpus_promotions_status
    ON synapse_corpus_promotions(status);
"#,
        dim = EMBED_DIM
    ))?;
    ensure_column(
        conn,
        "synapse_corpus_sources",
        "sync_interval_secs",
        "sync_interval_secs INTEGER",
    )?;
    ensure_column(
        conn,
        "synapse_corpus_sources",
        "enabled",
        "enabled INTEGER NOT NULL DEFAULT 1",
    )?;
    Ok(())
}

fn chunks_with_spans(text: &str) -> Vec<(String, usize, usize)> {
    let mut chunks = Vec::new();
    let mut cursor = 0usize;
    for raw in text.split("\n\n") {
        let Some(local_start) = raw.find(|c: char| !c.is_whitespace()) else {
            cursor += raw.len() + 2;
            continue;
        };
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            cursor += raw.len() + 2;
            continue;
        }
        let start = cursor + local_start;
        let end = start + trimmed.len();
        chunks.push((trimmed.to_string(), start, end));
        cursor += raw.len() + 2;
    }
    if chunks.is_empty() {
        let trimmed = text.trim();
        if !trimmed.is_empty() {
            chunks.push((trimmed.to_string(), 0, trimmed.len()));
        }
    }
    chunks
}

pub fn put_corpus_document(conn: &Connection, doc: &NewCorpusDocument<'_>) -> Result<i64> {
    if doc.text.trim().is_empty() {
        return Err(Error::Other("corpus document text is empty".into()));
    }
    corpus_migrate(conn)?;
    let now = now_secs();
    let kind = source_kind_str(doc.source_kind);
    conn.execute(
        "INSERT INTO synapse_corpus_sources(kind, uri, title, created_ts)
         VALUES(?1, ?2, ?3, ?4)
         ON CONFLICT(kind, uri) DO UPDATE SET title=excluded.title",
        params![kind, doc.source_uri, doc.title, now],
    )?;
    let source_id: i64 = conn.query_row(
        "SELECT id FROM synapse_corpus_sources WHERE kind=?1 AND uri=?2",
        params![kind, doc.source_uri],
        |r| r.get(0),
    )?;
    if let Some(existing) = conn
        .query_row(
            "SELECT id FROM synapse_corpus_documents WHERE source_id=?1 AND external_id=?2",
            params![source_id, doc.external_id],
            |r| r.get::<_, i64>(0),
        )
        .optional()?
    {
        return Ok(existing);
    }

    let content_hash = blake3::hash(doc.text.as_bytes());
    conn.execute(
        "INSERT INTO synapse_corpus_documents
         (source_id, external_id, title, uri, published_ts, content_hash, created_ts)
         VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            source_id,
            doc.external_id,
            doc.title,
            doc.source_uri,
            doc.published_ts,
            content_hash.as_bytes().as_slice(),
            now
        ],
    )?;
    let doc_id = conn.last_insert_rowid();
    let mut insert_chunk = conn.prepare(
        "INSERT INTO synapse_corpus_chunks(document_id, ordinal, text, start, end)
         VALUES(?1, ?2, ?3, ?4, ?5)",
    )?;
    for (ordinal, (text, start, end)) in chunks_with_spans(doc.text).into_iter().enumerate() {
        insert_chunk.execute(params![
            doc_id,
            ordinal as i64,
            text,
            start as i64,
            end as i64
        ])?;
    }
    Ok(doc_id)
}

pub fn upsert_corpus_sync_source(
    conn: &Connection,
    kind: CorpusSourceKind,
    uri: &str,
    title: Option<&str>,
    sync_interval_secs: i64,
) -> Result<i64> {
    if sync_interval_secs <= 0 {
        return Err(Error::Other("sync interval must be positive".into()));
    }
    corpus_migrate(conn)?;
    let now = now_secs();
    let kind = source_kind_str(kind);
    conn.execute(
        "INSERT INTO synapse_corpus_sources
         (kind, uri, title, created_ts, sync_interval_secs, enabled)
         VALUES(?1, ?2, ?3, ?4, ?5, 1)
         ON CONFLICT(kind, uri) DO UPDATE SET
             title=excluded.title,
             sync_interval_secs=excluded.sync_interval_secs,
             enabled=1",
        params![kind, uri, title, now, sync_interval_secs],
    )?;
    Ok(conn.query_row(
        "SELECT id FROM synapse_corpus_sources WHERE kind=?1 AND uri=?2",
        params![kind, uri],
        |r| r.get(0),
    )?)
}

pub fn due_corpus_sync_sources(
    conn: &Connection,
    now_ts: i64,
    limit: usize,
) -> Result<Vec<CorpusSyncSource>> {
    if limit == 0 {
        return Ok(Vec::new());
    }
    corpus_migrate(conn)?;
    let mut stmt = conn.prepare(
        "SELECT id, kind, uri, title, sync_interval_secs, last_sync_ts
         FROM synapse_corpus_sources
         WHERE enabled=1
           AND sync_interval_secs IS NOT NULL
           AND (last_sync_ts IS NULL OR last_sync_ts + sync_interval_secs <= ?1)
         ORDER BY COALESCE(last_sync_ts, 0) ASC, id ASC
         LIMIT ?2",
    )?;
    let rows = stmt.query_map(params![now_ts, limit as i64], |r| {
        let kind: String = r.get(1)?;
        Ok((
            r.get::<_, i64>(0)?,
            kind,
            r.get::<_, String>(2)?,
            r.get::<_, Option<String>>(3)?,
            r.get::<_, i64>(4)?,
            r.get::<_, Option<i64>>(5)?,
        ))
    })?;
    let mut out = Vec::new();
    for row in rows {
        let (id, kind, uri, title, sync_interval_secs, last_sync_ts) = row?;
        out.push(CorpusSyncSource {
            id,
            kind: source_kind_from_str(&kind)?,
            uri,
            title,
            sync_interval_secs,
            last_sync_ts,
        });
    }
    Ok(out)
}

pub fn mark_corpus_source_synced(conn: &Connection, source_id: i64, synced_ts: i64) -> Result<()> {
    corpus_migrate(conn)?;
    let changed = conn.execute(
        "UPDATE synapse_corpus_sources SET last_sync_ts=?1 WHERE id=?2",
        params![synced_ts, source_id],
    )?;
    if changed == 0 {
        return Err(Error::NotFound(format!("corpus_source_id={source_id}")));
    }
    Ok(())
}

#[derive(Default)]
struct ParsedRssItem {
    title: String,
    link: String,
    guid: String,
    description: String,
    content: String,
}

impl ParsedRssItem {
    fn into_doc(self, source_uri: &str) -> Option<(String, String, String)> {
        let body = first_non_empty(&[&self.content, &self.description])?;
        let title = first_non_empty(&[&self.title, &self.link, &self.guid])
            .unwrap_or("untitled")
            .to_string();
        let external_id = first_non_empty(&[&self.guid, &self.link])
            .map(str::to_string)
            .unwrap_or_else(|| {
                let hash = blake3::hash(format!("{title}\n{body}").as_bytes());
                format!("{source_uri}#{}", hash)
            });
        let text = if self.title.trim().is_empty() {
            body.to_string()
        } else {
            format!("{}\n\n{}", self.title.trim(), body)
        };
        Some((external_id, title, text))
    }
}

fn first_non_empty<'a>(values: &[&'a str]) -> Option<&'a str> {
    values.iter().map(|v| v.trim()).find(|v| !v.is_empty())
}

fn rss_field_name(raw: &[u8]) -> Option<&'static str> {
    match raw {
        b"title" => Some("title"),
        b"link" => Some("link"),
        b"guid" => Some("guid"),
        b"description" => Some("description"),
        b"content:encoded" | b"encoded" => Some("content"),
        _ => None,
    }
}

fn append_rss_text(item: &mut ParsedRssItem, field: &str, text: &str) {
    let slot = match field {
        "title" => &mut item.title,
        "link" => &mut item.link,
        "guid" => &mut item.guid,
        "description" => &mut item.description,
        "content" => &mut item.content,
        _ => return,
    };
    if !slot.is_empty() && !text.trim().is_empty() {
        slot.push(' ');
    }
    slot.push_str(text.trim());
}

pub fn ingest_rss_xml(conn: &Connection, source_uri: &str, xml: &str) -> Result<Vec<i64>> {
    corpus_migrate(conn)?;
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);
    let mut in_item = false;
    let mut current_field: Option<&'static str> = None;
    let mut current_item = ParsedRssItem::default();
    let mut imported = Vec::new();

    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) => match e.name().as_ref() {
                b"item" => {
                    in_item = true;
                    current_field = None;
                    current_item = ParsedRssItem::default();
                }
                name if in_item => {
                    current_field = rss_field_name(name);
                }
                _ => {}
            },
            Ok(Event::Text(e)) if in_item => {
                if let Some(field) = current_field {
                    let decoded = e
                        .decode()
                        .map_err(|err| Error::Format(format!("rss text decode: {err}")))?;
                    let text = unescape(&decoded)
                        .map_err(|err| Error::Format(format!("rss text unescape: {err}")))?;
                    append_rss_text(&mut current_item, field, &text);
                }
            }
            Ok(Event::CData(e)) if in_item => {
                if let Some(field) = current_field {
                    let text = e
                        .decode()
                        .map_err(|err| Error::Format(format!("rss cdata decode: {err}")))?;
                    append_rss_text(&mut current_item, field, &text);
                }
            }
            Ok(Event::End(e)) => match e.name().as_ref() {
                b"item" if in_item => {
                    if let Some((external_id, title, text)) =
                        std::mem::take(&mut current_item).into_doc(source_uri)
                    {
                        let doc = NewCorpusDocument {
                            source_kind: CorpusSourceKind::Rss,
                            source_uri,
                            external_id: &external_id,
                            title: &title,
                            text: &text,
                            published_ts: None,
                        };
                        imported.push(put_corpus_document(conn, &doc)?);
                    }
                    in_item = false;
                    current_field = None;
                }
                name if in_item && rss_field_name(name) == current_field => {
                    current_field = None;
                }
                _ => {}
            },
            Ok(Event::Eof) => break,
            Err(err) => return Err(Error::Format(format!("rss xml parse: {err}"))),
            _ => {}
        }
    }

    Ok(imported)
}

fn is_transcript_timecode(line: &str) -> bool {
    line.contains("-->")
        || (line.len() >= 8
            && line.as_bytes().get(2) == Some(&b':')
            && line.as_bytes().get(5) == Some(&b':'))
}

fn clean_transcript_text(raw: &str) -> String {
    let mut lines = Vec::new();
    for line in raw.lines().map(str::trim) {
        if line.is_empty()
            || line == "WEBVTT"
            || line.starts_with("NOTE")
            || line.starts_with("STYLE")
            || line.starts_with("Kind:")
            || line.starts_with("Language:")
            || line.chars().all(|c| c.is_ascii_digit())
            || is_transcript_timecode(line)
        {
            continue;
        }
        lines.push(line);
    }
    lines.join("\n\n")
}

fn is_youtube_id(raw: &str) -> bool {
    raw.len() == 11
        && raw
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

fn query_param<'a>(query: &'a str, key: &str) -> Option<&'a str> {
    query.split('&').find_map(|part| {
        let (k, v) = part.split_once('=')?;
        if k == key { Some(v) } else { None }
    })
}

pub fn youtube_video_id(input: &str) -> Option<String> {
    let trimmed = input.trim();
    if is_youtube_id(trimmed) {
        return Some(trimmed.to_string());
    }
    let (_, after_scheme) = trimmed.split_once("://")?;
    let (host, path_query) = after_scheme.split_once('/').unwrap_or((after_scheme, ""));
    let host = host.strip_prefix("www.").unwrap_or(host);
    if host == "youtu.be" {
        let id = path_query.split(['?', '/', '#']).next().unwrap_or_default();
        return is_youtube_id(id).then(|| id.to_string());
    }
    if host == "youtube.com" || host.ends_with(".youtube.com") {
        let query = path_query
            .split_once('?')
            .map(|(_, q)| q)
            .unwrap_or_default();
        if let Some(id) = query_param(query, "v").filter(|id| is_youtube_id(id)) {
            return Some(id.to_string());
        }
        if let Some(rest) = path_query.strip_prefix("shorts/") {
            let id = rest.split(['?', '/', '#']).next().unwrap_or_default();
            return is_youtube_id(id).then(|| id.to_string());
        }
    }
    None
}

pub fn ingest_youtube_transcript(
    conn: &Connection,
    video_uri: &str,
    video_id: &str,
    title: &str,
    transcript: &str,
) -> Result<i64> {
    let text = clean_transcript_text(transcript);
    if text.trim().is_empty() {
        return Err(Error::Other("youtube transcript text is empty".into()));
    }
    let doc = NewCorpusDocument {
        source_kind: CorpusSourceKind::Youtube,
        source_uri: video_uri,
        external_id: video_id,
        title,
        text: &text,
        published_ts: None,
    };
    put_corpus_document(conn, &doc)
}

fn collapse_ws(raw: &str) -> String {
    raw.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn lower_ascii_slice(raw: &str, start: usize, len: usize) -> String {
    raw.get(start..start.saturating_add(len))
        .unwrap_or_default()
        .to_ascii_lowercase()
}

fn extract_title_from_html(html: &str) -> Option<String> {
    let lower = html.to_ascii_lowercase();
    let start = lower.find("<title")?;
    let open_end = lower[start..].find('>')? + start + 1;
    let close = lower[open_end..].find("</title>")? + open_end;
    let raw = html.get(open_end..close)?.trim();
    if raw.is_empty() {
        None
    } else {
        Some(collapse_ws(
            &htmlescape::decode_html(raw).unwrap_or_else(|_| raw.to_string()),
        ))
    }
}

fn html_to_text(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut i = 0usize;
    let mut skip_until: Option<&'static str> = None;
    while i < html.len() {
        let Some(rest) = html.get(i..) else {
            break;
        };
        if let Some(end_tag) = skip_until {
            let lower = rest.to_ascii_lowercase();
            if let Some(end) = lower.find(end_tag) {
                i += end + end_tag.len();
                skip_until = None;
            } else {
                break;
            }
            continue;
        }
        if rest.starts_with("<!--") {
            if let Some(end) = rest.find("-->") {
                i += end + 3;
            } else {
                break;
            }
            out.push(' ');
            continue;
        }
        if rest.starts_with('<') {
            let lower_tag = lower_ascii_slice(html, i, 24);
            if lower_tag.starts_with("<script") {
                skip_until = Some("</script>");
            } else if lower_tag.starts_with("<style") {
                skip_until = Some("</style>");
            }
            if let Some(end) = rest.find('>') {
                i += end + 1;
            } else {
                break;
            }
            out.push(' ');
            continue;
        }
        if let Some(ch) = rest.chars().next() {
            out.push(ch);
            i += ch.len_utf8();
        } else {
            break;
        }
    }
    let decoded = htmlescape::decode_html(&out).unwrap_or(out);
    collapse_ws(&decoded)
}

pub fn ingest_web_html(
    conn: &Connection,
    page_uri: &str,
    title: Option<&str>,
    html: &str,
) -> Result<i64> {
    let text = html_to_text(html);
    if text.trim().is_empty() {
        return Err(Error::Other("web HTML text is empty".into()));
    }
    let fallback_title = extract_title_from_html(html);
    let title = title
        .filter(|t| !t.trim().is_empty())
        .map(str::trim)
        .or(fallback_title.as_deref())
        .unwrap_or(page_uri);
    let doc = NewCorpusDocument {
        source_kind: CorpusSourceKind::Web,
        source_uri: page_uri,
        external_id: page_uri,
        title,
        text: &text,
        published_ts: None,
    };
    put_corpus_document(conn, &doc)
}

#[cfg(feature = "pdf-ingest")]
pub fn ingest_pdf_bytes(
    conn: &Connection,
    pdf_uri: &str,
    title: &str,
    bytes: &[u8],
) -> Result<i64> {
    let text = pdf_extract::extract_text_from_mem(bytes)
        .map_err(|err| Error::Format(format!("pdf text extract: {err}")))?;
    if text.trim().is_empty() {
        return Err(Error::Other("PDF text is empty".into()));
    }
    let doc = NewCorpusDocument {
        source_kind: CorpusSourceKind::Pdf,
        source_uri: pdf_uri,
        external_id: pdf_uri,
        title,
        text: &text,
        published_ts: None,
    };
    put_corpus_document(conn, &doc)
}

#[cfg(not(feature = "pdf-ingest"))]
pub fn ingest_pdf_bytes(
    _conn: &Connection,
    _pdf_uri: &str,
    _title: &str,
    _bytes: &[u8],
) -> Result<i64> {
    Err(Error::Other(
        "PDF ingest is not included in this portable build; use text/web ingest or install a full build"
            .into(),
    ))
}

fn normalized_content_type(content_type: Option<&str>) -> String {
    content_type
        .and_then(|ct| ct.split(';').next())
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase()
}

fn looks_like_rss(bytes: &[u8]) -> bool {
    let prefix = String::from_utf8_lossy(&bytes[..bytes.len().min(512)]).to_ascii_lowercase();
    prefix.contains("<rss") || prefix.contains("<feed")
}

fn looks_like_html(bytes: &[u8]) -> bool {
    let prefix = String::from_utf8_lossy(&bytes[..bytes.len().min(512)]).to_ascii_lowercase();
    prefix.contains("<!doctype html") || prefix.contains("<html")
}

pub fn ingest_fetched_document(
    conn: &Connection,
    uri: &str,
    content_type: Option<&str>,
    title: Option<&str>,
    bytes: &[u8],
) -> Result<Vec<i64>> {
    if bytes.is_empty() {
        return Err(Error::Other("fetched document is empty".into()));
    }
    let content_type = normalized_content_type(content_type);
    let uri_lower = uri.to_ascii_lowercase();

    if content_type.contains("rss")
        || content_type.contains("atom")
        || content_type.ends_with("+xml")
        || (content_type.contains("xml") && looks_like_rss(bytes))
        || looks_like_rss(bytes)
    {
        let xml = std::str::from_utf8(bytes)
            .map_err(|err| Error::Format(format!("fetched RSS is not UTF-8: {err}")))?;
        return ingest_rss_xml(conn, uri, xml);
    }

    if content_type == "application/pdf" || uri_lower.ends_with(".pdf") {
        let id = ingest_pdf_bytes(conn, uri, title.unwrap_or(uri), bytes)?;
        return Ok(vec![id]);
    }

    if content_type == "text/html"
        || content_type == "application/xhtml+xml"
        || looks_like_html(bytes)
    {
        let html = std::str::from_utf8(bytes)
            .map_err(|err| Error::Format(format!("fetched HTML is not UTF-8: {err}")))?;
        let id = ingest_web_html(conn, uri, title, html)?;
        return Ok(vec![id]);
    }

    let text = std::str::from_utf8(bytes)
        .map_err(|err| Error::Format(format!("fetched text is not UTF-8: {err}")))?;
    let doc = NewCorpusDocument {
        source_kind: CorpusSourceKind::Text,
        source_uri: uri,
        external_id: uri,
        title: title.unwrap_or(uri),
        text,
        published_ts: None,
    };
    Ok(vec![put_corpus_document(conn, &doc)?])
}

fn fts5_query(raw: &str) -> Option<String> {
    let tokens: Vec<String> = raw
        .split_whitespace()
        .map(|t| {
            t.chars()
                .filter(|c| c.is_alphanumeric() || *c == '_' || *c == '-')
                .collect::<String>()
                .to_lowercase()
        })
        .filter(|t| t.len() > 1)
        .take(16)
        .collect();
    if tokens.is_empty() {
        None
    } else {
        Some(
            tokens
                .into_iter()
                .map(|t| format!("\"{}\"", t.replace('"', "\"\"")))
                .collect::<Vec<_>>()
                .join(" OR "),
        )
    }
}

fn vec_blob(embedding: &[f32]) -> Result<Vec<u8>> {
    if embedding.len() != EMBED_DIM {
        return Err(Error::DimMismatch {
            expected: EMBED_DIM,
            got: embedding.len(),
        });
    }
    let mut bytes = Vec::with_capacity(embedding.len() * 4);
    for v in embedding {
        bytes.extend_from_slice(&v.to_le_bytes());
    }
    Ok(bytes)
}

fn fetch_hits(
    conn: &Connection,
    ids: &[i64],
    scores: &HashMap<i64, f64>,
) -> Result<Vec<CorpusHit>> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for id in ids {
        if !seen.insert(*id) {
            continue;
        }
        let hit = conn.query_row(
            "SELECT c.id, c.document_id, d.title, c.text
             FROM synapse_corpus_chunks c
             JOIN synapse_corpus_documents d ON d.id = c.document_id
             WHERE c.id=?1",
            [id],
            |r| {
                Ok(CorpusHit {
                    chunk_id: r.get(0)?,
                    document_id: r.get(1)?,
                    title: r.get(2)?,
                    text: r.get(3)?,
                    score: scores.get(id).copied().unwrap_or(0.0),
                })
            },
        )?;
        out.push(hit);
    }
    Ok(out)
}

fn rrf_score(rank: usize) -> f64 {
    1.0 / (60.0 + rank as f64 + 1.0)
}

fn mmr_tokens(text: &str) -> HashSet<String> {
    text.split_whitespace()
        .map(|t| {
            t.chars()
                .filter(|c| c.is_alphanumeric() || *c == '_' || *c == '-')
                .collect::<String>()
                .to_lowercase()
        })
        .filter(|t| t.len() > 1)
        .collect()
}

fn compact_whitespace(raw: &str) -> String {
    raw.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn is_gold_stopword(token: &str) -> bool {
    matches!(
        token,
        "about"
            | "after"
            | "before"
            | "from"
            | "into"
            | "need"
            | "needs"
            | "that"
            | "the"
            | "then"
            | "this"
            | "with"
    )
}

fn gold_candidate_query(title: &str, text: &str, chunk_id: i64) -> String {
    let tokens = text
        .split_whitespace()
        .map(|t| {
            t.chars()
                .filter(|c| c.is_alphanumeric() || *c == '_' || *c == '-')
                .collect::<String>()
                .to_lowercase()
        })
        .filter(|t| t.len() > 2 && !is_gold_stopword(t))
        .take(8)
        .collect::<Vec<_>>();
    if !tokens.is_empty() {
        return tokens.join(" ");
    }

    let compact_title = compact_whitespace(title);
    if !compact_title.is_empty() {
        compact_title
    } else {
        format!("corpus chunk {chunk_id}")
    }
}

fn jaccard(a: &HashSet<String>, b: &HashSet<String>) -> f64 {
    if a.is_empty() && b.is_empty() {
        return 0.0;
    }
    let intersection = a.intersection(b).count() as f64;
    let union = a.union(b).count() as f64;
    if union == 0.0 {
        0.0
    } else {
        intersection / union
    }
}

fn mmr_diversify(hits: Vec<CorpusHit>, top_k: usize) -> Vec<CorpusHit> {
    if hits.len() <= top_k {
        return hits;
    }
    let max_score = hits.iter().map(|h| h.score).fold(0.0_f64, f64::max);
    let tokens: Vec<HashSet<String>> = hits.iter().map(|h| mmr_tokens(&h.text)).collect();
    let lambda = 0.72_f64;
    let mut selected = Vec::with_capacity(top_k);
    let mut used = vec![false; hits.len()];

    while selected.len() < top_k {
        let mut best: Option<(usize, f64)> = None;
        for (idx, hit) in hits.iter().enumerate() {
            if used[idx] {
                continue;
            }
            let relevance = if max_score > 0.0 {
                hit.score / max_score
            } else {
                0.0
            };
            let similarity = selected
                .iter()
                .map(|selected_idx| jaccard(&tokens[idx], &tokens[*selected_idx]))
                .fold(0.0_f64, f64::max);
            let mmr_score = lambda * relevance - (1.0 - lambda) * similarity;
            let is_better = best
                .map(|(best_idx, best_score)| {
                    mmr_score > best_score
                        || ((mmr_score - best_score).abs() < f64::EPSILON
                            && hit.score > hits[best_idx].score)
                })
                .unwrap_or(true);
            if is_better {
                best = Some((idx, mmr_score));
            }
        }
        let Some((idx, _)) = best else {
            break;
        };
        used[idx] = true;
        selected.push(idx);
    }

    selected.into_iter().map(|idx| hits[idx].clone()).collect()
}

pub fn search_corpus(
    conn: &Connection,
    query: &str,
    query_embedding: Option<&[f32]>,
    top_k: usize,
    reranker: Option<&dyn CorpusReranker>,
) -> Result<Vec<CorpusHit>> {
    if top_k == 0 {
        return Ok(Vec::new());
    }
    corpus_migrate(conn)?;
    let candidate_k = (top_k * 3).max(top_k + 8);
    let mut scores: HashMap<i64, f64> = HashMap::new();
    let mut order: Vec<i64> = Vec::new();

    if let Some(match_query) = fts5_query(query) {
        let mut stmt = conn.prepare(
            "SELECT c.id, f.rank
             FROM synapse_corpus_chunks_fts f
             JOIN synapse_corpus_chunks c ON c.id = f.rowid
             WHERE synapse_corpus_chunks_fts MATCH ?1
             ORDER BY f.rank
             LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![match_query, candidate_k as i64], |r| {
            r.get::<_, i64>(0)
        })?;
        for (rank, row) in rows.enumerate() {
            let id = row?;
            scores
                .entry(id)
                .and_modify(|s| *s += rrf_score(rank))
                .or_insert_with(|| rrf_score(rank));
            order.push(id);
        }
    }

    if let Some(embedding) = query_embedding {
        let blob = vec_blob(embedding)?;
        let vec_rows = conn
            .prepare(
                "SELECT chunk_id, distance
                 FROM synapse_corpus_vec
                 WHERE embedding MATCH ?1 AND k = ?2
                 ORDER BY distance",
            )
            .and_then(|mut stmt| {
                stmt.query_map(params![blob, candidate_k as i64], |r| r.get::<_, i64>(0))?
                    .collect::<std::result::Result<Vec<_>, _>>()
            })
            .unwrap_or_default();
        for (rank, id) in vec_rows.into_iter().enumerate() {
            scores
                .entry(id)
                .and_modify(|s| *s += rrf_score(rank))
                .or_insert_with(|| rrf_score(rank));
            order.push(id);
        }
    }

    order.sort_by(|a, b| {
        scores
            .get(b)
            .copied()
            .unwrap_or(0.0)
            .partial_cmp(&scores.get(a).copied().unwrap_or(0.0))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    order.dedup();
    let mut hits = fetch_hits(conn, &order, &scores)?;
    if let Some(r) = reranker {
        hits = r.rerank(query, hits);
    }
    Ok(mmr_diversify(hits, top_k))
}

pub fn set_corpus_chunk_embedding(
    conn: &Connection,
    chunk_id: i64,
    embedding: &[f32],
) -> Result<()> {
    corpus_migrate(conn)?;
    conn.execute(
        "INSERT OR REPLACE INTO synapse_corpus_vec(chunk_id, embedding) VALUES(?1, ?2)",
        params![chunk_id, vec_blob(embedding)?],
    )?;
    Ok(())
}

pub fn gold_candidates_from_corpus(conn: &Connection, limit: usize) -> Result<Vec<GoldCandidate>> {
    if limit == 0 {
        return Ok(Vec::new());
    }
    corpus_migrate(conn)?;
    let mut stmt = conn.prepare(
        "SELECT c.id, d.title, d.uri, c.text
         FROM synapse_corpus_chunks c
         JOIN synapse_corpus_documents d ON d.id = c.document_id
         ORDER BY COALESCE(d.published_ts, d.created_ts) DESC, d.id DESC, c.ordinal ASC
         LIMIT ?1",
    )?;
    let rows = stmt.query_map([limit as i64], |r| {
        let chunk_id = r.get::<_, i64>(0)?;
        let title = r.get::<_, String>(1)?;
        let source_uri = r.get::<_, String>(2)?;
        let text = r.get::<_, String>(3)?;
        Ok(GoldCandidate {
            query: gold_candidate_query(&title, &text, chunk_id),
            relevant_chunk_ids: vec![chunk_id],
            title,
            source_uri,
            text_preview: text.chars().take(240).collect(),
        })
    })?;
    Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
}

pub fn rank_gold_questions(
    conn: &Connection,
    gold: &[GoldQuestion],
    limit: usize,
) -> Result<Vec<Vec<i64>>> {
    let mut rankings = Vec::with_capacity(gold.len());
    for q in gold {
        let hits = search_corpus(conn, &q.query, None, limit, None)?;
        rankings.push(hits.into_iter().map(|h| h.chunk_id).collect());
    }
    Ok(rankings)
}

pub fn import_synapse_docs_to_corpus(conn: &Connection, limit: usize) -> Result<Vec<i64>> {
    if limit == 0 {
        return Ok(Vec::new());
    }
    corpus_migrate(conn)?;
    let mut stmt = conn.prepare(
        "SELECT id, uri, title, text, ts
         FROM docs
         ORDER BY ts DESC, id DESC
         LIMIT ?1",
    )?;
    let rows = stmt.query_map([limit as i64], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, Option<String>>(1)?,
            r.get::<_, Option<String>>(2)?,
            r.get::<_, String>(3)?,
            r.get::<_, i64>(4)?,
        ))
    })?;
    let mut imported = Vec::new();
    for row in rows {
        let (doc_id, uri, title, text, ts) = row?;
        if text.trim().is_empty() {
            continue;
        }
        let source_uri = uri.unwrap_or_else(|| format!("synapse:doc:{doc_id}"));
        let external_id = format!("synapse-doc:{doc_id}");
        let title = title
            .filter(|t| !t.trim().is_empty())
            .unwrap_or_else(|| format!("Synapse doc {doc_id}"));
        let published_ts = if ts > 9_999_999_999 {
            Some(ts / 1000)
        } else {
            Some(ts)
        };
        let doc = NewCorpusDocument {
            source_kind: CorpusSourceKind::Text,
            source_uri: &source_uri,
            external_id: &external_id,
            title: &title,
            text: &text,
            published_ts,
        };
        imported.push(put_corpus_document(conn, &doc)?);
    }
    Ok(imported)
}

pub fn bootstrap_eval_from_corpus(
    conn: &Connection,
    candidate_limit: usize,
    min_gold: usize,
    ranking_limit: usize,
) -> Result<EvalBootstrap> {
    let candidates = gold_candidates_from_corpus(conn, candidate_limit)?;
    if candidates.len() < min_gold {
        return Err(Error::Other(format!(
            "gold set too small: {} < {}",
            candidates.len(),
            min_gold
        )));
    }
    let gold = candidates
        .iter()
        .map(|c| GoldQuestion {
            query: c.query.clone(),
            relevant_chunk_ids: c.relevant_chunk_ids.clone(),
        })
        .collect::<Vec<_>>();
    let baseline_rankings = rank_gold_questions(conn, &gold, ranking_limit)?;
    let baseline = evaluate_rankings(&gold, &baseline_rankings)?;
    Ok(EvalBootstrap {
        gold_count: gold.len(),
        candidates,
        gold,
        baseline_rankings,
        baseline,
        min_gold,
    })
}

pub fn queue_promotion(
    conn: &Connection,
    chunk_id: i64,
    kind: PromotionKind,
    rationale: &str,
) -> Result<i64> {
    corpus_migrate(conn)?;
    conn.execute(
        "INSERT INTO synapse_corpus_promotions(chunk_id, kind, rationale, status, created_ts)
         VALUES(?1, ?2, ?3, 'pending', ?4)",
        params![chunk_id, promotion_kind_str(kind), rationale, now_secs()],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn verify_promotion(conn: &Connection, promotion_id: i64, verifier: &str) -> Result<()> {
    let changed = conn.execute(
        "UPDATE synapse_corpus_promotions
         SET status='verified', verifier=?1, verified_ts=?2
         WHERE id=?3 AND status='pending'",
        params![verifier, now_secs(), promotion_id],
    )?;
    if changed == 0 {
        return Err(Error::NotFound(format!("promotion_id={promotion_id}")));
    }
    Ok(())
}

pub fn ready_promotions(conn: &Connection) -> Result<Vec<i64>> {
    corpus_migrate(conn)?;
    let mut stmt = conn.prepare(
        "SELECT id FROM synapse_corpus_promotions
         WHERE status='verified'
         ORDER BY verified_ts ASC, id ASC",
    )?;
    let rows = stmt.query_map([], |r| r.get::<_, i64>(0))?;
    Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
}

pub fn evaluate_rankings(gold: &[GoldQuestion], rankings: &[Vec<i64>]) -> Result<EvalReport> {
    if gold.len() != rankings.len() {
        return Err(Error::Other(format!(
            "gold/rankings length mismatch: {} != {}",
            gold.len(),
            rankings.len()
        )));
    }
    if gold.is_empty() {
        return Ok(EvalReport {
            recall_at_5: 0.0,
            mrr: 0.0,
            false_recall_rate: 0.0,
        });
    }

    let mut recall_hits = 0usize;
    let mut reciprocal_sum = 0.0;
    let mut false_returned = 0usize;
    let mut returned = 0usize;

    for (q, ranked) in gold.iter().zip(rankings.iter()) {
        let relevant: HashSet<i64> = q.relevant_chunk_ids.iter().copied().collect();
        let top5 = ranked.iter().take(5);
        if top5.clone().any(|id| relevant.contains(id)) {
            recall_hits += 1;
        }
        if let Some((rank, _)) = ranked
            .iter()
            .enumerate()
            .find(|(_, id)| relevant.contains(id))
        {
            reciprocal_sum += 1.0 / (rank as f64 + 1.0);
        }
        for id in ranked.iter().take(5) {
            returned += 1;
            if !relevant.contains(id) {
                false_returned += 1;
            }
        }
    }

    Ok(EvalReport {
        recall_at_5: recall_hits as f64 / gold.len() as f64,
        mrr: reciprocal_sum / gold.len() as f64,
        false_recall_rate: if returned == 0 {
            0.0
        } else {
            false_returned as f64 / returned as f64
        },
    })
}

pub fn evaluate_rankings_gate(
    gold: &[GoldQuestion],
    baseline_rankings: &[Vec<i64>],
    candidate_rankings: &[Vec<i64>],
    min_gold: usize,
) -> Result<EvalGateReport> {
    if gold.len() < min_gold {
        return Err(Error::Other(format!(
            "gold set too small: {} < {}",
            gold.len(),
            min_gold
        )));
    }
    let baseline = evaluate_rankings(gold, baseline_rankings)?;
    let candidate = evaluate_rankings(gold, candidate_rankings)?;
    let recall_at_5_delta = candidate.recall_at_5 - baseline.recall_at_5;
    let mrr_delta = candidate.mrr - baseline.mrr;
    let false_recall_rate_delta = candidate.false_recall_rate - baseline.false_recall_rate;
    Ok(EvalGateReport {
        passed: recall_at_5_delta > 0.0 && mrr_delta > 0.0 && false_recall_rate_delta < 0.0,
        baseline,
        candidate,
        recall_at_5_delta,
        mrr_delta,
        false_recall_rate_delta,
        min_gold,
        gold_count: gold.len(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conn() -> Connection {
        register_vec_extension();
        let conn = Connection::open_in_memory().unwrap();
        corpus_migrate(&conn).unwrap();
        conn
    }

    #[test]
    fn corpus_ingest_chunks_and_dedups_by_source_external_id() {
        let conn = conn();
        let doc = NewCorpusDocument {
            source_kind: CorpusSourceKind::Web,
            source_uri: "https://example.test/noledge",
            external_id: "article-1",
            title: "Noledge Audit",
            text: "Noledge uses hybrid retrieval.\n\nSynapse should promote only verified decisions.",
            published_ts: Some(1_790_000_000),
        };

        let first = put_corpus_document(&conn, &doc).unwrap();
        let second = put_corpus_document(&conn, &doc).unwrap();
        let chunks: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM synapse_corpus_chunks WHERE document_id=?1",
                [first],
                |r| r.get(0),
            )
            .unwrap();

        assert!(first > 0);
        assert_eq!(first, second);
        assert_eq!(chunks, 2);
    }

    #[test]
    fn corpus_search_uses_keyword_retrieval_before_promotion() {
        let conn = conn();
        let doc = NewCorpusDocument {
            source_kind: CorpusSourceKind::Rss,
            source_uri: "https://feeds.example.test/rss",
            external_id: "rss-1",
            title: "Promotion Gate",
            text: "Unverified corpus snippets stay in the sidecar.\n\nVerified durable decisions can be promoted.",
            published_ts: None,
        };
        put_corpus_document(&conn, &doc).unwrap();

        let hits = search_corpus(&conn, "verified durable decisions", None, 5, None).unwrap();

        assert_eq!(hits.len(), 1);
        assert!(hits[0].text.contains("Verified durable decisions"));
    }

    #[test]
    fn corpus_search_uses_vector_leg_when_embedding_is_available() {
        let conn = conn();
        let a = put_corpus_document(
            &conn,
            &NewCorpusDocument {
                source_kind: CorpusSourceKind::Text,
                source_uri: "manual:a",
                external_id: "a",
                title: "A",
                text: "apple banana",
                published_ts: None,
            },
        )
        .unwrap();
        let b = put_corpus_document(
            &conn,
            &NewCorpusDocument {
                source_kind: CorpusSourceKind::Text,
                source_uri: "manual:b",
                external_id: "b",
                title: "B",
                text: "carrot daikon",
                published_ts: None,
            },
        )
        .unwrap();
        let a_chunk: i64 = conn
            .query_row(
                "SELECT id FROM synapse_corpus_chunks WHERE document_id=?1",
                [a],
                |r| r.get(0),
            )
            .unwrap();
        let b_chunk: i64 = conn
            .query_row(
                "SELECT id FROM synapse_corpus_chunks WHERE document_id=?1",
                [b],
                |r| r.get(0),
            )
            .unwrap();
        let mut a_vec = vec![0.0; crate::types::EMBED_DIM];
        let mut b_vec = vec![0.0; crate::types::EMBED_DIM];
        a_vec[1] = 1.0;
        b_vec[0] = 1.0;
        set_corpus_chunk_embedding(&conn, a_chunk, &a_vec).unwrap();
        set_corpus_chunk_embedding(&conn, b_chunk, &b_vec).unwrap();

        let hits = search_corpus(&conn, "unrelated", Some(&b_vec), 2, None).unwrap();

        assert_eq!(hits[0].chunk_id, b_chunk);
    }

    #[test]
    fn corpus_search_mmr_diversifies_near_duplicate_chunks() {
        let conn = conn();
        for (external_id, title, text) in [
            (
                "dup-a",
                "Duplicate A",
                "alpha beta gamma same repeated framing same repeated framing",
            ),
            (
                "dup-b",
                "Duplicate B",
                "alpha beta gamma same repeated framing same repeated framing",
            ),
            (
                "different",
                "Different Angle",
                "alpha beta gamma different evidence angle clinical audit workflow",
            ),
        ] {
            put_corpus_document(
                &conn,
                &NewCorpusDocument {
                    source_kind: CorpusSourceKind::Text,
                    source_uri: &format!("manual:{external_id}"),
                    external_id,
                    title,
                    text,
                    published_ts: None,
                },
            )
            .unwrap();
        }

        let hits = search_corpus(&conn, "alpha beta gamma", None, 2, None).unwrap();

        assert_eq!(hits.len(), 2);
        assert!(
            hits.iter()
                .any(|h| h.text.contains("different evidence angle")),
            "expected MMR to keep the distinct relevant angle in top-2, got {hits:?}"
        );
    }

    #[test]
    fn rss_ingest_imports_items_and_dedups_by_guid() {
        let conn = conn();
        let feed = r#"<?xml version="1.0" encoding="UTF-8"?>
<rss version="2.0" xmlns:content="http://purl.org/rss/1.0/modules/content/">
  <channel>
    <title>Agent Feeds</title>
    <item>
      <title>RAG Memory</title>
      <link>https://example.test/rag-memory</link>
      <guid isPermaLink="false">rag-memory-1</guid>
      <description>Short description should be indexed.</description>
      <content:encoded><![CDATA[Retrieval augmented corpus updates agents daily.]]></content:encoded>
    </item>
    <item>
      <title>Promotion Gates</title>
      <link>https://example.test/promotion-gates</link>
      <guid>https://example.test/promotion-gates</guid>
      <description>Verified facts and decisions are promoted deliberately.</description>
    </item>
  </channel>
</rss>"#;

        let first = ingest_rss_xml(&conn, "https://feeds.example.test/agents.xml", feed).unwrap();
        let second = ingest_rss_xml(&conn, "https://feeds.example.test/agents.xml", feed).unwrap();
        let docs: i64 = conn
            .query_row("SELECT COUNT(*) FROM synapse_corpus_documents", [], |r| {
                r.get(0)
            })
            .unwrap();
        let hits =
            search_corpus(&conn, "retrieval augmented corpus updates", None, 3, None).unwrap();

        assert_eq!(first.len(), 2);
        assert_eq!(second, first);
        assert_eq!(docs, 2);
        assert!(hits.iter().any(|h| {
            h.title == "RAG Memory"
                && h.text
                    .contains("Retrieval augmented corpus updates agents daily")
        }));
    }

    #[test]
    fn youtube_transcript_ingest_cleans_vtt_and_dedups_by_video_id() {
        let conn = conn();
        let transcript = r#"WEBVTT

00:00:00.000 --> 00:00:03.200
Noledge lets agents query large documentation libraries.

00:00:03.200 --> 00:00:07.000
Promotion gates keep durable memory clean.
"#;

        let first = ingest_youtube_transcript(
            &conn,
            "https://www.youtube.com/watch?v=QvP8G_BqL_Q",
            "QvP8G_BqL_Q",
            "Noledge - all the knowledge",
            transcript,
        )
        .unwrap();
        let second = ingest_youtube_transcript(
            &conn,
            "https://www.youtube.com/watch?v=QvP8G_BqL_Q",
            "QvP8G_BqL_Q",
            "Noledge - all the knowledge",
            transcript,
        )
        .unwrap();
        let docs: i64 = conn
            .query_row("SELECT COUNT(*) FROM synapse_corpus_documents", [], |r| {
                r.get(0)
            })
            .unwrap();
        let hits = search_corpus(&conn, "promotion gates durable memory", None, 3, None).unwrap();

        assert_eq!(first, second);
        assert_eq!(docs, 1);
        assert!(hits.iter().any(|h| {
            h.title == "Noledge - all the knowledge"
                && h.text.contains("Promotion gates keep durable memory clean")
                && !h.text.contains("-->")
        }));
    }

    #[test]
    fn youtube_video_id_extracts_common_url_forms() {
        assert_eq!(
            youtube_video_id("https://www.youtube.com/watch?v=QvP8G_BqL_Q&list=abc"),
            Some("QvP8G_BqL_Q".to_string())
        );
        assert_eq!(
            youtube_video_id("https://youtu.be/QvP8G_BqL_Q?t=42"),
            Some("QvP8G_BqL_Q".to_string())
        );
        assert_eq!(
            youtube_video_id("QvP8G_BqL_Q"),
            Some("QvP8G_BqL_Q".to_string())
        );
        assert_eq!(youtube_video_id("https://example.test/nope"), None);
    }

    #[test]
    fn web_html_ingest_extracts_readable_text_and_dedups_by_url() {
        let conn = conn();
        let html = r#"<!doctype html>
<html>
  <head>
    <title>Agent Memory Field Guide</title>
    <style>.hidden { display: none; }</style>
    <script>window.secretNoise = "do not index";</script>
  </head>
  <body>
    <article>
      <h1>Agent Memory Field Guide</h1>
      <p>Hybrid retrieval keeps fresh documentation available to agents.</p>
      <p>Promotion gates keep durable facts separate from raw web notes.</p>
    </article>
  </body>
</html>"#;

        let first = ingest_web_html(
            &conn,
            "https://example.test/agent-memory-field-guide",
            None,
            html,
        )
        .unwrap();
        let second = ingest_web_html(
            &conn,
            "https://example.test/agent-memory-field-guide",
            None,
            html,
        )
        .unwrap();
        let docs: i64 = conn
            .query_row("SELECT COUNT(*) FROM synapse_corpus_documents", [], |r| {
                r.get(0)
            })
            .unwrap();
        let hits =
            search_corpus(&conn, "hybrid retrieval fresh documentation", None, 3, None).unwrap();

        assert_eq!(first, second);
        assert_eq!(docs, 1);
        assert!(hits.iter().any(|h| {
            h.title == "Agent Memory Field Guide"
                && h.text
                    .contains("Hybrid retrieval keeps fresh documentation available")
                && !h.text.contains("secretNoise")
                && !h.text.contains("display: none")
        }));
    }

    fn minimal_pdf_bytes(text: &str) -> Vec<u8> {
        let escaped = text
            .replace('\\', "\\\\")
            .replace('(', "\\(")
            .replace(')', "\\)");
        let stream = format!("BT /F1 18 Tf 72 720 Td ({escaped}) Tj ET");
        let objects = [
            "1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n".to_string(),
            "2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n".to_string(),
            "3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>\nendobj\n".to_string(),
            "4 0 obj\n<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>\nendobj\n".to_string(),
            format!(
                "5 0 obj\n<< /Length {} >>\nstream\n{}\nendstream\nendobj\n",
                stream.len(),
                stream
            ),
        ];
        let mut pdf = b"%PDF-1.4\n".to_vec();
        let mut offsets = vec![0usize];
        for object in objects {
            offsets.push(pdf.len());
            pdf.extend_from_slice(object.as_bytes());
        }
        let xref_start = pdf.len();
        pdf.extend_from_slice(format!("xref\n0 {}\n", offsets.len()).as_bytes());
        pdf.extend_from_slice(b"0000000000 65535 f \n");
        for offset in offsets.iter().skip(1) {
            pdf.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
        }
        pdf.extend_from_slice(
            format!(
                "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{}\n%%EOF\n",
                offsets.len(),
                xref_start
            )
            .as_bytes(),
        );
        pdf
    }

    #[test]
    fn pdf_ingest_extracts_text_and_dedups_by_uri() {
        let conn = conn();
        let pdf = minimal_pdf_bytes("Hybrid PDF corpus ingest keeps research searchable.");

        let first = ingest_pdf_bytes(
            &conn,
            "file:///tmp/hybrid-research.pdf",
            "Hybrid Research PDF",
            &pdf,
        )
        .unwrap();
        let second = ingest_pdf_bytes(
            &conn,
            "file:///tmp/hybrid-research.pdf",
            "Hybrid Research PDF",
            &pdf,
        )
        .unwrap();
        let docs: i64 = conn
            .query_row("SELECT COUNT(*) FROM synapse_corpus_documents", [], |r| {
                r.get(0)
            })
            .unwrap();
        let hits = search_corpus(&conn, "hybrid PDF research searchable", None, 3, None).unwrap();

        assert_eq!(first, second);
        assert_eq!(docs, 1);
        assert!(hits.iter().any(|h| {
            h.title == "Hybrid Research PDF"
                && h.text
                    .contains("Hybrid PDF corpus ingest keeps research searchable")
        }));
    }

    #[test]
    fn fetched_document_routes_by_content_type_and_uri() {
        let conn = conn();
        let rss = br#"<?xml version="1.0"?>
<rss version="2.0"><channel><item><title>Fetched RSS</title><guid>rss-1</guid><description>Fetched feed updates keep agents current.</description></item></channel></rss>"#;
        let html = br#"<html><head><title>Fetched HTML</title></head><body><p>Fetched web documentation enters the sidecar.</p></body></html>"#;
        let pdf = minimal_pdf_bytes("Fetched PDF research enters the sidecar.");

        let rss_ids = ingest_fetched_document(
            &conn,
            "https://feeds.example.test/current.xml",
            Some("application/rss+xml; charset=utf-8"),
            None,
            rss,
        )
        .unwrap();
        let html_ids = ingest_fetched_document(
            &conn,
            "https://example.test/current",
            Some("text/html"),
            None,
            html,
        )
        .unwrap();
        let pdf_ids = ingest_fetched_document(
            &conn,
            "https://example.test/current.pdf",
            Some("application/octet-stream"),
            Some("Fetched PDF"),
            &pdf,
        )
        .unwrap();
        let hits = search_corpus(&conn, "fetched sidecar current", None, 5, None).unwrap();

        assert_eq!(rss_ids.len(), 1);
        assert_eq!(html_ids.len(), 1);
        assert_eq!(pdf_ids.len(), 1);
        assert!(hits.iter().any(|h| h.title == "Fetched RSS"));
        assert!(hits.iter().any(|h| h.title == "Fetched HTML"));
        assert!(hits.iter().any(|h| h.title == "Fetched PDF"));
    }

    #[test]
    fn scheduled_sources_are_due_until_synced_then_due_after_interval() {
        let conn = conn();
        let source_id = upsert_corpus_sync_source(
            &conn,
            CorpusSourceKind::Rss,
            "https://feeds.example.test/daily.xml",
            Some("Daily Feed"),
            3_600,
        )
        .unwrap();

        let due_now = due_corpus_sync_sources(&conn, 10_000, 10).unwrap();
        assert_eq!(due_now.len(), 1);
        assert_eq!(due_now[0].id, source_id);
        assert_eq!(due_now[0].kind, CorpusSourceKind::Rss);
        assert_eq!(due_now[0].uri, "https://feeds.example.test/daily.xml");
        assert_eq!(due_now[0].sync_interval_secs, 3_600);

        mark_corpus_source_synced(&conn, source_id, 10_000).unwrap();
        assert!(
            due_corpus_sync_sources(&conn, 12_000, 10)
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            due_corpus_sync_sources(&conn, 13_600, 10).unwrap()[0].id,
            source_id
        );
    }

    #[test]
    fn promotion_gate_hides_unverified_candidates_until_verified() {
        let conn = conn();
        let doc = NewCorpusDocument {
            source_kind: CorpusSourceKind::Text,
            source_uri: "manual:test",
            external_id: "note-1",
            title: "Decision Candidate",
            text: "Decision: use corpus sidecar before durable Synapse promotion.",
            published_ts: None,
        };
        let doc_id = put_corpus_document(&conn, &doc).unwrap();
        let chunk_id: i64 = conn
            .query_row(
                "SELECT id FROM synapse_corpus_chunks WHERE document_id=?1",
                [doc_id],
                |r| r.get(0),
            )
            .unwrap();
        let promotion_id = queue_promotion(
            &conn,
            chunk_id,
            PromotionKind::Decision,
            "durable architecture",
        )
        .unwrap();

        assert!(ready_promotions(&conn).unwrap().is_empty());
        verify_promotion(&conn, promotion_id, "eval:manual").unwrap();
        assert_eq!(ready_promotions(&conn).unwrap(), vec![promotion_id]);
    }

    #[test]
    fn eval_report_computes_recall_mrr_and_false_recall_rate() {
        let gold = vec![
            GoldQuestion {
                query: "alpha".into(),
                relevant_chunk_ids: vec![10],
            },
            GoldQuestion {
                query: "beta".into(),
                relevant_chunk_ids: vec![20],
            },
        ];
        let rankings = vec![vec![1, 10, 3, 4, 5], vec![21, 22, 23, 24, 25]];

        let report = evaluate_rankings(&gold, &rankings).unwrap();

        assert_eq!(report.recall_at_5, 0.5);
        assert_eq!(report.mrr, 0.25);
        assert_eq!(report.false_recall_rate, 0.9);
    }

    #[test]
    fn eval_gate_requires_recall_mrr_gain_and_false_recall_drop() {
        let gold = vec![
            GoldQuestion {
                query: "alpha".into(),
                relevant_chunk_ids: vec![10],
            },
            GoldQuestion {
                query: "beta".into(),
                relevant_chunk_ids: vec![20],
            },
        ];
        let baseline = vec![vec![1, 2, 3, 4, 5], vec![20, 21, 22, 23, 24]];
        let candidate = vec![vec![10], vec![20]];

        let passed = evaluate_rankings_gate(&gold, &baseline, &candidate, 2).unwrap();
        assert!(passed.passed);
        assert_eq!(passed.baseline.recall_at_5, 0.5);
        assert_eq!(passed.candidate.recall_at_5, 1.0);
        assert!(passed.mrr_delta > 0.0);
        assert!(passed.false_recall_rate_delta < 0.0);

        let unchanged = vec![vec![10, 1, 2, 3, 4], vec![20, 21, 22, 23, 24]];
        let failed = evaluate_rankings_gate(&gold, &unchanged, &unchanged, 2).unwrap();
        assert!(!failed.passed);
        assert!(failed.false_recall_rate_delta >= 0.0);

        let too_small = evaluate_rankings_gate(&gold[..1], &baseline[..1], &candidate[..1], 2);
        assert!(too_small.is_err());
    }

    #[test]
    fn gold_candidates_from_corpus_use_real_chunk_ids_and_previews() {
        let conn = conn();
        let doc_id = put_corpus_document(
            &conn,
            &NewCorpusDocument {
                source_kind: CorpusSourceKind::Web,
                source_uri: "https://example.test/temporal-rag",
                external_id: "temporal-rag",
                title: "Temporal RAG Notes",
                text: "Graph changes need time-aware assertions before memory promotion.",
                published_ts: None,
            },
        )
        .unwrap();
        let chunk_id: i64 = conn
            .query_row(
                "SELECT id FROM synapse_corpus_chunks WHERE document_id=?1",
                [doc_id],
                |r| r.get(0),
            )
            .unwrap();

        let candidates = gold_candidates_from_corpus(&conn, 10).unwrap();

        assert_eq!(candidates.len(), 1);
        assert!(candidates[0].query.contains("graph changes"));
        assert_eq!(candidates[0].relevant_chunk_ids, vec![chunk_id]);
        assert_eq!(
            candidates[0].source_uri,
            "https://example.test/temporal-rag"
        );
        assert!(candidates[0].text_preview.contains("time-aware assertions"));

        let gold = vec![GoldQuestion {
            query: candidates[0].query.clone(),
            relevant_chunk_ids: candidates[0].relevant_chunk_ids.clone(),
        }];
        assert_eq!(
            rank_gold_questions(&conn, &gold, 1).unwrap(),
            vec![vec![chunk_id]]
        );
    }

    #[test]
    fn rank_gold_questions_returns_corpus_chunk_ids_in_question_order() {
        let conn = conn();
        let alpha_doc = put_corpus_document(
            &conn,
            &NewCorpusDocument {
                source_kind: CorpusSourceKind::Text,
                source_uri: "manual:alpha",
                external_id: "alpha",
                title: "Alpha Memory",
                text: "Alpha recall uses durable verified facts.",
                published_ts: None,
            },
        )
        .unwrap();
        let beta_doc = put_corpus_document(
            &conn,
            &NewCorpusDocument {
                source_kind: CorpusSourceKind::Text,
                source_uri: "manual:beta",
                external_id: "beta",
                title: "Beta Corpus",
                text: "Beta retrieval uses raw sidecar chunks.",
                published_ts: None,
            },
        )
        .unwrap();
        let alpha_chunk: i64 = conn
            .query_row(
                "SELECT id FROM synapse_corpus_chunks WHERE document_id=?1",
                [alpha_doc],
                |r| r.get(0),
            )
            .unwrap();
        let beta_chunk: i64 = conn
            .query_row(
                "SELECT id FROM synapse_corpus_chunks WHERE document_id=?1",
                [beta_doc],
                |r| r.get(0),
            )
            .unwrap();
        let gold = vec![
            GoldQuestion {
                query: "raw sidecar chunks".into(),
                relevant_chunk_ids: vec![beta_chunk],
            },
            GoldQuestion {
                query: "durable verified facts".into(),
                relevant_chunk_ids: vec![alpha_chunk],
            },
        ];

        let rankings = rank_gold_questions(&conn, &gold, 1).unwrap();

        assert_eq!(rankings, vec![vec![beta_chunk], vec![alpha_chunk]]);
    }

    #[test]
    fn import_synapse_docs_to_corpus_bootstraps_gold_from_real_usage() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let mut store = crate::Store::open(tmp.path()).unwrap();
        let durable_id = store
            .put(&crate::PutRequest {
                uri: Some("synapse://decision/corpus-eval".into()),
                title: Some("Corpus Eval Decision".into()),
                text: "Gold questions should come from real Synapse decisions before temporal graph work.".into(),
                meta: Some(serde_json::json!({"kind": "decision"})),
                embedding: None,
            })
            .unwrap();
        let fact_id = store
            .put(&crate::PutRequest {
                uri: Some("synapse://fact/promotion-gate".into()),
                title: Some("Promotion Gate Fact".into()),
                text: "Verified promotion gates reduce false durable recall from raw corpus notes."
                    .into(),
                meta: Some(serde_json::json!({"kind": "fact"})),
                embedding: None,
            })
            .unwrap();

        let first = import_synapse_docs_to_corpus(&store.conn, 100).unwrap();
        let second = import_synapse_docs_to_corpus(&store.conn, 100).unwrap();
        let docs: i64 = store
            .conn
            .query_row("SELECT COUNT(*) FROM synapse_corpus_documents", [], |r| {
                r.get(0)
            })
            .unwrap();

        assert_eq!(first.len(), 2);
        assert_eq!(second, first);
        assert_eq!(docs, 2);

        let candidates = gold_candidates_from_corpus(&store.conn, 100).unwrap();
        assert_eq!(candidates.len(), 2);
        assert!(candidates.iter().any(|c| {
            c.source_uri == "synapse://decision/corpus-eval"
                && c.relevant_chunk_ids.len() == 1
                && c.query.contains("gold questions")
        }));
        assert!(candidates.iter().any(|c| {
            c.source_uri == "synapse://fact/promotion-gate"
                && c.relevant_chunk_ids.len() == 1
                && c.query.contains("verified promotion")
        }));

        let gold: Vec<GoldQuestion> = candidates
            .into_iter()
            .map(|c| GoldQuestion {
                query: c.query,
                relevant_chunk_ids: c.relevant_chunk_ids,
            })
            .collect();
        let rankings = rank_gold_questions(&store.conn, &gold, 1).unwrap();

        assert_eq!(rankings.len(), 2);
        assert!(rankings.iter().all(|ids| ids.len() == 1));
        let mut external_ids = Vec::new();
        for corpus_doc_id in first {
            external_ids.push(
                store
                    .conn
                    .query_row(
                        "SELECT external_id FROM synapse_corpus_documents WHERE id=?1",
                        [corpus_doc_id],
                        |r| r.get::<_, String>(0),
                    )
                    .unwrap(),
            );
        }
        assert!(external_ids.contains(&format!("synapse-doc:{durable_id}")));
        assert!(external_ids.contains(&format!("synapse-doc:{fact_id}")));
    }

    #[test]
    fn bootstrap_eval_from_corpus_requires_min_gold_and_snapshots_baseline() {
        let conn = conn();
        for (external_id, text) in [
            (
                "decision",
                "Decision gold bootstrap should require enough real Synapse examples.",
            ),
            (
                "fact",
                "Fact baseline rankings should be captured before retrieval changes.",
            ),
        ] {
            put_corpus_document(
                &conn,
                &NewCorpusDocument {
                    source_kind: CorpusSourceKind::Text,
                    source_uri: &format!("manual:{external_id}"),
                    external_id,
                    title: external_id,
                    text,
                    published_ts: None,
                },
            )
            .unwrap();
        }

        let too_small = bootstrap_eval_from_corpus(&conn, 100, 3, 5);
        assert!(too_small.is_err());

        let boot = bootstrap_eval_from_corpus(&conn, 100, 2, 5).unwrap();

        assert_eq!(boot.min_gold, 2);
        assert_eq!(boot.gold_count, 2);
        assert_eq!(boot.candidates.len(), 2);
        assert_eq!(boot.gold.len(), 2);
        assert_eq!(boot.baseline_rankings.len(), 2);
        assert_eq!(boot.baseline.recall_at_5, 1.0);
        assert_eq!(boot.baseline.mrr, 1.0);
    }
}
