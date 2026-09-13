#!/usr/bin/env -S cargo +stable run -p synapse-pack --example savings_report --
//! Token-savings report: full pack vs delta pack vs cache-stable.
//!
//! Run: `cargo run -p synapse-pack --example savings_report`

use synapse_pack::{Candidate, Kind, PackOptions, estimate_tokens, pack, pack_delta, render};

fn make_candidates(n: usize, tokens_each: usize) -> Vec<Candidate> {
    let line = "alpha beta gamma delta epsilon zeta eta theta iota kappa lambda. ";
    let per_line = estimate_tokens(line);
    let lines = (tokens_each / per_line.max(1)).max(1);
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let mut text = String::new();
        for _ in 0..lines {
            text.push_str(line);
        }
        out.push(Candidate {
            id: i as i64,
            title: format!("fact-{i}"),
            text,
            score: 1.0 - (i as f32) * 0.01,
            kind: match i % 3 {
                0 => Kind::KnownFact,
                1 => Kind::Decision,
                _ => Kind::File,
            },
        });
    }
    out
}

fn main() {
    let cands = make_candidates(50, 200);
    let naive: usize = cands.iter().map(|c| estimate_tokens(&c.text)).sum();
    println!("=== Synapse-Pack Token-Savings Report ===\n");
    println!("Input: 50 candidates × ~200 tokens each = {naive} naive tokens\n");

    // 1. Full pack.
    let p_full = pack(
        cands.clone(),
        &PackOptions {
            budget_tokens: 4000,
            header_reserve: 64,
            ..PackOptions::default()
        },
    );
    println!(
        "FULL pack:    {:>5} tokens used  (savings {:>5.1}% vs naive)",
        p_full.used_tokens,
        p_full.savings_pct()
    );

    // 2. Delta pack — 40 of 50 already delivered.
    let prev_ids: Vec<i64> = (0..40).collect();
    let p_delta = pack_delta(
        cands.clone(),
        &PackOptions {
            budget_tokens: 4000,
            header_reserve: 64,
            prev_used_ids: prev_ids,
            cache_stable_order: false,
        },
    );
    println!(
        "DELTA pack:   {:>5} tokens used  (savings {:>5.1}% vs naive, {} ids skipped)",
        p_delta.used_tokens,
        p_delta.savings_pct(),
        p_delta.delta_skipped_ids.len()
    );

    // 3. Cache-stable pack — 10 prev-used ids lead the prefix.
    let prev_ids_cs: Vec<i64> = (0..10).collect();
    let p_cs = pack(
        cands.clone(),
        &PackOptions {
            budget_tokens: 4000,
            header_reserve: 64,
            prev_used_ids: prev_ids_cs,
            cache_stable_order: true,
        },
    );
    println!(
        "CACHE-STABLE: {:>5} tokens used  (savings {:>5.1}% vs naive, prefix-stable for prompt-cache)",
        p_cs.used_tokens,
        p_cs.savings_pct()
    );

    // 4. Delta + cache-stable combined (incremental loop with cache reuse).
    let p_combo = pack_delta(
        cands.clone(),
        &PackOptions {
            budget_tokens: 4000,
            header_reserve: 64,
            prev_used_ids: (0..40).collect(),
            cache_stable_order: true,
        },
    );
    println!(
        "DELTA+CS:     {:>5} tokens used  (savings {:>5.1}% vs naive, combined)",
        p_combo.used_tokens,
        p_combo.savings_pct()
    );

    println!("\n=== Render preview (delta pack, first 300 chars) ===");
    let out = render(&p_delta);
    let preview: String = out.chars().take(300).collect();
    println!("{preview}...");
}
