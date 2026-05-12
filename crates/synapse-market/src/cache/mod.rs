use lru::LruCache;
use std::num::NonZeroUsize;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::store::page::Bar;

#[derive(Hash, Eq, PartialEq, Clone, Debug)]
pub struct PageKey {
    pub series_id: u64,
    pub page_idx: u32,
}

pub struct DecodedPage {
    pub ts: Vec<i64>,
    pub close: Vec<f32>,
    pub volume: Option<Vec<f32>>,
    /// Full bars — kept for point_lookup
    pub bars: Vec<Bar>,
}

pub struct HotSet {
    pages: LruCache<PageKey, DecodedPage>,
    hits: AtomicU64,
    misses: AtomicU64,
}

impl HotSet {
    pub fn new(capacity: usize) -> Self {
        let cap = NonZeroUsize::new(capacity.max(1)).unwrap();
        Self {
            pages: LruCache::new(cap),
            hits: AtomicU64::new(0),
            misses: AtomicU64::new(0),
        }
    }

    pub fn get_or_load<F: FnOnce() -> DecodedPage>(&mut self, k: PageKey, load: F) -> &DecodedPage {
        if self.pages.contains(&k) {
            self.hits.fetch_add(1, Ordering::Relaxed);
        } else {
            self.misses.fetch_add(1, Ordering::Relaxed);
            let page = load();
            self.pages.put(k.clone(), page);
        }
        self.pages.get(&k).unwrap()
    }

    pub fn stats(&self) -> (u64, u64) {
        (self.hits.load(Ordering::Relaxed), self.misses.load(Ordering::Relaxed))
    }
}
