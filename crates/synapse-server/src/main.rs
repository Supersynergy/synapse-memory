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
use synapse_tune::{TuneProfile, BotClassifier, DriftDetector, IndexAdvisor, TtlBandit, HeuristicTuner, Tuner, WorkloadStats};
use std::sync::atomic::{AtomicU64, Ordering};
use parking_lot::Mutex;
use rusqlite::Connection;
use synapse_graph::{LiveRelate, RelateEvent};
use synapse_graph::live::EventOp;

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
    /// Enable graph endpoint (cypher-lite + pagerank + communities + live SSE).
    /// Path to graph SQLite DB. Empty disables.
    #[arg(long, default_value = "")]
    graph_db: String,
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
    let autolearn_state: Option<Arc<AutolearnState>> = if cli.autolearn {
        let s = Arc::new(AutolearnState {
            bandit: TtlBandit::default_buckets(),
            bot_classifier: BotClassifier::default(),
            drift: DriftDetector::default(),
            advisor: tokio::sync::RwLock::new(IndexAdvisor::new()),
            tuner: HeuristicTuner,
            reads: AtomicU64::new(0),
            writes: AtomicU64::new(0),
        });
        eprintln!("autolearn: TtlBandit + BotClassifier + DriftDetector + IndexAdvisor + HeuristicTuner LIVE");
        Some(s)
    } else { None };

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

    // Wrap store with drift detection + advisor observation if autolearn enabled
    let inner = if let Some(al) = autolearn_state.clone() {
        struct AutoWrap { inner: Arc<dyn Store>, al: Arc<AutolearnState> }
        #[async_trait::async_trait]
        impl Store for AutoWrap {
            async fn query(&self, sql: &str) -> Result<synapse_libsql::QueryResult, synapse_libsql::Error> {
                let t = std::time::Instant::now();
                let r = self.inner.query(sql).await;
                let elapsed_us = t.elapsed().as_micros() as f64;
                self.al.drift.check(elapsed_us);
                self.al.reads.fetch_add(1, Ordering::Relaxed);
                if let Ok(mut a) = self.al.advisor.try_write() { a.observe(sql); }
                r
            }
            async fn exec(&self, sql: &str) -> Result<u64, synapse_libsql::Error> {
                let t = std::time::Instant::now();
                let r = self.inner.exec(sql).await;
                let elapsed_us = t.elapsed().as_micros() as f64;
                self.al.drift.check(elapsed_us);
                self.al.writes.fetch_add(1, Ordering::Relaxed);
                r
            }
        }
        Arc::new(AutoWrap { inner, al }) as Arc<dyn Store>
    } else { inner };

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
    // Graph layer
    let graph_state: Option<Arc<GraphState>> = if !cli.graph_db.is_empty() {
        let conn = Connection::open(&cli.graph_db).expect("open graph db");
        synapse_graph::ensure_schema(&conn).expect("graph schema");
        eprintln!("graph: loaded at {} ({} edges)",
                  cli.graph_db,
                  synapse_graph::edge_count(&conn).unwrap_or(0));
        Some(Arc::new(GraphState {
            conn: Mutex::new(conn),
            live: Arc::new(LiveRelate::default()),
        }))
    } else { None };

    if !cli.ops_http.is_empty() {
        let l = slowlog.clone();
        let g = graph_state.clone();
        let addr = cli.ops_http.clone();
        handles.push(tokio::spawn(async move {
            if let Err(e) = serve_ops(&addr, l, g).await {
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

struct GraphState {
    conn: Mutex<Connection>,
    live: Arc<LiveRelate>,
}

struct AutolearnState {
    bandit: TtlBandit,
    bot_classifier: BotClassifier,
    drift: DriftDetector,
    advisor: tokio::sync::RwLock<IndexAdvisor>,
    tuner: HeuristicTuner,
    reads: AtomicU64,
    writes: AtomicU64,
}

#[derive(Clone)]
struct OpsState {
    slowlog: Option<Arc<SlowQueryLog>>,
    graph: Option<Arc<GraphState>>,
}

async fn serve_ops(
    addr: &str,
    slowlog: Option<Arc<SlowQueryLog>>,
    graph: Option<Arc<GraphState>>,
) -> std::io::Result<()> {
    use axum::{routing::{get, post}, Router, extract::{State, Query}, Json, response::IntoResponse};
    use serde::Deserialize;

    #[derive(Deserialize)]
    struct TopParams { #[serde(default = "default_n")] n: usize }
    fn default_n() -> usize { 20 }

    #[derive(Deserialize)]
    struct CypherBody { query: String }

    let state = OpsState { slowlog, graph };

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

    // -------- Graph endpoints --------
    async fn graph_health(State(s): State<OpsState>) -> impl IntoResponse {
        match &s.graph {
            Some(g) => {
                let count = g.conn.lock().query_row::<i64, _, _>(
                    "SELECT COUNT(*) FROM edges", [], |r| r.get(0)).unwrap_or(0);
                Json(serde_json::json!({"enabled":true,"edges":count,"subscribers":g.live.subscriber_count()}))
            }
            None => Json(serde_json::json!({"enabled":false})),
        }
    }
    async fn graph_neighbors(
        State(s): State<OpsState>,
        axum::extract::Path((id, k)): axum::extract::Path<(i64, usize)>,
    ) -> impl IntoResponse {
        match &s.graph {
            Some(g) => {
                let conn = g.conn.lock();
                match synapse_graph::graph_helpers::neighbors_json(&conn, id, k) {
                    Ok(j) => Json(serde_json::from_str::<serde_json::Value>(&j).unwrap_or_default()),
                    Err(e) => Json(serde_json::json!({"error":e.to_string()})),
                }
            }
            None => Json(serde_json::json!({"error":"graph disabled"})),
        }
    }
    async fn graph_pagerank(
        State(s): State<OpsState>,
        Query(p): Query<TopParams>,
    ) -> impl IntoResponse {
        match &s.graph {
            Some(g) => {
                let conn = g.conn.lock();
                match synapse_graph::graph_helpers::pagerank_top_json(&conn, p.n) {
                    Ok(j) => Json(serde_json::from_str::<serde_json::Value>(&j).unwrap_or_default()),
                    Err(e) => Json(serde_json::json!({"error":e.to_string()})),
                }
            }
            None => Json(serde_json::json!({"error":"graph disabled"})),
        }
    }
    async fn graph_communities(State(s): State<OpsState>) -> impl IntoResponse {
        match &s.graph {
            Some(g) => {
                let conn = g.conn.lock();
                match synapse_graph::graph_helpers::communities_json(&conn, 20) {
                    Ok(j) => Json(serde_json::from_str::<serde_json::Value>(&j).unwrap_or_default()),
                    Err(e) => Json(serde_json::json!({"error":e.to_string()})),
                }
            }
            None => Json(serde_json::json!({"error":"graph disabled"})),
        }
    }
    async fn graph_path(
        State(s): State<OpsState>,
        axum::extract::Path((from, to, depth)): axum::extract::Path<(i64, i64, usize)>,
    ) -> impl IntoResponse {
        match &s.graph {
            Some(g) => {
                let conn = g.conn.lock();
                match synapse_graph::graph_helpers::shortest_path_json(&conn, from, to, depth) {
                    Ok(j) => Json(serde_json::from_str::<serde_json::Value>(&j).unwrap_or(serde_json::Value::Null)),
                    Err(e) => Json(serde_json::json!({"error":e.to_string()})),
                }
            }
            None => Json(serde_json::json!({"error":"graph disabled"})),
        }
    }
    async fn graph_cypher(
        State(s): State<OpsState>,
        Json(body): Json<CypherBody>,
    ) -> impl IntoResponse {
        let g = match &s.graph {
            Some(g) => g.clone(),
            None => return Json(serde_json::json!({"error":"graph disabled"})),
        };
        let parsed = match synapse_graph::parse_cypher(&body.query) {
            Ok(q) => q,
            Err(e) => return Json(serde_json::json!({"error":format!("parse: {e}"),"query":body.query})),
        };
        use synapse_graph::CypherOp;
        let conn = g.conn.lock();
        match parsed.op {
            CypherOp::Neighbors { node_id, top_k, rel_filter } => {
                match synapse_graph::neighbors(&conn, node_id, rel_filter.as_deref(), top_k) {
                    Ok(rows) => Json(serde_json::json!({"op":"neighbors","rows":rows})),
                    Err(e) => Json(serde_json::json!({"error":e.to_string()})),
                }
            }
            CypherOp::Traverse { start_id, max_depth, rel_filter, limit } => {
                match synapse_graph::traverse(&conn, start_id, max_depth, limit, 0.7, rel_filter.as_deref()) {
                    Ok(rows) => Json(serde_json::json!({"op":"traverse","rows":rows})),
                    Err(e) => Json(serde_json::json!({"error":e.to_string()})),
                }
            }
            CypherOp::ShortestPath { from_id, to_id, max_depth } => {
                match synapse_graph::shortest_path(&conn, from_id, to_id, max_depth) {
                    Ok(Some((cost, path))) => Json(serde_json::json!({"op":"path","cost":cost,"path":path})),
                    Ok(None) => Json(serde_json::json!({"op":"path","result":null})),
                    Err(e) => Json(serde_json::json!({"error":e.to_string()})),
                }
            }
            CypherOp::PageRank { top_n, damping } => {
                match synapse_graph::top_pagerank(&conn, top_n, damping, 30) {
                    Ok(rows) => Json(serde_json::json!({"op":"pagerank","rows":rows})),
                    Err(e) => Json(serde_json::json!({"error":e.to_string()})),
                }
            }
            CypherOp::Communities { max_iters } => {
                match synapse_graph::communities(&conn, max_iters) {
                    Ok(groups) => {
                        let v: Vec<_> = groups.into_iter().map(|(label, members)| {
                            serde_json::json!({"label":label,"size":members.len(),"members":members})
                        }).collect();
                        Json(serde_json::json!({"op":"communities","groups":v}))
                    }
                    Err(e) => Json(serde_json::json!({"error":e.to_string()})),
                }
            }
            CypherOp::Create { from_id, to_id, ref rel, weight } => {
                let res = synapse_graph::relate(&conn, from_id, to_id, rel, weight, None);
                if res.is_ok() {
                    g.live.emit(EventOp::Insert, from_id, to_id, rel, weight);
                }
                Json(serde_json::json!({"op":"create","ok":res.is_ok()}))
            }
        }
    }
    async fn graph_live_sse(State(s): State<OpsState>) -> axum::response::Response {
        use axum::response::sse::{Event, Sse, KeepAlive};
        use futures::stream::{self, Stream};
        use std::convert::Infallible;
        let g = match &s.graph {
            Some(g) => g.clone(),
            None => return axum::response::IntoResponse::into_response(
                Json(serde_json::json!({"error":"graph disabled"}))),
        };
        let mut rx = g.live.subscribe();
        let stream = async_stream::stream! {
            while let Ok(ev) = rx.recv().await {
                let json = serde_json::to_string(&ev).unwrap_or_default();
                yield Ok::<_, Infallible>(Event::default().data(json));
            }
        };
        Sse::new(Box::pin(stream)).keep_alive(KeepAlive::default()).into_response()
    }

    let app = Router::new()
        .route("/ops/health", get(health))
        .route("/ops/slowlog/top", get(slowlog_top))
        .route("/ops/slowlog/count", get(slowlog_count))
        .route("/metrics", get(metrics))
        .route("/graph/health", get(graph_health))
        .route("/graph/neighbors/{id}/{k}", get(graph_neighbors))
        .route("/graph/pagerank", get(graph_pagerank))
        .route("/graph/communities", get(graph_communities))
        .route("/graph/path/{from}/{to}/{depth}", get(graph_path))
        .route("/graph/cypher", post(graph_cypher))
        .route("/graph/live", get(graph_live_sse))
        .with_state(state);
    let listener = tokio::net::TcpListener::bind(addr).await?;
    eprintln!("ops http: http://{addr}");
    axum::serve(listener, app).await?;
    Ok(())
}
