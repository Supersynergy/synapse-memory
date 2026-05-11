use serde::{Deserialize, Serialize};
use serde_json::Value;

#[cfg(feature = "embed-1024")]
pub const EMBED_DIM: usize = 1024;
#[cfg(all(feature = "embed-768", not(feature = "embed-1024")))]
pub const EMBED_DIM: usize = 768;
#[cfg(not(any(feature = "embed-768", feature = "embed-1024")))]
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

/// Comparison operator for metadata predicate.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum PredicateOp {
    Eq,
    Ne,
    In,
}

/// Simple metadata filter: `meta->>key op value`.
/// `value` must be a JSON scalar (string / number / bool / null).
/// For `In`, `value` must be a JSON array.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetadataPredicate {
    pub key: String,
    pub op: PredicateOp,
    pub value: Value,
}

impl MetadataPredicate {
    /// Evaluate the predicate against a parsed `meta` JSON object.
    /// Returns `false` if `meta` is None or the key is absent.
    pub fn matches(&self, meta: Option<&Value>) -> bool {
        let obj = match meta {
            Some(v) => v,
            None => return false,
        };
        let field = match obj.get(&self.key) {
            Some(f) => f,
            None => return false,
        };
        match self.op {
            PredicateOp::Eq => field == &self.value,
            PredicateOp::Ne => field != &self.value,
            PredicateOp::In => {
                if let Value::Array(arr) = &self.value {
                    arr.iter().any(|v| v == field)
                } else {
                    false
                }
            }
        }
    }

    /// Estimate filter selectivity (fraction of docs expected to pass).
    /// Used to compute ef-boost multiplier.
    /// Without per-key stats we use a conservative default per op:
    /// Eq → 0.5, Ne → 0.9, In(n) → min(n*0.2, 0.9).
    pub fn estimated_selectivity(&self) -> f64 {
        match self.op {
            PredicateOp::Eq => 0.5,
            PredicateOp::Ne => 0.9,
            PredicateOp::In => {
                if let Value::Array(arr) = &self.value {
                    (arr.len() as f64 * 0.2).min(0.9)
                } else {
                    0.5
                }
            }
        }
    }
}

/// Options for filtered vector search.
#[derive(Debug, Clone, Default)]
pub struct SearchOptions {
    /// If set, only return docs matching the predicate.
    pub filter: Option<MetadataPredicate>,
    /// ef-boost multiplier override (default: auto from selectivity).
    /// Range 1..=32.
    pub ef_multiplier: Option<usize>,
    /// Conformal recall target in (0, 1]. When set and a `ConformalCalibrator` is provided,
    /// triggers exact-rerank fallback if predicted recall lower bound < target.
    /// Feature `conformal` must be enabled.
    #[cfg(feature = "conformal")]
    pub conformal_target: Option<f32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Hit {
    pub id: i64,
    pub uri: Option<String>,
    pub title: Option<String>,
    pub text: String,
    pub score: f64,
}
