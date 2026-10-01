//! Cranelift-JIT query compiler for Synapse.
//!
//! Scope: simple WHERE-filter + projection (no JOIN/GROUP BY).
//! Feature-gated `jit` — default off (cranelift ~5 MB).
//!
//! # Usage (feature = "jit")
//! ```ignore
//! let mut engine = JitEngine::new()?;
//! let schema = Schema { columns: vec!["a".into(), "b".into()] };
//! let query = QueryPlan::filter("a", CmpOp::Gt, Value::I64(5));
//! let func_id = engine.compile(&query, &schema)?;
//! let results = engine.execute(func_id, &rows)?;
//! ```

pub mod ir;
pub mod schema;

pub use ir::{CmpOp, Expr, GroupBySumPlan, HashJoinPlan, QueryPlan};
pub use schema::Schema;

#[cfg(feature = "jit")]
pub mod jit;
#[cfg(feature = "jit")]
pub use jit::{GroupByJitEngine, HashJoinJitEngine, JitEngine};

mod tests;

// ── Value type shared by IR + runtime ──────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    I64(i64),
    F64(f64),
    Bool(bool),
    Null,
}

impl Value {
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Value::I64(v) => Some(*v),
            _ => None,
        }
    }
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Value::F64(v) => Some(*v),
            Value::I64(v) => Some(*v as f64),
            _ => None,
        }
    }
}

// ── Row: one record, fixed-width i64 columns ───────────────────────────────

#[derive(Debug, Clone)]
pub struct Row(pub Vec<i64>);
