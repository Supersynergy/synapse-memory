//! Integration test: unix-socket permissions + default-on token auth.
//! Oracle for bd issue `synapse-memory-cx4`.

use serde_json::{Value, json};
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

struct Daemon(Child);

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn frame(stream: &mut UnixStream, req: &Value) -> Value {
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

fn spawn_daemon(dir: &Path) -> (Daemon, PathBuf) {
    let db = dir.join("brain.db");
    let sock = dir.join("test.sock");
    // Ensure the API-key env var cannot leak in from the dev environment.
    let child = Command::new(env!("CARGO_BIN_EXE_synapsed"))
        .args(["-f", db.to_str().unwrap(), "-s", sock.to_str().unwrap()])
        .env_remove("SYNAPSE_API_KEY")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn synapsed");
    // Wait for the socket file to appear.
    let deadline = Instant::now() + Duration::from_secs(60);
    while !sock.exists() {
        assert!(Instant::now() < deadline, "daemon did not bind socket");
        std::thread::sleep(Duration::from_millis(100));
    }
    (Daemon(child), sock)
}

#[test]
fn socket_is_0600_and_auth_is_default_on() {
    let dir = tempfile::tempdir().unwrap();
    let (_d, sock) = spawn_daemon(dir.path());

    // 1. Socket file must not be world/group accessible.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&sock).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "socket perms: {mode:o}");
    }

    // 2. Protected op without auth is refused; Ping is allowed.
    let mut s = UnixStream::connect(&sock).unwrap();
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
}
