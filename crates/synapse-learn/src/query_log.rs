use anyhow::Result;
use rusqlite::{params, Connection};
use std::path::{Path, PathBuf};

pub struct QueryLog {
    conn: Connection,
}

#[derive(Debug)]
pub struct QueryEvent {
    pub ts: i64,
    pub query_text: String,
    pub query_embed: Option<Vec<u8>>,
    pub result_doc_id: i64,
    pub rank: i32,
    pub score: f64,
    pub bm25_score: Option<f64>,
    pub vec_score: Option<f64>,
    pub session_id: Option<String>,
}

impl QueryLog {
    pub fn default_path() -> PathBuf {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
        PathBuf::from(home).join(".synapse").join("query_log.db")
    }

    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        if let Some(parent) = path.as_ref().parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS query_events (
                id INTEGER PRIMARY KEY,
                ts INTEGER NOT NULL,
                query_text TEXT NOT NULL,
                query_embed BLOB,
                result_doc_id INTEGER NOT NULL,
                rank INTEGER NOT NULL,
                score REAL NOT NULL,
                bm25_score REAL,
                vec_score REAL,
                clicked INTEGER DEFAULT 0,
                dwell_ms INTEGER DEFAULT 0,
                session_id TEXT
            );
            CREATE INDEX IF NOT EXISTS idx_ts ON query_events(ts);
            CREATE INDEX IF NOT EXISTS idx_query ON query_events(query_text);
            "#,
        )?;
        Ok(Self { conn })
    }

    pub fn log_event(&self, ev: &QueryEvent) -> Result<i64> {
        self.conn.execute(
            r#"INSERT INTO query_events
               (ts, query_text, query_embed, result_doc_id, rank, score, bm25_score, vec_score, session_id)
               VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)"#,
            params![
                ev.ts,
                ev.query_text,
                ev.query_embed,
                ev.result_doc_id,
                ev.rank,
                ev.score,
                ev.bm25_score,
                ev.vec_score,
                ev.session_id,
            ],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn mark_click(&self, id: i64, dwell_ms: i64) -> Result<()> {
        self.conn.execute(
            "UPDATE query_events SET clicked=1, dwell_ms=?1 WHERE id=?2",
            params![dwell_ms, id],
        )?;
        Ok(())
    }

    /// Export in LightGBM LibSVM format for LambdaMART training.
    /// Each row: <label> qid:<qid> 1:<bm25> 2:<vec_score> 3:<rank> 4:<score>
    /// label = clicked (1/0)
    pub fn export_libsvm(&self, out_path: impl AsRef<Path>) -> Result<usize> {
        use std::collections::HashMap;
        use std::fmt::Write as FmtWrite;
        use std::io::Write;

        struct Row {
            clicked: i64,
            qid: i64,
            bm25: f64,
            vec_score: f64,
            rank: i64,
            score: f64,
        }

        let mut stmt = self.conn.prepare(
            r#"SELECT id, query_text, result_doc_id, rank, score,
                      COALESCE(bm25_score,0.0), COALESCE(vec_score,0.0), clicked
               FROM query_events
               ORDER BY query_text, rank"#,
        )?;

        let mut qid_map: HashMap<String, i64> = HashMap::new();
        let mut next_qid: i64 = 1;
        let mut rows: Vec<Row> = Vec::new();

        let iter = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(1)?,
                r.get::<_, i64>(3)?,
                r.get::<_, f64>(4)?,
                r.get::<_, f64>(5)?,
                r.get::<_, f64>(6)?,
                r.get::<_, i64>(7)?,
            ))
        })?;

        for item in iter {
            let (qt, rank, score, bm25, vec_score, clicked) = item?;
            let qid = *qid_map.entry(qt).or_insert_with(|| {
                let id = next_qid;
                next_qid += 1;
                id
            });
            rows.push(Row { clicked, qid, bm25, vec_score, rank, score });
        }

        let mut out = std::fs::File::create(out_path)?;
        let mut line = String::new();
        for r in &rows {
            line.clear();
            write!(
                line,
                "{} qid:{} 1:{:.6} 2:{:.6} 3:{} 4:{:.6}",
                r.clicked, r.qid, r.bm25, r.vec_score, r.rank, r.score
            )
            .unwrap();
            writeln!(out, "{}", line)?;
        }
        Ok(rows.len())
    }
}
