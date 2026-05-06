//! Unix Domain Socket server with 4-byte LE length-prefix + bincode frames.
//! Accepts pre-computed f32 vectors — zero embedding overhead.
//! Frame: [4-byte LE len][bincode(VecRawQuery)]
//! Response: [4-byte LE len][bincode(Vec<(i64, f32)>)]

use std::path::Path;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};

use crate::index::SharedIndex;

#[derive(Debug, Serialize, Deserialize)]
pub struct VecRawQuery {
    pub vec: Vec<f32>,
    pub limit: u32,
    pub mode: u8, // 1=binary_first 2=strict 3=binary_only
}

pub async fn serve(sock_path: &str, index: SharedIndex) -> crate::error::Result<()> {
    let path = Path::new(sock_path);
    if path.exists() {
        std::fs::remove_file(path)?;
    }
    let listener = UnixListener::bind(path)?;
    tracing::info!("Vec UDS listening at {}", sock_path);

    loop {
        match listener.accept().await {
            Ok((stream, _)) => {
                let idx = Arc::clone(&index);
                tokio::spawn(async move {
                    if let Err(e) = handle_conn(stream, idx).await {
                        tracing::debug!("vec socket conn closed: {}", e);
                    }
                });
            }
            Err(e) => tracing::error!("vec socket accept error: {}", e),
        }
    }
}

async fn handle_conn(mut stream: UnixStream, index: SharedIndex) -> crate::error::Result<()> {
    loop {
        let mut len_buf = [0u8; 4];
        match stream.read_exact(&mut len_buf).await {
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(e) => return Err(e.into()),
        }
        let len = u32::from_le_bytes(len_buf) as usize;
        if len == 0 || len > 4_194_304 {
            break;
        }
        let mut body = vec![0u8; len];
        stream.read_exact(&mut body).await?;

        let resp_bytes = match bincode::deserialize::<VecRawQuery>(&body) {
            Err(e) => {
                let err = format!("deserialize error: {e}");
                bincode::serialize::<Vec<(i64, f32)>>(&vec![]).unwrap_or_default();
                err.into_bytes()
            }
            Ok(q) => {
                let hits = {
                    let g = index.load();
                    match q.mode {
                        2 => g.search_strict(&q.vec, q.limit as usize),
                        3 => g.search_binary_only(&q.vec, q.limit as usize),
                        _ => g.search_binary_first(&q.vec, q.limit as usize),
                    }
                };
                let pairs: Vec<(i64, f32)> = hits.into_iter().map(|h| (h.id, h.score)).collect();
                bincode::serialize(&pairs).unwrap_or_default()
            }
        };

        stream.write_all(&(resp_bytes.len() as u32).to_le_bytes()).await?;
        stream.write_all(&resp_bytes).await?;
    }
    Ok(())
}
