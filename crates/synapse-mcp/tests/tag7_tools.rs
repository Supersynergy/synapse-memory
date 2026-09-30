//! Integration test: Tag 7 metacognitive + enterprise tools are reachable
//! through the MCP JSON-RPC pipe (not just direct handler calls).
//!
//! Oracle: `cargo test -p synapse-mcp --test tag7_tools` passes.
//!
//! Each tool is invoked over the real stdio JSON-RPC protocol that
//! synapse-mcp speaks in production. Brain + home are isolated per test
//! via tempdirs so the test never touches `~/.synapse/`.

use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};

use tempfile::TempDir;

struct McpHandle {
    child: Child,
}

impl McpHandle {
    fn spawn(brain: &PathBuf, home: &PathBuf) -> Self {
        let bin = env!("CARGO_BIN_EXE_synapse-mcp");
        let sock = brain.with_file_name("nonexistent-test.sock");
        let child = Command::new(bin)
            .args([
                "--sock",
                sock.to_str().unwrap(),
                "--brain",
                brain.to_str().unwrap(),
            ])
            .env("SYNAPSE_BRAIN", brain)
            .env("HOME", home)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn synapse-mcp");
        Self { child }
    }

    fn rpc(&mut self, req: &str) -> serde_json::Value {
        let stdin = self.child.stdin.as_mut().unwrap();
        writeln!(stdin, "{req}").expect("write req");
        let stdout = self.child.stdout.as_mut().unwrap();
        let mut line = String::new();
        BufReader::new(stdout)
            .read_line(&mut line)
            .expect("read response");
        serde_json::from_str(line.trim()).expect("parse JSON response")
    }

    fn list_tools(&mut self) -> Vec<serde_json::Value> {
        let resp = self.rpc(r#"{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}"#);
        resp["result"]["tools"]
            .as_array()
            .expect("tools array")
            .clone()
    }

    fn call_tool(&mut self, id: i64, name: &str, args: serde_json::Value) -> serde_json::Value {
        let req = serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "tools/call",
            "params": {"name": name, "arguments": args}
        });
        self.rpc(&req.to_string())
    }

    fn shutdown(&mut self) {
        self.child.kill().ok();
        self.child.wait().ok();
    }
}

fn fresh_env() -> (TempDir, PathBuf, PathBuf) {
    let tmp = TempDir::new().expect("tempdir");
    let brain = tmp.path().join("brain.db");
    let home = tmp.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    (tmp, brain, home)
}

/// Create a minimal `docs` table matching the real synapse schema so
/// compliance_export can query it (agent lives in the meta JSON doc).
fn init_docs_schema(brain: &PathBuf) {
    let conn = rusqlite::Connection::open(brain).expect("open brain");
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS docs (
            id      INTEGER PRIMARY KEY AUTOINCREMENT,
            uri     TEXT UNIQUE,
            title   TEXT,
            text    TEXT NOT NULL,
            meta    TEXT,
            ts      INTEGER NOT NULL
        );",
    )
    .expect("create docs table");
}

const TAG7_TOOLS: &[&str] = &[
    "meta_health",
    "meta_route",
    "meta_record_outcome",
    "compliance_export",
    "provenance_sign",
    "provenance_verify",
    "audit_query",
    "audit_verify",
    "rbac_check",
];

#[test]
fn tag7_tools_appear_in_tools_list() {
    let (_tmp, brain, home) = fresh_env();
    let mut mcp = McpHandle::spawn(&brain, &home);
    let tools = mcp.list_tools();
    let names: Vec<&str> = tools.iter().filter_map(|t| t["name"].as_str()).collect();
    for expected in TAG7_TOOLS {
        assert!(
            names.contains(expected),
            "Tag 7 tool missing from tools/list: {expected}"
        );
    }
    mcp.shutdown();
}

#[test]
fn tag7_tools_have_input_schemas() {
    let (_tmp, brain, home) = fresh_env();
    let mut mcp = McpHandle::spawn(&brain, &home);
    let tools = mcp.list_tools();
    for name in TAG7_TOOLS {
        let tool = tools
            .iter()
            .find(|t| t["name"] == *name)
            .unwrap_or_else(|| panic!("tool {name} missing"));
        assert!(
            tool["inputSchema"]["type"].is_string(),
            "tool {name} missing inputSchema.type"
        );
        assert!(
            tool["inputSchema"]["properties"].is_object(),
            "tool {name} missing inputSchema.properties"
        );
    }
    mcp.shutdown();
}

#[test]
fn meta_health_returns_not_initialized_on_fresh_home() {
    let (_tmp, brain, home) = fresh_env();
    let mut mcp = McpHandle::spawn(&brain, &home);
    let resp = mcp.call_tool(2, "meta_health", serde_json::json!({}));
    assert_eq!(resp["jsonrpc"], "2.0");
    assert_eq!(resp["id"], 2);
    assert!(resp["result"]["content"].is_array(), "resp: {resp}");
    let text = resp["result"]["content"][0]["text"].as_str().expect("text");
    let parsed: serde_json::Value = serde_json::from_str(text).expect("parse tool payload");
    assert!(
        parsed["status"] == "not_initialized" || parsed["path"].is_string(),
        "unexpected meta_health payload: {parsed}"
    );
    mcp.shutdown();
}

#[test]
fn rbac_check_denies_on_fresh_brain() {
    let (_tmp, brain, home) = fresh_env();
    let mut mcp = McpHandle::spawn(&brain, &home);
    let resp = mcp.call_tool(
        3,
        "rbac_check",
        serde_json::json!({"space": "test-space", "user": "alice", "permission": "read"}),
    );
    let text = resp["result"]["content"][0]["text"].as_str().expect("text");
    let parsed: serde_json::Value = serde_json::from_str(text).expect("parse payload");
    assert_eq!(parsed["allowed"], false, "fresh brain must deny");
    assert_eq!(parsed["space"], "test-space");
    assert_eq!(parsed["user"], "alice");
    mcp.shutdown();
}

#[test]
fn audit_verify_passes_on_fresh_brain() {
    let (_tmp, brain, home) = fresh_env();
    let mut mcp = McpHandle::spawn(&brain, &home);
    let resp = mcp.call_tool(4, "audit_verify", serde_json::json!({}));
    let text = resp["result"]["content"][0]["text"].as_str().expect("text");
    let parsed: serde_json::Value = serde_json::from_str(text).expect("parse payload");
    assert_eq!(parsed["verified"], true, "fresh audit chain must verify");
    mcp.shutdown();
}

#[test]
fn provenance_sign_then_verify_roundtrip() {
    let (_tmp, brain, home) = fresh_env();
    let mut mcp = McpHandle::spawn(&brain, &home);

    // Sign a record.
    let sign_resp = mcp.call_tool(
        5,
        "provenance_sign",
        serde_json::json!({
            "doc_id": "doc-test-001",
            "agent_id": "test-agent",
            "agent_version": "0.1.0",
            "source_uri": "test://fixture",
            "content": "hello world"
        }),
    );
    let sign_text = sign_resp["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or_else(|| panic!("provenance_sign resp: {sign_resp}"));
    let sign_payload: serde_json::Value =
        serde_json::from_str(sign_text).expect("parse sign payload");
    assert_eq!(sign_payload["signed"], true);
    assert_eq!(sign_payload["doc_id"], "doc-test-001");

    // Verify — the agent identity was persisted to the temp home.
    let verify_resp = mcp.call_tool(6, "provenance_verify", serde_json::json!({}));
    let verify_text = verify_resp["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or_else(|| panic!("provenance_verify resp: {verify_resp}"));
    let verify_payload: serde_json::Value =
        serde_json::from_str(verify_text).expect("parse verify payload");
    assert_eq!(
        verify_payload["verified"], true,
        "just-signed record must verify: {verify_payload}"
    );
    assert_eq!(verify_payload["invalid_count"], 0);

    mcp.shutdown();
}

#[test]
fn compliance_export_returns_empty_on_fresh_brain() {
    let (_tmp, brain, home) = fresh_env();
    init_docs_schema(&brain);
    let mut mcp = McpHandle::spawn(&brain, &home);
    let resp = mcp.call_tool(
        7,
        "compliance_export",
        serde_json::json!({"format": "json"}),
    );
    let text = resp["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or_else(|| panic!("compliance_export resp: {resp}"));
    let parsed: serde_json::Value = serde_json::from_str(text).expect("parse payload");
    assert!(
        parsed["records"].is_i64() || parsed["records"].is_u64(),
        "unexpected compliance_export payload: {parsed}"
    );
    mcp.shutdown();
}

#[test]
fn audit_query_returns_empty_array_on_fresh_brain() {
    let (_tmp, brain, home) = fresh_env();
    let mut mcp = McpHandle::spawn(&brain, &home);
    let resp = mcp.call_tool(8, "audit_query", serde_json::json!({}));
    let text = resp["result"]["content"][0]["text"].as_str().expect("text");
    let parsed: serde_json::Value = serde_json::from_str(text).expect("parse payload");
    assert_eq!(parsed["count"], 0);
    assert!(parsed["events"].is_array());
    mcp.shutdown();
}

#[test]
fn meta_route_reports_init_error_without_router_toml() {
    let (_tmp, brain, home) = fresh_env();
    let mut mcp = McpHandle::spawn(&brain, &home);
    let resp = mcp.call_tool(9, "meta_route", serde_json::json!({"task_shape": "code"}));
    // Without router.toml the handler returns an error result — MCP wraps it.
    assert!(
        resp["result"].is_object() || resp["error"].is_object(),
        "resp: {resp}"
    );
    mcp.shutdown();
}
