//! Smoke tests for `syn sign` and `syn verify` subcommands.

use std::process::Command;
use tempfile::tempdir;

fn synapse_bin() -> &'static str {
    env!("CARGO_BIN_EXE_synapse")
}

#[test]
fn sign_and_verify_roundtrip() {
    let dir = tempdir().unwrap();
    let db = dir.path().join("brain.db");
    let sk = dir.path().join("node.sk");
    let vk = dir.path().join("node.vk");

    // init
    Command::new(synapse_bin())
        .args(["-f", db.to_str().unwrap(), "init"])
        .status()
        .expect("init");

    // keygen
    let status = Command::new(synapse_bin())
        .args([
            "-f",
            db.to_str().unwrap(),
            "keygen",
            "--sk",
            sk.to_str().unwrap(),
            "--vk",
            vk.to_str().unwrap(),
        ])
        .status()
        .expect("keygen");
    assert!(status.success(), "keygen failed");

    // put
    let out = Command::new(synapse_bin())
        .args([
            "-f",
            db.to_str().unwrap(),
            "put",
            "--text",
            "hello signed world",
            "--no-embed",
        ])
        .output()
        .expect("put");
    assert!(out.status.success(), "put failed");
    let id_str = String::from_utf8_lossy(&out.stdout).trim().to_string();
    let id: i64 = id_str.parse().expect("id is integer");

    // sign
    let status = Command::new(synapse_bin())
        .args([
            "-f",
            db.to_str().unwrap(),
            "sign",
            &id.to_string(),
            "--sk",
            sk.to_str().unwrap(),
        ])
        .status()
        .expect("sign");
    assert!(status.success(), "sign failed");

    // verify
    let status = Command::new(synapse_bin())
        .args([
            "-f",
            db.to_str().unwrap(),
            "verify",
            &id.to_string(),
            "--vk",
            vk.to_str().unwrap(),
        ])
        .status()
        .expect("verify");
    assert!(status.success(), "verify failed");
}

#[test]
fn merge_snap_produces_output() {
    let dir = tempdir().unwrap();
    let db = dir.path().join("brain.db");
    let peer = dir.path().join("peer.brainpack");
    let out = dir.path().join("merged.brainpack");

    // init + put
    Command::new(synapse_bin())
        .args(["-f", db.to_str().unwrap(), "init"])
        .status()
        .expect("init");
    Command::new(synapse_bin())
        .args([
            "-f",
            db.to_str().unwrap(),
            "put",
            "--text",
            "doc1",
            "--no-embed",
        ])
        .status()
        .expect("put");

    // export snap to use as "peer"
    let status = Command::new(synapse_bin())
        .args(["-f", db.to_str().unwrap(), "snap", peer.to_str().unwrap()])
        .status()
        .expect("snap");
    assert!(status.success(), "snap failed");

    // merge-snap
    let status = Command::new(synapse_bin())
        .args([
            "-f",
            db.to_str().unwrap(),
            "merge-snap",
            peer.to_str().unwrap(),
            "-o",
            out.to_str().unwrap(),
        ])
        .status()
        .expect("merge-snap");
    assert!(status.success(), "merge-snap failed");
    assert!(out.exists(), "merged brainpack not created");
}
