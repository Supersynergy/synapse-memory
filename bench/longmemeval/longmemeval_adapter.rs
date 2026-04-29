//! LongMemEval adapter (skeleton).
//!
//! Loads the LongMemEval-S JSONL format and runs Synapse `recall()` over each
//! example. Compile-only scaffold — full bench harness wired in T2.9.
//!
//! Schema (LongMemEval public release):
//! ```jsonc
//! { "question_id": "...",
//!   "question": "When did Alice move?",
//!   "haystack_sessions": [
//!     {"session_id": "s1", "messages": [{"role":"user","content":"..."}]}
//!   ],
//!   "answer": "March 2024",
//!   "question_type": "single-session-user" }
//! ```
//!
//! Intentionally a free-standing `.rs` (not a crate) — wired in once data is
//! downloaded under `~/projects/synapse/bench/longmemeval/data/`.

use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LmeMessage {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LmeSession {
    pub session_id: String,
    pub messages: Vec<LmeMessage>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LmeExample {
    pub question_id: String,
    pub question: String,
    pub haystack_sessions: Vec<LmeSession>,
    pub answer: String,
    pub question_type: String,
}

pub fn load_jsonl(path: impl AsRef<Path>) -> std::io::Result<Vec<LmeExample>> {
    use std::io::{BufRead, BufReader};
    let f = std::fs::File::open(path)?;
    let r = BufReader::new(f);
    let mut out = Vec::new();
    for line in r.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<LmeExample>(&line) {
            Ok(ex) => out.push(ex),
            Err(e) => eprintln!("skip bad line: {e}"),
        }
    }
    Ok(out)
}

/// Trivial recall-quality metric: substring match of `answer` in any returned hit text.
pub fn answer_in_hits(answer: &str, hit_texts: &[&str]) -> bool {
    let needle = answer.to_lowercase();
    hit_texts.iter().any(|t| t.to_lowercase().contains(&needle))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metric_basic() {
        assert!(answer_in_hits("March 2024", &["she moved in March 2024."]));
        assert!(!answer_in_hits("March 2024", &["unrelated"]));
    }

    #[test]
    fn parse_one_example() {
        let line = r#"{"question_id":"q1","question":"x?","haystack_sessions":[],"answer":"a","question_type":"t"}"#;
        let ex: LmeExample = serde_json::from_str(line).unwrap();
        assert_eq!(ex.question_id, "q1");
    }
}
