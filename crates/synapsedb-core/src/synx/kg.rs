//! Temporal knowledge-graph edges — Graphiti-style memory evolution.
//!
//! Edges live in their own chunk kind (see `ChunkKind::MerkleNode` is taken;
//! we serialise edges under `SchemaDef` until a dedicated `KGEdge = 0x09`
//! reaches v0.3). For v0.2 we carry the JSON directly in a `TextBlob` tagged
//! `kg-edge`, which lets older readers ignore it gracefully.

use serde::{Deserialize, Serialize};

/// Memory scope — mem0 feature parity.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum Scope {
    /// Shared across every user and session.
    Global,
    /// Per end-user (tenant) memory.
    User(String),
    /// Per session/conversation memory.
    Session { user: String, session: String },
    /// Per project/workspace.
    Project(String),
}

impl Scope {
    pub fn as_tag(&self) -> String {
        match self {
            Scope::Global => "global".into(),
            Scope::User(u) => format!("user:{u}"),
            Scope::Session { user, session } => format!("session:{user}/{session}"),
            Scope::Project(p) => format!("project:{p}"),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum EdgeKind {
    /// `b` supersedes `a` (evolution — cognee-style).
    Supersedes,
    /// `a` references `b`.
    References,
    /// `a` contradicts `b`.
    Contradicts,
    /// `a` summarises `b`.
    Summarises,
    /// Custom label for ecosystem extension.
    Custom(String),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Edge {
    pub from: String,
    pub to: String,
    pub kind: EdgeKind,
    /// Unix seconds when the edge was observed.
    pub valid_from: i64,
    /// Unix seconds when the edge no longer holds (exclusive). 0 = open-ended.
    pub valid_to: i64,
    pub scope: Scope,
}

impl Edge {
    pub fn new(from: impl Into<String>, to: impl Into<String>, kind: EdgeKind) -> Self {
        Self {
            from: from.into(),
            to: to.into(),
            kind,
            valid_from: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0),
            valid_to: 0,
            scope: Scope::Global,
        }
    }

    pub fn with_scope(mut self, s: Scope) -> Self {
        self.scope = s;
        self
    }
}

/// Collection serialised as a single KG chunk.
#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct EdgeSet {
    pub edges: Vec<Edge>,
}

impl EdgeSet {
    pub fn to_json(&self) -> Vec<u8> {
        // safe: edges are all trivially serialisable
        serde_json::to_vec(self).unwrap_or_default()
    }

    pub fn from_json(bytes: &[u8]) -> crate::error::Result<Self> {
        serde_json::from_slice(bytes).map_err(Into::into)
    }

    /// Filter edges that are valid at the given unix timestamp.
    pub fn valid_at(&self, ts: i64) -> impl Iterator<Item = &Edge> {
        self.edges
            .iter()
            .filter(move |e| e.valid_from <= ts && (e.valid_to == 0 || ts < e.valid_to))
    }

    /// Resolve which doc currently supersedes `id` (transitively).
    pub fn resolve_current(&self, id: &str, now: i64) -> String {
        let mut cur = id.to_string();
        let mut seen = std::collections::HashSet::new();
        seen.insert(cur.clone());
        loop {
            let next = self.valid_at(now).find_map(|e| {
                if matches!(e.kind, EdgeKind::Supersedes) && e.from == cur {
                    Some(e.to.clone())
                } else {
                    None
                }
            });
            match next {
                Some(n) if !seen.contains(&n) => {
                    seen.insert(n.clone());
                    cur = n;
                }
                _ => break,
            }
        }
        cur
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scope_tag_roundtrip() {
        let s = Scope::Session {
            user: "maxim".into(),
            session: "2026-04-20".into(),
        };
        assert_eq!(s.as_tag(), "session:maxim/2026-04-20");
    }

    #[test]
    fn supersedes_chain_resolved() {
        let e1 = Edge::new("v1", "v2", EdgeKind::Supersedes);
        let e2 = Edge::new("v2", "v3", EdgeKind::Supersedes);
        let set = EdgeSet {
            edges: vec![e1, e2],
        };
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64
            + 10;
        assert_eq!(set.resolve_current("v1", now), "v3");
    }

    #[test]
    fn valid_at_filters_temporal() {
        let mut e = Edge::new("a", "b", EdgeKind::References);
        e.valid_from = 100;
        e.valid_to = 200;
        let set = EdgeSet { edges: vec![e] };
        assert_eq!(set.valid_at(150).count(), 1);
        assert_eq!(set.valid_at(250).count(), 0);
    }

    #[test]
    fn json_roundtrip() {
        let set = EdgeSet {
            edges: vec![Edge::new("x", "y", EdgeKind::Summarises)
                .with_scope(Scope::Project("supersynergy".into()))],
        };
        let b = set.to_json();
        let set2 = EdgeSet::from_json(&b).unwrap();
        assert_eq!(set2.edges.len(), 1);
        assert!(matches!(set2.edges[0].scope, Scope::Project(_)));
    }
}
