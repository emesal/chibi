# Development Workflow

## Branch Strategy

- **`main`**: Clean release history (squash commits only)
- **`dev`**: Detailed development history (all commits preserved)
- **Feature branches**: `feature/`, `bugfix/`, `refactor/`, `chore/`, `docs/`, `hotfix/`

## Two Ways to Merge

### Option 1: Local Merge (Quick Changes)

For small, straightforward changes:

```bash
just pre-push           # Run tests
just merge-to-dev       # Merge locally + tag
git push origin dev     # Push to remote
```

**Result:** Immediate merge to dev, tagged locally.

### Option 2: Pull Request (Review Workflow)

For larger changes or when review is needed:

```bash
just pre-push           # Run tests
just pr                 # Create PR to dev
```

**Result:** PR created on GitHub. When merged, GitHub Actions automatically tags the feature branch.

## Auto-Tagging

The `.github/workflows/auto-tag-features.yml` workflow automatically creates tags when PRs are merged to `dev`:

- Detects branch name (e.g., `feature/auth-system`)
- Creates tag with that name on the merge commit
- Pushes tag to remote

This preserves the branch for archaeology (`just show-feature auth-system` will work).

## Tree Freeze

During release preparation, the tree can be locked to bugfixes, docs, and hotfixes only:

```bash
just freeze "preparing 0.x.y"  # Lock tree
just thaw                       # Unlock tree
```

While frozen, only `bugfix/*`, `docs/*`, and `hotfix/*` branches can be merged.

## Release Cycle

```bash
just release 0.x.y      # Squash dev→main, run tests, tag release
just push-release 0.x.y # Push to GitHub
just update-deps        # Update dependencies post-release
```

## Archaeology

Use `just` commands on the `dev` branch for detailed history:

```bash
just blame <file>         # Per-commit attribution
just show-feature <name>  # What did this feature change?
just history <file>       # Full change history
just list-features        # Show all feature and bugfix tags
```

## GitHub Pages

Documentation is automatically published to GitHub Pages on every push to `main`:
- Built via `.github/workflows/docs.yml`
- Published to: https://emesal.github.io/chibi/
