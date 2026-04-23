---
name: forge-debug
description: Use when encountering any bug, test failure, or unexpected behavior — enforces root-cause investigation before fixes. Integrates with Forge memory to check for known issues and store findings.
---

# Forge Debug — Systematic Debugging with Memory

Find root cause BEFORE attempting fixes. Symptom fixes are failure.

**Iron Law:** NO FIXES WITHOUT ROOT CAUSE INVESTIGATION FIRST.

**Forge advantage:** Check if this bug was seen before via recall. Use find-symbol and symbols to navigate code precisely. Blast-radius shows what else might be affected. Store the root cause as a lesson for future sessions.

## The Protocol

### Phase 1: OBSERVE (do NOT fix yet)

**Goal:** Gather all evidence before forming hypotheses.

```bash
# Check if this issue was seen before
forge-next recall "<error message or symptom keywords>" --type lesson --limit 5

# Understand what depends on the broken code
forge-next blast-radius --file <broken-file>

# Check project conventions for test/lint commands
forge-next compile-context --agent claude-code | grep -A 2 "test_command"
```

**Steps:**
1. **Reproduce the bug** — get the exact error message, stack trace, or unexpected output
2. **Note what changed** — `git log --oneline -10`, `git diff`
3. **Check Forge memory** — has this been seen before? Is there a known lesson?
4. **Read the error carefully** — the error message often tells you exactly what's wrong

**Evidence to collect:**
- Exact error message / stack trace
- Which test fails (if any)
- When it started happening (which commit?)
- What's different between working and broken state

### Phase 2: HYPOTHESIZE

Form 2-3 hypotheses about the root cause. For each:
- What evidence would **confirm** it?
- What evidence would **rule it out**?

**Do NOT start fixing based on a hypothesis. Test it first.**

**Common root cause categories:**

| Category | Signs | Investigation |
|----------|-------|---------------|
| **Wrong input** | Unexpected values in error | Add logging/assertions at entry point |
| **State corruption** | Works first time, fails on repeat | Check initialization, cleanup, shared state |
| **Race condition** | Intermittent, timing-dependent | Add sleep to widen window, check locks |
| **Missing dependency** | Module not found, undefined | Check imports, build config, PATH |
| **Version mismatch** | Works locally, fails in CI | Check dependency versions, pinning |
| **Data format** | Parse error, unexpected type | Compare actual vs expected format |
| **Config error** | Works with defaults, fails with custom | Diff config against working state |

### Phase 3: ISOLATE

Narrow down to the exact cause:

**Technique 1: Binary search (which commit?)**
```bash
git bisect start
git bisect bad HEAD
git bisect good <last-known-good-commit>
# Git walks you through the commits
```

**Technique 2: Minimal reproduction**
Strip away everything except the failing path. Create the smallest test that fails.

**Technique 3: Symbol tracing**
```bash
# Find the function definition
forge-next find-symbol <function_name>

# See what calls it
forge-next blast-radius --file <file_path>

# Get overview of all symbols in the file
forge-next symbols --file <file_path>
```

**Technique 4: Print debugging (tactical)**
Add temporary logging at key points. For Rust:
```rust
eprintln!("[debug] value = {:?}", value);
```
Remove ALL debug prints before committing.

**Technique 5: Diff against working state**
```bash
git stash       # set aside your changes
# test if bug exists without your changes
git stash pop   # restore
```

**Key technique:** Write a test that reproduces the bug FIRST (Red in TDD). If you can't reproduce it in a test, you don't understand it yet.

### Phase 4: FIX (now you may change code)

Only after you can explain:
1. **What** is broken (specific function/line)
2. **Why** it's broken (root cause, not symptom)
3. **How** the fix addresses the root cause (not just the symptom)

Fix using TDD:
1. Write (or update) the test that reproduces the bug
2. Verify test fails (with the exact error you observed)
3. Apply the minimal fix
4. Verify test passes
5. Run full test suite (from project conventions)
6. Run lint

**Fix anti-patterns:**
- "Add a try-catch around it" — symptom fix, root cause still exists
- "Check for null before accessing" — why is it null? Fix the source
- "Increase the timeout" — why is it slow? Fix the bottleneck
- "Disable the check" — the check exists for a reason

### Phase 5: VERIFY

After fixing, verify thoroughly:

```bash
# Run the specific failing test
<test_command> -- <test_name>

# Run the full suite
<test_command>

# Run lint
<lint_command>

# Check blast radius — did the fix break anything else?
forge-next blast-radius --file <fixed-file>
```

### Phase 6: REMEMBER

```bash
forge-next remember --type lesson --title "<what was broken>" --content "<root cause + fix>"
```

Store the root cause so future sessions don't re-investigate the same issue.

**Good lesson format:**
```
Root cause: [precise cause]
Symptom: [what was observed]
Fix: [what was changed]
Prevention: [how to avoid in future]
```

## Common Root Causes by Domain

### Database Issues
- **Silent write failure**: Read-only connection used for writes (returns Ok but no rows affected)
- **FK constraint blocking inserts**: Foreign key points to non-existent row
- **Schema mismatch**: Column added in code but not in migration/production DB
- **Race condition in WAL mode**: Concurrent writers without proper locking

### API/Network Issues
- **Timeout masking errors**: 30s timeout returns generic error instead of real cause
- **Rate limiting**: Server returns 429 but code doesn't handle it
- **Serialization mismatch**: Server sends snake_case, client expects camelCase
- **Missing headers**: Auth token, Content-Type, Accept not set

### Async/Concurrency Issues
- **Dropped futures**: `tokio::spawn` without `.await` on the handle — task may not finish
- **Lock ordering**: Mutex A then B vs B then A → deadlock
- **Shared mutable state**: Multiple tasks modifying same HashMap without synchronization
- **Channel closed**: Receiver dropped while sender still active

### File System Issues
- **Path format mismatch**: Absolute vs relative, with/without trailing slash
- **Symlink traversal**: Following symlinks outside expected directory
- **Permission denied**: File created with wrong umask
- **File not found**: Path works locally but uses different separator on other OS

## Debugging Decision Tree

```
Error occurs
  ├── Error message clear? → Read it. It often says exactly what's wrong.
  ├── Reproduces reliably?
  │     ├── Yes → Write a failing test, then investigate
  │     └── No → Race condition or state-dependent — add logging, widen the window
  ├── Worked before?
  │     ├── Yes → git bisect to find the breaking commit
  │     └── No → Never worked — check assumptions, read the spec
  ├── Works in tests, fails in production?
  │     └── Environment difference — check: DB mode, permissions, concurrency, config
  └── Error in dependency?
        ├── Yes → Check version pinning, read changelog
        └── No → It's your code. Trace the call path.
```

## Anti-Patterns

| Don't | Do |
|-------|-----|
| "Let me just try this quick fix" | Investigate root cause first |
| Fix the symptom | Fix the cause |
| Skip reproduction | Write a failing test first |
| Forget to store the lesson | `forge-next remember --type lesson` |
| Fix in tests only | Verify in production too |
| Add error suppression | Understand why the error occurs |
| Debug by reading code | Debug by running code and observing |
| Assume you know the cause | Gather evidence, test hypotheses |
