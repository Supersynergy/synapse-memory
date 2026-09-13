//! synapse-mcp: MCP (stdio JSON-RPC 2.0) bridge to synapsed.
//! Translates MCP tool calls -> msgpack-rpc over unix socket.
//! Market tools (smx_*) are handled locally without synapsed.

use anyhow::{Context, Result};
use clap::Parser;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::LazyLock;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;

use synapse_decay::recall_multiplier;
#[cfg(feature = "market")]
use synapse_market::Market;
#[cfg(feature = "market")]
use synapse_market::ffi::smx_query_range;
use synapse_pack::{
    Candidate, Kind, PackOptions, estimate_tokens, has_context_trigger, kind_tag, pack, pack_delta,
    render,
};

type AgentScope = (String, Option<String>, String);
#[cfg(feature = "market")]
type MarketSeries = Vec<(String, Vec<f64>)>;

/// Pack cache: keyed by (query hash, budget, prev_pack_id hash) → rendered pack.
/// LRU with capacity 64. Hits skip recall+pack entirely → -100 % on repeat queries.
const PACK_CACHE_CAP: usize = 64;

#[derive(Debug, Clone)]
struct PackCacheEntry {
    rendered: String,
    used_ids: Vec<i64>,
    used_kinds: Vec<String>,
    used_tokens: usize,
    naive_tokens: usize,
    savings_pct: f32,
    pack_id: String,
}

#[derive(Debug, Default)]
struct PackCache {
    inner: HashMap<u64, PackCacheEntry>,
    order: std::collections::VecDeque<u64>,
}

impl PackCache {
    fn key(query: &str, budget: usize, prev_pack_id: Option<&str>) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        query.hash(&mut h);
        budget.hash(&mut h);
        if let Some(p) = prev_pack_id {
            p.hash(&mut h);
        }
        h.finish()
    }

    fn get(&mut self, k: u64) -> Option<&PackCacheEntry> {
        if self.inner.contains_key(&k) {
            // move to back (most-recent)
            self.order.retain(|&x| x != k);
            self.order.push_back(k);
            self.inner.get(&k)
        } else {
            None
        }
    }

    fn put(&mut self, k: u64, v: PackCacheEntry) {
        if self.inner.len() >= PACK_CACHE_CAP
            && !self.inner.contains_key(&k)
            && let Some(old) = self.order.pop_front()
        {
            self.inner.remove(&old);
        }
        self.order.retain(|&x| x != k);
        self.order.push_back(k);
        self.inner.insert(k, v);
    }

    #[allow(dead_code)]
    fn len(&self) -> usize {
        self.inner.len()
    }
}

static PACK_CACHE: LazyLock<std::sync::Mutex<PackCache>> =
    LazyLock::new(|| std::sync::Mutex::new(PackCache::default()));

#[derive(Parser)]
#[command(name = "synapse-mcp", about = "MCP server (stdio) for synapsed")]
struct Cli {
    #[arg(short = 's', long, default_value = "/tmp/synapse.sock")]
    sock: PathBuf,
    /// Path to synapse-market DB for smx_* tools (default: $SMX_DB or /tmp/synapse_market.db)
    #[arg(long, env = "SMX_DB", default_value = "/tmp/synapse_market.db")]
    market_db: PathBuf,
    /// Brain DB whose sibling `*.learn.db` holds the self-learning reward tables.
    /// Default: $SYNAPSE_BRAIN or ~/.synapse/brain.db.
    #[arg(long, env = "SYNAPSE_BRAIN")]
    brain: Option<PathBuf>,
    /// Verify the loaded noise model matches the trainer: evaluate a parity file
    /// (`*.parity.json`) and report max |Rust − CatBoost| prob diff, then exit.
    #[arg(long)]
    noise_selftest: Option<PathBuf>,
}

/// Compare the native Rust noise-model eval against the Python/CatBoost parity vectors.
fn run_noise_selftest(path: &PathBuf) -> Result<()> {
    let model = NOISE_MODEL.as_ref().context(
        "no noise model loaded (set SYNAPSE_CTXOS_MODEL or ~/.synapse/ctxos_noise_model.json)",
    )?;
    let data: Value = serde_json::from_slice(&std::fs::read(path)?)?;
    let cases = data
        .get("cases")
        .and_then(|v| v.as_array())
        .context("parity file missing cases")?;
    let mut max_diff = 0.0f64;
    for (i, c) in cases.iter().enumerate() {
        let fv: Vec<f64> = c
            .get("features")
            .and_then(|v| v.as_array())
            .map(|a| a.iter().filter_map(|v| v.as_f64()).collect())
            .unwrap_or_default();
        let expected = c.get("prob").and_then(|v| v.as_f64()).unwrap_or(0.0);
        let got = catboost_prob(model, &fv);
        let diff = (got - expected).abs();
        max_diff = max_diff.max(diff);
        println!("case {i}: expected={expected:.6} got={got:.6} diff={diff:.2e}");
    }
    println!("max_diff={max_diff:.2e}");
    if max_diff > 1e-5 {
        anyhow::bail!("parity FAILED (max_diff {max_diff:.2e} > 1e-5)");
    }
    println!("parity OK");
    Ok(())
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
    if let Some(b) = &cli.brain {
        // Make the brain path visible to the self-learning helpers.
        unsafe { std::env::set_var("SYNAPSE_BRAIN", b) };
    }
    if let Some(parity) = &cli.noise_selftest {
        return run_noise_selftest(parity);
    }
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
        "tools/list" => {
            #[allow(unused_mut)]
            let mut list = json!({"tools": [
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
            {"name": "context_pack", "description": "Retrieve + pack the minimal VERBATIM context for a task within a token budget. Returns a STATE card (best-first, lost-in-the-middle-safe), never narrative. Call this FIRST for any task needing prior knowledge. Pass prev_pack_id from the previous turn to get a delta pack (skips already-delivered ids, -70% tokens on incremental loops). Set cache_stable_order=true to order blocks so prev-used ids lead (prompt-cache-stable prefix, -20% on Claude). Cache hits (same query+budget) return in O(1) with cache_hit=true.", "inputSchema": {"type": "object", "properties": {
                "query": {"type": "string", "description": "Task or question to gather context for"},
                "budget_tokens": {"type": "integer", "default": 4000},
                "k": {"type": "integer", "default": 32, "description": "Candidates per query angle before packing (raw + high-signal-terms merged)"},
                "kinds": {"type": "array", "items": {"type": "string"}, "description": "Optional filter: known-fact, decision, file, chat"},
                "prev_pack_id": {"type": "string", "description": "pack_id from previous turn → delta pack (skip already-delivered ids)"},
                "cache_stable_order": {"type": "boolean", "default": false, "description": "Order blocks so prev-used ids lead for prompt-cache reuse"},
                "use_cache": {"type": "boolean", "default": true, "description": "Use LRU pack cache (repeat query → O(1) hit)"}
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
            // ── Swarm / Mega-Session Spiegelung ──────────────────────────────
            {"name": "session_ingest", "description": "Ingest events from a swarm/mega-session (cmux, multi-agent, long coding session). Stores as kind=session-summary with meta.session_id, meta.agent_role, meta.turn_range. Enables session_replay + progressive summarization. Call at end of session or per-chunk.", "inputSchema": {"type": "object", "properties": {
                "session_id": {"type": "string", "description": "Unique session id (e.g. cmux workspace id)"},
                "agent_role": {"type": "string", "description": "Role of the agent (planner, worker, reviewer, etc.)"},
                "events": {"type": "array", "items": {"type": "object", "properties": {
                    "turn": {"type": "integer"}, "kind": {"type": "string"},
                    "tool": {"type": "string"}, "content": {"type": "string"}
                }}},
                "summary": {"type": "string", "description": "Optional distilled summary of this chunk"},
                "turn_start": {"type": "integer"}, "turn_end": {"type": "integer"},
                "repo": {"type": "string", "description": "Optional repo path this session worked on"}
            }, "required": ["session_id", "events"]}},
            {"name": "session_replay", "description": "Reconstruct a session from stored session-summary docs. Searches by meta.session_id, orders by ts, packs into a token-budgeted replay. Use to resume a mega-session in a fresh 200k window — the prior session's decisions + events come back verbatim, best-first.", "inputSchema": {"type": "object", "properties": {
                "session_id": {"type": "string"},
                "agent_role": {"type": "string", "description": "Optional: filter to one agent role"},
                "budget_tokens": {"type": "integer", "default": 8000}
            }, "required": ["session_id"]}},
            {"name": "context_needed", "description": "Zero-cost predicate: does this prompt warrant a context pack? Returns {trigger: bool}. Routers and prompt hooks call this FIRST — smalltalk spends zero tokens, no retrieval runs.", "inputSchema": {"type": "object", "properties": {
                "query": {"type": "string", "description": "The raw user prompt to classify"},
                "task": {"type": "string", "description": "Alias for query"}
            }, "required": []}},
            // ── Affiliate Attribution (cookieless, E.L.A.B.-compatible) ───────
            {"name": "affiliate_attribute", "description": "Cookieless server-side attribution lock for affiliate/E.L.A.B. funnels. Stores ctx_hash → partner_id binding as kind=affiliate-attribution. Replaces 3rd-party cookies, ad-blocker-resistent, DSGVO-clean (no PII). Call on first touch (QR scan / link click / invitation). Merchant SDK calls this to claim commission. always_keep: true.", "inputSchema": {"type": "object", "properties": {
                "ctx_hash": {"type": "string", "description": "Synapse context hash (from context_pack) — serves as cross-device user id"},
                "partner_id": {"type": "string", "description": "Affiliate/partner id (e.g. REF-XXXXXXXX)"},
                "merchant_id": {"type": "string", "description": "Merchant / offer id"},
                "event_type": {"type": "string", "enum": ["invitation", "opt_in", "booking", "show", "close", "referral", "claim"], "description": "E.L.A.B. funnel phase or commission-claim"},
                "attribution_type": {"type": "string", "enum": ["qr_scan", "referral", "showroom", "direct", "online"], "default": "qr_scan"},
                "commission_split": {"type": "object", "description": "Optional override: {primary: pct, secondary: pct}"},
                "source_data": {"type": "object", "description": "UTM params, campaign info, creative id, etc."},
                "expires_days": {"type": "integer", "default": 90, "description": "Attribution expiry in days"}
            }, "required": ["ctx_hash", "partner_id", "merchant_id", "event_type"]}},
            {"name": "affiliate_lookup", "description": "Resolve active attribution for a ctx_hash + merchant_id. Returns partner_id, attribution_type, commission_split, expires_at, event_chain. Used by merchant SDK at checkout to claim commission. Also returns DuckLake snapshot id if payout_chain was written.", "inputSchema": {"type": "object", "properties": {
                "ctx_hash": {"type": "string"},
                "merchant_id": {"type": "string"},
                "include_chain": {"type": "boolean", "default": false, "description": "Include full E.L.A.B. event chain (invitation → ... → close)"}
            }, "required": ["ctx_hash", "merchant_id"]}},
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
            }, "required": ["doc_id", "vk"]}}
            ]});
            // smx_* market tools only exist in the full (engine) build.
            #[cfg(feature = "market")]
            if let Some(arr) = list.get_mut("tools").and_then(|v| v.as_array_mut()) {
                arr.extend(market_tools());
            }
            // Tool-list truncation: if the client passes a `query` hint in
            // tools/list params, keep only tools whose name or description
            // contains any query term (case-insensitive). This cuts the
            // tool-list payload from ~30 tools to 2-5 → -500-2000 tok/req.
            // Tools with `always_keep: true` in description are preserved.
            if let Some(q) = req.params.get("query").and_then(|v| v.as_str())
                && !q.trim().is_empty()
            {
                let terms: Vec<String> = q
                    .split_whitespace()
                    .filter(|t| t.len() >= 3)
                    .map(|t| t.to_ascii_lowercase())
                    .collect();
                if !terms.is_empty()
                    && let Some(arr) = list.get_mut("tools").and_then(|v| v.as_array_mut())
                {
                    arr.retain(|t| {
                        let name = t
                            .get("name")
                            .and_then(|v| v.as_str())
                            .unwrap_or_default()
                            .to_ascii_lowercase();
                        let desc = t
                            .get("description")
                            .and_then(|v| v.as_str())
                            .unwrap_or_default()
                            .to_ascii_lowercase();
                        if desc.contains("always_keep:") {
                            return true;
                        }
                        terms
                            .iter()
                            .any(|term| name.contains(term) || desc.contains(term))
                    });
                }
            }
            Ok(list)
        }
        "tools/call" => {
            let name = req
                .params
                .get("name")
                .and_then(|v| v.as_str())
                .context("missing tool name")?;
            let args = req.params.get("arguments").cloned().unwrap_or(json!({}));
            let result = if name.starts_with("smx_") {
                market_dispatch(market_db, name, &args)?
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
        "affiliate_attribute" => return affiliate_attribute(sock, &args).await,
        "affiliate_lookup" => return affiliate_lookup(sock, &args).await,
        "session_ingest" => return session_ingest(sock, &args).await,
        "session_replay" => return session_replay(sock, &args).await,
        "context_needed" => return context_needed(&args),
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
const CTXOS_INSTRUCTIONS: &str = "Synapse Context-OS (local, no cloud).\
 RULE 1: Before answering any task, call context_pack(query=<user task>). Returns minimal VERBATIM state, best-first, never narrative.\
 RULE 2: Pass prev_pack_id from the previous turn to get a delta pack (-70% tokens on incremental loops).\
 RULE 3: After answering, call context_feedback(pack_id, used_ids, gate) so retrieval self-improves.\
 RULE 4: Use context_state for current truth on a topic. Use context_remember to persist durable facts.\
 For dumb models: just call context_pack(query=...) FIRST, then answer. That alone gives 200k-window the effective reach of 10M.";

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

/// Brain DB path; its sibling `*.learn.db` holds the self-learning reward tables.
fn brain_path() -> PathBuf {
    if let Ok(p) = std::env::var("SYNAPSE_BRAIN")
        && !p.is_empty()
    {
        return PathBuf::from(p);
    }
    if let Ok(home) = std::env::var("HOME") {
        return PathBuf::from(home).join(".synapse/brain.db");
    }
    PathBuf::from(".synapse/brain.db")
}

const KIND_TAGS: [&str; 7] = [
    "known-fact",
    "decision",
    "file",
    "chat",
    "session-summary",
    "codebase-map",
    "other",
];

/// Learned per-kind ranking bonus (win-rate × 0.03), read from the learn store.
/// Empty map if the store is unavailable — retrieval still works, just unlearned.
fn learn_bonus_map() -> HashMap<&'static str, f32> {
    let mut m = HashMap::new();
    let lp = brain_path().with_extension("learn.db");
    if let Ok(store) = synapse_learn::LearnStore::open(&lp) {
        for tag in KIND_TAGS {
            if let Ok(b) = store.memory_type_bonus(tag) {
                m.insert(tag, b as f32);
            }
        }
    }
    m
}

/// Record reward for the kinds the agent actually used, plus a global ctxpack bandit arm.
/// Returns the number of kind-rewards written (0 if the store is unavailable).
fn record_ctx_reward(kinds: &[&str], hit: bool) -> usize {
    let lp = brain_path().with_extension("learn.db");
    let Ok(store) = synapse_learn::LearnStore::open(&lp) else {
        return 0;
    };
    let mut n = 0;
    for k in kinds {
        if store.update_memory_type_reward(k, hit).is_ok() {
            n += 1;
        }
    }
    let _ = store.update_bandit("ctxpack", hit);
    n
}

// ── Verify-Gate Degradation ───────────────────────────────────────────────────
// Tracks per-session whether context_feedback was called after a pack. Sessions
// that skip feedback get degraded pack budgets (soft stick, not a hard block).
// This nudges dumb models toward the self-learning loop without breaking them.

static FEEDBACK_STATE: LazyLock<std::sync::Mutex<FeedbackState>> =
    LazyLock::new(|| std::sync::Mutex::new(FeedbackState::default()));

#[derive(Default)]
struct FeedbackState {
    /// pack_id → (created_secs, feedback_received: bool)
    packs: HashMap<String, (i64, bool)>,
}

impl FeedbackState {
    fn register_pack(&mut self, pack_id: &str) {
        self.packs.insert(pack_id.to_string(), (now_secs(), false));
        // GC: drop packs older than 1h with no feedback.
        let cutoff = now_secs() - 3600;
        self.packs.retain(|_, (ts, _)| *ts > cutoff);
    }
    fn mark_feedback(&mut self, pack_id: &str) {
        if let Some(e) = self.packs.get_mut(pack_id) {
            e.1 = true;
        }
    }
    /// Degradation factor: if the last 3+ packs have no feedback, shrink budget
    /// to 60% (dumb models get less context until they participate in the loop).
    fn budget_factor(&self, requested: usize) -> usize {
        let open: usize = self.packs.values().filter(|(_, fb)| !fb).count();
        if open >= 6 {
            (requested * 2 / 5).max(512)
        } else if open >= 3 {
            (requested * 3 / 5).max(512)
        } else {
            requested
        }
    }
}

/// Skill-preload hints: suggest 0-3 skills the router should lazy-load based on
/// query terms. The router (separate system) reads `manifest.preload_skills`
/// and swaps 15k-tok system-prompt skills for 0-1 actually-relevant ones.
///
/// Keep this dumb + keyword-based — the router does the heavy lifting.
fn skill_preload_hints(query: &str) -> Vec<&'static str> {
    let q = query.to_ascii_lowercase();
    let mut out: Vec<&'static str> = Vec::new();
    // Trading / investing
    if q.split_whitespace().any(|t| {
        matches!(
            t,
            "trade"
                | "trading"
                | "stock"
                | "portfolio"
                | "invest"
                | "investing"
                | "ipo"
                | "pre-ipo"
                | "kelly"
                | "asymbet"
                | "winvestment"
                | "backtest"
        )
    }) {
        out.push("asymbet");
        out.push("winvestment-profet");
    }
    // Marketing / copy
    if q.split_whitespace().any(|t| {
        matches!(
            t,
            "marketing"
                | "copy"
                | "copywriting"
                | "landing"
                | "seo"
                | "ad"
                | "ads"
                | "cro"
                | "funnel"
                | "brand"
        )
    }) {
        out.push("copywriting");
        out.push("cro");
    }
    // Code / repo work
    if q.split_whitespace().any(|t| {
        matches!(
            t,
            "code"
                | "repo"
                | "rust"
                | "typescript"
                | "refactor"
                | "bug"
                | "test"
                | "cargo"
                | "build"
                | "deploy"
        )
    }) {
        out.push("agent-token-saver");
    }
    // Research / web
    if q.split_whitespace().any(|t| {
        matches!(
            t,
            "research" | "scrape" | "web" | "url" | "article" | "news" | "source"
        )
    }) {
        out.push("superscrape");
    }
    // Writing / books
    if q.split_whitespace().any(|t| {
        matches!(
            t,
            "book" | "write" | "writing" | "author" | "publish" | "hörbuch" | "audiobook"
        )
    }) {
        out.push("universalbook");
    }
    out.truncate(3);
    out
}

/// Auto-pack trigger: queries that look like real tasks (not greetings or
/// single-word noise) get auto-packed by the server if the model doesn't call
/// context_pack itself. This is the dumb-model safety net.
#[allow(dead_code)]
fn hit_kind(title: &str, meta: &Value) -> Kind {
    let s = meta
        .get("kind")
        .or_else(|| meta.get("type"))
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .unwrap_or_else(|| title.to_string());
    Kind::from_meta(&s)
}

/// Fraction of `terms` present in `text`, scaled to a small score nudge.
fn term_overlap_boost(terms: &[String], text: &str) -> f32 {
    if terms.is_empty() {
        return 0.0;
    }
    let lt = text.to_ascii_lowercase();
    let hits = terms.iter().filter(|t| lt.contains(t.as_str())).count();
    (hits as f32 / terms.len() as f32) * 0.1
}

/// Low-signal noise that must never enter a context pack (via negativa): telepathy
/// heartbeats, harness task-notifications, status JSON, session/briefing logs, tiny stubs.
/// Dropping these is the highest-leverage recall win — they crowd out real knowledge.
fn is_noise(h: &Value) -> bool {
    let title = h.get("title").and_then(|v| v.as_str()).unwrap_or("");
    let uri = h.get("uri").and_then(|v| v.as_str()).unwrap_or("");
    let text = h.get("text").and_then(|v| v.as_str()).unwrap_or("");

    // telepathy status / reply spam
    if title.contains("[telepathy]") || text.contains("[telepathy]") {
        return true;
    }
    // harness task-notification / tool-call dumps
    if text.contains("<task-notification>") || text.contains("tool-use-id") {
        return true;
    }
    // machine status heartbeats (JSON)
    if text.contains("\"models_loaded\"")
        || text.contains("\"desktop_procs\"")
        || text.contains("\"cli_sessions\"")
    {
        return true;
    }
    // log / briefing artifacts
    if uri.ends_with(".log")
        || title.ends_with(".log")
        || title.contains("sched_briefing")
        || text.starts_with("Agent [briefing]")
    {
        return true;
    }
    // empty / stub — no real content to pack
    if text.trim().len() < 40 {
        return true;
    }
    false
}

/// Learned noise classifier (logistic), trained offline by tools/ctxos/train_noise_model.py
/// and applied natively here — no Python at runtime. Generalizes beyond the hard `is_noise`
/// patterns. Absent file → `None` → pattern-only filtering (graceful).
#[derive(Debug, Deserialize, Default)]
struct NoiseModel {
    #[serde(default)]
    kind: String,
    // logistic
    #[serde(default)]
    weights: HashMap<String, f64>,
    #[serde(default)]
    bias: f64,
    #[serde(default = "default_threshold")]
    threshold: f64,
    // catboost_oblivious
    #[serde(default)]
    feature_order: Vec<String>,
    #[serde(default = "default_scale")]
    scale: f64,
    #[serde(default)]
    trees: Vec<CatTree>,
}

#[derive(Debug, Deserialize, Default)]
struct CatTree {
    splits: Vec<CatSplit>,
    leaves: Vec<f64>,
}

#[derive(Debug, Deserialize)]
struct CatSplit {
    feature: usize,
    border: f64,
}

fn default_threshold() -> f64 {
    0.5
}

fn default_scale() -> f64 {
    1.0
}

fn noise_model_path() -> PathBuf {
    if let Ok(p) = std::env::var("SYNAPSE_CTXOS_MODEL")
        && !p.is_empty()
    {
        return PathBuf::from(p);
    }
    if let Ok(home) = std::env::var("HOME") {
        return PathBuf::from(home).join(".synapse/ctxos_noise_model.json");
    }
    PathBuf::from(".synapse/ctxos_noise_model.json")
}

static NOISE_MODEL: LazyLock<Option<NoiseModel>> = LazyLock::new(|| {
    let path = noise_model_path();
    let bytes = std::fs::read(&path).ok()?;
    serde_json::from_slice(&bytes).ok()
});

/// Doc features for the learned classifier. MUST stay byte-for-byte identical to
/// `features()` in tools/ctxos/train_noise_model.py (char-based counts, same ratios).
fn noise_features(title: &str, uri: &str, text: &str) -> HashMap<&'static str, f64> {
    let n = text.chars().count();
    let nf = n as f64;
    let digits = text.chars().filter(|c| c.is_numeric()).count() as f64;
    let upper = text.chars().filter(|c| c.is_uppercase()).count() as f64;
    let punct = text
        .chars()
        .filter(|c| !c.is_alphanumeric() && !c.is_whitespace())
        .count() as f64;
    let nlines = text.split('\n').count();
    let words: Vec<&str> = text.split_whitespace().collect();
    let nwords = words.len();
    let uniq = words
        .iter()
        .map(|w| w.to_lowercase())
        .collect::<std::collections::HashSet<_>>()
        .len();
    let angles = (text.matches('<').count() + text.matches('>').count()) as f64;

    let mut m = HashMap::new();
    m.insert("log_len", (1.0 + nf).ln());
    m.insert("frac_digit", if n > 0 { digits / nf } else { 0.0 });
    m.insert("frac_upper", if n > 0 { upper / nf } else { 0.0 });
    m.insert("frac_punct", if n > 0 { punct / nf } else { 0.0 });
    m.insert(
        "brace_json",
        if text.contains("\": ") || text.trim_start().starts_with('{') {
            1.0
        } else {
            0.0
        },
    );
    m.insert("log_nlines", (1.0 + nlines as f64).ln());
    m.insert(
        "avg_line_len",
        if nlines > 0 { nf / nlines as f64 } else { 0.0 },
    );
    m.insert(
        "title_marker",
        if title.contains(':') || title.contains('/') {
            1.0
        } else {
            0.0
        },
    );
    m.insert("uri_log", if uri.ends_with(".log") { 1.0 } else { 0.0 });
    m.insert("angle_frac", if n > 0 { angles / nf } else { 0.0 });
    m.insert(
        "uniq_ratio",
        if nwords > 0 {
            uniq as f64 / nwords as f64
        } else {
            0.0
        },
    );
    m
}

fn sigmoid(z: f64) -> f64 {
    1.0 / (1.0 + (-z).exp())
}

/// P(noise) from the learned model — CatBoost oblivious-tree ensemble or logistic fallback.
fn learned_noise_prob(m: &NoiseModel, feats: &HashMap<&'static str, f64>) -> f64 {
    if m.kind == "catboost_oblivious" && !m.trees.is_empty() {
        let fvec: Vec<f64> = m
            .feature_order
            .iter()
            .map(|k| feats.get(k.as_str()).copied().unwrap_or(0.0))
            .collect();
        return catboost_prob(m, &fvec);
    }
    let mut z = m.bias;
    for (k, w) in &m.weights {
        z += w * feats.get(k.as_str()).copied().unwrap_or(0.0);
    }
    sigmoid(z)
}

/// Evaluate the CatBoost oblivious ensemble over a feature vector aligned to `feature_order`.
/// Oblivious tree: leaf index = OR of (feature > border) << split_position. Same convention
/// as CatBoost's model JSON, verified against predict_proba parity vectors.
fn catboost_prob(m: &NoiseModel, fvec: &[f64]) -> f64 {
    let mut sum = 0.0;
    for tree in &m.trees {
        let mut idx = 0usize;
        for (pos, s) in tree.splits.iter().enumerate() {
            if fvec.get(s.feature).copied().unwrap_or(0.0) > s.border {
                idx |= 1 << pos;
            }
        }
        sum += tree.leaves.get(idx).copied().unwrap_or(0.0);
    }
    sigmoid(m.scale * sum + m.bias)
}

/// True if `h` is noise — hard patterns first, then the learned model if loaded.
fn drop_as_noise(h: &Value) -> bool {
    if is_noise(h) {
        return true;
    }
    if let Some(model) = NOISE_MODEL.as_ref() {
        let title = h.get("title").and_then(|v| v.as_str()).unwrap_or("");
        let uri = h.get("uri").and_then(|v| v.as_str()).unwrap_or("");
        let text = h.get("text").and_then(|v| v.as_str()).unwrap_or("");
        let feats = noise_features(title, uri, text);
        if learned_noise_prob(model, &feats) > model.threshold {
            return true;
        }
    }
    false
}

/// Recall booster: union the raw-query hybrid search with a high-signal-terms search,
/// drop noise (via negativa), dedup by id (keeping the higher daemon score).
/// Returns the clean candidates and the number of noise docs filtered out.
async fn recall_candidates(sock: &PathBuf, query: &str, k: usize) -> Result<(Vec<Value>, usize)> {
    let raw = hybrid_hits(sock, query, k).await?;
    let terms = query_terms(query);
    let term_query = terms.join(" ");
    let extra = if !terms.is_empty() && term_query != query {
        hybrid_hits(sock, &term_query, k).await.unwrap_or_default()
    } else {
        Vec::new()
    };

    let mut by_id: HashMap<i64, Value> = HashMap::new();
    let mut order: Vec<i64> = Vec::new();
    let mut noise = 0usize;
    for h in raw.into_iter().chain(extra) {
        if drop_as_noise(&h) {
            noise += 1;
            continue;
        }
        let id = h.get("id").and_then(|v| v.as_i64()).unwrap_or_default();
        match by_id.get(&id) {
            Some(existing) => {
                let es = existing
                    .get("score")
                    .and_then(|v| v.as_f64())
                    .unwrap_or(0.0);
                let ns = h.get("score").and_then(|v| v.as_f64()).unwrap_or(0.0);
                if ns > es {
                    by_id.insert(id, h);
                }
            }
            None => {
                order.push(id);
                by_id.insert(id, h);
            }
        }
    }
    let clean = order
        .into_iter()
        .filter_map(|id| by_id.remove(&id))
        .collect();
    Ok((clean, noise))
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
    let k = args.get("k").and_then(|v| v.as_u64()).unwrap_or(32) as usize;
    let prev_pack_id: Option<&str> = args.get("prev_pack_id").and_then(|v| v.as_str());
    let cache_stable: bool = args
        .get("cache_stable_order")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let use_cache: bool = args
        .get("use_cache")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    let kinds_filter: Option<Vec<String>> = args.get("kinds").and_then(|v| v.as_array()).map(|a| {
        a.iter()
            .filter_map(|x| x.as_str().map(|s| s.to_ascii_lowercase()))
            .collect()
    });

    // Verify-gate degradation: shrink budget if the session has 3+ unanswered packs.
    let budget = if let Ok(state) = FEEDBACK_STATE.lock() {
        state.budget_factor(budget)
    } else {
        budget
    };

    // 0. Pack cache hit → skip recall+pack entirely.
    let cache_key = PackCache::key(query, budget, prev_pack_id);
    if use_cache
        && prev_pack_id.is_none()
        && let Ok(mut cache) = PACK_CACHE.lock()
        && let Some(entry) = cache.get(cache_key)
    {
        return Ok(json!({
            "pack_id": entry.pack_id,
            "context": entry.rendered,
            "manifest": {
                "used_ids": entry.used_ids,
                "dropped_ids": Vec::<i64>::new(),
                "deduped_ids": Vec::<i64>::new(),
                "delta_skipped_ids": Vec::<i64>::new(),
                "used_tokens": entry.used_tokens,
                "budget_tokens": budget,
                "naive_tokens": entry.naive_tokens,
                "savings_pct": entry.savings_pct,
                "noise_filtered": 0,
                "cache_hit": true,
                "blocks": Vec::<Value>::new(),
            }
        }));
    }

    // Recall: union of the raw-query search and a high-signal-terms search, noise-filtered + deduped.
    let (hits, noise_filtered) = recall_candidates(sock, query, k).await?;
    let terms = query_terms(query);
    let learned = learn_bonus_map();
    let lstore = synapse_learn::LearnStore::open(brain_path().with_extension("learn.db")).ok();
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
        let mut score = h.get("score").and_then(|v| v.as_f64()).unwrap_or(0.0) as f32;
        let meta = h.get("meta").cloned().unwrap_or(Value::Null);
        let kind = hit_kind(&title, &meta);
        // learned per-kind bonus — feedback on this kind lifts it in future packs
        score += learned.get(kind_tag(kind)).copied().unwrap_or(0.0);
        // recall: a candidate matching more query terms ranks higher (survives the budget)
        score += term_overlap_boost(&terms, text);
        score += term_overlap_boost(&terms, &title);
        // Ebbinghaus decay: fresh + feedback-accepted memories resist decay;
        // stale session dumps sink. Nudge, never a hard filter.
        let interactions = lstore
            .as_ref()
            .and_then(|s| s.accept_count(id).ok())
            .unwrap_or(0);
        score *= recall_multiplier(
            kind_tag(kind),
            h.get("ts").and_then(|v| v.as_i64()),
            interactions,
            now_secs(),
        ) as f32;
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

    // Resolve prev_pack_id → prev pack entry (ids + kinds) via cache lookup.
    let prev_entry = prev_pack_id.and_then(|pid| {
        PACK_CACHE
            .lock()
            .ok()
            .and_then(|c| c.inner.values().find(|e| e.pack_id == pid).cloned())
    });
    // Implicit feedback: continuing from a pack is a weak acceptance signal for
    // the kinds it delivered — closes the learning loop even when the agent
    // never calls context_feedback.
    let implicit_rewards = prev_entry
        .as_ref()
        .map(|e| {
            let kinds: Vec<&str> = e.used_kinds.iter().map(String::as_str).collect();
            record_ctx_reward(&kinds, true)
        })
        .unwrap_or(0);
    let prev_used_ids: Vec<i64> = prev_entry.map(|e| e.used_ids).unwrap_or_default();

    let opts = PackOptions {
        budget_tokens: budget,
        header_reserve: 64,
        prev_used_ids: prev_used_ids.clone(),
        cache_stable_order: cache_stable,
    };
    // Delta path: skip already-packed ids. Falls back to full pack if prev is empty.
    let packed = if prev_used_ids.is_empty() {
        pack(cands, &opts)
    } else {
        pack_delta(cands, &opts)
    };
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
    let savings_pct = packed.savings_pct();
    let used_tokens = packed.used_tokens;
    let naive_tokens = packed.naive_tokens;

    // Store in cache for future hits.
    if use_cache
        && prev_pack_id.is_none()
        && let Ok(mut cache) = PACK_CACHE.lock()
    {
        cache.put(
            cache_key,
            PackCacheEntry {
                rendered: rendered.clone(),
                used_ids: used_ids.clone(),
                used_kinds: packed
                    .blocks
                    .iter()
                    .map(|b| kind_tag(b.kind).to_string())
                    .collect(),
                used_tokens,
                naive_tokens,
                savings_pct,
                pack_id: pid.clone(),
            },
        );
    }

    // Register pack for verify-gate degradation tracking.
    if let Ok(mut state) = FEEDBACK_STATE.lock() {
        state.register_pack(&pid);
    }

    // Skill-preload hints: suggest skills the router should lazy-load based on
    // query terms. The router (separate system) reads this and swaps 15k-tok
    // system-prompt skills for 0-1 actually-relevant ones.
    let preload_skills = skill_preload_hints(query);

    Ok(json!({
        "pack_id": pid,
        "context": rendered,
        "manifest": {
            "used_ids": used_ids,
            "dropped_ids": packed.dropped_ids,
            "deduped_ids": packed.deduped_ids,
            "delta_skipped_ids": packed.delta_skipped_ids,
            "used_tokens": used_tokens,
            "budget_tokens": packed.budget_tokens,
            "naive_tokens": naive_tokens,
            "savings_pct": savings_pct,
            "noise_filtered": noise_filtered,
            "cache_hit": false,
            "implicit_feedback_rewards": implicit_rewards,
            "blocks": blocks,
            "preload_skills": preload_skills,
        }
    }))
}

/// Zero-cost trigger predicate — the Phase-3 wiring for the router: hooks and
/// routers ask this before paying for a pack.
fn context_needed(args: &Value) -> Result<Value> {
    let query = args
        .get("query")
        .or_else(|| args.get("task"))
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    Ok(json!({
        "trigger": has_context_trigger(query),
        "hint": "if trigger=true, call context_pack(query, budget_tokens) next"
    }))
}

async fn context_feedback(sock: &PathBuf, args: &Value) -> Result<Value> {
    let pack_id = args.get("pack_id").and_then(|v| v.as_str()).unwrap_or("");
    let gate = args
        .get("gate")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    let used_ids: Vec<i64> = args
        .get("used_ids")
        .and_then(|v| v.as_array())
        .map(|a| a.iter().filter_map(|x| x.as_i64()).collect())
        .unwrap_or_default();

    // Resolve the kind of each used doc so we can reward the right kinds.
    let mut kinds: Vec<&'static str> = Vec::new();
    if !used_ids.is_empty() {
        let placeholders = std::iter::repeat_n("?", used_ids.len())
            .collect::<Vec<_>>()
            .join(",");
        let rows = sql_rows(
            sock,
            format!("SELECT id, title, meta FROM docs WHERE id IN ({placeholders})"),
            used_ids.iter().map(|id| json!(id)).collect(),
        )
        .await
        .unwrap_or_default();
        for r in &rows {
            let title = r.get("title").and_then(|v| v.as_str()).unwrap_or_default();
            let meta = r.get("meta").cloned().unwrap_or(Value::Null);
            kinds.push(kind_tag(hit_kind(title, &meta)));
        }
    }

    // Close the self-learning loop: gate=pass rewards the used kinds, fail dampens.
    let rewarded = if gate == "unknown" {
        0
    } else {
        record_ctx_reward(&kinds, gate == "pass")
    };

    // Mark feedback received → clears verify-gate degradation for this pack.
    if !pack_id.is_empty()
        && let Ok(mut state) = FEEDBACK_STATE.lock()
    {
        state.mark_feedback(pack_id);
    }

    // Persist the raw feedback event too (sweepable, auditable).
    let payload = json!({
        "pack_id": pack_id,
        "used_ids": used_ids,
        "gate": gate,
        "kinds": kinds,
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
    Ok(json!({
        "ok": true,
        "gate": gate,
        "rewarded_kinds": kinds,
        "learn_updates": rewarded,
    }))
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

// ── Affiliate Attribution (cookieless, E.L.A.B.-compatible) ──────────────────
// affiliate_attribute: stores ctx_hash → partner_id binding as kind=affiliate-attribution.
// Replaces 3rd-party cookies: server-side, cross-device, ad-blocker-resistent, DSGVO-clean.
// affiliate_lookup: resolves active attribution + optional E.L.A.B. event chain.
async fn affiliate_attribute(sock: &PathBuf, args: &Value) -> Result<Value> {
    let ctx_hash = args
        .get("ctx_hash")
        .and_then(|v| v.as_str())
        .context("ctx_hash")?;
    let partner_id = args
        .get("partner_id")
        .and_then(|v| v.as_str())
        .context("partner_id")?;
    let merchant_id = args
        .get("merchant_id")
        .and_then(|v| v.as_str())
        .context("merchant_id")?;
    let event_type = args
        .get("event_type")
        .and_then(|v| v.as_str())
        .context("event_type")?;
    let attribution_type = args
        .get("attribution_type")
        .and_then(|v| v.as_str())
        .unwrap_or("qr_scan");
    let expires_days = args
        .get("expires_days")
        .and_then(|v| v.as_i64())
        .unwrap_or(90);
    let now = now_secs();
    let expires_at = now + expires_days * 86400;

    let mut split = json!({"primary": 100, "secondary": 0});
    if let Some(cs) = args.get("commission_split").and_then(|v| v.as_object()) {
        if let Some(p) = cs.get("primary").and_then(|v| v.as_i64()) {
            split["primary"] = json!(p);
        }
        if let Some(s) = cs.get("secondary").and_then(|v| v.as_i64()) {
            split["secondary"] = json!(s);
        }
    }
    let mut source = json!({});
    if let Some(sd) = args.get("source_data").and_then(|v| v.as_object()) {
        source = json!(sd);
    }

    let title = format!("affiliate/{ctx_hash}/{merchant_id}/{event_type}/{partner_id}");
    let text = format!(
        "ctx_hash={ctx_hash}\npartner_id={partner_id}\nmerchant_id={merchant_id}\n\
         event_type={event_type}\nattribution_type={attribution_type}\n\
         commission_split={split}\nsource_data={source}\nexpires_at={expires_at}"
    );
    let meta = json!({
        "schema": "synapse.ctxos.v1",
        "kind": "affiliate-attribution",
        "ctx_hash": ctx_hash,
        "partner_id": partner_id,
        "merchant_id": merchant_id,
        "event_type": event_type,
        "attribution_type": attribution_type,
        "commission_split": split,
        "source_data": source,
        "expires_at": expires_at,
        "ts": now,
    });

    daemon_call(
        sock,
        json!({"op": "Put", "args": {
            "title": title,
            "uri": Value::Null,
            "text": text,
            "meta": meta,
            "embed": false,
        }}),
    )
    .await
}

async fn affiliate_lookup(sock: &PathBuf, args: &Value) -> Result<Value> {
    let ctx_hash = args
        .get("ctx_hash")
        .and_then(|v| v.as_str())
        .context("ctx_hash")?;
    let merchant_id = args
        .get("merchant_id")
        .and_then(|v| v.as_str())
        .context("merchant_id")?;
    let include_chain = args
        .get("include_chain")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let q = format!("affiliate/{ctx_hash}/{merchant_id}/");
    let res = daemon_call(
        sock,
        json!({"op": "Search", "args": {"q": q, "mode": "Lex", "limit": 64, "embed_query": false}}),
    )
    .await?;

    let now = now_secs();
    let mut hits: Vec<Value> = Vec::new();
    if let Some(docs) = res.get("Hits").and_then(|v| v.as_array()) {
        for d in docs {
            let meta = d.get("meta").cloned().unwrap_or(Value::Null);
            let expires_at = meta.get("expires_at").and_then(|v| v.as_i64()).unwrap_or(0);
            if expires_at > 0 && now > expires_at {
                continue;
            }
            if !include_chain {
                let et = meta
                    .get("event_type")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                if et == "claim" {
                    continue;
                }
            }
            hits.push(json!({
                "partner_id": meta.get("partner_id").cloned().unwrap_or(Value::Null),
                "event_type": meta.get("event_type").cloned().unwrap_or(Value::Null),
                "attribution_type": meta.get("attribution_type").cloned().unwrap_or(Value::Null),
                "commission_split": meta.get("commission_split").cloned().unwrap_or(Value::Null),
                "source_data": meta.get("source_data").cloned().unwrap_or(Value::Null),
                "expires_at": expires_at,
                "ts": meta.get("ts").cloned().unwrap_or(Value::Null),
                "doc_id": d.get("id").cloned().unwrap_or(Value::Null),
            }));
        }
    }
    let primary = hits
        .iter()
        .find(|h| {
            h.get("event_type").and_then(|v| v.as_str()) == Some("invitation")
                || h.get("event_type").and_then(|v| v.as_str()) == Some("opt_in")
        })
        .cloned();
    Ok(json!({
        "ctx_hash": ctx_hash,
        "merchant_id": merchant_id,
        "active": !hits.is_empty(),
        "primary_attribution": primary,
        "event_chain": hits,
        "chain_length": hits.len(),
    }))
}

// ── Swarm / Mega-Session Spiegelung ───────────────────────────────────────────
// session_ingest: nimmt Events aus cmux/Swarm-Sessions (Tool-Calls, Antworten,
// File-Edits, Decisions) und speichert sie als kind=session-summary Docs mit
// meta.session_id, meta.agent_role, meta.turn_range. Das ermöglicht späteres
// session_replay + progressive Summarization.
async fn session_ingest(sock: &PathBuf, args: &Value) -> Result<Value> {
    let session_id = args
        .get("session_id")
        .and_then(|v| v.as_str())
        .context("session_id")?;
    let agent_role = args
        .get("agent_role")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    let events = args
        .get("events")
        .and_then(|v| v.as_array())
        .context("events")?;
    let summary = args.get("summary").and_then(|v| v.as_str()).unwrap_or("");
    let turn_start = args.get("turn_start").and_then(|v| v.as_u64()).unwrap_or(0) as i64;
    let turn_end = args.get("turn_end").and_then(|v| v.as_u64()).unwrap_or(0) as i64;
    let repo = args.get("repo").and_then(|v| v.as_str()).unwrap_or("");

    // Build a compact verbatim event log: one line per event.
    let mut log = String::new();
    for ev in events {
        let turn = ev.get("turn").and_then(|v| v.as_u64()).unwrap_or(0);
        let kind = ev.get("kind").and_then(|v| v.as_str()).unwrap_or("event");
        let tool = ev.get("tool").and_then(|v| v.as_str()).unwrap_or("");
        let content = ev.get("content").and_then(|v| v.as_str()).unwrap_or("");
        // Truncate individual event content to keep the log packable.
        let snippet: String = content.chars().take(500).collect();
        log.push_str(&format!("[t{turn} {kind} {tool}] {snippet}\n"));
    }
    if !summary.is_empty() {
        log.push_str(&format!("\n=== SUMMARY ===\n{summary}\n"));
    }

    let title = format!("session/{session_id}/{agent_role}/t{turn_start}-t{turn_end}");
    let mut meta = json!({
        "schema": "synapse.ctxos.v1",
        "kind": "session-summary",
        "session_id": session_id,
        "agent_role": agent_role,
        "turn_range": [turn_start, turn_end],
    });
    if !repo.is_empty() {
        meta["repo"] = json!(repo);
    }

    daemon_call(
        sock,
        json!({"op": "Put", "args": {
            "title": title,
            "uri": Value::Null,
            "text": log,
            "meta": meta,
            "embed": true,
        }}),
    )
    .await
}

// session_replay: baut eine Session aus gespeicherten session-summary Docs wieder
// auf. Sucht nach meta.session_id == <id>, sortiert nach turn_range, packt sie.
async fn session_replay(sock: &PathBuf, args: &Value) -> Result<Value> {
    let session_id = args
        .get("session_id")
        .and_then(|v| v.as_str())
        .context("session_id")?;
    let budget = args
        .get("budget_tokens")
        .and_then(|v| v.as_u64())
        .unwrap_or(8000) as usize;
    let agent_role: Option<&str> = args.get("agent_role").and_then(|v| v.as_str());

    // Query session-summary docs for this session via SQL filter on meta.
    let filter_clause = if let Some(role) = agent_role {
        format!(
            "WHERE meta LIKE '%\"session_id\":\"{sid}\"%' AND meta LIKE '%\"agent_role\":\"{role}\"%'",
            sid = session_id,
        )
    } else {
        format!(
            "WHERE meta LIKE '%\"session_id\":\"{sid}\"%'",
            sid = session_id
        )
    };
    let rows = sql_rows(
        sock,
        format!(
            "SELECT id, title, text, meta, ts FROM docs {filter_clause} ORDER BY ts ASC LIMIT 200"
        ),
        Vec::new(),
    )
    .await
    .unwrap_or_default();

    let mut cands = Vec::new();
    for r in &rows {
        let id = r.get("id").and_then(|v| v.as_i64()).unwrap_or_default();
        let title = r
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        let text = r
            .get("text")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        if text.is_empty() {
            continue;
        }
        let meta = r.get("meta").cloned().unwrap_or(Value::Null);
        let kind = hit_kind(&title, &meta);
        cands.push(Candidate {
            id,
            title,
            text,
            score: 1.0, // all session events equal weight; order by ts (already sorted)
            kind,
        });
    }

    if cands.is_empty() {
        return Ok(json!({
            "session_id": session_id,
            "events": 0,
            "context": "",
            "manifest": {"used_tokens": 0, "budget_tokens": budget, "savings_pct": 0.0},
        }));
    }

    let opts = PackOptions {
        budget_tokens: budget,
        header_reserve: 64,
        ..PackOptions::default()
    };
    let packed = pack(cands, &opts);
    let rendered = render(&packed);
    let used_ids: Vec<i64> = packed.blocks.iter().map(|b| b.id).collect();
    let pid = pack_id(session_id, &used_ids);

    Ok(json!({
        "pack_id": pid,
        "session_id": session_id,
        "events": packed.blocks.len(),
        "context": rendered,
        "manifest": {
            "used_ids": used_ids,
            "used_tokens": packed.used_tokens,
            "budget_tokens": packed.budget_tokens,
            "naive_tokens": packed.naive_tokens,
            "savings_pct": packed.savings_pct(),
        }
    }))
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

/// smx_* tool schemas — only present in the full (engine) build.
#[cfg(feature = "market")]
fn market_tools() -> Vec<Value> {
    vec![
        json!({"name": "smx_candles", "description": "Return OHLCV candles for a ticker in a time range.", "inputSchema": {"type": "object", "properties": {
            "ticker": {"type": "string"},
            "start": {"type": "integer", "description": "Unix timestamp seconds"},
            "end": {"type": "integer", "description": "Unix timestamp seconds"},
            "limit": {"type": "integer", "default": 500}
        }, "required": ["ticker", "start", "end"]}}),
        json!({"name": "smx_signal_similar", "description": "Find N most similar past market regimes for ticker at date_ts.", "inputSchema": {"type": "object", "properties": {
            "ticker": {"type": "string"},
            "date_ts": {"type": "integer", "description": "Unix timestamp seconds of reference day"},
            "n": {"type": "integer", "default": 10}
        }, "required": ["ticker", "date_ts"]}}),
        json!({"name": "smx_pattern_stats", "description": "Aggregate stats for a named pattern across all stored signals.", "inputSchema": {"type": "object", "properties": {
            "pattern": {"type": "string", "description": "Pattern name or SQL LIKE expression"}
        }, "required": ["pattern"]}}),
        json!({"name": "smx_correlation", "description": "Return pairwise close-price correlation matrix for tickers over last N days.", "inputSchema": {"type": "object", "properties": {
            "tickers": {"type": "array", "items": {"type": "string"}},
            "days": {"type": "integer", "default": 30}
        }, "required": ["tickers"]}}),
    ]
}

/// Route smx_* tool calls; bails in the portable build that excludes the engine.
#[cfg(feature = "market")]
fn market_dispatch(market_db: &PathBuf, name: &str, args: &Value) -> Result<Value> {
    market_tool_call(market_db, name, args)
}

#[cfg(not(feature = "market"))]
fn market_dispatch(_market_db: &PathBuf, _name: &str, _args: &Value) -> Result<Value> {
    anyhow::bail!(
        "market (smx_*) tools are not built in this binary; rebuild with --features market"
    )
}

/// Handle smx_* market tools locally (no synapsed socket needed).
#[cfg(feature = "market")]
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

#[cfg(feature = "market")]
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

    #[test]
    fn term_overlap_boost_scales_with_matches() {
        let terms = vec![
            "synapse".to_string(),
            "context".to_string(),
            "packer".to_string(),
        ];
        let none = term_overlap_boost(&terms, "totally unrelated prose");
        let some = term_overlap_boost(&terms, "the synapse context engine");
        let all = term_overlap_boost(&terms, "synapse context packer notes");
        assert_eq!(none, 0.0);
        assert!(some > none && all > some, "more matches => bigger boost");
        assert!(all <= 0.1 + f32::EPSILON, "boost is bounded");
        assert_eq!(term_overlap_boost(&[], "anything"), 0.0);
    }

    #[test]
    fn is_noise_drops_spam_keeps_knowledge() {
        let telepathy = json!({"text": "[telepathy][ollama.status] {\"models_loaded\": 18}"});
        let notif = json!({"text": "<task-notification> <task-id>abc</task-id> done"});
        let status = json!({"text": "{\"desktop_procs\": 0, \"cli_sessions\": 2}"});
        let log = json!({"uri": "sched_briefing.log", "text": "Agent [briefing] log running ..."});
        let stub = json!({"text": "ok"});
        let real = json!({"title": "known-fact:speedtune", "text": "Context-OS packs verbatim STATE within a token budget; deletion tiers."});
        for n in [&telepathy, &notif, &status, &log, &stub] {
            assert!(is_noise(n), "should drop noise: {n}");
        }
        assert!(!is_noise(&real), "must keep real knowledge");
    }

    #[test]
    fn noise_features_and_logistic_apply() {
        let f = noise_features("known-fact:x", "", "value = 42\npath src/a.rs:9");
        // keys present + flags correct (parity with the Python trainer)
        assert!(f.contains_key("log_len") && f.contains_key("uniq_ratio"));
        assert_eq!(f["title_marker"], 1.0); // title has ':'
        assert_eq!(f["uri_log"], 0.0);
        assert!(f["frac_digit"] > 0.0);
        // logistic apply: positive bias + no weights => sigmoid(bias)
        let m = NoiseModel {
            weights: HashMap::new(),
            bias: 0.0,
            threshold: 0.5,
            ..Default::default()
        };
        assert!((learned_noise_prob(&m, &f) - 0.5).abs() < 1e-9);
    }

    #[test]
    fn pack_cache_put_get_lru_evicts() {
        let mut cache = PackCache::default();
        let k1 = PackCache::key("alpha", 1000, None);
        let k2 = PackCache::key("beta", 1000, None);
        let e1 = PackCacheEntry {
            rendered: "STATE alpha".into(),
            used_ids: vec![1, 2],
            used_kinds: vec!["decision".into()],
            used_tokens: 100,
            naive_tokens: 500,
            savings_pct: 80.0,
            pack_id: "pid-alpha".into(),
        };
        cache.put(k1, e1.clone());
        assert_eq!(cache.len(), 1);
        assert!(cache.get(k1).is_some());
        assert!(cache.get(k2).is_none(), "unrelated key must miss");
        cache.put(k2, e1.clone());
        assert_eq!(cache.len(), 2);
    }

    #[test]
    fn pack_cache_key_differs_on_prev_pack_id() {
        let k_no_prev = PackCache::key("query", 1000, None);
        let k_with_prev = PackCache::key("query", 1000, Some("pid-abc"));
        assert_ne!(
            k_no_prev, k_with_prev,
            "prev_pack_id must change the cache key"
        );
    }

    #[test]
    fn pack_cache_key_stable_for_same_inputs() {
        let k1 = PackCache::key("query", 1000, None);
        let k2 = PackCache::key("query", 1000, None);
        assert_eq!(k1, k2, "same inputs must hash to same key");
    }

    #[test]
    fn pack_cache_lru_evicts_oldest_when_full() {
        let mut cache = PackCache::default();
        // Fill to capacity.
        for i in 0..PACK_CACHE_CAP {
            let k = PackCache::key(&format!("q{i}"), 1000, None);
            cache.put(
                k,
                PackCacheEntry {
                    rendered: format!("STATE {i}"),
                    used_ids: vec![i as i64],
                    used_kinds: vec!["chat".into()],
                    used_tokens: 10,
                    naive_tokens: 50,
                    savings_pct: 80.0,
                    pack_id: format!("pid-{i}"),
                },
            );
        }
        assert_eq!(cache.len(), PACK_CACHE_CAP);
        // Insert one more → oldest (q0) must be evicted.
        let new_k = PackCache::key("q-new", 1000, None);
        cache.put(
            new_k,
            PackCacheEntry {
                rendered: "STATE new".into(),
                used_ids: vec![999],
                used_kinds: vec!["other".into()],
                used_tokens: 10,
                naive_tokens: 50,
                savings_pct: 80.0,
                pack_id: "pid-new".into(),
            },
        );
        assert_eq!(cache.len(), PACK_CACHE_CAP, "cap must be maintained");
        let old_k = PackCache::key("q0", 1000, None);
        assert!(cache.get(old_k).is_none(), "oldest entry must be evicted");
        assert!(cache.get(new_k).is_some(), "newest entry must be present");
    }

    #[test]
    fn feedback_state_degrades_after_three_open_packs() {
        let mut state = FeedbackState::default();
        // No packs → no degradation.
        assert_eq!(state.budget_factor(4000), 4000);
        // Register 3 packs with no feedback → 60% budget.
        state.register_pack("pid-1");
        state.register_pack("pid-2");
        state.register_pack("pid-3");
        assert_eq!(state.budget_factor(4000), 2400);
        // Mark feedback for one → still 2 open, no degradation.
        state.mark_feedback("pid-1");
        assert_eq!(state.budget_factor(4000), 4000);
        // 6+ open packs → 40% budget.
        for i in 4..=7 {
            state.register_pack(&format!("pid-{i}"));
        }
        assert_eq!(state.budget_factor(4000), 1600);
    }

    #[test]
    fn skill_preload_hints_match_trading_query() {
        let hints = skill_preload_hints("how do I build a trading portfolio with kelly sizing");
        assert!(
            hints.contains(&"asymbet"),
            "asymbet must be hinted for trading query"
        );
        assert!(hints.contains(&"winvestment-profet"));
        assert!(hints.len() <= 3);
    }

    #[test]
    fn skill_preload_hints_match_code_query() {
        let hints = skill_preload_hints("refactor the rust repo and fix the cargo build bug");
        assert!(hints.contains(&"agent-token-saver"));
    }

    #[test]
    fn skill_preload_hints_empty_for_greeting() {
        let hints = skill_preload_hints("hello how are you today my friend");
        assert!(hints.is_empty(), "no skill hints for greetings");
    }

    #[test]
    fn has_context_trigger_matches_real_tasks() {
        assert!(has_context_trigger(
            "how do I implement a delta pack in synapse-pack"
        ));
        assert!(has_context_trigger(
            "wie kann ich das token-saving optimieren"
        ));
        assert!(!has_context_trigger("hi"));
        assert!(!has_context_trigger("thanks"));
        assert!(!has_context_trigger("ok"));
    }
}
