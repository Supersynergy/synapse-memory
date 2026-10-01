//! SynapsQL — unified SQL facade.
//!
//! MySQL wire :3306 · Postgres wire :5432 · HTTP/gRPC :9477
//! Vector `<=>` · HYBRID_RANK · WITH RECALL_GUARANTEE · AS OF · Graph-CTE
//!
//! ## Killer features (v2)
//! - ProxySQL-style statement fingerprinting
//! - Blake3-keyed LRU result cache (conformal write-epoch invalidation)
//! - Per-connection prepared-statement cache (Vitess pattern, max 1000/conn)
//! - Read/write query classifier (MyDuck pattern)
//! - Full introspection intercept (SELECT 1, VERSION(), SHOW VARIABLES)
//! - Global QPS counter (target ≥10k single core)

pub mod parser;
pub mod pool;
pub mod server;
pub mod sql_ext;

pub use server::Service;
