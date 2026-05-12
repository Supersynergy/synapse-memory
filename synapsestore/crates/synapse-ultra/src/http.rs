use std::sync::Arc;

use axum::extract::{Query, State};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use tokio::net::TcpListener;

use crate::cache::{CacheKey, T0Cache};
use crate::embed::Embedder;
use crate::index::{Hit, SharedIndex};

pub struct AppState {
    pub index: SharedIndex,
    pub cache: Arc<T0Cache>,
    pub embedder: Arc<Embedder>,
}

#[derive(Deserialize)]
pub struct VecQuery {
    pub q: String,
    #[serde(default = "default_limit")]
    pub limit: usize,
    /// "binary_first" (default), "strict", "binary_only"
    #[serde(default)]
    pub mode: SearchMode,
}

#[derive(Deserialize, Default, Clone, Copy)]
#[serde(rename_all = "snake_case")]
pub enum SearchMode {
    #[default]
    BinaryFirst,
    Strict,
    BinaryOnly,
    #[cfg(feature = "hnsw")]
    Hnsw,
}

fn default_limit() -> usize {
    10
}

#[derive(Serialize)]
pub struct HitResp {
    pub id: i64,
    pub score: f32,
}

#[derive(Serialize)]
pub struct StatsResp {
    pub rows: usize,
    pub cache_size: usize,
    pub version: &'static str,
}

#[derive(Deserialize)]
pub struct BatchQuery {
    pub queries: Vec<String>,
    #[serde(default = "default_limit")]
    pub limit: usize,
}

pub async fn build_router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/ping", get(ping))
        .route("/vec", get(vec_search))
        .route("/find", get(vec_search))
        .route("/hybrid", get(vec_search))
        .route("/vec_batch", post(vec_batch))
        .route("/vec_raw", post(vec_raw))
        .route("/vec_raw_batch", post(vec_raw_batch))
        .route("/stats", get(stats))
        .with_state(state)
}

async fn ping() -> &'static str {
    "pong"
}

async fn vec_search(
    State(state): State<Arc<AppState>>,
    Query(params): Query<VecQuery>,
) -> impl IntoResponse {
    let mode_byte = match params.mode {
        SearchMode::BinaryFirst => 1u8,
        SearchMode::Strict => 2u8,
        SearchMode::BinaryOnly => 3u8,
        #[cfg(feature = "hnsw")]
        SearchMode::Hnsw => 4u8,
    };
    let key = CacheKey::new(&params.q, mode_byte, params.limit as u16);

    if let Some(hits) = state.cache.get(&key) {
        return Json(hits_to_resp(&hits));
    }

    let emb = match state.embedder.embed(&params.q) {
        Ok(e) => e,
        Err(e) => {
            tracing::error!("embed: {}", e);
            return Json(vec![]);
        }
    };

    let guard = state.index.load();
    let hits = match params.mode {
        SearchMode::BinaryFirst => guard.search_binary_first(&emb, params.limit),
        SearchMode::Strict => guard.search_strict(&emb, params.limit),
        SearchMode::BinaryOnly => guard.search_binary_only(&emb, params.limit),
        #[cfg(feature = "hnsw")]
        SearchMode::Hnsw => guard.search_hnsw(&emb, params.limit),
    };

    state.cache.put(key, hits.clone());
    Json(hits_to_resp(&hits))
}

async fn vec_batch(
    State(state): State<Arc<AppState>>,
    Json(body): Json<BatchQuery>,
) -> impl IntoResponse {
    let results: Vec<Vec<HitResp>> = body
        .queries
        .iter()
        .map(|q| {
            let key = CacheKey::new(q, 1, body.limit as u16);
            if let Some(hits) = state.cache.get(&key) {
                return hits_to_resp(&hits);
            }
            match state.embedder.embed(q) {
                Err(_) => vec![],
                Ok(emb) => {
                    let guard = state.index.load();
                    let hits = guard.search_binary_first(&emb, body.limit);
                    state.cache.put(key, hits.clone());
                    hits_to_resp(&hits)
                }
            }
        })
        .collect();
    Json(results)
}

#[derive(Deserialize)]
pub struct VecRawQuery {
    pub vec: Vec<f32>,
    #[serde(default = "default_limit")]
    pub limit: usize,
    #[serde(default)]
    pub mode: SearchMode,
    /// Auto-route to optimal backend by target recall.
    /// ≥0.98 → Strict (cascade, R=0.994, 697 QPS)
    /// 0.94-0.97 → Hnsw  (R=0.982, 1631 QPS)
    /// <0.94 → BinaryFirst (R=0.888, 4845 QPS)
    /// When set, overrides `mode`.
    pub target_recall: Option<f32>,
}

fn recall_to_mode(target_recall: f32) -> SearchMode {
    if target_recall >= 0.98 {
        SearchMode::Strict
    } else if target_recall >= 0.94 {
        #[cfg(feature = "hnsw")]
        return SearchMode::Hnsw;
        #[cfg(not(feature = "hnsw"))]
        return SearchMode::Strict;
    } else {
        SearchMode::BinaryFirst
    }
}

/// Raw-vector search — bypasses embedder. For benchmarks + bring-your-own-embedding.
/// Supports `target_recall` param to auto-select backend (overrides `mode`).
async fn vec_raw(
    State(state): State<Arc<AppState>>,
    Json(body): Json<VecRawQuery>,
) -> impl IntoResponse {
    let effective_mode = body.target_recall.map(recall_to_mode).unwrap_or(body.mode);
    let guard = state.index.load();
    let hits = match effective_mode {
        SearchMode::BinaryFirst => guard.search_binary_first(&body.vec, body.limit),
        SearchMode::Strict => guard.search_strict(&body.vec, body.limit),
        SearchMode::BinaryOnly => guard.search_binary_only(&body.vec, body.limit),
        #[cfg(feature = "hnsw")]
        SearchMode::Hnsw => guard.search_hnsw(&body.vec, body.limit),
    };
    Json(hits_to_resp(&hits))
}

#[derive(Deserialize)]
pub struct VecRawBatchQuery {
    pub vecs: Vec<Vec<f32>>,
    #[serde(default = "default_limit")]
    pub limit: usize,
    #[serde(default)]
    pub mode: SearchMode,
}

/// Batch raw-vector search — amortizes HTTP overhead across N queries.
/// Returns Vec<Vec<HitResp>>, one inner vec per query.
///
/// Fast path: when mode == Strict and batch ≥ 8, routes through
/// `search_batch_blas` which issues a single GEMM call (Accelerate/AMX on
/// macOS, ndarray on Linux) instead of N sequential per-query scans.
async fn vec_raw_batch(
    State(state): State<Arc<AppState>>,
    Json(body): Json<VecRawBatchQuery>,
) -> impl IntoResponse {
    let guard = state.index.load();

    // BLAS GEMM fast path: Strict mode + batch ≥ 8.
    if matches!(body.mode, SearchMode::Strict) && body.vecs.len() >= 8 {
        let batch_hits = guard.search_batch_blas(&body.vecs, body.limit);
        let results: Vec<Vec<HitResp>> = batch_hits.iter().map(|hits| hits_to_resp(hits)).collect();
        return Json(results);
    }

    let results: Vec<Vec<HitResp>> = body
        .vecs
        .iter()
        .map(|vec| {
            let hits = match body.mode {
                SearchMode::BinaryFirst => guard.search_binary_first(vec, body.limit),
                SearchMode::Strict => guard.search_strict(vec, body.limit),
                SearchMode::BinaryOnly => guard.search_binary_only(vec, body.limit),
                #[cfg(feature = "hnsw")]
                SearchMode::Hnsw => guard.search_hnsw(vec, body.limit),
            };
            hits_to_resp(&hits)
        })
        .collect();
    Json(results)
}

async fn stats(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let guard = state.index.load();
    Json(StatsResp {
        rows: guard.n_rows(),
        cache_size: state.cache.len(),
        version: env!("CARGO_PKG_VERSION"),
    })
}

fn hits_to_resp(hits: &[Hit]) -> Vec<HitResp> {
    hits.iter()
        .map(|h| HitResp {
            id: h.id,
            score: h.score,
        })
        .collect()
}

pub async fn serve(state: Arc<AppState>, bind: &str) -> crate::error::Result<()> {
    let router = build_router(state).await;
    let listener = TcpListener::bind(bind).await?;
    tracing::info!("HTTP server listening on {}", bind);
    axum::serve(listener, router).await?;
    Ok(())
}
