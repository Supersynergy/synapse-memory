use synapse_mlx_olap::{AggOp, Column, MlxOlapEngine, RecordBatch};

fn simple_batch() -> RecordBatch {
    RecordBatch::new(
        vec!["grp".to_string(), "val".to_string()],
        vec![
            Column::Str(vec![
                "a".into(),
                "b".into(),
                "a".into(),
                "b".into(),
                "a".into(),
            ]),
            Column::Float(vec![1.0, 2.0, 3.0, 4.0, 5.0]),
        ],
    )
    .unwrap()
}

#[test]
fn test_sum_scalar() {
    let engine = MlxOlapEngine::cpu().unwrap();
    let batch = simple_batch();
    let result = engine.execute_agg(&batch, AggOp::Sum, "val", None).unwrap();
    let col = result.columns[0].as_floats().unwrap();
    assert!((col[0] - 15.0).abs() < 1e-9);
}

#[test]
fn test_avg_scalar() {
    let engine = MlxOlapEngine::cpu().unwrap();
    let batch = simple_batch();
    let result = engine.execute_agg(&batch, AggOp::Avg, "val", None).unwrap();
    let col = result.columns[0].as_floats().unwrap();
    assert!((col[0] - 3.0).abs() < 1e-9);
}

#[test]
fn test_count_scalar() {
    let engine = MlxOlapEngine::cpu().unwrap();
    let batch = simple_batch();
    let result = engine
        .execute_agg(&batch, AggOp::Count, "val", None)
        .unwrap();
    assert_eq!(result.columns[0].as_floats().unwrap()[0], 5.0);
}

#[test]
fn test_min_max() {
    let engine = MlxOlapEngine::cpu().unwrap();
    let batch = simple_batch();
    let min_r = engine.execute_agg(&batch, AggOp::Min, "val", None).unwrap();
    let max_r = engine.execute_agg(&batch, AggOp::Max, "val", None).unwrap();
    assert!((min_r.columns[0].as_floats().unwrap()[0] - 1.0).abs() < 1e-9);
    assert!((max_r.columns[0].as_floats().unwrap()[0] - 5.0).abs() < 1e-9);
}

#[test]
fn test_group_by_sum() {
    let engine = MlxOlapEngine::cpu().unwrap();
    let batch = simple_batch();
    let result = engine
        .execute_agg(&batch, AggOp::Sum, "val", Some("grp"))
        .unwrap();
    // sorted keys: a, b
    let keys = result.columns[0].as_strs().unwrap();
    let vals = result.columns[1].as_floats().unwrap();
    assert_eq!(keys, &["a", "b"]);
    assert!((vals[0] - 9.0).abs() < 1e-9); // 1+3+5
    assert!((vals[1] - 6.0).abs() < 1e-9); // 2+4
}

#[test]
fn test_group_by_avg() {
    let engine = MlxOlapEngine::cpu().unwrap();
    let batch = simple_batch();
    let result = engine
        .execute_agg(&batch, AggOp::Avg, "val", Some("grp"))
        .unwrap();
    let vals = result.columns[1].as_floats().unwrap();
    assert!((vals[0] - 3.0).abs() < 1e-9); // (1+3+5)/3
    assert!((vals[1] - 3.0).abs() < 1e-9); // (2+4)/2
}

#[test]
fn test_group_by_count() {
    let engine = MlxOlapEngine::cpu().unwrap();
    let batch = simple_batch();
    let result = engine
        .execute_agg(&batch, AggOp::Count, "val", Some("grp"))
        .unwrap();
    let vals = result.columns[1].as_floats().unwrap();
    assert_eq!(vals[0], 3.0); // a: 3 rows
    assert_eq!(vals[1], 2.0); // b: 2 rows
}

#[test]
fn test_int_column() {
    let engine = MlxOlapEngine::cpu().unwrap();
    let batch =
        RecordBatch::new(vec!["val".to_string()], vec![Column::Int(vec![10, 20, 30])]).unwrap();
    let result = engine.execute_agg(&batch, AggOp::Sum, "val", None).unwrap();
    assert!((result.columns[0].as_floats().unwrap()[0] - 60.0).abs() < 1e-9);
}
