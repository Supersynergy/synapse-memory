//! synapse-ops — backup + slow query log + observability.
//!
//! Closes Tier-6 gap from HONEST-GAP-ANALYSIS.md (no observability today).

pub mod backup;
pub mod slowlog;

pub use backup::{Backup, BackupTarget};
pub use slowlog::{SlowQueryLog, SlowEntry};
