# chibi upgrade notes

this file only documents breaking changes that require user action.

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
