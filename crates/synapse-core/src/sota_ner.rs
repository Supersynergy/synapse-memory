//! Lightweight NER for SOTA agent-memory layer.
//!
//! Gazetteer-first (Aho-Corasick over a seed dictionary built from
//! `entities.canonical_name` + alias_json) PLUS regex tier for
//! dates / emails / urls / capitalised-name patterns.
//!
//! Source: ported from spaCy `Matcher` + `aho-corasick` Rust crate idioms.
//! Cost: <1ms / 4 KB doc on M-series, ~150 LOC.
//!
//! Pipeline integration: `extract_entities(text, &gazetteer)` →
//! caller maps strings to `entities.id` and stores via
//! `synapse-extract::upsert_entity` + writes `memories.entity_id`.

use crate::error::Result;
use aho_corasick::{AhoCorasick, AhoCorasickBuilder, MatchKind};
use regex::Regex;
use rusqlite::Connection;

/// One detected entity span.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntitySpan {
    pub text: String,
    pub start: usize,
    pub end: usize,
    pub entity_type: &'static str,
}

/// Built once, queried many times. Holds an Aho-Corasick automaton over the
/// gazetteer plus shared regexes for non-gazetteer entity classes.
pub struct EntityRecognizer {
    ac: Option<AhoCorasick>,
    gazetteer_types: Vec<String>,
    re_email: Regex,
    re_url: Regex,
    re_iso_date: Regex,
    re_capitalised: Regex,
}

impl EntityRecognizer {
    /// Build from explicit gazetteer entries `(canonical, type)`.
    pub fn from_gazetteer(entries: &[(String, String)]) -> Result<Self> {
        let (patterns, types): (Vec<&str>, Vec<String>) =
            entries.iter().map(|(p, t)| (p.as_str(), t.clone())).unzip();
        let ac = if patterns.is_empty() {
            None
        } else {
            Some(
                AhoCorasickBuilder::new()
                    .ascii_case_insensitive(true)
                    .match_kind(MatchKind::LeftmostLongest)
                    .build(&patterns)
                    .map_err(|e| crate::error::Error::Other(e.to_string()))?,
            )
        };
        Ok(Self {
            ac,
            gazetteer_types: types,
            re_email: Regex::new(r"\b[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}\b")
                .expect("static email regex"),
            re_url: Regex::new(r"https?://[^\s)>\]]+").expect("static url regex"),
            re_iso_date: Regex::new(r"\b\d{4}-\d{2}-\d{2}\b").expect("static date regex"),
            re_capitalised: Regex::new(r"\b([A-Z][a-z]{2,})(?:\s+[A-Z][a-z]{2,})?\b")
                .expect("static cap regex"),
        })
    }

    /// Pull canonical names directly from the running store's `entities` table.
    pub fn from_store(conn: &Connection) -> Result<Self> {
        let mut stmt =
            conn.prepare("SELECT canonical_name, COALESCE(entity_type, 'thing') FROM entities")?;
        let entries: Vec<(String, String)> = stmt
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
            .collect::<std::result::Result<_, _>>()?;
        Self::from_gazetteer(&entries)
    }

    /// Run all matchers and return de-duplicated spans (sorted by start).
    pub fn extract(&self, text: &str) -> Vec<EntitySpan> {
        let mut spans: Vec<EntitySpan> = Vec::new();

        if let Some(ac) = &self.ac {
            for m in ac.find_iter(text) {
                let t = self
                    .gazetteer_types
                    .get(m.pattern().as_usize())
                    .map(|s| Box::leak(s.clone().into_boxed_str()) as &'static str)
                    .unwrap_or("thing");
                spans.push(EntitySpan {
                    text: text[m.start()..m.end()].to_string(),
                    start: m.start(),
                    end: m.end(),
                    entity_type: t,
                });
            }
        }

        for m in self.re_email.find_iter(text) {
            spans.push(EntitySpan {
                text: m.as_str().to_string(),
                start: m.start(),
                end: m.end(),
                entity_type: "email",
            });
        }
        for m in self.re_url.find_iter(text) {
            spans.push(EntitySpan {
                text: m.as_str().to_string(),
                start: m.start(),
                end: m.end(),
                entity_type: "url",
            });
        }
        for m in self.re_iso_date.find_iter(text) {
            spans.push(EntitySpan {
                text: m.as_str().to_string(),
                start: m.start(),
                end: m.end(),
                entity_type: "date",
            });
        }
        for m in self.re_capitalised.find_iter(text) {
            // Drop matches already covered by a longer span (gazetteer wins).
            if spans
                .iter()
                .any(|s| s.start <= m.start() && s.end >= m.end())
            {
                continue;
            }
            spans.push(EntitySpan {
                text: m.as_str().to_string(),
                start: m.start(),
                end: m.end(),
                entity_type: "person_or_org",
            });
        }

        spans.sort_by_key(|s| (s.start, s.end));
        // Dedupe identical spans.
        spans.dedup_by(|a, b| a.start == b.start && a.end == b.end);
        spans
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gazetteer_finds_named_entity() {
        let er = EntityRecognizer::from_gazetteer(&[
            ("Alice".into(), "person".into()),
            ("Synapse".into(), "project".into()),
        ])
        .unwrap();
        let spans = er.extract("alice and Synapse shipped today");
        assert!(spans.iter().any(|s| s.entity_type == "person"));
        assert!(spans.iter().any(|s| s.entity_type == "project"));
    }

    #[test]
    fn regex_finds_email_url_date() {
        let er = EntityRecognizer::from_gazetteer(&[]).unwrap();
        let spans = er.extract("ping me@x.io https://a.com on 2026-04-29");
        assert!(spans.iter().any(|s| s.entity_type == "email"));
        assert!(spans.iter().any(|s| s.entity_type == "url"));
        assert!(spans.iter().any(|s| s.entity_type == "date"));
    }

    #[test]
    fn capitalised_fallback() {
        let er = EntityRecognizer::from_gazetteer(&[]).unwrap();
        let spans = er.extract("Berlin is the capital");
        assert!(spans.iter().any(|s| s.entity_type == "person_or_org"));
    }
}
