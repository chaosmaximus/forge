---
name: forge-tdd
description: Use when implementing any feature or bugfix — enforces test-first discipline with Forge memory integration. Write the test, watch it fail, implement, verify, store decision.
---

# Forge TDD — Test-Driven Development with Memory

Write the test first. Watch it fail. Write minimal code to pass. Store the decision.

**Iron Law:** NO PRODUCTION CODE WITHOUT A FAILING TEST FIRST.

**Forge advantage:** Every TDD cycle checks blast-radius before editing, reads project conventions for the test command, and stores decisions after completing.

## When to Use

**Always:** New features, bug fixes, refactoring, behavior changes.
**Only exception:** User explicitly says "skip TDD."

## Setup — Read Project Conventions

Before writing any test, check what testing tools and patterns this project uses:

```bash
forge-next compile-context --agent claude-code | grep -A 2 "test_command\|test_patterns\|lint_command"
```

This tells you: what command runs tests, what test attribute patterns to use, and how to lint. Never assume — check the context.

## The Cycle

### 1. RECALL — Check what exists

```bash
forge-next recall "<feature keywords>" --limit 5
forge-next blast-radius --file <target-file>
```

Understand what you're changing and what depends on it BEFORE writing anything.

**What to look for:**
- Prior decisions about this area (may constrain your approach)
- Existing tests for the module (extend, don't duplicate)
- Callers that depend on the file you're changing (may break)

### 2. RED — Write the failing test

Write ONE minimal test showing what the feature should do.

**Rules:**
- Test describes the **behavior**, not the implementation
- One assertion per test (or tightly related group)
- Name says what it tests: `test_recall_score_discrimination` not `test_recall`
- Test file/location follows existing project patterns

**Run the test. Verify it FAILS with the expected error.**

Use the test command from project conventions. If none, auto-detect:
```bash
# Check for project-specific test command in context
# Fallback detection: Cargo.toml → cargo test, package.json → npm test, etc.
```

If it doesn't fail: your test doesn't test what you think. Fix the test first.

**Red flag patterns (test doesn't actually test anything):**
- Assertion on a constant: `assert_eq!(1, 1)` — tests nothing
- Mock returns what you assert: `mock.returns(5); assert_eq!(result, 5)` — tests the mock
- No assertion at all — test passes by not crashing

### 3. GREEN — Write minimal code to pass

Write the MINIMUM code to make the test pass. No more.

**Rules:**
- Don't add "while I'm here" improvements
- Don't add error handling for cases without tests
- Don't refactor yet
- If you wrote code before the test: delete it, start over

**Run the test. Verify it PASSES.**

**If the test is hard to make pass**, your design may be wrong. Consider:
- Is the function doing too much? Split it.
- Does it depend on too many things? Inject dependencies.
- Is the test testing the wrong level? (Unit test a service call → test the logic, not the HTTP layer)

### 4. REFACTOR — Clean up (stay green)

Now improve the code. Run tests after EVERY change.

**Rules:**
- Tests must stay green throughout
- Extract helpers, rename, simplify
- Run lint (from project conventions) — zero warnings
- Don't change behavior during refactoring

**What to refactor:**
- Duplicated logic → extract a function
- Magic numbers → named constants
- Complex conditionals → early returns or match
- Long function → split into focused pieces

**What NOT to refactor:**
- Code in other files (scope creep)
- Working code that isn't part of your change
- Performance optimization without evidence of a problem

### 5. REMEMBER — Store the decision

```bash
forge-next remember --type decision --title "<what you built>" --content "<why and how>"
```

This closes the loop — future sessions recall this decision via `forge-next recall`.

## Test Quality Checklist

Before considering a test "done", verify:

- [ ] **Fails for the right reason** — error message matches your expectation
- [ ] **Tests behavior, not implementation** — would survive a refactor
- [ ] **Descriptive name** — someone can understand the intent without reading the body
- [ ] **Independent** — doesn't depend on other test execution order
- [ ] **Fast** — unit tests should complete in <1 second each
- [ ] **Deterministic** — same input always produces same result (no time dependencies, no random)

## When to Write Which Type of Test

| Change | Test type | Scope |
|--------|-----------|-------|
| New pure function | Unit test | Input → output, edge cases |
| New API endpoint | Integration test | Request → response, status codes |
| Bug fix | Regression test | Reproduce the exact bug, then fix |
| Refactoring | Existing tests should pass | No new tests needed if behavior unchanged |
| New CLI command | E2E test | Invoke binary, check stdout/stderr |
| Database change | Integration test | Write → read → verify with real DB |

## Edge Cases to Always Test

For any new function, consider testing:

1. **Empty input** — empty string, empty vec, None
2. **Boundary values** — 0, 1, -1, MAX, MIN
3. **Unicode/special chars** — in user input, file paths, identifiers
4. **Concurrent access** — if shared state exists, test with multiple threads
5. **Error paths** — invalid input, missing files, network failure
6. **Large input** — does it handle 10K items? 1M chars?

Don't test all of these for every function — use judgment. But for public APIs and critical paths, test at least empty + boundary + error.

## Mocking Guidelines

**Mock at boundaries, test real logic.**

| Mock this | Don't mock this |
|-----------|-----------------|
| External HTTP APIs | Your own functions |
| Database connections (for unit tests) | Pure logic |
| File system (for unit tests) | Data structures |
| Time/randomness | Business rules |
| Third-party services | Your domain model |

**When NOT to mock the database:**
- Integration tests should use a real (in-memory) database
- If a bug was caused by DB behavior (FK constraints, NULL handling), mocking hides it
- For Rust/SQLite: use `Connection::open_in_memory()` — it's fast enough

**Mock anti-patterns:**
- Mocking the thing you're testing (testing the mock, not the code)
- Mock chain: `mock_a.returns(mock_b.returns(mock_c...))` — redesign the interface
- Mocking value objects — just construct the real thing

## Integration Tests After Unit Tests

After the unit TDD cycle, add integration tests that verify the full pipeline:
- Use realistic data (not synthetic)
- Test the actual code path that production uses
- Verify data formats match between producers and consumers
- For services: test with a real (in-memory) database, not mocks

## Test Naming Convention

```
test_<unit>_<scenario>_<expected_behavior>
```

Examples:
- `test_recall_empty_query_returns_empty` — not `test_recall_1`
- `test_blast_radius_nonexistent_file_returns_zero` — not `test_blast_radius`
- `test_remember_decision_creates_affects_edges` — describes the behavior

## Commit After Each Cycle

After each RED → GREEN → REFACTOR cycle:
```bash
git add <changed files>
git commit -m "feat: add <behavior> with tests"
```

Small, atomic commits. Each commit should leave tests green.

## Anti-Patterns

| Don't | Do |
|-------|-----|
| Write code then write tests | Write test first, watch it fail |
| Test implementation details | Test behavior and outcomes |
| Skip TDD for "simple" changes | Simple changes are where bugs hide |
| Mock everything | Mock boundaries, test real logic |
| Commit without running full suite | Run full test suite before every commit |
| Write tests that never fail | If test can't fail, it doesn't protect anything |
| Test private methods directly | Test through the public interface |
| Assert on entire large structures | Assert on the specific field/property that matters |
