//! synapse-mcp: MCP (stdio JSON-RPC 2.0) bridge to synapsed.
//! Translates MCP tool calls -> msgpack-rpc over unix socket.
//! Market tools (smx_*) are handled locally without synapsed.

use anyhow::{Context, Result};
use clap::Parser;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;

use synapse_market::Market;
use synapse_market::ffi::smx_query_range;
use synapse_pack::{Candidate, Kind, PackOptions, pack, render};

type AgentScope = (String, Option<String>, String);
type MarketSeries = Vec<(String, Vec<f64>)>;

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
    #[serde(rename = "jsonrpc")]
    _jsonrpc: String,
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
            "serverInfo": {"name": "synapse", "version": env!("CARGO_PKG_VERSION")},
            "instructions": CTXOS_INSTRUCTIONS
        })),
        "tools/list" => Ok(json!({"tools": [
            // ── Coding-agent-friendly aliases ────────────────────────────────
            {"name": "memory_save", "description": "Save a memory with optional tags. Returns doc id.", "inputSchema": {"type": "object", "properties": {
                "text": {"type": "string"}, "title": {"type": "string"},
                "tags": {"type": "array", "items": {"type": "string"}}
            }, "required": ["text"]}},
            {"name": "memory_search", "description": "Hybrid semantic + keyword search over memories. Returns top-k results.", "inputSchema": {"type": "object", "properties": {
                "query": {"type": "string"}, "k": {"type": "integer", "default": 10},
                "mode": {"type": "string", "enum": ["Lex", "Vec", "Hybrid"], "default": "Hybrid"},
                "embed_query": {"type": "boolean", "default": true}
            }, "required": ["query"]}},
            {"name": "memory_recent", "description": "Return the n most recently saved memories.", "inputSchema": {"type": "object", "properties": {
                "n": {"type": "integer", "default": 20}
            }}},
            {"name": "memory_delete", "description": "Delete a memory by id.", "inputSchema": {"type": "object", "properties": {
                "id": {"type": "integer"}
            }, "required": ["id"]}},
            {"name": "agent_observe", "description": "Store a typed scoped agent observation with freshness metadata.", "inputSchema": {"type": "object", "properties": {
                "agent_id": {"type": "string"}, "project": {"type": "string"},
                "text": {"type": "string"}, "title": {"type": "string"},
                "kind": {"type": "string", "default": "observation"},
                "tags": {"type": "array", "items": {"type": "string"}},
                "source_uri": {"type": "string"}, "confidence": {"type": "number"},
                "valid_from": {}, "valid_until": {}, "embed": {"type": "boolean", "default": true}
            }, "required": ["agent_id", "text"]}},
            {"name": "agent_search_index", "description": "Return compact scoped hits for first-pass agent recall.", "inputSchema": {"type": "object", "properties": {
                "agent_id": {"type": "string"}, "project": {"type": "string"},
                "query": {"type": "string"}, "limit": {"type": "integer", "default": 8},
                "snippet_chars": {"type": "integer", "default": 240}
            }, "required": ["agent_id", "query"]}},
            {"name": "agent_get_observations", "description": "Hydrate full scoped agent observations by id.", "inputSchema": {"type": "object", "properties": {
                "agent_id": {"type": "string"}, "project": {"type": "string"},
                "ids": {"type": "array", "items": {"type": "integer"}},
                "max_chars": {"type": "integer"}
            }, "required": ["agent_id", "ids"]}},
            {"name": "agent_context", "description": "Build a token-budgeted XML context pack for an agent query.", "inputSchema": {"type": "object", "properties": {
                "agent_id": {"type": "string"}, "project": {"type": "string"},
                "query": {"type": "string"}, "token_budget": {"type": "integer", "default": 800},
                "index_k": {"type": "integer", "default": 8}, "full_k": {"type": "integer", "default": 3}
            }, "required": ["agent_id", "query"]}},
            {"name": "agent_feedback", "description": "Log accepted/rejected recall outcomes for learned reranking.", "inputSchema": {"type": "object", "properties": {
                "agent_id": {"type": "string"}, "project": {"type": "string"},
                "query": {"type": "string"}, "hit_ids": {"type": "array", "items": {"type": "integer"}},
                "outcome": {"type": "string"}, "accepted": {"type": "boolean", "default": true}
            }, "required": ["agent_id", "query", "outcome"]}},
            // ── Context-OS: works for ANY agent, no scope required ────────────
            {"name": "context_pack", "description": "Retrieve + pack the minimal VERBATIM context for a task within a token budget. Returns a STATE card (best-first, lost-in-the-middle-safe), never narrative. Call this FIRST for any task needing prior knowledge.", "inputSchema": {"type": "object", "properties": {
                "query": {"type": "string", "description": "Task or question to gather context for"},
                "budget_tokens": {"type": "integer", "default": 4000},
                "k": {"type": "integer", "default": 24, "description": "Candidates to retrieve before packing"},
                "kinds": {"type": "array", "items": {"type": "string"}, "description": "Optional filter: known-fact, decision, file, chat"}
            }, "required": ["query"]}},
            {"name": "context_feedback", "description": "After the turn, report which doc ids you actually used and whether your verify-gate passed. Closes the self-learning loop so retrieval improves.", "inputSchema": {"type": "object", "properties": {
                "pack_id": {"type": "string"}, "used_ids": {"type": "array", "items": {"type": "integer"}},
                "gate": {"type": "string", "enum": ["pass", "fail", "unknown"], "default": "unknown"}
            }, "required": ["used_ids"]}},
            {"name": "context_state", "description": "Current-truth card for a topic: latest verified facts + decisions, newest-first, with supersession marked. Use to know what is currently true.", "inputSchema": {"type": "object", "properties": {
                "topic": {"type": "string"}, "k": {"type": "integer", "default": 12}
            }, "required": ["topic"]}},
            {"name": "context_remember", "description": "Persist a durable fact or decision (embedded, searchable). Optionally supersede an older doc id and tag a topic.", "inputSchema": {"type": "object", "properties": {
                "text": {"type": "string"}, "title": {"type": "string"},
                "kind": {"type": "string", "enum": ["known-fact", "decision"], "default": "known-fact"},
                "topic": {"type": "string"}, "supersedes": {"type": "integer"}
            }, "required": ["text"]}},
            // ── Low-level tools ───────────────────────────────────────────────
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
    match name {
        "agent_observe" => return agent_observe(sock, &args).await,
        "agent_search_index" => return agent_search_index(sock, &args).await,
        "agent_get_observations" => return agent_get_observations(sock, &args).await,
        "agent_context" => return agent_context(sock, &args).await,
        "agent_feedback" => return agent_feedback(sock, &args).await,
        "context_pack" => return context_pack(sock, &args).await,
        "context_feedback" => return context_feedback(sock, &args).await,
        "context_state" => return context_state(sock, &args).await,
        "context_remember" => return context_remember(sock, &args).await,
        _ => {}
    }

    let req = match name {
        // ── Coding-agent aliases ──────────────────────────────────────────────
        "memory_save" => {
            let tags = args
                .get("tags")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str())
                        .collect::<Vec<_>>()
                        .join(",")
                })
                .unwrap_or_default();
            let title = args.get("title").and_then(|v| v.as_str());
            let uri_str = if tags.is_empty() {
                String::new()
            } else {
                format!("tags:{tags}")
            };
            json!({"op": "Put", "args": {
                "title": title,
                "uri": if uri_str.is_empty() { Value::Null } else { Value::String(uri_str) },
                "text": args.get("text").and_then(|v| v.as_str()).unwrap_or_default(),
                "meta": null,
                "embed": false,
            }})
        }
        "memory_search" => json!({"op": "Search", "args": {
            "mode": args.get("mode").and_then(|v| v.as_str()).unwrap_or("Hybrid"),
            "q": args.get("query").and_then(|v| v.as_str()).unwrap_or_default(),
            "limit": args.get("k").and_then(|v| v.as_u64()).unwrap_or(10),
            "embed_query": args.get("embed_query").and_then(|v| v.as_bool()).unwrap_or(true),
        }}),
        "memory_recent" => json!({"op": "Timeline", "args": {
            "limit": args.get("n").and_then(|v| v.as_u64()).unwrap_or(20),
            "offset": 0,
        }}),
        "memory_delete" => {
            let id = args
                .get("id")
                .and_then(|v| v.as_i64())
                .context("memory_delete requires id")?;
            json!({"op": "Delete", "args": {"id": id}})
        }
        // ── Low-level pass-throughs ───────────────────────────────────────────
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
    daemon_call(sock, req).await
}

async fn daemon_call(sock: &PathBuf, req: Value) -> Result<Value> {
    let mut stream = UnixStream::connect(sock)
        .await
        .context("connect synapsed")?;
    if let Ok(token) = std::env::var("SYNAPSE_API_KEY")
        && !token.is_empty()
    {
        let auth =
            daemon_roundtrip(&mut stream, json!({"op": "Auth", "args": {"token": token}})).await?;
        if let Some(err) = auth.get("Err").and_then(|v| v.as_str()) {
            anyhow::bail!("{err}");
        }
    }
    daemon_roundtrip(&mut stream, req).await
}

async fn daemon_roundtrip(stream: &mut UnixStream, req: Value) -> Result<Value> {
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

const AGENT_SCHEMA: &str = "synapse.agentdb.v1";

fn agent_scope(args: &Value) -> Result<AgentScope> {
    let agent_id = args
        .get("agent_id")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .context("agent_id")?
        .to_string();
    let project = args
        .get("project")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let scope = match &project {
        Some(project) => format!(
            "agent/{}/{}",
            scope_component(project),
            scope_component(&agent_id)
        ),
        None => format!("agent/{}", scope_component(&agent_id)),
    };
    Ok((agent_id, project, scope))
}

fn scope_component(value: &str) -> String {
    value.replace('%', "%25").replace('/', "%2F")
}

fn estimate_tokens(text: &str) -> usize {
    if text.is_empty() {
        0
    } else {
        text.len().div_ceil(4).max(1)
    }
}

fn compact_text(text: &str, max_chars: usize) -> String {
    if max_chars == 0 {
        return String::new();
    }
    let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.chars().count() <= max_chars {
        return collapsed;
    }
    let trimmed = collapsed
        .chars()
        .take(max_chars)
        .collect::<String>()
        .trim_end()
        .to_string();
    format!("{trimmed}...")
}

fn truncate_chars(text: &str, max_chars: usize) -> Option<String> {
    if text.chars().count() <= max_chars {
        return None;
    }
    let truncated = text.chars().take(max_chars).collect::<String>();
    Some(format!("{}\n[truncated]", truncated.trim_end()))
}

fn query_terms(query: &str) -> Vec<String> {
    let mut terms: Vec<String> = query
        .to_ascii_lowercase()
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|term| term.len() > 2 && !term.chars().all(|c| c.is_ascii_digit()))
        .map(str::to_string)
        .collect();
    let mut expanded = terms.clone();
    for term in terms.drain(..) {
        for extra in agent_recall_expansions(&term) {
            if !expanded.iter().any(|existing| existing == extra) {
                expanded.push(extra.to_string());
            }
        }
    }
    expanded
}

fn agent_recall_expansions(term: &str) -> &'static [&'static str] {
    match term {
        "context" | "token" | "tokens" | "save" => &[
            "compact",
            "hydrate",
            "index",
            "observations",
            "progressive",
            "disclosure",
        ],
        "slippage" | "version" | "versions" => &[
            "freshness",
            "source",
            "source_uri",
            "package",
            "versions",
            "local",
        ],
        "feedback" | "rerank" => &[
            "accepted",
            "rejected",
            "edits",
            "tests",
            "feedback",
            "reranking",
        ],
        "fallback" | "embeddings" => &["lexical", "scoped", "embedding", "fallback", "degrade"],
        "deployment" | "local" => &["single", "file", "unix", "socket", "deployment"],
        "graph" | "temporal" => &["graph", "temporal", "enrichment", "behind", "hot", "path"],
        _ => &[],
    }
}

fn xml_escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn parse_meta(value: &Value) -> Value {
    match value {
        Value::String(s) if !s.is_empty() => serde_json::from_str(s).unwrap_or(Value::Null),
        Value::Object(_) => value.clone(),
        _ => Value::Null,
    }
}

fn freshness(meta: &Value) -> &'static str {
    let valid_until = meta.get("valid_until").and_then(|v| v.as_f64());
    if let Some(valid_until) = valid_until {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs_f64();
        if valid_until < now {
            return "stale";
        }
        return "current";
    }
    if meta.get("source_uri").is_some() {
        "current"
    } else {
        "unknown"
    }
}

async fn sql_rows(sock: &PathBuf, query: String, params: Vec<Value>) -> Result<Vec<Value>> {
    let resp = daemon_call(
        sock,
        json!({"op": "Sql", "args": {"query": query, "params": params}}),
    )
    .await?;
    let rows_payload = resp.get("Rows").context("SQL response missing Rows")?;
    let cols = rows_payload
        .get("cols")
        .and_then(|v| v.as_array())
        .context("SQL response missing cols")?;
    let rows = rows_payload
        .get("rows")
        .and_then(|v| v.as_array())
        .context("SQL response missing rows")?;
    let col_names: Vec<String> = cols
        .iter()
        .filter_map(|v| v.as_str().map(str::to_string))
        .collect();
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let arr = row.as_array().context("SQL row must be array")?;
        let mut map = serde_json::Map::new();
        for (idx, col) in col_names.iter().enumerate() {
            let mut value = arr.get(idx).cloned().unwrap_or(Value::Null);
            if col == "meta" {
                value = parse_meta(&value);
            }
            map.insert(col.clone(), value);
        }
        out.push(Value::Object(map));
    }
    Ok(out)
}

fn doc_id(doc: &Value) -> i64 {
    doc.get("id").and_then(|v| v.as_i64()).unwrap_or_default()
}

fn doc_text(doc: &Value) -> &str {
    doc.get("text").and_then(|v| v.as_str()).unwrap_or_default()
}

fn rank_docs(query: &str, mut docs: Vec<Value>) -> Vec<Value> {
    let q = query.to_ascii_lowercase();
    let terms = query_terms(query);
    docs.sort_by(|a, b| {
        let score = |doc: &Value| {
            let text = doc_text(doc).to_ascii_lowercase();
            let exact = if text.contains(&q) { 1_i64 } else { 0 };
            let overlap = terms
                .iter()
                .filter(|term| text.contains(term.as_str()))
                .count() as i64;
            exact * 1_000_000 + overlap * 10_000 + doc_id(doc)
        };
        score(b).cmp(&score(a))
    });
    docs
}

fn compact_hit(rank: usize, doc: &Value, snippet_chars: usize) -> Value {
    let meta = doc.get("meta").cloned().unwrap_or(Value::Null);
    let text = doc_text(doc);
    json!({
        "rank": rank,
        "id": doc.get("id").cloned().unwrap_or(Value::Null),
        "score": 0.0,
        "title": doc.get("title").cloned().unwrap_or(Value::Null),
        "uri": doc.get("uri").cloned().unwrap_or(Value::Null),
        "kind": meta.get("kind").or_else(|| meta.get("type")).cloned().unwrap_or(json!("memory")),
        "tags": meta.get("tags").cloned().unwrap_or(json!([])),
        "freshness": freshness(&meta),
        "confidence": meta.get("confidence").cloned().unwrap_or(Value::Null),
        "token_estimate": estimate_tokens(text),
        "snippet": compact_text(text, snippet_chars),
        "meta": meta,
    })
}

async fn agent_observe(sock: &PathBuf, args: &Value) -> Result<Value> {
    let (agent_id, project, scope) = agent_scope(args)?;
    let text = args.get("text").and_then(|v| v.as_str()).context("text")?;
    let kind = args
        .get("kind")
        .and_then(|v| v.as_str())
        .unwrap_or("observation");
    let mut meta = json!({
        "schema": AGENT_SCHEMA,
        "scope": scope,
        "agent_id": agent_id,
        "kind": kind,
    });
    if let Some(project) = project {
        meta["project"] = json!(project);
    }
    for key in [
        "tags",
        "source_uri",
        "confidence",
        "valid_from",
        "valid_until",
    ] {
        if let Some(value) = args.get(key) {
            meta[key] = value.clone();
        }
    }
    daemon_call(
        sock,
        json!({"op": "Put", "args": {
            "title": args.get("title"),
            "uri": args.get("source_uri"),
            "text": text,
            "meta": meta,
            "embed": args.get("embed").and_then(|v| v.as_bool()).unwrap_or(true),
        }}),
    )
    .await
}

async fn agent_search_index(sock: &PathBuf, args: &Value) -> Result<Value> {
    let (_, _, scope) = agent_scope(args)?;
    let query = args
        .get("query")
        .and_then(|v| v.as_str())
        .context("query")?;
    let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(8) as usize;
    let snippet_chars = args
        .get("snippet_chars")
        .and_then(|v| v.as_u64())
        .unwrap_or(240) as usize;
    let fetch_k = (limit * 12).max(80);
    let terms = query_terms(query);
    let mut query_rows = Vec::new();
    if !terms.is_empty() {
        let mut filters = Vec::new();
        let mut params = vec![json!(scope.clone())];
        for term in terms.iter().take(16) {
            filters.push("(lower(coalesce(title,'')) LIKE ? OR lower(coalesce(text,'')) LIKE ?)");
            let needle = format!("%{}%", term.to_ascii_lowercase());
            params.push(json!(needle));
            params.push(json!(needle));
        }
        params.push(json!(fetch_k));
        query_rows = sql_rows(
            sock,
            format!(
                "SELECT id, uri, title, substr(text,1,4000) AS text, meta, ts FROM docs \
                 WHERE meta IS NOT NULL AND json_valid(meta) AND json_extract(meta, '$.scope') = ? \
                 AND ({}) ORDER BY id DESC LIMIT ?",
                filters.join(" OR ")
            ),
            params,
        )
        .await?;
    }
    let recent_rows = sql_rows(
        sock,
        "SELECT id, uri, title, substr(text,1,4000) AS text, meta, ts FROM docs \
         WHERE meta IS NOT NULL AND json_valid(meta) AND json_extract(meta, '$.scope') = ? \
         ORDER BY id DESC LIMIT ?"
            .to_string(),
        vec![json!(scope), json!(fetch_k)],
    )
    .await?;
    let mut seen = HashSet::new();
    let mut rows = Vec::new();
    for doc in query_rows.into_iter().chain(recent_rows) {
        if seen.insert(doc_id(&doc)) {
            rows.push(doc);
        }
    }
    let ranked = rank_docs(query, rows);
    let index: Vec<Value> = ranked
        .iter()
        .take(limit)
        .enumerate()
        .map(|(idx, doc)| compact_hit(idx + 1, doc, snippet_chars))
        .collect();
    Ok(json!({"schema": AGENT_SCHEMA, "scope": scope, "index": index}))
}

async fn agent_get_observations(sock: &PathBuf, args: &Value) -> Result<Value> {
    let (_, _, scope) = agent_scope(args)?;
    let ids: Vec<i64> = args
        .get("ids")
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_i64()).collect())
        .unwrap_or_default();
    if ids.is_empty() {
        return Ok(json!({"schema": AGENT_SCHEMA, "scope": scope, "observations": []}));
    }
    let placeholders = std::iter::repeat_n("?", ids.len())
        .collect::<Vec<_>>()
        .join(",");
    let rows = sql_rows(
        sock,
        format!("SELECT id, uri, title, text, meta, ts FROM docs WHERE id IN ({placeholders})"),
        ids.iter().map(|id| json!(id)).collect(),
    )
    .await?;
    let max_chars = args
        .get("max_chars")
        .and_then(|v| v.as_u64())
        .map(|n| n as usize);
    let mut by_id = HashMap::new();
    for mut doc in rows {
        let meta = doc.get("meta").cloned().unwrap_or(Value::Null);
        if meta.get("scope").and_then(|v| v.as_str()) != Some(scope.as_str()) {
            continue;
        }
        if let Some(max_chars) = max_chars
            && let Some(text) = doc.get("text").and_then(|v| v.as_str())
            && let Some(truncated) = truncate_chars(text, max_chars)
        {
            doc["text"] = json!(truncated);
            doc["truncated"] = json!(true);
        }
        doc["kind"] = meta
            .get("kind")
            .or_else(|| meta.get("type"))
            .cloned()
            .unwrap_or(json!("memory"));
        doc["freshness"] = json!(freshness(&meta));
        doc["token_estimate"] = json!(estimate_tokens(doc_text(&doc)));
        by_id.insert(doc_id(&doc), doc);
    }
    let observations: Vec<Value> = ids.into_iter().filter_map(|id| by_id.remove(&id)).collect();
    Ok(json!({"schema": AGENT_SCHEMA, "scope": scope, "observations": observations}))
}

async fn agent_context(sock: &PathBuf, args: &Value) -> Result<Value> {
    let (_, _, scope) = agent_scope(args)?;
    let query = args
        .get("query")
        .and_then(|v| v.as_str())
        .context("query")?;
    let token_budget = args
        .get("token_budget")
        .and_then(|v| v.as_u64())
        .unwrap_or(800) as usize;
    let index_k = args.get("index_k").and_then(|v| v.as_u64()).unwrap_or(8) as usize;
    let full_k = args.get("full_k").and_then(|v| v.as_u64()).unwrap_or(3) as usize;
    let index_resp = agent_search_index(
        sock,
        &json!({
            "agent_id": args.get("agent_id").cloned().unwrap_or(Value::Null),
            "project": args.get("project").cloned().unwrap_or(Value::Null),
            "query": query,
            "limit": index_k,
            "snippet_chars": 220,
        }),
    )
    .await?;
    let raw_index = index_resp
        .get("index")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let mut used_tokens = estimate_tokens(query) + 64;
    let mut index = Vec::new();
    for hit in raw_index {
        let snippet = hit
            .get("snippet")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        let hit_tokens = estimate_tokens(snippet) + 18;
        if !index.is_empty() && used_tokens + hit_tokens > token_budget {
            break;
        }
        if used_tokens + hit_tokens > token_budget {
            continue;
        }
        used_tokens += hit_tokens;
        index.push(hit);
    }
    let ids: Vec<Value> = index
        .iter()
        .filter_map(|hit| hit.get("id").and_then(|v| v.as_i64()).map(|id| json!(id)))
        .collect();
    let obs_resp = agent_get_observations(
        sock,
        &json!({
            "agent_id": args.get("agent_id").cloned().unwrap_or(Value::Null),
            "project": args.get("project").cloned().unwrap_or(Value::Null),
            "ids": ids,
        }),
    )
    .await?;
    let all_docs = obs_resp
        .get("observations")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let mut by_id = HashMap::new();
    for doc in all_docs.iter() {
        by_id.insert(doc_id(doc), doc.clone());
    }
    let mut selected = Vec::new();
    for hit in index.iter().take(full_k) {
        let Some(id) = hit.get("id").and_then(|v| v.as_i64()) else {
            continue;
        };
        let Some(doc) = by_id.get(&id) else {
            continue;
        };
        let doc_tokens = estimate_tokens(doc_text(doc));
        if used_tokens + doc_tokens > token_budget {
            let remaining = token_budget.saturating_sub(used_tokens);
            if remaining <= 8 {
                continue;
            }
            let max_chars = remaining.saturating_sub(4) * 4;
            let Some(text) = truncate_chars(doc_text(doc), max_chars) else {
                continue;
            };
            let mut clipped = doc.clone();
            clipped["text"] = json!(text);
            clipped["truncated"] = json!(true);
            let clipped_tokens = estimate_tokens(doc_text(&clipped));
            if used_tokens + clipped_tokens > token_budget {
                continue;
            }
            selected.push(clipped);
            used_tokens += clipped_tokens;
            continue;
        }
        selected.push(doc.clone());
        used_tokens += doc_tokens;
    }
    let naive_tokens: usize = all_docs
        .iter()
        .map(|doc| estimate_tokens(doc_text(doc)))
        .sum();
    let saved = naive_tokens.saturating_sub(used_tokens);
    let savings_pct = if naive_tokens == 0 {
        0.0
    } else {
        ((saved as f64 / naive_tokens as f64) * 1000.0).round() / 10.0
    };
    let context = render_agent_context(query, &scope, token_budget, used_tokens, &index, &selected);
    Ok(json!({
        "schema": AGENT_SCHEMA,
        "scope": scope,
        "query": query,
        "token_budget": token_budget,
        "estimated_tokens": used_tokens,
        "naive_full_recall_tokens": naive_tokens,
        "token_savings_pct": savings_pct,
        "index": index,
        "observations": selected,
        "context": context,
    }))
}

fn render_agent_context(
    query: &str,
    scope: &str,
    token_budget: usize,
    estimated_tokens: usize,
    index: &[Value],
    observations: &[Value],
) -> String {
    let mut lines = vec![
        format!(
            "<synapse_agent_context schema=\"{}\" scope=\"{}\" token_budget=\"{}\" estimated_tokens=\"{}\">",
            xml_escape(AGENT_SCHEMA),
            xml_escape(scope),
            token_budget,
            estimated_tokens
        ),
        format!("  <query>{}</query>", xml_escape(query)),
        "  <search_index>".to_string(),
    ];
    for hit in index {
        lines.push(format!(
            "    <hit id=\"{}\" rank=\"{}\" kind=\"{}\" freshness=\"{}\" score=\"{}\"><title>{}</title><snippet>{}</snippet></hit>",
            hit.get("id").and_then(|v| v.as_i64()).unwrap_or_default(),
            hit.get("rank").and_then(|v| v.as_u64()).unwrap_or_default(),
            xml_escape(hit.get("kind").and_then(|v| v.as_str()).unwrap_or("memory")),
            xml_escape(hit.get("freshness").and_then(|v| v.as_str()).unwrap_or("unknown")),
            hit.get("score").and_then(|v| v.as_f64()).unwrap_or_default(),
            xml_escape(hit.get("title").and_then(|v| v.as_str()).unwrap_or_default()),
            xml_escape(hit.get("snippet").and_then(|v| v.as_str()).unwrap_or_default())
        ));
    }
    lines.push("  </search_index>".to_string());
    lines.push("  <observations>".to_string());
    for doc in observations {
        lines.push(format!(
            "    <observation id=\"{}\" kind=\"{}\" freshness=\"{}\">",
            doc_id(doc),
            xml_escape(doc.get("kind").and_then(|v| v.as_str()).unwrap_or("memory")),
            xml_escape(
                doc.get("freshness")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown")
            )
        ));
        lines.push(format!(
            "      <title>{}</title>",
            xml_escape(
                doc.get("title")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
            )
        ));
        lines.push(format!("      <text>{}</text>", xml_escape(doc_text(doc))));
        lines.push("    </observation>".to_string());
    }
    lines.push("  </observations>".to_string());
    lines.push("</synapse_agent_context>".to_string());
    lines.join("\n")
}

async fn agent_feedback(sock: &PathBuf, args: &Value) -> Result<Value> {
    let (agent_id, project, scope) = agent_scope(args)?;
    let query = args
        .get("query")
        .and_then(|v| v.as_str())
        .context("query")?;
    let outcome = args
        .get("outcome")
        .and_then(|v| v.as_str())
        .context("outcome")?;
    let hit_ids = args.get("hit_ids").cloned().unwrap_or(json!([]));
    let accepted = args
        .get("accepted")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    let mut meta = json!({
        "schema": AGENT_SCHEMA,
        "scope": scope,
        "agent_id": agent_id,
        "kind": "feedback",
    });
    if let Some(project) = project {
        meta["project"] = json!(project);
    }
    let payload = json!({
        "query": query,
        "hit_ids": hit_ids,
        "outcome": outcome,
        "accepted": accepted,
        "ts": ts,
    });
    daemon_call(
        sock,
        json!({"op": "Put", "args": {
            "title": format!("agent-feedback/{outcome}"),
            "uri": Value::Null,
            "text": payload.to_string(),
            "meta": meta,
            "embed": false,
        }}),
    )
    .await
}

// ── Context-OS (ctxos) tools ──────────────────────────────────────────────────

/// Server-wide guidance Codex/Gemini read on init. First 512 chars are self-contained.
const CTXOS_INSTRUCTIONS: &str = "Synapse Context-OS (local, no cloud). Call context_pack FIRST for any task needing prior context — it returns the minimal VERBATIM state within a token budget, best-first ordered, never narrative. After the turn, call context_feedback with the doc ids you actually used and whether your verify-gate passed, so retrieval self-improves. Use context_state to get the current truth for a topic plus what superseded what. Use context_remember to persist a durable fact or decision.";

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

/// Stable short id for a pack (query + selected ids), so feedback can reference it.
fn pack_id(query: &str, ids: &[i64]) -> String {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in query.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    for id in ids {
        for b in id.to_le_bytes() {
            h ^= b as u64;
            h = h.wrapping_mul(0x100000001b3);
        }
    }
    format!("pk_{h:016x}")
}

fn hit_kind(title: &str, meta: &Value) -> Kind {
    let s = meta
        .get("kind")
        .or_else(|| meta.get("type"))
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .unwrap_or_else(|| title.to_string());
    Kind::from_meta(&s)
}

fn kind_tag(k: Kind) -> &'static str {
    match k {
        Kind::KnownFact => "known-fact",
        Kind::Decision => "decision",
        Kind::File => "file",
        Kind::Chat => "chat",
        Kind::Other => "other",
    }
}

async fn hybrid_hits(sock: &PathBuf, query: &str, limit: usize) -> Result<Vec<Value>> {
    let resp = daemon_call(
        sock,
        json!({"op": "Search", "args": {
            "mode": "Hybrid", "q": query, "limit": limit, "embed_query": true
        }}),
    )
    .await?;
    Ok(resp
        .get("Hits")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default())
}

async fn context_pack(sock: &PathBuf, args: &Value) -> Result<Value> {
    let query = args
        .get("query")
        .or_else(|| args.get("task"))
        .and_then(|v| v.as_str())
        .context("query")?;
    let budget = args
        .get("budget_tokens")
        .and_then(|v| v.as_u64())
        .unwrap_or(4000) as usize;
    let k = args.get("k").and_then(|v| v.as_u64()).unwrap_or(24) as usize;
    let kinds_filter: Option<Vec<String>> = args.get("kinds").and_then(|v| v.as_array()).map(|a| {
        a.iter()
            .filter_map(|x| x.as_str().map(|s| s.to_ascii_lowercase()))
            .collect()
    });

    let hits = hybrid_hits(sock, query, k).await?;
    let mut cands = Vec::new();
    for h in &hits {
        let text = h.get("text").and_then(|v| v.as_str()).unwrap_or_default();
        if text.is_empty() {
            continue;
        }
        let id = h.get("id").and_then(|v| v.as_i64()).unwrap_or_default();
        let title = h
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        let score = h.get("score").and_then(|v| v.as_f64()).unwrap_or(0.0) as f32;
        let meta = h.get("meta").cloned().unwrap_or(Value::Null);
        let kind = hit_kind(&title, &meta);
        if let Some(filter) = &kinds_filter
            && !filter.iter().any(|f| kind_tag(kind).contains(f.as_str()))
        {
            continue;
        }
        cands.push(Candidate {
            id,
            title,
            text: text.to_string(),
            score,
            kind,
        });
    }

    let opts = PackOptions {
        budget_tokens: budget,
        header_reserve: 64,
    };
    let packed = pack(cands, &opts);
    let rendered = render(&packed);
    let used_ids: Vec<i64> = packed.blocks.iter().map(|b| b.id).collect();
    let pid = pack_id(query, &used_ids);
    let blocks: Vec<Value> = packed
        .blocks
        .iter()
        .map(|b| {
            json!({
                "id": b.id,
                "kind": kind_tag(b.kind),
                "tier": format!("{:?}", b.tier),
                "tokens": b.tokens
            })
        })
        .collect();
    Ok(json!({
        "pack_id": pid,
        "context": rendered,
        "manifest": {
            "used_ids": used_ids,
            "dropped_ids": packed.dropped_ids,
            "deduped_ids": packed.deduped_ids,
            "used_tokens": packed.used_tokens,
            "budget_tokens": packed.budget_tokens,
            "naive_tokens": packed.naive_tokens,
            "savings_pct": packed.savings_pct(),
            "blocks": blocks,
        }
    }))
}

async fn context_feedback(sock: &PathBuf, args: &Value) -> Result<Value> {
    let pack_id = args.get("pack_id").and_then(|v| v.as_str()).unwrap_or("");
    let used_ids = args.get("used_ids").cloned().unwrap_or(json!([]));
    let gate = args
        .get("gate")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    let payload = json!({
        "pack_id": pack_id,
        "used_ids": used_ids,
        "gate": gate,
        "ts": now_secs(),
    });
    let meta = json!({"schema": "synapse.ctxos.v1", "kind": "ctx-feedback", "gate": gate});
    daemon_call(
        sock,
        json!({"op": "Put", "args": {
            "title": format!("ctx-feedback/{gate}"),
            "uri": Value::Null,
            "text": payload.to_string(),
            "meta": meta,
            "embed": false,
        }}),
    )
    .await?;
    Ok(json!({"ok": true, "recorded": payload}))
}

async fn context_remember(sock: &PathBuf, args: &Value) -> Result<Value> {
    let text = args.get("text").and_then(|v| v.as_str()).context("text")?;
    let title = args.get("title").and_then(|v| v.as_str());
    let kind = args
        .get("kind")
        .and_then(|v| v.as_str())
        .unwrap_or("known-fact");
    let mut meta = json!({"schema": "synapse.ctxos.v1", "kind": kind});
    if let Some(topic) = args.get("topic").and_then(|v| v.as_str()) {
        meta["topic"] = json!(topic);
    }
    if let Some(sup) = args.get("supersedes").and_then(|v| v.as_i64()) {
        meta["supersedes"] = json!(sup);
    }
    daemon_call(
        sock,
        json!({"op": "Put", "args": {
            "title": title,
            "uri": Value::Null,
            "text": text,
            "meta": meta,
            "embed": true,
        }}),
    )
    .await
}

async fn context_state(sock: &PathBuf, args: &Value) -> Result<Value> {
    let topic = args
        .get("topic")
        .and_then(|v| v.as_str())
        .context("topic")?;
    let k = args.get("k").and_then(|v| v.as_u64()).unwrap_or(12) as usize;
    let hits = hybrid_hits(sock, topic, k * 2).await?;

    // Keep verified knowledge only (facts + decisions); collect supersession edges.
    let mut superseded: HashSet<i64> = HashSet::new();
    let mut items: Vec<Value> = Vec::new();
    for h in &hits {
        let meta = h.get("meta").cloned().unwrap_or(Value::Null);
        if let Some(sup) = meta.get("supersedes").and_then(|v| v.as_i64()) {
            superseded.insert(sup);
        }
        let title = h.get("title").and_then(|v| v.as_str()).unwrap_or_default();
        let kind = hit_kind(title, &meta);
        if !matches!(kind, Kind::KnownFact | Kind::Decision) {
            continue;
        }
        let id = h.get("id").and_then(|v| v.as_i64()).unwrap_or_default();
        let text = h.get("text").and_then(|v| v.as_str()).unwrap_or_default();
        let head = text
            .lines()
            .map(str::trim)
            .find(|l| !l.is_empty())
            .unwrap_or("");
        let open = text.lines().any(|l| {
            let u = l.to_ascii_uppercase();
            u.contains("TODO") || u.contains("OPEN") || u.contains("UNVERIFIED")
        });
        items.push(json!({
            "id": id,
            "kind": kind_tag(kind),
            "title": title,
            "head": head,
            "ts": h.get("ts").cloned().unwrap_or(Value::Null),
            "open": open,
        }));
    }
    // newest-first, drop superseded, cap at k
    items.sort_by(|a, b| {
        b.get("ts")
            .and_then(|v| v.as_i64())
            .unwrap_or(0)
            .cmp(&a.get("ts").and_then(|v| v.as_i64()).unwrap_or(0))
    });
    let current: Vec<Value> = items
        .into_iter()
        .filter(|it| {
            !superseded.contains(&it.get("id").and_then(|v| v.as_i64()).unwrap_or_default())
        })
        .take(k)
        .collect();

    let mut card = format!("CURRENT STATE — {topic} [{} verified]\n", current.len());
    for it in &current {
        let flag = if it.get("open").and_then(|v| v.as_bool()).unwrap_or(false) {
            " ⚠open"
        } else {
            ""
        };
        card.push_str(&format!(
            "- [{}|{}]{} {} :: {}\n",
            it.get("id").and_then(|v| v.as_i64()).unwrap_or_default(),
            it.get("kind").and_then(|v| v.as_str()).unwrap_or("other"),
            flag,
            it.get("title").and_then(|v| v.as_str()).unwrap_or_default(),
            it.get("head").and_then(|v| v.as_str()).unwrap_or_default(),
        ));
    }
    Ok(json!({"topic": topic, "state": card, "items": current}))
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
            let mut series: MarketSeries = Vec::new();
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
    if da * db == 0.0 { 0.0 } else { num / (da * db) }
}

fn json_array_to_bytes(v: Option<&Value>) -> Result<Vec<u8>> {
    let arr = v
        .and_then(|v| v.as_array())
        .context("expected byte array")?;
    arr.iter()
        .map(|b| b.as_u64().map(|n| n as u8).context("byte value"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_scope_escapes_separator_chars() {
        let (_, _, a) = agent_scope(&json!({"agent_id": "a/b", "project": "p"})).unwrap();
        let (_, _, b) = agent_scope(&json!({"agent_id": "b", "project": "p/a"})).unwrap();

        assert_eq!(a, "agent/p/a%2Fb");
        assert_eq!(b, "agent/p%2Fa/b");
        assert_ne!(a, b);
    }

    #[test]
    fn compact_and_truncate_are_unicode_safe() {
        assert_eq!(compact_text("äöüß alpha", 3), "äöü...");
        assert_eq!(
            truncate_chars("äöüß alpha", 4).unwrap(),
            "äöüß\n[truncated]"
        );
    }
}
