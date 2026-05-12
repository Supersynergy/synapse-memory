use xorf::{Filter, Xor8};

/// Immutable xor-filter over a set of u64 keys.
/// Built once at series-close, persisted as `.xor` sidecar.
/// ~10 bits/entry, <0.4% FPR, does not saturate.
pub struct SeriesXorFilter {
    inner: Xor8,
}

impl SeriesXorFilter {
    /// Build from a slice of hashed keys. Requires ≥2 keys; returns None otherwise.
    pub fn build(keys: &[u64]) -> Option<Self> {
        if keys.len() < 2 {
            return None;
        }
        Some(Self { inner: Xor8::from(keys) })
    }

    pub fn contains(&self, key: u64) -> bool {
        self.inner.contains(&key)
    }

    /// Serialize: seed(8) + block_length(8) + fingerprints(n).
    pub fn serialize(&self) -> Vec<u8> {
        let fp = &self.inner.fingerprints;
        let mut out = Vec::with_capacity(16 + fp.len());
        out.extend_from_slice(&self.inner.seed.to_le_bytes());
        out.extend_from_slice(&(self.inner.block_length as u64).to_le_bytes());
        out.extend_from_slice(fp);
        out
    }

    pub fn deserialize(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < 16 {
            return None;
        }
        let seed = u64::from_le_bytes(bytes[0..8].try_into().ok()?);
        let block_length = u64::from_le_bytes(bytes[8..16].try_into().ok()?) as usize;
        let fingerprints: Box<[u8]> = bytes[16..].to_vec().into_boxed_slice();
        Some(Self {
            inner: Xor8 { seed, block_length, fingerprints },
        })
    }

    /// Heap bytes consumed by fingerprints.
    pub fn size_bytes(&self) -> usize {
        16 + self.inner.fingerprints.len()
    }
}
