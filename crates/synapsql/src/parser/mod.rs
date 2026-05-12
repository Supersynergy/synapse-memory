//! SQL parser layer: fingerprinting, rewriting, caching, transactions.
//!
//! Patterns stolen from:
//! - ProxySQL: statement fingerprinting (normalize literals → `?`)
//! - Vitess vtgate: query-plan cache keyed by fingerprint
//! - MyDuck: read/write split on AST node type
//! - maxpert/marmot: MVCC snapshot isolation (BEGIN READ ONLY / AS OF TIMESTAMP)
//! - Apache Hive + TanStack/db: predicate pushdown optimization

pub mod rewriter;
pub mod cache;
pub mod fingerprint;
pub mod transactions;

pub use rewriter::{rewrite, RewriteResult};
pub use cache::{QueryCache, PlanCache};
pub use fingerprint::fingerprint;
pub use transactions::{classify_txn, is_txn_statement, TxnStatement, IsolationLevel};
