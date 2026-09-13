//! Thompson-sampling shard router.
use anyhow::Result;
use rand::RngExt;
use std::collections::HashMap;

pub type ShardId = String;

/// Route name → Beta prior (wins, losses) as persisted in `route_reward`.
pub type RoutePriors = HashMap<String, (u32, u32)>;

pub struct ShardBandit {
    pub priors: HashMap<ShardId, (u32, u32)>,
}

impl ShardBandit {
    pub fn new(priors: HashMap<ShardId, (u32, u32)>) -> Self {
        Self { priors }
    }

    pub fn pick_shard(&self, candidates: &[ShardId]) -> Option<ShardId> {
        if candidates.is_empty() {
            return None;
        }
        let mut rng = rand::rng();
        let mut best: Option<(&ShardId, f64)> = None;
        for sid in candidates {
            let (w, l) = self.priors.get(sid).copied().unwrap_or((1, 1));
            let alpha = w as f64;
            let beta_param = l as f64;
            let sample = if alpha > 0.0 && beta_param > 0.0 {
                crate::sampling::beta(&mut rng, alpha, beta_param).unwrap_or(0.5)
            } else {
                0.5
            };
            if best.map(|(_, s)| sample > s).unwrap_or(true) {
                best = Some((sid, sample));
            }
        }
        best.map(|(s, _)| s.clone())
    }

    pub fn reward(&mut self, shard_id: &ShardId, hit: bool) {
        let entry = self.priors.entry(shard_id.clone()).or_insert((1, 1));
        if hit {
            entry.0 += 1;
        } else {
            entry.1 += 1;
        }
    }
}

pub fn load_from_db(store: &crate::LearnStore, shard_ids: &[ShardId]) -> Result<ShardBandit> {
    let mut priors = HashMap::new();
    for sid in shard_ids {
        let (w, l) = store.get_bandit_prior(sid)?;
        priors.insert(sid.clone(), (w, l));
    }
    Ok(ShardBandit::new(priors))
}

// ── Retrieval-route bandit ────────────────────────────────────────────────────
// Picks which daemon search mode serves a context pack: lexical (cheap FTS5),
// semantic (vec kNN), or hybrid (RRF fusion). Feedback on a pack updates the
// route's Beta prior, so the selector converges on the route that actually
// produces accepted memories for this brain.

/// Retrieval arms, in stable order.
pub const ROUTE_ARMS: [&str; 3] = ["lexical", "semantic", "hybrid"];
/// Below this many real (non-prior) observations the selector stays on the
/// deterministic default — avoids noisy cold-start routing.
pub const ROUTE_MIN_SAMPLES: u32 = 24;
/// Fraction of picks forced to a uniform-random arm so a stuck route can recover.
pub const ROUTE_EXPLORE_FLOOR: f64 = 0.10;
/// Cold-start / fallback route — the strongest general recall.
pub const ROUTE_DEFAULT: &str = "hybrid";

/// Pick a retrieval route from persisted Beta priors.
///
/// Returns `(route, selected_by)` where `selected_by` is `"default"` below the
/// sample threshold and `"bandit"` once Thompson sampling takes over. Priors
/// absent from the map count as Beta(1,1).
pub fn select_route(priors: &RoutePriors) -> (String, &'static str) {
    let observed: u32 = ROUTE_ARMS
        .iter()
        .map(|arm| {
            let (w, l) = priors.get(*arm).copied().unwrap_or((1, 1));
            w.saturating_sub(1) + l.saturating_sub(1)
        })
        .sum();
    if observed < ROUTE_MIN_SAMPLES {
        return (ROUTE_DEFAULT.to_string(), "default");
    }

    let mut rng = rand::rng();
    if rng.random::<f64>() < ROUTE_EXPLORE_FLOOR {
        let idx = rng.random_range(0..ROUTE_ARMS.len());
        return (ROUTE_ARMS[idx].to_string(), "bandit");
    }

    let mut best = ROUTE_DEFAULT;
    let mut best_sample = f64::MIN;
    for arm in ROUTE_ARMS {
        let (w, l) = priors.get(arm).copied().unwrap_or((1, 1));
        let sample = crate::sampling::beta(&mut rng, w as f64, l as f64).unwrap_or(0.5);
        if sample > best_sample {
            best_sample = sample;
            best = arm;
        }
    }
    (best.to_string(), "bandit")
}

/// Load route priors from the learn store and pick a route.
pub fn select_route_from_store(store: &crate::LearnStore) -> (String, &'static str) {
    match store.route_priors() {
        Ok(priors) => select_route(&priors),
        Err(_) => (ROUTE_DEFAULT.to_string(), "default"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bandit_converges() {
        let shards: Vec<ShardId> = (0..4).map(|i| format!("s{i}")).collect();
        // shard "s2" has much higher win rate — should be picked most
        let mut priors: HashMap<ShardId, (u32, u32)> =
            shards.iter().map(|s| (s.clone(), (1u32, 1u32))).collect();
        // pre-weight s2
        priors.insert("s2".into(), (80, 5));
        let bandit = ShardBandit::new(priors);
        let mut counts: HashMap<ShardId, usize> = HashMap::new();
        for _ in 0..100 {
            if let Some(s) = bandit.pick_shard(&shards) {
                *counts.entry(s).or_default() += 1;
            }
        }
        let s2_count = *counts.get("s2").unwrap_or(&0);
        assert!(s2_count > 50, "s2 should win majority, got {s2_count}");
    }

    #[test]
    fn route_defaults_below_min_samples() {
        // No observations → deterministic default, never a bandit pick.
        let priors: HashMap<String, (u32, u32)> = HashMap::new();
        for _ in 0..50 {
            let (route, by) = select_route(&priors);
            assert_eq!(route, ROUTE_DEFAULT);
            assert_eq!(by, "default");
        }
        // Sparse observations (one per arm) still below the floor.
        let priors: HashMap<String, (u32, u32)> = ROUTE_ARMS
            .iter()
            .map(|arm| (arm.to_string(), (3u32, 3u32)))
            .collect();
        let (_, by) = select_route(&priors);
        assert_eq!(by, "default");
    }

    #[test]
    fn route_bandit_converges_on_best_arm() {
        // "lexical" dominates; with enough samples the bandit picks it ~always
        // (exploration floor aside).
        let mut priors: HashMap<String, (u32, u32)> = ROUTE_ARMS
            .iter()
            .map(|arm| (arm.to_string(), (2u32, 60u32)))
            .collect();
        priors.insert("lexical".into(), (80, 5));
        let mut counts: HashMap<String, usize> = HashMap::new();
        for _ in 0..300 {
            let (route, by) = select_route(&priors);
            assert_eq!(by, "bandit");
            *counts.entry(route).or_default() += 1;
        }
        let lex = *counts.get("lexical").unwrap_or(&0);
        assert!(
            lex > 200,
            "dominant arm should win most picks after min samples, got {lex}"
        );
    }

    #[test]
    fn route_bandit_explores_weaker_arms() {
        // With overwhelming priors, the 10% floor must still pick losers.
        let mut priors: HashMap<String, (u32, u32)> = ROUTE_ARMS
            .iter()
            .map(|arm| (arm.to_string(), (2u32, 500u32)))
            .collect();
        priors.insert("hybrid".into(), (500, 2));
        let mut off_arm = 0usize;
        for _ in 0..400 {
            let (route, _) = select_route(&priors);
            if route != "hybrid" {
                off_arm += 1;
            }
        }
        assert!(off_arm >= 3, "exploration floor never fired in 400 picks");
    }
}
