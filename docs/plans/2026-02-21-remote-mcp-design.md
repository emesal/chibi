# remote MCP server support design

issue: TBD

## summary

add native support for remote MCP servers over streamable HTTP transport.
currently the bridge only spawns local child processes via stdio — this extends
it to connect to remote MCP endpoints, making `chibi-mcp-bridge` a universal
MCP client.

## config

`ServerConfig` becomes an untagged enum discriminated by field presence:

```toml
# local server (stdio transport, unchanged)
[servers.serena]
command = "uvx"
args = ["serena"]

# remote server (streamable HTTP transport, new)
[servers.remote-ai]
url = "https://my-server.com/mcp"

# remote server with auth headers
[servers.remote-ai-auth]
url = "https://my-server.com/mcp"

[servers.remote-ai-auth.headers]
Authorization = "Bearer sk-..."
X-Custom = "value"
```

presence of `command` → stdio. presence of `url` → streamable HTTP.

## approach: enum-based ServerConfig

```rust
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum ServerConfig {
    Stdio {
        command: String,
        #[serde(default)]
        args: Vec<String>,
    },
    StreamableHttp {
        url: String,
        #[serde(default)]
        headers: HashMap<String, String>,
    },
}
```

serde untagged tries variants in order. a post-parse validation pass catches
ambiguous configs (both `command` and `url` present) and gives a clear error,
since untagged would silently match the first variant and discard `url`.

## transport instantiation

`start_server` pattern-matches on the enum variant:

- `Stdio` — existing `TokioChildProcess` path (unchanged)
- `StreamableHttp` — constructs rmcp `StreamableHttpClientTransport` with
  the URL and injects custom headers via reqwest `HeaderMap`

a `build_streamable_http(url, headers)` helper handles transport construction,
URL validation, and header parsing.

## dependencies

add rmcp feature `transport-streamable-http-client-reqwest` to Cargo.toml.
this pulls in reqwest with rustls-tls.

## touch points

1. `config.rs` — `ServerConfig` enum + post-parse validation
2. `server.rs` — transport match in `start_server` + `build_streamable_http`
3. `Cargo.toml` — new rmcp feature flag
4. `docs/mcp.md` — remote server documentation
5. config tests — streamable HTTP parsing, mixed configs, ambiguous rejection

## error handling

- URL validation via reqwest's own parsing (or `url::Url`)
- connection failures surface as "server 'foo': failed to connect to <url>"
- invalid header names/values caught at parse time

## design notes

- chibi-core is unaffected — the bridge's TCP protocol is transport-agnostic.
  chibi-core doesn't know whether a server is local or remote.
- no SSE support — streamable HTTP is the current MCP standard, SSE is
  deprecated. if needed later, rmcp supports it and the enum extends trivially.
