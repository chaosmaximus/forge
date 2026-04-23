---
name: forge-migrate
description: "Use when adopting existing code into a new workspace, migrating prototypes, or integrating code from another project. Skips discovery/PRD — goes straight to analysis, copy, and integration testing. Example triggers: 'migrate this code', 'adopt this prototype', 'bring this into the workspace', 'copy and integrate'."
---

# Forge Migrate — Adopt Existing Code

For code that already exists and needs to be brought into a new workspace. Skips discovery and PRD — the code IS the spec.

## When to Use

- Moving a prototype into a production workspace
- Copying code from one project into another
- Adopting an external library/service into your monorepo
- Re-platforming (same logic, different structure/framework)

## The Process

### Phase 1: Analyze Source

```bash
forge-next recall "<source project keywords>" --limit 5
```

Understand what you're migrating:
1. **Read the entry point** — find main.py, main.rs, index.ts, etc.
2. **Map dependencies** — what does it import? External packages? Internal modules?
3. **Count and categorize** — files, lines, tests, configs
4. **Check conventions** — does it have tests? CI? Linting?

Present to user: "Source has N files, M lines, K tests. Dependencies: [list]. Ready to migrate?"

### Phase 2: Copy & Structure

1. Copy source files into the target workspace
2. Adapt structure to match the target's conventions (check `<project-conventions>` in context)
3. Rename/restructure packages if needed to avoid namespace collisions
   - **CRITICAL**: Check for naming conflicts before copying (e.g., `core` is always taken)
4. Update import paths for the new location

### Phase 3: Integration Test

This is the most important phase. Don't skip it.

1. **Install dependencies**: `pip install -e .`, `cargo build`, `npm install`
2. **Run the smoke test**: import the main module or build the full project
3. **Run existing tests**: use the source's test suite, adapted for the new location
4. **Fix what breaks**: namespace collisions, import paths, missing configs

If tests existed in the source, they must pass in the target. If they don't exist, write at least a smoke test.

### Phase 4: Remember

```bash
forge-next remember --type decision --title "Migrated <source> into <target>" \
  --content "Copied N files. Tests: M/M passing. Changes: [list of adaptations]"
```

## Anti-Patterns

| Don't | Do |
|-------|-----|
| Rewrite the code during migration | Copy first, verify it works, THEN improve |
| Skip integration testing | Run the full build/import chain |
| Use generic package names | Check for namespace collisions first |
| Assume PYTHONPATH hacks work | Use proper package installation (pip install -e .) |
