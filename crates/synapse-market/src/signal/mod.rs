pub mod embed;
pub mod similar;

use serde::{Deserialize, Serialize};

pub type SignalId = u64;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Signal {
    pub id: SignalId,
    pub ticker: String,
    pub pattern: String,
    pub ts: i64,
    pub vec: Vec<f32>,
}
