//! MCP bridge client — connects chibi-core to the chibi-mcp-bridge daemon.
//!
//! MCP tools are identified by virtual `mcp://server/tool` paths and appear
//! as regular `Tool` structs in the tools vec. Communication with the bridge
//! daemon uses JSON-over-TCP via a lockfile-discovered address.

use std::io;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use super::{Tool, ToolMetadata};

/// Check if a tool has an MCP virtual path.
pub fn is_mcp_tool(tool: &Tool) -> bool {
    tool.path.to_str().is_some_and(|p| p.starts_with("mcp://"))
}

/// Parse server and tool name from an `mcp://server/tool` path.
pub fn parse_mcp_path(path: &Path) -> Option<(&str, &str)> {
    let s = path.to_str()?;
    let rest = s.strip_prefix("mcp://")?;
    rest.split_once('/')
}

/// Convert bridge tool info into a chibi `Tool`.
pub fn mcp_tool_from_info(
    server: &str,
    name: &str,
    description: &str,
    parameters: serde_json::Value,
) -> Tool {
    Tool {
        name: format!("{server}_{name}"),
        description: description.to_string(),
        parameters,
        path: PathBuf::from(format!("mcp://{server}/{name}")),
        hooks: vec![],
        metadata: ToolMetadata::new(),
        summary_params: vec![],
    }
}

/// Lockfile content from the bridge daemon.
#[derive(serde::Deserialize)]
struct LockContent {
    #[allow(dead_code)]
    pid: u32,
    address: String,
    #[allow(dead_code)]
    started: u64,
    #[serde(default = "default_heartbeat_secs")]
    heartbeat_secs: u64,
    #[serde(default)]
    timestamp: u64,
}

fn default_heartbeat_secs() -> u64 {
    30
}

/// Check if a bridge lockfile is stale.
///
/// Returns true if the PID in the lockfile is no longer running, or if the
/// heartbeat timestamp is older than 1.5x the heartbeat interval.
fn is_lockfile_stale(lock: &LockContent) -> bool {
    // PID liveness: instant detection of a crashed bridge without waiting for
    // the heartbeat to expire. On Linux we probe /proc/<pid>; on other unix
    // variants we fall back to timestamp-only staleness.
    #[cfg(target_os = "linux")]
    {
        if !std::path::Path::new(&format!("/proc/{}", lock.pid)).exists() {
            return true;
        }
    }

    if lock.timestamp == 0 {
        return false; // Legacy lockfile without timestamp — assume alive
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or(std::time::Duration::ZERO)
        .as_secs();
    let stale_threshold = (lock.heartbeat_secs as f64 * 1.5) as u64;
    now.saturating_sub(lock.timestamp) > stale_threshold
}

/// Read the bridge address from the lockfile, verifying PID liveness.
pub fn read_bridge_address(home: &Path) -> io::Result<SocketAddr> {
    let lock_path = home.join("mcp-bridge.lock");
    let content = std::fs::read_to_string(&lock_path)?;
    let lock: LockContent = serde_json::from_str(&content).map_err(|e| {
        io::Error::new(io::ErrorKind::InvalidData, format!("invalid lockfile: {e}"))
    })?;

    // Check heartbeat staleness
    if is_lockfile_stale(&lock) {
        let _ = std::fs::remove_file(&lock_path);
        return Err(io::Error::new(
            io::ErrorKind::NotConnected,
            "bridge process not running (stale lockfile)",
        ));
    }

    lock.address.parse().map_err(|e| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("invalid address in lockfile: {e}"),
        )
    })
}

/// Ensure the bridge daemon is running, spawning it if necessary.
///
/// Uses a spawn-mutex file (`mcp-bridge-spawning.lock`) to prevent concurrent
/// callers from each spawning their own bridge instance. The mutex is held only
/// during the spawn+poll window and removed once the bridge lockfile appears.
///
/// Flow:
/// 1. Fast path: bridge lockfile exists and is fresh — return its address.
/// 2. Acquire spawn-mutex (O_CREAT | O_EXCL). If another process holds it,
///    skip spawning and just poll for the bridge lockfile.
/// 3. Spawn bridge, poll for lockfile (up to 10s), release spawn-mutex.
pub fn ensure_bridge_running(home: &Path) -> io::Result<SocketAddr> {
    // Fast path: bridge is already running.
    if let Ok(addr) = read_bridge_address(home) {
        return Ok(addr);
    }

    let spawn_mutex = home.join("mcp-bridge-spawning.lock");

    // Try to acquire the spawn-mutex exclusively.
    let we_spawn = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&spawn_mutex)
        .is_ok();

    if we_spawn {
        // We won the race — spawn the bridge.
        let result = spawn_bridge(home);
        // Release mutex before polling so other waiters can proceed.
        let _ = std::fs::remove_file(&spawn_mutex);
        result?;
    }
    // Whether we spawned or another process did, poll until the bridge is up.
    poll_for_bridge(home)
}

/// Spawn `chibi-mcp-bridge` as a detached background process.
fn spawn_bridge(home: &Path) -> io::Result<()> {
    let bridge_bin = which_bridge()?;
    let mut cmd = std::process::Command::new(&bridge_bin);
    if let Some(home_str) = home.to_str() {
        cmd.env("CHIBI_HOME", home_str);
    }
    cmd.stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    cmd.spawn()
        .map_err(|e| io::Error::other(format!("failed to spawn chibi-mcp-bridge: {e}")))?;
    Ok(())
}

/// Poll for the bridge lockfile to appear (up to 10s).
fn poll_for_bridge(home: &Path) -> io::Result<SocketAddr> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        std::thread::sleep(std::time::Duration::from_millis(100));
        if let Ok(addr) = read_bridge_address(home) {
            return Ok(addr);
        }
        if std::time::Instant::now() >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "bridge daemon did not start within 10 seconds",
            ));
        }
    }
}

/// Locate the `chibi-mcp-bridge` binary.
///
/// Checks next to the current executable first, then falls back to PATH.
fn which_bridge() -> io::Result<PathBuf> {
    // Check next to the current executable first
    if let Ok(exe) = std::env::current_exe() {
        let sibling = exe.with_file_name("chibi-mcp-bridge");
        if sibling.exists() {
            return Ok(sibling);
        }
    }
    // Fall back to PATH — Command::new will resolve it
    Ok(PathBuf::from("chibi-mcp-bridge"))
}

/// Send a JSON request to the bridge and read the response.
pub fn send_request(addr: SocketAddr, request: &str) -> io::Result<String> {
    use std::io::{Read, Write};
    use std::net::TcpStream;

    let mut stream = TcpStream::connect(addr)?;
    stream.set_read_timeout(Some(std::time::Duration::from_secs(30)))?;
    stream.write_all(request.as_bytes())?;
    stream.shutdown(std::net::Shutdown::Write)?;

    let mut response = String::new();
    stream.read_to_string(&mut response)?;
    Ok(response)
}

/// Bridge response for list_tools.
#[derive(serde::Deserialize)]
struct ListToolsResponse {
    ok: bool,
    #[serde(default)]
    tools: Vec<BridgeToolInfo>,
    #[serde(default)]
    error: Option<String>,
}

/// Tool info as returned by the bridge.
#[derive(serde::Deserialize)]
struct BridgeToolInfo {
    server: String,
    name: String,
    description: String,
    parameters: serde_json::Value,
}

/// Bridge response for call_tool.
#[derive(serde::Deserialize)]
struct CallToolResponse {
    ok: bool,
    #[serde(default)]
    result: Option<String>,
    #[serde(default)]
    error: Option<String>,
}

/// Load MCP tools from the bridge daemon.
///
/// Returns an empty vec if the bridge is not running and cannot be started
/// (e.g., no config file or binary not found).
pub fn load_mcp_tools(home: &Path) -> io::Result<Vec<Tool>> {
    // Only attempt if config file exists
    if !home.join("mcp-bridge.toml").exists() {
        return Ok(vec![]);
    }

    let addr = ensure_bridge_running(home)?;
    let response = send_request(addr, r#"{"op":"list_tools"}"#)?;
    let parsed: ListToolsResponse = serde_json::from_str(&response).map_err(|e| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("invalid bridge response: {e}"),
        )
    })?;

    if !parsed.ok {
        return Err(io::Error::other(
            parsed.error.unwrap_or_else(|| "bridge error".into()),
        ));
    }

    Ok(parsed
        .tools
        .into_iter()
        .map(|t| mcp_tool_from_info(&t.server, &t.name, &t.description, t.parameters))
        .collect())
}

/// Execute an MCP tool via the bridge daemon.
pub fn execute_mcp_tool(tool: &Tool, args: &serde_json::Value, home: &Path) -> io::Result<String> {
    let (server, tool_name) = parse_mcp_path(&tool.path).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("not an MCP tool path: {:?}", tool.path),
        )
    })?;

    let addr = read_bridge_address(home).or_else(|_| ensure_bridge_running(home))?;

    let request = serde_json::json!({
        "op": "call_tool",
        "server": server,
        "tool": tool_name,
        "args": args,
    });

    let response = send_request(addr, &request.to_string())?;
    let parsed: CallToolResponse = serde_json::from_str(&response).map_err(|e| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("invalid bridge response: {e}"),
        )
    })?;

    if !parsed.ok {
        return Err(io::Error::other(
            parsed.error.unwrap_or_else(|| "MCP tool error".into()),
        ));
    }

    Ok(parsed.result.unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn parse_mcp_path_valid() {
        let path = PathBuf::from("mcp://serena/find_symbol");
        assert_eq!(parse_mcp_path(&path), Some(("serena", "find_symbol")));
    }

    #[test]
    fn parse_mcp_path_underscores() {
        let path = PathBuf::from("mcp://foo/bar_baz");
        assert_eq!(parse_mcp_path(&path), Some(("foo", "bar_baz")));
    }

    #[test]
    fn parse_mcp_path_not_mcp() {
        let path = PathBuf::from("/usr/bin/tool");
        assert_eq!(parse_mcp_path(&path), None);
    }

    #[test]
    fn is_mcp_tool_true() {
        let tool = mcp_tool_from_info(
            "serena",
            "find_symbol",
            "find symbols",
            serde_json::json!({}),
        );
        assert!(is_mcp_tool(&tool));
    }

    #[test]
    fn is_mcp_tool_false() {
        let tool = Tool {
            name: "my_plugin".into(),
            description: "a plugin".into(),
            parameters: serde_json::json!({}),
            path: PathBuf::from("/home/user/.chibi/plugins/my_plugin"),
            hooks: vec![],
            metadata: ToolMetadata::new(),
            summary_params: vec![],
        };
        assert!(!is_mcp_tool(&tool));
    }

    #[test]
    fn mcp_tool_from_info_creates_correct_tool() {
        let tool = mcp_tool_from_info(
            "serena",
            "find_symbol",
            "find code symbols by name",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "name": {"type": "string"}
                }
            }),
        );
        assert_eq!(tool.name, "serena_find_symbol");
        assert_eq!(tool.description, "find code symbols by name");
        assert_eq!(tool.path, PathBuf::from("mcp://serena/find_symbol"));
        assert!(tool.hooks.is_empty());
        assert!(tool.summary_params.is_empty());
    }
}
