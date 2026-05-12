//! Index Advisor — analyze query log → recommend missing indexes.
//!
//! Heuristic v1: regex extracts WHERE/ORDER BY columns from query log.
//! Frequency × selectivity → ranked candidates.
//!
//! P3: CatBoost ranking model trained on (query, plan, gain).

use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct IndexCandidate {
    pub table: String,
    pub columns: Vec<String>,
    pub kind: IndexKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum IndexKind {
    BTree,
    Covering,
    Functional,
    Partial,
}

#[derive(Debug, Clone)]
pub struct Recommendation {
    pub candidate: IndexCandidate,
    pub score: f64,
    pub seen_count: u64,
    pub create_sql: String,
}

pub struct IndexAdvisor {
    seen: HashMap<IndexCandidate, u64>,
}

impl IndexAdvisor {
    pub fn new() -> Self {
        Self {
            seen: HashMap::new(),
        }
    }

    /// Record observed query SQL. Extract candidates via regex.
    pub fn observe(&mut self, sql: &str) {
        let lower = sql.to_lowercase();
        // VERY simple WHERE column extractor — production: AST parser.
        if let Some(table) = extract_from_table(&lower) {
            if let Some(cols) = extract_where_cols(&lower) {
                let cand = IndexCandidate {
                    table: table.clone(),
                    columns: cols,
                    kind: IndexKind::BTree,
                };
                *self.seen.entry(cand).or_insert(0) += 1;
            }
        }
    }

    /// Top-N recommendations by frequency.
    pub fn top_n(&self, n: usize) -> Vec<Recommendation> {
        let mut v: Vec<_> = self.seen.iter().collect();
        v.sort_by_key(|(_, count)| std::cmp::Reverse(**count));
        v.into_iter()
            .take(n)
            .map(|(cand, count)| {
                let score = (*count as f64).ln_1p();
                let create_sql = format!(
                    "CREATE INDEX idx_{}_{} ON {} ({});",
                    cand.table,
                    cand.columns.join("_"),
                    cand.table,
                    cand.columns.join(", "),
                );
                Recommendation {
                    candidate: cand.clone(),
                    score,
                    seen_count: *count,
                    create_sql,
                }
            })
            .collect()
    }

    pub fn len(&self) -> usize {
        self.seen.len()
    }
}

impl Default for IndexAdvisor {
    fn default() -> Self {
        Self::new()
    }
}

fn extract_from_table(sql: &str) -> Option<String> {
    let from_pos = sql.find(" from ")?;
    let after = &sql[from_pos + 6..];
    let end = after
        .find(|c: char| c.is_whitespace() || c == ',' || c == ';')
        .unwrap_or(after.len());
    let table = after[..end].trim_matches('`').to_string();
    if table.is_empty() {
        None
    } else {
        Some(table)
    }
}

fn extract_where_cols(sql: &str) -> Option<Vec<String>> {
    let where_pos = sql.find(" where ")?;
    let after = &sql[where_pos + 7..];
    let mut cols = Vec::new();
    // Match pattern: identifier (= | > | < | LIKE | IN)
    for token in after
        .split(|c: char| !c.is_alphanumeric() && c != '_')
        .filter(|s| !s.is_empty())
    {
        // Skip keywords
        if matches!(
            token,
            "and" | "or" | "not" | "in" | "is" | "null" | "like" | "between"
        ) {
            continue;
        }
        if token
            .chars()
            .next()
            .map(|c| c.is_alphabetic())
            .unwrap_or(false)
            && !token.chars().all(|c| c.is_ascii_digit())
            && token.len() > 2
        {
            cols.push(token.to_string());
            if cols.len() >= 3 {
                break;
            }
        }
    }
    if cols.is_empty() {
        None
    } else {
        Some(cols)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_simple_index_candidate() {
        let mut a = IndexAdvisor::new();
        a.observe("SELECT * FROM wp_posts WHERE post_status = 'publish'");
        let top = a.top_n(5);
        assert_eq!(top.len(), 1);
        assert_eq!(top[0].candidate.table, "wp_posts");
        assert!(top[0]
            .candidate
            .columns
            .contains(&"post_status".to_string()));
        assert!(top[0].create_sql.contains("CREATE INDEX"));
    }

    #[test]
    fn ranks_by_frequency() {
        let mut a = IndexAdvisor::new();
        for _ in 0..10 {
            a.observe("SELECT * FROM wp_posts WHERE post_status = 'x'");
        }
        a.observe("SELECT * FROM wp_users WHERE user_login = 'y'");
        let top = a.top_n(2);
        assert_eq!(top[0].seen_count, 10);
        assert_eq!(top[1].seen_count, 1);
    }

    #[test]
    fn handles_no_where() {
        let mut a = IndexAdvisor::new();
        a.observe("SELECT 1");
        assert_eq!(a.len(), 0);
    }
}
