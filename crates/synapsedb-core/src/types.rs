use serde::{Deserialize, Serialize};

pub const EMBED_DIM: usize = 384;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Doc {
    pub id: i64,
    pub uri: Option<String>,
    pub title: Option<String>,
    pub text: String,
    pub meta: Option<serde_json::Value>,
    pub ts: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PutRequest {
    pub uri: Option<String>,
    pub title: Option<String>,
    pub text: String,
    pub meta: Option<serde_json::Value>,
    pub embedding: Option<Vec<f32>>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum SearchMode {
    Lex,
    Vec,
    Hybrid,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Hit {
    pub id: i64,
    pub uri: Option<String>,
    pub title: Option<String>,
    pub text: String,
    pub score: f64,
}
