//! Ed25519 signing and verification for `.synx` footers.
//!
//! v0.3 surface. Keys are 32-byte seeds; signatures are 64 bytes over the
//! manifest hash. Uses `ed25519-dalek` when the `synx-sign` feature is on;
//! otherwise degrades to a stub that refuses to sign and always fails
//! verification so callers fail closed.

#[cfg(feature = "synx-sign")]
pub use imp::*;
#[cfg(not(feature = "synx-sign"))]
pub use stub::*;

#[cfg(feature = "synx-sign")]
mod imp {
    use crate::error::{Error, Result};
    use crate::sign::random_signing_key;
    use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};

    pub fn generate_key() -> ([u8; 32], [u8; 32]) {
        let sk = random_signing_key();
        let vk = sk.verifying_key();
        (sk.to_bytes(), vk.to_bytes())
    }

    pub fn sign_manifest(manifest_hash: &[u8; 32], secret: &[u8; 32]) -> Result<[u8; 64]> {
        let sk = SigningKey::from_bytes(secret);
        Ok(sk.sign(manifest_hash).to_bytes())
    }

    pub fn verify_manifest(
        manifest_hash: &[u8; 32],
        signature: &[u8; 64],
        pubkey: &[u8; 32],
    ) -> Result<()> {
        let vk = VerifyingKey::from_bytes(pubkey)
            .map_err(|e| Error::Format(format!("bad pubkey: {e}")))?;
        let sig = Signature::from_bytes(signature);
        vk.verify(manifest_hash, &sig)
            .map_err(|e| Error::Format(format!("signature invalid: {e}")))
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn sign_verify_roundtrip() {
            let (sk, pk) = generate_key();
            let h = [7u8; 32];
            let sig = sign_manifest(&h, &sk).unwrap();
            verify_manifest(&h, &sig, &pk).unwrap();
            let bad_h = [8u8; 32];
            assert!(verify_manifest(&bad_h, &sig, &pk).is_err());
        }
    }
}

#[cfg(not(feature = "synx-sign"))]
mod stub {
    use crate::error::{Error, Result};

    pub fn generate_key() -> ([u8; 32], [u8; 32]) {
        ([0u8; 32], [0u8; 32])
    }

    pub fn sign_manifest(_h: &[u8; 32], _sk: &[u8; 32]) -> Result<[u8; 64]> {
        Err(Error::Format(
            "synx-sign feature disabled — rebuild with --features synx-sign".into(),
        ))
    }

    pub fn verify_manifest(_h: &[u8; 32], _sig: &[u8; 64], _pk: &[u8; 32]) -> Result<()> {
        Err(Error::Format("synx-sign feature disabled".into()))
    }
}
