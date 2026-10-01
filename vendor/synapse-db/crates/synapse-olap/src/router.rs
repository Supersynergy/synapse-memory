/// Auto-routing heuristic: classify SQL as OLTP or OLAP.
/// Available without the `olap` feature — zero extra deps.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Engine {
    /// Single-row / point-lookup / small-write → synapse-core SQLite path
    Oltp,
    /// Aggregations / GROUP BY / full-table-scan → DuckDB path
    Olap,
}

/// Heuristic: uppercase the SQL once and check for OLAP indicators.
///
/// Rules (in order of strength):
/// 1. Any aggregate function          → OLAP
/// 2. GROUP BY clause                 → OLAP
/// 3. HAVING clause                   → OLAP
/// 4. Window functions (OVER)         → OLAP
/// 5. DISTINCT on a SELECT            → OLAP
/// 6. Everything else                 → OLTP
pub fn is_olap(sql: &str) -> bool {
    let up = sql.to_uppercase();
    // aggregate functions
    let agg_fns = [
        "COUNT(",
        "SUM(",
        "AVG(",
        "MIN(",
        "MAX(",
        "STDDEV(",
        "VARIANCE(",
        "PERCENTILE_",
    ];
    if agg_fns.iter().any(|f| up.contains(f)) {
        return true;
    }
    if up.contains("GROUP BY") || up.contains("HAVING ") || up.contains(" OVER ") {
        return true;
    }
    // DISTINCT in a SELECT (not CREATE/INSERT DISTINCT)
    if up.contains("SELECT DISTINCT") {
        return true;
    }
    false
}

pub fn auto_route(sql: &str) -> Engine {
    if is_olap(sql) {
        Engine::Olap
    } else {
        Engine::Oltp
    }
}
