# chibi upgrade notes

this file only documents breaking changes that require user action.

## 0.7.0 -> 0.8.0

- **JSON mode moved to `chibi-json` binary**: the `--json-config`, `--json-output`, and `--json-schema` flags are removed from `chibi`. use the new `chibi-json` binary instead — it reads JSON from stdin, emits JSONL output, and always runs in trust mode.
  - **migration**: `chibi --json-config --json-output < input.json` → `chibi-json < input.json`
  - **migration**: `chibi --json-schema` → `chibi-json --json-schema`
  - `cargo install --path .` installs both binaries

- **fuel budget replaces recursion depth**: the agentic loop now uses a unified fuel budget instead of separate recursion depth and empty response counters.
  - `max_recursion_depth` → `fuel` (default: 30)
  - `max_empty_responses` → `fuel_empty_response_cost` (default: 15)
  - **migration**: update `config.toml` and any `local.toml` files to use the new field names
  - fuel consumed per: tool-call round (1), agent continuation (1), empty response (`fuel_empty_response_cost`). first turn is free.
  - hooks can set fuel (`pre_agentic_loop`) or adjust it (`post_tool_batch` via `fuel_delta`)

- **7 plugins removed, replaced by builtins**: the following plugins are no longer in chibi-plugins and are now built-in tools:
  - `read_file` → `file_head` / `file_lines`
  - `run_command` → `shell_exec`
  - `recurse` → `call_agent`
  - `fetch_url` → `fetch_url` (now a coding tool)
  - `read_context` → `read_context` (now a builtin)
  - `fetch-mcp` → removed (no replacement)
  - `github-mcp` → removed (no replacement)
  - **migration**: delete old plugin scripts from `~/.chibi/plugins/`, the built-in tools activate automatically

- **plugins submodule removed from chibi-dev**: plugins are now managed independently in the [chibi-plugins](https://github.com/emesal/chibi-plugins) repo. install individually by symlinking or copying to `~/.chibi/plugins/`.

- **models.toml: `context_window` and `supports_tool_calls` removed**: model capabilities now come from ratatoskr's built-in registry automatically. the global `context_window_limit` in config.toml is unchanged (and now optional — defaults to auto-resolution from ratatoskr).
  - **migration**: delete per-model `context_window` and `supports_tool_calls` fields from `~/.chibi/models.toml`. keep `[models."...".api]` parameter overrides.

- **zero-config defaults**: `api_key`, `model`, and `context_window_limit` are now all optional in config.toml. a bare config.toml (or no config.toml at all) works for free-tier OpenRouter access.

- **permission prompts default to Y/n**: permission prompts now default to yes instead of no. use `--trust` (`-t`) for full auto-approval in automation/headless environments. fail-safe deny still applies when no TTY and no `--trust`.

- **permission system now deny-only**: custom permission plugins should return `{"denied": true}` to block, or `{}` for no opinion. the old allow/deny model is removed.

- **AGENTS.md auto-loading**: chibi now auto-detects project root via VCS markers (`.git`, `.hg`, etc.) and loads AGENTS.md files from `~/AGENTS.md`, `~/.chibi/AGENTS.md`, project root, and each directory down to cwd. no configuration needed — this is automatic behaviour.

- **new `[tools]` config section**: filter tools by name or category. `exclude_categories` supports `builtin`, `file`, `agent`, `coding`. no action needed — additive feature.

## 0.6.0 -> 0.7.0

- **plugin communication changed**: plugins now receive parameters via stdin instead of environment variables
  - tools: read JSON params from stdin (was `CHIBI_TOOL_ARGS` env var)
  - hooks: read JSON data from stdin (was `CHIBI_HOOK_DATA` env var)
  - `CHIBI_HOOK`, `CHIBI_TOOL_NAME`, and `CHIBI_VERBOSE` env vars unchanged
  - **migration**: replace `json.loads(os.environ["CHIBI_TOOL_ARGS"])` with `json.load(sys.stdin)`
  - **migration**: replace `json.loads(os.environ["CHIBI_HOOK_DATA"])` with `json.load(sys.stdin)`

## 0.5.1 -> 0.6.0

- **ratatoskr integration**: LLM communication now uses the [ratatoskr](https://github.com/emesal/ratatoskr) crate instead of direct HTTP calls
  - HTTP/SSE streaming is now handled by ratatoskr's `ModelGateway`
  - chibi's `gateway.rs` provides type conversions between internal types and ratatoskr
  - some API parameters not yet passed through — see [#109](https://github.com/emesal/chibi/issues/109)
- **removed `base_url`**: custom API endpoints are not currently supported

## 0.5.0 -> 0.5.1

- transcript.jsonl will be automatically migrated to a partitioned system
- `-d`/`-D` renamed from `--delete-*` to `--destroy-*`
- `delete_context` command renamed to `destroy_context` (JSON input)
- other changes

## 0.4.1 -> 0.5.0

- completely reworked code
  - context representation on disk changed
  - CLI changed
  - everything changed :3
  - but documentation exists now

now ready to start tracking changes for user convenience

## 0.4.0 -> 0.4.1
- new context state format -> clear (0.3) or archive (0.4) contexts before upgrading to preserve history (see --help)
- human-readable transcripts are now md files. if wanted, old transcripts can be migrated with

```bash
find $HOME/.chibi/contexts -type f -name "transcript.txt" -exec sh -c 'mv -i "$1" "${1%.txt}.md"' _ {} \;
```

## 0.3.0 -> 0.4.0
- CLI parameters changed! Existing scripts need to be updated.
