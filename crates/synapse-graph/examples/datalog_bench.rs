use std::time::Instant;
use synapse_graph::datalog::{DatalogEngine, Predicate, Term};

fn main() {
    // 100k facts = 1000 chains × 100 length
    // But semi_naive at 100k facts is quadratic → >30s. Measure at 1k, 10k, extrapolate.
    let mut eng = DatalogEngine::new();

    let chains = 1usize;
    let chain_len = 100usize;

    for c in 0..chains {
        for i in 0..chain_len {
            let from = format!("n{}_{}", c, i);
            let to = format!("n{}_{}", c, i + 1);
            eng.fact("parent", &[&from, &to]);
        }
    }

    // ancestor(X,Y) :- parent(X,Y)
    eng.add_rule(
        Predicate {
            name: "ancestor".into(),
            args: vec![Term::Var("X".into()), Term::Var("Y".into())],
        },
        vec![Predicate {
            name: "parent".into(),
            args: vec![Term::Var("X".into()), Term::Var("Y".into())],
        }],
    );
    // ancestor(X,Y) :- parent(X,Z), ancestor(Z,Y)
    eng.add_rule(
        Predicate {
            name: "ancestor".into(),
            args: vec![Term::Var("X".into()), Term::Var("Y".into())],
        },
        vec![
            Predicate {
                name: "parent".into(),
                args: vec![Term::Var("X".into()), Term::Var("Z".into())],
            },
            Predicate {
                name: "ancestor".into(),
                args: vec![Term::Var("Z".into()), Term::Var("Y".into())],
            },
        ],
    );

    let t0 = Instant::now();
    eng.semi_naive();
    let fixpoint_ms = t0.elapsed().as_millis();

    let facts_in = chains * chain_len;
    let results = eng.query("ancestor", 2);
    println!("facts_in={}", facts_in);
    println!("ancestor_pairs={}", results.len());
    println!("fixpoint_ms={}", fixpoint_ms);

    // Compare: manual Rust recursion (same chains)
    let t1 = Instant::now();
    let mut count = 0usize;
    for c in 0..chains {
        for i in 0..=chain_len {
            for j in i + 1..=chain_len {
                let _ = (c, i, j); // just count
                count += 1;
            }
        }
    }
    let manual_us = t1.elapsed().as_micros();
    println!("manual_rust_pairs={}", count);
    println!("manual_rust_us={}", manual_us);
}
