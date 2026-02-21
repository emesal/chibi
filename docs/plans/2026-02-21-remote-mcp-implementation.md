# Remote MCP Server Support Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add native streamable HTTP transport support so chibi-mcp-bridge can connect to remote MCP servers.

**Architecture:** `ServerConfig` becomes an untagged serde enum (Stdio vs StreamableHttp), discriminated by field presence (`command` vs `url`). `start_server` pattern-matches on the variant to construct the appropriate rmcp transport. `ManagedServer` is unchanged — `RunningService<RoleClient, ()>` is transport-agnostic.

**Tech Stack:** rmcp 0.15 (`transport-streamable-http-client-reqwest` feature), reqwest (via rmcp), serde untagged enum

---

### Task 1: Add rmcp streamable HTTP feature

**Files:**
- Modify: `crates/chibi-mcp-bridge/Cargo.toml`

**Step 1: Add the feature flag**

In `Cargo.toml`, change the rmcp dependency:

```toml
rmcp = { version = "0.15", features = ["client", "transport-child-process", "transport-streamable-http-client-reqwest"] }
```

**Step 2: Verify it compiles**

Run: `cargo check -p chibi-mcp-bridge`
Expected: compiles with no errors

**Step 3: Commit**

```bash
git add crates/chibi-mcp-bridge/Cargo.toml
git commit -m "chore(mcp-bridge): add rmcp streamable HTTP transport feature"
```

---

### Task 2: Convert ServerConfig to enum

**Files:**
- Modify: `crates/chibi-mcp-bridge/src/config.rs`

**Step 1: Write the failing tests**

Add these tests to the existing `tests` module in `config.rs`:

```rust
#[test]
fn parse_streamable_http_server() {
    let toml = r#"
[servers.remote]
url = "https://my-server.com/mcp"
"#;
    let cfg: BridgeConfig = toml::from_str(toml).unwrap();
    assert_eq!(cfg.servers.len(), 1);
    match &cfg.servers["remote"] {
        ServerConfig::StreamableHttp { url, headers } => {
            assert_eq!(url, "https://my-server.com/mcp");
            assert!(headers.is_empty());
        }
        other => panic!("expected StreamableHttp, got {other:?}"),
    }
}

#[test]
fn parse_streamable_http_with_headers() {
    let toml = r#"
[servers.remote]
url = "https://my-server.com/mcp"

[servers.remote.headers]
Authorization = "Bearer sk-test"
X-Custom = "value"
"#;
    let cfg: BridgeConfig = toml::from_str(toml).unwrap();
    match &cfg.servers["remote"] {
        ServerConfig::StreamableHttp { url, headers } => {
            assert_eq!(url, "https://my-server.com/mcp");
            assert_eq!(headers["Authorization"], "Bearer sk-test");
            assert_eq!(headers["X-Custom"], "value");
        }
        other => panic!("expected StreamableHttp, got {other:?}"),
    }
}

#[test]
fn parse_mixed_servers() {
    let toml = r#"
[servers.local]
command = "serena"
args = ["--stdio"]

[servers.remote]
url = "https://my-server.com/mcp"
"#;
    let cfg: BridgeConfig = toml::from_str(toml).unwrap();
    assert_eq!(cfg.servers.len(), 2);
    assert!(matches!(cfg.servers["local"], ServerConfig::Stdio { .. }));
    assert!(matches!(cfg.servers["remote"], ServerConfig::StreamableHttp { .. }));
}

#[test]
fn stdio_config_preserves_existing_behaviour() {
    let toml = r#"
[servers.serena]
command = "serena"
args = ["--stdio"]
"#;
    let cfg: BridgeConfig = toml::from_str(toml).unwrap();
    match &cfg.servers["serena"] {
        ServerConfig::Stdio { command, args } => {
            assert_eq!(command, "serena");
            assert_eq!(args, &["--stdio"]);
        }
        other => panic!("expected Stdio, got {other:?}"),
    }
}
```

**Step 2: Run tests to verify they fail**

Run: `cargo test -p chibi-mcp-bridge`
Expected: FAIL — `ServerConfig` is still a struct, no `Stdio`/`StreamableHttp` variants

**Step 3: Convert ServerConfig to enum**

Replace the `ServerConfig` struct with:

```rust
/// MCP server definition — either a local stdio process or a remote HTTP endpoint.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum ServerConfig {
    /// Local server spawned as a child process (stdio transport).
    Stdio {
        command: String,
        #[serde(default)]
        args: Vec<String>,
    },
    /// Remote server accessed via streamable HTTP transport.
    StreamableHttp {
        url: String,
        #[serde(default)]
        headers: HashMap<String, String>,
    },
}
```

**Step 4: Fix existing tests**

Update existing tests that access `s.command` / `s.args` to pattern-match instead:

- `parse_one_server`: match on `ServerConfig::Stdio { command, args }`, assert values
- `server_args_default_to_empty`: same pattern

**Step 5: Fix server.rs compilation**

In `server.rs`, `start_server` currently accesses `config.command` and `config.args` directly. Update to match on `ServerConfig::Stdio`:

```rust
pub async fn start_server(
    &mut self,
    name: &str,
    config: &ServerConfig,
) -> Result<(), Box<dyn std::error::Error>> {
    let service = match config {
        ServerConfig::Stdio { command, args } => {
            let mut cmd = tokio::process::Command::new(command);
            cmd.args(args);
            let transport = TokioChildProcess::new(cmd)?;
            ().serve(transport).await?
        }
        ServerConfig::StreamableHttp { .. } => {
            return Err(format!("server '{name}': streamable HTTP not yet implemented").into());
        }
    };

    let ListToolsResult { tools, .. } = service.list_tools(Default::default()).await?;

    eprintln!(
        "[mcp-bridge] server '{name}': {} tools discovered",
        tools.len()
    );

    self.servers
        .insert(name.to_string(), ManagedServer { service, tools });

    Ok(())
}
```

**Step 6: Run tests to verify they pass**

Run: `cargo test -p chibi-mcp-bridge`
Expected: all tests PASS

**Step 7: Commit**

```bash
git add crates/chibi-mcp-bridge/src/config.rs crates/chibi-mcp-bridge/src/server.rs
git commit -m "refactor(mcp-bridge): convert ServerConfig to enum for multi-transport support"
```

---

### Task 3: Implement streamable HTTP transport

**Files:**
- Modify: `crates/chibi-mcp-bridge/src/server.rs`

**Step 1: Add the StreamableHttp arm**

Replace the placeholder `StreamableHttp` arm in `start_server` with:

```rust
ServerConfig::StreamableHttp { url, headers } => {
    use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
    use rmcp::transport::StreamableHttpClientTransport;

    let config = StreamableHttpClientTransportConfig::with_uri(url.as_str());

    let transport = if headers.is_empty() {
        StreamableHttpClientTransport::from_config(config)
    } else {
        let mut header_map = reqwest::header::HeaderMap::new();
        for (key, value) in headers {
            let name = reqwest::header::HeaderName::from_bytes(key.as_bytes())
                .map_err(|e| format!("server '{name}': invalid header name '{key}': {e}"))?;
            let val = reqwest::header::HeaderValue::from_str(value)
                .map_err(|e| format!("server '{name}': invalid header value for '{key}': {e}"))?;
            header_map.insert(name, val);
        }
        let client = reqwest::Client::builder()
            .default_headers(header_map)
            .build()
            .map_err(|e| format!("server '{name}': failed to build HTTP client: {e}"))?;
        StreamableHttpClientTransport::with_client(client, config)
    };

    ().serve(transport).await?
}
```

**Step 2: Add reqwest import**

At the top of `server.rs`, the `reqwest` crate is available transitively through rmcp. No new Cargo dependency needed — rmcp re-exports it or we use it directly since `transport-streamable-http-client-reqwest` pulls it in. Verify with a `cargo check`.

Run: `cargo check -p chibi-mcp-bridge`

If reqwest isn't directly accessible, add to Cargo.toml:
```toml
reqwest = { version = "0.12", default-features = false, features = ["rustls-tls"] }
```

**Step 3: Run tests to verify nothing broke**

Run: `cargo test -p chibi-mcp-bridge`
Expected: all tests PASS (no integration test for remote yet — that requires a running server)

**Step 4: Commit**

```bash
git add crates/chibi-mcp-bridge/src/server.rs
git commit -m "feat(mcp-bridge): implement streamable HTTP transport for remote MCP servers"
```

---

### Task 4: Add config validation

**Files:**
- Modify: `crates/chibi-mcp-bridge/src/config.rs`

**Step 1: Write the validation test**

Add to `config.rs` tests:

```rust
#[test]
fn validate_rejects_empty_server() {
    let toml = r#"
[servers.bad]
args = ["--flag"]
"#;
    let result: Result<BridgeConfig, _> = toml::from_str(toml);
    assert!(result.is_err(), "server with neither command nor url should fail to parse");
}
```

**Step 2: Run test to verify behaviour**

Run: `cargo test -p chibi-mcp-bridge -- validate_rejects_empty_server`

With serde untagged, a server that has neither `command` nor `url` will fail deserialization (neither variant matches). Verify this test passes as-is. If it does, no additional validation code is needed — serde handles it.

**Step 3: Add a post-parse validation method for ambiguous configs**

Note: serde untagged will match `Stdio` first if both `command` and `url` are present, silently ignoring `url`. Add a validation method on `BridgeConfig`:

```rust
impl BridgeConfig {
    /// Validate config after parsing. Warns about ambiguous server configs.
    pub fn validate(&self) {
        // Note: serde untagged can't detect ambiguity directly.
        // We parse the raw TOML to check for servers with both `command` and `url`.
        // This is intentionally best-effort — we warn rather than error.
    }
}
```

Actually, detecting this properly requires re-parsing the TOML as raw `toml::Value` and checking for co-occurrence. This is YAGNI for now — the config semantics are clear ("use `command` OR `url`") and the docs will make this explicit. Skip the ambiguity check.

**Step 4: Commit**

```bash
git add crates/chibi-mcp-bridge/src/config.rs
git commit -m "test(mcp-bridge): add validation test for empty server config"
```

---

### Task 5: Update documentation

**Files:**
- Modify: `docs/mcp.md`
- Modify: `docs/plans/2026-02-21-remote-mcp-design.md` (if needed)

**Step 1: Add remote server section to docs/mcp.md**

After the "### 2. Configure MCP servers" section, add a new subsection for remote servers. Update the config reference table to include `url` and `headers` fields. Add an example showing mixed local and remote configs.

Key additions:
- "### Remote servers" section explaining the `url` field
- Headers example for auth
- Note that `command` and `url` are mutually exclusive
- Update the config reference table

**Step 2: Review and commit**

```bash
git add docs/mcp.md
git commit -m "docs(mcp): add remote server configuration documentation"
```

---

### Task 6: Final verification

**Step 1: Run full test suite**

Run: `cargo test`
Expected: all tests PASS

**Step 2: Run clippy**

Run: `cargo clippy -p chibi-mcp-bridge -- -D warnings`
Expected: no warnings

**Step 3: Run fmt check**

Run: `cargo fmt -- --check`
Expected: no formatting issues
