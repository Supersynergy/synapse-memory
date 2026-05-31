use anyhow::Result;
use rusqlite::{Connection, params};
use std::path::Path;

pub struct LearnStore {
    pub conn: Connection,
}

impl LearnStore {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS learn_bandit (
                shard_id TEXT PRIMARY KEY,
                wins INTEGER NOT NULL DEFAULT 0,
                losses INTEGER NOT NULL DEFAULT 0
            );
            CREATE TABLE IF NOT EXISTS learn_rrf_alpha (
                shape_hash INTEGER PRIMARY KEY,
                bucket INTEGER NOT NULL DEFAULT 2,
                wins INTEGER NOT NULL DEFAULT 0,
                losses INTEGER NOT NULL DEFAULT 0
            );
            CREATE TABLE IF NOT EXISTS feedback (
                ts INTEGER NOT NULL,
                query TEXT NOT NULL,
                query_emb BLOB,
                accepted_doc_id INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_fb_ts ON feedback(ts);
            CREATE TABLE IF NOT EXISTS context_query_log (
                context_id TEXT PRIMARY KEY,
                ts INTEGER NOT NULL,
                query TEXT NOT NULL,
                mode TEXT NOT NULL,
                route TEXT NOT NULL,
                doc_ids TEXT NOT NULL,
                accepted_doc_id INTEGER,
                reward INTEGER
            );
            CREATE INDEX IF NOT EXISTS idx_context_query_log_ts ON context_query_log(ts);
            CREATE TABLE IF NOT EXISTS memory_type_reward (
                kind TEXT PRIMARY KEY,
                wins INTEGER NOT NULL DEFAULT 1,
                losses INTEGER NOT NULL DEFAULT 1
            );
            CREATE TABLE IF NOT EXISTS route_reward (
                route TEXT PRIMARY KEY,
                wins INTEGER NOT NULL DEFAULT 1,
                losses INTEGER NOT NULL DEFAULT 1
            );
            CREATE TABLE IF NOT EXISTS learn_calibration (
                bucket INTEGER PRIMARY KEY,
                correction REAL NOT NULL DEFAULT 1.0
            );
        "#,
        )?;
        Ok(Self { conn })
    }

    pub fn get_bandit_prior(&self, shard_id: &str) -> Result<(u32, u32)> {
        let r = self.conn.query_row(
            "SELECT wins, losses FROM learn_bandit WHERE shard_id=?1",
            params![shard_id],
            |r| Ok((r.get::<_, u32>(0)?, r.get::<_, u32>(1)?)),
        );
        match r {
            Ok(v) => Ok(v),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok((1, 1)),
            Err(e) => Err(e.into()),
        }
    }

    pub fn update_bandit(&self, shard_id: &str, hit: bool) -> Result<()> {
        if hit {
            self.conn.execute(
                "INSERT INTO learn_bandit(shard_id,wins,losses) VALUES(?1,1,1)
                 ON CONFLICT(shard_id) DO UPDATE SET wins=wins+1",
                params![shard_id],
            )?;
        } else {
            self.conn.execute(
                "INSERT INTO learn_bandit(shard_id,wins,losses) VALUES(?1,1,1)
                 ON CONFLICT(shard_id) DO UPDATE SET losses=losses+1",
                params![shard_id],
            )?;
        }
        Ok(())
    }

    pub fn get_rrf_prior(&self, shape_hash: u8) -> Result<(usize, u32, u32)> {
        let r = self.conn.query_row(
            "SELECT bucket, wins, losses FROM learn_rrf_alpha WHERE shape_hash=?1",
            params![shape_hash as i64],
            |r| {
                Ok((
                    r.get::<_, i64>(0)? as usize,
                    r.get::<_, u32>(1)?,
                    r.get::<_, u32>(2)?,
                ))
            },
        );
        match r {
            Ok(v) => Ok(v),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok((2, 1, 1)),
            Err(e) => Err(e.into()),
        }
    }

    pub fn update_rrf(&self, shape_hash: u8, bucket: usize, hit: bool) -> Result<()> {
        if hit {
            self.conn.execute(
                "INSERT INTO learn_rrf_alpha(shape_hash,bucket,wins,losses) VALUES(?1,?2,1,1)
                 ON CONFLICT(shape_hash) DO UPDATE SET wins=wins+1, bucket=?2",
                params![shape_hash as i64, bucket as i64],
            )?;
        } else {
            self.conn.execute(
                "INSERT INTO learn_rrf_alpha(shape_hash,bucket,wins,losses) VALUES(?1,?2,1,1)
                 ON CONFLICT(shape_hash) DO UPDATE SET losses=losses+1",
                params![shape_hash as i64, bucket as i64],
            )?;
        }
        Ok(())
    }

    pub fn log_feedback(
        &self,
        ts: i64,
        query: &str,
        emb: Option<&[u8]>,
        doc_id: i64,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT INTO feedback(ts,query,query_emb,accepted_doc_id) VALUES(?1,?2,?3,?4)",
            params![ts, query, emb, doc_id],
        )?;
        Ok(())
    }

    pub fn log_context_query(
        &self,
        context_id: &str,
        ts: i64,
        query: &str,
        mode: &str,
        route: &str,
        doc_ids: &[i64],
    ) -> Result<()> {
        let doc_ids_json = serde_json::to_string(doc_ids)?;
        self.conn.execute(
            "INSERT OR REPLACE INTO context_query_log(context_id,ts,query,mode,route,doc_ids)
             VALUES(?1,?2,?3,?4,?5,?6)",
            params![context_id, ts, query, mode, route, doc_ids_json],
        )?;
        Ok(())
    }

    pub fn reward_context(&self, context_id: &str, accepted_doc_id: i64, hit: bool) -> Result<()> {
        self.conn.execute(
            "UPDATE context_query_log SET accepted_doc_id=?1, reward=?2 WHERE context_id=?3",
            params![accepted_doc_id, if hit { 1 } else { 0 }, context_id],
        )?;
        let row = self.conn.query_row(
            "SELECT route FROM context_query_log WHERE context_id=?1",
            params![context_id],
            |r| r.get::<_, String>(0),
        );
        if let Ok(route) = row {
            self.update_route_reward(&route, hit)?;
        }
        Ok(())
    }

    pub fn update_memory_type_reward(&self, kind: &str, hit: bool) -> Result<()> {
        if hit {
            self.conn.execute(
                "INSERT INTO memory_type_reward(kind,wins,losses) VALUES(?1,1,1)
                 ON CONFLICT(kind) DO UPDATE SET wins=wins+1",
                params![kind],
            )?;
        } else {
            self.conn.execute(
                "INSERT INTO memory_type_reward(kind,wins,losses) VALUES(?1,1,1)
                 ON CONFLICT(kind) DO UPDATE SET losses=losses+1",
                params![kind],
            )?;
        }
        Ok(())
    }

    pub fn update_route_reward(&self, route: &str, hit: bool) -> Result<()> {
        if hit {
            self.conn.execute(
                "INSERT INTO route_reward(route,wins,losses) VALUES(?1,1,1)
                 ON CONFLICT(route) DO UPDATE SET wins=wins+1",
                params![route],
            )?;
        } else {
            self.conn.execute(
                "INSERT INTO route_reward(route,wins,losses) VALUES(?1,1,1)
                 ON CONFLICT(route) DO UPDATE SET losses=losses+1",
                params![route],
            )?;
        }
        Ok(())
    }

    pub fn memory_type_bonus(&self, kind: &str) -> Result<f64> {
        let r = self.conn.query_row(
            "SELECT wins, losses FROM memory_type_reward WHERE kind=?1",
            params![kind],
            |r| Ok((r.get::<_, f64>(0)?, r.get::<_, f64>(1)?)),
        );
        let (wins, losses) = match r {
            Ok(v) => v,
            Err(rusqlite::Error::QueryReturnedNoRows) => return Ok(0.0),
            Err(e) => return Err(e.into()),
        };
        Ok((wins / (wins + losses)).clamp(0.0, 1.0) * 0.03)
    }

    pub fn get_calibration(&self, bucket: i64) -> Result<f64> {
        let r = self.conn.query_row(
            "SELECT correction FROM learn_calibration WHERE bucket=?1",
            params![bucket],
            |r| r.get::<_, f64>(0),
        );
        match r {
            Ok(v) => Ok(v),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(1.0),
            Err(e) => Err(e.into()),
        }
    }
}
