//! Integration test: spawn synapse-mcp, send tools/list, assert 7 tools.

use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};

#[test]
fn tools_list_returns_seven_tools() {
    // Build first (relies on `cargo test` already having built the workspace)
    let bin = env!("CARGO_BIN_EXE_synapse-mcp");
    let mut child = Command::new(bin)
        .args(["--sock", "/tmp/synapse-test-nonexistent.sock"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn synapse-mcp");

    let stdin = child.stdin.as_mut().unwrap();
    // Send tools/list request
    let req = r#"{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}"#;
    writeln!(stdin, "{req}").unwrap();

    let stdout = child.stdout.take().unwrap();
    let mut reader = BufReader::new(stdout);
    let mut line = String::new();
    reader.read_line(&mut line).expect("read response");

    child.kill().ok();
    child.wait().ok();

    let resp: serde_json::Value = serde_json::from_str(line.trim()).expect("parse JSON response");
    let tools = resp["result"]["tools"].as_array().expect("tools array");
    let names: Vec<&str> = tools.iter().filter_map(|t| t["name"].as_str()).collect();
    assert_eq!(names.len(), 7, "expected 7 tools, got: {:?}", names);
    for expected in ["put", "search", "merge", "timeline", "verify", "synapse_merge", "synapse_verify"] {
        assert!(names.contains(&expected), "missing tool: {expected}");
    }
}

/// Smoke test: synapse_merge tool appears in tools/list with correct schema.
#[test]
fn synapse_merge_tool_has_snapshot_path_schema() {
    let bin = env!("CARGO_BIN_EXE_synapse-mcp");
    let mut child = Command::new(bin)
        .args(["--sock", "/tmp/synapse-test-nonexistent.sock"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn synapse-mcp");

    let stdin = child.stdin.as_mut().unwrap();
    writeln!(stdin, r#"{{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{{}}}}"#).unwrap();

    let stdout = child.stdout.take().unwrap();
    let mut reader = BufReader::new(stdout);
    let mut line = String::new();
    reader.read_line(&mut line).expect("read response");
    child.kill().ok();
    child.wait().ok();

    let resp: serde_json::Value = serde_json::from_str(line.trim()).expect("parse JSON");
    let tools = resp["result"]["tools"].as_array().expect("tools");
    let merge_tool = tools.iter().find(|t| t["name"] == "synapse_merge").expect("synapse_merge tool");
    assert!(merge_tool["inputSchema"]["properties"]["snapshot_path"].is_object());
}

/// Smoke test: synapse_verify tool appears with doc_id schema.
#[test]
fn synapse_verify_tool_has_doc_id_schema() {
    let bin = env!("CARGO_BIN_EXE_synapse-mcp");
    let mut child = Command::new(bin)
        .args(["--sock", "/tmp/synapse-test-nonexistent.sock"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn synapse-mcp");

    let stdin = child.stdin.as_mut().unwrap();
    writeln!(stdin, r#"{{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{{}}}}"#).unwrap();

    let stdout = child.stdout.take().unwrap();
    let mut reader = BufReader::new(stdout);
    let mut line = String::new();
    reader.read_line(&mut line).expect("read response");
    child.kill().ok();
    child.wait().ok();

    let resp: serde_json::Value = serde_json::from_str(line.trim()).expect("parse JSON");
    let tools = resp["result"]["tools"].as_array().expect("tools");
    let verify_tool = tools.iter().find(|t| t["name"] == "synapse_verify").expect("synapse_verify tool");
    assert!(verify_tool["inputSchema"]["properties"]["doc_id"].is_object());
}
