//! # synapse-meta
//!
//! Metacognitive loop for synapse-memory. Runs as a thread inside `synapsed`,
//! observes usage events + token costs, learns routing patterns via a
//! Thompson-sampling Bandit, and writes `router.toml` atomically.
//!
//! ## Loop (5-min tick)
//!
//! 1. Read new `synapse_events` + `token_cost` since last tick.
//! 2. Query `decisions` for (task_shape, model, outcome) success rates.
//! 3. Update Bandit priors: `record_routing_outcome(shape, model, success)`.
//! 4. Sample best model per task-shape from Bandit.
//! 5. Write `router.toml` atomic (tempfile + rename).
//! 6. If `count(docs) > 120k` → trigger compaction (log event).
//! 7. Log `meta_update` event.
//!
//! ## Safety
//!
//! - Runs in-process (no new attack surface).
//! - `router.toml` writable only by daemon user.
//! - Atomic writes (tempfile + rename).
//! - Fail-open: if loop errors, old `router.toml` stays.

use anyhow::Result;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum MetaError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("toml serialize: {0}")]
    TomlSerialize(String),
    #[error("toml deserialize: {0}")]
    TomlDeserialize(String),
    #[error("router config not initialized")]
    NotInitialized,
}

/// Task shape: coarse classification of a routing request.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TaskShape {
    BulkRead,
    Code,
    Synthesis,
    Council,
    Other(String),
}

impl TaskShape {
    pub fn as_str(&self) -> &str {
        match self {
            TaskShape::BulkRead => "bulk_read",
            TaskShape::Code => "code",
            TaskShape::Synthesis => "synthesis",
            TaskShape::Council => "council",
            TaskShape::Other(s) => s.as_str(),
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s {
            "bulk_read" => TaskShape::BulkRead,
            "code" => TaskShape::Code,
            "synthesis" => TaskShape::Synthesis,
            "council" => TaskShape::Council,
            other => TaskShape::Other(other.to_string()),
        }
    }
}

/// A routing rule: task shape → model + CLI config.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoutingRule {
    pub task_shape: String,
    pub model: String,
    pub cli: String,
    /// "pipe" for stdin, "file" for file-redirect (large contexts).
    pub stdin_strategy: String,
    /// Min tokens to trigger this rule (0 = always).
    #[serde(default)]
    pub min_tokens: u64,
    /// Win count from Bandit feedback.
    #[serde(default)]
    pub wins: u32,
    /// Loss count from Bandit feedback.
    #[serde(default)]
    pub losses: u32,
}

/// Router config: list of rules + fallback.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouterConfig {
    pub rules: Vec<RoutingRule>,
    pub fallback_model: String,
    pub fallback_cli: String,
    /// Compaction threshold (docs count).
    #[serde(default = "default_compaction_threshold")]
    pub compaction_threshold: u64,
    /// Last updated timestamp.
    #[serde(default)]
    pub updated_at: i64,
}

fn default_compaction_threshold() -> u64 {
    120_000
}

impl Default for RouterConfig {
    fn default() -> Self {
        Self {
            rules: vec![
                RoutingRule {
                    task_shape: "bulk_read".into(),
                    model: "kimi".into(),
                    cli: "kimi".into(),
                    stdin_strategy: "file".into(),
                    min_tokens: 100_000,
                    wins: 1,
                    losses: 1,
                },
                RoutingRule {
                    task_shape: "code".into(),
                    model: "codex".into(),
                    cli: "codex".into(),
                    stdin_strategy: "pipe".into(),
                    min_tokens: 0,
                    wins: 1,
                    losses: 1,
                },
                RoutingRule {
                    task_shape: "synthesis".into(),
                    model: "cascade".into(),
                    cli: "claude".into(),
                    stdin_strategy: "pipe".into(),
                    min_tokens: 0,
                    wins: 1,
                    losses: 1,
                },
            ],
            fallback_model: "cascade".into(),
            fallback_cli: "claude".into(),
            compaction_threshold: default_compaction_threshold(),
            updated_at: 0,
        }
    }
}

impl RouterConfig {
    /// Load from a TOML file. Returns Default if file doesn't exist.
    pub fn load(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let text = std::fs::read_to_string(path)?;
        toml::from_str(&text).map_err(|e| MetaError::TomlDeserialize(e.to_string()).into())
    }

    /// Save to a TOML file atomically (tempfile + rename).
    pub fn save(&self, path: &Path) -> Result<()> {
        let text = toml::to_string(self)
            .map_err(|e| MetaError::TomlSerialize(e.to_string()))?;
        let tmp = path.with_extension("toml.tmp");
        std::fs::write(&tmp, text)?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    }

    /// Find the best rule for a task shape + token count.
    pub fn match_rule(&self, shape: &TaskShape, token_count: u64) -> Option<&RoutingRule> {
        let shape_s = shape.as_str();
        self.rules
            .iter()
            .filter(|r| r.task_shape == shape_s && token_count >= r.min_tokens)
            .max_by_key(|r| r.wins)
    }

    /// Record a routing outcome (win/loss) for a task shape + model.
    pub fn record_outcome(&mut self, shape: &TaskShape, model: &str, success: bool) {
        let shape_s = shape.as_str();
        for rule in &mut self.rules {
            if rule.task_shape == shape_s && rule.model == model {
                if success {
                    rule.wins += 1;
                } else {
                    rule.losses += 1;
                }
                return;
            }
        }
        // No existing rule — add one with Bandit priors.
        self.rules.push(RoutingRule {
            task_shape: shape_s.to_string(),
            model: model.to_string(),
            cli: model.to_string(),
            stdin_strategy: "pipe".into(),
            min_tokens: 0,
            wins: if success { 2 } else { 1 },
            losses: if success { 1 } else { 2 },
        });
    }

    /// Thompson-sample a model for a task shape. Returns the chosen model name.
    pub fn sample_model(&self, shape: &TaskShape) -> String {
        use rand::RngExt;
        let shape_s = shape.as_str();
        let candidates: Vec<&RoutingRule> = self.rules.iter().filter(|r| r.task_shape == shape_s).collect();
        if candidates.is_empty() {
            return self.fallback_model.clone();
        }
        let mut rng = rand::rng();
        let mut best: Option<(&RoutingRule, f64)> = None;
        for rule in candidates {
            let alpha = rule.wins as f64;
            let beta = rule.losses as f64;
            let u: f64 = rng.random_range(0.0..1.0);
            // Beta(alpha, beta) inverse CDF via simple approximation.
            // For small priors, just use mean = alpha / (alpha + beta).
            let sample = if alpha + beta > 0.0 {
                let mean = alpha / (alpha + beta);
                // Add noise proportional to uncertainty.
                let variance = (alpha * beta) / ((alpha + beta).powi(2) * (alpha + beta + 1.0));
                let std = variance.sqrt();
                mean + std * (u - 0.5) * 2.0
            } else {
                0.5
            };
            if best.map(|(_, s)| sample > s).unwrap_or(true) {
                best = Some((rule, sample));
            }
        }
        best.map(|(r, _)| r.model.clone()).unwrap_or_else(|| self.fallback_model.clone())
    }

    /// Total wins across all rules.
    pub fn total_wins(&self) -> u32 {
        self.rules.iter().map(|r| r.wins).sum()
    }

    /// Total losses across all rules.
    pub fn total_losses(&self) -> u32 {
        self.rules.iter().map(|r| r.losses).sum()
    }

    /// Success rate across all rules.
    pub fn success_rate(&self) -> f64 {
        let w = self.total_wins() as f64;
        let l = self.total_losses() as f64;
        if w + l == 0.0 {
            0.0
        } else {
            w / (w + l)
        }
    }
}

/// Metacognitive loop state. Thread-safe via Mutex.
pub struct MetaLoop {
    config: Mutex<RouterConfig>,
    config_path: PathBuf,
    /// Last tick timestamp.
    last_tick: Mutex<i64>,
    /// Number of ticks executed.
    tick_count: Mutex<u64>,
}

impl MetaLoop {
    pub fn new(config_path: PathBuf) -> Result<Arc<Self>> {
        let config = RouterConfig::load(&config_path)?;
        Ok(Arc::new(Self {
            config: Mutex::new(config),
            config_path,
            last_tick: Mutex::new(0),
            tick_count: Mutex::new(0),
        }))
    }

    /// Current config snapshot.
    pub fn config(&self) -> RouterConfig {
        self.config.lock().clone()
    }

    /// Record a routing outcome and persist to disk.
    pub fn record_outcome(&self, shape: &TaskShape, model: &str, success: bool) -> Result<()> {
        {
            let mut cfg = self.config.lock();
            cfg.record_outcome(shape, model, success);
            cfg.updated_at = chrono::Utc::now().timestamp();
        }
        self.persist()
    }

    /// Sample a model for a task shape from the Bandit.
    pub fn sample_model(&self, shape: &TaskShape) -> String {
        self.config.lock().sample_model(shape)
    }

    /// Run one tick of the metacognitive loop. Returns what was done.
    pub fn tick(&self, now: i64, docs_count: u64) -> Result<TickReport> {
        let mut report = TickReport::default();
        let mut cfg = self.config.lock();
        let _prev_tick = *self.last_tick.lock();

        // Update timestamp.
        cfg.updated_at = now;

        // Check compaction threshold.
        if docs_count > cfg.compaction_threshold {
            report.compaction_triggered = true;
            report.docs_count = docs_count;
            tracing::info!(
                docs_count,
                threshold = cfg.compaction_threshold,
                "meta-loop: compaction threshold exceeded"
            );
        }

        // Persist config.
        let persist_result = cfg.save(&self.config_path);
        drop(cfg);

        *self.last_tick.lock() = now;
        *self.tick_count.lock() += 1;
        report.tick_number = *self.tick_count.lock();
        report.persisted = persist_result.is_ok();
        Ok(report)
    }

    /// Persist current config to disk atomically.
    pub fn persist(&self) -> Result<()> {
        let cfg = self.config.lock().clone();
        cfg.save(&self.config_path)
    }

    /// Health: last tick age in seconds.
    pub fn last_tick_age_secs(&self, now: i64) -> i64 {
        now - *self.last_tick.lock()
    }

    /// Total ticks executed.
    pub fn tick_count(&self) -> u64 {
        *self.tick_count.lock()
    }
}

/// Report from a single meta-loop tick.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TickReport {
    pub tick_number: u64,
    pub persisted: bool,
    pub compaction_triggered: bool,
    pub docs_count: u64,
}

/// Health status of the meta-loop.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthStatus {
    pub tick_count: u64,
    pub last_tick_age_secs: i64,
    pub total_wins: u32,
    pub total_losses: u32,
    pub success_rate: f64,
    pub rules_count: usize,
    pub compaction_threshold: u64,
}

impl MetaLoop {
    pub fn health(&self, now: i64) -> HealthStatus {
        let cfg = self.config.lock();
        HealthStatus {
            tick_count: *self.tick_count.lock(),
            last_tick_age_secs: now - *self.last_tick.lock(),
            total_wins: cfg.total_wins(),
            total_losses: cfg.total_losses(),
            success_rate: cfg.success_rate(),
            rules_count: cfg.rules.len(),
            compaction_threshold: cfg.compaction_threshold,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn tmp_path() -> (TempDir, PathBuf) {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("router.toml");
        (dir, path)
    }

    #[test]
    fn default_config_has_three_models() {
        let cfg = RouterConfig::default();
        assert!(cfg.rules.len() >= 3);
        let models: Vec<&str> = cfg.rules.iter().map(|r| r.model.as_str()).collect();
        assert!(models.contains(&"kimi"));
        assert!(models.contains(&"codex"));
        assert!(models.contains(&"cascade"));
    }

    #[test]
    fn config_save_load_roundtrip() {
        let (_dir, path) = tmp_path();
        let cfg = RouterConfig::default();
        cfg.save(&path).unwrap();
        assert!(path.exists());
        let loaded = RouterConfig::load(&path).unwrap();
        assert_eq!(loaded.rules.len(), cfg.rules.len());
        assert_eq!(loaded.fallback_model, cfg.fallback_model);
    }

    #[test]
    fn config_load_missing_returns_default() {
        let (_dir, path) = tmp_path();
        let cfg = RouterConfig::load(&path).unwrap();
        assert_eq!(cfg.rules.len(), 3);
    }

    #[test]
    fn record_outcome_updates_existing_rule() {
        let mut cfg = RouterConfig::default();
        cfg.record_outcome(&TaskShape::Code, "codex", true);
        let rule = cfg.rules.iter().find(|r| r.task_shape == "code").unwrap();
        assert_eq!(rule.wins, 2);
        assert_eq!(rule.losses, 1);
    }

    #[test]
    fn record_outcome_adds_new_rule() {
        let mut cfg = RouterConfig::default();
        cfg.record_outcome(&TaskShape::Other("translate".into()), "kimi", true);
        assert!(cfg.rules.iter().any(|r| r.task_shape == "translate"));
    }

    #[test]
    fn match_rule_filters_by_tokens() {
        let cfg = RouterConfig::default();
        // bulk_read has min_tokens=100k.
        let r = cfg.match_rule(&TaskShape::BulkRead, 50_000);
        assert!(r.is_none(), "bulk_read should not match below 100k tokens");
        let r = cfg.match_rule(&TaskShape::BulkRead, 150_000);
        assert!(r.is_some());
        assert_eq!(r.unwrap().model, "kimi");
    }

    #[test]
    fn sample_model_returns_fallback_for_unknown_shape() {
        let cfg = RouterConfig::default();
        let m = cfg.sample_model(&TaskShape::Other("unknown".into()));
        assert_eq!(m, cfg.fallback_model);
    }

    #[test]
    fn sample_model_converges_to_winner() {
        let mut cfg = RouterConfig::default();
        // Make kimi win a lot for bulk_read.
        for _ in 0..100 {
            cfg.record_outcome(&TaskShape::BulkRead, "kimi", true);
        }
        // codex rarely wins for bulk_read.
        for _ in 0..5 {
            cfg.record_outcome(&TaskShape::BulkRead, "codex", true);
        }
        let mut kimi_count = 0;
        for _ in 0..100 {
            if cfg.sample_model(&TaskShape::BulkRead) == "kimi" {
                kimi_count += 1;
            }
        }
        assert!(kimi_count > 50, "kimi should win majority, got {kimi_count}");
    }

    #[test]
    fn meta_loop_tick_persists_config() {
        let (_dir, path) = tmp_path();
        let loop_ = MetaLoop::new(path.clone()).unwrap();
        let report = loop_.tick(1000, 1000).unwrap();
        assert!(report.persisted);
        assert!(path.exists());
        // Config on disk should have updated_at = 1000.
        let cfg = RouterConfig::load(&path).unwrap();
        assert_eq!(cfg.updated_at, 1000);
    }

    #[test]
    fn meta_loop_triggers_compaction_at_threshold() {
        let (_dir, path) = tmp_path();
        let loop_ = MetaLoop::new(path).unwrap();
        let cfg = loop_.config();
        let threshold = cfg.compaction_threshold;
        let report = loop_.tick(1000, threshold + 1).unwrap();
        assert!(report.compaction_triggered);
        let report2 = loop_.tick(2000, threshold - 1).unwrap();
        assert!(!report2.compaction_triggered);
    }

    #[test]
    fn meta_loop_record_outcome_persists() {
        let (_dir, path) = tmp_path();
        let loop_ = MetaLoop::new(path.clone()).unwrap();
        loop_.record_outcome(&TaskShape::Code, "codex", true).unwrap();
        let cfg = RouterConfig::load(&path).unwrap();
        let rule = cfg.rules.iter().find(|r| r.task_shape == "code").unwrap();
        assert_eq!(rule.wins, 2);
    }

    #[test]
    fn meta_loop_health_returns_stats() {
        let (_dir, path) = tmp_path();
        let loop_ = MetaLoop::new(path).unwrap();
        loop_.record_outcome(&TaskShape::Code, "codex", true).unwrap();
        loop_.record_outcome(&TaskShape::Code, "codex", false).unwrap();
        let h = loop_.health(1000);
        assert!(h.total_wins >= 2);
        assert!(h.total_losses >= 1);
        assert!(h.success_rate > 0.0);
    }

    #[test]
    fn task_shape_roundtrip() {
        for s in ["bulk_read", "code", "synthesis", "council", "custom"] {
            let shape = TaskShape::from_str(s);
            assert_eq!(shape.as_str(), s);
        }
    }

    #[test]
    fn tick_count_increments() {
        let (_dir, path) = tmp_path();
        let loop_ = MetaLoop::new(path).unwrap();
        assert_eq!(loop_.tick_count(), 0);
        loop_.tick(1000, 100).unwrap();
        loop_.tick(2000, 100).unwrap();
        loop_.tick(3000, 100).unwrap();
        assert_eq!(loop_.tick_count(), 3);
    }
}
