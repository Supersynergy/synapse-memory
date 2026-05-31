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

/// Comparison operator for metadata predicate (kept for backward-compat flat construction).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum PredicateOp {
    Eq,
    Ne,
    In,
}

/// Compound metadata filter supporting Eq/Ne/Lt/Gt/Lte/Gte/In and AND/OR/NOT.
///
/// Backward-compat: callers using the old flat struct can migrate to `MetadataPredicate::Eq`,
/// `MetadataPredicate::Ne`, or `MetadataPredicate::In` variants directly.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MetadataPredicate {
    /// key == value
    Eq { key: String, value: Value },
    /// key != value
    Ne { key: String, value: Value },
    /// key < numeric threshold
    Lt { key: String, value: f64 },
    /// key > numeric threshold
    Gt { key: String, value: f64 },
    /// key <= numeric threshold
    Lte { key: String, value: f64 },
    /// key >= numeric threshold
    Gte { key: String, value: f64 },
    /// key is one of values
    In { key: String, values: Vec<Value> },
    /// all sub-predicates must match
    And(Vec<MetadataPredicate>),
    /// at least one sub-predicate must match
    Or(Vec<MetadataPredicate>),
    /// sub-predicate must not match
    Not(Box<MetadataPredicate>),
}

impl MetadataPredicate {
    /// Evaluate the predicate against a parsed `meta` JSON object.
    pub fn matches(&self, meta: Option<&Value>) -> bool {
        match self {
            MetadataPredicate::And(preds) => preds.iter().all(|p| p.matches(meta)),
            MetadataPredicate::Or(preds) => preds.iter().any(|p| p.matches(meta)),
            MetadataPredicate::Not(pred) => !pred.matches(meta),
            _ => {
                let obj = match meta {
                    Some(v) => v,
                    None => return false,
                };
                self.matches_field(obj)
            }
        }
    }

    fn matches_field(&self, obj: &Value) -> bool {
        match self {
            MetadataPredicate::Eq { key, value } => obj.get(key) == Some(value),
            MetadataPredicate::Ne { key, value } => obj.get(key).is_some_and(|f| f != value),
            MetadataPredicate::Lt { key, value } => obj
                .get(key)
                .and_then(|f| f.as_f64())
                .is_some_and(|n| n < *value),
            MetadataPredicate::Gt { key, value } => obj
                .get(key)
                .and_then(|f| f.as_f64())
                .is_some_and(|n| n > *value),
            MetadataPredicate::Lte { key, value } => obj
                .get(key)
                .and_then(|f| f.as_f64())
                .is_some_and(|n| n <= *value),
            MetadataPredicate::Gte { key, value } => obj
                .get(key)
                .and_then(|f| f.as_f64())
                .is_some_and(|n| n >= *value),
            MetadataPredicate::In { key, values } => {
                obj.get(key).is_some_and(|f| values.iter().any(|v| v == f))
            }
            MetadataPredicate::And(_) | MetadataPredicate::Or(_) | MetadataPredicate::Not(_) => {
                unreachable!("compound handled in matches()")
            }
        }
    }

    /// Estimate filter selectivity (fraction of docs expected to pass).
    /// Compound predicates multiply sub-selectivities (product rule).
    pub fn estimated_selectivity(&self) -> f64 {
        match self {
            MetadataPredicate::Eq { .. } => 0.5,
            MetadataPredicate::Ne { .. } => 0.9,
            MetadataPredicate::Lt { .. } | MetadataPredicate::Gt { .. } => 0.3,
            MetadataPredicate::Lte { .. } | MetadataPredicate::Gte { .. } => 0.35,
            MetadataPredicate::In { values, .. } => (values.len() as f64 * 0.2).min(0.9),
            MetadataPredicate::And(preds) => preds
                .iter()
                .map(|p| p.estimated_selectivity())
                .product::<f64>()
                .max(0.01),
            MetadataPredicate::Or(preds) => {
                // P(A∪B) ≈ 1 - ∏(1 - sᵢ)
                let miss: f64 = preds
                    .iter()
                    .map(|p| 1.0 - p.estimated_selectivity())
                    .product();
                (1.0 - miss).min(0.99)
            }
            MetadataPredicate::Not(pred) => (1.0 - pred.estimated_selectivity()).max(0.01),
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meta: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ts: Option<i64>,
}
