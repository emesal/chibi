# Plugin Audit Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** audit all plugins for redundancy/breakage, add `fetch_url` builtin, remove redundant plugins, and remove the plugins submodule from chibi-dev.

**Architecture:** `fetch_url` becomes a new coding tool in `coding_tools.rs`, following the existing pattern (async fn, reqwest GET, returns body or structured error). plugin removals happen in the plugins submodule repo. finally the submodule reference is removed from chibi-dev.

**Tech Stack:** rust, reqwest (already a workspace dependency), git submodules

---

### Task 1: Add `fetch_url` builtin tool

**Files:**
- Modify: `crates/chibi-core/src/tools/coding_tools.rs`
- Modify: `crates/chibi-core/src/tools/mod.rs`

**Step 1: Add the tool constant and definition**

In `coding_tools.rs`, add after the existing constants:

```rust
pub const FETCH_URL_TOOL_NAME: &str = "fetch_url";
```

Add to `CODING_TOOL_DEFS` array:

```rust
BuiltinToolDef {
    name: FETCH_URL_TOOL_NAME,
    description: "Fetch content from a URL via HTTP GET and return the response body. Follows redirects. Use for retrieving web pages, API responses, or raw file content.",
    properties: &[
        ToolPropertyDef {
            name: "url",
            prop_type: "string",
            description: "URL to fetch (must start with http:// or https://)",
            default: None,
        },
        ToolPropertyDef {
            name: "max_bytes",
            prop_type: "integer",
            description: "Maximum response body size in bytes (default: 1048576 = 1MB)",
            default: Some(1_048_576),
        },
        ToolPropertyDef {
            name: "timeout_secs",
            prop_type: "integer",
            description: "Request timeout in seconds (default: 30)",
            default: Some(30),
        },
    ],
    required: &["url"],
    summary_params: &["url"],
},
```

**Step 2: Add the execution function**

```rust
/// Execute fetch_url: HTTP GET a URL and return the response body.
///
/// Follows redirects (up to 10). Validates URL scheme (http/https only).
/// Limits response body to `max_bytes` to prevent unbounded memory usage.
async fn execute_fetch_url(args: &serde_json::Value) -> io::Result<String> {
    let url = require_str_param(args, "url")?;
    let max_bytes = args.get_u64_or("max_bytes", 1_048_576) as usize;
    let timeout_secs = args.get_u64_or("timeout_secs", 30);

    // Validate URL scheme
    if !url.starts_with("http://") && !url.starts_with("https://") {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("URL must start with http:// or https://, got: {}", url),
        ));
    }

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(timeout_secs))
        .redirect(reqwest::redirect::Policy::limited(10))
        .build()
        .map_err(|e| io::Error::other(format!("Failed to create HTTP client: {}", e)))?;

    let response = client
        .get(&url)
        .send()
        .await
        .map_err(|e| io::Error::other(format!("Request failed: {}", e)))?;

    let status = response.status();
    if !status.is_success() {
        return Err(io::Error::other(format!(
            "HTTP {} {}",
            status.as_u16(),
            status.canonical_reason().unwrap_or("Unknown"),
        )));
    }

    // Read body with size limit
    let bytes = response
        .bytes()
        .await
        .map_err(|e| io::Error::other(format!("Failed to read response body: {}", e)))?;

    if bytes.len() > max_bytes {
        let truncated = String::from_utf8_lossy(&bytes[..max_bytes]);
        Ok(format!(
            "{}\n\n[Truncated: response was {} bytes, limit is {} bytes]",
            truncated,
            bytes.len(),
            max_bytes,
        ))
    } else {
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    }
}
```

**Step 3: Wire into dispatch**

In `execute_coding_tool` match, add:

```rust
FETCH_URL_TOOL_NAME => Some(execute_fetch_url(args).await),
```

In `is_coding_tool` match, add `FETCH_URL_TOOL_NAME`.

**Step 4: Update mod.rs exports**

Add `FETCH_URL_TOOL_NAME` to the re-export line in `mod.rs`.

**Step 5: Update registry test**

Update `test_coding_tool_registry_contains_all_tools` to expect 9 tools and include `FETCH_URL_TOOL_NAME`. Update `test_is_coding_tool` to assert `is_coding_tool(FETCH_URL_TOOL_NAME)`.

**Step 6: Add tests**

```rust
#[tokio::test]
async fn test_fetch_url_invalid_scheme() {
    let a = args(&[("url", serde_json::json!("ftp://example.com"))]);
    let result = execute_fetch_url(&a).await;
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("http://"));
}

#[tokio::test]
async fn test_fetch_url_missing_param() {
    let a = args(&[]);
    let result = execute_fetch_url(&a).await;
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("Missing"));
}
```

**Step 7: Build and test**

Run: `cargo build && cargo test -p chibi-core`

**Step 8: Commit**

```
feat: add fetch_url builtin tool
```

---

### Task 2: Remove redundant plugins from plugins repo

**Context:** the plugins are in a submodule at `plugins/` pointing to `https://github.com/emesal/chibi-plugins.git`. we need to remove 6 plugins from that repo.

**Step 1: Remove plugin directories**

In the `plugins/` submodule directory, delete:
- `read_file/`
- `run_command/`
- `recurse/`
- `fetch_url/`
- `fetch-mcp/`
- `github-mcp/`

**Step 2: Update plugins README**

Update `plugins/README.md` to remove the deleted plugins from any listings/documentation, and add a note that `fetch_url` is now a builtin.

**Step 3: Commit in submodule**

```
refactor: remove redundant plugins

- read_file: superseded by file_head/file_lines builtins
- run_command: superseded by shell_exec builtin + pre_shell_exec hook
- recurse: superseded by call_agent builtin
- fetch_url: now a builtin tool in chibi-core
- fetch-mcp: redundant MCP wrapper, fetch_url builtin covers this
- github-mcp: broken, superseded by gh CLI via shell_exec
```

**Step 4: Push submodule changes**

Push the submodule commit to the plugins repo.

---

### Task 3: Remove submodule from chibi-dev

**Step 1: Deinit and remove submodule**

```bash
git submodule deinit -f plugins
git rm -f plugins
rm -rf .git/modules/plugins
```

**Step 2: Verify .gitmodules is removed**

The `git rm` should have cleaned up `.gitmodules`. Verify it's gone or empty.

**Step 3: Commit**

```
refactor: remove plugins submodule

the plugins repo (chibi-plugins) now lives independently.
no build dependency exists — users install plugins individually.
```

---

### Task 4: Update AGENTS.md documentation

**Files:**
- Modify: `AGENTS.md`

**Step 1: Update the plugins section**

Ensure the plugins documentation reflects:
- `fetch_url` is now a builtin coding tool
- the plugins submodule no longer exists
- mention that plugins are installed from the separate chibi-plugins repo

**Step 2: Commit**

```
docs: update AGENTS.md for plugin audit changes
```

---

### Task 5: Update issue #131

**Step 1: Check off completed tasks on the issue**

**Step 2: Close the issue with a comment summarising what was done**

Summary:
- added `fetch_url` builtin
- removed 6 redundant plugins (read_file, run_command, recurse, fetch_url, fetch-mcp, github-mcp)
- removed plugins submodule from chibi-dev
- kept: agent-skills, web_search, coffee-table, file-permission, hook-inspector, bofh_in_the_shell, hello_chibi
