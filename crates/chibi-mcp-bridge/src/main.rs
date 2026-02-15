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

/// Lockfile content written atomically on startup.
#[derive(serde::Serialize)]
struct LockContent {
    pid: u32,
    address: String,
    started: u64,
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

/// Write bridge lockfile atomically (O_CREAT | O_EXCL).
/// Returns `AlreadyExists` if another bridge instance is running.
fn write_lockfile(home: &Path, addr: &SocketAddr) -> std::io::Result<PathBuf> {
    let lock_path = home.join("mcp-bridge.lock");

    if let Some(parent) = lock_path.parent() {
        fs::create_dir_all(parent)?;
    }

    let content = LockContent {
        pid: std::process::id(),
        address: addr.to_string(),
        started: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or(Duration::ZERO)
            .as_secs(),
    };

    let json = serde_json::to_string(&content).map_err(std::io::Error::other)?;

    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&lock_path)?;

    file.write_all(json.as_bytes())?;
    file.sync_all()?;

    Ok(lock_path)
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

    // Start MCP servers
    let mut server_manager = ServerManager::new();
    for (name, server_config) in &config.servers {
        if let Err(e) = server_manager.start_server(name, server_config).await {
            eprintln!("[mcp-bridge] failed to start server '{name}': {e}");
        }
    }

    // Spawn background summary generation (if enabled)
    let all_tools = server_manager.list_all_tools();
    if config.summary.enabled && !all_tools.is_empty() {
        let summary_home = home.clone();
        let summary_model = config.summary.model.clone();
        tokio::spawn(async move {
            let mut cache = cache::SummaryCache::load(&summary_home);
            let count = summary::fill_cache_gaps(&mut cache, &all_tools, &summary_model).await;
            if count > 0 {
                eprintln!("[mcp-bridge] generated {count} new tool summaries");
            }
        });
    }

    let bridge = Arc::new(Bridge { server_manager });

    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    eprintln!("[mcp-bridge] listening on {addr}");

    let lock_path = write_lockfile(&home, &addr)?;
    eprintln!("[mcp-bridge] lockfile: {}", lock_path.display());

    let idle_timeout = Duration::from_secs(config.idle_timeout_minutes * 60);
    let last_activity = Arc::new(Mutex::new(Instant::now()));

    // Spawn idle timeout watchdog
    let watchdog_activity = Arc::clone(&last_activity);
    let watchdog_lock_path = lock_path.clone();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(10)).await;
            let elapsed = watchdog_activity.lock().await.elapsed();
            if elapsed >= idle_timeout {
                eprintln!("[mcp-bridge] idle timeout reached, shutting down");
                remove_lockfile(&watchdog_lock_path);
                std::process::exit(0);
            }
        }
    });

    // Handle incoming connections
    loop {
        let (mut stream, peer) = listener.accept().await?;
        eprintln!("[mcp-bridge] connection from {peer}");
        *last_activity.lock().await = Instant::now();

        let bridge = Arc::clone(&bridge);
        tokio::spawn(async move {
            if let Err(e) = handle_connection(&mut stream, &bridge).await {
                eprintln!("[mcp-bridge] error handling connection: {e}");
            }
        });
    }
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
}
