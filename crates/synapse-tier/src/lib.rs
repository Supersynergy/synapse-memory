//! synapse-tier — cold-tier blob storage abstraction.
//!
//! Cluster-E of synapse-gap-sprint. Closes "Cloud-managed SaaS" gap.
//! Pattern: `object_store` crate (arrow-rs maintainer, used by Lance/DataFusion).
//!
//! Use case: push old segments to S3 to scale beyond local M-Max ≤8TB SSD.
//! Pricing target: match Turbopuffer ($10/M vec/mo) via object-storage tier.
//!
//! **STATUS**: trait-only scaffold. Wire object_store next iteration.

use async_trait::async_trait;
use bytes::Bytes;

#[derive(Debug, thiserror::Error)]
pub enum TierError {
    #[error("not enabled — build with --features s3-backend|gcs-backend|azure-backend")]
    NotEnabled,
    #[error("not found: {0}")]
    NotFound(String),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("storage: {0}")]
    Storage(String),
}

#[async_trait]
pub trait ColdTier: Send + Sync {
    async fn put(&self, key: &str, bytes: Bytes) -> Result<(), TierError>;
    async fn get(&self, key: &str) -> Result<Bytes, TierError>;
    async fn delete(&self, key: &str) -> Result<(), TierError>;
    async fn exists(&self, key: &str) -> Result<bool, TierError>;
}

/// In-memory tier for tests + dev fallback.
pub struct MemoryTier {
    inner: tokio::sync::RwLock<std::collections::HashMap<String, Bytes>>,
}

impl Default for MemoryTier {
    fn default() -> Self {
        Self {
            inner: tokio::sync::RwLock::new(Default::default()),
        }
    }
}

#[async_trait]
impl ColdTier for MemoryTier {
    async fn put(&self, key: &str, bytes: Bytes) -> Result<(), TierError> {
        self.inner.write().await.insert(key.into(), bytes);
        Ok(())
    }
    async fn get(&self, key: &str) -> Result<Bytes, TierError> {
        self.inner
            .read()
            .await
            .get(key)
            .cloned()
            .ok_or_else(|| TierError::NotFound(key.into()))
    }
    async fn delete(&self, key: &str) -> Result<(), TierError> {
        self.inner.write().await.remove(key);
        Ok(())
    }
    async fn exists(&self, key: &str) -> Result<bool, TierError> {
        Ok(self.inner.read().await.contains_key(key))
    }
}

#[cfg(any(feature = "s3-backend", feature = "gcs-backend", feature = "azure-backend"))]
pub mod object_store_tier {
    //! Real `object_store` crate adapter — S3/GCS/Azure cold-tier.
    use super::*;
    use object_store::{path::Path, ObjectStore};
    use std::sync::Arc;

    pub struct ObjectStoreTier {
        store: Arc<dyn ObjectStore>,
    }

    impl ObjectStoreTier {
        pub fn new(store: Arc<dyn ObjectStore>) -> Self {
            Self { store }
        }
    }

    #[async_trait]
    impl ColdTier for ObjectStoreTier {
        async fn put(&self, key: &str, bytes: Bytes) -> Result<(), TierError> {
            let path = Path::from(key);
            self.store
                .put(&path, bytes.into())
                .await
                .map(|_| ())
                .map_err(|e| TierError::Storage(e.to_string()))
        }
        async fn get(&self, key: &str) -> Result<Bytes, TierError> {
            let path = Path::from(key);
            match self.store.get(&path).await {
                Ok(r) => r
                    .bytes()
                    .await
                    .map_err(|e| TierError::Storage(e.to_string())),
                Err(object_store::Error::NotFound { .. }) => Err(TierError::NotFound(key.into())),
                Err(e) => Err(TierError::Storage(e.to_string())),
            }
        }
        async fn delete(&self, key: &str) -> Result<(), TierError> {
            let path = Path::from(key);
            self.store
                .delete(&path)
                .await
                .map_err(|e| TierError::Storage(e.to_string()))
        }
        async fn exists(&self, key: &str) -> Result<bool, TierError> {
            let path = Path::from(key);
            match self.store.head(&path).await {
                Ok(_) => Ok(true),
                Err(object_store::Error::NotFound { .. }) => Ok(false),
                Err(e) => Err(TierError::Storage(e.to_string())),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(feature = "s3-backend")]
    #[tokio::test]
    async fn object_store_tier_with_inmemory() {
        use crate::object_store_tier::ObjectStoreTier;
        use object_store::memory::InMemory;
        use std::sync::Arc;
        let inner = Arc::new(InMemory::new()) as Arc<dyn object_store::ObjectStore>;
        let t = ObjectStoreTier::new(inner);
        t.put("k1", Bytes::from_static(b"hi")).await.unwrap();
        assert!(t.exists("k1").await.unwrap());
        assert_eq!(&t.get("k1").await.unwrap()[..], b"hi");
        t.delete("k1").await.unwrap();
        assert!(!t.exists("k1").await.unwrap());
    }

    #[tokio::test]
    async fn memory_tier_roundtrip() {
        let t = MemoryTier::default();
        t.put("k1", Bytes::from_static(b"hello")).await.unwrap();
        assert!(t.exists("k1").await.unwrap());
        assert_eq!(&t.get("k1").await.unwrap()[..], b"hello");
        t.delete("k1").await.unwrap();
        assert!(!t.exists("k1").await.unwrap());
        assert!(matches!(t.get("k1").await, Err(TierError::NotFound(_))));
    }
}
