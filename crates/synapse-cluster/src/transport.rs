//! TCP transport: length-prefix framing (4-byte LE u32 + JSON payload).

use std::sync::Arc;

use anyhow::{bail, Result};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tracing::debug;

use synapse_core::sync::{Op, OpId};

use crate::proto::{Request, Response};
use crate::{Clock, Node, PeerInfo};

const MAX_FRAME: u32 = 16 * 1024 * 1024; // 16 MiB

async fn send_msg<T: serde::Serialize>(stream: &mut TcpStream, msg: &T) -> Result<()> {
    let bytes = serde_json::to_vec(msg)?;
    let len = bytes.len() as u32;
    if len > MAX_FRAME {
        bail!("frame too large: {len}");
    }
    stream.write_all(&len.to_le_bytes()).await?;
    stream.write_all(&bytes).await?;
    Ok(())
}

async fn recv_msg<T: serde::de::DeserializeOwned>(stream: &mut TcpStream) -> Result<T> {
    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf).await?;
    let len = u32::from_le_bytes(len_buf);
    if len > MAX_FRAME {
        bail!("frame too large: {len}");
    }
    let mut buf = vec![0u8; len as usize];
    stream.read_exact(&mut buf).await?;
    Ok(serde_json::from_slice(&buf)?)
}

/// Server-side: handle one incoming connection.
pub async fn handle_connection(mut stream: TcpStream, node: Arc<Node>) -> Result<()> {
    let req: Request = recv_msg(&mut stream).await?;
    match req {
        Request::Hello { from } => {
            debug!(from = %from, "Hello");
            send_msg(&mut stream, &Response::Ok).await?;
        }
        Request::PullChanges { since } => {
            let st = node.state.read().await;
            let ops: Vec<(OpId, Op)> = st
                .ops
                .iter()
                .filter(|(c, _, _)| *c > since)
                .map(|(_, id, op)| (*id, op.clone()))
                .collect();
            send_msg(&mut stream, &Response::Changes { ops }).await?;
        }
        Request::PushChanges { ops } => {
            node.merge_peer_delta(ops).await;
            send_msg(&mut stream, &Response::Ok).await?;
        }
    }
    Ok(())
}

/// Client-side: connect to `peer` and pull ops since `since`.
pub async fn pull_from_peer(peer: &PeerInfo, since: Clock) -> Result<Vec<(OpId, Op)>> {
    let mut stream = TcpStream::connect(peer.addr).await?;
    send_msg(&mut stream, &Request::PullChanges { since }).await?;
    let resp: Response = recv_msg(&mut stream).await?;
    match resp {
        Response::Changes { ops } => Ok(ops),
        Response::Err { msg } => bail!("peer error: {msg}"),
        _ => bail!("unexpected response"),
    }
}

/// Client-side: push ops to `peer` and wait for acknowledgement.
pub async fn push_to_peer(peer: &PeerInfo, ops: Vec<(OpId, Op)>) -> Result<()> {
    let mut stream = TcpStream::connect(peer.addr).await?;
    send_msg(&mut stream, &Request::PushChanges { ops }).await?;
    let _resp: Response = recv_msg(&mut stream).await?;
    Ok(())
}
