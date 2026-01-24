# Configuration Reference

Chibi uses a layered configuration system. Settings are resolved in this order (later overrides earlier):

1. **Defaults** - Built-in default values
2. **Global config** (`~/.chibi/config.toml`) - User's base configuration
3. **Model metadata** (`~/.chibi/models.toml`) - Per-model settings
4. **Context config** (`~/.chibi/contexts/<name>/local.toml`) - Per-context overrides
5. **CLI flags** - Command-line arguments (highest priority)

## Global Configuration (config.toml)

Create `~/.chibi/config.toml`:

```toml
# =============================================================================
# Required Settings
# =============================================================================

# OpenRouter API key (get one at https://openrouter.ai/settings/keys)
api_key = "your-openrouter-api-key-here"

# Model to use (see https://openrouter.ai/models)
model = "anthropic/claude-sonnet-4"

# Context window limit (tokens) - used for warning calculations
context_window_limit = 200000

# Warning threshold (0-100) - warn when context exceeds this percentage
warn_threshold_percent = 80.0

# =============================================================================
# Optional Settings
# =============================================================================

# Custom API endpoint (default: OpenRouter)
# base_url = "https://openrouter.ai/api/v1/chat/completions"

# Default username shown to the LLM (default: "user")
username = "user"

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
# Safety Limits
# =============================================================================

# Maximum recursion depth for autonomous tool loops (default: 30)
max_recursion_depth = 30

# Context lock heartbeat interval in seconds (default: 30)
lock_heartbeat_seconds = 30

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

# Override API key (useful for different providers)
api_key = "sk-different-key"

# Override base URL
base_url = "https://api.openai.com/v1/chat/completions"

# Override username
username = "alice"

# Override context window
context_window_limit = 128000

# Override warning threshold
warn_threshold_percent = 90.0

# Override auto-compact behavior
auto_compact = true
auto_compact_threshold = 85.0

# Override recursion depth
max_recursion_depth = 25

# Override reflection
reflection_enabled = false

# Context-specific API parameters
[api]
temperature = 0.3
max_tokens = 8000

[api.reasoning]
effort = "high"
```

Set username via CLI (automatically saves to local.toml):

```bash
chibi -u alice "Hello"  # Persists to local.toml
chibi -U bob "Hello"    # Transient, doesn't persist
```

## API Parameters Reference

### Generation Control

| Parameter | Type | Range | Description |
|-----------|------|-------|-------------|
| `temperature` | float | 0.0-2.0 | Sampling temperature. Higher = more random. |
| `max_tokens` | integer | 1+ | Maximum tokens to generate. |
| `top_p` | float | 0.0-1.0 | Nucleus sampling. Lower = more focused. |
| `stop` | array | - | Sequences that stop generation. |
| `seed` | integer | - | Random seed for reproducibility. |

### Sampling Penalties

| Parameter | Type | Range | Description |
|-----------|------|-------|-------------|
| `frequency_penalty` | float | -2.0 to 2.0 | Penalize frequent tokens. |
| `presence_penalty` | float | -2.0 to 2.0 | Penalize tokens that appeared. |

### Tool Control

| Parameter | Type | Values | Description |
|-----------|------|--------|-------------|
| `tool_choice` | string | `auto`, `none`, `required` | How the model uses tools. |
| `parallel_tool_calls` | boolean | - | Allow multiple tool calls at once. |

### OpenRouter-Specific

| Parameter | Type | Description |
|-----------|------|-------------|
| `prompt_caching` | boolean | Enable prompt caching (default: true). |

### Reasoning Configuration

For models that support extended thinking (chain-of-thought reasoning):

| Parameter | Type | Values | Description |
|-----------|------|--------|-------------|
| `reasoning.effort` | string | `xhigh`, `high`, `medium`, `low`, `minimal`, `none` | Reasoning effort level. |
| `reasoning.max_tokens` | integer | 1024-128000 | Token budget for reasoning. |
| `reasoning.exclude` | boolean | - | Hide reasoning from response. |
| `reasoning.enabled` | boolean | - | Explicitly enable/disable. |

**Note:** Use either `effort` OR `max_tokens`, not both. Different models support different options:

- **Effort-based:** OpenAI o1/o3/GPT-5 series, Grok models
- **Token-based:** Anthropic Claude, Gemini thinking models, Qwen models

### Response Format

```toml
[api.response_format]
type = "text"  # or "json_object" or "json_schema"

# For json_schema, also provide:
# json_schema = { ... }
```

## Configuration Merge Order

When resolving API parameters, chibi merges in this order:

1. **Defaults** (`prompt_caching=true`, `reasoning.effort="medium"`, `parallel_tool_calls=true`)
2. **Global config** (`config.toml` `[api]` section)
3. **Model metadata** (`models.toml` `[models."name".api]` section)
4. **Context config** (`local.toml` `[api]` section)

Each layer can override specific values while inheriting others.

## Environment Variables

Chibi does not use environment variables for configuration. All settings come from the config files described above.

Plugins receive these environment variables:
- `CHIBI_TOOL_ARGS` - JSON arguments for tool calls
- `CHIBI_VERBOSE=1` - Set when `-v` flag is used
- `CHIBI_HOOK` - Hook point name (for hook calls)
- `CHIBI_HOOK_DATA` - JSON data for hook calls
