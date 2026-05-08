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

#[cfg(test)]
mod tests {
    use super::*;
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
