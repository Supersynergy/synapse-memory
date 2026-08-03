//! # synapse-decay
//!
//! Ebbinghaus-style memory decay scoring + interaction-graph for context retention.
//!
//! Pure functions, no IO. The MCP layer calls `decay_score` to re-rank candidates
//! before packing — memories that haven't been touched in days decay, fresh ones
//! stay. This keeps a 200k context window effective over 100+ sessions.
//!
//! ## Model
//!
//! Each memory has:
//! - `last_touched_secs` — last access timestamp
//! - `strength` — [0..1], decays exponentially with half-life `HALF_LIFE_SECS`
//! - `interactions` — number of times this memory was used in feedback
//!
//! Decay score = `strength * recency_boost * interaction_boost`
//!
//! ## Why
//!
//! Without decay, a 200k window fills with stale memories. With decay, old
//! memories drop out naturally, leaving room for fresh ones. The interaction
//! graph (future work) will connect memories that co-occur in packs, so using
//! memory A boosts memory B's score.

use serde::{Deserialize, Serialize};

/// Default half-life: 24h. After 24h without access, strength halves.
pub const DEFAULT_HALF_LIFE_SECS: i64 = 86_400;

/// A memory's decay-relevant metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecayMeta {
    pub id: i64,
    pub last_touched_secs: i64,
    pub strength: f32,
    pub interactions: u32,
}

impl DecayMeta {
    pub fn new(id: i64, now_secs: i64) -> Self {
        Self {
            id,
            last_touched_secs: now_secs,
            strength: 1.0,
            interactions: 0,
        }
    }

    /// Record an interaction (feedback received). Boosts strength by 0.1,
    /// capped at 1.0, and updates last_touched.
    pub fn touch(&mut self, now_secs: i64) {
        self.last_touched_secs = now_secs;
        self.interactions = self.interactions.saturating_add(1);
        self.strength = (self.strength + 0.1).min(1.0);
    }

    /// Apply Ebbinghaus decay since last touch. Mutates strength.
    pub fn decay(&mut self, now_secs: i64, half_life_secs: i64) {
        let elapsed = (now_secs - self.last_touched_secs).max(0);
        if elapsed == 0 {
            return;
        }
        // Exponential decay: strength *= 0.5^(elapsed / half_life)
        let halvings = elapsed as f32 / half_life_secs as f32;
        self.strength *= 0.5_f32.powf(halvings);
    }
}

/// Compute the decay score for a memory at a given time. Does NOT mutate.
/// Score = strength * recency_boost * interaction_boost.
pub fn decay_score(meta: &DecayMeta, now_secs: i64, half_life_secs: i64) -> f32 {
    let elapsed = (now_secs - meta.last_touched_secs).max(0);
    let halvings = elapsed as f32 / half_life_secs as f32;
    let decayed_strength = meta.strength * 0.5_f32.powf(halvings);
    // Recency boost: memories touched in the last hour get +20%.
    let recency_boost = if elapsed < 3600 { 1.2 } else { 1.0 };
    // Interaction boost: each interaction adds 5%, capped at +50%.
    let interaction_boost = 1.0 + (meta.interactions as f32 * 0.05).min(0.5);
    decayed_strength * recency_boost * interaction_boost
}

/// Re-rank candidates by decay score (descending). Memories with higher
/// decay scores come first. Returns indices into the input slice.
pub fn decay_order(metas: &[DecayMeta], now_secs: i64, half_life_secs: i64) -> Vec<usize> {
    let mut idx: Vec<usize> = (0..metas.len()).collect();
    idx.sort_by(|&a, &b| {
        let sa = decay_score(&metas[a], now_secs, half_life_secs);
        let sb = decay_score(&metas[b], now_secs, half_life_secs);
        sb.partial_cmp(&sa).unwrap_or(std::cmp::Ordering::Equal)
    });
    idx
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_memory_has_full_strength() {
        let m = DecayMeta::new(1, 1000);
        let s = decay_score(&m, 1000, DEFAULT_HALF_LIFE_SECS);
        // Fresh memory: strength=1.0, recency_boost=1.2 (elapsed < 3600), no interactions.
        assert!((s - 1.2).abs() < 1e-6, "fresh memory score should be 1.2 (recency boost)");
    }

    #[test]
    fn decay_halves_strength_after_one_half_life() {
        let mut m = DecayMeta::new(1, 1000);
        m.strength = 1.0;
        m.decay(1000 + DEFAULT_HALF_LIFE_SECS, DEFAULT_HALF_LIFE_SECS);
        assert!((m.strength - 0.5).abs() < 1e-6, "strength should halve");
    }

    #[test]
    fn decay_score_decreases_with_time() {
        let m = DecayMeta::new(1, 1000);
        let s_now = decay_score(&m, 1000, DEFAULT_HALF_LIFE_SECS);
        let s_later = decay_score(&m, 1000 + DEFAULT_HALF_LIFE_SECS, DEFAULT_HALF_LIFE_SECS);
        assert!(s_later < s_now, "score must decrease with time");
    }

    #[test]
    fn interaction_boost_capped_at_50_percent() {
        let mut m = DecayMeta::new(1, 1000);
        m.interactions = 100;
        let s = decay_score(&m, 1000, DEFAULT_HALF_LIFE_SECS);
        // interaction_boost capped at 1.5, recency_boost = 1.2 (elapsed < 3600)
        // score = 1.0 * 1.2 * 1.5 = 1.8
        assert!((s - 1.8).abs() < 1e-6, "capped boost: got {s}");
    }

    #[test]
    fn touch_boosts_strength_and_updates_timestamp() {
        let mut m = DecayMeta::new(1, 1000);
        m.strength = 0.5;
        m.touch(2000);
        assert_eq!(m.last_touched_secs, 2000);
        assert!((m.strength - 0.6).abs() < 1e-6);
        assert_eq!(m.interactions, 1);
    }

    #[test]
    fn touch_caps_strength_at_one() {
        let mut m = DecayMeta::new(1, 1000);
        m.strength = 0.95;
        m.touch(2000);
        assert!((m.strength - 1.0).abs() < 1e-6, "strength must cap at 1.0");
    }

    #[test]
    fn decay_order_puts_fresh_first() {
        let now = 10_000;
        let metas = vec![
            DecayMeta { id: 1, last_touched_secs: now, strength: 1.0, interactions: 0 },
            DecayMeta { id: 2, last_touched_secs: now - 100_000, strength: 1.0, interactions: 0 },
            DecayMeta { id: 3, last_touched_secs: now - 1000, strength: 1.0, interactions: 5 },
        ];
        let order = decay_order(&metas, now, DEFAULT_HALF_LIFE_SECS);
        // id=3 has interaction boost → highest score, then id=1 (fresh), then id=2 (old).
        assert_eq!(metas[order[0]].id, 3);
        assert_eq!(metas[order[1]].id, 1);
        assert_eq!(metas[order[2]].id, 2);
    }

    #[test]
    fn decay_with_zero_elapsed_is_noop() {
        let mut m = DecayMeta::new(1, 1000);
        m.strength = 0.7;
        m.decay(1000, DEFAULT_HALF_LIFE_SECS);
        assert!((m.strength - 0.7).abs() < 1e-6, "no decay when elapsed=0");
    }
}
