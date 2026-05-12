//! SQL parser layer: fingerprinting, rewriting, caching.
//!
//! Patterns stolen from:
//! - ProxySQL: statement fingerprinting (normalize literals → `?`)
//! - Vitess vtgate: query-plan cache keyed by fingerprint
//! - MyDuck: read/write split on AST node type

pub mod rewriter;
pub mod cache;
pub mod fingerprint;

pub use rewriter::{rewrite, RewriteResult};
pub use cache::QueryCache;
pub use fingerprint::fingerprint;
