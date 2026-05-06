//! Unix socket server with 4-byte LE length-prefix + msgpack frames.

use std::path::Path;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};

use crate::cache::{CacheKey, T0Cache};
use crate::embed::Embedder;
use crate::index::SharedIndex;

#[derive(Debug, Deserialize, Default, Clone, Copy, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum SocketMode {
    #[default]
    BinaryFirst,
    Strict,
    BinaryOnly,
    #[cfg(feature = "hnsw")]
    Hnsw,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "op", content = "args")]
pub enum UltraRequest {
    Ping,
    Search { q: String, limit: usize, #[serde(default)] mode: SocketMode },
    Stats,
}

#[derive(Debug, Serialize)]
#[serde(tag = "type", content = "data")]
pub enum UltraResponse {
    Pong,
    Hits(Vec<HitMsg>),
    Stats { rows: usize, cache_size: usize },
    Err(String),
}

#[derive(Debug, Serialize)]
pub struct HitMsg {
    pub id: i64,
    pub score: f32,
}

pub async fn serve(
    sock_path: &str,
    index: SharedIndex,
    cache: Arc<T0Cache>,
    embedder: Arc<Embedder>,
) -> crate::error::Result<()> {
    let path = Path::new(sock_path);
    if path.exists() {
        std::fs::remove_file(path)?;
    }
    let listener = UnixListener::bind(path)?;
    tracing::info!("Unix socket listening at {}", sock_path);

    loop {
        match listener.accept().await {
            Ok((stream, _)) => {
                let idx = Arc::clone(&index);
                let cache = Arc::clone(&cache);
                let emb = Arc::clone(&embedder);
                tokio::spawn(async move {
                    if let Err(e) = handle_conn(stream, idx, cache, emb).await {
                        tracing::debug!("socket conn closed: {}", e);
                    }
                });
            }
            Err(e) => tracing::error!("accept error: {}", e),
        }
    }
}

async fn handle_conn(
    mut stream: UnixStream,
    index: SharedIndex,
    cache: Arc<T0Cache>,
    embedder: Arc<Embedder>,
) -> crate::error::Result<()> {
    loop {
        let mut len_buf = [0u8; 4];
        match stream.read_exact(&mut len_buf).await {
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(e) => return Err(e.into()),
        }
        let len = u32::from_le_bytes(len_buf) as usize;
        if len == 0 || len > 1_048_576 { break; }
        let mut body = vec![0u8; len];
        stream.read_exact(&mut body).await?;

        let req: UltraRequest = match rmp_serde::from_slice(&body) {
            Ok(r) => r,
            Err(e) => { send_resp(&mut stream, &UltraResponse::Err(e.to_string())).await?; continue; }
        };

        let resp = dispatch(req, &index, &cache, &embedder).await;
        send_resp(&mut stream, &resp).await?;
    }
    Ok(())
}

async fn dispatch(req: UltraRequest, index: &SharedIndex, cache: &Arc<T0Cache>, embedder: &Arc<Embedder>) -> UltraResponse {
    match req {
        UltraRequest::Ping => UltraResponse::Pong,
        UltraRequest::Stats => {
            let g = index.load();
            UltraResponse::Stats { rows: g.n_rows(), cache_size: cache.len() }
        }
        UltraRequest::Search { q, limit, mode } => {
            let mode_byte: u8 = match mode {
                SocketMode::BinaryFirst => 1,
                SocketMode::Strict => 2,
                SocketMode::BinaryOnly => 3,
                #[cfg(feature = "hnsw")]
                SocketMode::Hnsw => 4,
            };
            let key = CacheKey::new(&q, mode_byte, limit as u16);
            if let Some(hits) = cache.get(&key) {
                return UltraResponse::Hits(hits.iter().map(|h| HitMsg { id: h.id, score: h.score }).collect());
            }
            match embedder.embed(&q) {
                Err(e) => UltraResponse::Err(e.to_string()),
                Ok(emb) => {
                    let g = index.load();
                    let hits = match mode {
                        SocketMode::BinaryFirst => g.search_binary_first(&emb, limit),
                        SocketMode::Strict => g.search_strict(&emb, limit),
                        SocketMode::BinaryOnly => g.search_binary_only(&emb, limit),
                        #[cfg(feature = "hnsw")]
                        SocketMode::Hnsw => g.search_hnsw(&emb, limit),
                    };
                    cache.put(key, hits.clone());
                    UltraResponse::Hits(hits.iter().map(|h| HitMsg { id: h.id, score: h.score }).collect())
                }
            }
        }
    }
}

async fn send_resp(stream: &mut UnixStream, resp: &UltraResponse) -> crate::error::Result<()> {
    let bytes = rmp_serde::to_vec_named(resp).map_err(|e| crate::error::UltraError::Anyhow(anyhow::anyhow!(e)))?;
    stream.write_all(&(bytes.len() as u32).to_le_bytes()).await?;
    stream.write_all(&bytes).await?;
    Ok(())
}
