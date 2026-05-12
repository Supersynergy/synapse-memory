#[cfg(all(test, feature = "jit"))]
mod jit_tests {
    use crate::{GroupByJitEngine, HashJoinJitEngine, JitEngine, QueryPlan, Row, Schema};

    fn schema_1col() -> Schema {
        Schema { columns: vec!["val".into()] }
    }

    /// Smoke 1: compile + execute WHERE val > 5
    #[test]
    fn test_filter_gt() {
        let mut engine = JitEngine::new().expect("JitEngine::new");
        let schema = schema_1col();
        let plan = QueryPlan::filter_gt(0, 5);
        let func = engine.compile(&plan, &schema).expect("compile");

        let rows: Vec<Row> = (0..10).map(|i| Row(vec![i])).collect();
        let result = engine.execute(func, &plan, &rows).expect("execute");

        // rows with val > 5 → [6,7,8,9]
        assert_eq!(result.len(), 4);
        assert_eq!(result[0].0[0], 6);
        assert_eq!(result[3].0[0], 9);
    }

    /// Smoke 2: compile + execute SELECT val * 2 FROM t
    #[test]
    fn test_project_mul() {
        let mut engine = JitEngine::new().expect("JitEngine::new");
        let schema = schema_1col();
        let plan = QueryPlan::project_mul(0, 2);
        let func = engine.compile(&plan, &schema).expect("compile");

        let rows: Vec<Row> = (1..=5).map(|i| Row(vec![i])).collect();
        let result = engine.execute(func, &plan, &rows).expect("execute");

        assert_eq!(result.len(), 5);
        let vals: Vec<i64> = result.iter().map(|r| r.0[0]).collect();
        assert_eq!(vals, vec![2, 4, 6, 8, 10]);
    }

    /// GROUP BY SUM: 100 rows, 5 keys
    #[test]
    fn test_group_by_sum() {
        let mut engine = GroupByJitEngine::new().expect("GroupByJitEngine::new");
        let plan = QueryPlan::group_by_sum(0, 1);
        let func = engine.compile(&plan).expect("compile gb");

        // 20 rows: key=i%5, val=1 → each key sums to 20
        let rows: Vec<Row> = (0..100i64).map(|i| Row(vec![i % 5, 1])).collect();
        let mut result = engine.execute(func, &plan, &rows).expect("execute gb");
        result.sort_unstable_by_key(|p| p.0);

        assert_eq!(result.len(), 5);
        for (k, s) in &result {
            assert_eq!(*s, 20, "key {k} sum should be 20");
        }
    }

    /// HASH JOIN: left 0..20, right 0..10 step 2 → matches even keys 0,2,4..18
    #[test]
    fn test_hash_join() {
        let mut engine = HashJoinJitEngine::new().expect("HashJoinJitEngine::new");
        let plan = QueryPlan::hash_join(0, 0);
        let func = engine.compile(&plan).expect("compile hj");

        let left: Vec<Row> = (0..20i64).map(|i| Row(vec![i, i * 10])).collect();
        let right: Vec<Row> = (0..10i64).map(|i| Row(vec![i * 2])).collect();
        let mut result = engine.execute(func, &plan, &left, &right).expect("execute hj");
        result.sort_unstable_by_key(|r| r.0[0]);

        assert_eq!(result.len(), 10);
        for (i, r) in result.iter().enumerate() {
            assert_eq!(r.0[0], (i as i64) * 2);
        }
    }

    /// Cache: second compile with same plan returns same FuncId
    #[test]
    fn test_compile_cache() {
        let mut engine = JitEngine::new().expect("JitEngine::new");
        let schema = schema_1col();
        let plan = QueryPlan::filter_gt(0, 3);
        let id1 = engine.compile(&plan, &schema).expect("compile 1");
        let id2 = engine.compile(&plan, &schema).expect("compile 2");
        assert_eq!(id1, id2);
    }
}

#[cfg(test)]
mod interp_tests {
    use crate::{ir::interpret, QueryPlan, Row};

    #[test]
    fn interpreter_filter_gt() {
        let plan = QueryPlan::filter_gt(0, 5);
        let rows: Vec<Row> = (0..10).map(|i| Row(vec![i])).collect();
        let result = interpret(&plan, &rows);
        assert_eq!(result.len(), 4);
    }

    #[test]
    fn interpreter_group_by_sum() {
        use crate::ir::interpret_group_by_sum;
        let plan = crate::QueryPlan::group_by_sum(0, 1);
        let rows: Vec<crate::Row> = (0..10i64).map(|i| crate::Row(vec![i % 3, 1])).collect();
        let result = interpret_group_by_sum(&plan, &rows);
        assert_eq!(result.len(), 3);
        // keys 0,1,2; key0=rows 0,3,6,9=4; key1=rows1,4,7=3; key2=rows2,5,8=3
        let m: std::collections::HashMap<_, _> = result.into_iter().collect();
        assert_eq!(m[&0], 4);
        assert_eq!(m[&1], 3);
        assert_eq!(m[&2], 3);
    }

    #[test]
    fn interpreter_hash_join() {
        use crate::ir::interpret_hash_join;
        let plan = crate::QueryPlan::hash_join(0, 0);
        let left: Vec<crate::Row> = (0..5i64).map(|i| crate::Row(vec![i])).collect();
        let right: Vec<crate::Row> = vec![crate::Row(vec![1]), crate::Row(vec![3])];
        let result = interpret_hash_join(&plan, &left, &right);
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn interpreter_project_mul() {
        let plan = QueryPlan::project_mul(0, 3);
        let rows: Vec<Row> = (1..=3).map(|i| Row(vec![i])).collect();
        let result = interpret(&plan, &rows);
        assert_eq!(result[0].0[0], 3);
        assert_eq!(result[1].0[0], 6);
        assert_eq!(result[2].0[0], 9);
    }
}
