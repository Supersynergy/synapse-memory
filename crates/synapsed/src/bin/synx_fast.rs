use anyhow::{Context, Result, anyhow};
use serde_json::{Value, json};
use std::env;
use std::io::{self, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::Duration;

fn main() {
    if let Err(e) = run() {
        eprintln!("synx: {e}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let args: Vec<String> = env::args().skip(1).collect();
    let op = args.first().map(String::as_str).unwrap_or("ping");
    match op {
        "help" | "--help" | "-h" => print_help(),
        "doctor" => run_doctor()?,
        "ping" => print_response(&call(&json!({"op": "Ping"}), Duration::from_secs(2))?),
        "stats" => print_response(&call(&json!({"op": "Stats"}), Duration::from_secs(5))?),
        "sql" => {
            let query = parse_sql(&args[1..])?;
            print_response(&call(
                &json!({"op": "Sql", "args": {"query": query, "params": []}}),
                Duration::from_secs(30),
            )?);
        }
        "batch" => {
            let opts = parse_batch_opts(&args[1..])?;
            run_batch(opts)?;
        }
        "scoped" | "scope" => {
            let opts = parse_scoped_opts(&args[1..])?;
            let resp = call(&scoped_request(&opts), Duration::from_secs(10))?;
            print_hits(&resp)?;
        }
        "context" => {
            let opts = parse_context_opts(&args[1..])?;
            let resp = call(&scoped_request(&opts.search), Duration::from_secs(10))?;
            print_context(&opts, &resp)?;
        }
        "search" | "hybrid" | "find" | "vec" => {
            let mode = match op {
                "find" => "Lex",
                "vec" => "Vec",
                _ => "Hybrid",
            };
            let scoped = args[1..]
                .iter()
                .any(|arg| arg == "--scope" || arg == "-s" || arg == "--scope-key");
            let resp = if scoped {
                let mut scoped_args = vec![op.to_string()];
                scoped_args.extend(args[1..].iter().cloned());
                let opts = parse_scoped_opts(&scoped_args)?;
                call(&scoped_request(&opts), Duration::from_secs(10))?
            } else {
                let (query, limit) = parse_query_limit(&args[1..])?;
                call(
                    &json!({
                        "op": "Search",
                        "args": {
                            "mode": mode,
                            "q": query,
                            "limit": limit,
                            "embed_query": mode != "Lex"
                        }
                    }),
                    Duration::from_secs(10),
                )?
            };
            print_hits(&resp)?;
        }
        "put" => {
            let req = parse_put(&args[1..])?;
            print_response(&call(
                &json!({"op": "Put", "args": req}),
                Duration::from_secs(60),
            )?);
        }
        "put-batch" => {
            let batch_opts = parse_put_batch_opts(&args[1..])?;
            let batch = read_jsonl_batch(&batch_opts)?;
            eprintln!("batch {}...", batch.len());
            print_response(&call(
                &json!({"op": "PutBatch", "args": batch}),
                Duration::from_secs(600),
            )?);
        }
        other => return Err(anyhow!("unknown command `{other}`")),
    }
    Ok(())
}

fn print_help() {
    println!(
        "synx-fast: low-latency Synapse daemon CLI\n\n\
Usage:\n  \
synx-fast ping\n  \
synx-fast stats\n  \
synx-fast doctor\n  \
synx-fast find|hybrid|vec \"query\" [limit] [--scope PROJECT]\n  \
synx-fast scoped --scope PROJECT \"query\" [--scope-key scope] [--mode hybrid] [--limit 8]\n  \
synx-fast context --scope PROJECT \"query\" [--budget 900]\n  \
printf 'q1\\nq2\\n' | synx-fast batch hybrid --limit 8 [--scope PROJECT]\n  \
echo text | synx-fast put --title TITLE [--scope PROJECT]\n  \
cat items.jsonl | synx-fast put-batch [--scope PROJECT]\n\n\
Env:\n  SYNAPSE_SOCK   Unix socket path, default /tmp/synapse.sock\n\n\
Tip:\n  Use `doctor` first when hooks or Docker feel offline."
    );
}

fn parse_sql(args: &[String]) -> Result<String> {
    let query = if args.is_empty() {
        let mut s = String::new();
        io::stdin().read_to_string(&mut s)?;
        s
    } else {
        args.join(" ")
    };
    let query = query.trim().to_string();
    if query.is_empty() {
        return Err(anyhow!("missing SQL query"));
    }
    Ok(query)
}

fn socket_path() -> String {
    env::var("SYNAPSE_SOCK").unwrap_or_else(|_| "/tmp/synapse.sock".to_string())
}

fn call(req: &Value, timeout: Duration) -> Result<Value> {
    let mut stream = UnixStream::connect(socket_path()).context("daemon offline")?;
    stream.set_read_timeout(Some(timeout))?;
    stream.set_write_timeout(Some(timeout))?;
    send_on_stream(&mut stream, req)
}

fn send_on_stream(stream: &mut UnixStream, req: &Value) -> Result<Value> {
    let body = rmp_serde::to_vec_named(req)?;
    let len = u32::try_from(body.len()).context("request too large")?;
    stream.write_all(&len.to_le_bytes())?;
    stream.write_all(&body)?;

    let mut hdr = [0u8; 4];
    stream.read_exact(&mut hdr)?;
    let n = u32::from_le_bytes(hdr) as usize;
    let mut buf = vec![0u8; n];
    stream.read_exact(&mut buf)?;
    Ok(rmp_serde::from_slice(&buf)?)
}

struct BatchOpts {
    mode: &'static str,
    limit: usize,
    embed_query: bool,
    scope_key: Option<String>,
    scope_value: Option<String>,
}

#[derive(Clone)]
struct SearchOpts {
    mode: &'static str,
    query: String,
    limit: usize,
    embed_query: bool,
    scope_key: String,
    scope_value: String,
    candidate_limit: usize,
}

struct ContextOpts {
    search: SearchOpts,
    token_budget: usize,
    max_chars: usize,
    format: ContextFormat,
}

enum ContextFormat {
    Xml,
    Markdown,
}

struct PutBatchOpts {
    scope_key: Option<String>,
    scope_value: Option<String>,
}

fn parse_batch_opts(args: &[String]) -> Result<BatchOpts> {
    let mut mode = "Hybrid";
    let mut limit = 10usize;
    let mut i = 0usize;
    while i < args.len() {
        match args[i].as_str() {
            "--mode" | "-m" if i + 1 < args.len() => {
                mode = parse_search_mode(&args[i + 1])?;
                i += 2;
            }
            "--limit" | "-n" | "-k" if i + 1 < args.len() => {
                limit = args[i + 1].parse().context("invalid limit")?;
                i += 2;
            }
            s @ ("find" | "lex" | "hybrid" | "search" | "vec") => {
                mode = parse_search_mode(s)?;
                i += 1;
            }
            "--scope" | "-s" if i + 1 < args.len() => {
                i += 2;
            }
            "--scope-key" if i + 1 < args.len() => {
                i += 2;
            }
            s if s.chars().all(|c| c.is_ascii_digit()) => {
                limit = s.parse().context("invalid limit")?;
                i += 1;
            }
            other => return Err(anyhow!("unknown batch option `{other}`")),
        }
    }
    Ok(BatchOpts {
        mode,
        limit,
        embed_query: mode != "Lex",
        scope_key: option_value(args, "--scope-key"),
        scope_value: option_value(args, "--scope").or_else(|| option_value(args, "-s")),
    })
}

fn parse_scoped_opts(args: &[String]) -> Result<SearchOpts> {
    let mut mode = "Hybrid";
    let mut limit = 8usize;
    let mut candidate_limit = 64usize;
    let mut scope_key = "scope".to_string();
    let mut scope_value: Option<String> = None;
    let mut query_parts = Vec::new();
    let mut i = 0usize;
    while i < args.len() {
        match args[i].as_str() {
            "--mode" | "-m" if i + 1 < args.len() => {
                mode = parse_search_mode(&args[i + 1])?;
                i += 2;
            }
            "--limit" | "-n" | "-k" if i + 1 < args.len() => {
                limit = args[i + 1].parse().context("invalid limit")?;
                i += 2;
            }
            "--candidate-limit" if i + 1 < args.len() => {
                candidate_limit = args[i + 1].parse().context("invalid candidate limit")?;
                i += 2;
            }
            "--scope-key" if i + 1 < args.len() => {
                scope_key = args[i + 1].clone();
                i += 2;
            }
            "--scope" | "-s" if i + 1 < args.len() => {
                scope_value = Some(args[i + 1].clone());
                i += 2;
            }
            s @ ("find" | "lex" | "hybrid" | "search" | "vec") => {
                mode = parse_search_mode(s)?;
                i += 1;
            }
            s if s.chars().all(|c| c.is_ascii_digit()) => {
                limit = s.parse().context("invalid limit")?;
                i += 1;
            }
            s => {
                query_parts.push(s.to_string());
                i += 1;
            }
        }
    }

    let query = if query_parts.is_empty() {
        let mut s = String::new();
        io::stdin().read_to_string(&mut s)?;
        s.trim().to_string()
    } else {
        query_parts.join(" ")
    };
    if query.is_empty() {
        return Err(anyhow!("missing query"));
    }
    let scope_value = scope_value
        .or_else(|| env::var("SYNAPSE_SCOPE").ok())
        .ok_or_else(|| anyhow!("missing --scope (or SYNAPSE_SCOPE)"))?;

    Ok(SearchOpts {
        mode,
        query,
        limit,
        embed_query: mode != "Lex",
        scope_key,
        scope_value,
        candidate_limit,
    })
}

fn parse_context_opts(args: &[String]) -> Result<ContextOpts> {
    let mut rest = Vec::new();
    let mut token_budget = 900usize;
    let mut max_chars = 420usize;
    let mut format = ContextFormat::Xml;
    let mut i = 0usize;
    while i < args.len() {
        match args[i].as_str() {
            "--budget" | "--tokens" if i + 1 < args.len() => {
                token_budget = args[i + 1].parse().context("invalid token budget")?;
                i += 2;
            }
            "--max-chars" if i + 1 < args.len() => {
                max_chars = args[i + 1].parse().context("invalid max chars")?;
                i += 2;
            }
            "--markdown" | "--md" => {
                format = ContextFormat::Markdown;
                i += 1;
            }
            other => {
                rest.push(other.to_string());
                i += 1;
            }
        }
    }
    let mut search = parse_scoped_opts(&rest)?;
    search.limit = search.limit.max(1);
    Ok(ContextOpts {
        search,
        token_budget,
        max_chars,
        format,
    })
}

fn scoped_request(opts: &SearchOpts) -> Value {
    json!({
        "op": "SearchScoped",
        "args": {
            "mode": opts.mode,
            "q": opts.query,
            "limit": opts.limit,
            "embed_query": opts.embed_query,
            "scope_key": opts.scope_key,
            "scope_value": opts.scope_value,
            "candidate_limit": opts.candidate_limit
        }
    })
}

fn option_value(args: &[String], flag: &str) -> Option<String> {
    args.windows(2)
        .find(|pair| pair[0] == flag)
        .map(|pair| pair[1].clone())
}

fn parse_search_mode(s: &str) -> Result<&'static str> {
    match s.to_ascii_lowercase().as_str() {
        "find" | "lex" => Ok("Lex"),
        "vec" => Ok("Vec"),
        "hybrid" | "search" => Ok("Hybrid"),
        _ => Err(anyhow!("invalid search mode `{s}`")),
    }
}

fn run_batch(opts: BatchOpts) -> Result<()> {
    let mut input = String::new();
    io::stdin().read_to_string(&mut input)?;
    let queries: Vec<&str> = input
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect();
    if queries.is_empty() {
        return Ok(());
    }

    let timeout = Duration::from_secs(60);
    let mut stream = UnixStream::connect(socket_path()).context("daemon offline")?;
    stream.set_read_timeout(Some(timeout))?;
    stream.set_write_timeout(Some(timeout))?;

    let stdout = io::stdout();
    let mut out = io::BufWriter::new(stdout.lock());
    let resp = if let Some(scope) = opts.scope_value.as_deref() {
        let mut batches = Vec::with_capacity(queries.len());
        for q in &queries {
            batches.push(send_on_stream(
                &mut stream,
                &json!({
                    "op": "SearchScoped",
                    "args": {
                        "mode": opts.mode,
                        "q": q,
                        "limit": opts.limit,
                        "embed_query": opts.embed_query,
                        "scope_key": opts.scope_key.as_deref().unwrap_or("scope"),
                        "scope_value": scope
                    }
                }),
            )?);
        }
        json!({"ScopedBatchHits": batches})
    } else {
        send_on_stream(
            &mut stream,
            &json!({
                "op": "BatchSearch",
                "args": {
                    "queries": queries.iter().map(|q| json!({
                        "mode": opts.mode,
                        "q": q,
                        "limit": opts.limit,
                        "embed_query": opts.embed_query
                    })).collect::<Vec<_>>()
                }
            }),
        )?
    };

    if let Some(batches) = resp.get("BatchHits").and_then(Value::as_array) {
        for (q, hits) in queries.iter().zip(batches.iter()) {
            serde_json::to_writer(&mut out, &json!({"q": q, "response": {"Hits": hits}}))?;
            out.write_all(b"\n")?;
        }
    } else if let Some(batches) = resp.get("ScopedBatchHits").and_then(Value::as_array) {
        for (q, response) in queries.iter().zip(batches.iter()) {
            serde_json::to_writer(&mut out, &json!({"q": q, "response": response}))?;
            out.write_all(b"\n")?;
        }
    } else {
        for q in queries {
            serde_json::to_writer(&mut out, &json!({"q": q, "response": resp}))?;
            out.write_all(b"\n")?;
        }
    }
    out.flush()?;
    Ok(())
}

fn parse_query_limit(args: &[String]) -> Result<(String, usize)> {
    let mut query = None;
    let mut limit = 10usize;
    let mut i = 0usize;
    while i < args.len() {
        match args[i].as_str() {
            "--limit" | "-n" | "-k" if i + 1 < args.len() => {
                limit = args[i + 1].parse().context("invalid limit")?;
                i += 2;
            }
            s if query.is_none() => {
                query = Some(s.to_string());
                i += 1;
            }
            s if s.chars().all(|c| c.is_ascii_digit()) => {
                limit = s.parse().context("invalid limit")?;
                i += 1;
            }
            _ => i += 1,
        }
    }
    Ok((query.ok_or_else(|| anyhow!("missing query"))?, limit))
}

fn parse_put(args: &[String]) -> Result<Value> {
    let mut title: Option<String> = None;
    let mut uri: Option<String> = None;
    let mut meta: Option<Value> = None;
    let mut embed = true;
    let mut scope_key = "scope".to_string();
    let mut scope_value: Option<String> = None;
    let mut text_parts = Vec::new();
    let mut i = 0usize;
    while i < args.len() {
        match args[i].as_str() {
            "--title" if i + 1 < args.len() => {
                title = Some(args[i + 1].clone());
                i += 2;
            }
            "--uri" if i + 1 < args.len() => {
                uri = Some(args[i + 1].clone());
                i += 2;
            }
            "--meta" if i + 1 < args.len() => {
                meta = Some(serde_json::from_str(&args[i + 1]).context("invalid --meta JSON")?);
                i += 2;
            }
            "--scope-key" if i + 1 < args.len() => {
                scope_key = args[i + 1].clone();
                i += 2;
            }
            "--scope" | "-s" if i + 1 < args.len() => {
                scope_value = Some(args[i + 1].clone());
                i += 2;
            }
            "--no-embed" => {
                embed = false;
                i += 1;
            }
            s => {
                text_parts.push(s.to_string());
                i += 1;
            }
        }
    }
    let text = if text_parts.is_empty() {
        let mut s = String::new();
        io::stdin().read_to_string(&mut s)?;
        s
    } else {
        text_parts.join(" ")
    };
    let meta = merge_scope_meta(meta, scope_key.as_str(), scope_value.as_deref())?;
    Ok(json!({
        "title": title,
        "uri": uri,
        "text": text,
        "meta": meta,
        "embed": embed,
        "embedding": null
    }))
}

fn parse_put_batch_opts(args: &[String]) -> Result<PutBatchOpts> {
    let mut scope_key = None;
    let mut scope_value = None;
    let mut i = 0usize;
    while i < args.len() {
        match args[i].as_str() {
            "--scope-key" if i + 1 < args.len() => {
                scope_key = Some(args[i + 1].clone());
                i += 2;
            }
            "--scope" | "-s" if i + 1 < args.len() => {
                scope_value = Some(args[i + 1].clone());
                i += 2;
            }
            other => return Err(anyhow!("unknown put-batch option `{other}`")),
        }
    }
    Ok(PutBatchOpts {
        scope_key,
        scope_value,
    })
}

fn merge_scope_meta(meta: Option<Value>, key: &str, value: Option<&str>) -> Result<Value> {
    if value.is_none() {
        return Ok(meta.unwrap_or(Value::Null));
    }
    let mut meta = match meta {
        Some(Value::Object(obj)) => obj,
        Some(Value::Null) | None => serde_json::Map::new(),
        Some(_) => return Err(anyhow!("meta must be a JSON object when --scope is used")),
    };
    if let Some(value) = value {
        meta.insert(key.to_string(), Value::String(value.to_string()));
    }
    Ok(Value::Object(meta))
}

fn read_jsonl_batch(opts: &PutBatchOpts) -> Result<Vec<Value>> {
    let mut input = String::new();
    io::stdin().read_to_string(&mut input)?;
    let mut out = Vec::new();
    for (line_no, line) in input.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let v: Value = serde_json::from_str(trimmed)
            .with_context(|| format!("invalid JSONL at line {}", line_no + 1))?;
        let meta = merge_scope_meta(
            v.get("meta").cloned(),
            opts.scope_key.as_deref().unwrap_or("scope"),
            opts.scope_value.as_deref(),
        )?;
        out.push(json!({
            "title": v.get("title").cloned().unwrap_or(Value::Null),
            "uri": v.get("uri").cloned().unwrap_or(Value::Null),
            "text": v.get("text").or_else(|| v.get("content")).cloned().unwrap_or(Value::String(String::new())),
            "meta": meta,
            "embed": v.get("embed").and_then(Value::as_bool).unwrap_or(true),
            "embedding": v.get("embedding").cloned().unwrap_or(Value::Null)
        }));
    }
    Ok(out)
}

fn run_doctor() -> Result<()> {
    let sock = socket_path();
    println!("synx-fast doctor");
    println!("socket: {sock}");
    println!(
        "socket_exists: {}",
        if Path::new(&sock).exists() {
            "yes"
        } else {
            "no"
        }
    );
    match call(&json!({"op": "Ping"}), Duration::from_secs(2)) {
        Ok(resp) => println!("ping: {}", compact_json(&resp)),
        Err(err) => {
            println!("ping: FAIL ({err})");
            println!("fix: start synapsed with `synapsed --file .synapse/brain.db --sock {sock}`");
            return Ok(());
        }
    }
    match call(&json!({"op": "Stats"}), Duration::from_secs(5)) {
        Ok(resp) => println!("stats: {}", compact_json(&resp)),
        Err(err) => println!("stats: FAIL ({err})"),
    }
    println!(
        "env: SYNAPSE_SOCK={}",
        env::var("SYNAPSE_SOCK").unwrap_or_else(|_| "(unset)".to_string())
    );
    println!("status: OK");
    Ok(())
}

fn compact_json(resp: &Value) -> String {
    match resp {
        Value::String(s) => s.to_string(),
        other => serde_json::to_string(other).unwrap_or_else(|_| other.to_string()),
    }
}

fn print_context(opts: &ContextOpts, resp: &Value) -> Result<()> {
    if let Some(err) = resp.get("Err").and_then(Value::as_str) {
        return Err(anyhow!(err.to_string()));
    }
    let hits = resp
        .get("Hits")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("SearchScoped response did not contain Hits"))?;
    let mut used = 0usize;
    let mut selected = Vec::new();
    for hit in hits {
        let text = hit.get("text").and_then(Value::as_str).unwrap_or_default();
        let snippet = truncate(&normalize_ws(text), opts.max_chars);
        let cost = estimate_tokens(&snippet) + 24;
        if used + cost > opts.token_budget && !selected.is_empty() {
            break;
        }
        used += cost;
        selected.push((hit, snippet));
    }
    match opts.format {
        ContextFormat::Markdown => {
            println!(
                "# Synapse Context\n\nquery: `{}`  \nscope: `{}`  \nbudget: {} tokens, used: ~{}\n",
                opts.search.query, opts.search.scope_value, opts.token_budget, used
            );
            for (idx, (hit, snippet)) in selected.iter().enumerate() {
                let title = hit
                    .get("title")
                    .and_then(Value::as_str)
                    .unwrap_or("untitled");
                let score = hit.get("score").and_then(Value::as_f64).unwrap_or_default();
                println!(
                    "{}. `{}` score={:.6}\n   {}",
                    idx + 1,
                    title,
                    score,
                    snippet
                );
            }
        }
        ContextFormat::Xml => {
            println!(
                "<synapse_context query=\"{}\" scope_key=\"{}\" scope=\"{}\" budget_tokens=\"{}\" used_tokens=\"{}\">",
                xml_escape(&opts.search.query),
                xml_escape(&opts.search.scope_key),
                xml_escape(&opts.search.scope_value),
                opts.token_budget,
                used
            );
            for (idx, (hit, snippet)) in selected.iter().enumerate() {
                let id = hit.get("id").and_then(Value::as_i64).unwrap_or_default();
                let title = hit
                    .get("title")
                    .and_then(Value::as_str)
                    .unwrap_or("untitled");
                let score = hit.get("score").and_then(Value::as_f64).unwrap_or_default();
                println!(
                    "  <memory rank=\"{}\" id=\"{}\" score=\"{:.6}\" title=\"{}\">{}</memory>",
                    idx + 1,
                    id,
                    score,
                    xml_escape(title),
                    xml_escape(snippet)
                );
            }
            println!("</synapse_context>");
        }
    }
    Ok(())
}

fn normalize_ws(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn estimate_tokens(text: &str) -> usize {
    text.len().div_ceil(4).max(1)
}

fn xml_escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn print_response(resp: &Value) {
    match resp {
        Value::String(s) => println!("{s}"),
        other => println!(
            "{}",
            serde_json::to_string(other).unwrap_or_else(|_| other.to_string())
        ),
    }
}

fn print_hits(resp: &Value) -> Result<()> {
    if let Some(err) = resp.get("Err").and_then(Value::as_str) {
        return Err(anyhow!(err.to_string()));
    }
    let Some(hits) = resp.get("Hits").and_then(Value::as_array) else {
        print_response(resp);
        return Ok(());
    };
    for h in hits {
        let id = h.get("id").and_then(Value::as_i64).unwrap_or_default();
        let title = h
            .get("title")
            .and_then(Value::as_str)
            .or_else(|| h.get("uri").and_then(Value::as_str))
            .map(str::to_string)
            .unwrap_or_else(|| format!("id:{id}"));
        let score = h.get("score").and_then(Value::as_f64).unwrap_or_default();
        let snippet = h
            .get("text")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .replace('\n', " ");
        println!(
            "{score:.3}\t{}\t{}",
            truncate(&title, 80),
            truncate(&snippet, 140)
        );
    }
    Ok(())
}

fn truncate(s: &str, max_chars: usize) -> String {
    let mut out = String::new();
    for c in s.chars().take(max_chars) {
        out.push(c);
    }
    out
}
