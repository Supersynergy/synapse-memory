//! synapse-wal — write-ahead log for durable segment writes.
//!
//! Status: scaffolding (PR-0). See `docs/SCALE_100M_PLAN_2026-04-23.md` §2.4.
//!
//! Target (PR-E1): UC19 crash recovery under 60s at 100M.

#![allow(dead_code)]

#[derive(Debug, thiserror::Error)]
pub enum WalError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("corrupt record at offset {offset}: {reason}")]
    Corrupt { offset: u64, reason: &'static str },
}

// TODO(PR-E1): pub struct Wal { ... } with append / replay / truncate_to
// TODO(PR-E2): Db::open_with_recovery() composed over Wal + segment store
