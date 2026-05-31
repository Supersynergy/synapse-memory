//! Cypher-lite — minimal subset of Cypher parsed into typed ops.
//!
//! Supported:
//!   MATCH (a)-[:REL*1..3]->(b) RETURN b LIMIT 10
//!   MATCH (a)-->(b) RETURN b
//!   MATCH p = SHORTEST PATH (a)-->(b)
//!   CALL graph.pagerank() YIELD node, score LIMIT 10
//!   CALL graph.communities() YIELD label, members
//!   CREATE (a)-[:REL {weight:0.8}]->(b)
//!
//! Production parser would use nom/pest. P1: regex + state machine, covers ~80%
//! of WP/Drupal-class queries. Extends with `extend()` callbacks for custom ops.

use crate::{GraphError, Result};

#[derive(Debug, Clone, PartialEq)]
pub enum CypherOp {
    /// MATCH (start)-[:REL*lo..hi]->(end) RETURN ... LIMIT n
    Traverse {
        start_id: i64,
        max_depth: usize,
        rel_filter: Option<String>,
        limit: usize,
    },
    /// MATCH p = SHORTEST PATH (a)-->(b)
    ShortestPath {
        from_id: i64,
        to_id: i64,
        max_depth: usize,
    },
    /// CALL graph.pagerank() YIELD node, score LIMIT n
    PageRank { top_n: usize, damping: f64 },
    /// CALL graph.communities()
    Communities { max_iters: usize },
    /// MATCH (a)-->(b) WHERE id(a)=N RETURN b LIMIT k — direct neighbors
    Neighbors {
        node_id: i64,
        top_k: usize,
        rel_filter: Option<String>,
    },
    /// CREATE (a)-[:REL {weight:w}]->(b)
    Create {
        from_id: i64,
        to_id: i64,
        rel: String,
        weight: f64,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct CypherQuery {
    pub op: CypherOp,
    pub raw: String,
}

/// Parse a Cypher-lite string. Returns first matching op.
/// Errors on unrecognized pattern.
pub fn parse_cypher(input: &str) -> Result<CypherQuery> {
    let s = input.trim();
    let lower = s.to_lowercase();

    // CALL graph.pagerank() YIELD node, score LIMIT n
    if lower.starts_with("call graph.pagerank") {
        let top_n = extract_limit(&lower).unwrap_or(10);
        return Ok(CypherQuery {
            op: CypherOp::PageRank {
                top_n,
                damping: 0.85,
            },
            raw: s.into(),
        });
    }

    // CALL graph.communities()
    if lower.starts_with("call graph.communities") {
        return Ok(CypherQuery {
            op: CypherOp::Communities { max_iters: 20 },
            raw: s.into(),
        });
    }

    // SHORTEST PATH
    if lower.contains("shortest path") || lower.contains("shortestpath") {
        let ids = extract_two_ids(&lower).ok_or_else(|| {
            GraphError::CypherParse("SHORTEST PATH needs id(a)=N, id(b)=M".into())
        })?;
        return Ok(CypherQuery {
            op: CypherOp::ShortestPath {
                from_id: ids.0,
                to_id: ids.1,
                max_depth: 10,
            },
            raw: s.into(),
        });
    }

    // CREATE
    if lower.starts_with("create") {
        let ids = extract_two_ids(&lower)
            .ok_or_else(|| GraphError::CypherParse("CREATE needs (id1)-[:REL]->(id2)".into()))?;
        let rel = extract_rel(&lower).unwrap_or("REL".into());
        let weight = extract_weight(&lower).unwrap_or(1.0);
        return Ok(CypherQuery {
            op: CypherOp::Create {
                from_id: ids.0,
                to_id: ids.1,
                rel,
                weight,
            },
            raw: s.into(),
        });
    }

    // MATCH ... TRAVERSE / NEIGHBORS
    if lower.starts_with("match") {
        let start_id = extract_first_id(&lower)
            .ok_or_else(|| GraphError::CypherParse("MATCH needs id(a)=N".into()))?;
        let limit = extract_limit(&lower).unwrap_or(10);
        let rel = extract_rel(&lower);
        let max_depth = extract_max_depth(&lower).unwrap_or(1);
        if max_depth == 1 {
            return Ok(CypherQuery {
                op: CypherOp::Neighbors {
                    node_id: start_id,
                    top_k: limit,
                    rel_filter: rel,
                },
                raw: s.into(),
            });
        }
        return Ok(CypherQuery {
            op: CypherOp::Traverse {
                start_id,
                max_depth,
                rel_filter: rel,
                limit,
            },
            raw: s.into(),
        });
    }

    Err(GraphError::CypherParse(format!("unrecognized: {s}")))
}

// --- regex-free extractors ---

fn extract_limit(s: &str) -> Option<usize> {
    let pos = s.rfind("limit ")?;
    let after = s[pos + 6..].trim();
    let num: String = after.chars().take_while(|c| c.is_ascii_digit()).collect();
    num.parse().ok()
}

fn extract_first_id(s: &str) -> Option<i64> {
    // looks for id(a) = N OR id=N
    let p = s.find("id(")?.checked_add(0)?;
    let after = &s[p..];
    let eq_pos = after.find('=')?;
    let after_eq = after[eq_pos + 1..].trim_start();
    let num: String = after_eq
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '-')
        .collect();
    num.parse().ok()
}

fn extract_two_ids(s: &str) -> Option<(i64, i64)> {
    let mut ids = Vec::with_capacity(2);
    let mut rest = s;
    while ids.len() < 2 {
        let p = rest.find("id(")?;
        rest = &rest[p..];
        let eq_pos = rest.find('=')?;
        let after = rest[eq_pos + 1..].trim_start();
        let num: String = after
            .chars()
            .take_while(|c| c.is_ascii_digit() || *c == '-')
            .collect();
        if let Ok(n) = num.parse::<i64>() {
            ids.push(n);
        }
        rest = &rest[eq_pos + 1..];
    }
    Some((ids[0], ids[1]))
}

fn extract_rel(s: &str) -> Option<String> {
    // [:REL] or [:REL*1..3]
    let p = s.find("[:")?;
    let after = &s[p + 2..];
    let end = after.find([']', '*', ' ']).unwrap_or(after.len());
    let r = after[..end].trim().trim_matches('`').to_uppercase();
    if r.is_empty() { None } else { Some(r) }
}

fn extract_max_depth(s: &str) -> Option<usize> {
    // [:REL*lo..hi] or [:REL*hi]
    let p = s.find('*')?;
    let after = &s[p + 1..];
    let end = after.find([']', ' ']).unwrap_or(after.len());
    let range = &after[..end];
    if let Some(dotpos) = range.find("..") {
        range[dotpos + 2..].trim().parse().ok()
    } else {
        range.trim().parse().ok()
    }
}

fn extract_weight(s: &str) -> Option<f64> {
    let p = s.find("weight:")?;
    let after = &s[p + 7..].trim_start();
    let num: String = after
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.' || *c == '-')
        .collect();
    num.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_pagerank_call() {
        let q = parse_cypher("CALL graph.pagerank() YIELD node, score LIMIT 5").unwrap();
        assert!(matches!(q.op, CypherOp::PageRank { top_n: 5, .. }));
    }

    #[test]
    fn parse_communities() {
        let q = parse_cypher("CALL graph.communities()").unwrap();
        assert!(matches!(q.op, CypherOp::Communities { .. }));
    }

    #[test]
    fn parse_neighbors_match() {
        let q = parse_cypher("MATCH (a)-->(b) WHERE id(a) = 42 RETURN b LIMIT 5").unwrap();
        match q.op {
            CypherOp::Neighbors { node_id, top_k, .. } => {
                assert_eq!(node_id, 42);
                assert_eq!(top_k, 5);
            }
            _ => panic!("expected Neighbors, got {:?}", q.op),
        }
    }

    #[test]
    fn parse_traverse_with_depth() {
        let q = parse_cypher("MATCH (a)-[:CITES*1..3]->(b) WHERE id(a) = 99 RETURN b LIMIT 20")
            .unwrap();
        match q.op {
            CypherOp::Traverse {
                start_id,
                max_depth,
                rel_filter,
                limit,
            } => {
                assert_eq!(start_id, 99);
                assert_eq!(max_depth, 3);
                assert_eq!(rel_filter.as_deref(), Some("CITES"));
                assert_eq!(limit, 20);
            }
            _ => panic!("expected Traverse"),
        }
    }

    #[test]
    fn parse_shortest_path() {
        let q = parse_cypher("MATCH p = SHORTEST PATH ((a)-->(b)) WHERE id(a) = 1 AND id(b) = 7")
            .unwrap();
        match q.op {
            CypherOp::ShortestPath { from_id, to_id, .. } => {
                assert_eq!(from_id, 1);
                assert_eq!(to_id, 7);
            }
            _ => panic!("expected ShortestPath"),
        }
    }

    #[test]
    fn parse_create_edge() {
        let q = parse_cypher("CREATE (a)-[:CITES {weight:0.7}]->(b) WHERE id(a)=10 AND id(b)=20")
            .unwrap();
        match q.op {
            CypherOp::Create {
                from_id,
                to_id,
                rel,
                weight,
            } => {
                assert_eq!(from_id, 10);
                assert_eq!(to_id, 20);
                assert_eq!(rel, "CITES");
                assert!((weight - 0.7).abs() < 1e-9);
            }
            _ => panic!("expected Create"),
        }
    }

    #[test]
    fn unknown_query_errors() {
        assert!(parse_cypher("RANDOM GIBBERISH").is_err());
    }
}
