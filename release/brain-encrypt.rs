// brain-encrypt.rs — SQLCipher key derivation + open + plaintext->encrypted migration.
//
// Add to crate Cargo.toml:
//   rusqlite = { version = "0.32", features = ["bundled-sqlcipher-vendored-openssl"] }
//   hkdf = "0.12"
//   sha2 = "0.10"
//   blake3 = "1.5"
//   anyhow = "1"
//
// Key derivation:
//   ikm  = license_signature[..32]
//   salt = blake3(hw_fp).as_bytes()[..32]
//   info = b"synapse-brain-v1"
//   key  = HKDF-SHA256(ikm, salt, info, 32)
//
// SQLCipher accepts raw 32-byte key as `x'<hex>'` via PRAGMA key.

use anyhow::{anyhow, Context, Result};
use hkdf::Hkdf;
use rusqlite::{Connection, OpenFlags};
use sha2::Sha256;
use std::path::{Path, PathBuf};

pub fn derive_key(license_signature: &[u8], hw_fp: &str) -> Result<[u8; 32]> {
    if license_signature.len() < 32 {
        return Err(anyhow!("license signature too short ({}<32)", license_signature.len()));
    }
    let ikm = &license_signature[..32];
    let salt = blake3::hash(hw_fp.as_bytes());
    let hk = Hkdf::<Sha256>::new(Some(salt.as_bytes()), ikm);
    let mut okm = [0u8; 32];
    hk.expand(b"synapse-brain-v1", &mut okm)
        .map_err(|e| anyhow!("hkdf expand: {e}"))?;
    Ok(okm)
}

fn key_pragma_literal(key: &[u8; 32]) -> String {
    let mut s = String::from("x'");
    for b in key { s.push_str(&format!("{:02x}", b)); }
    s.push('\'');
    s
}

/// Open an encrypted brain DB. Fails if the file is plaintext (use migrate_from_plaintext first).
pub fn open_encrypted(path: &Path, key: &[u8; 32]) -> Result<Connection> {
    let conn = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE
            | OpenFlags::SQLITE_OPEN_NO_MUTEX | OpenFlags::SQLITE_OPEN_URI,
    )?;
    // Order matters: PRAGMA key BEFORE any other statement.
    conn.pragma_update(None, "key", key_pragma_literal(key))?;
    conn.pragma_update(None, "cipher_page_size", 4096)?;
    conn.pragma_update(None, "kdf_iter", 256_000)?;
    conn.pragma_update(None, "cipher_hmac_algorithm", "HMAC_SHA512")?;
    conn.pragma_update(None, "cipher_kdf_algorithm", "PBKDF2_HMAC_SHA512")?;

    // Probe: a wrong key throws on first read.
    conn.query_row("SELECT count(*) FROM sqlite_master", [], |r| r.get::<_, i64>(0))
        .context("decrypt probe failed (wrong key or unencrypted file)")?;
    Ok(conn)
}

/// One-shot migration: plaintext synapse.db -> synapse.enc.db.
/// Uses sqlcipher_export which streams data into the attached encrypted DB.
pub fn migrate_from_plaintext(plain: &Path, encrypted: &Path, key: &[u8; 32]) -> Result<PathBuf> {
    if !plain.exists() { return Err(anyhow!("plaintext db not found: {}", plain.display())); }
    if encrypted.exists() {
        return Err(anyhow!("encrypted target already exists: {}", encrypted.display()));
    }
    let conn = Connection::open(plain)?;
    let lit = key_pragma_literal(key);
    conn.execute_batch(&format!(
        "ATTACH DATABASE '{}' AS enc KEY \"{}\";\n\
         SELECT sqlcipher_export('enc');\n\
         DETACH DATABASE enc;",
        encrypted.display(), lit
    ))?;
    Ok(encrypted.to_path_buf())
}

#[cfg(test)]
mod t {
    use super::*;
    #[test] fn kdf_stable() {
        let sig = [7u8; 64];
        let a = derive_key(&sig, "fp-abc").unwrap();
        let b = derive_key(&sig, "fp-abc").unwrap();
        assert_eq!(a, b);
        let c = derive_key(&sig, "fp-xyz").unwrap();
        assert_ne!(a, c);
    }
}
