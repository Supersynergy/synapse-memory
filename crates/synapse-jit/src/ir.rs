//! Intermediate representation for compiled queries.

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CmpOp {
    Gt,
    Gte,
    Lt,
    Lte,
    Eq,
    Neq,
}

#[derive(Debug, Clone)]
pub enum Expr {
    /// Column reference by index in schema
    Col(usize),
    /// Literal constant
    Const(i64),
    /// Binary arithmetic: left * scalar
    Mul(Box<Expr>, i64),
    /// Comparison
    Cmp(Box<Expr>, CmpOp, Box<Expr>),
}

/// GROUP BY SUM plan: group on col_group, sum col_agg
#[derive(Debug, Clone)]
pub struct GroupBySumPlan {
    pub col_group: usize,
    pub col_agg: usize,
}

/// HASH JOIN plan: join left.left_key == right.right_key, output left.*
#[derive(Debug, Clone)]
pub struct HashJoinPlan {
    pub left_key: usize,
    pub right_key: usize,
}

/// A compiled query plan: filter predicate + projection expressions.
#[derive(Debug, Clone)]
pub struct QueryPlan {
    /// If Some, only rows where predicate == true are included
    pub filter: Option<Expr>,
    /// Projected columns/expressions; empty = SELECT *
    pub projections: Vec<Expr>,
}

impl QueryPlan {
    /// SELECT * FROM t WHERE col_idx > literal
    pub fn filter_gt(col_idx: usize, literal: i64) -> Self {
        QueryPlan {
            filter: Some(Expr::Cmp(
                Box::new(Expr::Col(col_idx)),
                CmpOp::Gt,
                Box::new(Expr::Const(literal)),
            )),
            projections: vec![],
        }
    }

    /// SELECT col_idx * factor FROM t
    pub fn project_mul(col_idx: usize, factor: i64) -> Self {
        QueryPlan {
            filter: None,
            projections: vec![Expr::Mul(Box::new(Expr::Col(col_idx)), factor)],
        }
    }

    /// SELECT col_group, SUM(col_agg) FROM t GROUP BY col_group
    pub fn group_by_sum(col_group: usize, col_agg: usize) -> GroupBySumPlan {
        GroupBySumPlan { col_group, col_agg }
    }

    /// HASH JOIN left on left_key == right on right_key
    pub fn hash_join(left_key: usize, right_key: usize) -> HashJoinPlan {
        HashJoinPlan { left_key, right_key }
    }

    /// blake3 fingerprint for cache keying
    pub fn fingerprint(&self, schema_ncols: usize) -> u64 {
        let mut h = blake3::Hasher::new();
        h.update(&schema_ncols.to_le_bytes());
        let desc = format!("{:?}", self);
        h.update(desc.as_bytes());
        let bytes = h.finalize();
        u64::from_le_bytes(bytes.as_bytes()[..8].try_into().unwrap())
    }
}

/// Interpreter fallback — no cranelift, pure Rust.
/// Used when `jit` feature is off OR as correctness baseline.
pub fn interpret(plan: &QueryPlan, rows: &[crate::Row]) -> Vec<crate::Row> {
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        if let Some(pred) = &plan.filter {
            if eval_expr(pred, row) == 0 {
                continue;
            }
        }
        if plan.projections.is_empty() {
            out.push(row.clone());
        } else {
            let cols: Vec<i64> = plan.projections.iter().map(|e| eval_expr(e, row)).collect();
            out.push(crate::Row(cols));
        }
    }
    out
}

/// Interpreter: GROUP BY col_group, SUM(col_agg). Returns (key, sum) pairs sorted by key.
pub fn interpret_group_by_sum(plan: &GroupBySumPlan, rows: &[crate::Row]) -> Vec<(i64, i64)> {
    let mut map: std::collections::HashMap<i64, i64> = std::collections::HashMap::new();
    for row in rows {
        let key = row.0[plan.col_group];
        let val = row.0[plan.col_agg];
        *map.entry(key).or_insert(0) += val;
    }
    let mut out: Vec<(i64, i64)> = map.into_iter().collect();
    out.sort_unstable_by_key(|p| p.0);
    out
}

/// Interpreter: HASH JOIN left ⋈ right on left.left_key == right.right_key.
/// Returns rows from left that have a matching key in right.
pub fn interpret_hash_join(plan: &HashJoinPlan, left: &[crate::Row], right: &[crate::Row]) -> Vec<crate::Row> {
    let mut build: std::collections::HashSet<i64> = std::collections::HashSet::new();
    for row in right {
        build.insert(row.0[plan.right_key]);
    }
    left.iter().filter(|r| build.contains(&r.0[plan.left_key])).cloned().collect()
}

fn eval_expr(e: &Expr, row: &crate::Row) -> i64 {
    match e {
        Expr::Col(i) => row.0[*i],
        Expr::Const(v) => *v,
        Expr::Mul(inner, factor) => eval_expr(inner, row) * factor,
        Expr::Cmp(lhs, op, rhs) => {
            let l = eval_expr(lhs, row);
            let r = eval_expr(rhs, row);
            let b = match op {
                CmpOp::Gt  => l > r,
                CmpOp::Gte => l >= r,
                CmpOp::Lt  => l < r,
                CmpOp::Lte => l <= r,
                CmpOp::Eq  => l == r,
                CmpOp::Neq => l != r,
            };
            b as i64
        }
    }
}
