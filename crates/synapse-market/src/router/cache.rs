use lru::LruCache;
use std::collections::HashMap;
use std::num::NonZeroUsize;
use std::path::Path;

use super::{Plan, QueryKey, learn::ThompsonSampler};

/// LRU plan cache with ε-greedy bandit routing.
pub struct PlanCache {
    cache: LruCache<QueryKey, Plan>,
    bandit: ThompsonSampler,
}

impl PlanCache {
    pub fn new(capacity: usize) -> Self {
        let cap = NonZeroUsize::new(capacity.max(1)).unwrap();
        Self {
            cache: LruCache::new(cap),
            bandit: ThompsonSampler::new(),
        }
    }

    /// Choose plan for key. Uses cached plan if winrate > 0.65, else bandit-sample.
    pub fn choose(&mut self, key: &QueryKey, candidates: &[Plan]) -> Plan {
        if let Some(&cached) = self.cache.get(key) {
            let wr = self.bandit.winrate(cached);
            if wr > 0.65 {
                return cached;
            }
        }
        self.bandit.choose(candidates)
    }

    /// Record actual execution result; updates bandit + refreshes cache.
    pub fn record(&mut self, key: QueryKey, plan: Plan, latency_us: u64) {
        self.bandit.record(plan, latency_us);
        self.cache.put(key, plan);
    }

    /// Persist to bincode file.
    pub fn save<P: AsRef<Path>>(&self, path: P) -> std::io::Result<()> {
        let snapshot = CacheSnapshot {
            bandit: self.bandit.clone(),
            entries: self.cache.iter().map(|(k, &v)| (k.clone(), v)).collect(),
        };
        let bytes = bincode::serialize(&snapshot)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
        std::fs::write(path, bytes)
    }

    /// Load from bincode file. Returns default cache on any error.
    pub fn load<P: AsRef<Path>>(path: P, capacity: usize) -> Self {
        let Ok(bytes) = std::fs::read(&path) else { return Self::new(capacity) };
        let Ok(snap): Result<CacheSnapshot, _> = bincode::deserialize(&bytes) else {
            return Self::new(capacity);
        };
        let cap = NonZeroUsize::new(capacity.max(1)).unwrap();
        let mut cache = LruCache::new(cap);
        for (k, v) in snap.entries {
            cache.put(k, v);
        }
        Self { cache, bandit: snap.bandit }
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
struct CacheSnapshot {
    bandit: ThompsonSampler,
    entries: Vec<(QueryKey, Plan)>,
}
