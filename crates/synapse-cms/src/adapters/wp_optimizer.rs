//! WP fast-path execution layer.
//!
//! Recognized patterns get rewritten + executed against in-memory caches
//! instead of round-tripping to row-store. First measurable WP win.

use std::collections::HashMap;
use std::sync::RwLock;

/// In-memory autoload kv cache. WP loads `wp_options WHERE autoload='yes'`
/// (200-2000 rows) on every pageload. Caching this in-process turns 1ms
/// SQLite read into <1µs map lookup.
pub struct AutoloadCache {
    inner: RwLock<HashMap<String, Vec<u8>>>,
}

impl AutoloadCache {
    pub fn new() -> Self {
        Self {
            inner: RwLock::new(HashMap::with_capacity(1024)),
        }
    }

    pub fn get(&self, name: &str) -> Option<Vec<u8>> {
        self.inner.read().ok()?.get(name).cloned()
    }

    pub fn set(&self, name: String, value: Vec<u8>) {
        if let Ok(mut g) = self.inner.write() {
            g.insert(name, value);
        }
    }

    /// Bulk load — call once at startup or after invalidation.
    pub fn load_all(&self, rows: impl IntoIterator<Item = (String, Vec<u8>)>) {
        if let Ok(mut g) = self.inner.write() {
            g.clear();
            for (k, v) in rows {
                g.insert(k, v);
            }
        }
    }

    pub fn len(&self) -> usize {
        self.inner.read().map(|g| g.len()).unwrap_or(0)
    }

    pub fn invalidate(&self, name: &str) {
        if let Ok(mut g) = self.inner.write() {
            g.remove(name);
        }
    }
}

impl Default for AutoloadCache {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cache_roundtrip() {
        let c = AutoloadCache::new();
        c.set("siteurl".into(), b"https://example.com".to_vec());
        assert_eq!(c.get("siteurl").unwrap(), b"https://example.com");
        assert_eq!(c.len(), 1);
    }
    #[test]
    fn cache_invalidate() {
        let c = AutoloadCache::new();
        c.set("k".into(), b"v".to_vec());
        c.invalidate("k");
        assert!(c.get("k").is_none());
    }
    #[test]
    fn bulk_load() {
        let c = AutoloadCache::new();
        c.load_all([
            ("a".to_string(), b"1".to_vec()),
            ("b".to_string(), b"2".to_vec()),
        ]);
        assert_eq!(c.len(), 2);
        assert_eq!(c.get("a").unwrap(), b"1");
    }
}
