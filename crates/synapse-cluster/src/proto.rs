//! Wire protocol messages for cluster gossip.

use serde::{Deserialize, Serialize};
use synapse_core::sync::{Op, OpId};

use crate::{Clock, NodeId};

/// Request messages sent over TCP.
#[derive(Debug, Serialize, Deserialize)]
pub enum Request {
    /// Handshake: identify caller node.
    Hello { from: NodeId },
    /// Ask the remote to return all ops with clock > `since`.
    PullChanges { since: Clock },
    /// Push local ops to the remote.
    PushChanges { ops: Vec<(OpId, Op)> },
}

/// Response messages.
#[derive(Debug, Serialize, Deserialize)]
pub enum Response {
    /// Acknowledgement (for Hello, PushChanges).
    Ok,
    /// Delta ops since the requested clock.
    Changes { ops: Vec<(OpId, Op)> },
    /// Protocol error.
    Err { msg: String },
}
