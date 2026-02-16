//! End-to-end integration tests for MCP bridge client.
//!
//! Spins up a mock TCP server that speaks the bridge protocol, writes a
//! lockfile pointing to it, and exercises load_mcp_tools / execute_mcp_tool.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener};
use std::path::Path;

use chibi_core::tools::mcp;

/// Spawn a mock bridge that handles one connection per call.
/// Returns the listener address.
fn spawn_mock_bridge(listener: &TcpListener) -> SocketAddr {
    listener.local_addr().unwrap()
}

/// Handle a single connection on the mock bridge, returning the parsed request.
fn handle_one_request(listener: &TcpListener) -> (String, std::net::TcpStream) {
    let (mut stream, _) = listener.accept().unwrap();
    let mut buf = String::new();
    stream.read_to_string(&mut buf).unwrap();
    (buf, stream)
}

/// Write a fake lockfile for tests.
fn write_test_lockfile(home: &Path, addr: SocketAddr) {
    let content = serde_json::json!({
        "pid": std::process::id(),
        "address": addr.to_string(),
        "started": 1000000,
    });
    std::fs::write(home.join("mcp-bridge.lock"), content.to_string()).unwrap();
}

/// Write a minimal mcp-bridge.toml so load_mcp_tools doesn't bail early.
fn write_test_config(home: &Path) {
    std::fs::write(home.join("mcp-bridge.toml"), "[servers]\n").unwrap();
}

#[test]
fn load_mcp_tools_returns_tools_from_mock_bridge() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path();

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = spawn_mock_bridge(&listener);

    write_test_lockfile(home, addr);
    write_test_config(home);

    // Spawn a thread to handle the list_tools request
    let handle = std::thread::spawn(move || {
        let (request, stream) = handle_one_request(&listener);
        let parsed: serde_json::Value = serde_json::from_str(&request).unwrap();
        assert_eq!(parsed["op"], "list_tools");

        let response = serde_json::json!({
            "ok": true,
            "tools": [{
                "server": "test_server",
                "name": "greet",
                "description": "say hello",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "name": {"type": "string"}
                    }
                }
            }]
        });

        // We need a new stream since the client shut down the write half.
        // The mock receives the full request, then writes the response.
        let mut writer = stream;
        writer.write_all(response.to_string().as_bytes()).unwrap();
    });

    let tools = mcp::load_mcp_tools(home).unwrap();
    handle.join().unwrap();

    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0].name, "test_server_greet");
    assert_eq!(tools[0].description, "say hello");
    assert!(mcp::is_mcp_tool(&tools[0]));
    assert_eq!(
        mcp::parse_mcp_path(&tools[0].path),
        Some(("test_server", "greet"))
    );
}

#[test]
fn execute_mcp_tool_sends_call_tool_and_returns_result() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path();

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = spawn_mock_bridge(&listener);

    write_test_lockfile(home, addr);
    write_test_config(home);

    let tool = mcp::mcp_tool_from_info("test_server", "greet", "say hello", serde_json::json!({}));
    let args = serde_json::json!({"name": "world"});

    let handle = std::thread::spawn(move || {
        let (request, stream) = handle_one_request(&listener);
        let parsed: serde_json::Value = serde_json::from_str(&request).unwrap();
        assert_eq!(parsed["op"], "call_tool");
        assert_eq!(parsed["server"], "test_server");
        assert_eq!(parsed["tool"], "greet");
        assert_eq!(parsed["args"]["name"], "world");

        let response = serde_json::json!({
            "ok": true,
            "result": "hello, world!"
        });

        let mut writer = stream;
        writer.write_all(response.to_string().as_bytes()).unwrap();
    });

    let result = mcp::execute_mcp_tool(&tool, &args, home).unwrap();
    handle.join().unwrap();

    assert_eq!(result, "hello, world!");
}

#[test]
fn load_mcp_tools_returns_empty_without_config() {
    let tmp = tempfile::tempdir().unwrap();
    // No mcp-bridge.toml — should return empty vec, not error
    let tools = mcp::load_mcp_tools(tmp.path()).unwrap();
    assert!(tools.is_empty());
}

#[test]
fn execute_mcp_tool_propagates_bridge_error() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path();

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = spawn_mock_bridge(&listener);

    write_test_lockfile(home, addr);
    write_test_config(home);

    let tool = mcp::mcp_tool_from_info("srv", "bad", "fails", serde_json::json!({}));

    let handle = std::thread::spawn(move || {
        let (_request, stream) = handle_one_request(&listener);
        let response = serde_json::json!({
            "ok": false,
            "error": "tool not found"
        });
        let mut writer = stream;
        writer.write_all(response.to_string().as_bytes()).unwrap();
    });

    let err = mcp::execute_mcp_tool(&tool, &serde_json::json!({}), home).unwrap_err();
    handle.join().unwrap();

    assert!(err.to_string().contains("tool not found"));
}
