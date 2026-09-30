//! synapse-ultra CLI — lean observe + ingest + lake surface.
//!
//! Usage:
//!   synapse-ultra init [--db PATH]
//!   synapse-ultra ingest --db PATH --jsonl FILE
//!   synapse-ultra ingest --db PATH --json '{"agent":"claude","kind":"decision",...}'
//!   synapse-ultra inspect [--db PATH]
//!   synapse-ultra why --db PATH --uri URI [--depth N]
//!   synapse-ultra graph --db PATH --uri URI [--depth N] [--dot]
//!   synapse-ultra replay --db PATH --session ID [--limit N]
//!   synapse-ultra cost --db PATH [--since -7d] [--by agent|model|day]
//!   synapse-ultra events --db PATH [--agent X] [--kind Y] [--session S] [--limit N]
//!   synapse-ultra search --db PATH "fts5 query" [--limit N]
//!   synapse-ultra lake init   --db PATH [--catalog PATH]
//!   synapse-ultra lake archive --db PATH --older-than DAYS [--catalog PATH]
//!   synapse-ultra lake analytics --db PATH [--catalog PATH]
//!   synapse-ultra doctor [--db PATH]

use anyhow::Result;
use clap::{Parser, Subcommand};
use synapse_ultra::{Ultra, UltraResult};

#[derive(Parser)]
#[command(
    name = "synapse-ultra",
    version,
    about = "Synapse Ultra — event log, graph-v2 CTE, observe CLI for synapse-memory"
)]
struct Cli {
    /// Path to the brain.db file. Defaults to ~/.synapse/brain.db.
    #[arg(long, global = true, env = "SYNAPSE_DB")]
    db: Option<std::path::PathBuf>,

    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Initialize the Ultra schema (idempotent).
    Init,
    /// Ingest events from JSON or JSONL.
    Ingest {
        /// JSON string of a single Event.
        #[arg(long)]
        json: Option<String>,
        /// Path to a JSONL file (one Event per line).
        #[arg(long)]
        jsonl: Option<std::path::PathBuf>,
    },
    /// Ingest a decision (creates graph nodes + edges via trigger).
    Decision {
        /// Decision URI (the thing being decided).
        #[arg(long)]
        uri: String,
        /// Agent making the decision.
        #[arg(long)]
        agent: String,
        /// Optional rationale text.
        #[arg(long)]
        rationale: Option<String>,
        /// Source URI (what caused this decision).
        #[arg(long)]
        source: Option<String>,
        /// Target URI (what this decision produces/derives).
        #[arg(long)]
        target: Option<String>,
        /// Optional session id.
        #[arg(long)]
        session: Option<String>,
    },
    /// Show brain statistics.
    Inspect,
    /// Decision-chain: what caused this URI?
    Why {
        #[arg(long)]
        uri: String,
        #[arg(long, default_value_t = 5)]
        depth: i64,
    },
    /// Forward graph traversal: what does this URI lead to?
    Graph {
        #[arg(long)]
        uri: String,
        #[arg(long, default_value_t = 3)]
        depth: i64,
        /// Output as Graphviz DOT (pipe to `dot -Tsvg`).
        #[arg(long)]
        dot: bool,
    },
    /// Replay a session chronologically.
    Replay {
        #[arg(long)]
        session: String,
        #[arg(long, default_value_t = 1000)]
        limit: i64,
    },
    /// Token cost report.
    Cost {
        /// Days back to include (e.g. 7 = last 7 days). Default 7.
        #[arg(long, default_value_t = 7)]
        days: i64,
    },
    /// List events matching filters.
    Events {
        #[arg(long)]
        agent: Option<String>,
        #[arg(long)]
        kind: Option<String>,
        #[arg(long)]
        session: Option<String>,
        #[arg(long)]
        uri: Option<String>,
        #[arg(long, default_value_t = 100)]
        limit: i64,
    },
    /// Trace one agent across all sessions within a time range.
    Trace {
        #[arg(long)]
        agent: String,
        /// Days back (default 1 = last 24h).
        #[arg(long, default_value_t = 1)]
        days: i64,
        #[arg(long, default_value_t = 1000)]
        limit: i64,
    },
    /// Daily summary: what happened on a given day?
    Daily {
        /// Days back (0 = today, 1 = yesterday, ...). Default 0.
        #[arg(long, default_value_t = 0)]
        days_back: i64,
    },
    /// Session timeline: chronological events + decisions for one session.
    Timeline {
        #[arg(long)]
        session: String,
        #[arg(long, default_value_t = 1000)]
        limit: i64,
    },
    /// List all sessions with event/decision counts and cost.
    Sessions {
        #[arg(long)]
        agent: Option<String>,
        #[arg(long, default_value_t = 50)]
        limit: i64,
    },
    /// Full-text search over event content via FTS5.
    Search {
        /// FTS5 query (e.g. 'refactor', 'error timeout', '"exact phrase"', 'refactor* OR test*').
        query: String,
        #[arg(long, default_value_t = 50)]
        limit: i64,
    },
    /// Optional DuckLake archive operations.
    #[command(subcommand)]
    Lake(LakeCmd),
    /// Health check (11-point audit: integrity, WAL, FTS, indexes, triggers, ...).
    Health {
        /// Emit JSON instead of human-readable output.
        #[arg(long, default_value_t = false)]
        json: bool,
    },
    /// Create a compressed (zstd) backup of brain.db.
    Backup {
        /// Destination directory (default: ~/.synapse/backups/).
        #[arg(long)]
        out: Option<std::path::PathBuf>,
    },
    /// Export metrics in Prometheus or JSON format.
    Metrics {
        /// Output format: prometheus | json.
        #[arg(long, default_value = "prometheus")]
        format: String,
    },
    /// Tag operations.
    #[command(subcommand)]
    Tags(TagsCmd),
    /// Health check (alias for `health`).
    Doctor,
}

#[derive(Subcommand)]
enum TagsCmd {
    /// Create a new tag.
    Add {
        name: String,
        #[arg(long)]
        color: Option<String>,
        #[arg(long)]
        description: Option<String>,
    },
    /// List all tags.
    List,
    /// Tag a doc/event.
    Tag {
        doc_id: i64,
        tag_name: String,
        #[arg(long, default_value = "manual")]
        source: String,
    },
    /// Bulk-tag multiple docs (reads doc IDs from stdin or args).
    Bulk {
        tag_name: String,
        /// Comma-separated doc IDs.
        #[arg(long)]
        ids: String,
        #[arg(long, default_value = "manual")]
        source: String,
    },
    /// Remove a tag from a doc.
    Untag { doc_id: i64, tag_name: String },
    /// List tags applied to a doc.
    For { doc_id: i64 },
    /// List docs with a given tag.
    Docs { tag_name: String },
    /// Add an auto-tag rule (keyword → tag).
    Rule { keyword: String, tag_name: String },
    /// List all auto-tag rules.
    Rules,
    /// Merge two tags (repoints doc_tags, deletes source).
    Merge { from: String, into: String },
    /// Delete tags with no associations.
    Cleanup,
    /// Tag statistics.
    Stats,
    /// Export all tags + associations + rules as JSON.
    Export,
    /// Import tags from a JSON file.
    Import { path: std::path::PathBuf },
}

#[derive(Subcommand)]
enum LakeCmd {
    /// Initialize the DuckLake catalog.
    Init {
        #[arg(long, default_value = "metadata.ducklake")]
        catalog: std::path::PathBuf,
    },
    /// Archive events older than N days to DuckLake.
    Archive {
        #[arg(long)]
        older_than: i64,
        #[arg(long, default_value = "metadata.ducklake")]
        catalog: std::path::PathBuf,
    },
    /// Start an interactive DuckDB analytics shell.
    Analytics {
        #[arg(long, default_value = "metadata.ducklake")]
        catalog: std::path::PathBuf,
    },
}

fn default_db() -> std::path::PathBuf {
    dirs_next::home_dir()
        .map(|h| h.join(".synapse").join("brain.db"))
        .unwrap_or_else(|| std::path::PathBuf::from("brain.db"))
}

fn open_ultra(db: &std::path::Path) -> UltraResult<Ultra> {
    if let Some(parent) = db.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent).ok();
    }
    Ultra::open(db)
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    let db = cli.db.clone().unwrap_or_else(default_db);
    let ultra = open_ultra(&db)?;

    match cli.cmd {
        Cmd::Init => {
            ultra.migrate()?;
            println!("ultra: schema migrated at {}", db.display());
        }
        Cmd::Ingest { json, jsonl } => {
            ultra.migrate()?;
            let count = ultra.with_conn(|c| {
                let mut n = 0;
                if let Some(j) = json {
                    synapse_ultra::events::ingest_event_json(c, &j)?;
                    n += 1;
                }
                if let Some(p) = jsonl {
                    n += synapse_ultra::events::ingest_jsonl_file(c, &p)?;
                }
                UltraResult::Ok(n)
            })?;
            println!("ultra: ingested {count} events");
        }
        Cmd::Decision {
            uri,
            agent,
            rationale,
            source,
            target,
            session,
        } => {
            ultra.migrate()?;
            let ts = chrono::Utc::now().timestamp();
            let id = ultra.with_conn(|c| {
                synapse_ultra::events::ingest_decision(
                    c,
                    &synapse_ultra::events::DecisionInput {
                        ts,
                        session_id: session.as_deref(),
                        agent: &agent,
                        uri: &uri,
                        rationale: rationale.as_deref(),
                        source_uri: source.as_deref(),
                        target_uri: target.as_deref(),
                        meta: None,
                    },
                )
            })?;
            println!("ultra: decision {id} ingested (uri={uri})");
            if source.is_some() || target.is_some() {
                println!("  graph nodes + edges auto-populated by trigger");
            }
        }
        Cmd::Inspect => {
            ultra.migrate()?;
            let stats = ultra.with_conn(synapse_ultra::observe::brain_stats)?;
            println!("Synapse Ultra — brain stats ({})", db.display());
            println!("  docs:              {}", stats.docs);
            println!("  events:            {}", stats.events);
            println!("  decisions:         {}", stats.decisions);
            println!("  graph_nodes:       {}", stats.graph_nodes);
            println!("  graph_edges:       {}", stats.graph_edges);
            println!("  sessions:          {}", stats.sessions);
            println!("  token_cost rows:   {}", stats.token_cost_rows);
            println!("  total cost USD:    ${:.4}", stats.total_cost_usd);
            println!(
                "  total tokens:      {} in / {} out",
                stats.total_input_tokens, stats.total_output_tokens
            );
            println!("  db size:           {} bytes", stats.db_size_bytes);
            println!("  ultra schema:      v{}", stats.ultra_schema_version);
            let agents = ultra.with_conn(|c| synapse_ultra::observe::top_agents(c, 5))?;
            if !agents.is_empty() {
                println!("  top agents:");
                for (a, c) in agents {
                    println!("    {a}: {c}");
                }
            }
            let kinds = ultra.with_conn(|c| synapse_ultra::observe::top_kinds(c, 5))?;
            if !kinds.is_empty() {
                println!("  top kinds:");
                for (k, c) in kinds {
                    println!("    {k}: {c}");
                }
            }
        }
        Cmd::Why { uri, depth } => {
            ultra.migrate()?;
            let steps = ultra.with_conn(|c| synapse_ultra::graph::why(c, &uri, depth))?;
            if steps.is_empty() {
                println!("ultra: no chain found for {uri}");
            } else {
                for s in steps {
                    println!("[d{}] {} ({})  path: {}", s.depth, s.uri, s.kind, s.path);
                }
            }
        }
        Cmd::Graph { uri, depth, dot } => {
            ultra.migrate()?;
            if dot {
                let dot_str = ultra.with_conn(|c| synapse_ultra::graph::to_dot(c, &uri, depth))?;
                print!("{dot_str}");
            } else {
                let steps =
                    ultra.with_conn(|c| synapse_ultra::graph::graph_expand(c, &uri, depth))?;
                if steps.is_empty() {
                    println!("ultra: no forward chain found for {uri}");
                } else {
                    for s in steps {
                        println!("[d{}] {}  path: {}", s.depth, s.uri, s.path);
                    }
                }
            }
        }
        Cmd::Replay { session, limit } => {
            ultra.migrate()?;
            let entries =
                ultra.with_conn(|c| synapse_ultra::observe::replay(c, &session, limit))?;
            if entries.is_empty() {
                println!("ultra: no events for session {session}");
            } else {
                for e in entries {
                    let uri = e.uri.unwrap_or_default();
                    let content = e.content.unwrap_or_default();
                    let content_preview = if content.len() > 120 {
                        format!("{}…", &content[..120])
                    } else {
                        content
                    };
                    println!("[{}] {} uri={} {}", e.ts, e.kind, uri, content_preview);
                }
            }
        }
        Cmd::Cost { days } => {
            ultra.migrate()?;
            let since = chrono::Utc::now().timestamp() - days * 86400;
            let rows = ultra.with_conn(|c| synapse_ultra::observe::cost_by_day(c, since))?;
            if rows.is_empty() {
                println!("ultra: no token_cost rows in the last {days} days");
            } else {
                println!(
                    "{:<12} {:<14} {:<28} {:>6} {:>10} {:>10} {:>10}",
                    "day", "agent", "model", "calls", "in_tok", "out_tok", "usd"
                );
                for r in rows {
                    println!(
                        "{:<12} {:<14} {:<28} {:>6} {:>10} {:>10} {:>10.4}",
                        r.bucket,
                        r.agent,
                        r.model,
                        r.calls,
                        r.input_tokens,
                        r.output_tokens,
                        r.cost_usd
                    );
                }
            }
        }
        Cmd::Events {
            agent,
            kind,
            session,
            uri,
            limit,
        } => {
            ultra.migrate()?;
            let mut filter = synapse_ultra::EventFilter::new().limit(limit);
            if let Some(a) = agent {
                filter = filter.agent(a);
            }
            if let Some(k) = kind {
                filter = filter.kind(k);
            }
            if let Some(s) = session {
                filter = filter.session(s);
            }
            if let Some(u) = uri {
                filter = filter.uri(u);
            }
            let rows = ultra.with_conn(|c| synapse_ultra::events::query_events(c, &filter))?;
            if rows.is_empty() {
                println!("ultra: no events match");
            } else {
                for r in rows {
                    let content = r.content.unwrap_or_default();
                    let preview = if content.len() > 100 {
                        format!("{}…", &content[..100])
                    } else {
                        content
                    };
                    println!(
                        "[{}] {} {} uri={} {}",
                        r.ts,
                        r.agent,
                        r.kind,
                        r.uri.unwrap_or_default(),
                        preview
                    );
                }
            }
        }
        Cmd::Lake(lc) => match lc {
            LakeCmd::Init { catalog } => {
                let cfg = synapse_ultra::lake::LakeConfig {
                    catalog_path: catalog.clone(),
                    data_dir: catalog
                        .parent()
                        .unwrap_or(std::path::Path::new("."))
                        .join("lake-data"),
                };
                synapse_ultra::lake::init(&cfg)?;
                println!(
                    "ultra: ducklake catalog initialized at {}",
                    catalog.display()
                );
            }
            LakeCmd::Archive {
                older_than,
                catalog,
            } => {
                let cfg = synapse_ultra::lake::LakeConfig {
                    catalog_path: catalog.clone(),
                    data_dir: catalog
                        .parent()
                        .unwrap_or(std::path::Path::new("."))
                        .join("lake-data"),
                };
                let cutoff = chrono::Utc::now().timestamp() - older_than * 86400;
                let n = synapse_ultra::lake::archive(&db, &cfg, cutoff)?;
                println!("ultra: archived {n} events older than {older_than} days");
            }
            LakeCmd::Analytics { catalog } => {
                let cfg = synapse_ultra::lake::LakeConfig {
                    catalog_path: catalog.clone(),
                    data_dir: catalog
                        .parent()
                        .unwrap_or(std::path::Path::new("."))
                        .join("lake-data"),
                };
                synapse_ultra::lake::analytics_shell(&db, &cfg)?;
            }
        },
        Cmd::Doctor => {
            ultra.migrate()?;
            let report = ultra.with_conn(synapse_ultra::ops::health_check)?;
            println!("# ultra doctor — {}", db.display());
            println!("overall: {}", if report.overall_ok { "OK" } else { "FAIL" });
            for c in &report.checks {
                let mark = if c.ok { "✓" } else { "✗" };
                println!("  {mark} {:<24} {}", c.name, c.detail);
            }
        }
        Cmd::Trace { agent, days, limit } => {
            ultra.migrate()?;
            let now = chrono::Utc::now().timestamp();
            let since = now - days * 86400;
            let rows = ultra
                .with_conn(|c| synapse_ultra::observe::agent_trace(c, &agent, since, now, limit))?;
            if rows.is_empty() {
                println!("ultra: trace — no events for agent '{agent}' in last {days}d");
                return Ok(());
            }
            println!(
                "# agent trace: {agent} (last {days}d, {} events)",
                rows.len()
            );
            for r in rows {
                let sess = r.session_id.as_deref().unwrap_or("-");
                let uri = r.uri.as_deref().unwrap_or("-");
                let preview = r.content_preview.as_deref().unwrap_or("");
                let preview = if preview.len() > 80 {
                    &preview[..80]
                } else {
                    preview
                };
                println!("{}\t{}\t{}\t{}\t{}", r.ts, sess, r.kind, uri, preview);
            }
        }
        Cmd::Daily { days_back } => {
            ultra.migrate()?;
            let now = chrono::Utc::now().timestamp();
            let day_end = now - days_back * 86400;
            let day_start = day_end - 86400;
            let s = ultra
                .with_conn(|c| synapse_ultra::observe::daily_summary(c, day_start, day_end))?;
            println!(
                "# daily summary — day -{}d  [{} .. {}]",
                days_back, day_start, day_end
            );
            println!("events:       {}", s.total_events);
            println!("decisions:    {}", s.total_decisions);
            println!("sessions:     {}", s.total_sessions);
            println!("cost_usd:     {:.4}", s.total_cost_usd);
            println!(
                "tokens:       in={} out={}",
                s.total_input_tokens, s.total_output_tokens
            );
            println!(
                "graph_growth: +{} nodes +{} edges",
                s.new_graph_nodes, s.new_graph_edges
            );
            println!();
            println!("## agents ({}):", s.agents.len());
            for a in &s.agents {
                println!("  {}:", a.agent);
                println!(
                    "    events={} decisions={} sessions={} cost=${:.4}",
                    a.events, a.decisions, a.sessions, a.cost_usd
                );
                println!("    tokens: in={} out={}", a.input_tokens, a.output_tokens);
                println!("    first_ts={} last_ts={}", a.first_ts, a.last_ts);
                if !a.top_kinds.is_empty() {
                    let kinds = a
                        .top_kinds
                        .iter()
                        .map(|(k, c)| format!("{k}={c}"))
                        .collect::<Vec<_>>()
                        .join(" ");
                    println!("    top_kinds: {kinds}");
                }
                if !a.top_uris.is_empty() {
                    let uris = a
                        .top_uris
                        .iter()
                        .map(|(u, c)| format!("{u}={c}"))
                        .collect::<Vec<_>>()
                        .join(" ");
                    println!("    top_uris:  {uris}");
                }
            }
            if !s.top_decisions.is_empty() {
                println!();
                println!("## top decisions ({}):", s.top_decisions.len());
                for d in &s.top_decisions {
                    let rat = d.rationale.as_deref().unwrap_or("-");
                    let rat = if rat.len() > 80 { &rat[..80] } else { rat };
                    println!("  {}d#{}  agent={}  uri={}", d.ts, d.id, d.agent, d.uri);
                    if let Some(src) = &d.source_uri {
                        println!("    source: {src}");
                    }
                    if let Some(tgt) = &d.target_uri {
                        println!("    target: {tgt}");
                    }
                    println!("    rationale: {rat}");
                }
            }
        }
        Cmd::Timeline { session, limit } => {
            ultra.migrate()?;
            let rows = ultra
                .with_conn(|c| synapse_ultra::observe::session_timeline(c, &session, limit))?;
            if rows.is_empty() {
                println!("ultra: timeline — no events for session '{session}'");
                return Ok(());
            }
            println!("# session timeline: {session} ({} rows)", rows.len());
            for r in rows {
                let marker = if r.is_decision { "DECISION" } else { "event" };
                let uri = r.uri.as_deref().unwrap_or("-");
                let preview = r.content_preview.as_deref().unwrap_or("");
                let preview = if preview.len() > 80 {
                    &preview[..80]
                } else {
                    preview
                };
                println!(
                    "{}\t{}\t{}\t{}\t{}\t{}",
                    r.ts, marker, r.agent, r.kind, uri, preview
                );
            }
        }
        Cmd::Sessions { agent, limit } => {
            ultra.migrate()?;
            let rows = ultra
                .with_conn(|c| synapse_ultra::observe::list_sessions(c, agent.as_deref(), limit))?;
            if rows.is_empty() {
                println!("ultra: sessions — no sessions found");
                return Ok(());
            }
            println!("# sessions ({}):", rows.len());
            println!("session_id\tagent\tevents\tdecisions\tfirst_ts\tlast_ts\tcost_usd");
            for r in rows {
                println!(
                    "{}\t{}\t{}\t{}\t{}\t{}\t{:.4}",
                    r.session_id, r.agent, r.events, r.decisions, r.first_ts, r.last_ts, r.cost_usd
                );
            }
        }
        Cmd::Search { query, limit } => {
            ultra.migrate()?;
            let rows = ultra
                .with_conn(|c| synapse_ultra::events::search_events(c, &query, Some(limit)))?;
            if rows.is_empty() {
                println!("ultra: search — no hits for '{query}'");
                return Ok(());
            }
            println!("# ultra search: '{query}' ({} hits)", rows.len());
            for r in rows {
                let content = r.content.unwrap_or_default();
                let preview = if content.len() > 100 {
                    format!("{}…", &content[..100])
                } else {
                    content
                };
                println!(
                    "[{}] {} {} uri={} {}",
                    r.ts,
                    r.agent,
                    r.kind,
                    r.uri.unwrap_or_default(),
                    preview
                );
            }
        }
        Cmd::Health { json } => {
            ultra.migrate()?;
            let report = ultra.with_conn(synapse_ultra::ops::health_check)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                println!("# ultra health — {}", db.display());
                println!("overall: {}", if report.overall_ok { "OK" } else { "FAIL" });
                println!(
                    "db_size: {} bytes ({} pages × {} bytes)",
                    report.db_size_bytes, report.page_count, report.page_size
                );
                println!(
                    "journal_mode: {}  synchronous: {}  foreign_keys: {}",
                    report.journal_mode, report.synchronous, report.foreign_keys
                );
                println!("ultra_schema: v{}", report.ultra_schema_version);
                println!();
                for c in &report.checks {
                    let mark = if c.ok { "✓" } else { "✗" };
                    println!("  {mark} {:<24} {}", c.name, c.detail);
                }
            }
        }
        Cmd::Backup { out } => {
            ultra.migrate()?;
            let dest = out.unwrap_or_else(|| {
                dirs_next::home_dir()
                    .map(|h| h.join(".synapse").join("backups"))
                    .unwrap_or_else(|| std::path::PathBuf::from("backups"))
            });
            let report = synapse_ultra::ops::create_backup(&db, &dest)?;
            println!("# ultra backup");
            println!("  path:        {}", report.backup_path.display());
            println!("  original:    {} bytes", report.original_bytes);
            println!("  compressed:  {} bytes", report.compressed_bytes);
            println!(
                "  ratio:       {:.2} ({:.0}% compression)",
                report.compression_ratio,
                (1.0 - report.compression_ratio) * 100.0
            );
            println!("  sha256:      {}", report.sha256);
            println!("  ts:          {}", report.ts);
        }
        Cmd::Metrics { format } => {
            ultra.migrate()?;
            match format.as_str() {
                "json" => {
                    let j = ultra.with_conn(synapse_ultra::ops::metrics_json)?;
                    println!("{}", serde_json::to_string_pretty(&j)?);
                }
                _ => {
                    let p = ultra.with_conn(synapse_ultra::ops::prometheus)?;
                    print!("{}", p);
                }
            }
        }
        Cmd::Tags(cmd) => {
            ultra.migrate()?;
            ultra.with_conn(|c| -> Result<()> {
                synapse_ultra::tags::migrate(c)?;
                match cmd {
                    TagsCmd::Add {
                        name,
                        color,
                        description,
                    } => {
                        let id = synapse_ultra::tags::create_tag(
                            c,
                            &name,
                            color.as_deref(),
                            description.as_deref(),
                        )?;
                        println!("ultra: tag '{name}' → id={id}");
                    }
                    TagsCmd::List => {
                        let tags = synapse_ultra::tags::list_tags(c)?;
                        if tags.is_empty() {
                            println!("ultra: no tags yet");
                        } else {
                            println!("# tags ({})", tags.len());
                            println!("id\tname\tcolor\tdescription");
                            for t in tags {
                                println!(
                                    "{}\t{}\t{}\t{}",
                                    t.id,
                                    t.name,
                                    t.color.unwrap_or_default(),
                                    t.description.unwrap_or_default()
                                );
                            }
                        }
                    }
                    TagsCmd::Tag {
                        doc_id,
                        tag_name,
                        source,
                    } => {
                        synapse_ultra::tags::tag_doc(c, doc_id, &tag_name, &source)?;
                        println!("ultra: doc {doc_id} tagged '{tag_name}' (source={source})");
                    }
                    TagsCmd::Bulk {
                        tag_name,
                        ids,
                        source,
                    } => {
                        let doc_ids: Vec<i64> = ids
                            .split(',')
                            .filter_map(|s| s.trim().parse().ok())
                            .collect();
                        let n = synapse_ultra::tags::bulk_tag(c, &doc_ids, &tag_name, &source)?;
                        println!("ultra: bulk-tagged {n} docs with '{tag_name}'");
                    }
                    TagsCmd::Untag { doc_id, tag_name } => {
                        synapse_ultra::tags::untag_doc(c, doc_id, &tag_name)?;
                        println!("ultra: removed tag '{tag_name}' from doc {doc_id}");
                    }
                    TagsCmd::For { doc_id } => {
                        let tags = synapse_ultra::tags::tags_for_doc(c, doc_id)?;
                        if tags.is_empty() {
                            println!("ultra: doc {doc_id} has no tags");
                        } else {
                            println!("# tags for doc {doc_id} ({})", tags.len());
                            for t in tags {
                                println!("  {} (source={}, ts={})", t.tag_name, t.source, t.ts);
                            }
                        }
                    }
                    TagsCmd::Docs { tag_name } => {
                        let docs = synapse_ultra::tags::docs_for_tag(c, &tag_name)?;
                        if docs.is_empty() {
                            println!("ultra: no docs tagged '{tag_name}'");
                        } else {
                            println!("# docs with tag '{tag_name}' ({})", docs.len());
                            for d in docs {
                                println!("  doc_id={} source={} ts={}", d.doc_id, d.source, d.ts);
                            }
                        }
                    }
                    TagsCmd::Rule { keyword, tag_name } => {
                        let id = synapse_ultra::tags::add_rule(c, &keyword, &tag_name)?;
                        println!("ultra: rule {id} — keyword='{keyword}' → tag='{tag_name}'");
                    }
                    TagsCmd::Rules => {
                        let rules = synapse_ultra::tags::list_rules(c)?;
                        if rules.is_empty() {
                            println!("ultra: no auto-tag rules");
                        } else {
                            println!("# auto-tag rules ({})", rules.len());
                            println!("id\tkeyword\ttag\tenabled");
                            for r in rules {
                                println!("{}\t{}\t{}\t{}", r.id, r.keyword, r.tag_name, r.enabled);
                            }
                        }
                    }
                    TagsCmd::Merge { from, into } => {
                        let n = synapse_ultra::tags::merge_tags(c, &from, &into)?;
                        println!(
                            "ultra: merged '{from}' into '{into}' — {n} associations repointed"
                        );
                    }
                    TagsCmd::Cleanup => {
                        let n = synapse_ultra::tags::cleanup_orphans(c)?;
                        println!("ultra: removed {n} orphan tags");
                    }
                    TagsCmd::Stats => {
                        let s = synapse_ultra::tags::stats(c)?;
                        println!("# tag stats");
                        println!("  total_tags:         {}", s.total_tags);
                        println!("  total_associations: {}", s.total_associations);
                        println!("  total_rules:        {}", s.total_rules);
                        if !s.top_tags.is_empty() {
                            println!("  top_tags:");
                            for (name, count) in s.top_tags {
                                println!("    {name}: {count}");
                            }
                        }
                    }
                    TagsCmd::Export => {
                        let export = synapse_ultra::tags::export(c)?;
                        println!("{}", serde_json::to_string_pretty(&export)?);
                    }
                    TagsCmd::Import { path } => {
                        let content = std::fs::read_to_string(&path)?;
                        let data: synapse_ultra::tags::TagExport = serde_json::from_str(&content)?;
                        let n = synapse_ultra::tags::import(c, &data)?;
                        println!("ultra: imported {n} items from {}", path.display());
                    }
                }
                Ok(())
            })?;
        }
    }
    Ok(())
}

fn main() -> Result<()> {
    run()
}
