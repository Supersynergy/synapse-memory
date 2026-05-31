//! Multi-protocol server: auto-detects MySQL / Postgres / HTTP per port.

pub mod brain_adapter;
pub mod http;
pub mod mysql;
pub mod pg;

use std::sync::Arc;
use synapse_libsql::Store;

/// Unified server handle — start all three wire protocols concurrently.
pub struct Service {
    pub store: Arc<dyn Store>,
    pub mysql_addr: String,
    pub pg_addr: String,
    pub http_addr: String,
}

impl Service {
    pub fn new(store: Arc<dyn Store>) -> Self {
        Self {
            store,
            mysql_addr: "127.0.0.1:3306".into(),
            pg_addr: "127.0.0.1:5432".into(),
            http_addr: "127.0.0.1:9477".into(),
        }
    }

    pub fn with_mysql(mut self, addr: &str) -> Self {
        self.mysql_addr = addr.into();
        self
    }
    pub fn with_pg(mut self, addr: &str) -> Self {
        self.pg_addr = addr.into();
        self
    }
    pub fn with_http(mut self, addr: &str) -> Self {
        self.http_addr = addr.into();
        self
    }

    /// Spawn all listeners; returns on first fatal error.
    pub async fn run(self) -> std::io::Result<()> {
        let s1 = self.store.clone();
        let s2 = self.store.clone();
        let ma = self.mysql_addr.clone();
        let pa = self.pg_addr.clone();

        let mysql_task = tokio::spawn(async move { synapse_mysql::serve(&ma, s1).await });
        let pg_task = tokio::spawn(async move { synapse_pg::serve(&pa, s2).await });
        let http_task = tokio::spawn(async move { http::serve(&self.http_addr).await });

        tokio::select! {
            r = mysql_task => r.map_err(std::io::Error::other)?,
            r = pg_task    => r.map_err(std::io::Error::other)?,
            r = http_task  => r.map_err(std::io::Error::other)?,
        }
    }
}
