/// Aggregation operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AggOp {
    Count,
    Sum,
    Avg,
    Min,
    Max,
}
