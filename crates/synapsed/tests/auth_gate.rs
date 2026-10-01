//! Integration test: socket permissions + default-on token auth.
//! Oracle for bd issue `synapse-memory-cx4`.
//!
//! Unix: daemon binds a unix socket (must be 0600). Windows: TCP loopback
//! `127.0.0.1:<port>` (token auth is the access control there).

use serde_json::{Value, json};
use std::io::{Read, Write};
#[cfg(not(unix))]
use std::net::TcpStream;
#[cfg(unix)]
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

struct Daemon(Child);

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[cfg(unix)]
type Conn = UnixStream;
#[cfg(not(unix))]
type Conn = TcpStream;

fn frame(stream: &mut Conn, req: &Value) -> Value {
    let body = rmp_serde::to_vec_named(req).unwrap();
    stream
        .write_all(&(body.len() as u32).to_le_bytes())
        .unwrap();
    stream.write_all(&body).unwrap();
    stream.flush().unwrap();
    let mut hdr = [0u8; 4];
    stream.read_exact(&mut hdr).unwrap();
    let n = u32::from_le_bytes(hdr) as usize;
    let mut buf = vec![0u8; n];
    stream.read_exact(&mut buf).unwrap();
    rmp_serde::from_slice(&buf).unwrap()
}

#[cfg(unix)]
fn connect(endpoint: &str) -> Conn {
    UnixStream::connect(endpoint).unwrap()
}

#[cfg(not(unix))]
fn connect(endpoint: &str) -> Conn {
    TcpStream::connect(endpoint).unwrap()
}

#[cfg(unix)]
fn wait_ready(endpoint: &str) {
    // Wait for the socket file to appear.
    let deadline = Instant::now() + Duration::from_secs(60);
    while !Path::new(endpoint).exists() {
        assert!(Instant::now() < deadline, "daemon did not bind socket");
        std::thread::sleep(Duration::from_millis(100));
    }
}

#[cfg(not(unix))]
fn wait_ready(endpoint: &str) {
    let deadline = Instant::now() + Duration::from_secs(60);
    while TcpStream::connect(endpoint).is_err() {
        assert!(Instant::now() < deadline, "daemon did not bind {endpoint}");
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn spawn_daemon(dir: &Path) -> (Daemon, String) {
    let db = dir.join("brain.db");
    #[cfg(unix)]
    let endpoint = dir.join("test.sock").display().to_string();
    #[cfg(not(unix))]
    let endpoint = {
        // Reserve a free port from the OS, release it, hand it to the daemon.
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        format!("127.0.0.1:{port}")
    };
    // Ensure the API-key env var cannot leak in from the dev environment.
    let child = Command::new(env!("CARGO_BIN_EXE_synapsed"))
        .args(["-f", db.to_str().unwrap(), "-s", &endpoint])
        .env_remove("SYNAPSE_API_KEY")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn synapsed");
    wait_ready(&endpoint);
    (Daemon(child), endpoint)
}

#[test]
fn socket_is_0600_and_auth_is_default_on() {
    let dir = tempfile::tempdir().unwrap();
    let (_d, endpoint) = spawn_daemon(dir.path());

    // 1. Socket file must not be world/group accessible.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&endpoint).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "socket perms: {mode:o}");
    }

    // 2. Protected op without auth is refused; Ping is allowed.
    let mut s = connect(&endpoint);
    let sql = json!({"op": "Sql", "args": {"query": "SELECT COUNT(*) FROM docs", "params": []}});
    let r = frame(&mut s, &sql);
    assert!(
        r.get("Err")
            .and_then(Value::as_str)
            .is_some_and(|e| e.contains("auth")),
        "unauthed Sql must be refused, got {r}"
    );
    let r = frame(&mut s, &json!({"op": "Ping"}));
    assert!(
        r.get("Err").is_none(),
        "Ping should be allowed unauthed, got {r}"
    );
    // Regression: a successful Ping must NOT mark the session as authed.
    let r = frame(&mut s, &sql);
    assert!(
        r.get("Err")
            .and_then(Value::as_str)
            .is_some_and(|e| e.contains("auth")),
        "Ping must not bypass auth, got {r}"
    );

    // 3. auth.token file exists with 0600 and unlocks the session.
    let tok_path = dir.path().join("auth.token");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&tok_path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "auth.token perms: {mode:o}");
    }
    let token = std::fs::read_to_string(&tok_path).unwrap();
    let token = token.trim();
    assert!(!token.is_empty(), "auth.token must be non-empty");

    let r = frame(
        &mut s,
        &json!({"op": "Auth", "args": {"token": "wrong-token"}}),
    );
    assert!(
        r.get("Err")
            .and_then(Value::as_str)
            .is_some_and(|e| e.contains("invalid")),
        "bad token must be rejected, got {r}"
    );

    let r = frame(&mut s, &json!({"op": "Auth", "args": {"token": token}}));
    assert!(r.get("Err").is_none(), "good token must auth, got {r}");

    let r = frame(&mut s, &sql);
    assert!(r.get("Err").is_none(), "authed Sql must succeed, got {r}");

    // Sql sandbox (oracle for -4oi): ATTACH denied, PRAGMA denied, write denied.
    for bad in [
        "ATTACH DATABASE '/tmp/evil.db' AS evil",
        "PRAGMA writable_schema=ON",
        "DELETE FROM docs",
        "CREATE TABLE x(y)",
    ] {
        let r = frame(
            &mut s,
            &json!({"op": "Sql", "args": {"query": bad, "params": []}}),
        );
        assert!(
            r.get("Err").is_some(),
            "sandboxed query must fail: {bad} → {r}"
        );
    }
    // Recursion bomb must be interrupted by the progress-handler deadline.
    let t = Instant::now();
    let r = frame(
        &mut s,
        &json!({"op": "Sql", "args": {"query":
            "WITH RECURSIVE c(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM c) SELECT COUNT(*) FROM c",
            "params": []}}),
    );
    assert!(
        r.get("Err").is_some(),
        "recursion bomb must be interrupted, got {r}"
    );
    assert!(
        t.elapsed() < Duration::from_secs(15),
        "bomb ran too long: {:?}",
        t.elapsed()
    );

    // 3b. Audit trail (oracle for -5dm): a Put over the socket must append
    // to audit_events in the same brain.
    let r = frame(
        &mut s,
        &json!({
            "op": "Put",
            "args": {
                "text": "audit trail test doc",
                "title": "audit-test",
                "uri": "test://audit",
                "embed": false,
            }
        }),
    );
    assert!(r.get("Err").is_none(), "authed Put failed: {r}");
    let r = frame(
        &mut s,
        &json!({"op": "Sql", "args": {
            "query": "SELECT COUNT(*) FROM audit_events WHERE action='write'", "params": []}}),
    );
    let rows = r
        .get("Rows")
        .and_then(|v| v.get("rows"))
        .and_then(Value::as_array)
        .expect("rows response");
    let n = rows[0][0].as_i64().unwrap_or(0);
    assert!(n >= 1, "expected audit_events after Put, got {n} ({r})");

    // 4. SnapMerge must refuse paths outside --snap-dir (arbitrary file write).
    #[cfg(unix)]
    {
        let r = frame(
            &mut s,
            &json!({
                "op": "SnapMerge",
                "args": {
                    "snapshot_path": "/etc/passwd",
                    "out_path": "/tmp/synapse-escape-test",
                    "level": 0,
                }
            }),
        );
        assert!(
            r.get("Err")
                .and_then(Value::as_str)
                .is_some_and(|e| e.contains("snap")),
            "SnapMerge escape must be refused, got {r}"
        );
        assert!(!Path::new("/tmp/synapse-escape-test").exists());
    }
}
