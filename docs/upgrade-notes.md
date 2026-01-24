# chibi upgrade notes

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
