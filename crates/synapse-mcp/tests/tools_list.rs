//! Integration test: spawn synapse-mcp, send tools/list, assert 5 tools.

use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};

#[test]
fn tools_list_returns_five_tools() {
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
    assert_eq!(names.len(), 5, "expected 5 tools, got: {:?}", names);
    for expected in ["put", "search", "merge", "timeline", "verify"] {
        assert!(names.contains(&expected), "missing tool: {expected}");
    }
}
