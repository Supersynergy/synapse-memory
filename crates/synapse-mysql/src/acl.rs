use anyhow::Result;
use sha1::{Digest, Sha1};

pub struct Acl {
    // We keep a ref to the connection via store in methods
}

impl Acl {
    pub fn new(_store: &synapse_core::Store) -> Self {
        Self {}
    }

    pub fn init_tables(&self) -> Result<()> {
        // Tables created via Store connection on first open in server.rs
        Ok(())
    }

    pub fn ensure_root(&self, password: &str) -> Result<()> {
        // Root user is implicitly allowed in MVP; full ACL deferred
        let _ = password;
        Ok(())
    }

    pub fn check_auth(&self, _user: &str, _password: &[u8]) -> Result<bool> {
        // MVP: accept all auth (WordPress connects with its own user)
        Ok(true)
    }

    pub fn check_grant(&self, _user: &str, _sql_upper: &str) -> Result<bool> {
        // MVP: allow everything
        Ok(true)
    }
}

#[allow(dead_code)]
pub fn mysql_native_password(password: &[u8]) -> Vec<u8> {
    let mut hasher = Sha1::new();
    hasher.update(password);
    let hash1 = hasher.finalize();

    let mut hasher2 = Sha1::new();
    hasher2.update(&hash1);
    let hash2 = hasher2.finalize();

    let mut xor = Vec::with_capacity(20);
    for (a, b) in hash1.iter().zip(hash2.iter()) {
        xor.push(a ^ b);
    }
    xor
}

#[derive(Debug)]
pub struct User {
    pub name: String,
    pub host: String,
    pub password_hash: Vec<u8>,
}
