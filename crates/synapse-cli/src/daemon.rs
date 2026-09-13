//! synapsed socket client — warm path for context/prime without `Store::open`.
//! Wire: length-prefixed msgpack, JSON-shaped `{"op":..,"args":{..}}` frames —
//! same protocol synapse-mcp uses. Every call fails fast and callers fall back
//! to a local `Store::open`, so a missing/stale daemon never breaks the CLI.

use std::io::{Read, Write};
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result};
use serde_json::{Value, json};
use synapse_core::Hit;

use crate::SearchBestEffortResult;

const MAX_FRAME: usize = 256 * 1024 * 1024;

fn sock_path() -> PathBuf {
    std::env::var("SYNAPSE_SOCK")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/tmp/synapse.sock"))
}

static DISABLED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Process-wide opt-out used by the global `--no-daemon` flag.
pub fn set_disabled(v: bool) {
    DISABLED.store(v, std::sync::atomic::Ordering::Relaxed);
}

pub fn disabled() -> bool {
    DISABLED.load(std::sync::atomic::Ordering::Relaxed)
        || matches!(
            std::env::var("SYNAPSE_NO_DAEMON").as_deref(),
            Ok("1" | "true" | "yes")
        )
}

#[cfg(unix)]
fn roundtrip(stream: &mut std::os::unix::net::UnixStream, req: &Value) -> Result<Value> {
    let body = rmp_serde::to_vec_named(req)?;
    stream.write_all(&(body.len() as u32).to_le_bytes())?;
    stream.write_all(&body)?;
    stream.flush()?;
    let mut hdr = [0u8; 4];
    stream.read_exact(&mut hdr)?;
    let n = u32::from_le_bytes(hdr) as usize;
    anyhow::ensure!(n <= MAX_FRAME, "daemon frame too large: {n} bytes");
    let mut buf = vec![0u8; n];
    stream.read_exact(&mut buf)?;
    Ok(rmp_serde::from_slice(&buf)?)
}

#[cfg(unix)]
fn call(req: &Value) -> Result<Value> {
    let path = sock_path();
    let mut stream = std::os::unix::net::UnixStream::connect(&path)
        .with_context(|| format!("connect {}", path.display()))?;
    stream.set_read_timeout(Some(Duration::from_secs(30)))?;
    stream.set_write_timeout(Some(Duration::from_secs(10)))?;
    if let Some(token) = std::env::var("SYNAPSE_API_KEY")
        .ok()
        .filter(|t| !t.is_empty())
    {
        let auth = roundtrip(
            &mut stream,
            &json!({"op": "Auth", "args": {"token": token}}),
        )?;
        if let Some(err) = auth.get("Err").and_then(|v| v.as_str()) {
            anyhow::bail!("daemon auth failed: {err}");
        }
    }
    roundtrip(&mut stream, req)
}

/// True when a synapsed answers on the socket. Used by doctor/onboard status.
#[cfg(unix)]
pub fn available() -> bool {
    if disabled() {
        return false;
    }
    let path = sock_path();
    if !path.exists() {
        return false;
    }
    call(&json!({"op": "Ping"}))
        .map(|v| v.get("Pong").is_some())
        .unwrap_or(false)
}

#[cfg(not(unix))]
pub fn available() -> bool {
    false
}

fn hits(resp: Value) -> Result<Vec<Hit>> {
    if let Some(err) = resp.get("Err").and_then(|v| v.as_str()) {
        anyhow::bail!("daemon error: {err}");
    }
    let list = resp.get("Hits").cloned().unwrap_or(Value::Null);
    Ok(serde_json::from_value(list).unwrap_or_default())
}

/// Daemon search mirroring `search_best_effort`: lexical → hybrid → timeline.
/// Returns (hits, route). Any failure propagates so callers can fall back.
#[cfg(unix)]
pub fn search_best_effort(query: &str, limit: usize) -> Result<SearchBestEffortResult> {
    let lex = hits(call(&json!({"op": "Search", "args": {
        "mode": "Lex", "q": query, "limit": limit, "embed_query": false,
    }}))?)?;
    if !lex.is_empty() {
        return Ok((lex, "lexical".to_string()));
    }

    let hyb = hits(call(&json!({"op": "Search", "args": {
        "mode": "Hybrid", "q": query, "limit": limit, "embed_query": true,
    }}))?)?;
    if !hyb.is_empty() {
        return Ok((hyb, "hybrid".to_string()));
    }

    let docs = call(&json!({"op": "Timeline", "args": {"limit": limit, "offset": 0}}))?;
    if let Some(err) = docs.get("Err").and_then(|v| v.as_str()) {
        anyhow::bail!("daemon error: {err}");
    }
    let list = docs.get("Docs").cloned().unwrap_or(Value::Null);
    let docs: Vec<synapse_core::Doc> = serde_json::from_value(list).unwrap_or_default();
    Ok((
        docs.into_iter()
            .map(|d| Hit {
                id: d.id,
                uri: d.uri,
                title: d.title,
                text: d.text,
                score: 0.0,
                meta: d.meta,
                ts: Some(d.ts),
            })
            .collect(),
        "timeline".to_string(),
    ))
}

#[cfg(not(unix))]
pub fn search_best_effort(_query: &str, _limit: usize) -> Result<SearchBestEffortResult> {
    anyhow::bail!("synapsed socket is unix-only; use the local store path")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hits_propagates_daemon_error() {
        let err = hits(json!({"Err": "boom"})).unwrap_err();
        assert!(err.to_string().contains("boom"));
    }

    #[test]
    fn hits_decodes_hit_list() {
        let resp = json!({"Hits": [{
            "id": 7, "uri": "mem://x", "title": "t", "text": "body",
            "score": 1.5, "meta": {"kind": "fact"}, "ts": 42
        }]});
        let out = hits(resp).unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].id, 7);
        assert_eq!(out[0].ts, Some(42));
    }

    #[test]
    fn hits_tolerates_missing_key() {
        assert!(hits(json!({})).unwrap().is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn roundtrip_decodes_length_prefixed_msgpack() {
        let (mut a, mut b) = std::os::unix::net::UnixStream::pair().unwrap();
        let handle = std::thread::spawn(move || {
            let mut hdr = [0u8; 4];
            a.read_exact(&mut hdr).unwrap();
            let n = u32::from_le_bytes(hdr) as usize;
            let mut buf = vec![0u8; n];
            a.read_exact(&mut buf).unwrap();
            let req: Value = rmp_serde::from_slice(&buf).unwrap();
            assert_eq!(req["op"], "Ping");
            let body = rmp_serde::to_vec_named(&json!({"Pong": true})).unwrap();
            a.write_all(&(body.len() as u32).to_le_bytes()).unwrap();
            a.write_all(&body).unwrap();
        });
        let resp = roundtrip(&mut b, &json!({"op": "Ping"})).unwrap();
        handle.join().unwrap();
        assert_eq!(resp["Pong"], true);
    }

    #[cfg(unix)]
    #[test]
    fn roundtrip_rejects_oversized_frame() {
        let (mut a, mut b) = std::os::unix::net::UnixStream::pair().unwrap();
        let handle = std::thread::spawn(move || {
            let mut hdr = [0u8; 4];
            a.read_exact(&mut hdr).unwrap();
            let n = u32::from_le_bytes(hdr) as usize;
            let mut buf = vec![0u8; n];
            a.read_exact(&mut buf).unwrap();
            a.write_all(&((MAX_FRAME as u32 + 1).to_le_bytes()))
                .unwrap();
        });
        let err = roundtrip(&mut b, &json!({"op": "Ping"})).unwrap_err();
        handle.join().unwrap();
        assert!(err.to_string().contains("too large"));
    }

    #[cfg(unix)]
    #[test]
    fn missing_socket_fails_fast() {
        unsafe { std::env::set_var("SYNAPSE_SOCK", "/tmp/synapse-test-nonexistent.sock") };
        let resp = call(&json!({"op": "Ping"}));
        unsafe { std::env::remove_var("SYNAPSE_SOCK") };
        assert!(resp.is_err());
    }
}
