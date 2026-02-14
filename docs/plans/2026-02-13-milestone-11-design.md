# milestone 11 — phased implementation plan

**date:** 2026-02-13
**milestone:** 11
**status:** planned

## overview

eight issues spanning config, tools, JSON modes, plugin health, and architectural assessment. ordered to build foundations first, extract stable interfaces last, and gate the release on a plugin audit.

---

## phase 1: foundation — config, tools, and project root

### #125 — read AGENTS.md from standard locations

add per-project and global instruction file loading, following the emerging [AGENTS.md convention](https://agents.md/).

**VCS root auto-detection:**
chibi currently defaults project root to cwd unless `--project-root` or `CHIBI_PROJECT_ROOT` is set. add auto-detection by walking up from cwd looking for VCS root markers:

| VCS | marker |
|-----|--------|
| git | `.git` (dir or file) |
| fossil | `.fslckout` or `_FOSSIL_` (file) |
| mercurial | `.hg` (dir) |
| svn | `.svn` (dir) |
| bazaar | `.bzr` (dir) |
| pijul | `.pijul` (dir) |
| jujutsu | `.jj` (dir) |
| cvs | `CVS/` (dir) — walk up while present; highest directory still containing `CVS/` is root |

nearest marker wins. explicit `--project-root` / `CHIBI_PROJECT_ROOT` still override.

**AGENTS.md discovery:**
once project root is determined, load instruction files from these locations (all found files concatenated in order, later entries appear later in the prompt):

1. `~/AGENTS.md` — user-global, tool-independent
2. `~/.chibi/AGENTS.md` — chibi-global
3. directory walk from project root down to cwd, checking for `AGENTS.md` at each level

content is appended to the system prompt after the base prompt. empty files are skipped.

### #132 — configuration surface to disable builtin tools

`tools.include`/`tools.exclude` already exists in `local.toml` but is undocumented.

**additions:**
- add tool categories: `coding`, `filesystem`, `agent`, `builtin`
- add `tools.exclude_categories` config field (list of category names)
- support the same `[tools]` config section in global `config.toml`, not just `local.toml`
- disabled tools are omitted from the API tool list entirely
- document the full tools config surface (available tools, categories, config syntax)

### #128 — coding tools work out-of-the-box with zero configuration

builds on #132. with no plugins installed:
- coding tools are already sent to the LLM
- destructive tools (`shell_exec`, `file_edit`, `write_file`) use interactive TTY permission prompts
- read-only tools (`dir_list`, `glob_files`, `grep_files`, `file_head`) need no permission

**work needed:**
- verify and document the zero-plugin experience
- assess headless/piped mode story (currently fail-safe denies all write ops) — may need a `--trust` or `--non-interactive-allow` flag
- document which plugins enhance vs gate functionality

---

## phase 2: assessment

### #130 — twelve-factor app audit

assessment only — no code changes in this milestone.

review chibi against the [twelve-factor methodology](https://12factor.net/). deliverable: a document per factor with verdict (satisfied / needs work / doesn't apply) and follow-up issues filed for future milestones where warranted.

---

## phase 3: ratatoskr detour

### #129 — builtin default free preset

**blocked on ratatoskr gaining model preset support.**

fey shifts to ratatoskr to land preset support and push to main. then chibi-side:
- wire up preset resolution in config/model handling
- ship a builtin default free preset (e.g. free-tier openrouter model)
- `config.toml` becomes optional — chibi is usable immediately after install

---

## phase 4: JSON modes

### #14 — --json-output should extend to all output

- audit all output paths for JSON mode compliance
- inventory which outputs should be available in JSON format
- make those output paths respect `--json-output`
- mark remaining flags as incompatible with `--json-output`

### #133 — pure JSON frontend: separate chibi-json crate

- new `crates/chibi-json/` workspace member
- depends on `chibi-core` only (no TUI/markdown/readline)
- exposes all JSON modes currently in chibi-cli
- chibi-cli may delegate to chibi-json for JSON operations

done after #14 so the extracted interface is complete and stable.

---

## phase 5: pre-release

### #131 — audit plugins for compatibility, functionality, and redundancy

final quality gate before tagging a release:
- test each plugin against current chibi version
- identify and fix broken plugins
- identify plugins made redundant by builtins or other changes
- deprecate or remove redundant plugins
- document plugin status

this runs last to catch any breakage from phases 1–4.

---

## dependency graph

```
#125 (AGENTS.md + VCS root)
  └→ #132 (disable builtin tools)
       └→ #128 (coding tools OOB)
            └→ #130 (twelve-factor audit)
                 └→ #129 (default preset — ratatoskr detour)
                      └→ #14 (JSON output)
                           └→ #133 (chibi-json crate)
                                └→ #131 (plugin audit — pre-release gate)
```
