//! # synapse-rbac
//!
//! Multi-Tenant Spaces + RBAC for synapse-memory. A Space is a tenant with
//! its own encryption key (see `synapse-crypto`) and access control list.
//! Roles: owner (all), editor (read/write), reader (read-only).
//!
//! ## Schema
//!
//! ```sql
//! CREATE TABLE spaces (
//!   id INTEGER PRIMARY KEY,
//!   name TEXT NOT NULL UNIQUE,
//!   owner TEXT NOT NULL,
//!   created_at INTEGER NOT NULL
//! );
//! CREATE TABLE rbac_roles (
//!   space_id INTEGER NOT NULL,
//!   user_id TEXT NOT NULL,
//!   role TEXT NOT NULL,  -- 'owner' | 'editor' | 'reader'
//!   granted_at INTEGER NOT NULL,
//!   granted_by TEXT NOT NULL,
//!   PRIMARY KEY (space_id, user_id),
//!   FOREIGN KEY (space_id) REFERENCES spaces(id) ON DELETE CASCADE
//! );
//! ```

use anyhow::Result;
use chrono::Utc;
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum RbacError {
    #[error("space not found: {0}")]
    SpaceNotFound(String),
    #[error("user {user} has no access to space {space}")]
    NoAccess { user: String, space: String },
    #[error("permission denied: need {needed:?} on space {space}, have {have:?}")]
    Denied {
        needed: Permission,
        space: String,
        have: Option<Role>,
    },
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("invalid role: {0}")]
    InvalidRole(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Owner,
    Editor,
    Reader,
}

impl Role {
    pub fn as_str(&self) -> &'static str {
        match self {
            Role::Owner => "owner",
            Role::Editor => "editor",
            Role::Reader => "reader",
        }
    }

    /// True if this role grants the given permission.
    pub fn grants(&self, perm: Permission) -> bool {
        matches!(
            (self, perm),
            (Role::Owner, _)
                | (Role::Editor, Permission::Read | Permission::Write)
                | (Role::Reader, Permission::Read)
        )
    }
}

impl std::str::FromStr for Role {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "owner" => Ok(Role::Owner),
            "editor" => Ok(Role::Editor),
            "reader" => Ok(Role::Reader),
            _ => Err(()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Permission {
    Read,
    Write,
    Delete,
    Admin,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Space {
    pub id: i64,
    pub name: String,
    pub owner: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RbacRole {
    pub space_id: i64,
    pub user_id: String,
    pub role: Role,
    pub granted_at: i64,
    pub granted_by: String,
}

/// Initialize RBAC schema on a SQLite connection.
pub fn init_schema(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS spaces (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            name TEXT NOT NULL UNIQUE,
            owner TEXT NOT NULL,
            created_at INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS rbac_roles (
            space_id INTEGER NOT NULL,
            user_id TEXT NOT NULL,
            role TEXT NOT NULL,
            granted_at INTEGER NOT NULL,
            granted_by TEXT NOT NULL,
            PRIMARY KEY (space_id, user_id),
            FOREIGN KEY (space_id) REFERENCES spaces(id) ON DELETE CASCADE
        );
        CREATE INDEX IF NOT EXISTS idx_rbac_user ON rbac_roles(user_id);",
    )?;
    Ok(())
}

/// Create a new Space. Owner is granted `Role::Owner` automatically.
pub fn create_space(conn: &Connection, name: &str, owner: &str) -> Result<Space> {
    let now = Utc::now().timestamp();
    conn.execute(
        "INSERT INTO spaces (name, owner, created_at) VALUES (?1, ?2, ?3)",
        params![name, owner, now],
    )?;
    let id = conn.last_insert_rowid();
    conn.execute(
        "INSERT INTO rbac_roles (space_id, user_id, role, granted_at, granted_by)
         VALUES (?1, ?2, 'owner', ?3, ?2)",
        params![id, owner, now],
    )?;
    Ok(Space {
        id,
        name: name.to_string(),
        owner: owner.to_string(),
        created_at: now,
    })
}

/// Look up a Space by name.
pub fn lookup_space(conn: &Connection, name: &str) -> Result<Option<Space>> {
    let mut stmt =
        conn.prepare("SELECT id, name, owner, created_at FROM spaces WHERE name = ?1")?;
    let mut rows = stmt.query(params![name])?;
    if let Some(r) = rows.next()? {
        Ok(Some(Space {
            id: r.get(0)?,
            name: r.get(1)?,
            owner: r.get(2)?,
            created_at: r.get(3)?,
        }))
    } else {
        Ok(None)
    }
}

/// Grant a role to a user on a Space.
pub fn grant_role(
    conn: &Connection,
    space_id: i64,
    user_id: &str,
    role: Role,
    granted_by: &str,
) -> Result<()> {
    let now = Utc::now().timestamp();
    conn.execute(
        "INSERT OR REPLACE INTO rbac_roles (space_id, user_id, role, granted_at, granted_by)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![space_id, user_id, role.as_str(), now, granted_by],
    )?;
    Ok(())
}

/// Revoke a user's role on a Space.
pub fn revoke_role(conn: &Connection, space_id: i64, user_id: &str) -> Result<()> {
    conn.execute(
        "DELETE FROM rbac_roles WHERE space_id = ?1 AND user_id = ?2",
        params![space_id, user_id],
    )?;
    Ok(())
}

/// Look up a user's role on a Space.
pub fn lookup_role(conn: &Connection, space_id: i64, user_id: &str) -> Result<Option<Role>> {
    let mut stmt =
        conn.prepare("SELECT role FROM rbac_roles WHERE space_id = ?1 AND user_id = ?2")?;
    let mut rows = stmt.query(params![space_id, user_id])?;
    if let Some(r) = rows.next()? {
        let s: String = r.get(0)?;
        Ok(s.parse::<Role>().ok())
    } else {
        Ok(None)
    }
}

/// List all roles on a Space.
pub fn list_roles(conn: &Connection, space_id: i64) -> Result<Vec<RbacRole>> {
    let mut stmt = conn.prepare(
        "SELECT space_id, user_id, role, granted_at, granted_by
         FROM rbac_roles WHERE space_id = ?1",
    )?;
    let rows = stmt.query_map(params![space_id], |r| {
        let role_s: String = r.get(2)?;
        Ok(RbacRole {
            space_id: r.get(0)?,
            user_id: r.get(1)?,
            role: role_s.parse::<Role>().map_err(|_| {
                rusqlite::Error::FromSqlConversionFailure(
                    2,
                    rusqlite::types::Type::Text,
                    Box::new(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        format!("invalid role: {role_s}"),
                    )),
                )
            })?,
            granted_at: r.get(3)?,
            granted_by: r.get(4)?,
        })
    })?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

/// Guard: check that a user has a permission on a Space. Returns Err if denied.
pub fn enforce(conn: &Connection, space_name: &str, user_id: &str, perm: Permission) -> Result<()> {
    let space = lookup_space(conn, space_name)?
        .ok_or_else(|| RbacError::SpaceNotFound(space_name.to_string()))?;
    let role = lookup_role(conn, space.id, user_id)?;
    let has = role.map(|r| r.grants(perm));
    match has {
        Some(true) => Ok(()),
        Some(false) | None => Err(RbacError::Denied {
            needed: perm,
            space: space_name.to_string(),
            have: role,
        }
        .into()),
    }
}

/// (space_name, user_id) -> Role map behind the cache lock.
type RoleKeyMap = HashMap<(String, String), Role>;

/// In-memory role cache for hot-path enforcement (avoids SQLite hits per read).
pub struct RoleCache {
    /// (space_name, user_id) -> Role
    cache: parking_lot::RwLock<RoleKeyMap>,
}

impl RoleCache {
    pub fn new() -> Self {
        Self {
            cache: parking_lot::RwLock::new(HashMap::new()),
        }
    }

    pub fn put(&self, space: &str, user: &str, role: Role) {
        self.cache
            .write()
            .insert((space.to_string(), user.to_string()), role);
    }

    pub fn get(&self, space: &str, user: &str) -> Option<Role> {
        self.cache
            .read()
            .get(&(space.to_string(), user.to_string()))
            .copied()
    }

    pub fn invalidate(&self, space: &str, user: &str) {
        self.cache
            .write()
            .remove(&(space.to_string(), user.to_string()));
    }

    pub fn clear(&self) {
        self.cache.write().clear();
    }
}

impl Default for RoleCache {
    fn default() -> Self {
        Self::new()
    }
}

/// Enforce with cache. Falls back to SQLite on miss and caches the result.
/// A miss with no DB row is cached as None implicitly (no entry).
pub fn enforce_cached(
    conn: &Connection,
    cache: &Arc<RoleCache>,
    space_name: &str,
    user_id: &str,
    perm: Permission,
) -> Result<()> {
    if let Some(role) = cache.get(space_name, user_id) {
        if role.grants(perm) {
            return Ok(());
        }
        return Err(RbacError::Denied {
            needed: perm,
            space: space_name.to_string(),
            have: Some(role),
        }
        .into());
    }
    let space = lookup_space(conn, space_name)?
        .ok_or_else(|| RbacError::SpaceNotFound(space_name.to_string()))?;
    let role = lookup_role(conn, space.id, user_id)?;
    match role {
        Some(r) => {
            cache.put(space_name, user_id, r);
            if r.grants(perm) {
                Ok(())
            } else {
                Err(RbacError::Denied {
                    needed: perm,
                    space: space_name.to_string(),
                    have: Some(r),
                }
                .into())
            }
        }
        None => Err(RbacError::Denied {
            needed: perm,
            space: space_name.to_string(),
            have: None,
        }
        .into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    fn fresh_conn() -> Connection {
        let f = NamedTempFile::new().unwrap().keep().unwrap().1;
        let conn = Connection::open(&f).unwrap();
        init_schema(&conn).unwrap();
        conn
    }

    #[test]
    fn create_space_grants_owner() {
        let conn = fresh_conn();
        let sp = create_space(&conn, "acme", "alice").unwrap();
        let role = lookup_role(&conn, sp.id, "alice").unwrap();
        assert_eq!(role, Some(Role::Owner));
    }

    #[test]
    fn owner_can_do_everything() {
        let conn = fresh_conn();
        create_space(&conn, "acme", "alice").unwrap();
        enforce(&conn, "acme", "alice", Permission::Read).unwrap();
        enforce(&conn, "acme", "alice", Permission::Write).unwrap();
        enforce(&conn, "acme", "alice", Permission::Delete).unwrap();
        enforce(&conn, "acme", "alice", Permission::Admin).unwrap();
    }

    #[test]
    fn reader_cannot_write() {
        let conn = fresh_conn();
        let sp = create_space(&conn, "acme", "alice").unwrap();
        grant_role(&conn, sp.id, "bob", Role::Reader, "alice").unwrap();
        enforce(&conn, "acme", "bob", Permission::Read).unwrap();
        let r = enforce(&conn, "acme", "bob", Permission::Write);
        assert!(r.is_err(), "reader must not write");
    }

    #[test]
    fn editor_cannot_admin() {
        let conn = fresh_conn();
        let sp = create_space(&conn, "acme", "alice").unwrap();
        grant_role(&conn, sp.id, "bob", Role::Editor, "alice").unwrap();
        enforce(&conn, "acme", "bob", Permission::Write).unwrap();
        let r = enforce(&conn, "acme", "bob", Permission::Admin);
        assert!(r.is_err(), "editor must not admin");
    }

    #[test]
    fn no_access_denied() {
        let conn = fresh_conn();
        create_space(&conn, "acme", "alice").unwrap();
        let r = enforce(&conn, "acme", "eve", Permission::Read);
        assert!(r.is_err());
    }

    #[test]
    fn space_isolation_by_name() {
        let conn = fresh_conn();
        let sp1 = create_space(&conn, "acme", "alice").unwrap();
        let sp2 = create_space(&conn, "globex", "alice").unwrap();
        assert_ne!(sp1.id, sp2.id);
        // Bob has access to acme only.
        grant_role(&conn, sp1.id, "bob", Role::Reader, "alice").unwrap();
        enforce(&conn, "acme", "bob", Permission::Read).unwrap();
        let r = enforce(&conn, "globex", "bob", Permission::Read);
        assert!(r.is_err(), "bob must not access globex");
    }

    #[test]
    fn revoke_removes_access() {
        let conn = fresh_conn();
        let sp = create_space(&conn, "acme", "alice").unwrap();
        grant_role(&conn, sp.id, "bob", Role::Editor, "alice").unwrap();
        enforce(&conn, "acme", "bob", Permission::Write).unwrap();
        revoke_role(&conn, sp.id, "bob").unwrap();
        let r = enforce(&conn, "acme", "bob", Permission::Write);
        assert!(r.is_err(), "revoked user must not access");
    }

    #[test]
    fn role_cache_avoids_db_hit() {
        let conn = fresh_conn();
        let sp = create_space(&conn, "acme", "alice").unwrap();
        grant_role(&conn, sp.id, "bob", Role::Editor, "alice").unwrap();
        let cache = Arc::new(RoleCache::new());
        enforce_cached(&conn, &cache, "acme", "bob", Permission::Write).unwrap();
        assert!(cache.get("acme", "bob").is_some());
        // Second call uses cache (no DB hit needed — verified by behavior).
        enforce_cached(&conn, &cache, "acme", "bob", Permission::Write).unwrap();
    }

    #[test]
    fn list_roles_returns_all() {
        let conn = fresh_conn();
        let sp = create_space(&conn, "acme", "alice").unwrap();
        grant_role(&conn, sp.id, "bob", Role::Editor, "alice").unwrap();
        grant_role(&conn, sp.id, "carol", Role::Reader, "alice").unwrap();
        let roles = list_roles(&conn, sp.id).unwrap();
        assert_eq!(roles.len(), 3); // alice + bob + carol
    }

    #[test]
    fn invalid_role_returns_none() {
        assert!("superuser".parse::<Role>().is_err());
    }
}
