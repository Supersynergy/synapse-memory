pub mod compile;
pub mod predicate;

pub use compile::{CompiledFilter, CompiledFn, compile};
pub use predicate::{Col, Op, Predicate};

use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};

/// Cache of compiled filters keyed by predicate hash.
pub struct FilterCache {
    inner: HashMap<u64, CompiledFn>,
}

impl FilterCache {
    pub fn new() -> Self {
        Self {
            inner: HashMap::new(),
        }
    }

    pub fn get_or_compile(&mut self, p: &Predicate) -> anyhow::Result<&CompiledFn> {
        let key = predicate_hash(p);
        if let std::collections::hash_map::Entry::Vacant(e) = self.inner.entry(key) {
            let compiled = compile(p)?;
            e.insert(compiled);
        }
        Ok(self.inner.get(&key).unwrap())
    }
}

impl Default for FilterCache {
    fn default() -> Self {
        Self::new()
    }
}

fn predicate_hash(p: &Predicate) -> u64 {
    let mut h = DefaultHasher::new();
    p.hash(&mut h);
    h.finish()
}
