//! Ed25519 signing for entries and .brainpack files.

use crate::error::{Error, Result};
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use rand::rngs::OsRng;
use std::path::Path;

/// Generate a new keypair, writing secret key and public key to files.
pub fn keygen(secret_path: impl AsRef<Path>, public_path: impl AsRef<Path>) -> Result<()> {
    let sk = SigningKey::generate(&mut OsRng);
    std::fs::write(secret_path, sk.to_bytes())?;
    std::fs::write(public_path, sk.verifying_key().to_bytes())?;
    Ok(())
}

/// Load a signing key from a 32-byte file.
pub fn load_signing_key(path: impl AsRef<Path>) -> Result<SigningKey> {
    let bytes = std::fs::read(path)?;
    let arr: [u8; 32] = bytes
        .try_into()
        .map_err(|_| Error::Other("signing key must be 32 bytes".into()))?;
    Ok(SigningKey::from_bytes(&arr))
}

/// Load a verifying key from a 32-byte file.
pub fn load_verifying_key(path: impl AsRef<Path>) -> Result<VerifyingKey> {
    let bytes = std::fs::read(path)?;
    let arr: [u8; 32] = bytes
        .try_into()
        .map_err(|_| Error::Other("verifying key must be 32 bytes".into()))?;
    VerifyingKey::from_bytes(&arr).map_err(|e| Error::Other(e.to_string()))
}

/// Sign a byte slice, returning 64-byte signature.
pub fn sign_bytes(key: &SigningKey, data: &[u8]) -> [u8; 64] {
    key.sign(data).to_bytes()
}

/// Verify signature over data. Returns Ok(()) or Err.
pub fn verify_bytes(key: &VerifyingKey, data: &[u8], sig_bytes: &[u8; 64]) -> Result<()> {
    let sig = Signature::from_bytes(sig_bytes);
    key.verify(data, &sig)
        .map_err(|e| Error::Other(format!("signature invalid: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_sign_verify() {
        let sk = SigningKey::generate(&mut OsRng);
        let vk = sk.verifying_key();
        let data = b"hello synapse";
        let sig = sign_bytes(&sk, data);
        verify_bytes(&vk, data, &sig).unwrap();
    }

    #[test]
    fn tamper_detected() {
        let sk = SigningKey::generate(&mut OsRng);
        let vk = sk.verifying_key();
        let sig = sign_bytes(&sk, b"original");
        assert!(verify_bytes(&vk, b"tampered", &sig).is_err());
    }

    #[test]
    fn keygen_load() {
        let tmp = tempfile::TempDir::new().unwrap();
        let sk_path = tmp.path().join("key.sk");
        let vk_path = tmp.path().join("key.vk");
        keygen(&sk_path, &vk_path).unwrap();
        let sk = load_signing_key(&sk_path).unwrap();
        let vk = load_verifying_key(&vk_path).unwrap();
        let sig = sign_bytes(&sk, b"test");
        verify_bytes(&vk, b"test", &sig).unwrap();
    }
}
