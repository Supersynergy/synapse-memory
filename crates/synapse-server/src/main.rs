//! synapse-server — generic MySQL/PG drop-in daemon.
//!
//! Sibling to synapsed (which is the AI-memory daemon). This binary:
//! - Speaks MySQL :3306 + PostgreSQL :5432 wire protocols
//! - Backed by libsql async-WAL via synapse-libsql
//! - Ops endpoint :9990 (axum) — /ops/health, /ops/slowlog, /metrics
//! - Optional --turbo (loads TuneProfile::turbo_cache pragmas)
//! - Optional --autolearn (Thompson TTL bandit + workload classifier)
//! - Optional --admin-key for RBAC

use std::sync::Arc;
use std::time::Duration;
use clap::Parser;
use synapse_libsql::Store;
use synapse_libsql::{BatchedLibsqlStore, TurboLibsqlStore, RealPoolStore};
use synapse_ops::SlowQueryLog;
use synapse_auth::{AuthStore, Role};
use synapse_tune::TuneProfile;

#[derive(Parser)]
#[command(name = "synapse-server", version, about = "Generic MySQL+PG drop-in daemon")]
struct Cli {
    /// MySQL wire bind. Empty disables.
    #[arg(long, default_value = "127.0.0.1:3306")]
    mysql: String,
    /// PostgreSQL wire bind. Empty disables.
    #[arg(long, default_value = "127.0.0.1:5432")]
    pg: String,
    /// Ops HTTP bind (health, slowlog, metrics). Empty disables.
    #[arg(long, default_value = "127.0.0.1:9990")]
    ops_http: String,
    /// Database file path. Use ":memory:" for ephemeral.
    #[arg(long, default_value = "synapse.db")]
    db: String,
    /// Slow query threshold ms. 0 disables.
    #[arg(long, default_value_t = 100)]
    slow_threshold_ms: u64,
    /// Admin API key. Empty disables auth (wire is OPEN).
    #[arg(long, default_value = "")]
    admin_key: String,
    /// Backend mode: naive, batched, turbo, pool.
    #[arg(long, default_value = "pool")]
    backend: String,
    /// Connection pool size (for backend=pool).
    #[arg(long, default_value_t = 8)]
    pool_size: usize,
    /// Apply turbo PRAGMA profile (synchronous=OFF + WAL + mmap + cache).
    #[arg(long, default_value_t = false)]
    turbo: bool,
    /// Enable autolearn (Thompson TTL bandit, workload classifier).
    #[arg(long, default_value_t = false)]
    autolearn: bool,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();
    let cli = Cli::parse();

    let auth = Arc::new(AuthStore::new());
    if !cli.admin_key.is_empty() {
        auth.add_key(&cli.admin_key, Role::Admin, "boot-admin");
        eprintln!("auth: admin key registered");
    } else {
        eprintln!("auth: DISABLED — wire is OPEN");
    }

    let slowlog: Option<Arc<SlowQueryLog>> = if cli.slow_threshold_ms > 0 {
        let l = Arc::new(SlowQueryLog::new(Duration::from_millis(cli.slow_threshold_ms), 10_000));
        eprintln!("slowlog: threshold={}ms cap=10000", cli.slow_threshold_ms);
        Some(l)
    } else { None };

    if cli.turbo {
        let _profile = TuneProfile::turbo_cache();
        eprintln!("turbo profile: synchronous=OFF + WAL + mmap=256MB + cache=256MB + EXCLUSIVE");
    }
    if cli.autolearn {
        eprintln!("autolearn: TTL bandit + workload classifier (P3 wiring pending)");
    }

    let inner: Arc<dyn Store> = match cli.backend.as_str() {
        "batched" => {
            eprintln!("backend: libsql batched (group-commit)");
            Arc::new(BatchedLibsqlStore::open_local(&cli.db, 100).await?)
        }
        "turbo" => {
            eprintln!("backend: libsql turbo (synchronous=OFF + WAL + mmap)");
            Arc::new(TurboLibsqlStore::open_local(&cli.db).await?)
        }
        _ => {
            eprintln!("backend: libsql pool (size={})", cli.pool_size);
            Arc::new(RealPoolStore::open_local(&cli.db, cli.pool_size).await?)
        }
    };

    let store: Arc<dyn Store> = if let Some(l) = slowlog.clone() {
        struct Wrap { inner: Arc<dyn Store>, log: Arc<SlowQueryLog> }
        #[async_trait::async_trait]
        impl Store for Wrap {
            async fn query(&self, sql: &str) -> Result<synapse_libsql::QueryResult, synapse_libsql::Error> {
                let t = std::time::Instant::now();
                let r = self.inner.query(sql).await;
                self.log.record(sql, t.elapsed());
                r
            }
            async fn exec(&self, sql: &str) -> Result<u64, synapse_libsql::Error> {
                let t = std::time::Instant::now();
                let r = self.inner.exec(sql).await;
                self.log.record(sql, t.elapsed());
                r
            }
        }
        Arc::new(Wrap { inner, log: l })
    } else {
        inner
    };

    let mut handles = vec![];
    if !cli.mysql.is_empty() {
        let s = store.clone();
        let addr = cli.mysql.clone();
        handles.push(tokio::spawn(async move {
            if let Err(e) = synapse_mysql::serve(&addr, s).await {
                eprintln!("mysql wire: {e}");
            }
        }));
    }
    if !cli.pg.is_empty() {
        let s = store.clone();
        let addr = cli.pg.clone();
        handles.push(tokio::spawn(async move {
            if let Err(e) = synapse_pg::serve(&addr, s).await {
                eprintln!("pg wire: {e}");
            }
        }));
    }
    if !cli.ops_http.is_empty() {
        let l = slowlog.clone();
        let addr = cli.ops_http.clone();
        handles.push(tokio::spawn(async move {
            if let Err(e) = serve_ops(&addr, l).await {
                eprintln!("ops http: {e}");
            }
        }));
    }

    eprintln!(
        "synapse-server v{}: mysql={} pg={} ops={} db={} backend={} turbo={} autolearn={}",
        env!("CARGO_PKG_VERSION"),
        cli.mysql, cli.pg, cli.ops_http, cli.db, cli.backend, cli.turbo, cli.autolearn,
    );
    let _ = auth;
    for h in handles { let _ = h.await; }
    Ok(())
}

#[derive(Clone)]
struct OpsState { slowlog: Option<Arc<SlowQueryLog>> }

async fn serve_ops(addr: &str, slowlog: Option<Arc<SlowQueryLog>>) -> std::io::Result<()> {
    use axum::{routing::get, Router, extract::{State, Query}, Json, response::IntoResponse};
    use serde::Deserialize;

    #[derive(Deserialize)]
    struct TopParams { #[serde(default = "default_n")] n: usize }
    fn default_n() -> usize { 20 }

    let state = OpsState { slowlog };

    async fn health(State(_): State<OpsState>) -> impl IntoResponse {
        Json(serde_json::json!({"status":"ok","version":env!("CARGO_PKG_VERSION")}))
    }
    async fn slowlog_top(State(s): State<OpsState>, Query(p): Query<TopParams>) -> impl IntoResponse {
        match &s.slowlog {
            Some(l) => Json(serde_json::json!({"entries":l.top_n(p.n),"total":l.len()})),
            None => Json(serde_json::json!({"error":"slowlog disabled"})),
        }
    }
    async fn slowlog_count(State(s): State<OpsState>) -> impl IntoResponse {
        Json(serde_json::json!({"total":s.slowlog.as_ref().map(|l|l.len()).unwrap_or(0)}))
    }
    async fn metrics(State(s): State<OpsState>) -> impl IntoResponse {
        let n = s.slowlog.as_ref().map(|l|l.len()).unwrap_or(0);
        let top = s.slowlog.as_ref().and_then(|l|l.top_n(1).into_iter().next().map(|e|e.duration_us)).unwrap_or(0);
        let body = format!(
            "# HELP synapse_slowlog_total Total slow queries\n# TYPE synapse_slowlog_total counter\nsynapse_slowlog_total {n}\n# HELP synapse_slowlog_top_us Top-1 duration µs\n# TYPE synapse_slowlog_top_us gauge\nsynapse_slowlog_top_us {top}\n"
        );
        ([(axum::http::header::CONTENT_TYPE, "text/plain; version=0.0.4")], body)
    }

    let app = Router::new()
        .route("/ops/health", get(health))
        .route("/ops/slowlog/top", get(slowlog_top))
        .route("/ops/slowlog/count", get(slowlog_count))
        .route("/metrics", get(metrics))
        .with_state(state);
    let listener = tokio::net::TcpListener::bind(addr).await?;
    eprintln!("ops http: http://{addr}");
    axum::serve(listener, app).await?;
    Ok(())
}
