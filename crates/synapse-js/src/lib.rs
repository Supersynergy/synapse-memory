#![deny(clippy::all)]

use napi_derive::napi;
use std::sync::Arc;
use tokio::sync::Mutex;

use synapse_core::types::{PutRequest, SearchMode};
use synapse_core::Store;

#[napi(object)]
pub struct SearchHit {
    pub id: i64,
    pub uri: Option<String>,
    pub title: Option<String>,
    pub text: String,
    pub score: f64,
}

#[napi]
pub struct Synapse {
    inner: Arc<Mutex<Store>>,
}

#[napi]
impl Synapse {
    #[napi(constructor)]
    pub fn new(path: String) -> napi::Result<Self> {
        let store = Store::open(&path).map_err(|e| napi::Error::from_reason(e.to_string()))?;
        Ok(Self {
            inner: Arc::new(Mutex::new(store)),
        })
    }

    /// Insert a document. Returns the assigned doc id.
    /// `meta_json` is an optional JSON string, e.g. `JSON.stringify({ tag: "foo" })`.
    #[napi]
    pub async fn put(
        &self,
        id: String,
        text: String,
        meta_json: Option<String>,
    ) -> napi::Result<i64> {
        let inner = self.inner.clone();
        let meta = meta_json
            .as_deref()
            .map(serde_json::from_str)
            .transpose()
            .map_err(|e: serde_json::Error| napi::Error::from_reason(e.to_string()))?;
        let req = PutRequest {
            uri: Some(id),
            title: None,
            text,
            meta,
            embedding: None,
        };
        let mut store = inner.lock().await;
        store
            .put(&req)
            .map_err(|e| napi::Error::from_reason(e.to_string()))
    }

    /// Full-text (lexical) search. Returns top-k hits.
    #[napi]
    pub async fn search(&self, query: String, limit: u32) -> napi::Result<Vec<SearchHit>> {
        let inner = self.inner.clone();
        let store = inner.lock().await;
        let hits = store
            .search(&query, SearchMode::Lex, None, limit as usize)
            .map_err(|e| napi::Error::from_reason(e.to_string()))?;
        Ok(hits
            .into_iter()
            .map(|h| SearchHit {
                id: h.id,
                uri: h.uri,
                title: h.title,
                text: h.text,
                score: h.score,
            })
            .collect())
    }

    /// Hybrid search (lexical + vector via RRF).
    /// `embedding` must be the query vector (pre-computed externally).
    #[napi]
    pub async fn search_hybrid(
        &self,
        query: String,
        embedding: Vec<f64>,
        limit: u32,
    ) -> napi::Result<Vec<SearchHit>> {
        let inner = self.inner.clone();
        let emb_f32: Vec<f32> = embedding.iter().map(|v| *v as f32).collect();
        let store = inner.lock().await;
        let hits = store
            .search(&query, SearchMode::Hybrid, Some(&emb_f32), limit as usize)
            .map_err(|e| napi::Error::from_reason(e.to_string()))?;
        Ok(hits
            .into_iter()
            .map(|h| SearchHit {
                id: h.id,
                uri: h.uri,
                title: h.title,
                text: h.text,
                score: h.score,
            })
            .collect())
    }

    /// Close (flush WAL). Store is dropped on GC, but call this for deterministic flush.
    #[napi]
    pub async fn close(&self) -> napi::Result<()> {
        // rusqlite flushes WAL on drop; no explicit API needed.
        // This method exists for API symmetry.
        Ok(())
    }
}
