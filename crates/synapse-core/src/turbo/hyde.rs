//! HyDE (Hypothetical Document Embedding) query augmentation.
//!
//! Sends the user query to a local Ollama model, obtains a hypothetical answer,
//! then substitutes that text for the raw query before embedding.
//! Falls back silently to the original query when Ollama is unreachable.

use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tracing::warn;

const DEFAULT_OLLAMA_URL: &str = "http://localhost:11434";
const PROMPT_PREFIX: &str = "Write a short passage that directly answers the question: ";

/// Configuration for HyDE query augmentation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HydeConfig {
    /// Ollama model name (default: "phi4-mini").
    pub model: String,
    /// Max tokens for the hypothetical document (default: 128).
    pub max_tokens: u32,
    /// Ollama base URL (default: "http://localhost:11434").
    pub ollama_url: String,
}

impl Default for HydeConfig {
    fn default() -> Self {
        Self {
            model: "phi4-mini".to_string(),
            max_tokens: 128,
            ollama_url: DEFAULT_OLLAMA_URL.to_string(),
        }
    }
}

#[derive(Serialize)]
struct GenerateRequest<'a> {
    model: &'a str,
    prompt: String,
    stream: bool,
    options: GenerateOptions,
}

#[derive(Serialize)]
struct GenerateOptions {
    num_predict: u32,
}

#[derive(Deserialize)]
struct GenerateResponse {
    response: String,
}

/// Expand `query` into a hypothetical document via Ollama.
///
/// On any network/parse error, logs a warning and returns the original query
/// unchanged (silent fallback — never propagates errors).
pub fn expand(config: &HydeConfig, query: &str) -> String {
    match try_expand(config, query) {
        Ok(text) if !text.trim().is_empty() => text,
        Ok(_) => {
            warn!(model = %config.model, "HyDE: empty response, using original query");
            query.to_string()
        }
        Err(e) => {
            warn!(model = %config.model, error = %e, "HyDE: Ollama unavailable, fallback to original query");
            query.to_string()
        }
    }
}

fn try_expand(config: &HydeConfig, query: &str) -> Result<String, String> {
    let client = Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|e| format!("client build: {e}"))?;

    let body = GenerateRequest {
        model: &config.model,
        prompt: format!("{PROMPT_PREFIX}{query}"),
        stream: false,
        options: GenerateOptions {
            num_predict: config.max_tokens,
        },
    };

    let resp = client
        .post(format!("{}/api/generate", config.ollama_url))
        .json(&body)
        .send()
        .map_err(|e| format!("request: {e}"))?;

    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()));
    }

    let parsed: GenerateResponse = resp.json().map_err(|e| format!("parse: {e}"))?;

    Ok(parsed.response.trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expand_fallback_when_ollama_down() {
        let cfg = HydeConfig {
            model: "phi4-mini".to_string(),
            max_tokens: 64,
            ollama_url: "http://127.0.0.1:19999".to_string(), // nothing listening
        };
        let result = expand(&cfg, "what is Synapse?");
        assert_eq!(result, "what is Synapse?");
    }

    #[test]
    fn expand_smoke_real_ollama() {
        if std::env::var("OLLAMA_AVAILABLE").is_err() {
            return; // skip unless env set
        }
        let cfg = HydeConfig::default();
        let result = expand(&cfg, "what is vector search?");
        // Must be longer than original query if LLM actually ran
        assert!(result.len() > "what is vector search?".len());
    }
}
