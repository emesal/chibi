# chibi upgrade notes

## 0.4.1 -> 0.4.2 (upcoming)

### Reasoning configuration change

The `api.reasoning_effort` field has been replaced with a more flexible `api.reasoning` section:

**Old format:**
```toml
[api]
reasoning_effort = "high"
```

**New format:**
```toml
[api.reasoning]
effort = "high"
# OR use token budget instead:
# max_tokens = 16000
```

The new format supports:
- `effort` - effort level (xhigh, high, medium, low, minimal, none)
- `max_tokens` - token budget for reasoning (for Anthropic, Gemini, Qwen)
- `exclude` - hide reasoning from response
- `enabled` - explicitly enable/disable reasoning

Use either `effort` OR `max_tokens`, not both. See [configuration.md](configuration.md) for details.

## 0.4.0 -> 0.4.1
- new context state format -> clear (0.3) or archive (0.4) contexts before upgrading to preserve history (see --help)
- human-readable transcripts are now md files. if wanted, old transcripts can be migrated with

```bash
find $HOME/.chibi/contexts -type f -name "transcript.txt" -exec sh -c 'mv -i "$1" "${1%.txt}.md"' _ {} \;
```

## 0.3.0 -> 0.4.0
- CLI parameters changed! Existing scripts need to be updated.
