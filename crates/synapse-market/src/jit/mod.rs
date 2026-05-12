pub mod compile;
pub mod predicate;

pub use compile::{compile, CompiledFilter, CompiledFn};
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
        if !self.inner.contains_key(&key) {
            let compiled = compile(p)?;
            self.inner.insert(key, compiled);
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
