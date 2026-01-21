# Agent Skills Implementation Specification

This document specifies the implementation of [Agent Skills](https://agentskills.io) support for chibi, consisting of:

1. **Chibi core changes** - Plugin directory support, multi-tool schemas, tool blocking
2. **`agent-skills` plugin** - Compatibility layer + marketplace integration

## Overview

The Agent Skills standard defines a portable format for giving AI agents domain-specific expertise. Skills are directories containing a `SKILL.md` file with YAML frontmatter (metadata) and markdown body (instructions).

Chibi's plugin system is architecturally different (executable-based, JSON schemas), so we implement Agent Skills as a **compatibility layer plugin** that:
- Parses SKILL.md files and exposes them as chibi tools
- Handles skill invocation and progressive disclosure
- Provides marketplace functionality for installing skills
- Enforces `allowed-tools` restrictions

---

## Part 1: Chibi Core Changes

### 1.1 Plugin Directory Support

**Current behavior:** `load_tools` only loads executable files directly in `~/.chibi/plugins/`.

**New behavior:** Support two plugin structures:
- `plugins/[name]` - Single executable file (existing)
- `plugins/[name]/[name]` - Directory with same-named executable inside

**`.disabled` convention:**
- Skip any entry (file or directory) ending in `.disabled`
- `~/.chibi/plugins.disabled/` is reserved for disabled plugins (never scanned)

**File:** `src/tools.rs` - `load_tools` function

```rust
pub fn load_tools(plugins_dir: &PathBuf, verbose: bool) -> io::Result<Vec<Tool>> {
    let mut tools = Vec::new();

    if !plugins_dir.exists() {
        return Ok(tools);
    }

    let entries = fs::read_dir(plugins_dir)?;

    for entry in entries.flatten() {
        let path = entry.path();
        let file_name = path.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("");

        // Skip .disabled entries
        if file_name.ends_with(".disabled") {
            continue;
        }

        // Determine the executable path
        let exec_path = if path.is_dir() {
            // Directory plugin: look for plugins/[name]/[name]
            let inner = path.join(file_name);
            if !inner.exists() || inner.is_dir() {
                if verbose {
                    eprintln!("[WARN] Plugin directory {:?} missing executable", file_name);
                }
                continue;
            }
            inner
        } else {
            path.clone()
        };

        // Check if executable (on Unix)
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if let Ok(metadata) = exec_path.metadata()
                && metadata.permissions().mode() & 0o111 == 0
            {
                continue; // Not executable
            }
        }

        // Try to get schema(s) from the tool
        match get_tool_schemas(&exec_path, verbose) {
            Ok(new_tools) => tools.extend(new_tools),
            Err(e) => {
                if verbose {
                    eprintln!("[WARN] Failed to load tool {:?}: {}", exec_path.file_name(), e);
                }
            }
        }
    }

    Ok(tools)
}
```

### 1.2 Multi-Tool Schema Support

**Current behavior:** `get_tool_schema` expects a single JSON object with `name`, `description`, `parameters`.

**New behavior:** Support both single tool and array of tools:
- Single object: `{"name": "...", "description": "...", ...}` → one tool
- Array: `[{"name": "...", ...}, {"name": "...", ...}]` → multiple tools

**File:** `src/tools.rs` - rename `get_tool_schema` to `get_tool_schemas`

```rust
/// Get tool schema(s) by calling plugin with --schema
/// Returns Vec<Tool> to support plugins that provide multiple tools
fn get_tool_schemas(path: &PathBuf, verbose: bool) -> io::Result<Vec<Tool>> {
    let output = Command::new(path)
        .arg("--schema")
        .output()
        .map_err(|e| io::Error::other(format!("Failed to execute tool: {}", e)))?;

    if !output.status.success() {
        return Err(io::Error::other(format!(
            "Tool returned error: {}",
            String::from_utf8_lossy(&output.stderr)
        )));
    }

    let schema_str = String::from_utf8(output.stdout).map_err(|e| {
        io::Error::new(
            ErrorKind::InvalidData,
            format!("Invalid UTF-8 in schema: {}", e),
        )
    })?;

    let schema: serde_json::Value = serde_json::from_str(&schema_str).map_err(|e| {
        io::Error::new(
            ErrorKind::InvalidData,
            format!("Invalid JSON schema: {}", e),
        )
    })?;

    // Handle array of tools or single tool
    let schemas: Vec<&serde_json::Value> = if let Some(arr) = schema.as_array() {
        arr.iter().collect()
    } else {
        vec![&schema]
    };

    let mut tools = Vec::new();
    for s in schemas {
        match parse_single_tool_schema(s, path) {
            Ok(tool) => tools.push(tool),
            Err(e) => {
                if verbose {
                    eprintln!("[WARN] Failed to parse tool in {:?}: {}", path.file_name(), e);
                }
            }
        }
    }

    if tools.is_empty() {
        return Err(io::Error::new(
            ErrorKind::InvalidData,
            "No valid tools found in schema",
        ));
    }

    Ok(tools)
}

fn parse_single_tool_schema(schema: &serde_json::Value, path: &PathBuf) -> io::Result<Tool> {
    let name = schema["name"]
        .as_str()
        .ok_or_else(|| io::Error::new(ErrorKind::InvalidData, "Schema missing 'name' field"))?
        .to_string();

    let description = schema["description"]
        .as_str()
        .ok_or_else(|| {
            io::Error::new(ErrorKind::InvalidData, "Schema missing 'description' field")
        })?
        .to_string();

    let parameters = schema["parameters"].clone();

    // Parse hooks array (optional)
    let hooks = if let Some(hooks_array) = schema["hooks"].as_array() {
        hooks_array
            .iter()
            .filter_map(|v| v.as_str().and_then(HookPoint::from_str))
            .collect()
    } else {
        Vec::new()
    };

    Ok(Tool {
        name,
        description,
        parameters,
        path: path.clone(),
        hooks,
    })
}
```

### 1.3 Tool Blocking via `pre_tool` Hook

**Current behavior:** `pre_tool` hook can modify arguments but cannot prevent tool execution.

**New behavior:** If a `pre_tool` hook returns `{"block": true, "message": "..."}`, the tool call is blocked and the message is returned as the tool result.

**File:** `src/api.rs` - in the tool execution loop

```rust
// Execute pre_tool hooks (can modify arguments OR block execution)
let pre_hook_data = serde_json::json!({
    "tool_name": tc.name,
    "arguments": args,
});
let pre_hook_results =
    tools::execute_hook(tools, tools::HookPoint::PreTool, &pre_hook_data, verbose)?;

let mut blocked = false;
let mut block_message = String::new();

for (hook_tool_name, result) in pre_hook_results {
    // Check for block signal
    if result.get("block").and_then(|v| v.as_bool()).unwrap_or(false) {
        blocked = true;
        block_message = result
            .get("message")
            .and_then(|v| v.as_str())
            .unwrap_or("Tool call blocked by hook")
            .to_string();
        if verbose {
            eprintln!(
                "[Hook pre_tool: {} blocked {} - {}]",
                hook_tool_name, tc.name, block_message
            );
        }
        break;
    }

    // Check for argument modification (existing behavior)
    if let Some(modified_args) = result.get("arguments") {
        if verbose {
            eprintln!(
                "[Hook pre_tool: {} modified arguments for {}]",
                hook_tool_name, tc.name
            );
        }
        args = modified_args.clone();
    }
}

// If blocked, skip execution and use block message as result
let result = if blocked {
    block_message
} else if tc.name == tools::REFLECTION_TOOL_NAME && use_reflection {
    // ... existing tool execution logic
```

---

## Part 2: `agent-skills` Plugin Specification

### 2.1 Directory Structure

```
~/.chibi/plugins/agent-skills/
├── agent-skills              # Main executable (Python with uv)
├── skills/                   # Installed skills directory
│   ├── pdf-processing/
│   │   ├── SKILL.md
│   │   └── scripts/
│   │       └── extract.py
│   └── code-review/
│       └── SKILL.md
└── lib/                      # Internal modules
    ├── __init__.py
    ├── parser.py             # SKILL.md parsing (YAML frontmatter)
    ├── marketplace.py        # GitHub marketplace operations
    └── state.py              # Active skill state management
```

### 2.2 Plugin Executable Interface

The `agent-skills` executable handles:
- `--schema` → Returns array of tool schemas
- Tool invocations → Routes to appropriate handler
- Hook invocations → Handles lifecycle events

```python
#!/usr/bin/env -S uv run --quiet --script
# /// script
# requires-python = ">=3.10"
# dependencies = ["pyyaml"]
# ///

import json
import os
import subprocess
import sys
from pathlib import Path

# Add lib to path
PLUGIN_DIR = Path(__file__).parent
sys.path.insert(0, str(PLUGIN_DIR / "lib"))

from parser import parse_skill, list_skills
from marketplace import install_skill, remove_skill, search_skills, list_available
from state import get_active_skill, set_active_skill, clear_active_skill

SKILLS_DIR = PLUGIN_DIR / "skills"

def get_schema():
    """Generate schema for all tools provided by this plugin."""
    tools = []

    # Core management tools
    tools.append({
        "name": "skill_marketplace",
        "description": "Install, remove, search, or list Agent Skills from the marketplace",
        "parameters": {
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["install", "remove", "search", "list", "list_installed"],
                    "description": "Action to perform"
                },
                "skill_ref": {
                    "type": "string",
                    "description": "Skill reference (owner/name) for install/remove"
                },
                "query": {
                    "type": "string",
                    "description": "Search query for search action"
                }
            },
            "required": ["action"]
        }
    })

    tools.append({
        "name": "read_skill_file",
        "description": "Read a file from an installed skill's directory (scripts, references, etc.)",
        "parameters": {
            "type": "object",
            "properties": {
                "skill": {
                    "type": "string",
                    "description": "Name of the installed skill"
                },
                "path": {
                    "type": "string",
                    "description": "Relative path to the file within the skill directory"
                }
            },
            "required": ["skill", "path"]
        }
    })

    tools.append({
        "name": "run_skill_script",
        "description": "Execute a script from an installed skill's directory (e.g., scripts/extract.py)",
        "parameters": {
            "type": "object",
            "properties": {
                "skill": {
                    "type": "string",
                    "description": "Name of the installed skill"
                },
                "script": {
                    "type": "string",
                    "description": "Relative path to the script within the skill directory"
                },
                "args": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "Arguments to pass to the script (optional)"
                },
                "stdin": {
                    "type": "string",
                    "description": "Input to pass to the script via stdin (optional)"
                }
            },
            "required": ["skill", "script"]
        }
    })

    # One tool per installed skill
    for skill in list_skills(SKILLS_DIR):
        tools.append({
            "name": f"skill_{skill.name}",
            "description": skill.description,
            "parameters": {
                "type": "object",
                "properties": {
                    "arguments": {
                        "type": "string",
                        "description": "Arguments to pass to the skill (optional)"
                    }
                }
            }
        })

    # Register for hooks
    # Note: hooks are registered on the first tool only (chibi applies to all from same plugin)
    if tools:
        tools[0]["hooks"] = ["post_system_prompt", "pre_tool", "on_start"]

    return tools

def handle_hook():
    """Handle hook invocations."""
    hook = os.environ.get("CHIBI_HOOK", "")
    hook_data = json.loads(os.environ.get("CHIBI_HOOK_DATA", "{}"))

    if hook == "on_start":
        # Clear any stale active skill state
        clear_active_skill()
        print("{}")

    elif hook == "post_system_prompt":
        # Inject skill index into system prompt
        skills = list_skills(SKILLS_DIR)
        if not skills:
            print("{}")
            return

        index_lines = ["## Available Agent Skills", ""]
        for skill in skills:
            index_lines.append(f"- **{skill.name}**: {skill.description}")
        index_lines.append("")
        index_lines.append("Use skill_[name] tools to invoke a skill and receive detailed instructions.")

        print(json.dumps({"inject": "\n".join(index_lines)}))

    elif hook == "pre_tool":
        tool_name = hook_data.get("tool_name", "")

        # Track skill activation
        if tool_name.startswith("skill_") and tool_name != "skill_marketplace":
            skill_name = tool_name[6:]  # Remove "skill_" prefix
            skill = parse_skill(SKILLS_DIR / skill_name / "SKILL.md")
            if skill:
                set_active_skill(skill_name, skill.allowed_tools)
                print("{}")
                return

        # Enforce allowed-tools for active skill
        active = get_active_skill()
        if active and active["allowed_tools"]:
            allowed = active["allowed_tools"]
            # Check if tool is allowed
            # allowed-tools format: "Read, Grep, Bash(git:*)"
            if not is_tool_allowed(tool_name, allowed):
                print(json.dumps({
                    "block": True,
                    "message": f"Tool '{tool_name}' is not allowed while skill '{active['name']}' is active. Allowed tools: {allowed}"
                }))
                return

        print("{}")

    else:
        print("{}")

def is_tool_allowed(tool_name: str, allowed_tools: str) -> bool:
    """Check if a tool is in the allowed-tools list."""
    # Parse allowed-tools string
    # Format: "Read, Grep, Bash(git:*)"
    allowed_list = [t.strip() for t in allowed_tools.split(",")]

    for allowed in allowed_list:
        if "(" in allowed:
            # Pattern match: Bash(git:*) matches Bash with git commands
            base, pattern = allowed.split("(", 1)
            pattern = pattern.rstrip(")")
            if tool_name == base:
                # For now, allow if base matches (full pattern matching is complex)
                return True
        else:
            if tool_name == allowed:
                return True

    return False

def handle_tool_call():
    """Handle tool invocations."""
    args = json.loads(os.environ.get("CHIBI_TOOL_ARGS", "{}"))
    tool_name = os.environ.get("CHIBI_TOOL_NAME", "")  # If chibi provides this

    # Determine which tool was called based on args structure
    # This is a simplification - in practice, chibi should pass the tool name

    if "action" in args:
        # skill_marketplace
        handle_marketplace(args)
    elif "script" in args and "skill" in args:
        # run_skill_script
        handle_run_skill_script(args)
    elif "skill" in args and "path" in args:
        # read_skill_file
        handle_read_skill_file(args)
    elif "arguments" in args or not args:
        # Skill invocation - need tool name from environment
        # Fallback: parse from somewhere or require tool_name in args
        handle_skill_invocation(tool_name, args)

def handle_marketplace(args):
    """Handle marketplace operations."""
    action = args["action"]

    if action == "install":
        skill_ref = args.get("skill_ref", "")
        if not skill_ref:
            print("Error: skill_ref required for install")
            return
        result = install_skill(skill_ref, SKILLS_DIR)
        print(result)

    elif action == "remove":
        skill_ref = args.get("skill_ref", "")
        if not skill_ref:
            print("Error: skill_ref required for remove")
            return
        result = remove_skill(skill_ref, SKILLS_DIR)
        print(result)

    elif action == "search":
        query = args.get("query", "")
        results = search_skills(query)
        print(json.dumps(results, indent=2))

    elif action == "list":
        results = list_available()
        print(json.dumps(results, indent=2))

    elif action == "list_installed":
        skills = list_skills(SKILLS_DIR)
        installed = [{"name": s.name, "description": s.description} for s in skills]
        print(json.dumps(installed, indent=2))

def handle_read_skill_file(args):
    """Read a file from a skill's directory."""
    skill_name = args["skill"]
    rel_path = args["path"]

    # Security: prevent path traversal
    skill_dir = SKILLS_DIR / skill_name
    if not skill_dir.exists():
        print(f"Error: Skill '{skill_name}' not found")
        return

    file_path = (skill_dir / rel_path).resolve()
    if not str(file_path).startswith(str(skill_dir.resolve())):
        print("Error: Path traversal not allowed")
        return

    if not file_path.exists():
        print(f"Error: File not found: {rel_path}")
        return

    try:
        content = file_path.read_text()
        print(content)
    except Exception as e:
        print(f"Error reading file: {e}")

def handle_run_skill_script(args):
    """Execute a script from a skill's directory."""
    skill_name = args["skill"]
    script_path = args["script"]
    script_args = args.get("args", [])
    stdin_input = args.get("stdin")

    # Security: validate and resolve path
    skill_dir = SKILLS_DIR / skill_name
    if not skill_dir.exists():
        print(f"Error: Skill '{skill_name}' not found")
        return

    full_path = (skill_dir / script_path).resolve()
    if not str(full_path).startswith(str(skill_dir.resolve())):
        print("Error: Path traversal not allowed")
        return

    if not full_path.exists():
        print(f"Error: Script not found: {script_path}")
        return

    # Check if executable (warn but still try to run via interpreter)
    is_executable = os.access(full_path, os.X_OK)

    # Execute the script
    try:
        # Determine how to run the script
        if is_executable:
            cmd = [str(full_path)] + script_args
        else:
            # Try to detect interpreter from shebang or extension
            ext = full_path.suffix.lower()
            if ext == ".py":
                cmd = ["python3", str(full_path)] + script_args
            elif ext == ".sh":
                cmd = ["bash", str(full_path)] + script_args
            elif ext == ".js":
                cmd = ["node", str(full_path)] + script_args
            else:
                # Fall back to trying to execute directly
                cmd = [str(full_path)] + script_args

        result = subprocess.run(
            cmd,
            capture_output=True,
            text=True,
            cwd=str(skill_dir),  # Run from skill directory
            input=stdin_input,
            timeout=120  # 2 minute timeout
        )

        output_parts = []
        if result.stdout:
            output_parts.append(result.stdout)
        if result.stderr:
            output_parts.append(f"[stderr]\n{result.stderr}")
        if result.returncode != 0:
            output_parts.append(f"[exit code: {result.returncode}]")

        print("\n".join(output_parts) if output_parts else "(no output)")

    except subprocess.TimeoutExpired:
        print("Error: Script execution timed out (120s limit)")
    except PermissionError:
        print(f"Error: Permission denied executing script. Make sure it's executable: chmod +x {script_path}")
    except Exception as e:
        print(f"Error executing script: {e}")

def handle_skill_invocation(tool_name: str, args: dict):
    """Invoke a skill and return its instructions."""
    # Extract skill name from tool_name (skill_pdf-processing -> pdf-processing)
    if not tool_name.startswith("skill_"):
        print("Error: Invalid skill tool name")
        return

    skill_name = tool_name[6:]
    skill_path = SKILLS_DIR / skill_name / "SKILL.md"

    if not skill_path.exists():
        print(f"Error: Skill '{skill_name}' not found")
        return

    skill = parse_skill(skill_path)
    if not skill:
        print(f"Error: Failed to parse skill '{skill_name}'")
        return

    # Return the skill body (instructions) as the tool result
    # Include any arguments passed
    arguments = args.get("arguments", "")

    response_parts = [
        f"# Skill: {skill.name}",
        "",
        skill.body,
    ]

    if arguments:
        response_parts.extend([
            "",
            "## Arguments",
            arguments
        ])

    # Include info about supporting files if they exist
    skill_dir = SKILLS_DIR / skill_name
    supporting_dirs = ["scripts", "references", "assets"]
    existing_dirs = [d for d in supporting_dirs if (skill_dir / d).exists()]

    if existing_dirs:
        response_parts.extend([
            "",
            "## Supporting Files",
            f"This skill has supporting files in: {', '.join(existing_dirs)}",
            "- Use `read_skill_file` to read file contents",
            "- Use `run_skill_script` to execute scripts"
        ])

    print("\n".join(response_parts))

if __name__ == "__main__":
    if len(sys.argv) > 1 and sys.argv[1] == "--schema":
        print(json.dumps(get_schema()))
    elif os.environ.get("CHIBI_HOOK"):
        handle_hook()
    else:
        handle_tool_call()
```

### 2.3 Supporting Modules

#### `lib/parser.py` - SKILL.md Parsing

```python
"""Parse SKILL.md files according to Agent Skills specification."""
import re
from dataclasses import dataclass
from pathlib import Path
from typing import Optional, List
import yaml

@dataclass
class Skill:
    name: str
    description: str
    body: str
    allowed_tools: Optional[str] = None
    license: Optional[str] = None
    compatibility: Optional[str] = None
    metadata: Optional[dict] = None

def parse_skill(skill_path: Path) -> Optional[Skill]:
    """Parse a SKILL.md file and return a Skill object."""
    if not skill_path.exists():
        return None

    content = skill_path.read_text()

    # Extract YAML frontmatter
    frontmatter_match = re.match(r'^---\s*\n(.*?)\n---\s*\n', content, re.DOTALL)
    if not frontmatter_match:
        return None

    try:
        frontmatter = yaml.safe_load(frontmatter_match.group(1))
    except yaml.YAMLError:
        return None

    # Required fields
    name = frontmatter.get("name")
    description = frontmatter.get("description")

    if not name or not description:
        return None

    # Validate name format (per spec)
    if not re.match(r'^[a-z0-9]+(-[a-z0-9]+)*$', name):
        return None

    if len(name) > 64 or len(description) > 1024:
        return None

    # Body is everything after frontmatter
    body = content[frontmatter_match.end():]

    return Skill(
        name=name,
        description=description,
        body=body.strip(),
        allowed_tools=frontmatter.get("allowed-tools"),
        license=frontmatter.get("license"),
        compatibility=frontmatter.get("compatibility"),
        metadata=frontmatter.get("metadata"),
    )

def list_skills(skills_dir: Path) -> List[Skill]:
    """List all valid skills in the skills directory."""
    skills = []

    if not skills_dir.exists():
        return skills

    for entry in skills_dir.iterdir():
        if entry.is_dir() and not entry.name.startswith("."):
            skill_path = entry / "SKILL.md"
            skill = parse_skill(skill_path)
            if skill:
                skills.append(skill)

    return sorted(skills, key=lambda s: s.name)
```

#### `lib/marketplace.py` - Marketplace Operations

```python
"""Marketplace operations for installing/managing skills."""
import json
import subprocess
import shutil
from pathlib import Path
from typing import List, Dict, Any

# Default marketplace sources
MARKETPLACE_SOURCES = [
    "https://github.com/anthropics/skills",
    # Add more sources as needed
]

def install_skill(skill_ref: str, skills_dir: Path) -> str:
    """
    Install a skill from the marketplace.

    skill_ref format: "owner/skill-name" or full GitHub URL
    """
    skills_dir.mkdir(parents=True, exist_ok=True)

    # Parse skill reference
    if skill_ref.startswith("http"):
        repo_url = skill_ref
        skill_name = skill_ref.rstrip("/").split("/")[-1]
    elif "/" in skill_ref:
        owner, skill_name = skill_ref.split("/", 1)
        repo_url = f"https://github.com/{owner}/skills"
    else:
        return f"Error: Invalid skill reference '{skill_ref}'. Use 'owner/skill-name' format."

    target_dir = skills_dir / skill_name

    if target_dir.exists():
        return f"Skill '{skill_name}' is already installed. Remove it first to reinstall."

    # Try to fetch from GitHub
    # For a monorepo like anthropics/skills, we need sparse checkout
    try:
        # Clone sparse checkout of just the skill directory
        temp_dir = skills_dir / f".tmp_{skill_name}"

        result = subprocess.run(
            ["git", "clone", "--depth", "1", "--filter=blob:none", "--sparse", repo_url, str(temp_dir)],
            capture_output=True,
            text=True
        )

        if result.returncode != 0:
            return f"Error cloning repository: {result.stderr}"

        # Set up sparse checkout for the skill
        subprocess.run(
            ["git", "-C", str(temp_dir), "sparse-checkout", "set", f"skills/{skill_name}"],
            capture_output=True
        )

        # Move the skill to the target location
        skill_source = temp_dir / "skills" / skill_name
        if skill_source.exists():
            shutil.move(str(skill_source), str(target_dir))
            shutil.rmtree(str(temp_dir))
            return f"Successfully installed skill '{skill_name}'."
        else:
            shutil.rmtree(str(temp_dir))
            return f"Error: Skill '{skill_name}' not found in repository."

    except Exception as e:
        return f"Error installing skill: {e}"

def remove_skill(skill_ref: str, skills_dir: Path) -> str:
    """Remove an installed skill."""
    skill_name = skill_ref.split("/")[-1] if "/" in skill_ref else skill_ref
    target_dir = skills_dir / skill_name

    if not target_dir.exists():
        return f"Skill '{skill_name}' is not installed."

    try:
        shutil.rmtree(str(target_dir))
        return f"Successfully removed skill '{skill_name}'."
    except Exception as e:
        return f"Error removing skill: {e}"

def search_skills(query: str) -> List[Dict[str, Any]]:
    """Search for skills in the marketplace."""
    # This would ideally query a marketplace API
    # For now, return a placeholder
    return [
        {"message": "Marketplace search not yet implemented. Check https://github.com/anthropics/skills for available skills."}
    ]

def list_available() -> List[Dict[str, Any]]:
    """List available skills from marketplace sources."""
    # This would ideally fetch from marketplace API
    return [
        {"message": "Marketplace listing not yet implemented. Check https://github.com/anthropics/skills for available skills."}
    ]
```

#### `lib/state.py` - Active Skill State Management

```python
"""Manage active skill state for allowed-tools enforcement."""
import json
from pathlib import Path
from typing import Optional, Dict, Any

# State file location (inside plugin directory)
STATE_FILE = Path(__file__).parent.parent / ".active_skill.json"

def get_active_skill() -> Optional[Dict[str, Any]]:
    """Get the currently active skill, if any."""
    if not STATE_FILE.exists():
        return None

    try:
        data = json.loads(STATE_FILE.read_text())
        return data
    except (json.JSONDecodeError, IOError):
        return None

def set_active_skill(name: str, allowed_tools: Optional[str]):
    """Set the active skill."""
    data = {
        "name": name,
        "allowed_tools": allowed_tools
    }
    STATE_FILE.write_text(json.dumps(data))

def clear_active_skill():
    """Clear the active skill state."""
    if STATE_FILE.exists():
        STATE_FILE.unlink()
```

### 2.4 CLI Invocation (via `chibi -P`)

The plugin supports direct CLI invocation for marketplace operations:

```bash
# Install a skill
chibi -P agent-skills install anthropics/pdf-processing

# Remove a skill
chibi -P agent-skills remove pdf-processing

# List installed skills
chibi -P agent-skills list_installed

# Search marketplace
chibi -P agent-skills search "code review"
```

This requires the plugin to parse `sys.argv` when not invoked via `--schema` or hooks:

```python
if __name__ == "__main__":
    if len(sys.argv) > 1 and sys.argv[1] == "--schema":
        print(json.dumps(get_schema()))
    elif os.environ.get("CHIBI_HOOK"):
        handle_hook()
    elif len(sys.argv) > 1 and sys.argv[1] not in ["--schema"]:
        # CLI mode: agent-skills <action> [args...]
        action = sys.argv[1]
        args = {"action": action}
        if len(sys.argv) > 2:
            if action in ["install", "remove"]:
                args["skill_ref"] = sys.argv[2]
            elif action == "search":
                args["query"] = " ".join(sys.argv[2:])
        handle_marketplace(args)
    else:
        handle_tool_call()
```

---

## Part 3: Implementation Notes

### 3.1 Tool Name Propagation

Currently chibi passes `CHIBI_TOOL_ARGS` but not the tool name. For multi-tool plugins, we need to know which tool was called.

**Option A:** Add `CHIBI_TOOL_NAME` environment variable (requires chibi change)

**Option B:** Include tool name in arguments (plugin can parse from args structure)

**Recommendation:** Implement Option A in chibi for cleanliness:

```rust
// In api.rs tool execution
.env("CHIBI_TOOL_NAME", &tc.name)
```

### 3.2 Schema Regeneration

When skills are installed/removed, the schema changes. Chibi loads schemas at startup, so users need to restart chibi to see new skills.

**Future enhancement:** Hot-reload plugins when their schema might have changed.

### 3.3 Skill Scope Lifetime

The `allowed-tools` enforcement tracks "active skill" state. The scope ends when:
- A new skill is invoked (new skill becomes active)
- The conversation ends (on_start hook clears state)
- User explicitly asks to "stop" the skill (could add a tool for this)

For now, keeping it simple: skill stays active until another skill is invoked or session ends.

### 3.4 Security Considerations

- Path traversal prevention in `read_skill_file` and `run_skill_script`
- Skill names validated against spec (alphanumeric + hyphens only)
- Marketplace installs are git clones (verify source trustworthiness)
- `allowed-tools` is advisory enforcement - LLM can technically ignore
- `run_skill_script` has a 120s timeout to prevent runaway processes
- Scripts run with the skill directory as CWD, sandboxed to that context
- Non-executable scripts are run via detected interpreter (python3, bash, node)

---

## Part 4: Testing Checklist

### Chibi Core Changes
- [ ] Plugin directory structure: `plugins/foo/foo` loads correctly
- [ ] `.disabled` suffix skips plugins
- [ ] Multi-tool schema: array of tools from single plugin
- [ ] `pre_tool` hook can block tool execution
- [ ] `CHIBI_TOOL_NAME` environment variable set

### Agent Skills Plugin
- [ ] `--schema` returns valid tool array
- [ ] `post_system_prompt` hook injects skill index
- [ ] Skill invocation returns SKILL.md body
- [ ] `read_skill_file` works with path traversal protection
- [ ] `run_skill_script` executes scripts with correct working directory
- [ ] `run_skill_script` handles stdin input
- [ ] `run_skill_script` respects timeout and returns errors properly
- [ ] `run_skill_script` prevents path traversal attacks
- [ ] `skill_marketplace install` clones and installs skill
- [ ] `skill_marketplace remove` deletes skill directory
- [ ] `pre_tool` hook enforces `allowed-tools`
- [ ] CLI mode works via `chibi -P agent-skills`

---

## References

- [Agent Skills Specification](https://agentskills.io/specification)
- [Anthropic Skills Repository](https://github.com/anthropics/skills)
- [Claude Code Skills Documentation](https://code.claude.com/docs/en/skills)
