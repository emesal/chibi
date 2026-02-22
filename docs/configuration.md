# Configuration Reference

Chibi uses a layered configuration system with separate files for core and CLI settings.

## Core Configuration

Settings are resolved in this order (later overrides earlier):

1. **Defaults** - Built-in default values
2. **Global config** (`~/.chibi/config.toml`) - User's base configuration
3. **Environment variables** - `CHIBI_API_KEY`, `CHIBI_MODEL` (see [below](#environment-variables))
4. **Model metadata** (`~/.chibi/models.toml`) - Per-model settings
5. **Context config** (`~/.chibi/contexts/<name>/local.toml`) - Per-context overrides
6. **CLI flags** - Command-line arguments (highest priority)

## CLI Presentation Configuration

Presentation settings (markdown rendering, images, color themes) live in separate files:

1. **Global CLI config** (`~/.chibi/cli.toml`) - CLI presentation settings
2. **Context CLI config** (`~/.chibi/contexts/<name>/cli.toml`) - Per-context overrides

## Home Directory

By default, chibi stores all data in `~/.chibi`. This can be overridden:

1. `--home <PATH>` CLI flag (highest priority)
2. `CHIBI_HOME` environment variable
3. `~/.chibi` default

Use `-n home` to see the resolved path.

## Project Root

Chibi auto-detects the project root for context-aware features (AGENTS.md loading, codebase indexing). Resolution order:

1. `--project-root` CLI flag (highest priority)
2. `CHIBI_PROJECT_ROOT` environment variable
3. VCS root detection (walk up from cwd)
4. Current working directory (fallback)

**Supported VCS markers** (checked in order, nearest match wins):

| Marker | VCS |
|--------|-----|
| `.git` (dir or file) | Git (incl. worktrees, submodules) |
| `.hg/` | Mercurial |
| `.svn/` | Subversion |
| `.bzr/` | Bazaar |
| `.pijul/` | Pijul |
| `.jj/` | Jujutsu |
| `.fslckout` (file) | Fossil |
| `_FOSSIL_` (file) | Fossil (alt) |
| `CVS/` | CVS (walks up to highest containing dir) |

## AGENTS.md

Chibi loads instruction files from standard locations and injects them into the system prompt. Files are concatenated in order; later entries appear later in the prompt and can effectively override earlier guidance.

**Discovery locations** (in order):

1. `~/AGENTS.md` — user-global, tool-independent instructions
2. `~/.chibi/AGENTS.md` — chibi-global instructions
3. `<project_root>/AGENTS.md` — project root
4. Each directory from project root down to cwd (e.g. `<project_root>/packages/frontend/AGENTS.md`)

Empty files are skipped. When cwd equals project root, the root file appears only once.

Content appears in the system prompt under `--- AGENT INSTRUCTIONS ---`, after the base prompt and before context metadata.

## Global Configuration (config.toml)

Create `~/.chibi/config.toml` (or `<CHIBI_HOME>/config.toml` if overridden):

All fields are optional. chibi works with no config file at all (free-tier OpenRouter, default model).

```toml
# =============================================================================
# Core Settings (all optional)
# =============================================================================

# API key for OpenRouter (https://openrouter.ai/settings/keys)
# Omit for free-tier access (no key needed)
# api_key = "your-api-key-here"

# Model to use (default: ratatoskr:free/agentic)
# model = "anthropic/claude-sonnet-4"

# Context window limit in tokens (default: fetched from ratatoskr registry)
# context_window_limit = 200000

# Warning threshold percentage (default: 80.0)
# warn_threshold_percent = 80.0

# =============================================================================
# Optional Settings
# =============================================================================

# Default username shown to the LLM (default: "user")
username = "user"

# Omit tools from API requests entirely for pure text mode (default: false)
no_tool_calls = false

# Fallback tool when LLM doesn't explicitly call call_agent/call_user
# Options: "call_user" (return to user) or "call_agent" (continue loop)
fallback_tool = "call_user"

# Cost tier for resolving subagent model presets (default: "free")
# Controls which ratatoskr preset tier is used when spawn_agent is given a
# preset capability name (e.g. "fast", "reasoning") instead of an explicit model.
# subagent_cost_tier = "free"

# =============================================================================
# Auto-Compaction
# =============================================================================

# Enable automatic context compaction (default: false)
auto_compact = false

# Threshold percentage to trigger auto-compaction (default: 80.0)
auto_compact_threshold = 80.0

# Target percentage of messages to archive during rolling compaction (default: 50.0)
rolling_compact_drop_percentage = 50.0

# =============================================================================
# Reflection (Persistent Memory)
# =============================================================================

# Enable the reflection feature (default: true)
reflection_enabled = true

# Maximum characters for reflection content (default: 10000)
reflection_character_limit = 10000

# =============================================================================
# Fuel Budget (Agentic Loop Limits)
# =============================================================================

# Total fuel budget for autonomous tool loops (default: 30)
# Each tool-call round and agent continuation costs 1 fuel. First turn is free.
# Set to 0 to disable fuel tracking entirely (unlimited mode — no budget enforced,
# no fuel info injected into prompts or hook payloads).
fuel = 30

# Fuel cost of an empty LLM response (default: 15)
# When the LLM returns an empty response (no text, no tool calls), this much
# fuel is consumed. High cost prevents infinite empty-response loops.
# Ignored when fuel = 0 (unlimited mode).
fuel_empty_response_cost = 15

# Context lock heartbeat interval in seconds (default: 30)
lock_heartbeat_seconds = 30

# =============================================================================
# Tool Output Caching
# =============================================================================

# Character threshold above which tool outputs are cached (default: 4000)
# When exceeded, output is cached to disk and a truncated preview is sent to the LLM
tool_output_cache_threshold = 4000

# Maximum age for cached outputs before automatic cleanup (default: 7)
# Note: Value is offset by 1 day, so:
#   0 = delete after 1 day (24 hours)
#   1 = delete after 2 days (48 hours)
#   7 = delete after 8 days (default)
tool_cache_max_age_days = 7

# Automatically cleanup old cache entries on exit (default: true)
auto_cleanup_cache = true

# Number of preview characters to show in truncated message (default: 500)
tool_cache_preview_chars = 500

# =============================================================================
# Built-in file operations
# =============================================================================

# Paths allowed for read-only file tools (default: empty = VFS only)
# When empty, file tools only work with vfs:/// URIs. Add paths to allow OS file access.
# file_tools_allowed_paths = ["~", "/tmp"]

# =============================================================================
# Tool Filtering (global baseline, per-context local.toml merges on top)
# =============================================================================

# [tools]
# include = ["update_todos", "shell_exec"]  # allowlist (local overrides entirely)
# exclude = ["file_grep"]                    # blocklist (local appends)
# exclude_categories = ["agent"]             # category blocklist (local appends)

# =============================================================================
# API Parameters
# =============================================================================

[api]
# Temperature for sampling (0.0 to 2.0)
# temperature = 0.7

# Maximum tokens to generate
# max_tokens = 4096

# Nucleus sampling parameter (0.0 to 1.0)
# top_p = 0.9

# Stop sequences (array of strings)
# stop = ["\n\n", "END"]

# Frequency penalty (-2.0 to 2.0)
# frequency_penalty = 0.0

# Presence penalty (-2.0 to 2.0)
# presence_penalty = 0.0

# Random seed for deterministic output
# seed = 12345

# Enable parallel tool calls (default: true)
# parallel_tool_calls = true

# Tool choice: "auto", "none", "required"
# tool_choice = "auto"

# Enable prompt caching (default: true, mainly benefits Anthropic models)
# prompt_caching = true

# Response format: "text" or "json_object"
# [api.response_format]
# type = "json_object"

# -----------------------------------------------------------------------------
# Reasoning Configuration (for models with extended thinking)
# -----------------------------------------------------------------------------
# Use EITHER effort OR max_tokens, not both.

[api.reasoning]
# Effort level: "xhigh", "high", "medium", "low", "minimal", "none"
# Supported by: OpenAI o1/o3/GPT-5 series, Grok models
effort = "medium"

# OR use token budget instead of effort level:
# Supported by: Anthropic Claude, Gemini thinking models, some Qwen models
# max_tokens = 16000

# Exclude reasoning from response (model still reasons internally)
# exclude = false

# Explicitly enable/disable reasoning
# enabled = true
```

## Model Metadata (models.toml)

Per-model API parameter overrides in `~/.chibi/models.toml`. Model capabilities (context window, tool call support) come from ratatoskr's registry automatically — no need to configure them here.

Use `chibi -M` to see what parameters a model supports.

```toml
# Each key should match the model name used in config.toml or local.toml

# Claude with extended thinking (token-based reasoning)
[models."anthropic/claude-sonnet-4".api.reasoning]
max_tokens = 32000

# OpenAI reasoning models (effort-based reasoning)
[models."openai/o3".api]
max_tokens = 100000

[models."openai/o3".api.reasoning]
effort = "high"

# Gemini thinking model (token-based reasoning)
[models."google/gemini-2.0-flash-thinking-exp:free".api.reasoning]
max_tokens = 16000
```

When you use a model, chibi checks for a matching entry and applies:
- `api.*` - Model-specific API parameters (merged with global settings)

## Per-Context Configuration (local.toml)

Each context can override settings in `~/.chibi/contexts/<name>/local.toml`:

```toml
# Override model for this context
model = "openai/o3"

# Override API key (useful for different billing accounts)
api_key = "sk-different-key"

# Override username
username = "alice"

# Override context window
context_window_limit = 128000

# Override warning threshold
warn_threshold_percent = 90.0

# Override tool omission
no_tool_calls = false

# Override auto-compact behavior
auto_compact = true
auto_compact_threshold = 85.0

# Override fuel budget
fuel = 25

# Override empty response fuel cost
fuel_empty_response_cost = 20

# Override reflection
reflection_enabled = false

# Override tool caching
tool_output_cache_threshold = 8000
tool_cache_max_age_days = 14
auto_cleanup_cache = false
tool_cache_preview_chars = 1000
file_tools_allowed_paths = ["~/projects"]

# Context-specific API parameters
[api]
temperature = 0.3
max_tokens = 8000

[api.reasoning]
effort = "high"

# Tool filtering (merges with global [tools] config)
[tools]
# Allowlist mode - only these tools are available (overrides global include)
# include = ["update_todos", "update_goals", "send_message"]

# Blocklist mode - these tools are excluded (appends to global exclude)
exclude = ["file_grep"]

# Exclude entire categories (appends to global exclude_categories)
# exclude_categories = ["coding"]
```

Set username via CLI (automatically saves to local.toml):

```bash
chibi -u alice "Hello"  # Persists to local.toml
chibi -U bob "Hello"    # Ephemeral, doesn't persist
```

Model can be set similarly:

```bash
chibi -m anthropic/claude-sonnet-4   # Persists to local.toml (validated live)
chibi -s model=anthropic/claude-sonnet-4 "Hello"  # Ephemeral, doesn't persist
```

## CLI Configuration (cli.toml)

CLI-specific presentation settings live in `~/.chibi/cli.toml`. These control how output is rendered in the terminal and are separate from core configuration to support future frontends.

```toml
# =============================================================================
# Markdown Rendering
# =============================================================================

# Render LLM output as formatted markdown in the terminal (default: true)
# Set to false for raw output (useful for piping)
render_markdown = true

# =============================================================================
# Diagnostics and Display
# =============================================================================

# Show extra diagnostic info: tools loaded, warnings, fuel, etc. (default: false)
# Equivalent to the -v / --verbose flag
verbose = false

# Hide tool call display (default: false — tool calls shown by default)
# Equivalent to the --hide-tool-calls flag
hide_tool_calls = false

# Show thinking/reasoning content from models that support extended thinking (default: false)
# Equivalent to the --show-thinking flag
show_thinking = false

# =============================================================================
# Image Configuration
# =============================================================================

[image]
# Render images inline in the terminal (default: true)
render_images = true

# Maximum bytes to download for remote images (default: 10 MB)
max_download_bytes = 10485760

# Timeout in seconds for fetching remote images (default: 5)
fetch_timeout_seconds = 5

# Allow fetching images over plain HTTP (default: false, HTTPS only)
allow_http = false

# Maximum image height in terminal lines (default: 25)
max_height_lines = 25

# Percentage of terminal width to use for images (default: 80)
max_width_percent = 80

# Image alignment: "left", "center", "right" (default: "center")
alignment = "center"

# Image rendering mode (default: "auto")
# Options: "auto", "truecolor", "ansi", "ascii", "placeholder"
render_mode = "auto"

# Enable individual rendering modes (default: all true)
enable_truecolor = true
enable_ansi = true
enable_ascii = true

# Image caching for remote images
cache_enabled = true
cache_max_bytes = 104857600  # 100 MB
cache_max_age_days = 30

# =============================================================================
# Markdown Color Scheme
# =============================================================================

[markdown_style]
bright = "#FFFF54"    # emphasis, h2 headers
head = "#54FF54"      # h3 headers
symbol = "#7ABFC7"    # bullets, language labels
grey = "#808080"      # borders, muted text
dark = "#000000"      # code block background
mid = "#3E31A2"       # table headers
light = "#352879"     # alternate backgrounds
```

### Per-Context CLI Overrides

Create `~/.chibi/contexts/<name>/cli.toml` to override CLI settings for specific contexts. Only specify fields you want to change:

```toml
# Disable markdown rendering for this context
render_markdown = false

# Always show verbose output in this context
verbose = true

# Always show thinking in this context
show_thinking = true

[image]
# Taller images in this context
max_height_lines = 50

[markdown_style]
# Different color scheme
bright = "#00FF00"
```

## API Parameters Reference

Chibi delegates LLM communication to the [ratatoskr](https://github.com/emesal/ratatoskr) crate.

### Generation Control

| Parameter | Type | Range | Description |
|-----------|------|-------|-------------|
| `temperature` | float | 0.0-2.0 | Sampling temperature. Higher = more random. |
| `max_tokens` | integer | 1+ | Maximum tokens to generate. |
| `top_p` | float | 0.0-1.0 | Nucleus sampling. Lower = more focused. |
| `stop` | array | - | Sequences that stop generation. |
| `seed` | integer | - | Random seed for reproducibility. |
| `frequency_penalty` | float | - | Penalize frequent tokens. |
| `presence_penalty` | float | - | Penalize tokens that appeared. |

### Behaviour

| Parameter | Type | Description |
|-----------|------|-------------|
| `tool_choice` | string | How the model uses tools (`auto`, `none`, `required`). |
| `parallel_tool_calls` | boolean | Allow multiple tool calls at once (default: true). |
| `response_format` | object | Force JSON output format. |
| `prompt_caching` | boolean | Enable prompt caching (default: true, mainly benefits Anthropic models). |
| `reasoning.*` | various | Extended thinking configuration (see below). |

### Reasoning Configuration

For models that support extended thinking (chain-of-thought reasoning).

| Parameter | Type | Values | Description |
|-----------|------|--------|-------------|
| `reasoning.effort` | string | `xhigh`, `high`, `medium`, `low`, `minimal`, `none` | Reasoning effort level. |
| `reasoning.max_tokens` | integer | 1024-128000 | Token budget for reasoning. |
| `reasoning.exclude` | boolean | - | Hide reasoning from response. |
| `reasoning.enabled` | boolean | - | Explicitly enable/disable. |

**Note:** Use either `effort` OR `max_tokens`, not both. Different models support different options:

- **Effort-based:** OpenAI o1/o3/GPT-5 series, Grok models
- **Token-based:** Anthropic Claude, Gemini thinking models, Qwen models

## Configuration Merge Order

When resolving API parameters, chibi merges in this order:

1. **Defaults** (`prompt_caching=true`, `reasoning.effort="medium"`, `parallel_tool_calls=true`)
2. **Global config** (`config.toml` `[api]` section)
3. **Model metadata** (`models.toml` `[models."name".api]` section)
4. **Context config** (`local.toml` `[api]` section)

Each layer can override specific values while inheriting others.

## Environment Variables

Two config fields can be set via environment variables, useful for CI/CD secret injection, container deployments, and quick model switching:

| Variable | Overrides | Example |
|----------|-----------|---------|
| `CHIBI_API_KEY` | `api_key` in config.toml | `CHIBI_API_KEY=sk-... chibi "hello"` |
| `CHIBI_MODEL` | `model` in config.toml | `CHIBI_MODEL=openai/o3 chibi "solve this"` |

**Priority:** env vars override `config.toml` but are overridden by `local.toml` and CLI flags. See [resolution order](#core-configuration) for the full hierarchy.

Chibi reads these environment variables for feature detection:
- `COLORTERM` - Checked for truecolor support (`truecolor` or `24bit`)
- `TERM` - Checked for color capability level (`truecolor`, `256color`, `color`)

Plugins receive these environment variables:
- `CHIBI_VERBOSE=1` - Set when `-v` flag is used
- `CHIBI_HOOK` - Hook point name (for hook calls)
- `CHIBI_TOOL_NAME` - Name of the tool being called

Plugin input is passed via stdin as JSON (tool arguments for tool calls, hook data for hooks).

## Coding Tools & Permissions

Chibi includes built-in coding tools that work out of the box — no plugins needed. These tools are automatically included in every API request (unless filtered out via [tool filtering](#tool-filtering-configuration) or `--no-tool-calls`).

**Permission-gated tools** prompt for confirmation before executing:

| Tool | Hook | What it does |
|------|------|-------------|
| `shell_exec` | `PreShellExec` | Execute shell commands |
| `file_edit` | `PreFileWrite` | Patch files (search/replace) |
| `write_file` | `PreFileWrite` | Create or overwrite files |
| `fetch_url` | `PreFetchUrl` | Fetch a URL (gated for sensitive addresses) |
| `summarize_content` | `PreFetchUrl` | Read and summarize a URL source (gated when source is a URL) |

The interactive prompt defaults to **allow** (`[Y/n]`) — press Enter to approve, or type `n` to deny. This makes sense because if you gave the LLM tools, you probably want it to use them.

**Read-only tools** execute without prompting: `dir_list`, `glob_files`, `grep_files`, `file_head`, `file_tail`, `file_lines`, `file_grep`, `index_query`, `index_status`, `index_update`.

### Headless / Automation Mode

When no TTY is available (piped input, CI, parent process), the permission handler cannot prompt and **fails safe by denying** all gated operations. Read-only tools still work.

To allow gated tools in headless mode, use trust mode:

```bash
echo '{"command": {"send_prompt": {"prompt": "list files"}}}' | chibi-json
chibi -t "refactor this module"
```

`-t` / `--trust` auto-approves all permission checks. Use with caution — the LLM will be able to execute arbitrary shell commands and write files without confirmation.

### Plugin Permission Policies

Plugins can implement custom permission logic via the `pre_file_write` and `pre_shell_exec` hooks. A plugin that returns `{"denied": true}` overrides all other approvals (deny wins). See [hooks documentation](hooks.md) for details.

### URL Security Policy

By default, `fetch_url` and `summarize_content` (when given a URL source) prompt for permission when fetching sensitive URLs (loopback, private network, link-local, cloud metadata). A URL policy replaces this interactive check with declarative rules — useful for automation and chibi-json.

```toml
[url_policy]
default = "deny"                          # deny all URLs by default
allow = [
    "preset:loopback",                    # allow localhost
    "https://api.example.com/*",          # allow by glob pattern
]
deny_override = ["preset:cloud_metadata"] # always deny, even if allowed above
```

**Evaluation order** (first match wins, highest priority first):

1. `deny_override` — unconditional deny
2. `allow_override` — unconditional allow (except deny_override)
3. `deny` — standard deny
4. `allow` — standard allow
5. `default` — fallback (`allow` if omitted)

**Rule types:**

- `preset:<category>` — matches a built-in category: `loopback`, `private_network`, `link_local`, `cloud_metadata`, `unparseable`
- bare string — glob pattern (`*` any sequence, `?` single char, `\*` literal asterisk)

**Config layers:** `config.toml` (global) → `local.toml` (per-context) → `JsonInput` (per-invocation). Each layer replaces the previous entirely (no merge). When no policy is set, the interactive permission handler applies as before.

## Tool Filtering Configuration

Control which tools are available to the LLM. Tool filtering can be configured globally in `config.toml` and per-context in `local.toml`.

```toml
[tools]
# Allowlist mode - only these tools are available
# When set, only listed tools can be used
include = ["update_todos", "update_goals", "update_reflection"]

# OR blocklist mode - these tools are excluded
# When set, listed tools are removed from available tools
# exclude = ["file_grep", "file_head", "file_tail"]

# Exclude entire tool categories
# exclude_categories = ["coding", "agent"]
```

**Tool Categories:**

| Category | Tools |
|----------|-------|
| `builtin` | update_todos, update_goals, update_reflection, send_message, call_agent, call_user, model_info, read_context |
| `file` | file_head, file_tail, file_lines, file_grep, write_file |
| `agent` | spawn_agent, summarize_content |
| `coding` | shell_exec, dir_list, glob_files, grep_files, file_edit, fetch_url, index_update, index_query, index_status |
| `vfs` | vfs_list, vfs_info, vfs_copy, vfs_move, vfs_mkdir, vfs_delete |
| `mcp` | MCP tools loaded from the bridge (named `<server>_<tool>`) |
| `plugin` | Tools loaded from the plugins directory |

**Global vs. per-context:**

- `[tools]` in `config.toml` sets the global baseline
- `[tools]` in `local.toml` merges on top:
  - `include`: local **overrides** global entirely (if set)
  - `exclude`: local **appends** to global
  - `exclude_categories`: local **appends** to global

**Filter Precedence:**
1. Config `include` (if set, only these tools considered)
2. Config `exclude` (remove from remaining)
3. Config `exclude_categories` (remove matching categories)
4. Hook `include` (intersect with remaining) — via `pre_api_tools` hook
5. Hook `exclude` (remove from remaining) — via `pre_api_tools` hook

For dynamic tool filtering based on context or other conditions, use the `pre_api_tools` hook. See [Hooks documentation](hooks.md).

## Storage Configuration

Configure transcript partitioning in `~/.chibi/config.toml`:

```toml
[storage]
# Rotate partition after N entries (default: 1000)
partition_max_entries = 1000

# Rotate partition after N estimated LLM tokens (default: 100000)
partition_max_tokens = 100000

# Rotate partition after N seconds (default: 2592000 = 30 days)
partition_max_age_seconds = 2592000

# Bytes per token for estimation heuristic (default: 3)
# Lower values = more conservative (higher token estimates)
# 3 handles mixed English/CJK content; use 4 for English-only
bytes_per_token = 3

# Build bloom filter indexes for search optimization (default: true)
enable_bloom_filters = true
```

Per-context overrides in `~/.chibi/contexts/<name>/local.toml`:

```toml
[storage]
partition_max_entries = 500
partition_max_tokens = 50000
bytes_per_token = 4  # Less conservative for this context
```

Partitions rotate when any threshold is reached. This keeps individual partition files manageable while enabling efficient search across conversation history.
