//! # synapse-crypto
//!
//! AES-256-GCM encryption-at-rest for synapse-memory. Per-Space key derivation
//! via argon2id. Sensitive columns (`content`, `meta`) are encrypted before
//! SQLite INSERT and decrypted on SELECT. Nonce is prepended to ciphertext.
//!
//! ## Threat model
//!
//! - Protects brain.db at rest (disk compromise, backup leak).
//! - Does NOT protect against a running `synapsed` with key in memory
//!   (use OS keyring / mlock for that, future work).
//! - Per-Space keys: compromise of one Space's key does not reveal others.
//!
//! ## Design
//!
//! - Master key encrypts Space Data Encryption Keys (DEKs) at rest.
//! - DEK is derived per-Space via argon2id from (master_key || space_id || salt).
//! - AES-256-GCM: 96-bit nonce, 128-bit tag.
//! - Nonce is random per encryption (never reused with same key).

use aes_gcm::{
    Aes256Gcm, Nonce,
    aead::{Aead, KeyInit, Payload},
};
use anyhow::{Context, Result};
use argon2::{Algorithm, Argon2, Params, Version};
use parking_lot::RwLock;
use rand::Rng;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use thiserror::Error;
use zeroize::Zeroize;

pub const KEY_LEN: usize = 32; // AES-256
pub const NONCE_LEN: usize = 12; // 96-bit GCM nonce
pub const SALT_LEN: usize = 16;
pub const TAG_LEN: usize = 16;

#[derive(Debug, Error)]
pub enum CryptoError {
    #[error("encryption failed: {0}")]
    Encrypt(String),
    #[error("decryption failed: {0}")]
    Decrypt(String),
    #[error("invalid key length: expected {expected}, got {actual}")]
    KeyLen { expected: usize, actual: usize },
    #[error("ciphertext too short: need nonce+tag+ct, got {0} bytes")]
    ShortCiphertext(usize),
    #[error("space key not loaded: {0}")]
    SpaceKeyMissing(String),
}

/// Master key + per-Space derived DEKs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MasterKey {
    /// 32-byte master key (argon2id-derived from passphrase or random).
    pub bytes: [u8; KEY_LEN],
    /// Argon2 salt for DEK derivation.
    pub salt: [u8; SALT_LEN],
}

impl MasterKey {
    /// Random master key (for new installs).
    pub fn random() -> Self {
        let mut bytes = [0u8; KEY_LEN];
        let mut salt = [0u8; SALT_LEN];
        rand::rng().fill_bytes(&mut bytes);
        rand::rng().fill_bytes(&mut salt);
        Self { bytes, salt }
    }

    /// Derive master key from passphrase via argon2id.
    pub fn from_passphrase(passphrase: &str) -> Result<Self> {
        let mut bytes = [0u8; KEY_LEN];
        let mut salt = [0u8; SALT_LEN];
        rand::rng().fill_bytes(&mut salt);
        let params = Params::new(64 * 1024, 3, 4, Some(KEY_LEN))
            .map_err(|e| CryptoError::Encrypt(format!("argon2 params: {e}")))?;
        let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
        argon2
            .hash_password_into(passphrase.as_bytes(), &salt, &mut bytes)
            .map_err(|e| CryptoError::Encrypt(format!("argon2 derive: {e}")))?;
        Ok(Self { bytes, salt })
    }

    /// Derive a per-Space DEK via argon2id from (master_key || space_id).
    pub fn derive_space_dek(&self, space_id: &str) -> Result<[u8; KEY_LEN]> {
        let mut dek = [0u8; KEY_LEN];
        // Mix master key + space_id into a per-space salt.
        let mut space_salt = [0u8; SALT_LEN];
        let mix = blake3::hash(&[self.salt.as_slice(), space_id.as_bytes()].concat());
        space_salt.copy_from_slice(&mix.as_bytes()[..SALT_LEN]);
        let params = Params::new(32 * 1024, 2, 2, Some(KEY_LEN))
            .map_err(|e| CryptoError::Encrypt(format!("argon2 params: {e}")))?;
        let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
        argon2
            .hash_password_into(&self.bytes, &space_salt, &mut dek)
            .map_err(|e| CryptoError::Encrypt(format!("argon2 space dek: {e}")))?;
        Ok(dek)
    }
}

impl Drop for MasterKey {
    fn drop(&mut self) {
        self.bytes.zeroize();
    }
}

/// Per-Space DEK map cached in the keyring.
type DekMap = HashMap<String, [u8; KEY_LEN]>;

/// Keyring: caches per-Space DEKs in memory. Thread-safe.
pub struct Keyring {
    master: MasterKey,
    deks: RwLock<DekMap>,
}

impl Keyring {
    pub fn new(master: MasterKey) -> Self {
        Self {
            master,
            deks: RwLock::new(HashMap::new()),
        }
    }

    /// Get or derive the DEK for a Space.
    pub fn dek(&self, space_id: &str) -> Result<[u8; KEY_LEN]> {
        if let Some(dek) = self.deks.read().get(space_id) {
            return Ok(*dek);
        }
        let dek = self.master.derive_space_dek(space_id)?;
        self.deks.write().insert(space_id.to_string(), dek);
        Ok(dek)
    }

    /// Rotate: drops cached DEKs (next access re-derives).
    pub fn rotate(&self) {
        self.deks.write().clear();
    }

    pub fn master(&self) -> &MasterKey {
        &self.master
    }
}

/// Encrypt plaintext under the given DEK. Returns nonce || ciphertext || tag.
pub fn encrypt(dek: &[u8; KEY_LEN], plaintext: &[u8]) -> Result<Vec<u8>> {
    let cipher = Aes256Gcm::new_from_slice(dek)
        .map_err(|e| CryptoError::Encrypt(format!("aes key init: {e}")))?;
    let mut nonce_bytes = [0u8; NONCE_LEN];
    rand::rng().fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);
    let ct = cipher
        .encrypt(
            nonce,
            Payload {
                msg: plaintext,
                aad: &[],
            },
        )
        .map_err(|e| CryptoError::Encrypt(format!("aes-gcm encrypt: {e}")))?;
    let mut out = Vec::with_capacity(NONCE_LEN + ct.len());
    out.extend_from_slice(&nonce_bytes);
    out.extend_from_slice(&ct);
    Ok(out)
}

/// Decrypt nonce || ciphertext || tag under the given DEK.
pub fn decrypt(dek: &[u8; KEY_LEN], blob: &[u8]) -> Result<Vec<u8>> {
    if blob.len() < NONCE_LEN + TAG_LEN {
        return Err(CryptoError::ShortCiphertext(blob.len()).into());
    }
    let (nonce_bytes, ct) = blob.split_at(NONCE_LEN);
    let cipher = Aes256Gcm::new_from_slice(dek)
        .map_err(|e| CryptoError::Decrypt(format!("aes key init: {e}")))?;
    let nonce = Nonce::from_slice(nonce_bytes);
    cipher
        .decrypt(nonce, Payload { msg: ct, aad: &[] })
        .map_err(|e| CryptoError::Decrypt(format!("aes-gcm decrypt: {e}")).into())
}

/// Convenience: encrypt under a Keyring's Space DEK.
pub fn encrypt_for_space(
    keyring: &Arc<Keyring>,
    space_id: &str,
    plaintext: &[u8],
) -> Result<Vec<u8>> {
    let dek = keyring.dek(space_id)?;
    encrypt(&dek, plaintext)
}

/// Convenience: decrypt under a Keyring's Space DEK.
pub fn decrypt_for_space(keyring: &Arc<Keyring>, space_id: &str, blob: &[u8]) -> Result<Vec<u8>> {
    let dek = keyring.dek(space_id)?;
    decrypt(&dek, blob)
}

/// Serialize a MasterKey to a base64 envelope for storage on disk.
pub fn encode_master_key(key: &MasterKey) -> String {
    use base64::Engine;
    let json = serde_json::to_string(key).unwrap_or_default();
    base64::engine::general_purpose::STANDARD.encode(json)
}

/// Deserialize a MasterKey from a base64 envelope.
pub fn decode_master_key(envelope: &str) -> Result<MasterKey> {
    use base64::Engine;
    let json = base64::engine::general_purpose::STANDARD
        .decode(envelope)
        .context("base64 decode")?;
    let key: MasterKey = serde_json::from_slice(&json).context("master key json")?;
    Ok(key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encrypt_decrypt_roundtrip() {
        let key = MasterKey::random();
        let dek = key.derive_space_dek("acme").unwrap();
        let pt = b"hello world - the quick brown fox";
        let ct = encrypt(&dek, pt).unwrap();
        assert_ne!(ct, pt);
        let pt2 = decrypt(&dek, &ct).unwrap();
        assert_eq!(pt2, pt);
    }

    #[test]
    fn per_space_keys_differ() {
        let key = MasterKey::random();
        let d1 = key.derive_space_dek("acme").unwrap();
        let d2 = key.derive_space_dek("globex").unwrap();
        assert_ne!(d1, d2, "different spaces must have different DEKs");
    }

    #[test]
    fn wrong_space_key_fails_decrypt() {
        let key = MasterKey::random();
        let d1 = key.derive_space_dek("acme").unwrap();
        let d2 = key.derive_space_dek("globex").unwrap();
        let ct = encrypt(&d1, b"secret").unwrap();
        let r = decrypt(&d2, &ct);
        assert!(r.is_err(), "decrypt with wrong space key must fail");
    }

    #[test]
    fn keyring_caches_dek() {
        let key = MasterKey::random();
        let kr = Arc::new(Keyring::new(key));
        let d1 = kr.dek("acme").unwrap();
        let d2 = kr.dek("acme").unwrap();
        assert_eq!(d1, d2);
    }

    #[test]
    fn keyring_rotation_drops_cache() {
        let key = MasterKey::random();
        let kr = Arc::new(Keyring::new(key));
        let _ = kr.dek("acme").unwrap();
        assert_eq!(kr.deks.read().len(), 1);
        kr.rotate();
        assert_eq!(kr.deks.read().len(), 0);
    }

    #[test]
    fn passphrase_derive_deterministic_given_salt() {
        let mk1 = MasterKey::from_passphrase("hunter2").unwrap();
        let mk2 = MasterKey {
            bytes: mk1.bytes,
            salt: mk1.salt,
        };
        let d1 = mk1.derive_space_dek("acme").unwrap();
        let d2 = mk2.derive_space_dek("acme").unwrap();
        assert_eq!(d1, d2);
    }

    #[test]
    fn short_ciphertext_errors() {
        let key = MasterKey::random();
        let dek = key.derive_space_dek("acme").unwrap();
        let r = decrypt(&dek, &[0u8; 5]);
        assert!(matches!(r, Err(e) if e.to_string().contains("too short")));
    }

    #[test]
    fn master_key_encode_decode_roundtrip() {
        let key = MasterKey::random();
        let env = encode_master_key(&key);
        let key2 = decode_master_key(&env).unwrap();
        assert_eq!(key.bytes, key2.bytes);
        assert_eq!(key.salt, key2.salt);
    }

    #[test]
    fn encrypt_for_space_roundtrip() {
        let key = MasterKey::random();
        let kr = Arc::new(Keyring::new(key));
        let ct = encrypt_for_space(&kr, "acme", b"secret").unwrap();
        let pt = decrypt_for_space(&kr, "acme", &ct).unwrap();
        assert_eq!(pt, b"secret");
    }
}
