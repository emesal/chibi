# Example: Using the MCP Bridge with Serena

[Serena](https://github.com/oraios/serena) is a semantic code intelligence MCP server. It understands your codebase at the symbol level — find definitions, trace references, search for patterns, navigate directory trees — and exposes all of this as MCP tools that chibi can call.

This document walks through a complete Serena setup as a concrete example of the MCP bridge in action. For the general MCP bridge reference, see [mcp.md](mcp.md).

---

## 1. Install Serena

You need [`uv`](https://docs.astral.sh/uv/getting-started/installation/) (Python toolchain) and Python ≥ 3.11.

```bash
git clone https://github.com/oraios/serena
cd serena
uv sync
```

Verify it works:

```bash
uv run --directory /path/to/serena serena-mcp-server --help
```

---

## 2. Configure the bridge

Add a `serena` entry to `~/.chibi/mcp-bridge.toml`:

```toml
[servers.serena]
command = "uv"
args = [
    "run",
    "--directory",
    "/path/to/serena",      # ← your clone path
    "serena-mcp-server"
]
```

On the next chibi invocation the bridge starts automatically and serena's tools are loaded (prefixed `serena_`).

---

## 3. Try it out

Use `-P` to call a tool directly, bypassing the LLM:

```bash
# List the symbols in a file
chibi -P serena_get_symbols_overview '{"relative_path": "src/main.rs"}'

# Search for a pattern across the codebase
chibi -P serena_search_for_pattern '{"substring_pattern": "fn execute"}'

# Find all references to a symbol
chibi -P serena_find_referencing_symbols '{"name_path": "Config", "relative_path": "src/config.rs"}'
```

Use `-v` to see which tools the LLM is calling in a normal conversation:

```bash
chibi -v "Where is the Config struct defined and what are its fields?"
# stderr: [Tool: serena_find_symbol(...)]
# stderr: [Tool: serena_get_symbols_overview(...)]
```

---

## 4. Troubleshooting

| Problem | Fix |
|---------|-----|
| `uv: command not found` | Install [uv](https://docs.astral.sh/uv/getting-started/installation/). |
| Tools don't appear | Run `chibi -v` — check for bridge errors on stderr. Ensure the path in `mcp-bridge.toml` is correct and `uv sync` has been run. |
| Stale bridge state | Delete `~/.chibi/mcp-bridge.lock` and retry. |
