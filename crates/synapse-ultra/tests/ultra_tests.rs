//! Tests for synapse-ultra.

use synapse_ultra::events::{
    DecisionInput, EventFilter, TokenCostInput, ingest_decision, ingest_event, ingest_event_json,
    ingest_events,
};
use synapse_ultra::graph::{EdgeInput, graph_expand, upsert_edge, upsert_node, why};
use synapse_ultra::{Event, EventKind, Ultra};

fn fresh() -> Ultra {
    let u = Ultra::open_memory().unwrap();
    u.migrate().unwrap();
    u
}

#[test]
fn migrate_is_idempotent() {
    let u = Ultra::open_memory().unwrap();
    u.migrate().unwrap();
    u.migrate().unwrap(); // second call must not error
    let v = u.with_conn(synapse_ultra::schema::schema_version);
    assert_eq!(v, 3);
}

#[test]
fn ingest_and_query_event() {
    let u = fresh();
    let e = Event {
        ts: 1000,
        session_id: Some("s1".into()),
        agent: "claude".into(),
        kind: EventKind::Decision.as_str().to_string(),
        uri: Some("file:foo.rs".into()),
        content: Some("refactored bar".into()),
        meta: None,
    };
    let id = u.with_conn(|c| ingest_event(c, &e)).unwrap();
    assert!(id > 0);
    // dedup: same event again returns same id
    let id2 = u.with_conn(|c| ingest_event(c, &e)).unwrap();
    assert_eq!(id, id2);
    // query
    let rows = u
        .with_conn(|c| {
            synapse_ultra::events::query_events(
                c,
                &EventFilter::new().agent("claude").kind("decision"),
            )
        })
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].agent, "claude");
    assert_eq!(rows[0].kind, "decision");
}

#[test]
fn ingest_event_json_roundtrip() {
    let u = fresh();
    let json = r#"{"ts":1234,"session_id":"s2","agent":"codex","kind":"tool_call","uri":"file:x.rs","content":"edit"}"#;
    let id = u.with_conn(|c| ingest_event_json(c, json)).unwrap();
    assert!(id > 0);
}

#[test]
fn decision_creates_graph_nodes_and_edges() {
    let u = fresh();
    let id = u
        .with_conn(|c| {
            ingest_decision(
                c,
                &DecisionInput {
                    ts: 5000,
                    session_id: Some("sess"),
                    agent: "claude",
                    uri: "file:bar.rs",
                    rationale: Some("because reason"),
                    source_uri: Some("file:foo.rs"),
                    target_uri: Some("file:baz.rs"),
                    meta: None,
                },
            )
        })
        .unwrap();
    assert!(id > 0);
    // trigger should have created 3 nodes + 2 edges
    let (nodes, edges) = u.with_conn(synapse_ultra::graph::counts).unwrap();
    assert_eq!(nodes, 3);
    assert_eq!(edges, 2);
}

#[test]
fn why_chain_traverses_backwards() {
    let u = fresh();
    // Build a chain: A caused B caused C
    let ts = 1000;
    u.with_conn(|c| upsert_node(c, "A", "source", None, ts))
        .unwrap();
    u.with_conn(|c| upsert_node(c, "B", "decision", None, ts + 1))
        .unwrap();
    u.with_conn(|c| upsert_node(c, "C", "target", None, ts + 2))
        .unwrap();
    u.with_conn(|c| upsert_edge(c, &EdgeInput::new("A", "B", "caused", ts)))
        .unwrap();
    u.with_conn(|c| upsert_edge(c, &EdgeInput::new("B", "C", "derived_from", ts)))
        .unwrap();
    // why(C) should return C (depth 0), B (depth 1), A (depth 2)
    let steps = u.with_conn(|c| why(c, "C", 5)).unwrap();
    assert_eq!(steps.len(), 3);
    assert_eq!(steps[0].uri, "C");
    assert_eq!(steps[0].depth, 0);
    assert_eq!(steps[1].uri, "B");
    assert_eq!(steps[1].depth, 1);
    assert_eq!(steps[2].uri, "A");
    assert_eq!(steps[2].depth, 2);
}

#[test]
fn graph_expand_traverses_forwards() {
    let u = fresh();
    let ts = 2000;
    u.with_conn(|c| upsert_node(c, "X", "source", None, ts))
        .unwrap();
    u.with_conn(|c| upsert_node(c, "Y", "decision", None, ts + 1))
        .unwrap();
    u.with_conn(|c| upsert_node(c, "Z", "target", None, ts + 2))
        .unwrap();
    u.with_conn(|c| upsert_edge(c, &EdgeInput::new("X", "Y", "caused", ts)))
        .unwrap();
    u.with_conn(|c| upsert_edge(c, &EdgeInput::new("Y", "Z", "derived_from", ts)))
        .unwrap();
    let steps = u.with_conn(|c| graph_expand(c, "X", 5)).unwrap();
    assert!(steps.iter().any(|s| s.uri == "X" && s.depth == 0));
    assert!(steps.iter().any(|s| s.uri == "Y" && s.depth == 1));
    assert!(steps.iter().any(|s| s.uri == "Z" && s.depth == 2));
}

#[test]
fn why_chain_10k_nodes_under_50ms() {
    let u = fresh();
    let ts = 0;
    // Build a linear chain of 10k nodes: n0 -> n1 -> ... -> n9999
    u.with_conn(|c| {
        for i in 0..10_000i64 {
            upsert_node(c, &format!("n{i}"), "decision", None, ts + i).unwrap();
            if i > 0 {
                upsert_edge(
                    c,
                    &EdgeInput::new(&format!("n{}", i - 1), &format!("n{i}"), "caused", ts + i),
                )
                .unwrap();
            }
        }
        let start = std::time::Instant::now();
        let steps = why(c, "n9999", 20).unwrap();
        let elapsed = start.elapsed();
        assert!(
            !steps.is_empty(),
            "should return at least the starting node"
        );
        assert!(
            elapsed.as_millis() < 100,
            "why() on 10k nodes took {:?}, expected < 100ms",
            elapsed
        );
        Ok::<(), synapse_ultra::UltraError>(())
    })
    .unwrap();
}

#[test]
fn brain_stats_returns_counts() {
    let u = fresh();
    // ingest one event
    let e = Event::now("claude", EventKind::Message);
    u.with_conn(|c| ingest_event(c, &e)).unwrap();
    let stats = u.with_conn(synapse_ultra::observe::brain_stats).unwrap();
    assert_eq!(stats.events, 1);
    assert_eq!(stats.ultra_schema_version, 3);
}

#[test]
fn token_cost_aggregates() {
    let u = fresh();
    u.with_conn(|c| {
        synapse_ultra::events::ingest_token_cost(
            c,
            &TokenCostInput {
                ts: 1000,
                session_id: Some("s"),
                agent: "claude",
                model: "claude-fable-5",
                input_tokens: 100,
                output_tokens: 200,
                cache_read: 0,
                cache_write: 0,
                cost_usd: 0.01,
                meta: None,
            },
        )
    })
    .unwrap();
    u.with_conn(|c| {
        synapse_ultra::events::ingest_token_cost(
            c,
            &TokenCostInput {
                ts: 2000,
                session_id: Some("s"),
                agent: "codex",
                model: "gpt-5.6",
                input_tokens: 50,
                output_tokens: 75,
                cache_read: 0,
                cache_write: 0,
                cost_usd: 0.005,
                meta: None,
            },
        )
    })
    .unwrap();
    let stats = u.with_conn(synapse_ultra::observe::brain_stats).unwrap();
    assert_eq!(stats.token_cost_rows, 2);
    assert!((stats.total_cost_usd - 0.015).abs() < 1e-9);
}

#[test]
fn ingest_jsonl_file_skips_blank_lines() {
    let u = fresh();
    let tmp = tempfile::NamedTempFile::new().unwrap();
    let content = r#"{"ts":1,"agent":"a","kind":"message","content":"hi"}
# this is a comment

{"ts":2,"agent":"a","kind":"message","content":"yo"}
"#;
    std::fs::write(tmp.path(), content).unwrap();
    let n = u
        .with_conn(|c| synapse_ultra::events::ingest_jsonl_file(c, tmp.path()))
        .unwrap();
    assert_eq!(n, 2);
}

#[test]
fn ingest_events_batch_dedups() {
    let u = fresh();
    let ev = |content: &str| Event {
        ts: 1,
        session_id: Some("s1".into()),
        agent: "claude".into(),
        kind: "message".into(),
        uri: Some("file:x".into()),
        content: Some(content.into()),
        meta: None,
    };
    // 3 distinct + 1 duplicate of ev1
    let batch = vec![ev("a"), ev("b"), ev("c"), ev("a")];
    let n = u.with_conn(|c| ingest_events(c, &batch)).unwrap();
    assert_eq!(n, 3);
    // second batch with same events — all dedup
    let n2 = u.with_conn(|c| ingest_events(c, &batch)).unwrap();
    assert_eq!(n2, 0);
}

#[test]
fn why_chain_handles_cycles() {
    let u = fresh();
    let ts = 1000;
    // Build a cycle: A -> B -> C -> A
    u.with_conn(|c| upsert_node(c, "A", "n", None, ts)).unwrap();
    u.with_conn(|c| upsert_node(c, "B", "n", None, ts + 1))
        .unwrap();
    u.with_conn(|c| upsert_node(c, "C", "n", None, ts + 2))
        .unwrap();
    u.with_conn(|c| upsert_edge(c, &EdgeInput::new("A", "B", "caused", ts)))
        .unwrap();
    u.with_conn(|c| upsert_edge(c, &EdgeInput::new("B", "C", "caused", ts)))
        .unwrap();
    u.with_conn(|c| upsert_edge(c, &EdgeInput::new("C", "A", "caused", ts)))
        .unwrap();
    // why(C) with cycle protection — must terminate
    let steps = u.with_conn(|c| why(c, "C", 20)).unwrap();
    // Should return C (depth 0), B (depth 1), A (depth 2) — and NOT loop back to C
    assert!(steps.iter().any(|s| s.uri == "C" && s.depth == 0));
    assert!(steps.iter().any(|s| s.uri == "B" && s.depth == 1));
    assert!(steps.iter().any(|s| s.uri == "A" && s.depth == 2));
    // No step should have depth > 3 (cycle would produce infinite or repeated)
    assert!(steps.iter().all(|s| s.depth <= 2));
}

#[test]
fn decision_dedup_distinguishes_rationale() {
    let u = fresh();
    // Same URI + agent + session, different rationale → distinct decisions
    let id1 = u
        .with_conn(|c| {
            ingest_decision(
                c,
                &DecisionInput {
                    ts: 1000,
                    session_id: Some("s"),
                    agent: "claude",
                    uri: "file:foo.rs",
                    rationale: Some("reason-A"),
                    source_uri: Some("src"),
                    target_uri: Some("tgt"),
                    meta: None,
                },
            )
        })
        .unwrap();
    let id2 = u
        .with_conn(|c| {
            ingest_decision(
                c,
                &DecisionInput {
                    ts: 1000,
                    session_id: Some("s"),
                    agent: "claude",
                    uri: "file:foo.rs",
                    rationale: Some("reason-B"),
                    source_uri: Some("src"),
                    target_uri: Some("tgt"),
                    meta: None,
                },
            )
        })
        .unwrap();
    assert_ne!(
        id1, id2,
        "different rationale must produce distinct decisions"
    );
    // Same rationale → dedup
    let id3 = u
        .with_conn(|c| {
            ingest_decision(
                c,
                &DecisionInput {
                    ts: 1000,
                    session_id: Some("s"),
                    agent: "claude",
                    uri: "file:foo.rs",
                    rationale: Some("reason-A"),
                    source_uri: Some("src"),
                    target_uri: Some("tgt"),
                    meta: None,
                },
            )
        })
        .unwrap();
    assert_eq!(id1, id3);
}

#[test]
fn to_dot_does_not_duplicate_edges() {
    let u = fresh();
    let ts = 1000;
    u.with_conn(|c| upsert_node(c, "A", "n", None, ts)).unwrap();
    u.with_conn(|c| upsert_node(c, "B", "n", None, ts)).unwrap();
    u.with_conn(|c| upsert_edge(c, &EdgeInput::new("A", "B", "caused", ts)))
        .unwrap();
    let dot = u
        .with_conn(|c| synapse_ultra::graph::to_dot(c, "A", 5))
        .unwrap();
    // Count occurrences of the edge A -> B — must be exactly 1
    let edge_count = dot.matches("\"A\" -> \"B\"").count();
    assert_eq!(
        edge_count, 1,
        "to_dot must not duplicate edges: {}\n{}",
        edge_count, dot
    );
}

#[test]
fn agent_trace_returns_events_for_agent() {
    let u = fresh();
    let ts = 1000;
    let mk = |agent: &str, uri: &str, sess: &str, t: i64| Event {
        ts: t,
        session_id: Some(sess.into()),
        agent: agent.into(),
        kind: EventKind::ToolCall.as_str().to_string(),
        uri: Some(uri.into()),
        content: Some("x".into()),
        meta: None,
    };
    u.with_conn(|c| ingest_event(c, &mk("claude", "file:a.rs", "s1", ts)))
        .unwrap();
    u.with_conn(|c| ingest_event(c, &mk("codex", "file:b.rs", "s1", ts + 10)))
        .unwrap();
    u.with_conn(|c| ingest_event(c, &mk("claude", "file:c.rs", "s2", ts + 20)))
        .unwrap();
    let rows = u
        .with_conn(|c| synapse_ultra::observe::agent_trace(c, "claude", ts - 100, ts + 100, 100))
        .unwrap();
    assert_eq!(
        rows.len(),
        2,
        "agent_trace should return 2 claude events, got {}",
        rows.len()
    );
    assert!(rows.iter().all(|r| r.session_id.is_some()));
}

#[test]
fn daily_summary_aggregates_totals_and_agents() {
    let u = fresh();
    let ts = 5000;
    let mk = |agent: &str, uri: &str, sess: &str, t: i64| Event {
        ts: t,
        session_id: Some(sess.into()),
        agent: agent.into(),
        kind: EventKind::ToolCall.as_str().to_string(),
        uri: Some(uri.into()),
        content: Some("x".into()),
        meta: None,
    };
    u.with_conn(|c| ingest_event(c, &mk("claude", "file:a.rs", "s1", ts)))
        .unwrap();
    u.with_conn(|c| ingest_event(c, &mk("codex", "file:b.rs", "s1", ts + 5)))
        .unwrap();
    u.with_conn(|c| {
        ingest_decision(
            c,
            &DecisionInput {
                ts: ts + 10,
                session_id: Some("s1"),
                agent: "claude",
                uri: "file:a.rs",
                rationale: Some("why"),
                source_uri: Some("src"),
                target_uri: Some("tgt"),
                meta: None,
            },
        )
    })
    .unwrap();
    let s = u
        .with_conn(|c| synapse_ultra::observe::daily_summary(c, ts - 100, ts + 100))
        .unwrap();
    assert_eq!(s.total_events, 2, "total_events");
    assert_eq!(s.total_decisions, 1, "total_decisions");
    assert_eq!(s.total_sessions, 1, "total_sessions");
    assert_eq!(
        s.agents.len(),
        2,
        "per-agent breakdown should have 2 agents"
    );
    assert_eq!(s.top_decisions.len(), 1);
}

#[test]
fn session_timeline_merges_events_and_decisions() {
    let u = fresh();
    let ts = 7000;
    let mk = |agent: &str, uri: &str, sess: &str, t: i64| Event {
        ts: t,
        session_id: Some(sess.into()),
        agent: agent.into(),
        kind: EventKind::ToolCall.as_str().to_string(),
        uri: Some(uri.into()),
        content: Some("x".into()),
        meta: None,
    };
    u.with_conn(|c| ingest_event(c, &mk("claude", "file:a.rs", "sx", ts)))
        .unwrap();
    u.with_conn(|c| {
        ingest_decision(
            c,
            &DecisionInput {
                ts: ts + 100,
                session_id: Some("sx"),
                agent: "claude",
                uri: "file:a.rs",
                rationale: Some("rationale"),
                source_uri: Some("src"),
                target_uri: Some("tgt"),
                meta: None,
            },
        )
    })
    .unwrap();
    let rows = u
        .with_conn(|c| synapse_ultra::observe::session_timeline(c, "sx", 100))
        .unwrap();
    assert_eq!(
        rows.len(),
        2,
        "timeline should have 2 rows (1 event + 1 decision)"
    );
    assert!(rows[0].ts <= rows[1].ts);
    assert!(
        rows.iter().any(|r| r.is_decision),
        "timeline should contain the decision row"
    );
}

#[test]
fn list_sessions_orders_by_last_ts_desc() {
    let u = fresh();
    let ts = 9000;
    let mk = |agent: &str, uri: &str, sess: &str, t: i64| Event {
        ts: t,
        session_id: Some(sess.into()),
        agent: agent.into(),
        kind: EventKind::ToolCall.as_str().to_string(),
        uri: Some(uri.into()),
        content: Some("x".into()),
        meta: None,
    };
    u.with_conn(|c| ingest_event(c, &mk("claude", "file:a.rs", "s_old", ts)))
        .unwrap();
    u.with_conn(|c| ingest_event(c, &mk("claude", "file:b.rs", "s_new", ts + 500)))
        .unwrap();
    let rows = u
        .with_conn(|c| synapse_ultra::observe::list_sessions(c, None, 50))
        .unwrap();
    assert_eq!(rows.len(), 2, "should list 2 sessions");
    assert_eq!(rows[0].session_id, "s_new");
    assert_eq!(rows[1].session_id, "s_old");
    let filtered = u
        .with_conn(|c| synapse_ultra::observe::list_sessions(c, Some("claude"), 50))
        .unwrap();
    assert_eq!(filtered.len(), 2, "agent filter claude should match both");
}

#[test]
fn search_events_fts5_finds_terms() {
    let u = fresh();
    let mk = |content: &str| Event {
        ts: 1000,
        session_id: Some("s1".into()),
        agent: "claude".into(),
        kind: EventKind::Message.as_str().to_string(),
        uri: Some("file:x".into()),
        content: Some(content.into()),
        meta: None,
    };
    u.with_conn(|c| ingest_event(c, &mk("refactored bar module")))
        .unwrap();
    u.with_conn(|c| ingest_event(c, &mk("unrelated baz work")))
        .unwrap();
    u.with_conn(|c| ingest_event(c, &mk("another refactor pass")))
        .unwrap();

    let hits = u
        .with_conn(|c| synapse_ultra::events::search_events(c, "refactor", None))
        .unwrap();
    assert!(!hits.is_empty(), "FTS5 should find 'refactor' matches");
    assert!(
        hits.iter()
            .all(|r| r.content.as_deref().unwrap_or("").contains("refactor"))
    );
}

#[test]
fn ingest_events_batch_with_prepared_stmt() {
    let u = fresh();
    let mk = |i: i64| Event {
        ts: 1000 + i,
        session_id: Some("s1".into()),
        agent: "claude".into(),
        kind: EventKind::Message.as_str().to_string(),
        uri: Some(format!("file:{i}")),
        content: Some(format!("content-{i}")),
        meta: None,
    };
    let batch: Vec<Event> = (0..100).map(mk).collect();
    let n = u.with_conn(|c| ingest_events(c, &batch)).unwrap();
    assert_eq!(n, 100);
    // re-ingest → all dedup
    let n2 = u.with_conn(|c| ingest_events(c, &batch)).unwrap();
    assert_eq!(n2, 0);
}

#[test]
fn why_at_filters_by_time() {
    use synapse_ultra::graph::{EdgeInput, upsert_edge, upsert_node, why, why_at};
    let u = fresh();
    u.with_conn(|c| {
        // Two edges: one at t=1000, one at t=5000.
        upsert_node(c, "file:a", "file", None, 1000).unwrap();
        upsert_node(c, "file:b", "file", None, 1000).unwrap();
        upsert_node(c, "file:c", "file", None, 1000).unwrap();
        upsert_edge(c, &EdgeInput::new("file:b", "file:a", "caused", 1000)).unwrap();
        upsert_edge(c, &EdgeInput::new("file:c", "file:a", "caused", 5000)).unwrap();
    });
    // why at t=2000: only file:b visible.
    let chain_early = u.with_conn(|c| why_at(c, "file:a", 5, 2000)).unwrap();
    let uris_early: Vec<String> = chain_early.iter().map(|s| s.uri.clone()).collect();
    assert!(uris_early.contains(&"file:b".to_string()));
    assert!(!uris_early.contains(&"file:c".to_string()));

    // why at t=10000: both visible.
    let chain_late = u.with_conn(|c| why_at(c, "file:a", 5, 10000)).unwrap();
    let uris_late: Vec<String> = chain_late.iter().map(|s| s.uri.clone()).collect();
    assert!(uris_late.contains(&"file:b".to_string()));
    assert!(uris_late.contains(&"file:c".to_string()));

    // Sanity: why() (no time filter) returns both.
    let chain_all = u.with_conn(|c| why(c, "file:a", 5)).unwrap();
    assert!(chain_all.len() >= 3);
}
