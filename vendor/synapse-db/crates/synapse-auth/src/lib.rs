//! synapse-auth — API-key + role-based access (RBAC).
//!
//! Closes Tier-4 gap (no auth at all in P1). MVP supports:
//! - SHA256-hashed API keys
//! - constant-time comparison (no timing leaks)
//! - role enum: ReadOnly, ReadWrite, Admin
//! - per-key role assignment
//!
//! Production-ready scaffold; full GRANT/REVOKE syntax = P3.

use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::RwLock;
use subtle::ConstantTimeEq;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Role {
    ReadOnly,
    ReadWrite,
    Admin,
}

impl Role {
    pub fn allows_write(&self) -> bool {
        matches!(self, Role::ReadWrite | Role::Admin)
    }
    pub fn allows_admin(&self) -> bool {
        matches!(self, Role::Admin)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    #[error("invalid api key")]
    InvalidKey,
    #[error("permission denied: {0:?} required {1}")]
    PermissionDenied(Role, &'static str),
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ApiKey {
    pub hash: [u8; 32],
    pub role: Role,
    pub label: String,
}

type ApiKeyMap = HashMap<[u8; 32], ApiKey>;

pub fn hash_key(plain: &str) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(plain.as_bytes());
    h.finalize().into()
}

pub struct AuthStore {
    keys: RwLock<ApiKeyMap>,
}

impl AuthStore {
    pub fn new() -> Self {
        Self {
            keys: RwLock::new(HashMap::new()),
        }
    }
    pub fn add_key(&self, plain: &str, role: Role, label: impl Into<String>) {
        let h = hash_key(plain);
        let k = ApiKey {
            hash: h,
            role,
            label: label.into(),
        };
        if let Ok(mut g) = self.keys.write() {
            g.insert(h, k);
        }
    }
    /// Constant-time lookup.
    pub fn authenticate(&self, plain: &str) -> Result<Role, AuthError> {
        let h = hash_key(plain);
        let g = self.keys.read().unwrap();
        for k in g.values() {
            if k.hash.ct_eq(&h).into() {
                return Ok(k.role);
            }
        }
        Err(AuthError::InvalidKey)
    }
    pub fn require_write(&self, plain: &str) -> Result<(), AuthError> {
        let r = self.authenticate(plain)?;
        if r.allows_write() {
            Ok(())
        } else {
            Err(AuthError::PermissionDenied(r, "ReadWrite or Admin"))
        }
    }
    pub fn require_admin(&self, plain: &str) -> Result<(), AuthError> {
        let r = self.authenticate(plain)?;
        if r.allows_admin() {
            Ok(())
        } else {
            Err(AuthError::PermissionDenied(r, "Admin"))
        }
    }
    pub fn revoke(&self, plain: &str) -> bool {
        let h = hash_key(plain);
        if let Ok(mut g) = self.keys.write() {
            return g.remove(&h).is_some();
        }
        false
    }
    pub fn count(&self) -> usize {
        self.keys.read().map(|g| g.len()).unwrap_or(0)
    }
}

impl Default for AuthStore {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_and_authenticate() {
        let s = AuthStore::new();
        s.add_key("secret123", Role::ReadWrite, "wp-app");
        assert_eq!(s.authenticate("secret123").unwrap(), Role::ReadWrite);
        assert!(matches!(
            s.authenticate("wrong"),
            Err(AuthError::InvalidKey)
        ));
    }
    #[test]
    fn role_gates() {
        let s = AuthStore::new();
        s.add_key("ro", Role::ReadOnly, "reader");
        s.add_key("rw", Role::ReadWrite, "writer");
        s.add_key("admin", Role::Admin, "admin");
        assert!(s.require_write("rw").is_ok());
        assert!(s.require_write("admin").is_ok());
        assert!(matches!(
            s.require_write("ro"),
            Err(AuthError::PermissionDenied(_, _))
        ));
        assert!(matches!(
            s.require_admin("rw"),
            Err(AuthError::PermissionDenied(_, _))
        ));
    }
    #[test]
    fn revoke_works() {
        let s = AuthStore::new();
        s.add_key("temp", Role::ReadOnly, "tmp");
        assert_eq!(s.count(), 1);
        assert!(s.revoke("temp"));
        assert_eq!(s.count(), 0);
        assert!(s.authenticate("temp").is_err());
    }
}
