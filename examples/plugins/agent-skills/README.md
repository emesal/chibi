# Agent Skills Plugin for Chibi

This plugin provides [Agent Skills](https://agentskills.io) support for chibi, enabling:
- Skill installation from GitHub marketplaces
- Progressive disclosure of specialized knowledge
- Tool restriction enforcement via `allowed-tools`
- Multi-tool plugin architecture

## Installation

Copy this directory to your chibi plugins directory:

```bash
cp -r examples/plugins/agent-skills ~/.chibi/plugins/
chmod +x ~/.chibi/plugins/agent-skills/agent-skills
```

## Requirements

- Python 3.10+
- `uv` (https://docs.astral.sh/uv/) - for dependency management
- `pyyaml` - installed automatically by uv

## Usage

The plugin automatically loads all skills from `~/.chibi/plugins/agent-skills/skills/` and provides:

**Management Tools:**
- `skill_marketplace` - Install, remove, search, or list skills
- `read_skill_file` - Read files from skill directories
- `run_skill_script` - Execute skill scripts with timeout protection

**Dynamic Tools:**
- `skill_[name]` - One tool per installed skill

## Installing Skills

Skills can be installed from GitHub repositories:

```bash
# From chibi, ask the LLM to install a skill:
chibi "Please use skill_marketplace to install the pdf-processing skill from anthropics/skills"
```

Or install manually by creating directories in `~/.chibi/plugins/agent-skills/skills/`.

## Creating Skills

Create a `SKILL.md` file with YAML frontmatter:

```markdown
---
name: my-skill
description: What this skill does
allowed-tools: Read, Grep
---

# My Skill Instructions

When this skill is activated, follow these instructions...
```

See the main README for complete skill authoring documentation.

## Architecture

- `agent-skills` - Main executable (Python with uv inline dependencies)
- `lib/parser.py` - SKILL.md parsing and validation
- `lib/marketplace.py` - GitHub sparse checkout for installation
- `lib/state.py` - Active skill tracking for tool restrictions
- `skills/` - Installed skills directory

## Security

- Path traversal protection in file operations
- Script execution sandboxed to skill directory
- 120-second timeout on script execution
- Tool restrictions enforced via pre_tool hook
- Skill names validated against spec
