//! synapse-mcp: MCP (stdio JSON-RPC 2.0) bridge to synapsed.
//! Translates MCP tool calls -> msgpack-rpc over unix socket.

use anyhow::{Context, Result};
use clap::Parser;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::PathBuf;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;

#[derive(Parser)]
#[command(name = "synapse-mcp", about = "MCP server (stdio) for synapsed")]
struct Cli {
    #[arg(short = 's', long, default_value = "/tmp/synapse.sock")]
    sock: PathBuf,
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
        let resp = handle(&cli.sock, &req).await;
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

async fn handle(sock: &PathBuf, req: &JsonRpc) -> Result<Value> {
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
            }, "required": ["id", "vk"]}}
        ]})),
        "tools/call" => {
            let name = req
                .params
                .get("name")
                .and_then(|v| v.as_str())
                .context("missing tool name")?;
            let args = req.params.get("arguments").cloned().unwrap_or(json!({}));
            let result = tool_call(sock, name, args).await?;
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

fn json_array_to_bytes(v: Option<&Value>) -> Result<Vec<u8>> {
    let arr = v
        .and_then(|v| v.as_array())
        .context("expected byte array")?;
    arr.iter()
        .map(|b| b.as_u64().map(|n| n as u8).context("byte value"))
        .collect()
}
