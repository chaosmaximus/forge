<!-- SPDX-License-Identifier: Apache-2.0 -->
---
name: forge-verify
description: Use before claiming work is complete — requires running verification commands and confirming output. Evidence before assertions. Integrates with Forge doctor and live UAT.
---

# Forge Verify — Evidence Before Assertions

Never claim work is done without proof. Run the commands. Check the output. Verify in production.

**Iron Law:** NO COMPLETION CLAIMS WITHOUT EVIDENCE.

## The Checklist

Before saying "done", "fixed", "working", or "all tests pass":

### 1. Run the tests (don't assume)

```bash
cargo test --workspace 2>&1 | grep "test result" | awk '{sum += $4; fail += $6} END {print sum " passed, " fail " failed"}'
```

**Read the output.** Don't say "all tests pass" without running them.

### 2. Run clippy (zero warnings)

```bash
cargo clippy -p forge-daemon -p forge-core -p forge-cli -- -W clippy::all 2>&1 | grep -c "^warning:"
```

Must be 0.

### 3. Check Forge health

```bash
forge-next doctor
forge-next manas-health
```

All checks should be [OK]. All 8 layers should be populated.

### 4. UAT the feature you built

Don't just test in unit tests. Test with the live daemon:

```bash
# Rebuild the daemon from the public repo and restart
# (daemon source lives at https://github.com/chaosmaximus/forge)
cargo install --git https://github.com/chaosmaximus/forge forge-daemon forge-cli
pkill -f "forge-daemon"; sleep 2
forge-daemon &
```

Then exercise the feature:
- Happy path: does it produce the expected output?
- Error path: does it fail gracefully?
- Edge case: empty input, large input, special characters?
- Production path: does the daemon handler path work, not just the unit test path?

### 5. Check for regressions

```bash
forge-next blast-radius --file <modified-file>
```

Are any callers or co-affected files broken by your change?

### 6. Adversarial review (for significant changes)

For changes touching 5+ files or adding new protocol variants:
- Dispatch a Codex adversarial review
- Fix all HIGH findings
- Address MEDIUM findings or document why not

### 7. Store completion

```bash
forge-next remember --type decision --title "<what was completed>" --content "<summary + test count>"
```

## Anti-Patterns

| Don't | Do |
|-------|-----|
| "Tests pass" (without running them) | Show the actual output |
| "It works" (tested only in unit tests) | UAT with live daemon |
| "No issues found" (without adversarial review) | Run Codex review for significant changes |
| Commit without pushing | Push after every commit |
| Skip doctor check | Run forge-next doctor after every restart |
