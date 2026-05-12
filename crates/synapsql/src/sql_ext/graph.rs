//! Graph-CTE — recursive `WITH GRAPH_TRAVERSE(...)` sugar.
//!
//! Expands a compact graph-traverse macro into a standard recursive CTE
//! that SQLite and libSQL both understand.
//!
//! # Syntax
//! ```sql
//! WITH GRAPH_TRAVERSE(start=42, edge_table=links, max_depth=3)
//! SELECT * FROM traverse_result;
//! ```
//!
//! Expands to:
//! ```sql
//! WITH RECURSIVE traverse_result(id, depth) AS (
//!     SELECT 42, 0
//!     UNION ALL
//!     SELECT links.target, t.depth + 1
//!     FROM links
//!     JOIN traverse_result t ON links.source = t.id
//!     WHERE t.depth < 3
//! )
//! SELECT * FROM traverse_result;
//! ```

#[derive(Debug, Clone)]
pub struct GraphTraverseArgs {
    pub start: u64,
    pub edge_table: String,
    pub max_depth: u32,
}

/// Expand `WITH GRAPH_TRAVERSE(...)` macro into a standard recursive CTE.
/// Returns `None` if no macro is present.
pub fn expand_graph_cte(sql: &str) -> Option<String> {
    let upper = sql.to_ascii_uppercase();
    let macro_start = upper.find("WITH GRAPH_TRAVERSE(")?;
    let args_start = macro_start + "WITH GRAPH_TRAVERSE(".len();
    let args_end = sql[args_start..].find(')')? + args_start;
    let args_str = &sql[args_start..args_end];

    let args = parse_graph_args(args_str)?;
    let rest = sql[args_end + 1..].trim_start_matches(',').trim();

    let cte = format!(
        "WITH RECURSIVE traverse_result(id, depth) AS (\n    \
         SELECT {start}, 0\n    \
         UNION ALL\n    \
         SELECT {et}.target, t.depth + 1\n    \
         FROM {et}\n    \
         JOIN traverse_result t ON {et}.source = t.id\n    \
         WHERE t.depth < {depth}\n\
         )\n{rest}",
        start = args.start,
        et    = args.edge_table,
        depth = args.max_depth,
        rest  = rest,
    );
    Some(cte)
}

fn parse_graph_args(s: &str) -> Option<GraphTraverseArgs> {
    let mut start = None;
    let mut edge_table = None;
    let mut max_depth = 3u32;

    for part in s.split(',') {
        let kv: Vec<&str> = part.splitn(2, '=').collect();
        if kv.len() != 2 { continue; }
        match kv[0].trim().to_ascii_lowercase().as_str() {
            "start"      => start      = kv[1].trim().parse().ok(),
            "edge_table" => edge_table = Some(kv[1].trim().to_owned()),
            "max_depth"  => max_depth  = kv[1].trim().parse().unwrap_or(3),
            _ => {}
        }
    }

    Some(GraphTraverseArgs {
        start:      start?,
        edge_table: edge_table?,
        max_depth,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expands_graph_macro() {
        let sql = "WITH GRAPH_TRAVERSE(start=1, edge_table=links, max_depth=2)\nSELECT * FROM traverse_result";
        let expanded = expand_graph_cte(sql).unwrap();
        assert!(expanded.contains("WITH RECURSIVE traverse_result"));
        assert!(expanded.contains("WHERE t.depth < 2"));
        assert!(expanded.contains("FROM links"));
    }

    #[test]
    fn passthrough_when_absent() {
        assert!(expand_graph_cte("SELECT * FROM docs").is_none());
    }
}
