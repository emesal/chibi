# retrieve_content security boundary fix [DONE]

> audit item 1: `retrieve_content` bypasses `file_tools_allowed_paths`

## problem

`agent_tools::read_file()` does raw `std::fs::read_to_string` with only tilde expansion — no path validation against `file_tools_allowed_paths`. an LLM can call `retrieve_content` with `source: "/etc/shadow"` to circumvent the allowlist. `fetch_url()` has no SSRF protection (localhost, cloud metadata, private ranges).

## design

### new module: `tools/security.rs`

**`validate_file_path(path, config) -> io::Result<PathBuf>`**
- extracted (moved) from `file_tools.rs::resolve_and_validate_path`
- tilde expansion → canonicalize → check `file_tools_allowed_paths`
- single source of truth for both `file_tools` and `agent_tools`

**`UrlSafety` enum**: `Safe` | `Sensitive(String)`

**`classify_url(url) -> UrlSafety`**
- parse URL, resolve hostname to IP
- sensitive ranges: loopback (127.0.0.0/8, ::1), link-local (169.254.0.0/16, fe80::/10), private RFC 1918 (10/8, 172.16/12, 192.168/16), cloud metadata (169.254.169.254), `localhost` hostname
- returns classification with human-readable reason

### changes to `agent_tools.rs`

- `read_file(path)` → `read_file(path, config)`, calls `security::validate_file_path`
- `retrieve_content`: for URLs classified as `Sensitive`, fires `PreFetchUrl` hook → `evaluate_permission` → deny/approve before fetching

### new hook: `PreFetchUrl`

- hook data: `{"tool_name": "retrieve_content", "url": "...", "safety": "sensitive", "reason": "loopback address"}`
- added to `hooks.rs` (count becomes 30)
- follows established pattern (`PreShellExec`, `PreFileWrite`)

### permission flow by binary

| binary    | sensitive URL behaviour                        |
|-----------|------------------------------------------------|
| chibi-cli | prompt via interactive permission handler       |
| chibi-cli `--trust` | auto-approve                        |
| chibi-json | auto-approve (programmatic caller decides)    |

chibi-json URL policy configuration is tracked separately in #147.

### files changed

| file | change |
|------|--------|
| `tools/security.rs` | new — `validate_file_path`, `classify_url`, `UrlSafety` |
| `tools/mod.rs` | add `pub mod security` + re-exports |
| `tools/file_tools.rs` | remove `resolve_and_validate_path`, import from `security` |
| `tools/agent_tools.rs` | `read_file` uses `validate_file_path`, `retrieve_content` fires `PreFetchUrl` hook for sensitive URLs |
| `tools/hooks.rs` | add `PreFetchUrl`, update count |
| `api/send.rs` | wire `PreFetchUrl` through `evaluate_permission` in `execute_tool_pure` |
