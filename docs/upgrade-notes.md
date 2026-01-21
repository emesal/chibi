# chibi upgrade notes

## 0.4.0 -> 0.4.1 (upcoming)
- new context state format -> clear (0.3) or archive (0.4) contexts before upgrading to preserve history (see --help)
- human-readable transcripts are now md files. if wanted, old transcripts can be migrated with

```bash
find $HOME/.chibi/contexts -type f -name "transcript.txt" -exec sh -c 'mv -i "$1" "${1%.txt}.md"' _ {} \;
```

## 0.3.0 -> 0.4.0
- CLI parameters changed! Existing scripts need to be updated.
