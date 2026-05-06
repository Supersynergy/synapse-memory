use std::collections::HashMap;
use std::sync::Mutex;

use crate::index::Hit;

// 16 shards to reduce contention under high concurrency
const SHARDS: usize = 16;
const DEFAULT_CAP_PER_SHARD: usize = 2048; // 16 * 2048 = 32768 total

/// ahash-based u64 key (10 cycles vs blake3 400 cycles)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CacheKey(pub u64);

impl CacheKey {
    #[inline]
    pub fn new(q: &str, mode: u8, limit: u16) -> Self {
        use std::hash::{Hash, Hasher};
        let mut h = ahash::AHasher::default();
        q.hash(&mut h);
        (mode as u64).hash(&mut h);
        (limit as u64).hash(&mut h);
        CacheKey(h.finish())
    }
}

struct Shard {
    map: HashMap<CacheKey, (Vec<Hit>, u64), ahash::RandomState>,
    order: std::collections::VecDeque<CacheKey>,
    cap: usize,
    gen: u64,
}

impl Shard {
    fn new(cap: usize) -> Self {
        Shard {
            map: HashMap::with_capacity_and_hasher(cap, ahash::RandomState::default()),
            order: std::collections::VecDeque::with_capacity(cap),
            cap,
            gen: 0,
        }
    }

    fn get(&mut self, key: &CacheKey) -> Option<&Vec<Hit>> {
        let gen = self.gen;
        match self.map.get(key) {
            Some((hits, entry_gen)) if *entry_gen == gen => {
                // Safety: we'll do a raw pointer trick to avoid borrow conflict
                let ptr = hits as *const Vec<Hit>;
                Some(unsafe { &*ptr })
            }
            _ => None,
        }
    }

    fn put(&mut self, key: CacheKey, hits: Vec<Hit>) {
        if self.map.len() >= self.cap {
            // evict oldest
            if let Some(old_key) = self.order.pop_front() {
                self.map.remove(&old_key);
            }
        }
        self.order.push_back(key);
        self.map.insert(key, (hits, self.gen));
    }

    fn invalidate(&mut self) {
        self.gen += 1;
        self.map.clear();
        self.order.clear();
    }
}

pub struct T0Cache {
    shards: Vec<Mutex<Shard>>,
}

impl T0Cache {
    pub fn new(total_cap: usize) -> Self {
        let per_shard = (total_cap / SHARDS).max(64);
        let shards = (0..SHARDS).map(|_| Mutex::new(Shard::new(per_shard))).collect();
        T0Cache { shards }
    }

    #[inline]
    fn shard_idx(key: &CacheKey) -> usize {
        (key.0 >> 60) as usize & (SHARDS - 1)
    }

    pub fn get(&self, key: &CacheKey) -> Option<Vec<Hit>> {
        let idx = Self::shard_idx(key);
        let mut shard = self.shards[idx].lock().unwrap();
        shard.get(key).cloned()
    }

    pub fn put(&self, key: CacheKey, hits: Vec<Hit>) {
        let idx = Self::shard_idx(&key);
        let mut shard = self.shards[idx].lock().unwrap();
        shard.put(key, hits);
    }

    pub fn invalidate(&self) {
        for shard in &self.shards {
            shard.lock().unwrap().invalidate();
        }
    }

    pub fn len(&self) -> usize {
        self.shards.iter().map(|s| s.lock().unwrap().map.len()).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::Hit;

    #[test]
    fn test_cache_put_get() {
        let c = T0Cache::new(128);
        let key = CacheKey::new("rust programming", 0, 10);
        let hits = vec![Hit { id: 1, score: 0.9 }];
        c.put(key, hits.clone());
        let got = c.get(&key).unwrap();
        assert_eq!(got[0].id, 1);
    }

    #[test]
    fn test_cache_miss() {
        let c = T0Cache::new(128);
        let key = CacheKey::new("unknown query", 0, 5);
        assert!(c.get(&key).is_none());
    }

    #[test]
    fn test_invalidate_clears() {
        let c = T0Cache::new(128);
        let key = CacheKey::new("test", 0, 5);
        c.put(key, vec![Hit { id: 1, score: 0.5 }]);
        c.invalidate();
        assert!(c.get(&key).is_none());
    }
}
