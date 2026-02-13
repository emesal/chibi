# Configuration Reference

Chibi uses a layered configuration system with separate files for core and CLI settings.

## Core Configuration

Settings are resolved in this order (later overrides earlier):

1. **Defaults** - Built-in default values
2. **Global config** (`~/.chibi/config.toml`) - User's base configuration
3. **Model metadata** (`~/.chibi/models.toml`) - Per-model settings
4. **Context config** (`~/.chibi/contexts/<name>/local.toml`) - Per-context overrides
5. **CLI flags** - Command-line arguments (highest priority)

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

```toml
# =============================================================================
# Required Settings
# =============================================================================

# API key for your LLM provider
# Currently chibi uses OpenRouter (https://openrouter.ai/settings/keys)
api_key = "your-api-key-here"

# Model to use (see https://openrouter.ai/models for available models)
model = "anthropic/claude-sonnet-4"

# Context window limit (tokens) - used for warning calculations
context_window_limit = 200000

# Warning threshold (0-100) - warn when context exceeds this percentage
warn_threshold_percent = 80.0

# =============================================================================
# Optional Settings
# =============================================================================

# Default username shown to the LLM (default: "user")
username = "user"

# Enable verbose output by default, equivalent to -v (default: false)
verbose = false

# Hide tool call display by default (default: false, verbose overrides)
hide_tool_calls = false

# Omit tools from API requests entirely for pure text mode (default: false)
no_tool_calls = false

# Fallback tool when LLM doesn't explicitly call call_agent/call_user
# Options: "call_user" (return to user) or "call_agent" (continue loop)
fallback_tool = "call_user"

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
fuel = 30

# Fuel cost of an empty LLM response (default: 15)
# When the LLM returns an empty response (no text, no tool calls), this much
# fuel is consumed. High cost prevents infinite empty-response loops.
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

# Paths allowed for read-only file tools (default: empty = cache only)
# When empty, file tools only work with cache_id. Add paths to allow file access.
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

Define per-model settings in `~/.chibi/models.toml`:

```toml
# Each key should match the model name used in config.toml or local.toml

[models."anthropic/claude-sonnet-4"]
context_window = 200000

[models."anthropic/claude-3.5-haiku"]
context_window = 200000

[models."openai/gpt-4o"]
context_window = 128000

[models."openai/o3"]
context_window = 200000

[models."openai/o3".api]
max_tokens = 100000

[models."openai/o3".api.reasoning]
effort = "high"

[models."google/gemini-2.0-flash-thinking-exp:free"]
context_window = 1048576

[models."google/gemini-2.0-flash-thinking-exp:free".api.reasoning]
max_tokens = 16000

[models."anthropic/claude-sonnet-4".api.reasoning]
max_tokens = 32000
```

When you use a model, chibi checks for a matching entry and applies:
- `context_window` - Overrides `context_window_limit` from config.toml
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

# Override verbose, tool call display, or tool omission
verbose = true
hide_tool_calls = false
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

[image]
# Taller images in this context
max_height_lines = 50

[markdown_style]
# Different color scheme
bright = "#00FF00"
```

## API Parameters Reference

Chibi delegates LLM communication to the [ratatoskr](https://github.com/emesal/ratatoskr) crate. Currently, only a subset of API parameters are passed through — see [issue #109](https://github.com/emesal/chibi/issues/109) for status.

### Generation Control (supported)

| Parameter | Type | Range | Description |
|-----------|------|-------|-------------|
| `temperature` | float | 0.0-2.0 | Sampling temperature. Higher = more random. |
| `max_tokens` | integer | 1+ | Maximum tokens to generate. |
| `top_p` | float | 0.0-1.0 | Nucleus sampling. Lower = more focused. |
| `stop` | array | - | Sequences that stop generation. |
| `seed` | integer | - | Random seed for reproducibility. |

### Additional Supported Parameters

| Parameter | Type | Description |
|-----------|------|-------------|
| `parallel_tool_calls` | boolean | Allow multiple tool calls at once (default: true). |
| `prompt_caching` | boolean | Enable prompt caching (default: true, mainly benefits Anthropic models). |
| `reasoning.*` | various | Extended thinking configuration (see below). |

### Not Yet Passed Through

The following parameters are parsed from config but not yet sent to the API ([#109](https://github.com/emesal/chibi/issues/109)):

| Parameter | Type | Description |
|-----------|------|-------------|
| `frequency_penalty` | float | Penalize frequent tokens. |
| `presence_penalty` | float | Penalize tokens that appeared. |
| `tool_choice` | string | How the model uses tools (`auto`, `none`, `required`). |
| `response_format` | object | Force JSON output format. |

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

Chibi does not use environment variables for configuration. All settings come from the config files described above.

Chibi reads these environment variables for feature detection:
- `COLORTERM` - Checked for truecolor support (`truecolor` or `24bit`)
- `TERM` - Checked for color capability level (`truecolor`, `256color`, `color`)

Plugins receive these environment variables:
- `CHIBI_VERBOSE=1` - Set when `-v` flag is used
- `CHIBI_HOOK` - Hook point name (for hook calls)
- `CHIBI_TOOL_NAME` - Name of the tool being called

Plugin input is passed via stdin as JSON (tool arguments for tool calls, hook data for hooks).

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
| `builtin` | update_todos, update_goals, update_reflection, send_message |
| `file` | file_head, file_tail, file_lines, file_grep, cache_list |
| `agent` | spawn_agent, retrieve_content |
| `coding` | shell_exec, dir_list, glob_files, grep_files, file_edit, index_update, index_query, index_status |
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
