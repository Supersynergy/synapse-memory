//! MiniMax-M2.7 highspeed LLM hooks for the SOTA pipeline.
//!
//! Implements `synapse_core::sota_pipeline::PipelineHooks` (decompose / grade /
//! hyde / summarize / merge) and `Extractor` against MiniMax's
//! OpenAI-compatible chat-completions endpoint.
//!
//! Env:
//!   MINIMAX_API_KEY       (required)
//!   MINIMAX_MODEL         (default: "MiniMax-M2.7-highspeed")
//!   MINIMAX_API_BASE      (default: "https://api.minimax.io/v1")
//!   MINIMAX_TIMEOUT_MS    (default: 8000)

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use synapse_core::error::Result as CoreResult;
use synapse_core::sota::MemoryType;
use synapse_core::sota_pipeline::PipelineHooks;

use crate::{ExtractedMemory, Extractor};

const DEFAULT_MODEL: &str = "MiniMax-M2";
const DEFAULT_BASE: &str = "https://api.minimax.io/v1";
const DEFAULT_TIMEOUT_MS: u64 = 8000;

#[derive(Debug, Clone)]
pub struct MinimaxClient {
    api_key: String,
    model: String,
    base: String,
    http: reqwest::blocking::Client,
}

#[derive(Serialize)]
struct ChatReq<'a> {
    model: &'a str,
    messages: Vec<Msg<'a>>,
    temperature: f32,
    max_tokens: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    response_format: Option<RespFmt>,
}

#[derive(Serialize)]
struct Msg<'a> {
    role: &'a str,
    content: &'a str,
}

#[derive(Serialize)]
struct RespFmt {
    #[serde(rename = "type")]
    ty: String,
}

#[derive(Deserialize)]
struct ChatResp {
    choices: Vec<Choice>,
}

#[derive(Deserialize)]
struct Choice {
    message: ChoiceMsg,
}

#[derive(Deserialize)]
struct ChoiceMsg {
    content: String,
}

impl MinimaxClient {
    pub fn from_env() -> Result<Self> {
        let api_key =
            std::env::var("MINIMAX_API_KEY").map_err(|_| anyhow!("MINIMAX_API_KEY not set"))?;
        let model = std::env::var("MINIMAX_MODEL").unwrap_or_else(|_| DEFAULT_MODEL.into());
        let base = std::env::var("MINIMAX_API_BASE").unwrap_or_else(|_| DEFAULT_BASE.into());
        let timeout_ms: u64 = std::env::var("MINIMAX_TIMEOUT_MS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(DEFAULT_TIMEOUT_MS);
        let http = reqwest::blocking::Client::builder()
            .timeout(Duration::from_millis(timeout_ms))
            .build()?;
        Ok(Self {
            api_key,
            model,
            base,
            http,
        })
    }

    pub fn new(api_key: String, model: String, base: String, timeout_ms: u64) -> Result<Self> {
        let http = reqwest::blocking::Client::builder()
            .timeout(Duration::from_millis(timeout_ms))
            .build()?;
        Ok(Self {
            api_key,
            model,
            base,
            http,
        })
    }

    fn chat(&self, system: &str, user: &str, max_tokens: u32, json: bool) -> Result<String> {
        let req = ChatReq {
            model: &self.model,
            messages: vec![
                Msg {
                    role: "system",
                    content: system,
                },
                Msg {
                    role: "user",
                    content: user,
                },
            ],
            temperature: 0.0,
            max_tokens,
            response_format: if json {
                Some(RespFmt {
                    ty: "json_object".into(),
                })
            } else {
                None
            },
        };
        let url = format!("{}/chat/completions", self.base.trim_end_matches('/'));
        let resp = self
            .http
            .post(&url)
            .bearer_auth(&self.api_key)
            .json(&req)
            .send()?;
        if !resp.status().is_success() {
            let s = resp.status();
            let b = resp.text().unwrap_or_default();
            return Err(anyhow!("minimax http {}: {}", s, b));
        }
        let parsed: ChatResp = resp.json()?;
        let raw = parsed
            .choices
            .into_iter()
            .next()
            .map(|c| c.message.content)
            .ok_or_else(|| anyhow!("minimax: empty choices"))?;
        Ok(strip_thinking(&raw))
    }
}

/// MiniMax-M2 emits `<think>...</think>` reasoning blocks before the actual
/// response. Strip them so JSON-mode parsers see clean payloads.
fn strip_thinking(s: &str) -> String {
    let mut out = s.to_string();
    while let (Some(open), Some(close)) = (out.find("<think>"), out.find("</think>")) {
        if close > open {
            out.replace_range(open..close + "</think>".len(), "");
        } else {
            break;
        }
    }
    out.trim().to_string()
}

/// PipelineHooks impl backed by MiniMax. All methods degrade to the trait
/// default on transport / parse error so a flaky network never breaks recall.
pub struct MinimaxHooks {
    client: MinimaxClient,
}

impl MinimaxHooks {
    pub fn new(client: MinimaxClient) -> Self {
        Self { client }
    }
    pub fn from_env() -> Result<Self> {
        Ok(Self {
            client: MinimaxClient::from_env()?,
        })
    }
}

impl PipelineHooks for MinimaxHooks {
    fn decompose(&self, query: &str) -> CoreResult<Vec<String>> {
        let sys = "You are a query decomposition module. Split the user's question into atomic sub-questions. Return JSON: {\"sub_queries\": [\"...\", \"...\"]}. If the question is already atomic, return the original as the only element. Max 4 sub-queries.";
        let raw = match self.client.chat(sys, query, 2048, true) {
            Ok(s) => s,
            Err(_) => return Ok(vec![query.to_string()]),
        };
        #[derive(Deserialize)]
        struct Out {
            sub_queries: Vec<String>,
        }
        match serde_json::from_str::<Out>(&raw) {
            Ok(o) if !o.sub_queries.is_empty() => Ok(o.sub_queries),
            _ => Ok(vec![query.to_string()]),
        }
    }

    fn grade(&self, query: &str, doc: &str) -> CoreResult<f64> {
        let sys = "You are a Self-RAG relevance grader. Given a query and a document, output JSON {\"score\": <float 0..1>} where 1 = directly answers, 0 = irrelevant. No prose.";
        let user = format!(
            "Query: {}\n\nDocument:\n{}",
            query,
            &doc[..doc.len().min(2000)]
        );
        let raw = match self.client.chat(sys, &user, 1024, true) {
            Ok(s) => s,
            Err(_) => return Ok(0.5),
        };
        #[derive(Deserialize)]
        struct Out {
            score: f64,
        }
        match serde_json::from_str::<Out>(&raw) {
            Ok(o) => Ok(o.score.clamp(0.0, 1.0)),
            Err(_) => Ok(0.5),
        }
    }

    fn hyde(&self, query: &str) -> CoreResult<String> {
        let sys = "Generate a single short hypothetical answer paragraph (<=80 words) that COULD plausibly answer the user's question. Output prose only, no preamble.";
        match self.client.chat(sys, query, 1024, false) {
            Ok(s) => Ok(s.trim().to_string()),
            Err(_) => Ok(format!(
                "{} relates to specific facts and recent context.",
                query.trim()
            )),
        }
    }

    fn summarize(&self, items: &[&str]) -> CoreResult<String> {
        if items.is_empty() {
            return Ok(String::new());
        }
        let sys = "Summarize the following memory snippets into one canonical statement, preserving every distinct fact. Output prose only.";
        let joined = items
            .iter()
            .enumerate()
            .map(|(i, s)| format!("[{i}] {}", &s[..s.len().min(800)]))
            .collect::<Vec<_>>()
            .join("\n");
        match self.client.chat(sys, &joined, 256, false) {
            Ok(s) => Ok(s.trim().to_string()),
            Err(_) => {
                // fallback identical to trait default
                let mut out = String::new();
                for s in items {
                    if !out.is_empty() {
                        out.push_str(" / ");
                    }
                    let head: String = s.chars().take(200).collect();
                    out.push_str(head.trim());
                }
                Ok(out)
            }
        }
    }

    fn merge(&self, existing: &str, new_text: &str) -> CoreResult<String> {
        if existing.contains(new_text.trim()) {
            return Ok(existing.to_string());
        }
        let sys = "Merge NEW into EXISTING memory. Keep all distinct facts from both. Drop duplicates. Output prose only.";
        let user = format!("EXISTING:\n{}\n\nNEW:\n{}", existing, new_text);
        match self.client.chat(sys, &user, 384, false) {
            Ok(s) => Ok(s.trim().to_string()),
            Err(_) => Ok(format!("{}\n{}", existing.trim_end(), new_text.trim())),
        }
    }
}

/// Extractor impl backed by MiniMax for high-quality fact extraction.
pub struct MinimaxExtractor {
    client: MinimaxClient,
}

impl MinimaxExtractor {
    pub fn new(client: MinimaxClient) -> Self {
        Self { client }
    }
    pub fn from_env() -> Result<Self> {
        Ok(Self {
            client: MinimaxClient::from_env()?,
        })
    }
}

#[derive(Deserialize)]
pub struct ExtractItem {
    #[serde(rename = "type")]
    ty: String,
    fact: String,
    #[serde(default)]
    entity: Option<String>,
    #[serde(default)]
    entities: Vec<String>,
    #[serde(default = "default_conf")]
    confidence: f64,
    #[serde(default)]
    event_date: Option<String>,
}

fn default_conf() -> f64 {
    0.8
}

/// Mem0-v3 hierarchical-extract output: facts + summary + topics in ONE call.
/// Shape mirrors mem0ai/mem0 v3 schema. Saves 2/3 of LLM round-trips vs v2.
#[derive(Deserialize)]
pub struct HierarchicalExtract {
    pub facts: Vec<ExtractItem>,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub topics: Vec<String>,
}

const HIER_EXTRACT_PROMPT: &str = r#"Extract from the passage. Output JSON:
{
  "facts": [{"type":"fact|decision|lesson|preference|episodic",
             "fact":"<one sentence>",
             "entities":["canonical name", ...],
             "event_date":"YYYY-MM-DD or null",
             "confidence":0..1}],
  "summary": "1-2 sentence gist",
  "topics": ["topic1","topic2"]
}
RULES:
- 1 atomic fact per item.
- type=preference if user states like/want/dislike.
- type=decision if action chosen.
- type=lesson if outcome insight.
- type=episodic if dated event.
- type=fact otherwise.
- event_date = WHEN event happened in source text (NOT now). null if unknown.
- Max 8 facts.
"#;

impl MinimaxExtractor {
    /// Hierarchical single-pass extract — returns full Mem0-v3 shape including
    /// summary + topics for upstream caller (e.g. evolve/compact).
    pub fn extract_hierarchical(&self, text: &str) -> Result<HierarchicalExtract> {
        let user = &text[..text.len().min(4000)];
        let raw = self.client.chat(HIER_EXTRACT_PROMPT, user, 4096, true)?;
        serde_json::from_str(&raw).map_err(|e| {
            anyhow!(
                "minimax hierarchical parse: {} | raw={}",
                e,
                &raw[..raw.len().min(200)]
            )
        })
    }
}

impl Extractor for MinimaxExtractor {
    fn name(&self) -> &'static str {
        "minimax-m2.7"
    }

    fn extract(&self, text: &str) -> Result<Vec<ExtractedMemory>> {
        // Single-pass hierarchical (Mem0-v3): one LLM call → facts+summary+topics.
        // We discard summary/topics here; the worker daemon should call
        // `extract_hierarchical` directly to capture them for evolve/compact.
        let h = self.extract_hierarchical(text)?;
        Ok(h.facts
            .into_iter()
            .map(|it| ExtractedMemory {
                memory_type: MemoryType::parse(&it.ty),
                // Prefer first canonical entity; fall back to legacy `entity` field.
                entity: it.entities.into_iter().next().or(it.entity),
                fact: it.fact,
                confidence: it.confidence.clamp(0.0, 1.0),
                event_date: it.event_date,
                // Auto-relate hook fills these elsewhere; legacy `extract` returns none.
                relations: Vec::new(),
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_requires_api_key() {
        // Unset key path returns Err.
        std::env::remove_var("MINIMAX_API_KEY");
        assert!(MinimaxClient::from_env().is_err());
    }
}
