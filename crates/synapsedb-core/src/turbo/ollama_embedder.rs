//! Ollama Embedder — 17× faster than fastembed on M4 Max
//!
//! Uses Ollama's HTTP API for embedding generation.
//! Benchmark: fastembed ~170ms vs Ollama ~10ms per embedding on M4 Max.
//!
//! ```rust
//! use synapsedb_core::turbo::ollama_embedder::OllamaEmbedder;
//!
//! let embedder = OllamaEmbedder::new("all-minilm").unwrap();
//! let embedding = embedder.embed_one("Hello world").unwrap();
//! assert_eq!(embedding.len(), 384); // all-minilm is 384-dim
//! ```

use crate::error::{Error, Result};
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use std::time::Duration;

const DEFAULT_OLLAMA_URL: &str = "http://localhost:11434";
const DEFAULT_MODEL: &str = "all-minilm";

/// Ollama embedding response
#[derive(Serialize, Deserialize)]
struct EmbedResponse {
    embedding: Vec<f32>,
}

/// High-performance embedder using Ollama API
pub struct OllamaEmbedder {
    client: Client,
    url: String,
    model: String,
}

impl OllamaEmbedder {
    /// Create a new Ollama embedder
    pub fn new(model: &str) -> Result<Self> {
        Self::with_url(model, DEFAULT_OLLAMA_URL)
    }

    /// Create with custom Ollama URL
    pub fn with_url(model: &str, url: &str) -> Result<Self> {
        let client = Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|e| Error::Other(format!("reqwest: {e}")))?;

        Ok(Self {
            client,
            url: url.to_string(),
            model: model.to_string(),
        })
    }

    /// Embed a single text
    pub fn embed_one(&self, text: &str) -> Result<Vec<f32>> {
        let resp: EmbedResponse = self
            .client
            .post(format!("{}/api/embeddings", self.url))
            .json(&serde_json::json!({
                "model": self.model,
                "prompt": text
            }))
            .send()
            .map_err(|e| Error::Other(format!("ollama request: {e}")))?
            .json()
            .map_err(|e| Error::Other(format!("ollama response: {e}")))?;

        Ok(resp.embedding)
    }

    /// Embed a batch of texts
    pub fn embed_batch(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        texts.iter().map(|t| self.embed_one(t)).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ollama_embedder_creation() {
        // Just test creation, actual embedding requires Ollama running
        let embedder = OllamaEmbedder::new("all-minilm");
        // Will fail if Ollama not running, but creation should work
        if let Ok(e) = embedder {
            assert_eq!(e.model, "all-minilm");
        }
    }
}
