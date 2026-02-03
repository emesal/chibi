# Development workflow automation

# Show workflow cheat sheet
default:
  @echo "=== Development Workflow ==="
  @echo ""
  @echo "📝 Starting work:"
  @echo "  just feature <name>       → Start new feature branch (from dev)"
  @echo "  just bugfix <name>        → Start new bugfix branch (from dev)"
  @echo "  just refactor <name>      → Start refactor branch (from dev)"
  @echo "  just chore <name>         → Start maintenance branch (from dev)"
  @echo "  just docs <name>          → Start docs-only branch (from dev)"
  @echo "  just hotfix <name>        → Start urgent fix branch (from main!)"
  @echo ""
  @echo "🔍 Before pushing:"
  @echo "  just pre-push             → Run fmt, clippy, tests"
  @echo "  just lint                 → Just format + clippy"
  @echo "  just test                 → Just run tests"
  @echo ""
  @echo "🔀 Finishing work:"
  @echo "  just merge-to-dev         → Merge current branch to dev + tag it (local)"
  @echo "  just pr                   → Create PR to dev (auto-tags on merge)"
  @echo ""
  @echo "🚀 Release cycle:"
  @echo "  just freeze <reason>      → Lock tree for release (bugfixes only)"
  @echo "  just thaw                 → Unlock tree for features again"
  @echo "  just release v0.x         → Squash dev→main, run tests, tag release"
  @echo "  just push-release v0.x    → Push release to github"
  @echo "  just update-deps          → Update dependencies post-release"
  @echo ""
  @echo "🔎 Archaeology:"
  @echo "  just list-features        → Show all feature/bugfix tags"
  @echo "  just show-feature <name>  → What did this feature change?"
  @echo "  just blame <file>         → Per-commit attribution"
  @echo "  just history <file>       → Full change history"
  @echo ""
  @echo "📚 Documentation:"
  @echo "  just rustdoc              → Build & open docs locally"
  @echo ""
  @echo "🛠️  Utility:"
  @echo "  just clean                → Remove build artifacts"
  @echo "  just rebuild              → Clean + rebuild release"
  @echo ""
  @echo "Run 'just --list' to see all available commands"

# List all available commands
list:
  @just --list

# === Tree Freeze (Release Discipline) ===

# Lock the tree for release preparation (bugfixes only)
freeze reason:
  #!/usr/bin/env bash
  set -e
  if [ -f .freeze ]; then
    echo "⚠️  Tree is already frozen:"
    cat .freeze
    exit 1
  fi
  echo "{{reason}}" > .freeze
  echo "frozen: $(date -Iseconds)" >> .freeze
  git add .freeze
  echo "🧊 Tree frozen: {{reason}}"
  echo "   Use 'just push-bugfix' for bugfix commits"
  echo "   Use 'just thaw' when ready to unfreeze"

# Unlock the tree for normal development
thaw:
  #!/usr/bin/env bash
  set -e
  if [ ! -f .freeze ]; then
    echo "Tree isn't frozen, nothing to thaw~"
    exit 0
  fi
  rm -f .freeze
  git add .freeze 2>/dev/null || true
  echo "🌸 Tree unfrozen, features welcome again~"

# Check if tree is frozen (used by pre-push)
check-freeze:
  #!/usr/bin/env bash
  if [ -f .freeze ]; then
    echo ""
    echo "⚠️  TREE IS FROZEN ⚠️"
    echo "$(head -1 .freeze)"
    echo ""
    echo "Options:"
    echo "  • If this is a bugfix: just push-bugfix"
    echo "  • To unfreeze:         just thaw"
    echo ""
    exit 1
  fi

# Push during freeze (confirms this is a bugfix)
push-bugfix: _frozen-guard lint test
  #!/usr/bin/env bash
  set -e
  BRANCH=$(git branch --show-current)
  echo "🐛 Pushing bugfix on $BRANCH during freeze..."
  git push

# Internal: ensure tree IS frozen (for push-bugfix)
_frozen-guard:
  #!/usr/bin/env bash
  if [ ! -f .freeze ]; then
    echo "Tree isn't frozen — just use regular 'git push' or 'just pre-push'"
    exit 1
  fi

# === Development Commands ===

# Start a new feature branch from dev
feature name: check-freeze
  #!/usr/bin/env bash
  set -e
  git checkout dev
  git pull
  git checkout -b "feature/{{name}}"
  echo "✓ Created and switched to feature/{{name}}"

# Start a new bugfix branch from dev
bugfix name:
  #!/usr/bin/env bash
  set -e
  git checkout dev
  git pull
  git checkout -b "bugfix/{{name}}"
  echo "✓ Created and switched to bugfix/{{name}}"

# Start a new refactor branch from dev
refactor name: check-freeze
  #!/usr/bin/env bash
  set -e
  git checkout dev
  git pull
  git checkout -b "refactor/{{name}}"
  echo "✓ Created and switched to refactor/{{name}}"

# Start a new chore branch from dev
chore name: check-freeze
  #!/usr/bin/env bash
  set -e
  git checkout dev
  git pull
  git checkout -b "chore/{{name}}"
  echo "✓ Created and switched to chore/{{name}}"

# Start a new docs branch from dev
docs name:
  #!/usr/bin/env bash
  set -e
  git checkout dev
  git pull
  git checkout -b "docs/{{name}}"
  echo "✓ Created and switched to docs/{{name}}"

# Start a new hotfix branch from main (for urgent production fixes)
hotfix name:
  #!/usr/bin/env bash
  set -e
  git checkout main
  git pull
  git checkout -b "hotfix/{{name}}"
  echo "✓ Created and switched to hotfix/{{name}}"
  echo "⚠️  This branch is from main, not dev!"
  echo "  After merging to main, sync back to dev with: git checkout dev && git merge main"

# Merge current branch into dev and tag it (local merge, for quick changes)
merge-to-dev: _merge-freeze-check
  #!/usr/bin/env bash
  set -e
  BRANCH=$(git branch --show-current)
  if [ "$BRANCH" = "dev" ] || [ "$BRANCH" = "main" ]; then
    echo "Error: Cannot merge dev or main into itself"
    exit 1
  fi
  git checkout dev
  git pull
  git merge --no-ff "$BRANCH" -m "Merge branch '$BRANCH' into dev"
  git tag "$BRANCH"
  echo "✓ Merged $BRANCH into dev and tagged as $BRANCH"
  echo "  Branch $BRANCH is now preserved as a tag"
  echo "  You can delete the branch with: git branch -d $BRANCH"

# Internal: check freeze status for merge (allows bugfix/* and docs/*)
_merge-freeze-check:
  #!/usr/bin/env bash
  if [ -f .freeze ]; then
    BRANCH=$(git branch --show-current)
    if [[ "$BRANCH" == bugfix/* ]] || [[ "$BRANCH" == docs/* ]] || [[ "$BRANCH" == hotfix/* ]]; then
      echo "🐛 Merging $BRANCH during freeze (allowed)"
    else
      echo ""
      echo "⚠️  TREE IS FROZEN ⚠️"
      echo "$(head -1 .freeze)"
      echo ""
      echo "Only bugfix/*, docs/*, and hotfix/* branches can be merged during freeze."
      echo "To unfreeze: just thaw"
      echo ""
      exit 1
    fi
  fi

# Create a pull request to dev (auto-tags on merge via GitHub Actions)
pr: _merge-freeze-check
  #!/usr/bin/env bash
  set -e
  BRANCH=$(git branch --show-current)
  if [ "$BRANCH" = "dev" ] || [ "$BRANCH" = "main" ]; then
    echo "Error: Cannot create PR from dev or main"
    exit 1
  fi

  # Push branch if not already pushed
  git push -u origin "$BRANCH" 2>/dev/null || git push origin "$BRANCH"

  # Create PR
  gh pr create --base dev --fill

  echo "✓ PR created to dev"
  echo "  When merged, GitHub Actions will automatically tag it as $BRANCH"

# === Linting & Testing ===

# Run formatting and clippy checks
lint:
  cargo fmt
  cargo clippy --all-targets --locked -- -D warnings

# Check formatting and clippy without modifying files
check:
  cargo fmt --check
  cargo clippy --all-targets --locked -- -D warnings

# Run all tests
test:
  cargo nextest run --all-targets --locked

# Full pre-push check (format, clippy, tests)
pre-push: check-freeze lint test
  @echo "✓ All checks passed, ready to push"

# Push with freeze check and all validations
push: pre-push
  git push

# Install to ~/.cargo/bin
install:
  cargo install --path crates/chibi-cli

# === Documentation ===

# Build and open documentation locally
rustdoc:
  cargo doc --no-deps --open

# Build documentation for all dependencies (slower)
rustdoc-all:
  cargo doc --open

# === Release Cycle Commands ===

# Lock the tree for release (squash-merge dev to main, tag release)
release version:
  #!/usr/bin/env bash
  set -e
  echo "Preparing release v{{version}}..."

  # Ensure we're on dev and up to date
  git checkout dev
  git pull

  # Run full checks before release
  echo "Running pre-release checks..."
  cargo fmt --check || (echo "❌ Code needs formatting. Run 'just lint' first." && exit 1)
  cargo clippy --all-targets --locked -- -D warnings || (echo "❌ Clippy errors found." && exit 1)
  cargo nextest run --all-targets --locked || (echo "❌ Tests failed." && exit 1)

  # Squash merge dev into main
  git checkout main
  git pull
  git merge --squash dev

  # Commit with release message
  git commit -m "Release v{{version}}"

  # Tag the release
  git tag -a "v{{version}}" -m "Release v{{version}}"

  echo "✓ Release v{{version}} prepared on main"
  echo "  Review with: git show HEAD"
  echo "  Push with: just push-release v{{version}}"

# Push a release to remote (after review)
push-release version:
  #!/usr/bin/env bash
  set -e
  git checkout main
  git push origin main
  git push origin "v{{version}}"

  # Sync dev forward
  git checkout dev
  git merge main
  git push origin dev

  echo "✓ Released v{{version}} and synced dev"

# Update dependencies after release
update-deps:
  #!/usr/bin/env bash
  set -e
  git checkout dev
  git pull
  echo "Updating dependencies..."
  cargo update
  cargo build
  cargo test
  git add Cargo.lock
  git commit -m "chore: update dependencies post-release"
  git push origin dev
  echo "✓ Dependencies updated on dev"

# === Archaeology Commands ===

# Show what a specific feature changed
show-feature name:
  #!/usr/bin/env bash
  set -e
  git checkout dev
  if ! git rev-parse "feature/{{name}}" >/dev/null 2>&1; then
    echo "Error: Tag feature/{{name}} not found"
    exit 1
  fi
  echo "=== Feature: feature/{{name}} ==="
  git show "feature/{{name}}"
  echo ""
  echo "=== All commits in this feature ==="
  git log "feature/{{name}}^..feature/{{name}}" --oneline

# Show what changed in a file, with per-commit attribution
blame file:
  git checkout dev
  git blame "{{file}}"

# Show full history of changes to a file
history file:
  git checkout dev
  git log -p "{{file}}"

# List all feature tags
list-features:
  @echo "=== Feature tags ==="
  @git tag -l 'feature/*'
  @echo ""
  @echo "=== Bugfix tags ==="
  @git tag -l 'bugfix/*'

# === Utility Commands ===

# Clean build artifacts
clean:
  cargo clean

# Update git submodules
update-submodules:
  git submodule update --init --recursive

# Full clean rebuild
rebuild: clean
  git submodule update --init --recursive
  cargo build --release
