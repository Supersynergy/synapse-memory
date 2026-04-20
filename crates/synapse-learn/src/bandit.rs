//! Thompson-sampling shard router.
use anyhow::Result;
use rand::Rng;
use statrs::distribution::{Beta, ContinuousCDF};
use std::collections::HashMap;

pub type ShardId = String;

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
        let mut rng = rand::thread_rng();
        let mut best: Option<(&ShardId, f64)> = None;
        for sid in candidates {
            let (w, l) = self.priors.get(sid).copied().unwrap_or((1, 1));
            let alpha = w as f64;
            let beta_param = l as f64;
            let sample = if alpha > 0.0 && beta_param > 0.0 {
                // Thompson sample via beta distribution quantile of uniform
                let u: f64 = rng.gen_range(0.0..1.0);
                Beta::new(alpha, beta_param)
                    .map(|b| b.inverse_cdf(u.clamp(1e-9, 1.0 - 1e-9)))
                    .unwrap_or(0.5)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bandit_converges() {
        let shards: Vec<ShardId> = (0..4).map(|i| format!("s{i}")).collect();
        // shard "s2" has much higher win rate — should be picked most
        let mut priors: HashMap<ShardId, (u32, u32)> = shards.iter()
            .map(|s| (s.clone(), (1u32, 1u32)))
            .collect();
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
}
