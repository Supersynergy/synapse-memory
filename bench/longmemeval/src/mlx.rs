//! MLX subprocess hooks. Shells out to `mlx_lm.generate` (Apple Silicon).
//!
//! Each call:
//!   - synthesizes a focused prompt (decompose / grade / hyde / summarize / merge)
//!   - spawns `mlx_lm.generate --model <id> --max-tokens N --prompt <p>`
//!   - parses loose output (line-list for decompose, "score:0.X" for grade, etc.)
//!   - on timeout / error / parse-failure → falls back to RuleHooks behaviour.
//!
//! Cache: SHA-256(prompt) → response, kept in a `Mutex<HashMap>` for the run.

use std::collections::HashMap;
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use synapse_core::error::Result;
use synapse_core::sota_pipeline::{PipelineHooks, RuleHooks};

pub struct MlxHooks {
    pub model: String,
    pub timeout_ms: u64,
    fallback: RuleHooks,
    cache: Mutex<HashMap<String, String>>,
    available: bool,
}

impl MlxHooks {
    pub fn new(model: String, timeout_ms: u64) -> Self {
        let available = which("mlx_lm.generate");
        if !available {
            eprintln!(
                "[mlx] mlx_lm.generate not found in PATH — MlxHooks will fall back to RuleHooks"
            );
        }
        Self {
            model,
            timeout_ms,
            fallback: RuleHooks::default(),
            cache: Mutex::new(HashMap::new()),
            available,
        }
    }

    fn cached(&self, key: &str) -> Option<String> {
        self.cache.lock().ok()?.get(key).cloned()
    }
    fn store(&self, key: String, val: String) {
        if let Ok(mut g) = self.cache.lock() {
            g.insert(key, val);
        }
    }

    /// Run mlx_lm.generate with a hard timeout. Returns trimmed stdout text or None.
    fn generate(&self, prompt: &str, max_tokens: u32) -> Option<String> {
        if !self.available {
            return None;
        }
        let key = sha_key(prompt);
        if let Some(v) = self.cached(&key) {
            return Some(v);
        }
        let mut child = Command::new("mlx_lm.generate")
            .arg("--model")
            .arg(&self.model)
            .arg("--max-tokens")
            .arg(max_tokens.to_string())
            .arg("--prompt")
            .arg(prompt)
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .ok()?;
        let deadline = Instant::now() + Duration::from_millis(self.timeout_ms);
        loop {
            match child.try_wait() {
                Ok(Some(_)) => break,
                Ok(None) => {
                    if Instant::now() >= deadline {
                        let _ = child.kill();
                        return None;
                    }
                    std::thread::sleep(Duration::from_millis(20));
                }
                Err(_) => return None,
            }
        }
        let out = child.wait_with_output().ok()?;
        let s = String::from_utf8_lossy(&out.stdout).to_string();
        let trimmed = strip_mlx_chrome(&s).trim().to_string();
        if trimmed.is_empty() {
            return None;
        }
        self.store(key, trimmed.clone());
        Some(trimmed)
    }
}

fn which(bin: &str) -> bool {
    Command::new("which")
        .arg(bin)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn sha_key(s: &str) -> String {
    // tiny non-crypto digest so we don't add a sha2 dep just for caching.
    let mut h: u64 = 1469598103934665603;
    for b in s.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(1099511628211);
    }
    format!("{:x}", h)
}

/// mlx_lm.generate prints "==========\n<text>\n==========" plus tps stats lines.
fn strip_mlx_chrome(s: &str) -> String {
    let mut between = false;
    let mut acc = String::new();
    for line in s.lines() {
        if line.starts_with("==========") {
            between = !between;
            continue;
        }
        if between {
            acc.push_str(line);
            acc.push('\n');
        }
    }
    if acc.is_empty() {
        return s.to_string();
    }
    acc
}

impl PipelineHooks for MlxHooks {
    fn decompose(&self, query: &str) -> Result<Vec<String>> {
        let prompt = format!(
            "Decompose this question into 1-3 atomic sub-questions, one per line, no numbering:\n\
             Question: {q}\nSub-questions:\n",
            q = query
        );
        if let Some(out) = self.generate(&prompt, 80) {
            let parts: Vec<String> = out
                .lines()
                .map(|l| l.trim_start_matches(|c: char| !c.is_alphabetic()).trim().to_string())
                .filter(|l| l.len() > 4)
                .take(3)
                .collect();
            if !parts.is_empty() {
                return Ok(parts);
            }
        }
        self.fallback.decompose(query)
    }

    fn grade(&self, query: &str, doc: &str) -> Result<f64> {
        let trimmed: String = doc.chars().take(800).collect();
        let prompt = format!(
            "Rate the relevance of the document to the query on a 0..1 scale.\n\
             Reply with only a number like 0.7.\nQuery: {q}\nDocument: {d}\nScore: ",
            q = query,
            d = trimmed
        );
        if let Some(out) = self.generate(&prompt, 8) {
            for tok in out.split(|c: char| !c.is_ascii_digit() && c != '.') {
                if let Ok(v) = tok.parse::<f64>() {
                    if (0.0..=1.0).contains(&v) {
                        return Ok(v);
                    }
                    if v > 1.0 && v <= 10.0 {
                        return Ok((v / 10.0).min(1.0));
                    }
                }
            }
        }
        self.fallback.grade(query, doc)
    }

    fn hyde(&self, query: &str) -> Result<String> {
        let prompt = format!(
            "Write a single short hypothetical paragraph (2-3 sentences) that would answer this question, even if you have to invent plausible facts:\n\
             Question: {q}\nAnswer:\n",
            q = query
        );
        if let Some(out) = self.generate(&prompt, 120) {
            return Ok(out);
        }
        self.fallback.hyde(query)
    }

    fn summarize(&self, items: &[&str]) -> Result<String> {
        if items.is_empty() {
            return Ok(String::new());
        }
        let joined: String = items
            .iter()
            .enumerate()
            .map(|(i, s)| format!("{}. {}", i + 1, &s.chars().take(400).collect::<String>()))
            .collect::<Vec<_>>()
            .join("\n");
        let prompt = format!(
            "Summarize the following items into one canonical sentence:\n{j}\nSummary:",
            j = joined
        );
        if let Some(out) = self.generate(&prompt, 80) {
            return Ok(out);
        }
        self.fallback.summarize(items)
    }

    fn merge(&self, existing: &str, new_text: &str) -> Result<String> {
        if existing.contains(new_text.trim()) {
            return Ok(existing.to_string());
        }
        let prompt = format!(
            "Merge the new fact into the existing memory; keep it short and non-redundant.\n\
             Existing: {e}\nNew: {n}\nMerged:",
            e = existing,
            n = new_text
        );
        if let Some(out) = self.generate(&prompt, 120) {
            return Ok(out);
        }
        self.fallback.merge(existing, new_text)
    }
}
