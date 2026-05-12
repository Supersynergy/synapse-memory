//! synapse-mcp: MCP (stdio JSON-RPC 2.0) bridge to synapsed.
//! Translates MCP tool calls -> msgpack-rpc over unix socket.
//! Market tools (smx_*) are handled locally without synapsed.

use anyhow::{Context, Result};
use clap::Parser;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::PathBuf;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;

use synapse_market::ffi::smx_query_range;
use synapse_market::Market;

#[derive(Parser)]
#[command(name = "synapse-mcp", about = "MCP server (stdio) for synapsed")]
struct Cli {
    #[arg(short = 's', long, default_value = "/tmp/synapse.sock")]
    sock: PathBuf,
    /// Path to synapse-market DB for smx_* tools (default: $SMX_DB or /tmp/synapse_market.db)
    #[arg(long, env = "SMX_DB", default_value = "/tmp/synapse_market.db")]
    market_db: PathBuf,
}

#[derive(Debug, Deserialize)]
struct JsonRpc {
    #[serde(default)]
    jsonrpc: String,
    id: Option<Value>,
    method: String,
    #[serde(default)]
    params: Value,
}

#[derive(Debug, Serialize)]
struct JsonRpcResp {
    jsonrpc: &'static str,
    id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<Value>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let stdin = tokio::io::stdin();
    let mut reader = BufReader::new(stdin).lines();
    let mut stdout = tokio::io::stdout();
    while let Some(line) = reader.next_line().await? {
        if line.is_empty() {
            continue;
        }
        let req: JsonRpc = match serde_json::from_str(&line) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("parse: {e}");
                continue;
            }
        };
        let id = req.id.clone().unwrap_or(Value::Null);
        let resp = handle(&cli.sock, &cli.market_db, &req).await;
        let out = match resp {
            Ok(v) => JsonRpcResp {
                jsonrpc: "2.0",
                id,
                result: Some(v),
                error: None,
            },
            Err(e) => JsonRpcResp {
                jsonrpc: "2.0",
                id,
                result: None,
                error: Some(json!({"code": -32000, "message": e.to_string()})),
            },
        };
        let s = serde_json::to_string(&out)?;
        stdout.write_all(s.as_bytes()).await?;
        stdout.write_all(b"\n").await?;
        stdout.flush().await?;
    }
    Ok(())
}

async fn handle(sock: &PathBuf, market_db: &PathBuf, req: &JsonRpc) -> Result<Value> {
    match req.method.as_str() {
        "initialize" => Ok(json!({
            "protocolVersion": "2024-11-05",
            "capabilities": {"tools": {}},
            "serverInfo": {"name": "synapse", "version": env!("CARGO_PKG_VERSION")}
        })),
        "tools/list" => Ok(json!({"tools": [
            {"name": "put", "description": "Append a memory.", "inputSchema": {"type": "object", "properties": {
                "text": {"type": "string"}, "title": {"type": "string"}, "uri": {"type": "string"}, "embed": {"type": "boolean"}
            }, "required": ["text"]}},
            {"name": "search", "description": "Search memories (lex/vec/hybrid).", "inputSchema": {"type": "object", "properties": {
                "q": {"type": "string"}, "mode": {"type": "string", "enum": ["Lex", "Vec", "Hybrid"]},
                "limit": {"type": "integer"}, "embed_query": {"type": "boolean"}
            }, "required": ["q"]}},
            {"name": "merge", "description": "Merge CRDT state into a doc.", "inputSchema": {"type": "object", "properties": {
                "id": {"type": "integer"}, "state": {"type": "array", "items": {"type": "integer"}}
            }, "required": ["id", "state"]}},
            {"name": "timeline", "description": "Return docs ordered by timestamp descending.", "inputSchema": {"type": "object", "properties": {
                "limit": {"type": "integer"}, "offset": {"type": "integer"}
            }}},
            {"name": "verify", "description": "Verify Ed25519 signature on a doc.", "inputSchema": {"type": "object", "properties": {
                "id": {"type": "integer"}, "vk": {"type": "array", "items": {"type": "integer"}}
            }, "required": ["id", "vk"]}},
            {"name": "synapse_merge", "description": "Merge a peer brainpack snapshot into the brain (CRDT, offline-safe). Requires synapsed to have filesystem access.", "inputSchema": {"type": "object", "properties": {
                "snapshot_path": {"type": "string", "description": "Absolute path to peer .brainpack file"},
                "out_path": {"type": "string", "description": "Output merged .brainpack path", "default": "/tmp/synapse-merged.brainpack"},
                "level": {"type": "integer", "default": 3}
            }, "required": ["snapshot_path"]}},
            {"name": "synapse_verify", "description": "Verify Ed25519 signature on a doc by id. Returns ok or error.", "inputSchema": {"type": "object", "properties": {
                "doc_id": {"type": "integer"}, "vk": {"type": "array", "items": {"type": "integer"}}
            }, "required": ["doc_id", "vk"]}},
            {"name": "smx_candles", "description": "Return OHLCV candles for a ticker in a time range.", "inputSchema": {"type": "object", "properties": {
                "ticker": {"type": "string"},
                "start": {"type": "integer", "description": "Unix timestamp seconds"},
                "end": {"type": "integer", "description": "Unix timestamp seconds"},
                "limit": {"type": "integer", "default": 500}
            }, "required": ["ticker", "start", "end"]}},
            {"name": "smx_signal_similar", "description": "Find N most similar past market regimes for ticker at date_ts.", "inputSchema": {"type": "object", "properties": {
                "ticker": {"type": "string"},
                "date_ts": {"type": "integer", "description": "Unix timestamp seconds of reference day"},
                "n": {"type": "integer", "default": 10}
            }, "required": ["ticker", "date_ts"]}},
            {"name": "smx_pattern_stats", "description": "Aggregate stats for a named pattern across all stored signals.", "inputSchema": {"type": "object", "properties": {
                "pattern": {"type": "string", "description": "Pattern name or SQL LIKE expression"}
            }, "required": ["pattern"]}},
            {"name": "smx_correlation", "description": "Return pairwise close-price correlation matrix for tickers over last N days.", "inputSchema": {"type": "object", "properties": {
                "tickers": {"type": "array", "items": {"type": "string"}},
                "days": {"type": "integer", "default": 30}
            }, "required": ["tickers"]}}
        ]})),
        "tools/call" => {
            let name = req
                .params
                .get("name")
                .and_then(|v| v.as_str())
                .context("missing tool name")?;
            let args = req.params.get("arguments").cloned().unwrap_or(json!({}));
            let result = if name.starts_with("smx_") {
                market_tool_call(market_db, name, &args)?
            } else {
                tool_call(sock, name, args).await?
            };
            Ok(json!({"content": [{"type": "text", "text": serde_json::to_string(&result)?}]}))
        }
        _ => Ok(json!({})),
    }
}

async fn tool_call(sock: &PathBuf, name: &str, args: Value) -> Result<Value> {
    let req = match name {
        "put" => json!({"op": "Put", "args": {
            "title": args.get("title"), "uri": args.get("uri"),
            "text": args.get("text").and_then(|v| v.as_str()).unwrap_or_default(),
            "meta": null,
            "embed": args.get("embed").and_then(|v| v.as_bool()).unwrap_or(false),
        }}),
        "search" => json!({"op": "Search", "args": {
            "mode": args.get("mode").and_then(|v| v.as_str()).unwrap_or("Lex"),
            "q": args.get("q").and_then(|v| v.as_str()).unwrap_or_default(),
            "limit": args.get("limit").and_then(|v| v.as_u64()).unwrap_or(10),
            "embed_query": args.get("embed_query").and_then(|v| v.as_bool()).unwrap_or(false),
        }}),
        "merge" => {
            let id = args
                .get("id")
                .and_then(|v| v.as_i64())
                .context("merge requires id")?;
            let state_bytes = json_array_to_bytes(args.get("state"))?;
            json!({"op": "Merge", "args": {"id": id, "state": state_bytes}})
        }
        "timeline" => json!({"op": "Timeline", "args": {
            "limit": args.get("limit").and_then(|v| v.as_u64()).unwrap_or(20),
            "offset": args.get("offset").and_then(|v| v.as_u64()).unwrap_or(0),
        }}),
        "verify" => {
            let id = args
                .get("id")
                .and_then(|v| v.as_i64())
                .context("verify requires id")?;
            let vk_bytes = json_array_to_bytes(args.get("vk"))?;
            json!({"op": "Verify", "args": {"id": id, "vk": vk_bytes}})
        }
        "synapse_merge" => {
            let snapshot_path = args
                .get("snapshot_path")
                .and_then(|v| v.as_str())
                .context("synapse_merge requires snapshot_path")?;
            let out_path = args
                .get("out_path")
                .and_then(|v| v.as_str())
                .unwrap_or("/tmp/synapse-merged.brainpack");
            let level = args.get("level").and_then(|v| v.as_i64()).unwrap_or(3) as i32;
            json!({"op": "SnapMerge", "args": {"snapshot_path": snapshot_path, "out_path": out_path, "level": level}})
        }
        "synapse_verify" => {
            let id = args
                .get("doc_id")
                .and_then(|v| v.as_i64())
                .context("synapse_verify requires doc_id")?;
            let vk_bytes = json_array_to_bytes(args.get("vk"))?;
            json!({"op": "Verify", "args": {"id": id, "vk": vk_bytes}})
        }
        _ => anyhow::bail!("unknown tool: {name}"),
    };
    let mut stream = UnixStream::connect(sock)
        .await
        .context("connect synapsed")?;
    let body = rmp_serde::to_vec_named(&req)?;
    use tokio::io::AsyncReadExt;
    stream.write_all(&(body.len() as u32).to_le_bytes()).await?;
    stream.write_all(&body).await?;
    stream.flush().await?;
    let mut hdr = [0u8; 4];
    stream.read_exact(&mut hdr).await?;
    let n = u32::from_le_bytes(hdr) as usize;
    let mut buf = vec![0u8; n];
    stream.read_exact(&mut buf).await?;
    let v: Value = rmp_serde::from_slice(&buf)?;
    Ok(v)
}

/// Handle smx_* market tools locally (no synapsed socket needed).
fn market_tool_call(market_db: &PathBuf, name: &str, args: &Value) -> Result<Value> {
    let m = Market::open(market_db).context("open market db")?;
    match name {
        "smx_candles" => {
            let ticker = args
                .get("ticker")
                .and_then(|v| v.as_str())
                .context("ticker")?;
            let start = args
                .get("start")
                .and_then(|v| v.as_i64())
                .context("start")?;
            let end = args.get("end").and_then(|v| v.as_i64()).context("end")?;
            let limit = args
                .get("limit")
                .and_then(|v| v.as_u64().map(|n| n as usize))
                .unwrap_or(500);
            let rows = smx_query_range(&m, ticker, start, end).unwrap_or_default();
            let candles: Vec<Value> = rows
                .into_iter()
                .take(limit)
                .map(|(ts, o, h, l, c, v)| {
                    json!({"ts": ts, "open": o, "high": h, "low": l, "close": c, "volume": v})
                })
                .collect();
            Ok(json!({"ticker": ticker, "candles": candles}))
        }
        "smx_signal_similar" => {
            let ticker = args
                .get("ticker")
                .and_then(|v| v.as_str())
                .context("ticker")?;
            let date_ts = args
                .get("date_ts")
                .and_then(|v| v.as_i64())
                .context("date_ts")?;
            let n = args
                .get("n")
                .and_then(|v| v.as_u64().map(|n| n as usize))
                .unwrap_or(10);
            let similar = m.regime_search(ticker, date_ts, n)?;
            let out: Vec<Value> = similar
                .into_iter()
                .map(|(ts, score)| json!({"ts": ts, "similarity": score}))
                .collect();
            Ok(json!({"ticker": ticker, "similar": out}))
        }
        "smx_pattern_stats" => {
            let pattern = args
                .get("pattern")
                .and_then(|v| v.as_str())
                .context("pattern")?;
            // Query signal_patterns table if it exists, else return stub
            let count: i64 = m
                .conn
                .query_row(
                    "SELECT COUNT(*) FROM signal_patterns WHERE name LIKE ?1",
                    rusqlite::params![pattern],
                    |r| r.get(0),
                )
                .unwrap_or(0);
            Ok(json!({
                "pattern": pattern,
                "n": count,
                "note": "signal_patterns table populated via ingest_signal() — stub if empty"
            }))
        }
        "smx_correlation" => {
            let tickers = args
                .get("tickers")
                .and_then(|v| v.as_array())
                .context("tickers")?;
            let days = args.get("days").and_then(|v| v.as_i64()).unwrap_or(30);
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs() as i64;
            let since = now - days * 86_400;
            let mut series: Vec<(String, Vec<f64>)> = Vec::new();
            for t in tickers {
                let sym = t.as_str().unwrap_or_default();
                let closes: Vec<f64> = smx_query_range(&m, sym, since, now)
                    .unwrap_or_default()
                    .into_iter()
                    .map(|r| r.4)
                    .collect();
                series.push((sym.to_string(), closes));
            }
            let matrix: Vec<Value> = series
                .iter()
                .map(|(a, va)| {
                    let row: Vec<Value> = series
                        .iter()
                        .map(|(b, vb)| {
                            let corr = pearson(va, vb);
                            json!({"ticker_b": b, "corr": corr})
                        })
                        .collect();
                    json!({"ticker_a": a, "row": row})
                })
                .collect();
            Ok(json!({"days": days, "matrix": matrix}))
        }
        _ => anyhow::bail!("unknown smx tool: {name}"),
    }
}

fn pearson(a: &[f64], b: &[f64]) -> f64 {
    let n = a.len().min(b.len());
    if n < 2 {
        return 0.0;
    }
    let (a, b) = (&a[..n], &b[..n]);
    let mean_a = a.iter().sum::<f64>() / n as f64;
    let mean_b = b.iter().sum::<f64>() / n as f64;
    let num: f64 = a
        .iter()
        .zip(b)
        .map(|(x, y)| (x - mean_a) * (y - mean_b))
        .sum();
    let da: f64 = a.iter().map(|x| (x - mean_a).powi(2)).sum::<f64>().sqrt();
    let db: f64 = b.iter().map(|y| (y - mean_b).powi(2)).sum::<f64>().sqrt();
    if da * db == 0.0 {
        0.0
    } else {
        num / (da * db)
    }
}

fn json_array_to_bytes(v: Option<&Value>) -> Result<Vec<u8>> {
    let arr = v
        .and_then(|v| v.as_array())
        .context("expected byte array")?;
    arr.iter()
        .map(|b| b.as_u64().map(|n| n as u8).context("byte value"))
        .collect()
}
