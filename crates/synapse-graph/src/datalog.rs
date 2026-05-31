//! Datalog-style query engine for synapse-graph.
//!
//! Feature-gated: `graph-datalog`.
//!
//! Syntax (S-expression): `(predicate arg1 arg2 ...)`
//! Variables start with `?` e.g. `?x`. Constants are anything else.
//!
//! Example:
//! ```
//! # #[cfg(feature = "graph-datalog")]
//! # {
//! use synapse_graph::datalog::{DatalogEngine, Predicate, Term, Op};
//! let mut eng = DatalogEngine::new();
//! eng.fact("parent", &["alice", "bob"]);
//! eng.fact("parent", &["bob", "carol"]);
//! // ancestor(X,Y) :- parent(X,Y)
//! eng.add_rule(
//!     Predicate { name: "ancestor".into(), args: vec![Term::Var("X".into()), Term::Var("Y".into())] },
//!     vec![Predicate { name: "parent".into(), args: vec![Term::Var("X".into()), Term::Var("Y".into())] }],
//! );
//! // ancestor(X,Y) :- parent(X,Z), ancestor(Z,Y)
//! eng.add_rule(
//!     Predicate { name: "ancestor".into(), args: vec![Term::Var("X".into()), Term::Var("Y".into())] },
//!     vec![
//!         Predicate { name: "parent".into(), args: vec![Term::Var("X".into()), Term::Var("Z".into())] },
//!         Predicate { name: "ancestor".into(), args: vec![Term::Var("Z".into()), Term::Var("Y".into())] },
//!     ],
//! );
//! eng.semi_naive();
//! let results = eng.query("ancestor", 2);
//! assert!(results.iter().any(|r| r == &["alice".to_string(), "carol".to_string()]));
//! # }
//! ```

use std::collections::{HashMap, HashSet};

/// A term: either a constant or a variable.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Term {
    Const(String),
    Var(String),
}

impl Term {
    pub fn is_var(&self) -> bool {
        matches!(self, Term::Var(_))
    }
}

/// A predicate with a name and argument list.
#[derive(Debug, Clone)]
pub struct Predicate {
    pub name: String,
    pub args: Vec<Term>,
}

/// Aggregation operator.
#[derive(Debug, Clone, Copy)]
pub enum Op {
    Count,
    Sum,
    Min,
    Max,
}

/// Aggregation result row.
#[derive(Debug, Clone)]
pub struct AggRow {
    pub group_key: String,
    pub value: f64,
}

type Tuple = Vec<String>;
type Binding = HashMap<String, String>;
pub type Rule = (Predicate, Vec<Predicate>);
type DeltaRelation<'a> = (&'a str, &'a HashSet<Tuple>);

fn match_args(args: &[Term], tuple: &[String]) -> Option<Binding> {
    if args.len() != tuple.len() {
        return None;
    }
    let mut b: Binding = HashMap::new();
    for (t, v) in args.iter().zip(tuple.iter()) {
        match t {
            Term::Const(c) => {
                if c != v {
                    return None;
                }
            }
            Term::Var(x) => {
                if let Some(prev) = b.get(x) {
                    if prev != v {
                        return None;
                    }
                } else {
                    b.insert(x.clone(), v.clone());
                }
            }
        }
    }
    Some(b)
}

fn merge_bindings(a: &Binding, b: &Binding) -> Option<Binding> {
    let mut out = a.clone();
    for (k, v) in b {
        if let Some(existing) = out.get(k) {
            if existing != v {
                return None;
            }
        } else {
            out.insert(k.clone(), v.clone());
        }
    }
    Some(out)
}

fn apply_binding(args: &[Term], b: &Binding) -> Option<Tuple> {
    args.iter()
        .map(|t| match t {
            Term::Const(c) => Some(c.clone()),
            Term::Var(x) => b.get(x).cloned(),
        })
        .collect()
}

/// Core Datalog engine with semi-naive evaluation.
pub struct DatalogEngine {
    /// EDB (extensional): predicate → set of tuples
    pub facts: HashMap<String, HashSet<Tuple>>,
    /// IDB (intensional): rules
    pub rules: Vec<Rule>,
}

impl DatalogEngine {
    pub fn new() -> Self {
        Self {
            facts: HashMap::new(),
            rules: Vec::new(),
        }
    }

    /// Add a base fact.
    pub fn fact(&mut self, pred: &str, args: &[&str]) {
        self.facts
            .entry(pred.to_string())
            .or_default()
            .insert(args.iter().map(|s| s.to_string()).collect());
    }

    /// Add a Datalog rule: head :- body.
    pub fn add_rule(&mut self, head: Predicate, body: Vec<Predicate>) {
        self.rules.push((head, body));
    }

    /// Parse S-expression `(pred ?x y)` into Predicate.
    pub fn parse_predicate(s: &str) -> Result<Predicate, String> {
        let s = s.trim();
        let s = s
            .strip_prefix('(')
            .and_then(|s| s.strip_suffix(')'))
            .ok_or_else(|| format!("expected (...): {s}"))?;
        let mut parts = s.split_whitespace();
        let name = parts.next().ok_or("empty predicate")?.to_string();
        let args = parts
            .map(|p| {
                if let Some(name) = p.strip_prefix('?') {
                    Term::Var(name.to_string())
                } else {
                    Term::Const(p.to_string())
                }
            })
            .collect();
        Ok(Predicate { name, args })
    }

    /// Semi-naive bottom-up evaluation to fixed point.
    pub fn semi_naive(&mut self) {
        // Collect IDB predicate names (heads of rules).
        let idb_preds: HashSet<String> = self.rules.iter().map(|(h, _)| h.name.clone()).collect();

        // Initial delta = whatever is already in IDB slots (may be empty).
        let mut delta: HashMap<String, HashSet<Tuple>> = idb_preds
            .iter()
            .map(|p| (p.clone(), self.facts.get(p).cloned().unwrap_or_default()))
            .collect();

        loop {
            let mut new_delta: HashMap<String, HashSet<Tuple>> = HashMap::new();

            for (head, body) in &self.rules {
                // Determine which body literals are IDB (can participate as delta).
                let idb_positions: Vec<usize> = body
                    .iter()
                    .enumerate()
                    .filter(|(_, p)| idb_preds.contains(&p.name))
                    .map(|(i, _)| i)
                    .collect();

                let candidates = if idb_positions.is_empty() {
                    // Pure EDB rule: derive once using full relations.
                    self.derive_rule(head, body)
                } else {
                    // For each IDB literal, use its delta; others use full relation.
                    let mut all: Vec<Tuple> = Vec::new();
                    for pos in &idb_positions {
                        let dp = &body[*pos].name;
                        if let Some(dt) = delta.get(dp.as_str()) {
                            if dt.is_empty() {
                                continue;
                            }
                            // Build a body where this literal uses delta, rest full.
                            let new_tuples = self.derive_rule_delta_at(head, body, *pos, dt);
                            all.extend(new_tuples);
                        }
                    }
                    all
                };

                let full = self.facts.entry(head.name.clone()).or_default();
                for t in candidates {
                    if full.insert(t.clone()) {
                        new_delta.entry(head.name.clone()).or_default().insert(t);
                    }
                }
            }

            if new_delta.values().all(|s| s.is_empty()) {
                break;
            }
            delta = new_delta;
        }
    }

    /// Derive using delta only at `delta_pos` body literal; rest use full facts.
    fn derive_rule_delta_at(
        &self,
        head: &Predicate,
        body: &[Predicate],
        delta_pos: usize,
        delta_set: &HashSet<Tuple>,
    ) -> Vec<Tuple> {
        let empty: HashSet<Tuple> = HashSet::new();
        let mut bindings: Vec<Binding> = vec![HashMap::new()];
        for (i, pred) in body.iter().enumerate() {
            let tuples: &HashSet<Tuple> = if i == delta_pos {
                delta_set
            } else {
                self.facts.get(&pred.name).unwrap_or(&empty)
            };
            let mut next: Vec<Binding> = Vec::new();
            for b in &bindings {
                for t in tuples {
                    if let Some(nb) = match_args(&pred.args, t)
                        && let Some(merged) = merge_bindings(b, &nb)
                    {
                        next.push(merged);
                    }
                }
            }
            bindings = next;
        }
        bindings
            .iter()
            .filter_map(|b| apply_binding(&head.args, b))
            .collect()
    }

    fn derive_rule(&self, head: &Predicate, body: &[Predicate]) -> Vec<Tuple> {
        self.derive_rule_with_delta(head, body, None)
    }

    /// Derive new tuples using semi-naive delta: at least one body literal must
    /// come from `delta_pred` (the IDB predicate being iterated) hitting `delta`.
    /// If `delta` is None, derive from full relations (used for EDB-only rules).
    fn derive_rule_with_delta<'a>(
        &'a self,
        head: &Predicate,
        body: &[Predicate],
        delta: Option<DeltaRelation<'a>>,
    ) -> Vec<Tuple> {
        let empty: HashSet<Tuple> = HashSet::new();
        let mut bindings: Vec<Binding> = vec![HashMap::new()];
        for pred in body {
            let tuples: &HashSet<Tuple> = if let Some((dp, dt)) = delta {
                if pred.name == dp {
                    dt
                } else {
                    self.facts.get(&pred.name).unwrap_or(&empty)
                }
            } else {
                self.facts.get(&pred.name).unwrap_or(&empty)
            };
            let mut next: Vec<Binding> = Vec::new();
            for b in &bindings {
                for t in tuples {
                    if let Some(nb) = match_args(&pred.args, t)
                        && let Some(merged) = merge_bindings(b, &nb)
                    {
                        next.push(merged);
                    }
                }
            }
            bindings = next;
        }
        bindings
            .iter()
            .filter_map(|b| apply_binding(&head.args, b))
            .collect()
    }

    /// Query all tuples for a predicate with arity check.
    pub fn query(&self, pred: &str, arity: usize) -> Vec<Tuple> {
        self.facts
            .get(pred)
            .map(|s| s.iter().filter(|t| t.len() == arity).cloned().collect())
            .unwrap_or_default()
    }

    /// Aggregate over a predicate.
    /// `predicate`: relation name, `group_by`: 0-based column index as string,
    /// `value_col`: 0-based column index to aggregate, `op`: aggregation op.
    pub fn aggregate(
        &self,
        predicate: &str,
        group_by_col: usize,
        value_col: usize,
        op: Op,
    ) -> Vec<AggRow> {
        let tuples = match self.facts.get(predicate) {
            Some(s) => s,
            None => return vec![],
        };
        let mut groups: HashMap<String, Vec<f64>> = HashMap::new();
        for t in tuples {
            let key = t.get(group_by_col).cloned().unwrap_or_default();
            let val: f64 = t.get(value_col).and_then(|v| v.parse().ok()).unwrap_or(0.0);
            groups.entry(key).or_default().push(val);
        }
        let mut out: Vec<AggRow> = groups
            .into_iter()
            .map(|(group_key, vals)| {
                let value = match op {
                    Op::Count => vals.len() as f64,
                    Op::Sum => vals.iter().sum(),
                    Op::Min => vals.iter().cloned().fold(f64::INFINITY, f64::min),
                    Op::Max => vals.iter().cloned().fold(f64::NEG_INFINITY, f64::max),
                };
                AggRow { group_key, value }
            })
            .collect();
        out.sort_by(|a, b| a.group_key.cmp(&b.group_key));
        out
    }
}

impl Default for DatalogEngine {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn family_tree() -> DatalogEngine {
        let mut eng = DatalogEngine::new();
        // gen1 → gen2 → gen3
        eng.fact("parent", &["alice", "bob"]);
        eng.fact("parent", &["alice", "carol"]);
        eng.fact("parent", &["bob", "dave"]);
        eng.fact("parent", &["carol", "eve"]);
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
        eng
    }

    #[test]
    fn transitive_closure() {
        let mut eng = family_tree();
        eng.semi_naive();
        let results = eng.query("ancestor", 2);
        // alice is ancestor of dave and eve (gen3)
        assert!(
            results.iter().any(|r| r[0] == "alice" && r[1] == "dave"),
            "alice->dave missing"
        );
        assert!(
            results.iter().any(|r| r[0] == "alice" && r[1] == "eve"),
            "alice->eve missing"
        );
        // direct parent facts also in ancestor
        assert!(
            results.iter().any(|r| r[0] == "alice" && r[1] == "bob"),
            "alice->bob missing"
        );
        // total: alice→{bob,carol,dave,eve}=4, bob→{dave}=1, carol→{eve}=1 = 6
        assert_eq!(results.len(), 6);
    }

    #[test]
    fn parse_predicate_round_trip() {
        let p = DatalogEngine::parse_predicate("(parent ?x ?y)").unwrap();
        assert_eq!(p.name, "parent");
        assert_eq!(p.args, vec![Term::Var("x".into()), Term::Var("y".into())]);
    }

    #[test]
    fn aggregate_count() {
        let mut eng = DatalogEngine::new();
        // score(person, value)
        eng.fact("score", &["alice", "10"]);
        eng.fact("score", &["alice", "20"]);
        eng.fact("score", &["bob", "5"]);
        let rows = eng.aggregate("score", 0, 1, Op::Count);
        let alice = rows.iter().find(|r| r.group_key == "alice").unwrap();
        assert_eq!(alice.value as usize, 2);
    }

    #[test]
    fn aggregate_sum() {
        let mut eng = DatalogEngine::new();
        eng.fact("score", &["alice", "10"]);
        eng.fact("score", &["alice", "20"]);
        let rows = eng.aggregate("score", 0, 1, Op::Sum);
        let alice = rows.iter().find(|r| r.group_key == "alice").unwrap();
        assert_eq!(alice.value, 30.0);
    }

    #[test]
    fn ancestor_100_chain_perf() {
        let n = 100usize;
        let mut eng = DatalogEngine::new();
        for i in 0..n - 1 {
            let a = i.to_string();
            let b = (i + 1).to_string();
            eng.fact("parent", &[a.as_str(), b.as_str()]);
        }
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
        let t0 = std::time::Instant::now();
        eng.semi_naive();
        let elapsed = t0.elapsed();
        let results = eng.query("ancestor", 2);
        let expected = n * (n - 1) / 2;
        assert_eq!(results.len(), expected, "wrong ancestor count");
        // Must complete well under 1s (old code took 7131ms for this case)
        assert!(
            elapsed.as_millis() < 1000,
            "too slow: {}ms",
            elapsed.as_millis()
        );
        eprintln!(
            "ancestor_100_chain: {}ms, {} tuples",
            elapsed.as_millis(),
            results.len()
        );
    }
}
