CREATE TABLE IF NOT EXISTS synapse_rerank_log (
    id INTEGER PRIMARY KEY,
    query_id TEXT NOT NULL,
    doc_id TEXT NOT NULL,
    vec_score REAL,
    fts_score REAL,
    recency_days REAL,
    title_exact_match INTEGER DEFAULT 0,
    path_depth INTEGER DEFAULT 3,
    doc_len INTEGER DEFAULT 200,
    clicked INTEGER DEFAULT 0,
    ts INTEGER DEFAULT (unixepoch())
);
CREATE INDEX IF NOT EXISTS idx_rerank_log_query ON synapse_rerank_log(query_id);
