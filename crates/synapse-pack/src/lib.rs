//! synapse-pack — token-budget context packer.
//!
//! Turns a set of retrieved candidates into the minimal **verbatim** STATE that
//! fits a token budget, then orders it to beat lost-in-the-middle.
//!
//! Design (research-grounded, see docs/SPEC-ctxos-v2.md):
//! - **Deletion-based, not summarization** — every surviving line is character-for-character
//!   from the input. Zero hallucination, file-paths / error-strings / line-numbers survive
//!   exactly (Morph Compact 98% verbatim; summarization multi-session retention only 37%).
//! - **Adaptive per-kind tiers** (ACON) — known-fact/decision kept rich, file/chat dumps cut hard.
//! - **Near-dup collapse** (SimHash) — never pay twice for the same fact.
//! - **Serial-position order** — best first, 2nd-best last (LongLLMLingua lost-in-the-middle).
//! - **Hard budget** — output token estimate never exceeds `budget_tokens`.
//!
//! Pure: no IO, no async, deterministic. The retrieval + daemon wiring lives in the caller.

use serde::{Deserialize, Serialize};

/// Source class of a candidate. Drives the trust prior and the floor tier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Kind {
    /// Curated verified fact — highest trust, never compressed below `Signatures`.
    KnownFact,
    /// Durable decision / ADR — high trust, floor `Signatures`.
    Decision,
    /// Source-file excerpt — may be cut to `OneLine`.
    File,
    /// Chat / session transcript — may be cut to `OneLine`.
    Chat,
    /// Anything else.
    Other,
}

impl Kind {
    /// Map a free-text kind string (from doc meta) to a [`Kind`].
    pub fn from_meta(s: &str) -> Self {
        let s = s.to_ascii_lowercase();
        if s.contains("known-fact") || s.contains("known_fact") || s == "fact" {
            Kind::KnownFact
        } else if s.contains("decision") || s.contains("adr") {
            Kind::Decision
        } else if s.contains("file") || s.contains("code") || s.contains("source") {
            Kind::File
        } else if s.contains("chat") || s.contains("session") || s.contains("transcript") {
            Kind::Chat
        } else {
            Kind::Other
        }
    }

    /// Lowest tier this kind may be compressed to (never below).
    fn floor(self) -> Tier {
        match self {
            Kind::KnownFact | Kind::Decision => Tier::Signatures,
            Kind::File | Kind::Chat | Kind::Other => Tier::OneLine,
        }
    }

    /// Static trust prior added to the retrieval score (known-fact wins ties).
    fn trust_prior(self) -> f32 {
        match self {
            Kind::KnownFact => 0.05,
            Kind::Decision => 0.03,
            Kind::File => 0.0,
            Kind::Chat => -0.01,
            Kind::Other => 0.0,
        }
    }
}

/// Compression tier — strictly deletion-based (output is a subset of input lines).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Tier {
    /// T3: title + single highest-signal line.
    OneLine = 0,
    /// T2: only lines carrying a fact (numbers, `=`, `->`, paths, CAPS terms).
    FactDelta = 1,
    /// T1: headings, first/last line, code fences, every line with a number/path/identifier.
    Signatures = 2,
    /// T0: verbatim, unchanged.
    Full = 3,
}

impl Tier {
    /// All tiers from richest to leanest. Greedy picks the richest that fits the
    /// remaining budget, which maximizes retained information per candidate.
    fn ladder() -> [Tier; 4] {
        [Tier::Full, Tier::Signatures, Tier::FactDelta, Tier::OneLine]
    }
}

/// A retrieved candidate before packing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Candidate {
    pub id: i64,
    #[serde(default)]
    pub title: String,
    pub text: String,
    pub score: f32,
    pub kind: Kind,
}

/// One block in the final pack.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PackedBlock {
    pub id: i64,
    pub kind: Kind,
    pub tier: Tier,
    pub title: String,
    pub text: String,
    pub tokens: usize,
}

/// Result of packing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Pack {
    pub blocks: Vec<PackedBlock>,
    pub used_tokens: usize,
    pub budget_tokens: usize,
    /// Candidate ids that did not fit at any tier.
    pub dropped_ids: Vec<i64>,
    /// Candidate ids removed as near-duplicates.
    pub deduped_ids: Vec<i64>,
    /// Σ tokens if every candidate were included verbatim (the naive baseline).
    pub naive_tokens: usize,
}

impl Pack {
    /// Tokens saved vs naive full-recall, as a 0–100 percentage.
    pub fn savings_pct(&self) -> f32 {
        if self.naive_tokens == 0 {
            return 0.0;
        }
        let saved = self.naive_tokens.saturating_sub(self.used_tokens);
        ((saved as f32 / self.naive_tokens as f32) * 1000.0).round() / 10.0
    }
}

/// Options for [`pack`].
#[derive(Debug, Clone)]
pub struct PackOptions {
    pub budget_tokens: usize,
    /// Tokens reserved for the rendered header / STATE card.
    pub header_reserve: usize,
}

impl Default for PackOptions {
    fn default() -> Self {
        Self {
            budget_tokens: 4000,
            header_reserve: 48,
        }
    }
}

/// Estimate token count of `text`.
///
/// A flat chars/4 ratio undercounts for code and non-English text: BPE
/// tokenizers split punctuation/symbol runs and long/compound identifiers
/// into multiple tokens, and commonly spend one or more tokens per CJK
/// codepoint rather than per 4 chars. This counts whitespace-delimited word
/// runs, punctuation/symbol runs, and CJK codepoints separately, then takes
/// the max against a chars/3 floor (for dense separator-free text like
/// base64/minified code/long paths). It is deliberately biased to
/// over-estimate: the packer's `budget_tokens` guarantee only holds if this
/// never reports fewer tokens than a real tokenizer would.
pub fn estimate_tokens(text: &str) -> usize {
    if text.is_empty() {
        return 0;
    }

    let mut chars = 0usize;
    let mut words = 0usize;
    let mut punct_runs = 0usize;
    let mut cjk = 0usize;
    let mut in_word = false;
    let mut in_punct = false;

    for c in text.chars() {
        chars += 1;
        if is_cjk(c) {
            cjk += 1;
            in_word = false;
            in_punct = false;
        } else if c.is_whitespace() {
            in_word = false;
            in_punct = false;
        } else if c.is_alphanumeric() || c == '_' {
            if !in_word {
                words += 1;
            }
            in_word = true;
            in_punct = false;
        } else {
            // punctuation / symbol (BPE tokenizers usually spend a token per run)
            if !in_punct {
                punct_runs += 1;
            }
            in_punct = true;
            in_word = false;
        }
    }

    // Words split into ~1.3 subword tokens on average (long identifiers,
    // plurals/suffixes); CJK codepoints run ~2 tokens/char in common BPE
    // vocabs (e.g. cl100k-style), leaning high on purpose.
    let word_tokens = (words * 13).div_ceil(10);
    let structural = word_tokens + punct_runs + cjk * 2;

    // Floor against a conservative chars/3 ratio so dense, separator-free
    // text (minified code, base64, long paths) can never be under-counted.
    let char_floor = chars.div_ceil(3);

    structural.max(char_floor).max(1)
}

/// True for common CJK ranges (Han, Hiragana/Katakana, Hangul syllables).
fn is_cjk(c: char) -> bool {
    matches!(c as u32,
        0x4E00..=0x9FFF   // CJK Unified Ideographs
        | 0x3400..=0x4DBF // CJK Extension A
        | 0x3040..=0x30FF // Hiragana + Katakana
        | 0xAC00..=0xD7A3 // Hangul syllables
        | 0xF900..=0xFAFF // CJK Compatibility Ideographs
    )
}

/// Pack candidates into the minimal verbatim STATE within budget.
///
/// Pipeline: trust-adjust → dedup → multiple-choice knapsack over tiers → serial-position order.
/// Guarantees `result.used_tokens <= options.budget_tokens`.
pub fn pack(mut cands: Vec<Candidate>, options: &PackOptions) -> Pack {
    let budget = options.budget_tokens;
    let naive_tokens: usize = cands.iter().map(|c| estimate_tokens(&c.text)).sum();

    // 1. Trust-adjust score, then sort best-first (stable on id for determinism).
    for c in &mut cands {
        c.score += c.kind.trust_prior();
    }
    cands.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.id.cmp(&b.id))
    });

    // 2. Near-dup collapse via SimHash (keep the higher-scored of a dup pair).
    let mut kept: Vec<Candidate> = Vec::with_capacity(cands.len());
    let mut kept_hashes: Vec<u64> = Vec::with_capacity(cands.len());
    let mut deduped_ids = Vec::new();
    for c in cands {
        let h = simhash(&c.text);
        if kept_hashes.iter().any(|&k| hamming(k, h) <= 3) {
            deduped_ids.push(c.id);
        } else {
            kept_hashes.push(h);
            kept.push(c);
        }
    }

    // 3. Multiple-choice knapsack: greedy by score, pick richest tier that fits.
    //    Header reserve scales down on small budgets so a tiny budget still fits content.
    let header = options.header_reserve.min(budget / 4);
    let avail = budget.saturating_sub(header);
    let mut used = 0usize;
    let mut chosen: Vec<PackedBlock> = Vec::new();
    let mut dropped_ids = Vec::new();
    for c in &kept {
        let floor = c.kind.floor();
        let mut placed = false;
        for tier in Tier::ladder() {
            if (tier as u8) < (floor as u8) {
                continue; // respect per-kind floor
            }
            let body = compress(&c.text, tier);
            let block_text = render_block_text(&c.title, &body);
            let toks = estimate_tokens(&block_text);
            if used + toks <= avail {
                used += toks;
                chosen.push(PackedBlock {
                    id: c.id,
                    kind: c.kind,
                    tier,
                    title: c.title.clone(),
                    text: body,
                    tokens: toks,
                });
                placed = true;
                break;
            }
        }
        if !placed {
            dropped_ids.push(c.id);
        }
    }

    // Min-one guarantee: if nothing fit but candidates exist, force the top one at
    // OneLine (floor bypassed only here) as long as a single line fits the full budget.
    if chosen.is_empty()
        && let Some(top) = kept.first()
    {
        let body = compress(&top.text, Tier::OneLine);
        let block_text = render_block_text(&top.title, &body);
        let toks = estimate_tokens(&block_text);
        if toks <= budget {
            used = toks;
            chosen.push(PackedBlock {
                id: top.id,
                kind: top.kind,
                tier: Tier::OneLine,
                title: top.title.clone(),
                text: body,
                tokens: toks,
            });
            dropped_ids.retain(|&d| d != top.id);
        }
    }

    // 4. Serial-position order: rank0 first, rank1 last, rest (desc) in the middle.
    let blocks = serial_position_order(chosen);

    Pack {
        blocks,
        used_tokens: used,
        budget_tokens: budget,
        dropped_ids,
        deduped_ids,
        naive_tokens,
    }
}

/// Render the pack to a single string: STATE-card header + ordered blocks.
pub fn render(pack: &Pack) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "STATE [{} facts · {} dropped · {} deduped · {}/{} tok · {}% saved]\n",
        pack.blocks.len(),
        pack.dropped_ids.len(),
        pack.deduped_ids.len(),
        pack.used_tokens,
        pack.budget_tokens,
        pack.savings_pct(),
    ));
    for b in &pack.blocks {
        out.push_str(&format!("\n[{}|{}|{:?}] ", b.id, kind_tag(b.kind), b.tier));
        if !b.title.is_empty() {
            out.push_str(&b.title);
            out.push('\n');
        }
        out.push_str(&b.text);
        out.push('\n');
    }
    out
}

/// Canonical short tag for a kind. Single source of truth shared with callers
/// (e.g. synapse-mcp) so the same kind never gets two different labels.
pub fn kind_tag(k: Kind) -> &'static str {
    match k {
        Kind::KnownFact => "known-fact",
        Kind::Decision => "decision",
        Kind::File => "file",
        Kind::Chat => "chat",
        Kind::Other => "other",
    }
}

fn render_block_text(title: &str, body: &str) -> String {
    if title.is_empty() {
        body.to_string()
    } else {
        format!("{title}\n{body}")
    }
}

/// Reorder blocks so the strongest is first and the second-strongest is last.
/// Input is assumed score-descending. Empty / single / pair handled as identity-ish.
fn serial_position_order(blocks: Vec<PackedBlock>) -> Vec<PackedBlock> {
    let n = blocks.len();
    if n <= 2 {
        return blocks;
    }
    let mut it = blocks.into_iter();
    let first = it.next().unwrap();
    let last = it.next().unwrap();
    let middle: Vec<PackedBlock> = it.collect();
    let mut out = Vec::with_capacity(n);
    out.push(first);
    out.extend(middle);
    out.push(last);
    out
}

/// Deletion-based compression: return a subset of `text`'s lines for the given tier.
fn compress(text: &str, tier: Tier) -> String {
    match tier {
        Tier::Full => text.to_string(),
        Tier::Signatures => keep_lines(text, signature_line),
        Tier::FactDelta => keep_lines(text, fact_line),
        Tier::OneLine => one_line(text),
    }
}

fn keep_lines(text: &str, pred: impl Fn(&str, usize, usize) -> bool) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let n = lines.len();
    let kept: Vec<&str> = lines
        .iter()
        .enumerate()
        .filter(|(i, l)| pred(l, *i, n))
        .map(|(_, l)| *l)
        .collect();
    if kept.is_empty() {
        // never return nothing for a non-empty doc — fall back to first non-blank line
        first_nonblank(text)
    } else {
        kept.join("\n")
    }
}

/// A line worth keeping at the `Signatures` tier.
fn signature_line(line: &str, idx: usize, n: usize) -> bool {
    let t = line.trim();
    if t.is_empty() {
        return false;
    }
    idx == 0
        || idx + 1 == n
        || t.starts_with('#')
        || t.starts_with("```")
        || t.starts_with("- ")
        || t.starts_with("* ")
        || t.starts_with("| ")
        || fact_line(line, idx, n)
}

/// A line carrying a fact: a number, assignment, arrow, path, colon-key, or CAPS term.
fn fact_line(line: &str, _idx: usize, _n: usize) -> bool {
    let t = line.trim();
    if t.is_empty() {
        return false;
    }
    let has_digit = t.chars().any(|c| c.is_ascii_digit());
    let has_op = t.contains('=') || t.contains("->") || t.contains("=>") || t.contains(':');
    let has_path = t.contains('/') && t.chars().any(|c| c == '.' || c == '_');
    let has_caps_term = t
        .split(|c: char| !c.is_ascii_alphanumeric() && c != '_')
        .any(|w| w.len() >= 2 && w.chars().all(|c| c.is_ascii_uppercase() || c == '_'));
    has_digit || has_op || has_path || has_caps_term
}

fn one_line(text: &str) -> String {
    // prefer the highest-signal line; fall back to the first non-blank.
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .max_by_key(|l| line_signal(l))
        .map(str::to_string)
        .unwrap_or_default()
}

/// Cheap signal score for a line: facts + length, capped.
fn line_signal(l: &str) -> usize {
    let digits = l.chars().filter(|c| c.is_ascii_digit()).count();
    let ops = l.matches('=').count() + l.matches("->").count() + l.matches(':').count();
    digits * 3 + ops * 4 + l.len().min(120) / 20
}

fn first_nonblank(text: &str) -> String {
    text.lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("")
        .to_string()
}

/// 64-bit SimHash over whitespace tokens (lowercased).
fn simhash(text: &str) -> u64 {
    let mut v = [0i32; 64];
    let mut any = false;
    for tok in text.split_whitespace() {
        any = true;
        let h = fnv1a(tok.to_ascii_lowercase().as_bytes());
        for (i, slot) in v.iter_mut().enumerate() {
            if (h >> i) & 1 == 1 {
                *slot += 1;
            } else {
                *slot -= 1;
            }
        }
    }
    if !any {
        return 0;
    }
    let mut out = 0u64;
    for (i, &slot) in v.iter().enumerate() {
        if slot > 0 {
            out |= 1u64 << i;
        }
    }
    out
}

fn hamming(a: u64, b: u64) -> u32 {
    (a ^ b).count_ones()
}

fn fnv1a(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cand(id: i64, text: &str, score: f32, kind: Kind) -> Candidate {
        Candidate {
            id,
            title: String::new(),
            text: text.to_string(),
            score,
            kind,
        }
    }

    #[test]
    fn budget_is_never_exceeded() {
        let cands = vec![
            cand(1, &"alpha beta gamma delta. ".repeat(50), 0.9, Kind::File),
            cand(2, &"epsilon zeta eta theta. ".repeat(50), 0.8, Kind::File),
            cand(3, &"iota kappa lambda mu. ".repeat(50), 0.7, Kind::Chat),
        ];
        let opts = PackOptions {
            budget_tokens: 120,
            header_reserve: 16,
        };
        let p = pack(cands, &opts);
        assert!(
            p.used_tokens <= opts.budget_tokens,
            "used {} > budget {}",
            p.used_tokens,
            opts.budget_tokens
        );
    }

    #[test]
    fn verbatim_preserves_file_paths_and_numbers() {
        let text = "intro prose line\nsrc/api/webhooks/stripe.ts:98 handler\nError: ECONNREFUSED 127.0.0.1:5432\nmore prose";
        let cands = vec![cand(1, text, 0.9, Kind::KnownFact)];
        let p = pack(
            cands,
            &PackOptions {
                budget_tokens: 1000,
                header_reserve: 16,
            },
        );
        let out = render(&p);
        assert!(
            out.contains("src/api/webhooks/stripe.ts:98"),
            "path must survive verbatim:\n{out}"
        );
        assert!(
            out.contains("ECONNREFUSED 127.0.0.1:5432"),
            "error string must survive verbatim:\n{out}"
        );
    }

    #[test]
    fn near_duplicates_are_collapsed() {
        let a = "the quick brown fox jumps over the lazy dog every single morning";
        let b = "the quick brown fox jumps over the lazy dog every single evening";
        let cands = vec![cand(1, a, 0.9, Kind::Other), cand(2, b, 0.8, Kind::Other)];
        let p = pack(cands, &PackOptions::default());
        assert_eq!(p.deduped_ids, vec![2], "second near-dup should be dropped");
        assert_eq!(p.blocks.len(), 1);
    }

    #[test]
    fn known_fact_never_below_signatures() {
        // tiny budget forces compression, but known-fact may not drop to OneLine
        let text = "# Heading\nplain narrative filler one\nvalue = 42\nplain narrative filler two";
        let cands = vec![cand(1, text, 0.9, Kind::KnownFact)];
        let p = pack(
            cands,
            &PackOptions {
                budget_tokens: 40,
                header_reserve: 8,
            },
        );
        if let Some(b) = p.blocks.first() {
            assert!(
                b.tier >= Tier::Signatures,
                "known-fact tier {:?} below floor",
                b.tier
            );
        }
    }

    #[test]
    fn fact_delta_keeps_numbers_drops_prose() {
        let text = "this is pure narrative prose with no signal\nlatency = 8ms\njust talking here\nthroughput -> 231000 rows/s";
        let body = compress(text, Tier::FactDelta);
        assert!(body.contains("latency = 8ms"));
        assert!(body.contains("throughput -> 231000 rows/s"));
        assert!(!body.contains("pure narrative prose"));
    }

    #[test]
    fn serial_position_puts_best_first_second_last() {
        // big budget, all Full, 3 distinct docs → check ordering by id proxy
        let cands = vec![
            cand(10, "aaa one", 0.9, Kind::Other),
            cand(20, "bbb two", 0.8, Kind::Other),
            cand(30, "ccc three", 0.7, Kind::Other),
        ];
        let p = pack(
            cands,
            &PackOptions {
                budget_tokens: 1000,
                header_reserve: 8,
            },
        );
        assert_eq!(p.blocks.len(), 3);
        assert_eq!(p.blocks[0].id, 10, "best first");
        assert_eq!(p.blocks[2].id, 20, "second-best last");
        assert_eq!(p.blocks[1].id, 30, "rest middle");
    }

    #[test]
    fn tiny_budget_still_returns_top_one() {
        // budget too small for full/signatures, but one short line must survive
        let cands = vec![
            cand(
                1,
                "lat = 8ms\nlots of extra prose here that will not fit at all in budget",
                0.9,
                Kind::File,
            ),
            cand(
                2,
                "other doc with different words entirely",
                0.5,
                Kind::File,
            ),
        ];
        let p = pack(
            cands,
            &PackOptions {
                budget_tokens: 24,
                header_reserve: 64,
            },
        );
        assert!(
            !p.blocks.is_empty(),
            "tiny budget must still return the top fact"
        );
        assert_eq!(
            p.blocks[0].id, 1,
            "top-scored candidate wins the single slot"
        );
        assert!(p.used_tokens <= 24, "still within budget");
    }

    #[test]
    fn empty_input_is_safe() {
        let p = pack(vec![], &PackOptions::default());
        assert_eq!(p.blocks.len(), 0);
        assert_eq!(p.used_tokens, 0);
        assert_eq!(p.savings_pct(), 0.0);
    }

    #[test]
    fn estimate_tokens_empty_is_zero() {
        assert_eq!(estimate_tokens(""), 0);
    }

    #[test]
    fn estimate_tokens_never_under_naive_baseline() {
        // Punctuation-dense, low-word-length text is exactly where chars/4
        // undercounts worst (each `.`-separated symbol is its own BPE token).
        let dense = "a.b.c.d.e.f.g.h.i.j.k.l.m.n.o.p";
        let naive_chars_over_4 = dense.len().div_ceil(4);
        let est = estimate_tokens(dense);
        assert!(
            est >= naive_chars_over_4,
            "estimate {est} must be >= naive chars/4 baseline {naive_chars_over_4}"
        );
    }

    #[test]
    fn estimate_tokens_code_sample_is_conservative() {
        let code = "fn add(a: i32, b: i32) -> i32 { a + b }\nlet x = a.field.get(&key)?;";
        let est = estimate_tokens(code);
        // Conservative floor: real BPE tokenizers run roughly 1 token per
        // ~3 chars for punctuation-heavy code; must not read lower than that.
        let floor = code.len() / 3;
        assert!(
            est >= floor,
            "code estimate {est} too low for {} chars (floor {floor})",
            code.len()
        );
    }

    #[test]
    fn estimate_tokens_prose_sample_covers_word_count() {
        let prose = "the quick brown fox jumps over the lazy dog every single morning";
        let words = prose.split_whitespace().count();
        let est = estimate_tokens(prose);
        assert!(
            est >= words,
            "prose estimate {est} must be >= word count {words}"
        );
    }

    #[test]
    fn estimate_tokens_cjk_sample_is_conservative() {
        let cjk = "你好世界这是一个测试";
        let chars = cjk.chars().count();
        let est = estimate_tokens(cjk);
        assert!(
            est >= chars,
            "CJK estimate {est} must be >= char count {chars}"
        );
    }

    #[test]
    fn savings_reported_when_compressed() {
        let big = "lorem ipsum dolor sit amet ".repeat(40);
        let cands = vec![cand(1, &big, 0.9, Kind::File)];
        let p = pack(
            cands,
            &PackOptions {
                budget_tokens: 30,
                header_reserve: 6,
            },
        );
        assert!(p.used_tokens <= 30);
        assert!(p.savings_pct() > 0.0, "should report savings vs naive");
    }
}
