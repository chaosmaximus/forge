---
name: forge-ship
description: "Use when forge-review has passed and work is ready to integrate or create a PR. Handles final verification, changelog generation, PR creation, memory persistence, and cleanup. Example triggers: 'ship this', 'create a PR', 'merge this', 'we are done, let us ship', 'push and create pull request'."
---

# Forge Ship

Final gate before code reaches production. Verification → Changelog → PR → Memory → Cleanup.

## Step 1: Final Verification

Run the project's verification + test commands from the
`<project-conventions>` block in your context (typical examples):
- Rust: `cargo fmt --all -- --check && cargo clippy --workspace -- -W clippy::all -D warnings && cargo test --workspace`
- Node: `npm run lint && npm test`
- Python: `ruff check . && pytest`

All checks must pass. Then:

3. Check for uncommitted changes: `git status --short`
4. Check for NEEDS_CLARIFICATION markers: `grep -r "NEEDS_CLARIFICATION" .`

If any verification fails: DO NOT proceed. Report the failure and return to forge-review.

## Step 2: Changelog

Generate a changelog entry from conventional commits:

```bash
# If git-cliff is available:
git-cliff --unreleased --output CHANGELOG.md

# If not, generate manually:
git log --format="- %s (%h)" $(git describe --tags --abbrev=0 2>/dev/null || echo HEAD~20)..HEAD
```

Present the changelog to the user for review before continuing.

## Step 3: Version Management (if releasing)

If the user wants a version bump:
1. Detect version files: `Cargo.toml`, `package.json`, `pyproject.toml`, `plugin.json`
2. Present current version and ask: patch (0.3.1), minor (0.4.0), or major (1.0.0)?
3. Update version in all detected files
4. Commit: `git commit -m "chore: bump version to X.Y.Z"`

## Step 4: Integration Options

Present exactly these options:
1. **Push and create Pull Request** (recommended)
2. Merge locally to base branch
3. Keep branch as-is (user handles later)
4. Create GitHub Release (tag + binaries)

If option 1: generate PR with:
- Title from the plan/PRD (under 70 chars)
- Body: Summary, Test plan, Forge review scores, Codex verdict

If option 4: create release with:
```bash
git tag -a v$VERSION -m "Release v$VERSION"
git push origin v$VERSION
gh release create v$VERSION --title "v$VERSION" --notes-file CHANGELOG.md
```

## Step 5: Store in Memory

```bash
forge-next remember --type decision --title "Shipped: [feature name]" \
  --content "[Summary of what was shipped, key decisions, review scores]"
```

Also update STATE.md with completion status.

## Step 6: Cleanup

1. Clean up agent team (shut down teammates)
2. Remove worktree branches that were merged
3. Announce: "Forge complete. [Feature] shipped. Run /forge for the next task."

## Step 7: Dependency Audit (optional)

If shipping to production, run:
```bash
# Python
pip-audit --format json 2>/dev/null

# Node
npm audit --json 2>/dev/null

# Rust
cargo audit --json 2>/dev/null
```

Report any vulnerabilities before the final ship.
