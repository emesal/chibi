# Changelog

All notable changes to chibi are documented here.
Versions follow [Semantic Versioning](https://semver.org/).

## [0.10.0] - 2026-03-06

### Bug Fixes

- **docs:** Resolve all rustdoc link and HTML tag warnings ([`b1a96d2`](https://github.com/emesal/chibi/commit/b1a96d2f092b3d454547abe6325ca26f476c596a))

- Unified path resolver for all file-accessing builtin tools, #192 ([`942f95d`](https://github.com/emesal/chibi/commit/942f95dece4785c4a8f23efad008ae8896ab2b53))

- Use floor_char_boundary to prevent UTF-8 panics on string truncation ([`e55ca93`](https://github.com/emesal/chibi/commit/e55ca93afbcf21e1376a77f64da77fd9cf0023a2))

- Resolve 12 important issues from codebase review ([`2045797`](https://github.com/emesal/chibi/commit/2045797294492e58b6aed721efac6baa528b69e2))

- Resolve 14 suggestions from codebase review ([`dbeb0a2`](https://github.com/emesal/chibi/commit/dbeb0a24c5d5df1c04416a1d2414f049c5c9ea79))

- Post-review cleanup — DRY site_flock_name, doc accuracy, dead code removal ([`5f04f7b`](https://github.com/emesal/chibi/commit/5f04f7bca0832a2f7471b38c8cf1116ede5e7af6))

- Clarify VFS cache stub to prevent LLM retry loops ([`f34faf1`](https://github.com/emesal/chibi/commit/f34faf1b5fdbbaa679fcfe1ab57c2d66559f9211))

- Prefer stored context cwd over live current_dir for path resolution ([`78f508e`](https://github.com/emesal/chibi/commit/78f508e443c36e1c9e76813b9a34c88eb92fb7e9))

- Replace search-centric cache stub with generic tool wording ([`2587959`](https://github.com/emesal/chibi/commit/25879591fd56f74078e872b4f074c2d23d6da8e5))

- Update TranscriptEntry struct literals in chibi-cli tests ([`b188412`](https://github.com/emesal/chibi/commit/b18841274e10a22b13ac87c5bb32a6f04caaff99))


### Chores

- Add git-cliff changelog generation, bump to v0.9.0 ([`3335a74`](https://github.com/emesal/chibi/commit/3335a7446765a00cd68f7e98756e0549f4ef5367))

- Set ISC licence and publish = false on all crates ([`fde1897`](https://github.com/emesal/chibi/commit/fde1897560f4e72fb18707859540f5c864fb3ad7))

- Notes for loop detection and cwd persistence ([`65c659a`](https://github.com/emesal/chibi/commit/65c659a01eadfa20057c126d8507290d9230b380))

- Disable call_agent as LLM tool ([`172975a`](https://github.com/emesal/chibi/commit/172975a5fc14b13900ae9e0da5c4f3665e6eb64d))

- Apply fmt from just lint ([`fe0994d`](https://github.com/emesal/chibi/commit/fe0994d7d3b25b0cc70fb783837bc361df496d58))


### Documentation

- Sync all docs/ with reality ([`035fdd8`](https://github.com/emesal/chibi/commit/035fdd821705eeee53717a0fcd1be7aabf047046))

- Update CHANGELOG for v0.9.0 ([`a491623`](https://github.com/emesal/chibi/commit/a491623aa2d7227680e4b9b201a1c97739972e88))

- Update CHANGELOG for v0.9.1 ([`ab7b1f5`](https://github.com/emesal/chibi/commit/ab7b1f57378f9776d814722b7d49c8460290d988))

- Codebase review findings (2026-02-25) ([`95abf09`](https://github.com/emesal/chibi/commit/95abf09d43f93164bbc32592f990a99f69322107))

- Update CHANGELOG for v0.9.2 ([`a0c97f1`](https://github.com/emesal/chibi/commit/a0c97f1b95398694d2c07ba0ef636377839edb6a))

- **tools:** Update mod.rs doc comments to reflect new group structure ([`b2dc2dd`](https://github.com/emesal/chibi/commit/b2dc2ddd7b9fc901732c47860438a070fe0de5d9))

- Update architecture.md for tool group refactor ([`3e7546a`](https://github.com/emesal/chibi/commit/3e7546a2b644556fe586c9aeef0c6d020050e348))

- Add loop prevention section to agentic.md ([`33be520`](https://github.com/emesal/chibi/commit/33be520f9c27f133541000833f0eb7a8efbbdda0))

- Update cache stub example in agentic.md ([`2518c86`](https://github.com/emesal/chibi/commit/2518c86037b4a52efd379aadf5de4cb32126536c))

- Update transcript format for unified flow control messages ([`898672f`](https://github.com/emesal/chibi/commit/898672f8dbb68b19d884427e8f4fc1af188a305f))

- Add call_agent and TranscriptEntry quirks to AGENTS.md ([`45269b6`](https://github.com/emesal/chibi/commit/45269b6952571e71e68c2b41bb2d485a0d7821fd))


### Features

- **vfs:** Introduce VfsCaller enum replacing &str caller ([`200cb2d`](https://github.com/emesal/chibi/commit/200cb2df272f060ad439384e75ba6d4d328ba879))

- Site identity, Vfs site_id, and flock registry types ([`6b90ac7`](https://github.com/emesal/chibi/commit/6b90ac78ff12395bf6c74aae8c6729c0c42b2068))

- VFS zones, flock ops, todos migration, /site/ bootstrap (tasks 6-8, 12) ([`78e4742`](https://github.com/emesal/chibi/commit/78e4742a0e30aba333de43348ba25129847aa636))

- Flock goals, LLM tools, and CLI flock management (tasks 9-11) ([`8b0925f`](https://github.com/emesal/chibi/commit/8b0925f7297ff9496adb2392bc62dec1b55c41f5))

- Complete flocks migration — hook payloads, read_context, compaction, and docs (tasks 13-16) ([`b33302f`](https://github.com/emesal/chibi/commit/b33302f109e92d6f71bdf3a3be9c0f0b79f35d05))

- Print site flock name in --version output ([`cdb2ef7`](https://github.com/emesal/chibi/commit/cdb2ef77e8dddf38cb99e4234cb27c709e5359c4))

- Persist cwd in ContextEntry for cross-session path stability ([`ec0f1f7`](https://github.com/emesal/chibi/commit/ec0f1f797c728e049f4b6fb544d6f8cbd11f36a0))

- Add LoopDetector for consecutive identical tool call detection ([`a59aa84`](https://github.com/emesal/chibi/commit/a59aa849d4f94b1279cc0f1f716d3f7a95d1e953))

- Wire LoopDetector into process_tool_calls with fuel penalty and warning injection ([`644769a`](https://github.com/emesal/chibi/commit/644769a3eca3f0aa5f3ff2572d9ad7d668181348))

- Extract stdout/stderr from JSON outputs for cache preview ([`84b9dc7`](https://github.com/emesal/chibi/commit/84b9dc78958d239af79321aafeeb52f158a411e7))

- Add role and flow_control fields to TranscriptEntry ([`77e2306`](https://github.com/emesal/chibi/commit/77e23062cc9d576376c788d1e57df4af562c90b9))

- Entry helpers with role, flow_control, and control_transfer ([`b4dcc9e`](https://github.com/emesal/chibi/commit/b4dcc9eaec747eeaa81e288828f7dd42fed0dd43))

- Entries_to_messages uses role field with backwards-compat fallback ([`2eb9f33`](https://github.com/emesal/chibi/commit/2eb9f33610398c7aa1cfaffd1702ed8099946324))

- Call_user produces message + control_transfer entries ([`32f8019`](https://github.com/emesal/chibi/commit/32f80197ea3c481ad67cb69808758dad87b85f92))

- Unified flow control messages (#211) ([`2dc235e`](https://github.com/emesal/chibi/commit/2dc235e6d36435ab5beab72b412f845ad65ec6d7))


### Refactoring

- **config:** Fold models.toml into config.toml and local.toml ([`64de559`](https://github.com/emesal/chibi/commit/64de559f3caffa7567d2bca34bcc328fcda12c7e))

- Extract apply_pre_tool_results and apply_pre_tool_output_results from execute_tool_pure ([`e633dfe`](https://github.com/emesal/chibi/commit/e633dfef6bdf4bc6aaf7e0291f3e1511422e3b58))

- Unify dual request-building paths (#4) ([`ef8b26b`](https://github.com/emesal/chibi/commit/ef8b26bdf9f80efd7a71664bbf1ebb342dac2a97))

- **tools:** Move shared BuiltinToolDef/require_str_param to mod.rs ([`ebd7fde`](https://github.com/emesal/chibi/commit/ebd7fdee63ea73c479723904e08934071c1b38a2))

- **tools:** Extract memory tool group (reflection, todos, goals, read_context) ([`812aadd`](https://github.com/emesal/chibi/commit/812aaddac85a65421f68e6495002be0d7b4dc00c))

- **tools:** Extract flow tool group (call_agent, call_user, send_message, model_info, spawn_agent, summarize_content) ([`99607ec`](https://github.com/emesal/chibi/commit/99607ecff1620d8276e83d4c7d294ebf6cece50e))

- **tools:** Extract fs_read tool group (file_head/tail/lines/grep, dir_list, glob_files, grep_files) ([`50d6f1f`](https://github.com/emesal/chibi/commit/50d6f1f13072505692120a8fcfaf741b18684ad4))

- **tools:** Extract fs_write tool group (write_file, file_edit) ([`0859266`](https://github.com/emesal/chibi/commit/08592663112e56ec544a69aff857b4ded90d5abf))

- **tools:** Extract shell, network, index tool groups ([`43a0cc5`](https://github.com/emesal/chibi/commit/43a0cc52c5d7a2e1fe9313569cf030f388424a25))

- **tools:** Update dispatchers to use per-group predicates ([`21092d7`](https://github.com/emesal/chibi/commit/21092d79c202e7891e1e7932eae9111b383c30d8))

- **tools:** Delete legacy builtin/file_tools/coding_tools/agent_tools modules ([`242a841`](https://github.com/emesal/chibi/commit/242a84142f0299b1c6a7b4912352e04f04674ec3))

- **vfs:** Migrate all call sites to VfsCaller enum ([`3f05bfb`](https://github.com/emesal/chibi/commit/3f05bfbdd5e471089ca48ac963d6a6211c1fb81d))

- Remove flow_control_call/result entry types ([`a1962e0`](https://github.com/emesal/chibi/commit/a1962e0d4601bdf11d7e926c43488cf628b11441))


### Tests

- **compact:** Add unit + integration tests for compaction logic, closes #168 ([`a68f149`](https://github.com/emesal/chibi/commit/a68f14932c3c94acd6cbd3c75997e6ef060effdd))

- **cache:** TTL expiry integration + execute_command cache tests, closes #172 ([`2be0c90`](https://github.com/emesal/chibi/commit/2be0c9056f097ee1cd4b3833824591390ef6edb5))

- **hooks:** PreTool, PreToolOutput, and PreApiTools filter tests, #173 ([`0631f31`](https://github.com/emesal/chibi/commit/0631f313f621dc99bb5de9a32b1b5eb58ca38d2b))

- **hooks:** PreApiRequest modification + PreAgenticLoop/PostToolBatch override tests, #173 ([`1b00083`](https://github.com/emesal/chibi/commit/1b00083c4276183123eef23fd8450d9348bb0a84))

- **hooks:** Failure cascade + ordering tests, #173 ([`744b447`](https://github.com/emesal/chibi/commit/744b447f140f7014aaaf89bbd570b781fe515d1e))

- **compaction:** Rolling_compact real-LLM integration test, #169 ([`74b94a8`](https://github.com/emesal/chibi/commit/74b94a85310f45e08e344d22e7ba7ea8e38d232a))

- **compaction:** Unit tests for manual compact — early return, empty summary, bootstrap structure ([`313e140`](https://github.com/emesal/chibi/commit/313e140c180a3fa8884c84000f1daaf08476fc06))

- **compaction:** Large transcript stress test — repeated rolling compaction, #174 ([`24f7ed1`](https://github.com/emesal/chibi/commit/24f7ed121afad71aeb4398b273adf129e8f7208b))


## [0.8.10] - 2026-02-22

### Bug Fixes

- **core:** Zero-config broken by derive(Default) bypassing serde defaults ([`a70b284`](https://github.com/emesal/chibi/commit/a70b284a9ed220defb1efa236a0400afd1a4216c))


### Documentation

- Sync README with reality, drop stale submodule references ([`4c0c2ad`](https://github.com/emesal/chibi/commit/4c0c2aded5fd497d265043c08e105e8844c6187f))

- Correct zero-config claim, openrouter requires api key ([`ddf4394`](https://github.com/emesal/chibi/commit/ddf4394fa6713cff52a5fff90a5087472e9d149c))

- **readme:** Rewrite intro paragraphs, drop lego metaphor ([`bedc672`](https://github.com/emesal/chibi/commit/bedc672fbb4d8ef7f8e50a88fe3469fea9ae0a18))


### Features

- **core:** Include flow-control tool exchanges in context ([`498cdbf`](https://github.com/emesal/chibi/commit/498cdbfe815c593c5cad0b9f857de9e62e49be5d))

- **cli:** Render markdown in -g/-G log output ([`ef0a104`](https://github.com/emesal/chibi/commit/ef0a104eb3a75d101806416b6cce4e921da7262c))


## [0.8.9] - 2026-02-22

### Bug Fixes

- **core:** Correctly reconstruct sequential tool call turns from context ([`482bfb2`](https://github.com/emesal/chibi/commit/482bfb203b8db4fe70677d2efa4e4bc34499b496))

- **core:** Suppress flow-control tool exchanges from context.jsonl (#180) ([`8fca6f7`](https://github.com/emesal/chibi/commit/8fca6f7642c9ce86e21b8e8bcc40fe5cc431103e))

- **core:** Make call_user immediately end the turn ([`bc77661`](https://github.com/emesal/chibi/commit/bc77661eeb2380a05f97db4ef5fbd8721ab63ec6))

- **compact:** Use char boundary for tool result preview truncation ([`b0d8218`](https://github.com/emesal/chibi/commit/b0d8218c42df79c86f7e10a1e265f273f8683a00))

- **core:** Make call_user immediately end the turn (#184) ([`357a034`](https://github.com/emesal/chibi/commit/357a0345ee6b655857dd3f2698beb06fdbf52247))

- **compact:** Remove post-compaction acknowledgment LLM call ([`1ae3230`](https://github.com/emesal/chibi/commit/1ae3230b47eea7600bc2481d5d6af078bb4b16de))


### Refactoring

- **prompts:** Overhaul default prompts and system prompt construction ([`bf47106`](https://github.com/emesal/chibi/commit/bf471062989e0d33e78c8360450d81d8d217753b))


## [0.8.8] - 2026-02-21

### Bug Fixes

- **mcp-bridge:** Prevent double-spawn race and improve stale lock detection ([`4715813`](https://github.com/emesal/chibi/commit/4715813930c8c886f2b990726668c1ad3f3b6544))


### Documentation

- Update auto-destroy documentation for promoted flags ([`6bd8cc8`](https://github.com/emesal/chibi/commit/6bd8cc8efb4057261f8f134ae34525513ff06973))


### Features

- **mcp-bridge:** Retry summary generation on transient errors with backoff ([`02b2d46`](https://github.com/emesal/chibi/commit/02b2d46eb3f448ee6fb61b21dd7cc8d1b67da765))

- **cli:** Add --destroy-at and --destroy-after-inactive flags ([`25e65e3`](https://github.com/emesal/chibi/commit/25e65e374d25b7c68394cd370357b3395548e960))


### Refactoring

- **prompts:** Embed defaults via include_str!, move to chibi-core/prompts/ ([`7dc5160`](https://github.com/emesal/chibi/commit/7dc51605734dd556af179fcac1ed607d734e1e78))

- **prompts:** Externalize remaining inline prompts to files ([`ab2549f`](https://github.com/emesal/chibi/commit/ab2549f2350945bab614444ca65a827c97ab4da4))

- **input:** Promote destroy flags from DebugKey to ExecutionFlags ([`4131ce9`](https://github.com/emesal/chibi/commit/4131ce93da937f87a42e7ffc0261e2683538a347))

- Promote destroy flags from --debug sub-keys to first-class CLI flags (#178) ([`3483419`](https://github.com/emesal/chibi/commit/3483419dbec33f1492e29c1936653e54f5c451bc))


### Tests

- **cli:** Update integration tests to use --destroy-after-inactive ([`a0ea7a1`](https://github.com/emesal/chibi/commit/a0ea7a194ce0e7f0880eb378627dacac038a6ff9))


## [0.8.7] - 2026-02-21

### Chores

- Add set-model plan files and fmt fix ([`8989555`](https://github.com/emesal/chibi/commit/8989555c45666204f222bf2ab3abf8d7b7e5983c))

- Add .worktrees to gitignore ([`630c657`](https://github.com/emesal/chibi/commit/630c6576b223dd761c1f0a9213801d21feddc600))

- Add .worktrees to gitignore ([`28cf001`](https://github.com/emesal/chibi/commit/28cf001e8fa4b4d80bf7c060192d87033ca45819))

- **mcp-bridge:** Switch default summary model to ratatoskr:free/summariser ([`d625ea6`](https://github.com/emesal/chibi/commit/d625ea6740838313fa1bcdd5c9866057f6b8d53e))


### Documentation

- Update cli-reference for set-model flags and reassigned -m/-M ([`158f85d`](https://github.com/emesal/chibi/commit/158f85de604fe72b0619fc7abe4b6a19befbd295))

- Document CLI shortcut for persistent model setting in per-context config ([`b670ee6`](https://github.com/emesal/chibi/commit/b670ee60aeec08d20e193f48d2e3e5ddd959d7fd))

- Document subagent_cost_tier and spawn_agent preset support ([`54a6365`](https://github.com/emesal/chibi/commit/54a6365a11e48a944064479ffe6653ac851407e2))

- **plans:** Add remote MCP server design and implementation plan ([`0b6f1b3`](https://github.com/emesal/chibi/commit/0b6f1b3837bafbd40912241966a0b81db1c88faa))


### Features

- **core:** Add Command::SetModel and CommandEvent::ModelSet ([`fac6e67`](https://github.com/emesal/chibi/commit/fac6e67fb74dce4a29d7889340988f76896277fd))

- **core:** Implement SetModel command handler with live validation ([`14c2a59`](https://github.com/emesal/chibi/commit/14c2a594fb052c74e2af867518c159d0040cd881))

- **cli:** Add -m/--set-model and -M/--set-model-for-context; make model-metadata long-only ([`38cb583`](https://github.com/emesal/chibi/commit/38cb5834dea67e9141c5178f921dfc5864966828))

- **cli:** Add -m/--set-model and -M/--set-model-for-context; make model-metadata long-only ([`dfc061f`](https://github.com/emesal/chibi/commit/dfc061fd328624cddfac824a60813da74b782cf5))

- **config:** Add subagent_cost_tier to config stack ([`0eed1ab`](https://github.com/emesal/chibi/commit/0eed1ab84976c85a0493d5703ddb88fa31ea57ad))

- **agent_tools:** Add preset to SpawnOptions and apply_spawn_options ([`2cc6849`](https://github.com/emesal/chibi/commit/2cc6849924f63e6ebcee0abda6427ee2073804f9))

- **agent_tools:** Wire gateway into spawn_agent for preset resolution ([`384caca`](https://github.com/emesal/chibi/commit/384cacad8725279200e1cb60727941a83f8de3d1))

- **agent_tools:** Dynamic preset capability list in spawn_agent tool description ([`9a885ea`](https://github.com/emesal/chibi/commit/9a885ea9963d3858c8be466c80a12cff29b3e254))

- **mcp-bridge:** Add remote MCP server support via streamable HTTP ([`0f0ce43`](https://github.com/emesal/chibi/commit/0f0ce43ab2a56d5797377d44c55d1cff90b51157))


### Refactoring

- **json:** Collapse JsonInput per-invocation fields into config overrides ([`9ca85a2`](https://github.com/emesal/chibi/commit/9ca85a28e932b48984763d362af5f76f360bf61c))


### Tests

- **core:** Use 1s heartbeat in lock tests to avoid 30s worst-case wakeup ([`b75e465`](https://github.com/emesal/chibi/commit/b75e465ba28ee538984090964d5125df68e42a94))


## [0.8.6] - 2026-02-20

### Bug Fixes

- Use ports.ubuntu.com for arm64 cross-compilation packages ([`18433bd`](https://github.com/emesal/chibi/commit/18433bdc1c65051ee2af4a40875174988ba355f1))

- Remove TranscriptEntry stdout leak + transcript.md machinery ([`9ba2c10`](https://github.com/emesal/chibi/commit/9ba2c100141893cbc333bbf1eabfc9c39ce91fd2))

- **context:** Reject reserved VFS caller names (SYSTEM) ([`eb50f06`](https://github.com/emesal/chibi/commit/eb50f062f57e60433b0e199d61b6812c95f64541))

- **vfs:** Validate backend config, eager dir creation, dedup require_str_param ([`c788d64`](https://github.com/emesal/chibi/commit/c788d64fc2b2cc42795df1a3d1a2c5a0c8f725e3))

- **vfs:** Fix OS permission gate applied to vfs:// paths in file tools ([`5f6028c`](https://github.com/emesal/chibi/commit/5f6028c5d9b4ee95ad45ead3be5ad6191f89d4e3))

- **vfs:** VfsPath::join explicitly rejects '.' components, add test ([`4cd515e`](https://github.com/emesal/chibi/commit/4cd515e0f13019b5fff8be0395ad7ec66910c75e))

- **vfs:** Replace blocking Path::exists()/is_dir() with tokio::fs::metadata in LocalBackend ([`3fc7fa3`](https://github.com/emesal/chibi/commit/3fc7fa3f6f29c18a47c7ec0ef4476bb993a15139))

- **docs:** Remove stale CHIBI_VERBOSE doc from execute_tool ([`e5d54e2`](https://github.com/emesal/chibi/commit/e5d54e21a9a0999c87cb3b7713a18b5f744b62e2))

- **config:** Restore show_thinking default to true ([`014324d`](https://github.com/emesal/chibi/commit/014324dbcf9242baf036a67925bd7374296fc7a8))

- **send:** Prevent double-emission of tool diagnostics for sequential tools ([`0ea1f6e`](https://github.com/emesal/chibi/commit/0ea1f6e8e7a334b8577522b2e992651d256615e9))

- **core:** Move NoopSink before test module (clippy items_after_test_module) ([`d9b063b`](https://github.com/emesal/chibi/commit/d9b063b5046c1126ca6742aed03326d1d6ca7710))


### Chores

- Cargo fmt (#161) ([`02be666`](https://github.com/emesal/chibi/commit/02be66696a6640f6a15f6330020aad92ce49d790))

- Fmt cleanup and add plan docs ([`c694fd0`](https://github.com/emesal/chibi/commit/c694fd0ab9c418503ece47f142f3df6855d728f5))


### Documentation

- Move detailed architecture/plugin info from AGENTS.md to docs/ ([`ae11631`](https://github.com/emesal/chibi/commit/ae1163141f933db09f6fe33e4fe39ef3ae37e008))

- Design for ExecutionFlags → config migration (#161) ([`ee76714`](https://github.com/emesal/chibi/commit/ee76714aefe01e09e69b184cf0fff80dc8ec0db2))

- Implementation plan for ExecutionFlags migration (#161) ([`da5fda2`](https://github.com/emesal/chibi/commit/da5fda200d482999dce2b98db3540af974ec40b7))

- Overhaul mcp-bridge-serena.md and link from mcp.md ([`48c14da`](https://github.com/emesal/chibi/commit/48c14daa7fee5911a20ca1a23404114367318f3c))

- Update for ExecutionFlags migration (#161) ([`0855c8b`](https://github.com/emesal/chibi/commit/0855c8b01246f0be5062370f7423108d5d252093))

- **config:** Document fuel=0 as unlimited mode sentinel ([`d5f78e4`](https://github.com/emesal/chibi/commit/d5f78e420739dd66827976a09128c5651aa8babf))

- Document fuel=0 unlimited mode ([`e5c0759`](https://github.com/emesal/chibi/commit/e5c0759248c0bc6887af917ae3d5b1f2159a3a08))

- VFS design document ([`ec7a3c1`](https://github.com/emesal/chibi/commit/ec7a3c13f213c1913357ea5a8f93475137da64df))

- VFS implementation plan ([`d34604e`](https://github.com/emesal/chibi/commit/d34604e383c448a9055d1354393ccc1f7ca2c07e))

- Add VFS documentation ([`ac1abac`](https://github.com/emesal/chibi/commit/ac1abacdf1d3ea3dad171a8d9a89d221243b7a3f))

- **vfs:** Document multi_thread runtime requirement on vfs_block_on ([`0241ab5`](https://github.com/emesal/chibi/commit/0241ab5c9044de64be97114a7b00c62c041ef6a7))

- **vfs:** Document intentional world-readable read in execute_file_edit_vfs ([`7a4dd2f`](https://github.com/emesal/chibi/commit/7a4dd2fb65d4f97247f20a1b16691a351a10bfcf))

- **vfs:** Document VfsConfig as intentionally global-only, not in ResolvedConfig ([`2e2de0e`](https://github.com/emesal/chibi/commit/2e2de0e7cb7a07af159b16de51b15c7c815ba275))

- Update vfs.md, hooks.md, agentic.md, configuration.md, plugins.md for VFS-backed tool cache ([`90a5ffa`](https://github.com/emesal/chibi/commit/90a5ffae90a913f3fb3cdd9e2ef134ae16ba5495))

- **vfs-cache:** Clarify vfs_uri_for three-slash format in comment ([`2e3ebab`](https://github.com/emesal/chibi/commit/2e3ebabf6d340093c636e5a0d447390c08c62ba7))

- Update configuration.md and cli-reference.md for presentation layer refactor ([`35a65eb`](https://github.com/emesal/chibi/commit/35a65eb049d011cd3521c5f4e4b871e828c90cc0))

- Update chibi-json stdout/stderr contract and done signal ([`7af7ca9`](https://github.com/emesal/chibi/commit/7af7ca9d4f4ea620eccd602efce5c232551839f0))

- Add structured error output plan files ([`34af757`](https://github.com/emesal/chibi/commit/34af75718474496a40b4816b409a68d372a494ec))


### Features

- Add show_thinking to core config layer (#161) ([`01d3ab0`](https://github.com/emesal/chibi/commit/01d3ab0dee7e0731f49b7359c049c096a628cf4b))

- Enable show_thinking by default ([`9142ed9`](https://github.com/emesal/chibi/commit/9142ed981dfca9098b085a769a633a5e8f262f19))

- **send:** Skip fuel tracking when fuel=0 (unlimited mode) ([`a3cf3ef`](https://github.com/emesal/chibi/commit/a3cf3ef33d902abb15fa3a10660f5f2d7ad99005))

- **send:** Omit fuel keys from hook payloads in unlimited mode ([`0c6dc50`](https://github.com/emesal/chibi/commit/0c6dc50219f4d651fa28f25afce147629f394a13))

- **vfs:** Add VfsPath newtype with validation and URI parsing ([`40b94a9`](https://github.com/emesal/chibi/commit/40b94a9bfac8870830b0cfc2676c6aee706615a6))

- **vfs:** Add backend trait, types, and permission model ([`863ded8`](https://github.com/emesal/chibi/commit/863ded84b393e77635d58026914f29385e273f3f))

- **vfs:** Add LocalBackend (disk-based VFS storage) ([`69cf4c5`](https://github.com/emesal/chibi/commit/69cf4c50255e868323fbddb6d9b08857b4e2b73f))

- **vfs:** Add Vfs router with zone-based permission enforcement ([`e981c52`](https://github.com/emesal/chibi/commit/e981c523b9395ab8d4c737f5c3454f98661e3c84))

- **vfs:** Add VfsConfig and wire Vfs into AppState ([`7f5203c`](https://github.com/emesal/chibi/commit/7f5203c73f92b799752b3eb572b1702f47c3e479))

- **vfs:** Add dedicated VFS tools (list, info, copy, move, mkdir, delete) ([`a448438`](https://github.com/emesal/chibi/commit/a4484382d01f831269cf16af2056a27a2a8f58ae))

- **vfs:** Wire vfs:// prefix into existing file and coding tools ([`c6cf0fc`](https://github.com/emesal/chibi/commit/c6cf0fc122a77f22e4aa4ad832cf0973861abd5b))

- **vfs:** Wire VFS tools into tool execution pipeline ([`1d285be`](https://github.com/emesal/chibi/commit/1d285bea9d5979e93f8f451d7189126b00eee135))

- **vfs:** Add Ord, PartialOrd, Serialize, Deserialize to VfsPath ([`7209d49`](https://github.com/emesal/chibi/commit/7209d496fd771a46970bb7193c49cd959f5ed995))

- **cache:** Replace file-based cache write with VFS at /sys/tool_cache ([`b99873e`](https://github.com/emesal/chibi/commit/b99873e020b5212ce9d809a499930514dbc59346))

- **cache:** Replace AppState cache cleanup with async VFS-backed versions ([`0a1092d`](https://github.com/emesal/chibi/commit/0a1092d9f046889277d124d6dcf7f8e31307061f))

- **cli/config:** Add verbose, hide_tool_calls, show_thinking to CliConfig; wire CLI flags ([`6ed56e7`](https://github.com/emesal/chibi/commit/6ed56e7070ef53cf26ee3836efba61265a59aede))

- **compact:** Route compaction output through OutputSink instead of eprintln ([`05e7bf5`](https://github.com/emesal/chibi/commit/05e7bf55dfaac67586eceb2b89a7536385f59a89))

- **core:** Add emit_done to OutputSink trait with default no-op ([`2e30872`](https://github.com/emesal/chibi/commit/2e30872e17e8d05ad24ca9209e06f1f792414a00))

- **json:** Implement emit_done with coarse error codes on stderr ([`785cd1c`](https://github.com/emesal/chibi/commit/785cd1cbf0093b1253879f82c152a6b8f6249f90))

- **json:** Wire emit_done in main, remove old stdout error emission ([`b481a55`](https://github.com/emesal/chibi/commit/b481a55c5847bd1e8dc0a8ef1bb64ecc5941da88))


### Refactoring

- Remove is_json_mode() from OutputSink/ResponseSink (#148) ([`dcc6a4b`](https://github.com/emesal/chibi/commit/dcc6a4bcaea7bc8c34dcd4ba2f12598ebf5d7dfc))

- Shrink ExecutionFlags to ephemeral modifiers only (#161) ([`d19c4fb`](https://github.com/emesal/chibi/commit/d19c4fb07677a5dbcf1c49e672008fef6510fa9f))

- Execution reads config instead of flags for behavioural settings (#161) ([`a8b4b5b`](https://github.com/emesal/chibi/commit/a8b4b5b1d535a23373a05c548e881a0cad9975e2))

- **cli:** Use set_field for flag→config overrides (#161) ([`27f5d07`](https://github.com/emesal/chibi/commit/27f5d07c6dc8f4cafd4aa220fbf9d2d5718ffffc))

- **json:** Remove flag merge logic, use config overrides (#161) ([`2fd62b7`](https://github.com/emesal/chibi/commit/2fd62b7725af2b9a4c2b5f1e44b186cb99a4cf53))

- **vfs:** Rename VFS_URI_PREFIX -> VFS_URI_SCHEME, add clarifying comment ([`314c993`](https://github.com/emesal/chibi/commit/314c993d95dbc00d84507ace3e90c95b42084930))

- **file-tools:** Remove cache_id param and cache_list tool, path= is the only resolver ([`b95d1dc`](https://github.com/emesal/chibi/commit/b95d1dc924f6c0d45f9337606b25016fbeaf0f5a))

- **state:** Remove tool_cache_dir, cache_file, cache_meta_file path helpers ([`b83310b`](https://github.com/emesal/chibi/commit/b83310b7cbb496f3785d50ad0dac469401ecf662))

- **cache:** Delete cache.rs — tool cache now lives in VFS ([`e3ff38f`](https://github.com/emesal/chibi/commit/e3ff38f17667e1c842e881e1efd35b52803fdc8f))

- **sink:** Replace Diagnostic with typed ResponseEvent variants ([`baa4a14`](https://github.com/emesal/chibi/commit/baa4a149baa1006739e5a3aca9cdcb4ef87ce53f))

- **send:** Emit typed ResponseEvent variants, remove verbose gating ([`f1adcdb`](https://github.com/emesal/chibi/commit/f1adcdbe3c0b1a3eafbaf40031e41898536a127b))

- **cli/sink:** Handle typed ResponseEvent variants with local filtering ([`dccfef4`](https://github.com/emesal/chibi/commit/dccfef43b8a23c9ef345a1687275ba7c32322724))

- **json/sink:** Emit all typed ResponseEvent variants as structured JSONL ([`59e175f`](https://github.com/emesal/chibi/commit/59e175fc0d51772a10ce2e686575438f5051c439))

- **core:** Remove verbose from PromptOptions ([`d8a62f9`](https://github.com/emesal/chibi/commit/d8a62f92ff5aac34c3644e571d099d0636eb1841))

- **config:** Remove verbose, hide_tool_calls, show_thinking from core; add CommandEvent ([`583ad0f`](https://github.com/emesal/chibi/commit/583ad0fb05dd6cea4cd1f86c21136f7bacde6ba7))

- **json:** Remove verbose load-time handling, emit CommandEvent ([`5ac961f`](https://github.com/emesal/chibi/commit/5ac961f572f506181a9b10de63ab25964dad4b36))

- **output:** Remove diagnostic/diagnostic_always from OutputSink ([`3961919`](https://github.com/emesal/chibi/commit/396191932c4f5294dd8ed1ed5acdd138bad14010))

- **load:** Replace LoadOptions.verbose with typed load events via OutputSink ([`f74d611`](https://github.com/emesal/chibi/commit/f74d6115fedc1b49f9a881e3c6fb481af1aab594))

- **core:** Deduplicate NoopSink to module level in chibi.rs ([`e4e5ec6`](https://github.com/emesal/chibi/commit/e4e5ec6204320f4616f8c9a2dc65ba27acaf80f5))

- **cli:** Simplify emit_event, remove dead verbose_only bool ([`b706289`](https://github.com/emesal/chibi/commit/b7062895cf72aa740b0471a8d4927cc9c3f4a234))

- **output:** Remove redundant ContextLoaded event (LoadSummary covers it) ([`1e3278d`](https://github.com/emesal/chibi/commit/1e3278d64b0e2c82224d2d6ce8935e19e5dcbdb2))


### Tests

- **send:** Verify fuel omitted from continuation prompt in unlimited mode ([`bbfaac1`](https://github.com/emesal/chibi/commit/bbfaac1bafc4c25af33d1b9bf30046ef7018528c))

- **vfs:** Add comprehensive multi-context integration test ([`a9b9718`](https://github.com/emesal/chibi/commit/a9b9718e68d70e1a879ae4ffecbde8478a0e31be))

- **cache:** Add end-to-end VFS cache flow integration test ([`04cd3d6`](https://github.com/emesal/chibi/commit/04cd3d625c5332766cbdc052d6312e98448c9ed1))

- **cli:** Add emit_event coverage for all CommandEvent variants ([`ed8c48b`](https://github.com/emesal/chibi/commit/ed8c48b22dceff958bfd63acf476f94d9369f628))

- **core:** Add compaction pure logic tests (#168) ([`7769063`](https://github.com/emesal/chibi/commit/7769063a4ecd3078d2a6a4da1cd22d3bba55a8ce))

- **core:** Add tool cache lifecycle tests (#172) ([`3ea8c98`](https://github.com/emesal/chibi/commit/3ea8c98327a19db3b5311533293b0585774dd77b))

- **core:** Add execution dispatch tests, shared test infra (#171, #175) ([`e5da43a`](https://github.com/emesal/chibi/commit/e5da43a00c2e90cc94715c1e8d13fbc20e95ce7a))


## [0.8.5] - 2026-02-17

### Bug Fixes

- **core:** Set shell_exec CWD to project_root ([`adcb339`](https://github.com/emesal/chibi/commit/adcb3395aa0e040713f0898acba7db29456284c5))

- **core:** Resolve file_tools relative paths against project_root ([`ce1d928`](https://github.com/emesal/chibi/commit/ce1d9287b3c6268fe86ecb3d2708123003cd028a))

- **core:** Add project_root to file_tools_allowed_paths ([`24e2b9e`](https://github.com/emesal/chibi/commit/24e2b9e30da8a3e33f321111427aa09f2294fc2e))

- **core:** Add integration tests for project_root consistency ([`8d830d0`](https://github.com/emesal/chibi/commit/8d830d026a82b6744362237849d26c67096a598d))

- Project_root consistently applied across file tools and shell_exec (#160) ([`2b9522e`](https://github.com/emesal/chibi/commit/2b9522e8cb8562184c1e1ef20f2ee7785a7a6071))


### Features

- Per-invocation config overrides (#157) ([`15641ea`](https://github.com/emesal/chibi/commit/15641ea821e21507a240f7b5dce845cd757799cc))

- Per-invocation config overrides (#163) ([`59be26a`](https://github.com/emesal/chibi/commit/59be26a423e939448fb90f960b987d8def8417de))


## [0.8.3] - 2026-02-16

### Bug Fixes

- **mcp-bridge:** MCP dispatch in agentic loop, mutex contention (#156) ([`20a8bb2`](https://github.com/emesal/chibi/commit/20a8bb2404ad81c70d3653c4101a6765d694d1ad))

- **mcp-bridge:** Bail on first summary generation failure ([`401e8d0`](https://github.com/emesal/chibi/commit/401e8d07bfcef747cefb2aebdd142cc28dc15674))

- **mcp-bridge:** Read api_key from config.toml for summary generation ([`c46518f`](https://github.com/emesal/chibi/commit/c46518f44e783706fedcd57abe0ab6f8aaa45382))

- **mcp-bridge:** Recover from stale lockfiles ([`64460d6`](https://github.com/emesal/chibi/commit/64460d66814848cc131330db7f4c38053dc0b30c))

- **mcp-bridge:** Skip caching empty summaries ([`b795345`](https://github.com/emesal/chibi/commit/b795345ac4791324af63b458cc97e63e3c89faa6))

- **mcp-bridge:** Bump max_tokens to 300 for summary generation ([`13ac3f8`](https://github.com/emesal/chibi/commit/13ac3f8d506802a1e32c3591f00845afea453f5c))

- **mcp-bridge:** Cross-platform lockfile staleness via heartbeat ([`04bdef6`](https://github.com/emesal/chibi/commit/04bdef6f0e2eee5c651052c18eca9befe131af2b))


### Documentation

- Add MCP server integration guide (#154) ([`d5871d1`](https://github.com/emesal/chibi/commit/d5871d10eacf6c3edd542b77173204dd9b727d55))


### Features

- **mcp-bridge:** Phase 1 — daemon skeleton with TCP bridge (#154) ([`4e03967`](https://github.com/emesal/chibi/commit/4e03967e602bc441b7906c8b090be8556059c4a0))

- **mcp:** Phase 2 — chibi-core bridge client and integration (#154) ([`7bcb44b`](https://github.com/emesal/chibi/commit/7bcb44ba610ee310bdfcf00cea3bc53fb2cbb509))

- **mcp-bridge:** Phase 3 — summary cache, LLM generation, docs (#154) ([`6c32860`](https://github.com/emesal/chibi/commit/6c328604710d34afe3b43ac0339c2d2a2b5c27b1))

- **mcp-bridge:** Add `enabled` toggle for tool summary generation (#154) ([`8bc2c76`](https://github.com/emesal/chibi/commit/8bc2c76626fcdd82722d6c9cf3e96f55a070d830))

- **mcp-bridge:** Wire summary cache into ListTools responses (#154) ([`ddbe7db`](https://github.com/emesal/chibi/commit/ddbe7dbc4656783688b329e81f105aba55a4c739))


### Tests

- **mcp-bridge:** Add missing lockfile staleness tests ([`3c18580`](https://github.com/emesal/chibi/commit/3c185806c08a603137ba03465f59220eeaea0853))


## [0.8.2] - 2026-02-15

### Features

- Two-tier file read permissions — cwd auto-allow + PreFileRead prompt ([`613a9db`](https://github.com/emesal/chibi/commit/613a9db75acdf737544a0499800cee6ce79bfecc))


## [0.8.0] - 2026-02-15

### Bug Fixes

- **security:** Close retrieve_content path/URL bypass, add SSRF protection ([`494b1f3`](https://github.com/emesal/chibi/commit/494b1f3aa413a2b3d6b57afc5a3392b779cd8e4f))

- Pre-0.8.0 audit items #2–12 — dedup, correctness, dead docs ([`2caf22c`](https://github.com/emesal/chibi/commit/2caf22c5ab1863773963b4c9d962961876abc897))

- Pre-0.8.0 audit items #13–22 — dead code cleanup, clippy, cache diagnostics ([`13f54da`](https://github.com/emesal/chibi/commit/13f54daf454dc10b784ceed59f48ebd428d488b5))

- Pre-0.8.0 audit design notes — architectural cleanups ([`75a7c0c`](https://github.com/emesal/chibi/commit/75a7c0c3b21fab30b5c177a5d7b4c5d06346b83c))

- Bubble config field inspection to binaries (closes #151) ([`ad92dc8`](https://github.com/emesal/chibi/commit/ad92dc81f2f2e1e6c1ce7e82ef40dfb31539ce65))

- Pre-0.8.0 code review fixes ([`e2f778b`](https://github.com/emesal/chibi/commit/e2f778b1ebc34e0c89df20ba6cd93a9de02dbd37))

- Structured JSON error output from chibi-json (#149) ([`2c9e1e7`](https://github.com/emesal/chibi/commit/2c9e1e7bdb33851aacbae81c08f3a6e571cf4f45))


### Documentation

- Update architecture for chibi-json, clippy fix, cargo fmt ([`3a60c16`](https://github.com/emesal/chibi/commit/3a60c16acae89e2179beeb0ef26b1b1b34cd66c1))

- Pre-0.8.0 audit — fix stale references, add upgrade notes, remove dead code ([`bf82189`](https://github.com/emesal/chibi/commit/bf8218949999f34b8feeba549135ff6793f1a284))

- Pre-0.8.0 audit — fix stale references, add upgrade notes, remove dead code (#145) ([`257b1f7`](https://github.com/emesal/chibi/commit/257b1f72fa25f6156f074c59346428ed7e0c6b14))

- Design for extract shared execute_command() into chibi-core (#143) ([`80bcb15`](https://github.com/emesal/chibi/commit/80bcb15f2c6bfae4713c3c9d5f92ac14d052f7b7))

- Implementation plan for execute_command() extraction (#143) ([`7dbc6fd`](https://github.com/emesal/chibi/commit/7dbc6fd408fb822a4c442cb6df78b37b4d7a2e6b))

- Design for configurable URL security policy (#147) ([`e12c3f6`](https://github.com/emesal/chibi/commit/e12c3f65adf3f7f9a5c42b5d1736279f7ddcd8aa))

- Implementation plan for URL security policy (#147) ([`96b1287`](https://github.com/emesal/chibi/commit/96b128752a308067b02da925a78ee08a28f4ed10))


### Features

- VCS root detection and AGENTS.md loading (#125) ([`7e899f8`](https://github.com/emesal/chibi/commit/7e899f895c39689a673fd56c23211381d651ec6c))

- Tool category filtering and global tools config (#132); docs (#125, #132) ([`ed4fa53`](https://github.com/emesal/chibi/commit/ed4fa536b379c2b28da69a7600f8cd4c32c07863))

- --trust flag, Y/n permission default, coding tools docs (#128) ([`dca75b0`](https://github.com/emesal/chibi/commit/dca75b0cdbe45e595c68626a32e9f5c828b44e24))

- --trust flag, Y/n permission default, coding tools docs (#128) ([`3dc25e1`](https://github.com/emesal/chibi/commit/3dc25e1fb6dd293411141a87040194336fa387c3))

- Zero-config §1–5 — optional config fields + context_window resolution ([`eeb1195`](https://github.com/emesal/chibi/commit/eeb11954bde293c23ae514d26fb88ce9f793ea7e))

- Zero-config §6–9 — ModelMetadata cleanup, api_key visibility, docs ([`382db41`](https://github.com/emesal/chibi/commit/382db412faf38f9d6126356a309fed94b812c918))

- Extract chibi-json crate from chibi-cli ([`d1c7ebb`](https://github.com/emesal/chibi/commit/d1c7ebb9ef2c44c2611611d3dfa807d6e2f1c1c3))

- Plugin audit — add fetch_url/read_context builtins, remove 7 redundant plugins (#131) ([`8e36358`](https://github.com/emesal/chibi/commit/8e36358705296e5fbc5c0e7752397a65e8125ead))

- Support CHIBI_API_KEY and CHIBI_MODEL env var overrides (#140) ([`3d31962`](https://github.com/emesal/chibi/commit/3d31962886008e91bda9d2db3d72bff13b54cead))

- URL policy types, evaluation logic, and config wiring (#147) ([`05f595f`](https://github.com/emesal/chibi/commit/05f595fd299ef17a810f511d7f6605a403c9b52c))

- URL policy integration, canonicalization, and docs (closes #147) ([`f44bfb8`](https://github.com/emesal/chibi/commit/f44bfb89fab7f7be3cfa1b17c12659c752b29547))


### Refactoring

- Route all chibi-cli output through OutputHandler (#14) ([`7df3ecd`](https://github.com/emesal/chibi/commit/7df3ecdc6c91480770f831c7e16d55c7c4fb6a70))

- Remove plugins submodule ([`9f3bc1d`](https://github.com/emesal/chibi/commit/9f3bc1db00ede8f7ea465c87d35fbff941ec09a8))

- Simplify config resolution with macros (#142) ([`0e29bd4`](https://github.com/emesal/chibi/commit/0e29bd4f771471abb1b47f959fb2604394476078))

- Extract shared execute_command() into chibi-core (#143) ([`f988ba7`](https://github.com/emesal/chibi/commit/f988ba707fd287d829ef47e2c141188d258915c2))


## [0.7.0] - 2026-02-12

### Bug Fixes

- Tolerate BrokenPipe when writing hook stdin ([`640e834`](https://github.com/emesal/chibi/commit/640e834e5f07b8b3617f7667886ab6280f616f10))

- Handle ratatoskr ToolCallEnd event in stream processing ([`9308e22`](https://github.com/emesal/chibi/commit/9308e22c5a5baf008944e38b42da1e50e1d87771))

- Display tool calls on their own line ([`2cf5a74`](https://github.com/emesal/chibi/commit/2cf5a74dcdfc48835b29a182a524caebde3320d8))

- Reflection_enabled now reads resolved config, not global ([`803cea6`](https://github.com/emesal/chibi/commit/803cea6efd25087a00542d731582961b8b2d0d53))

- Forward reasoning content to sink in json mode ([`91317bf`](https://github.com/emesal/chibi/commit/91317bff1b05af0c218343fe312d8b50970a3dde))

- Preserve tool history across context reload ([`bd9797d`](https://github.com/emesal/chibi/commit/bd9797d54f7ebc8eb77fd0ee4d04ad96edfc8759))

- Wire remaining unwired config fields and hooks through the stack ([`2175232`](https://github.com/emesal/chibi/commit/2175232559f46e62e0e3df94ecc17583f0787fa0))

- Wire StorageConfig through ResolvedConfig, remove bespoke resolution path ([`49f4bc8`](https://github.com/emesal/chibi/commit/49f4bc8cabdf6d45d013cd488ded4b1595340661))


### Chores

- Update dependencies post-release ([`d312f87`](https://github.com/emesal/chibi/commit/d312f876e32d454bb7ec9e828defc0a90367b781))


### Documentation

- Add model metadata CLI flag design ([`58541c1`](https://github.com/emesal/chibi/commit/58541c15f6a1b99221e7bb36271f6a88f939b0bd))

- Update model metadata plan to reflect implementation ([`f0c95fb`](https://github.com/emesal/chibi/commit/f0c95fb9469150bfa0cd28e44188f97ba2141323))

- Update for recent features on this branch ([`a30cdd8`](https://github.com/emesal/chibi/commit/a30cdd89aa4e60a36e1ab99aca2a64f45734b22f))


### Features

- Add -m/-M flags for model metadata lookup (#87, #88) ([`637a742`](https://github.com/emesal/chibi/commit/637a742ca210a0480ddcc8ac16a8199e283f63d8))

- Add spawn_agent and retrieve_content built-in tools (#112) ([`5596cf6`](https://github.com/emesal/chibi/commit/5596cf62f1a0b647573d2cf5c6e2ee8e75e18887))

- Add spawn_agent and retrieve_content built-in tools (#112) (#116) ([`dbfa130`](https://github.com/emesal/chibi/commit/dbfa13053952017dd6d6e6e4ce11caae56dc988a))

- Parallel tool execution (#118) ([`96d9083`](https://github.com/emesal/chibi/commit/96d90836ddb97ef0803eed724c5bf50733c45ca0))

- Parallel tool execution (#118) (#119) ([`3974f44`](https://github.com/emesal/chibi/commit/3974f44848e62a880cd9c5a2eb326b02143f9de4))

- Show built-in tools as separate readout in verbose mode ([`cf442f4`](https://github.com/emesal/chibi/commit/cf442f4887fe17270c37be118670f3fafc5914d2))

- Add verbose setting to config.toml/local.toml ([`f5a58a4`](https://github.com/emesal/chibi/commit/f5a58a4c9fc6e6d6f18e25f462e556b2eb3a8749))

- Show tool calls by default, add --hide-tool-calls option ([`bf75879`](https://github.com/emesal/chibi/commit/bf75879cf2c7093a93d524aa0ed16eedeabda40a))

- Add --no-tool-calls flag for pure text mode (#120) ([`af8d16f`](https://github.com/emesal/chibi/commit/af8d16feba4e4e5048ca1d668ed1b5115e5c2d14))

- Auto-disable tool calls for models that don't support them (#121) ([`cbf03f8`](https://github.com/emesal/chibi/commit/cbf03f8d121d893a33c56beabae3b896600eb718))

- Add model_info built-in tool (#114) ([`e1c3768`](https://github.com/emesal/chibi/commit/e1c3768f459356768ed93e80a59d44982d08f753))

- Add coding agent tools and codebase index (phases 1–4) ([`868a924`](https://github.com/emesal/chibi/commit/868a924965d88d86cf322d32eb26504c45d5f0cb))

- Add index tools, PostIndexFile hook, deprecate patch_file (phases 5–7) ([`19192dc`](https://github.com/emesal/chibi/commit/19192dcd6b47dc138a13050d5565fb8b8b4fb10f))

- Word-wrap markdown output at word boundaries ([`b741734`](https://github.com/emesal/chibi/commit/b74173461ccdc080f308207a8d9511b855e4e386))

- Wire reasoning/thinking content through the full stack ([`cde1ef9`](https://github.com/emesal/chibi/commit/cde1ef985bfd115aa92484064778659837b10b51))

- Add show_thinking config option and --show-thinking flag ([`1186384`](https://github.com/emesal/chibi/commit/1186384d02eefbde152bce2aea901ac0c1e15b1b))

- Wire reasoning/thinking content through the full stack (#134) ([`8e9bc6b`](https://github.com/emesal/chibi/commit/8e9bc6b55fea9964153ad60015b58412f725461e))

- Replace recursion depth with fuel budget for agentic loop ([`68415b2`](https://github.com/emesal/chibi/commit/68415b2bb8a847c42946e50383cf8d2db34a093a))


## [0.6.0] - 2026-02-03

### Bug Fixes

- Change default fallback tool from call_agent to call_user ([`1f863a7`](https://github.com/emesal/chibi/commit/1f863a7048b9d91bb390247ffc378c98261c3d93))


### Chores

- Update dependencies post-release ([`14a7d66`](https://github.com/emesal/chibi/commit/14a7d66ca07e9b54492313c2447d58882ad8a79e))

- Update plugins submodule (file-permission) ([`e025318`](https://github.com/emesal/chibi/commit/e025318263ea9bc90d578c4362edcf15fa8b6b59))

- Update dependencies post-release ([`b0942f5`](https://github.com/emesal/chibi/commit/b0942f563bcd4f4f7125c36ba5d7935d02fcb51f))

- Add ratatoskr as dependency ([`7f11ded`](https://github.com/emesal/chibi/commit/7f11ded711106237be48efb658391f9868bc1633))

- Clean up after ratatoskr integration ([`7fd3cfe`](https://github.com/emesal/chibi/commit/7fd3cfe7b3cfda8aca034a3d36349c2b5d30c0ec))


### Documentation

- Update for ratatoskr integration ([`e4d8666`](https://github.com/emesal/chibi/commit/e4d8666ee88255d2c34b01f8c60c09f334597c36))


### Features

- Context name, datetime prefix, improved call_agent ([`435f8ab`](https://github.com/emesal/chibi/commit/435f8ab91275eb866baa74890e014ca36813cf4e))

- Add write_file/patch_file tools and pre_file_write hook ([`107e904`](https://github.com/emesal/chibi/commit/107e90429568b5c13fb2cece5acc9c58f34ed5f4))

- Implement write_file/patch_file execution with permission gating ([`a41273f`](https://github.com/emesal/chibi/commit/a41273f89ca1e7c05451b10633cd2921a3180fd4))

- Add gateway module with type conversions ([`68fac4a`](https://github.com/emesal/chibi/commit/68fac4aa79caa02b2b6c1f272599a27f3827f376))

- Replace SSE parsing with ratatoskr chat_stream ([`dd93075`](https://github.com/emesal/chibi/commit/dd930756320e3c92235912c387415cd2c7874f87))


### Refactoring

- Rename transient->ephemeral, current_context->implied_context ([`dd32f7e`](https://github.com/emesal/chibi/commit/dd32f7edbcbbed07b4fb424e61450101a3634f63))

- Migrate compact.rs to ratatoskr, remove dead code ([`47e69b6`](https://github.com/emesal/chibi/commit/47e69b6970f861b63bd24f7ddcdda8ea8c11b367))


## [0.5.0] - 2026-01-24

### Bug Fixes

- Contexts don't pick up on new system prompts ([`5f85db3`](https://github.com/emesal/chibi/commit/5f85db3a1cc56319f57bdb461e4add1bbcc6466b))


## [0.2.0] - 2026-01-19

