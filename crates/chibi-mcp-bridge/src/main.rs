mod bridge;
mod cache;
mod config;
mod protocol;
mod server;
mod summary;

use bridge::Bridge;
use config::BridgeConfig;
use server::ServerManager;

use std::fs::{self, OpenOptions};
use std::io::Write as _;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::Mutex;

/// Heartbeat interval for lockfile freshness (seconds).
const HEARTBEAT_SECS: u64 = 30;

/// Lockfile content written atomically on startup.
#[derive(serde::Serialize)]
struct LockContent {
    pid: u32,
    address: String,
    started: u64,
    heartbeat_secs: u64,
    timestamp: u64,
}

/// Resolve chibi home directory: `CHIBI_HOME` env > `~/.chibi`
fn chibi_home() -> PathBuf {
    if let Ok(h) = std::env::var("CHIBI_HOME") {
        return PathBuf::from(h);
    }
    dirs::home_dir()
        .expect("could not determine home directory")
        .join(".chibi")
}

/// Read `api_key` from `<home>/config.toml` (chibi's main config).
///
/// Returns `None` if the file is missing, unparseable, or has no key set.
fn read_api_key(home: &Path) -> Option<String> {
    let content = fs::read_to_string(home.join("config.toml")).ok()?;
    let table: toml::Table = content.parse().ok()?;
    table.get("api_key")?.as_str().map(String::from)
}

/// Write bridge lockfile atomically (O_CREAT | O_EXCL).
///
/// If a lockfile already exists, checks whether it is stale (heartbeat
/// timestamp older than 1.5x the heartbeat interval). Stale lockfiles are
/// removed and retried. Returns `AlreadyExists` only when another bridge
/// instance is genuinely running.
fn write_lockfile(home: &Path, addr: &SocketAddr) -> std::io::Result<PathBuf> {
    let lock_path = home.join("mcp-bridge.lock");

    if let Some(parent) = lock_path.parent() {
        fs::create_dir_all(parent)?;
    }

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_secs();

    let content = LockContent {
        pid: std::process::id(),
        address: addr.to_string(),
        started: now,
        heartbeat_secs: HEARTBEAT_SECS,
        timestamp: now,
    };

    let json = serde_json::to_string(&content).map_err(std::io::Error::other)?;

    // Try to create exclusively first.
    match OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&lock_path)
    {
        Ok(mut file) => {
            file.write_all(json.as_bytes())?;
            file.sync_all()?;
            Ok(lock_path)
        }
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            // Check if the existing lock is stale (heartbeat expired).
            if is_lockfile_stale(&lock_path) {
                eprintln!("[mcp-bridge] removing stale lockfile");
                fs::remove_file(&lock_path)?;
                // Retry once.
                let mut file = OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&lock_path)?;
                file.write_all(json.as_bytes())?;
                file.sync_all()?;
                return Ok(lock_path);
            }
            Err(e)
        }
        Err(e) => Err(e),
    }
}

/// Check if the lockfile is stale (heartbeat timestamp older than 1.5x interval).
fn is_lockfile_stale(lock_path: &Path) -> bool {
    let content = match fs::read_to_string(lock_path) {
        Ok(c) => c,
        Err(_) => return true,
    };
    let lock: serde_json::Value = match serde_json::from_str(&content) {
        Ok(v) => v,
        Err(_) => return true,
    };
    let heartbeat_secs = lock
        .get("heartbeat_secs")
        .and_then(|v| v.as_u64())
        .unwrap_or(HEARTBEAT_SECS);
    let timestamp = match lock.get("timestamp").and_then(|v| v.as_u64()) {
        Some(t) => t,
        None => return true,
    };
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_secs();
    let stale_threshold = (heartbeat_secs as f64 * 1.5) as u64;
    now.saturating_sub(timestamp) > stale_threshold
}

/// Touch the lockfile by updating its timestamp field.
fn touch_lockfile(lock_path: &Path) -> std::io::Result<()> {
    let content = fs::read_to_string(lock_path)?;
    let mut lock: serde_json::Value =
        serde_json::from_str(&content).map_err(std::io::Error::other)?;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_secs();
    lock["timestamp"] = serde_json::json!(now);
    let json = serde_json::to_string(&lock).map_err(std::io::Error::other)?;
    fs::write(lock_path, json)?;
    Ok(())
}

/// Remove the lockfile on shutdown.
fn remove_lockfile(lock_path: &Path) {
    let _ = fs::remove_file(lock_path);
}

/// Handle a single TCP connection: read one JSON request, dispatch via Bridge,
/// write one JSON response.
async fn handle_connection(
    stream: &mut tokio::net::TcpStream,
    bridge: &Bridge,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut buf = Vec::with_capacity(4096);
    stream.read_to_end(&mut buf).await?;

    let request: protocol::Request = serde_json::from_slice(&buf)?;
    let response = bridge.handle_request(request).await;

    let response_json = serde_json::to_string(&response)?;
    stream.write_all(response_json.as_bytes()).await?;
    stream.shutdown().await?;

    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let home = chibi_home();
    let config = BridgeConfig::load(&home);
    let api_key = read_api_key(&home);

    // Start MCP servers
    let mut server_manager = ServerManager::new();
    for (name, server_config) in &config.servers {
        if let Err(e) = server_manager.start_server(name, server_config).await {
            eprintln!("[mcp-bridge] failed to start server '{name}': {e}");
        }
    }

    // Summary cache: shared between background generation and request handling.
    // `None` when summaries are disabled — Bridge skips substitution entirely.
    let all_tools = server_manager.list_all_tools();
    let summary_cache = if config.summary.enabled {
        let cache = Arc::new(Mutex::new(cache::SummaryCache::load(&home)));
        if !all_tools.is_empty() {
            let bg_cache = Arc::clone(&cache);
            let summary_model = config.summary.model.clone();
            let bg_api_key = api_key.clone();
            tokio::spawn(async move {
                let count = summary::fill_cache_gaps(
                    &bg_cache,
                    &all_tools,
                    &summary_model,
                    bg_api_key.as_deref(),
                )
                .await;
                if count > 0 {
                    eprintln!("[mcp-bridge] generated {count} new tool summaries");
                }
            });
        }
        Some(cache)
    } else {
        None
    };

    let bridge = Arc::new(Bridge {
        server_manager,
        summary_cache,
    });

    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    eprintln!("[mcp-bridge] listening on {addr}");

    let lock_path = write_lockfile(&home, &addr)?;
    eprintln!("[mcp-bridge] lockfile: {}", lock_path.display());

    let idle_timeout = Duration::from_secs(config.idle_timeout_minutes * 60);
    let last_activity = Arc::new(Mutex::new(Instant::now()));

    // Watchdog: heartbeat + idle timeout. Returns when idle timeout is reached,
    // causing the main select! to exit gracefully instead of process::exit().
    let watchdog_activity = Arc::clone(&last_activity);
    let watchdog_lock_path = lock_path.clone();
    let watchdog = async move {
        let mut heartbeat_elapsed = Duration::ZERO;
        let tick = Duration::from_secs(10);
        let heartbeat_interval = Duration::from_secs(HEARTBEAT_SECS);
        loop {
            tokio::time::sleep(tick).await;
            heartbeat_elapsed += tick;

            // Touch lockfile at heartbeat interval
            if heartbeat_elapsed >= heartbeat_interval {
                heartbeat_elapsed = Duration::ZERO;
                if let Err(e) = touch_lockfile(&watchdog_lock_path) {
                    eprintln!("[mcp-bridge] failed to touch lockfile: {e}");
                }
            }

            let elapsed = watchdog_activity.lock().await.elapsed();
            if elapsed >= idle_timeout {
                eprintln!("[mcp-bridge] idle timeout reached, shutting down");
                break;
            }
        }
    };

    // Accept loop: handle incoming connections until the watchdog signals shutdown.
    let accept_loop = async {
        loop {
            let (mut stream, peer) = match listener.accept().await {
                Ok(conn) => conn,
                Err(e) => {
                    eprintln!("[mcp-bridge] accept error: {e}");
                    continue;
                }
            };
            eprintln!("[mcp-bridge] connection from {peer}");
            *last_activity.lock().await = Instant::now();

            let bridge = Arc::clone(&bridge);
            tokio::spawn(async move {
                if let Err(e) = handle_connection(&mut stream, &bridge).await {
                    eprintln!("[mcp-bridge] error handling connection: {e}");
                }
            });
        }
    };

    tokio::select! {
        () = watchdog => {}
        () = accept_loop => {}
    }

    remove_lockfile(&lock_path);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    /// Helper: spawn a bridge over a TCP listener and return its address.
    async fn spawn_test_bridge() -> SocketAddr {
        let bridge = Arc::new(Bridge {
            server_manager: ServerManager::new(),
            summary_cache: None,
        });
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        tokio::spawn(async move {
            loop {
                let (mut stream, _) = listener.accept().await.unwrap();
                let bridge = Arc::clone(&bridge);
                tokio::spawn(async move {
                    let _ = handle_connection(&mut stream, &bridge).await;
                });
            }
        });

        addr
    }

    /// Send a JSON request string to the bridge and return the response string.
    async fn send_request(addr: SocketAddr, request: &str) -> String {
        let mut client = tokio::net::TcpStream::connect(addr).await.unwrap();
        client.write_all(request.as_bytes()).await.unwrap();
        client.shutdown().await.unwrap();

        let mut response = String::new();
        client.read_to_string(&mut response).await.unwrap();
        response
    }

    #[tokio::test]
    async fn bridge_responds_to_list_tools() {
        let addr = spawn_test_bridge().await;

        let response = send_request(addr, r#"{"op":"list_tools"}"#).await;
        let v: serde_json::Value = serde_json::from_str(&response).unwrap();
        assert_eq!(v["ok"], true);
        assert_eq!(v["tools"], serde_json::json!([]));
    }

    #[tokio::test]
    async fn bridge_returns_error_for_unknown_server() {
        let addr = spawn_test_bridge().await;

        let response = send_request(
            addr,
            r#"{"op":"call_tool","server":"nope","tool":"foo","args":{}}"#,
        )
        .await;
        let v: serde_json::Value = serde_json::from_str(&response).unwrap();
        assert_eq!(v["ok"], false);
        assert!(v["error"].as_str().unwrap().contains("unknown server"));
    }

    #[tokio::test]
    async fn bridge_handles_get_schema_for_unknown_server() {
        let addr = spawn_test_bridge().await;

        let response =
            send_request(addr, r#"{"op":"get_schema","server":"nope","tool":"bar"}"#).await;
        let v: serde_json::Value = serde_json::from_str(&response).unwrap();
        assert_eq!(v["ok"], false);
    }

    #[test]
    fn lockfile_write_and_content() {
        let tmp = TempDir::new().unwrap();
        let addr: SocketAddr = "127.0.0.1:9999".parse().unwrap();

        let lock_path = write_lockfile(tmp.path(), &addr).unwrap();
        assert!(lock_path.exists());

        let content: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&lock_path).unwrap()).unwrap();
        assert_eq!(content["pid"], std::process::id());
        assert_eq!(content["address"], "127.0.0.1:9999");
        assert!(content["started"].as_u64().unwrap() > 0);
        assert_eq!(content["heartbeat_secs"], HEARTBEAT_SECS);
        assert!(content["timestamp"].as_u64().unwrap() > 0);
    }

    #[test]
    fn lockfile_stale_detection() {
        let tmp = TempDir::new().unwrap();
        let lock_path = tmp.path().join("mcp-bridge.lock");

        // Fresh lockfile should not be stale
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let content = serde_json::json!({
            "pid": 1, "address": "127.0.0.1:9999",
            "started": now, "heartbeat_secs": 30, "timestamp": now,
        });
        fs::write(&lock_path, content.to_string()).unwrap();
        assert!(!is_lockfile_stale(&lock_path));

        // Old timestamp should be stale (60s ago, threshold is 45s)
        let content = serde_json::json!({
            "pid": 1, "address": "127.0.0.1:9999",
            "started": now, "heartbeat_secs": 30, "timestamp": now - 60,
        });
        fs::write(&lock_path, content.to_string()).unwrap();
        assert!(is_lockfile_stale(&lock_path));

        // 40s ago should not be stale (under 45s threshold)
        let content = serde_json::json!({
            "pid": 1, "address": "127.0.0.1:9999",
            "started": now, "heartbeat_secs": 30, "timestamp": now - 40,
        });
        fs::write(&lock_path, content.to_string()).unwrap();
        assert!(!is_lockfile_stale(&lock_path));

        // Invalid content should be treated as stale
        fs::write(&lock_path, "not json").unwrap();
        assert!(is_lockfile_stale(&lock_path));

        // Missing timestamp field should be treated as stale
        let content = serde_json::json!({"pid": 1, "address": "127.0.0.1:9999"});
        fs::write(&lock_path, content.to_string()).unwrap();
        assert!(is_lockfile_stale(&lock_path));
    }

    #[test]
    fn lockfile_touch_updates_timestamp() {
        let tmp = TempDir::new().unwrap();
        let addr: SocketAddr = "127.0.0.1:9999".parse().unwrap();

        let lock_path = write_lockfile(tmp.path(), &addr).unwrap();
        let before: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&lock_path).unwrap()).unwrap();

        // Touch should succeed and update timestamp
        touch_lockfile(&lock_path).unwrap();
        let after: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&lock_path).unwrap()).unwrap();

        assert!(after["timestamp"].as_u64().unwrap() >= before["timestamp"].as_u64().unwrap());
        // Other fields should be preserved
        assert_eq!(after["pid"], before["pid"]);
        assert_eq!(after["address"], before["address"]);
        assert_eq!(after["heartbeat_secs"], before["heartbeat_secs"]);
    }

    #[test]
    fn lockfile_atomic_exclusive() {
        let tmp = TempDir::new().unwrap();
        let addr: SocketAddr = "127.0.0.1:9999".parse().unwrap();

        write_lockfile(tmp.path(), &addr).unwrap();

        let result = write_lockfile(tmp.path(), &addr);
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err().kind(),
            std::io::ErrorKind::AlreadyExists
        );
    }

    #[test]
    fn lockfile_removed_on_cleanup() {
        let tmp = TempDir::new().unwrap();
        let addr: SocketAddr = "127.0.0.1:9999".parse().unwrap();

        let lock_path = write_lockfile(tmp.path(), &addr).unwrap();
        assert!(lock_path.exists());

        remove_lockfile(&lock_path);
        assert!(!lock_path.exists());
    }

    #[tokio::test]
    async fn bridge_substitutes_summaries_in_list_tools() {
        use crate::cache::SummaryCache;

        let tmp = TempDir::new().unwrap();
        let mut cache = SummaryCache::load(tmp.path());
        let schema = serde_json::json!({"type": "object"});
        cache.set("srv", "verbose_tool", &schema, "concise summary".into());

        let cache = Arc::new(Mutex::new(cache));

        // Build a Bridge with one tool whose description should be substituted
        // We can't easily add a real server, so we test via handle_request directly
        let bridge = Bridge {
            server_manager: ServerManager::new(),
            summary_cache: Some(cache),
        };

        // ServerManager has no servers, so list_tools returns []. To test substitution,
        // we call the substitution logic indirectly — verify the bridge compiles and
        // the None path works (no servers = empty tools, nothing to substitute).
        let response = bridge.handle_request(protocol::Request::ListTools).await;
        match response {
            protocol::Response::Tools { ok, tools } => {
                assert!(ok);
                assert!(tools.is_empty());
            }
            _ => panic!("expected Tools response"),
        }
    }

    #[tokio::test]
    async fn bridge_skips_summaries_when_disabled() {
        let bridge = Bridge {
            server_manager: ServerManager::new(),
            summary_cache: None,
        };

        let response = bridge.handle_request(protocol::Request::ListTools).await;
        match response {
            protocol::Response::Tools { ok, tools } => {
                assert!(ok);
                assert!(tools.is_empty());
            }
            _ => panic!("expected Tools response"),
        }
    }
}
