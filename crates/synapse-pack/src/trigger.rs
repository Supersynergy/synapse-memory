//! Auto-context trigger: should this prompt pull a context pack at all?
//!
//! Zero-cost gate for hooks and routers — a `false` verdict means "load nothing,
//! spend nothing". Deliberately a dumb lexical predicate (no model, no IO); the
//! heavy recall decision stays in `pack`.

/// Task-intent phrases (EN + DE). Matching one of these with ≥3 words is
/// enough evidence that stored context can help — short task prompts like
/// "implement the decay wiring" must not fall through a length gate.
const TASK_PHRASES: &[&str] = &[
    "how do i",
    "how to",
    "what is",
    "explain",
    "implement",
    "build",
    "fix ",
    "debug",
    "refactor",
    "write",
    "create",
    "design",
    "research",
    "analyze",
    "compare",
    "summarize",
    "optimize",
    "update",
    "upgrade",
    "deploy",
    "release",
    "warum",
    "wieso",
    "wo kann",
    "wie kann",
    "was ist",
    "erkläre",
    "implementiere",
    "baue",
    "schreibe",
    "untersuche",
    "optimiere",
    "erstelle",
    "ändere",
    "prüfe",
];

/// Returns true when the query looks like a task that benefits from stored
/// context (implement/fix/explain/…). False for smalltalk and short prompts.
pub fn has_context_trigger(query: &str) -> bool {
    let q = query.trim();
    let ql = q.to_ascii_lowercase();
    let words = q.split_whitespace().count();
    if words >= 3 && q.len() >= 15 {
        return TASK_PHRASES.iter().any(|t| ql.contains(t));
    }
    // Short prompts need the strong interrogative openers.
    words >= 2
        && ["how do i", "how to", "what is", "wie kann", "was ist"]
            .iter()
            .any(|t| ql.starts_with(t))
}

/// Extract the prompt text from a hook payload (Claude Code / Codex
/// UserPromptSubmit-style JSON) or pass raw text through unchanged.
pub fn hook_prompt(raw: &str) -> String {
    let trimmed = raw.trim();
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(trimmed) {
        for key in ["prompt", "user_prompt", "message", "query", "text"] {
            if let Some(s) = v.get(key).and_then(|x| x.as_str()) {
                return s.trim().to_string();
            }
        }
    }
    trimmed.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn has_context_trigger_matches_real_tasks() {
        assert!(has_context_trigger(
            "how do I implement a delta pack in synapse-pack"
        ));
        assert!(has_context_trigger(
            "wie kann ich das token-saving optimieren"
        ));
        // Short task prompts trigger too — the cost of a missed pack (agent
        // works blind) outweighs one bounded pack on a false positive.
        assert!(has_context_trigger("implement the decay wiring"));
        assert!(has_context_trigger("fix the broken test"));
        assert!(has_context_trigger("was ist der release status"));
        assert!(!has_context_trigger("hi"));
        assert!(!has_context_trigger("thanks"));
        assert!(!has_context_trigger("ok"));
        assert!(!has_context_trigger("mach mal"));
    }

    #[test]
    fn hook_prompt_reads_claude_json() {
        let raw = r#"{"prompt": "wie kann ich das token-saving optimieren", "session_id": "s1"}"#;
        assert_eq!(hook_prompt(raw), "wie kann ich das token-saving optimieren");
    }

    #[test]
    fn hook_prompt_passes_raw_text() {
        assert_eq!(
            hook_prompt("  explain the pack pipeline  "),
            "explain the pack pipeline"
        );
    }
}
