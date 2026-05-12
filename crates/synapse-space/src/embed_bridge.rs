//! Embed bridge: call synapsed unix socket for text→f32[384] embedding.
//!
//! Protocol: [u32 LE length][msgpack body] — matches synapsed wire format.
//! Socket: `/tmp/synapse.sock` (synapsed default `--sock`).
//!
//! If the daemon is not running, `embed_text` returns `Err` immediately with
//! a clear message — no silent FTS fallback.

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::UnixStream,
};

/// Default socket path — must match synapsed `--sock`.
pub const SOCK: &str = "/tmp/synapse.sock";

/// Subset of synapsed wire protocol (mirrors proto.rs exactly).
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "op", content = "args")]
enum Request {
    Ping,
    Embed { text: String },
}

#[derive(Debug, Serialize, Deserialize)]
enum Response {
    Pong,
    Embed {
        vec: Vec<f32>,
    },
    Err(String),
    // Catch-all for other variants we don't use here
    #[serde(other)]
    Unknown,
}

/// Send one msgpack-framed RPC and return the response.
async fn rpc(sock: &str, req: &Request) -> Result<Response> {
    let mut stream = match UnixStream::connect(sock).await {
        Ok(s) => s,
        Err(e) => bail!("synapsed not running at {sock}: {e}. Start with: synapsed --sock {sock}"),
    };
    let encoded = rmp_serde::to_vec_named(req)?;
    stream
        .write_all(&(encoded.len() as u32).to_le_bytes())
        .await?;
    stream.write_all(&encoded).await?;
    stream.flush().await?;
    let mut lenbuf = [0u8; 4];
    stream.read_exact(&mut lenbuf).await?;
    let len = u32::from_le_bytes(lenbuf) as usize;
    let mut buf = vec![0u8; len];
    stream.read_exact(&mut buf).await?;
    Ok(rmp_serde::from_slice::<Response>(&buf)?)
}

/// Embed a text string via synapsed. Returns raw f32[384] vector.
///
/// Returns `Err` if daemon not running — does NOT fall back to FTS-only.
pub async fn embed_text(text: &str) -> Result<Vec<f32>> {
    embed_text_on(SOCK, text).await
}

/// Like `embed_text` but with an explicit socket path (for testing).
pub async fn embed_text_on(sock: &str, text: &str) -> Result<Vec<f32>> {
    match rpc(
        sock,
        &Request::Embed {
            text: text.to_string(),
        },
    )
    .await?
    {
        Response::Embed { vec } => Ok(vec),
        Response::Err(e) => bail!("synapsed embed error: {e}"),
        other => bail!("unexpected response from synapsed: {:?}", other),
    }
}

/// Cosine similarity between two equal-length f32 vectors.
pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let na: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let nb: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if na == 0.0 || nb == 0.0 {
        return 0.0;
    }
    dot / (na * nb)
}
