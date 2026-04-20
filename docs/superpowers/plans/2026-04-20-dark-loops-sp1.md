# SP1 — Dark-Loop Closure Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Close 4 dark feedback loops in the Forge daemon (bugs #45, #53, #54, #55 from SESSION-GAPS.md) so `context-stats`, `stats`, `tools`, and `skills-list` report non-zero counters that reflect actual daemon work.

**Architecture:** Three writer-channel / direct-write counter fixes plus one populator-invocation at daemon start. All fixes extend existing patterns. One PR on feature branch `sp1/dark-loops`. Five commits total (4 fix + 1 integration test). Doctor probes deferred per spec §11.2. Adversarial Codex review via `codex exec` at every commit boundary. `simplify` skill run on changed code after every GREEN.

**Tech Stack:** Rust, SQLite (rusqlite 0.32, bundled), tokio async, tempfile for tests, `cargo test --workspace` + `cargo clippy --workspace -- -W clippy::all -D warnings` as gates.

**Spec:** `docs/superpowers/specs/2026-04-20-dark-loops-sp1-design.md` (commit `c4a8175`).

**Bugs covered:**
- **#45** `context_injections = 0` — proactive-context path never calls `try_send(RecordInjection)`.
- **#53** 24h extractions counter never increments — no writer-channel command exists for extraction metrics.
- **#54** 42 tools all `used: 0x` — `record_tool_use()` function exists but is never called from extractor tool-chunk scan.
- **#55** `skills-list` returns 0 — `refresh_skills()` exists but never auto-runs on daemon boot.

---

## File Structure

| File | Purpose | Tasks |
|------|---------|-------|
| `crates/daemon/src/main.rs:244+` | Daemon startup — add skills auto-index after auto-vacuum block | 1.3 |
| `crates/daemon/src/skills.rs` | Existing `refresh_skills(&conn, &path)` populator (no code change, test only) | 1.1, 1.5 |
| `crates/daemon/src/server/handler.rs:1985,2004,2037` | 3 proactive-context return points — add `try_send(RecordInjection)` | 2.3 |
| `crates/daemon/src/server/handler.rs:2762-2772` | Reference pattern (CompileContext RecordInjection) — DO NOT modify | 2.1 reference |
| `crates/daemon/src/workers/extractor.rs` | Extractor — parse tool names from chunks (#54), emit `RecordExtraction` (#53) | 3.3, 4.3 |
| `crates/daemon/src/chunk.rs` (or wherever `Chunk` struct lives) | Extend chunk struct with `tool_names: Vec<String>` | 3.3 |
| `crates/daemon/src/db/manas.rs:338-345` | Existing `record_tool_use(conn, tool_id)` — call site added in extractor (no code change to manas.rs unless helper needed) | 3.3 |
| `crates/daemon/src/server/writer.rs:19` | `WriteCommand` enum — add `RecordExtraction` variant | 4.3 |
| `crates/daemon/src/server/writer.rs:~210` | Writer match arm — add `RecordExtraction` handler | 4.3 |
| `crates/daemon/src/db/metrics.rs` (NEW file) | New DB helper `record_extraction()` | 4.3 |
| `crates/daemon/src/db/mod.rs` | Re-export `metrics` module | 4.3 |
| `crates/daemon/tests/e2e_sp1_dark_loops.rs` (NEW file) | Integration test covering all 4 loops | 5.1-5.5 |

**MUST NOT TOUCH** (per spec §11):
- `crates/daemon/src/db/ops.rs` — uncommitted 2A-4c1 T3 work lives here.
- `crates/daemon/src/db/schema.rs` — do not add or modify migrations (no schema changes required).
- Any new protocol variants on `Request`/`Response` (that's 2A-4c1's territory).

---

## Adversarial Codex Review Template

Every commit invokes this review before the `git commit` step. Used verbatim in tasks 1.6, 2.6, 3.6, 4.6, 5.4, 8.

```bash
codex exec <<'PROMPT'
You are an adversarial reviewer. This diff is about to be committed to a production Rust daemon (forge-daemon). Find reasons NOT to commit it.

Focus on these failure modes:
1. Concurrency: could any code path here drop a tick or deadlock under load?
2. Test coverage: does the test genuinely exercise the claim, or mock the critical boundary?
3. Schema/type assumptions: does anything here assume a column/struct shape that could change?
4. Error paths: what untested errors propagate as panics, silent drops, or incorrect state?
5. Spec drift: compare against docs/superpowers/specs/2026-04-20-dark-loops-sp1-design.md. Any mismatch?
6. Scope creep: does anything in this diff belong to the deferred doctor probes or to 2A-4c1?

Output: 5-10 specific issues ranked by severity (BLOCKER > HIGH > MEDIUM > LOW). For each, cite the diff line and explain the failure mode. If you find no blockers, say so explicitly.

The diff:
$(git diff --staged)
PROMPT
```

The reviewer (you) reads Codex output, decides which findings to act on. Any BLOCKER must be addressed before commit. HIGH findings discussed + either fixed or explicitly declined in PR description.

---

## `simplify` Skill Invocation Template

After every GREEN (test passing with minimal implementation), invoke:

```
Skill: simplify
```

The skill reviews recently-changed code for reuse, quality, and efficiency, then fixes issues inline. Accept its changes; if any touch behavior (not just style), re-run the unit test to confirm it still passes before committing.

---

## Task 0: Preflight

### Task 0: Branch hygiene + build verification

**Files:** none modified — verification only.

- [ ] **Step 0.1: Confirm current branch is `sp1/dark-loops`**

Run: `git branch --show-current`
Expected: `sp1/dark-loops`

If you're on `master`, stop and switch: `git checkout sp1/dark-loops`.

- [ ] **Step 0.2: Fetch latest from origin**

Run: `git fetch origin master`
Expected: fetch completes silently (or with list of new commits).

- [ ] **Step 0.3: Rebase onto origin/master**

Run: `git rebase origin/master`
Expected: "Current branch sp1/dark-loops is up to date." OR a clean rebase with SP1 commits on top.

If conflicts arise in `crates/daemon/src/db/ops.rs` — **stop**. Notify the human; do not resolve on your own. 2A-4c1 T3 may have landed; your plan needs coordination.

- [ ] **Step 0.4: Verify clean baseline build**

Run: `cargo build --workspace`
Expected: compiles without errors.

- [ ] **Step 0.5: Verify baseline test suite passes**

Run: `cargo test --workspace`
Expected: all tests pass (count may differ from the 1,294 baseline as 2A-4c1 tests land).

- [ ] **Step 0.6: Verify clippy clean**

Run: `cargo clippy --workspace -- -W clippy::all -D warnings`
Expected: 0 warnings.

- [ ] **Step 0.7: Record baseline counter values for dogfood comparison**

Run: `forge-next context-stats && forge-next stats && forge-next tools | head -10 && forge-next skills-list 2>&1 | head -5`
Expected (or similar): Injections=0, Extractions=0, tools all `used: 0x`, `SkillsList { skills: [], count: 0 }`.

Record the output; you will compare against post-fix values in Task 6 (dogfood validation).

---

## Fix 1: #55 Skills Registry Auto-Populate

**Smallest change — 3 lines in `main.rs` + tempdir test. Ship this first so the engineer confirms the TDD cycle + Codex loop work end-to-end before harder fixes.**

### Task 1.1: Write failing test for `refresh_skills()` auto-index

**Files:**
- Test (inline): `crates/daemon/src/skills.rs` (add to existing `#[cfg(test)] mod tests`)

- [ ] **Step 1.1: Inspect the existing `refresh_skills` signature**

Run: `grep -n "pub fn refresh_skills\|pub fn index_skills" crates/daemon/src/skills.rs`
Expected: find the entry point function signature. Typical shape:
```rust
pub fn refresh_skills(conn: &Connection, dir: &Path) -> anyhow::Result<usize>
```
Note the exact return type for the test.

- [ ] **Step 1.2: Inspect the existing `list_skills` signature**

Run: `grep -n "pub fn list_skills" crates/daemon/src/skills.rs`
Expected: find the read-side function that `handler.rs:5585` calls.

- [ ] **Step 1.3: Write the failing unit test**

Append to the `#[cfg(test)] mod tests` block in `crates/daemon/src/skills.rs`:

```rust
#[test]
fn test_refresh_skills_populates_registry_from_tempdir() {
    use tempfile::tempdir;
    use std::fs;
    use rusqlite::Connection;

    // Arrange: 3 fixture skills in a temp dir
    let dir = tempdir().unwrap();
    for name in ["alpha", "beta", "gamma"] {
        let skill_dir = dir.path().join(name);
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(
            skill_dir.join("SKILL.md"),
            format!(
                "---\nname: {name}\ndescription: fixture skill {name}\ncategory: test\n---\n\n# {name}\n"
            ),
        ).unwrap();
    }

    // In-memory DB with the skill_registry schema
    let conn = Connection::open_in_memory().unwrap();
    crate::db::schema::ensure_schema(&conn).unwrap();   // or whatever the project calls its migration entry point

    // Act
    let count = crate::skills::refresh_skills(&conn, dir.path()).unwrap();

    // Assert
    assert_eq!(count, 3, "refresh_skills should return the number indexed");
    let listed = crate::skills::list_skills(&conn, None).unwrap();
    assert_eq!(listed.len(), 3, "list_skills should return all 3 fixtures");
    let names: Vec<String> = listed.iter().map(|s| s.name.clone()).collect();
    assert!(names.contains(&"alpha".to_string()));
    assert!(names.contains(&"beta".to_string()));
    assert!(names.contains(&"gamma".to_string()));
}
```

**If `ensure_schema` / `list_skills` signatures differ**, adjust the test to match actual signatures (use `grep -n` to verify). Do NOT change production code in this step — only the test.

- [ ] **Step 1.4: Verify test compiles but FAILS**

Run: `cargo test -p forge-daemon --lib test_refresh_skills_populates_registry_from_tempdir`
Expected: if `refresh_skills` + `list_skills` already work on a fresh conn, this test may **pass immediately** (the bug is not in `refresh_skills`, it's in daemon init). If so, the test validates the function but doesn't drive the fix — see Step 1.5.

If it PASSES immediately: this confirms the code paths are correct and the bug is purely that they're never called. Jump to Step 1.5 (daemon init integration test).

If it FAILS: note the error; adjust the test to match actual signatures; re-run.

- [ ] **Step 1.5: Write the failing integration test for daemon-init auto-index**

This is the real test that drives the fix. Integration test in `crates/daemon/tests/test_skills_auto_index.rs` (NEW file):

```rust
//! Test: daemon startup auto-populates skill_registry from configured skills directory.

use std::fs;
use tempfile::tempdir;

#[test]
fn daemon_auto_indexes_skills_on_start() {
    // Arrange: temp skills dir with 2 fixtures
    let skills_dir = tempdir().unwrap();
    for name in ["fixture-a", "fixture-b"] {
        let p = skills_dir.path().join(name);
        fs::create_dir_all(&p).unwrap();
        fs::write(
            p.join("SKILL.md"),
            format!("---\nname: {name}\ndescription: test\ncategory: test\n---\n\n# {name}\n"),
        ).unwrap();
    }

    let db = tempdir().unwrap();
    let db_path = db.path().join("forge.db").to_string_lossy().to_string();

    // Act: simulate what main.rs should do after DaemonState::new
    let state = forge_daemon::server::handler::DaemonState::new(&db_path).unwrap();
    let count = forge_daemon::skills::refresh_skills(&state.conn, skills_dir.path()).unwrap();

    // Assert
    assert_eq!(count, 2, "refresh_skills should index both fixtures");
    let listed = forge_daemon::skills::list_skills(&state.conn, None).unwrap();
    assert_eq!(listed.len(), 2, "list_skills should return both fixtures");
}
```

**Note**: if `DaemonState::new` or `list_skills` visibility is private, either (a) add `#[cfg(test)]` pub exposure via a `pub(crate) fn test_helper_refresh(...)` in the daemon lib, or (b) keep this test inline in the daemon lib rather than integration test. Choose whichever requires less API surface change.

- [ ] **Step 1.6: Run the new test — expect FAIL or BUILD-ERROR**

Run: `cargo test -p forge-daemon --test test_skills_auto_index daemon_auto_indexes_skills_on_start`
Expected: if visibility is blocked → build error (E0603 "private"). If path is accessible → test passes trivially (because `refresh_skills` works; the real bug is it's never called). Either way this test is not the driver.

- [ ] **Step 1.7: Decision — write ONE test that truly drives the fix**

Replace the integration test with a test that drives `main.rs` itself. This is harder because `main.rs` is the binary entry and hard to unit-test. **Pragmatic option**: write a test that verifies the DaemonState's conn has a populated registry after we invoke the NEW auto-index helper that `main.rs` will call.

Final test form (inline in `skills.rs` `#[cfg(test)] mod tests`):

```rust
#[test]
fn test_auto_populate_skill_registry_integrates_with_daemonstate() {
    use tempfile::tempdir;
    use std::fs;

    let skills_dir = tempdir().unwrap();
    for name in ["auto-a", "auto-b"] {
        let p = skills_dir.path().join(name);
        fs::create_dir_all(&p).unwrap();
        fs::write(
            p.join("SKILL.md"),
            format!("---\nname: {name}\ndescription: test\ncategory: test\n---\n\n# {name}\n"),
        ).unwrap();
    }
    let db = tempdir().unwrap();
    let db_path = db.path().join("f.db").to_string_lossy().to_string();

    let state = crate::server::handler::DaemonState::new(&db_path).unwrap();

    // THE CALL UNDER TEST — exists after Step 1.10's helper extraction.
    let n = crate::skills::auto_populate_on_start(&state.conn, skills_dir.path()).unwrap();

    assert_eq!(n, 2);
    let listed = crate::skills::list_skills(&state.conn, None).unwrap();
    assert_eq!(listed.len(), 2);
}
```

Here `auto_populate_on_start` is a new thin wrapper to add in Step 1.10. It wraps `refresh_skills` with the directory-missing-OK semantics from spec §5.2.

- [ ] **Step 1.8: Run the test — expect FAIL (function does not exist)**

Run: `cargo test -p forge-daemon --lib test_auto_populate_skill_registry_integrates_with_daemonstate`
Expected: build error `no function or associated item named auto_populate_on_start`.

### Task 1 continued: GREEN + Simplify + Codex + Commit

- [ ] **Step 1.9: Write minimal `auto_populate_on_start` helper**

Add to `crates/daemon/src/skills.rs` (public API):

```rust
/// Auto-populate the skill registry from `skills_dir` on daemon boot.
/// Returns the count indexed. If the directory does not exist, returns Ok(0)
/// and the caller logs a warning (no panic). Other errors propagate.
pub fn auto_populate_on_start(
    conn: &rusqlite::Connection,
    skills_dir: &std::path::Path,
) -> anyhow::Result<usize> {
    if !skills_dir.exists() {
        return Ok(0);
    }
    refresh_skills(conn, skills_dir)
}
```

- [ ] **Step 1.10: Run the test — expect PASS**

Run: `cargo test -p forge-daemon --lib test_auto_populate_skill_registry_integrates_with_daemonstate`
Expected: PASS.

- [ ] **Step 1.11: Wire the call into `main.rs` after DaemonState::new + auto-vacuum**

Open `crates/daemon/src/main.rs`. After the auto-vacuum block (currently ends around line 245), add:

```rust
// #55 — auto-populate skill registry on boot
let skills_dir = std::env::var("FORGE_SKILLS_DIR")
    .map(std::path::PathBuf::from)
    .ok()
    .or_else(|| {
        // Fallback: project-local "skills/" if running in a workspace-rooted cwd
        let cwd_skills = std::path::PathBuf::from("skills");
        if cwd_skills.exists() { Some(cwd_skills) } else { None }
    })
    .or_else(|| {
        // Final fallback: ~/.forge/skills
        dirs::home_dir().map(|h| h.join(".forge").join("skills"))
    });

if let Some(dir) = skills_dir {
    match forge_daemon::skills::auto_populate_on_start(&worker_state.conn, &dir) {
        Ok(n) if n > 0 => tracing::info!(skills = n, path = %dir.display(), "Skill registry populated on boot"),
        Ok(_)          => tracing::info!(path = %dir.display(), "Skill directory empty or missing — skills registry not populated"),
        Err(e)         => tracing::warn!(error = %e, path = %dir.display(),
                                         "Skill auto-index failed; call RefreshSkillsIndex to retry"),
    }
} else {
    tracing::debug!("No skills directory configured (no FORGE_SKILLS_DIR, no ./skills, no ~/.forge/skills)");
}
```

**Note**: `dirs` crate — check `Cargo.toml`; if not present, use `std::env::var("HOME").map(...)` instead.

- [ ] **Step 1.12: Run the full test suite**

Run: `cargo test --workspace`
Expected: all tests pass.

- [ ] **Step 1.13: Run `simplify` skill on changed code**

Invoke the simplify skill:
```
Skill: simplify
```
Accept its suggestions. If any touch behavior (e.g., if it rewrites the env-var cascade), re-run `cargo test --workspace` to confirm.

- [ ] **Step 1.14: Run clippy**

Run: `cargo clippy --workspace -- -W clippy::all -D warnings`
Expected: 0 warnings.

- [ ] **Step 1.15: Stage changes**

Run:
```bash
git add crates/daemon/src/skills.rs crates/daemon/src/main.rs
git status --short
```
Expected: only the 2 files staged. No ops.rs, no bench files.

- [ ] **Step 1.16: Adversarial Codex review**

Use the template in the "Adversarial Codex Review Template" section. Run `codex exec` with the staged diff.

**Triage**: address BLOCKER + HIGH findings; log MEDIUM/LOW for PR description. Re-run Step 1.12 + 1.14 if code changed.

- [ ] **Step 1.17: Commit**

```bash
git commit -m "$(cat <<'EOF'
fix(skills): auto-index skill_registry on daemon start (#55)

Skill registry was empty because nothing populated it at boot —
refresh_skills() existed but only ran on an explicit RefreshSkillsIndex
request. Added auto_populate_on_start helper (directory-missing returns
Ok(0) rather than error) and wired it into main.rs after DaemonState::new
+ auto-vacuum.

Directory resolution cascade: FORGE_SKILLS_DIR env var → ./skills in cwd
→ ~/.forge/skills. If none resolve, daemon boots with an empty registry
and logs a debug note.

Closes SESSION-GAPS #55.
Spec: docs/superpowers/specs/2026-04-20-dark-loops-sp1-design.md §3.1

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

- [ ] **Step 1.18: Verify commit shape**

Run: `git log --oneline -2 && git show --stat HEAD`
Expected: top commit is `fix(skills): ...`, touches only `skills.rs` + `main.rs` (+ possibly `Cargo.toml` if `dirs` crate was added).

---

## Fix 2: #45 Proactive Context Injection Recording

**Copy of the existing CompileContext pattern (`handler.rs:2762-2772`) into 3 proactive-context return sites at `handler.rs:1985, 2004, 2037`. No new WriteCommand variant — reuse `RecordInjection`.**

### Task 2: Wire proactive paths through writer channel

**Files:**
- Reference (read only): `crates/daemon/src/server/handler.rs:2762-2772` — the working pattern.
- Modify: `crates/daemon/src/server/handler.rs:~1985, ~2004, ~2037`.
- Test: inline `#[cfg(test)]` in `handler.rs`, OR `crates/daemon/tests/test_proactive_injection.rs` (NEW integration test).

- [ ] **Step 2.1: Read the reference pattern at handler.rs:2750-2790**

Run: `sed -n '2750,2790p' crates/daemon/src/server/handler.rs`
Expected: see the CompileContext `try_send(WriteCommand::RecordInjection { ... })` call. Note the exact fields and the `state.writer_tx` access.

- [ ] **Step 2.2: Read the 3 proactive return points**

Run: `sed -n '1975,2050p' crates/daemon/src/server/handler.rs`
Expected: find the three branches where `build_proactive_context(...)` is called and its result returned in a Response. Identify:
- Exact hook-event string for each branch (`PreBashChecked`, `PostBashCheck`, `PostEditCheck` — confirm via nearby context).
- How `session_id` is accessed (local variable vs request field).
- Response construction — where `proactive_context: Vec<ProactiveInjection>` is wrapped.

- [ ] **Step 2.3: Write the failing integration test**

Create `crates/daemon/tests/test_proactive_injection.rs`:

```rust
//! Integration test: proactive-context handlers record context_effectiveness rows.

use forge_core::protocol::request::Request;
use serde_json::json;
use tempfile::tempdir;

#[test]
fn proactive_context_hook_records_injection_row() {
    // Boot a daemon-like state in-process.
    let db = tempdir().unwrap();
    let db_path = db.path().join("f.db").to_string_lossy().to_string();
    let state = forge_daemon::server::handler::DaemonState::new_writer(
        &db_path,
        tokio::sync::broadcast::channel(16).0,  // events
        std::sync::Arc::new(std::sync::Mutex::new(forge_core::hlc::HybridClock::default())),
        std::time::Instant::now(),
    ).expect("DaemonState::new_writer");

    // Start a session so the session_id is valid.
    // (Adapt to actual start_session handler signature.)
    // ... existing precedent in test_e2e_lifecycle.rs ...

    // Fire the PreBashChecked hook
    let req = Request::PreBashChecked { /* fields — match actual variant */ };
    let resp = forge_daemon::server::handler::handle(&state, req).expect("handle");

    // Assert the context_effectiveness table has a row with context_type='proactive'
    let count: i64 = state.conn.query_row(
        "SELECT COUNT(*) FROM context_effectiveness WHERE context_type = 'proactive'",
        [],
        |r| r.get(0),
    ).unwrap();
    assert!(count >= 1, "proactive hook should record at least 1 injection row; got {count}");
}
```

**Pragmatic simplification**: if full handler test harness is too heavy, replace with a unit test that calls `build_proactive_context` + the NEW helper we add in Step 2.5 (`record_proactive_injection(&state.writer_tx, session_id, hook_event, &ctx)`). Test the helper directly.

- [ ] **Step 2.4: Run the test — expect FAIL**

Run: `cargo test -p forge-daemon --test test_proactive_injection proactive_context_hook_records_injection_row`
Expected: test fails — `count == 0` because no row is inserted.

- [ ] **Step 2.5: Extract a helper to record proactive injection (DRY)**

Add near the top of `crates/daemon/src/server/handler.rs` (or in a new module `handler_helpers.rs` if the file is getting unwieldy):

```rust
/// Record a proactive-context injection via the writer channel.
/// No-op if writer is not available (maintains existing RecordInjection semantics).
fn record_proactive_injection(
    writer_tx: Option<&tokio::sync::mpsc::Sender<super::writer::WriteCommand>>,
    session_id: &str,
    hook_event: &str,
    proactive_context: &[forge_core::protocol::response::ProactiveInjection],
) {
    let Some(tx) = writer_tx else { return };
    let chars: usize = proactive_context.iter().map(|i| i.content.len()).sum();
    if chars == 0 {
        return; // empty context — nothing to record
    }
    let summary = proactive_context
        .iter()
        .map(|i| format!("{}:{}", i.knowledge_type, i.content.len()))
        .collect::<Vec<_>>()
        .join(",");
    let _ = tx.try_send(super::writer::WriteCommand::RecordInjection {
        session_id: session_id.to_string(),
        hook_event: hook_event.to_string(),
        context_type: "proactive".to_string(),
        content_summary: summary,
        chars_injected: chars,
    });
}
```

**Exact struct field names**: check `WriteCommand::RecordInjection` variant at `server/writer.rs:19+` and the existing call at `handler.rs:2762-2772`. Match verbatim.

- [ ] **Step 2.6: Invoke the helper at each of the 3 proactive return points**

At each site (lines ~1985, ~2004, ~2037), BEFORE the `return Response::Ok { ... }` that contains proactive_context, insert:

```rust
record_proactive_injection(state.writer_tx.as_ref(), &session_id, "PreBashChecked", &proactive_context);
```

with the hook_event string adjusted per site:
- line ~1985 → `"PreBashChecked"`
- line ~2004 → `"PostBashCheck"`
- line ~2037 → `"PostEditCheck"`

**Verify** the actual Request variant at each line matches these hook names; if the Request variant is named differently, use that name instead.

- [ ] **Step 2.7: Run the test — expect PASS**

Run: `cargo test -p forge-daemon --test test_proactive_injection`
Expected: PASS.

- [ ] **Step 2.8: Run the full test suite**

Run: `cargo test --workspace`
Expected: all tests pass. Pay attention to any handler-related tests that depend on the Response shape — the helper adds a side-effect (writer channel write) but does not change Response.

- [ ] **Step 2.9: `simplify` skill**

```
Skill: simplify
```
Accept its changes. Re-run `cargo test --workspace` if behavior touched.

- [ ] **Step 2.10: Clippy**

Run: `cargo clippy --workspace -- -W clippy::all -D warnings`
Expected: 0 warnings.

- [ ] **Step 2.11: Stage**

```bash
git add crates/daemon/src/server/handler.rs crates/daemon/tests/test_proactive_injection.rs
git status --short
```

- [ ] **Step 2.12: Adversarial Codex review**

Use the template. Pay extra attention to:
- Does Codex spot concurrency issues (the `try_send` may silently drop on backpressure — is that acceptable? Spec says yes).
- Does Codex check that the `context_type = 'proactive'` value is distinct from CompileContext's `'full_context'`?

Address BLOCKER + HIGH findings.

- [ ] **Step 2.13: Commit**

```bash
git commit -m "$(cat <<'EOF'
fix(injection): record proactive context injections (#45)

context_injections was 0 ever despite many PreBash/PostBash/PostEdit
hooks firing. CompileContext (SessionStart) recorded injections via
WriteCommand::RecordInjection, but the three build_proactive_context
return sites (handler.rs:~1985,~2004,~2037) never did.

Added record_proactive_injection() helper that mirrors the existing
CompileContext pattern and invoked it at all three sites. context_type
is "proactive" (distinct from CompileContext's "full_context") so
downstream analytics can split effectiveness by source.

Closes SESSION-GAPS #45 — "THE BET" per DAEMON-STRATEGY-V3 §4.
Spec: docs/superpowers/specs/2026-04-20-dark-loops-sp1-design.md §3.2

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

- [ ] **Step 2.14: Verify commit**

Run: `git log --oneline -3 && git show --stat HEAD`
Expected: `fix(injection): ...` on top, touches `handler.rs` + new test file.

---

## Fix 3: #54 Per-Tool Counter in Extractor

**Parse `<tool_use name="X">` from transcript chunks, call existing `record_tool_use(conn, tool_id)` per name. Uses direct `UPDATE` — no writer channel for this one.**

### Task 3: Per-tool counter

**Files:**
- Modify: `crates/daemon/src/workers/extractor.rs` — tool-name parse + `record_tool_use` calls.
- Modify: `crates/daemon/src/chunk.rs` (or wherever `Chunk` is defined) — add `tool_names: Vec<String>` field (if not already present).
- Reference (read only): `crates/daemon/src/db/manas.rs:338-345` — `record_tool_use` signature.
- Test: inline `#[cfg(test)]` in `workers/extractor.rs`.

- [ ] **Step 3.1: Inspect `record_tool_use` signature**

Run: `grep -n "pub fn record_tool_use" crates/daemon/src/db/manas.rs`
Expected: signature like `pub fn record_tool_use(conn: &Connection, tool_id: &str) -> rusqlite::Result<bool>`. Return is `true` if a row was updated, `false` if tool_id wasn't found.

- [ ] **Step 3.2: Inspect the `tool` table schema to confirm ID shape**

Run: `grep -n "CREATE TABLE.*tool\|CREATE TABLE IF NOT EXISTS tool" crates/daemon/src/db/schema.rs | head -10`
Expected: find the `tool` table definition. Determine whether `tool.id` is the same as `tool.name` (slug) or differs. If differs, we need a `get_tool_id_by_name` helper; if same, we can use name directly.

Likely: `tool.id` is a slugified name (e.g., "bash" for the Bash tool). Confirm by running `forge-next tools --output json` once during implementation.

- [ ] **Step 3.3: Inspect the existing tool-use detection path**

Run: `grep -n "has_tool_use\|tool_use_count" crates/daemon/src/workers/extractor.rs | head -20`
Expected: find where `has_tool_use: bool` is checked (the V1 audit cited lines ~267-295) and where `increment_tool_use_count` is called. This is where we'll extend to parse names.

- [ ] **Step 3.4: Inspect Chunk struct**

Run: `grep -rn "struct Chunk\b" crates/daemon/src/ | head -5`
Expected: find the Chunk struct. Note its current fields.

- [ ] **Step 3.5: Write the failing unit test**

Append to the `#[cfg(test)] mod tests` block in `crates/daemon/src/workers/extractor.rs`:

```rust
#[test]
fn test_parse_tool_names_from_transcript_chunk() {
    // Arrange: a transcript fragment with 3 tool uses
    let transcript = r#"
Some text here.
<tool_use name="Bash">
{"command": "ls"}
</tool_use>
More text.
<tool_use name="Read">
{"file_path": "/tmp/a"}
</tool_use>
<tool_use name="Edit">
{"file_path": "/tmp/a", "old_string": "x", "new_string": "y"}
</tool_use>
Final text.
"#;

    // Act
    let names = parse_tool_names(transcript);

    // Assert
    assert_eq!(names, vec!["Bash".to_string(), "Read".to_string(), "Edit".to_string()]);
}

#[test]
fn test_extractor_records_per_tool_use_counter() {
    use rusqlite::Connection;
    let conn = Connection::open_in_memory().unwrap();
    crate::db::schema::ensure_schema(&conn).unwrap();

    // Seed the tool table with 3 known tools
    for (id, name) in [("bash", "Bash"), ("read", "Read"), ("edit", "Edit")] {
        conn.execute(
            "INSERT INTO tool (id, name, kind, use_count) VALUES (?1, ?2, 'Cli', 0)",
            rusqlite::params![id, name],
        ).unwrap();
    }

    // Transcript with 2 Bash uses + 1 Read + 1 unknown "Ghost"
    let transcript = r#"
<tool_use name="Bash"></tool_use>
<tool_use name="Bash"></tool_use>
<tool_use name="Read"></tool_use>
<tool_use name="Ghost"></tool_use>
"#;

    // Act: record tool uses (NEW function — added in Step 3.7)
    record_tool_uses_from_transcript(&conn, transcript).unwrap();

    // Assert per-tool counters
    let bash_count: i64 = conn.query_row("SELECT use_count FROM tool WHERE id = 'bash'", [], |r| r.get(0)).unwrap();
    let read_count: i64 = conn.query_row("SELECT use_count FROM tool WHERE id = 'read'", [], |r| r.get(0)).unwrap();
    let edit_count: i64 = conn.query_row("SELECT use_count FROM tool WHERE id = 'edit'", [], |r| r.get(0)).unwrap();
    assert_eq!(bash_count, 2);
    assert_eq!(read_count, 1);
    assert_eq!(edit_count, 0, "Edit was not in transcript — should stay 0");
    // "Ghost" was not in registry — should not panic or insert, just log debug
}
```

- [ ] **Step 3.6: Run tests — expect FAIL**

Run: `cargo test -p forge-daemon --lib test_parse_tool_names test_extractor_records_per_tool`
Expected: build error — `parse_tool_names` and `record_tool_uses_from_transcript` do not exist.

- [ ] **Step 3.7: Add `parse_tool_names` + `record_tool_uses_from_transcript`**

Add to `crates/daemon/src/workers/extractor.rs` (NOT at module top — put near existing tool-use detection code, inside the same module):

```rust
/// Parse tool names from a transcript fragment. Matches `<tool_use name="X">`
/// openings. Order-preserving; duplicates included (two Bash calls → two entries).
fn parse_tool_names(transcript: &str) -> Vec<String> {
    let mut out = Vec::new();
    // Simple scan — avoids pulling in regex. Pattern is '<tool_use name="'
    let pat = "<tool_use name=\"";
    let mut i = 0;
    let bytes = transcript.as_bytes();
    while let Some(off) = transcript[i..].find(pat) {
        let start = i + off + pat.len();
        if let Some(end_off) = transcript[start..].find('"') {
            let name = &transcript[start..start + end_off];
            if !name.is_empty() {
                out.push(name.to_string());
            }
            i = start + end_off + 1;
        } else {
            break;
        }
        if i >= bytes.len() { break }
    }
    out
}

/// For each tool name found in the transcript, increment the registry counter.
/// Unknown names (not in `tool` table) log at debug and are skipped.
pub(crate) fn record_tool_uses_from_transcript(
    conn: &rusqlite::Connection,
    transcript: &str,
) -> rusqlite::Result<usize> {
    let names = parse_tool_names(transcript);
    let mut incremented = 0;
    for name in &names {
        // tool.id is the slug of name — confirm during dogfood
        let tool_id = name.to_lowercase();  // simplest slug; adjust if registry differs
        match crate::db::manas::record_tool_use(conn, &tool_id) {
            Ok(true)  => incremented += 1,
            Ok(false) => tracing::debug!(tool = %tool_id, "tool not in registry — skipping counter"),
            Err(e)    => tracing::warn!(error = %e, tool = %tool_id, "record_tool_use failed"),
        }
    }
    Ok(incremented)
}
```

- [ ] **Step 3.8: Run tests — expect PASS**

Run: `cargo test -p forge-daemon --lib test_parse_tool_names test_extractor_records_per_tool`
Expected: PASS. If `test_extractor_records_per_tool_use_counter` fails on the Bash count, check the slugification logic; you may need to match actual registry IDs (e.g., `"bash"` not `"Bash"`).

- [ ] **Step 3.9: Wire `record_tool_uses_from_transcript` into the existing tool-use detection path**

Locate the existing code (around lines 267-295 in `workers/extractor.rs`) where `has_tool_use` is counted + `session.tool_use_count` is incremented. Add a call to `record_tool_uses_from_transcript(&locked.conn, &transcript)` alongside the session counter increment.

**Exact insertion** — adapt to actual code shape:
```rust
// Existing:
let tool_use_count = chunks.iter().filter(|c| c.has_tool_use).count();
if tool_use_count > 0 {
    crate::sessions::increment_tool_use_count(&locked.conn, &session_id, tool_use_count)?;
    // NEW: also increment per-tool counters
    for chunk in &chunks {
        if chunk.has_tool_use {
            if let Err(e) = record_tool_uses_from_transcript(&locked.conn, &chunk.text) {
                tracing::warn!(error = %e, "per-tool counter update failed for chunk");
            }
        }
    }
}
```

**Adjust field name** (`chunk.text` vs `chunk.content`) to match actual Chunk struct.

- [ ] **Step 3.10: Run full test suite**

Run: `cargo test --workspace`
Expected: all tests pass.

- [ ] **Step 3.11: `simplify`**

```
Skill: simplify
```

- [ ] **Step 3.12: Clippy**

Run: `cargo clippy --workspace -- -W clippy::all -D warnings`
Expected: 0 warnings.

- [ ] **Step 3.13: Stage**

```bash
git add crates/daemon/src/workers/extractor.rs
git status --short
```

- [ ] **Step 3.14: Adversarial Codex review**

Use the template. Focus questions for Codex:
- Does `parse_tool_names` handle escaped quotes in attribute values?
- What happens with nested `<tool_use>` or malformed XML — can it infinite-loop?
- Is the slug logic (`to_lowercase`) sufficient for all registered tools, or will it miss multi-word tools?

Address findings. Re-run tests if code changed.

- [ ] **Step 3.15: Commit**

```bash
git commit -m "$(cat <<'EOF'
fix(tools): increment per-tool use_count in extractor (#54)

42 registered tools all showed `used: 0x` because record_tool_use()
was never called outside tests. Extractor already counted tool_use
chunks (session.tool_use_count) but did not attribute usage per tool.

Added parse_tool_names() (scans <tool_use name="X"> openings) and
record_tool_uses_from_transcript() (calls record_tool_use per name,
lowercase-slug match, debug-log on unknown names). Wired into the
existing has_tool_use increment branch alongside session counter.

Complementary to Phase 2A-4c1's session_tool_call table (row-per-
invocation log) — this is the aggregate counter that forge-next tools
reads today. Post-2A-4c2 (hook-driven ingestion), future ticket may
derive use_count from COUNT(*) on session_tool_call.

Closes SESSION-GAPS #54.
Spec: docs/superpowers/specs/2026-04-20-dark-loops-sp1-design.md §3.3

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Fix 4: #53 Extraction Metrics via Writer Channel

**Largest of the four. Add `WriteCommand::RecordExtraction` variant + writer arm + new `db/metrics.rs` helper. Extractor emits on both success and error paths.**

### Task 4: Extraction metric plumbing

**Files:**
- Modify: `crates/daemon/src/server/writer.rs:19+` — add variant.
- Modify: `crates/daemon/src/server/writer.rs:~210+` — add match arm.
- Create: `crates/daemon/src/db/metrics.rs` — new helper `record_extraction`.
- Modify: `crates/daemon/src/db/mod.rs` — re-export.
- Modify: `crates/daemon/src/workers/extractor.rs` — emit `RecordExtraction` at success + error paths.
- Test: inline `#[cfg(test)]` in `writer.rs` and `metrics.rs`.

- [ ] **Step 4.1: Verify the `metrics` table exists in schema**

Run: `grep -n "CREATE TABLE.*metrics\|CREATE TABLE IF NOT EXISTS metrics" crates/daemon/src/db/schema.rs`
Expected: find the table definition OR confirm absence. If the table does NOT exist, **STOP** — the plan must add a schema migration, which is out of scope per SP1 §9. Defer #53 to a separate plan and proceed to Fix 5 directly.

Likely: the table exists because `ops.rs:1403-1405` already queries `WHERE metric_type='extraction'`.

- [ ] **Step 4.2: Verify the stats query shape in ops.rs** (READ ONLY — DO NOT modify ops.rs)

Run: `sed -n '1395,1415p' crates/daemon/src/db/ops.rs | head -25`
Expected: see the SELECT that reads `metrics WHERE metric_type='extraction'`. Note the columns selected (e.g., `COUNT(*), SUM(CASE WHEN meta LIKE '%error%' ...)`). Our INSERT must satisfy those columns.

- [ ] **Step 4.3: Write failing test for `db::metrics::record_extraction`**

Create `crates/daemon/src/db/metrics.rs`:

```rust
//! Metrics table helpers. Owned by this module; do not modify ops.rs for SP1.
//!
//! The `metrics` table is the canonical store for counters read by
//! `forge-next stats`. See ops.rs:~1403 for the read query.

use rusqlite::{params, Connection};

/// Record one extraction event. Called from the writer actor in response
/// to `WriteCommand::RecordExtraction`. On success, `error` is None.
/// On error, `error` carries the stringified failure. Token counts and cost
/// are stored as JSON in the `meta` column.
pub fn record_extraction(
    conn: &Connection,
    session_id: &str,
    memories_created: usize,
    tokens_in: u64,
    tokens_out: u64,
    cost_cents: u64,
    error: Option<&str>,
) -> rusqlite::Result<()> {
    let meta = serde_json::json!({
        "tokens_in":  tokens_in,
        "tokens_out": tokens_out,
        "cost_cents": cost_cents,
        "error":      error,
    }).to_string();
    conn.execute(
        "INSERT INTO metrics (metric_type, session_id, value, meta, timestamp)
         VALUES ('extraction', ?1, ?2, ?3, datetime('now'))",
        params![session_id, memories_created as i64, meta],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        crate::db::schema::ensure_schema(&conn).unwrap();
        conn
    }

    #[test]
    fn test_record_extraction_success_writes_row() {
        let conn = setup();
        record_extraction(&conn, "sess-1", 5, 1000, 500, 12, None).unwrap();
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM metrics WHERE metric_type = 'extraction' AND session_id = 'sess-1'",
            [], |r| r.get(0)
        ).unwrap();
        assert_eq!(count, 1);
        let value: i64 = conn.query_row(
            "SELECT value FROM metrics WHERE metric_type = 'extraction' AND session_id = 'sess-1'",
            [], |r| r.get(0)
        ).unwrap();
        assert_eq!(value, 5);
    }

    #[test]
    fn test_record_extraction_error_writes_row_with_error_in_meta() {
        let conn = setup();
        record_extraction(&conn, "sess-2", 0, 0, 0, 0, Some("connection refused")).unwrap();
        let meta: String = conn.query_row(
            "SELECT meta FROM metrics WHERE metric_type = 'extraction' AND session_id = 'sess-2'",
            [], |r| r.get(0)
        ).unwrap();
        assert!(meta.contains("connection refused"), "meta should carry error string");
    }
}
```

- [ ] **Step 4.4: Re-export `metrics` in `db/mod.rs`**

Open `crates/daemon/src/db/mod.rs` and add:
```rust
pub mod metrics;
```

- [ ] **Step 4.5: Run new tests — expect PASS (already written alongside function)**

Run: `cargo test -p forge-daemon --lib metrics::tests::test_record_extraction`
Expected: PASS.

If the `metrics` table schema doesn't have `value`/`meta`/`timestamp` columns, adapt the INSERT to actual columns; check via `sqlite3 $DBPATH '.schema metrics'` on a real daemon DB.

- [ ] **Step 4.6: Add `WriteCommand::RecordExtraction` variant**

Edit `crates/daemon/src/server/writer.rs`. Find `pub enum WriteCommand` (line 19) and add a variant:

```rust
pub enum WriteCommand {
    // ... existing variants ...
    RecordInjection {
        session_id: String,
        hook_event: String,
        context_type: String,
        content_summary: String,
        chars_injected: usize,
    },
    /// Record extraction metric (success or error). SP1 #53.
    RecordExtraction {
        session_id: String,
        memories_created: usize,
        tokens_in: u64,
        tokens_out: u64,
        cost_cents: u64,
        error: Option<String>,
    },
    // ... rest ...
}
```

- [ ] **Step 4.7: Add writer match arm**

In `server/writer.rs` around line 210 (near the existing `RecordInjection` arm), add:

```rust
WriteCommand::RecordExtraction {
    session_id,
    memories_created,
    tokens_in,
    tokens_out,
    cost_cents,
    error,
} => {
    let _ = crate::db::metrics::record_extraction(
        &self.state.conn,
        &session_id,
        memories_created,
        tokens_in,
        tokens_out,
        cost_cents,
        error.as_deref(),
    );
}
```

`let _ =` — same best-effort semantics as the existing `RecordInjection` arm.

- [ ] **Step 4.8: Write failing test for extractor emitting RecordExtraction**

Add to `crates/daemon/src/workers/extractor.rs` inside `#[cfg(test)] mod tests`:

```rust
#[test]
fn test_extractor_success_emits_record_extraction() {
    use tokio::sync::mpsc;
    let (tx, mut rx) = mpsc::channel::<crate::server::writer::WriteCommand>(8);

    // Simulate an extractor success call (adapt to actual signature)
    emit_extraction_metric(&tx, "sess-1", 3, 1000, 500, 10, None);

    // Assert RecordExtraction landed in the channel
    let cmd = rx.try_recv().expect("expected RecordExtraction in channel");
    match cmd {
        crate::server::writer::WriteCommand::RecordExtraction { session_id, memories_created, error, .. } => {
            assert_eq!(session_id, "sess-1");
            assert_eq!(memories_created, 3);
            assert!(error.is_none());
        }
        other => panic!("unexpected command: {other:?}"),
    }
}

#[test]
fn test_extractor_error_emits_record_extraction_with_error() {
    use tokio::sync::mpsc;
    let (tx, mut rx) = mpsc::channel::<crate::server::writer::WriteCommand>(8);

    emit_extraction_metric(&tx, "sess-2", 0, 0, 0, 0, Some("http 500"));

    let cmd = rx.try_recv().expect("expected RecordExtraction in channel");
    match cmd {
        crate::server::writer::WriteCommand::RecordExtraction { error, .. } => {
            assert_eq!(error.as_deref(), Some("http 500"));
        }
        other => panic!("unexpected command: {other:?}"),
    }
}
```

- [ ] **Step 4.9: Run tests — expect FAIL (function missing)**

Run: `cargo test -p forge-daemon --lib test_extractor_success_emits test_extractor_error_emits`
Expected: build error — `emit_extraction_metric` does not exist.

- [ ] **Step 4.10: Add `emit_extraction_metric` helper**

Add to `crates/daemon/src/workers/extractor.rs`:

```rust
/// Emit a RecordExtraction command via the writer channel.
/// Best-effort: drops silently if the channel is full or closed.
fn emit_extraction_metric(
    tx: &tokio::sync::mpsc::Sender<crate::server::writer::WriteCommand>,
    session_id: &str,
    memories_created: usize,
    tokens_in: u64,
    tokens_out: u64,
    cost_cents: u64,
    error: Option<&str>,
) {
    let _ = tx.try_send(crate::server::writer::WriteCommand::RecordExtraction {
        session_id: session_id.to_string(),
        memories_created,
        tokens_in,
        tokens_out,
        cost_cents,
        error: error.map(|s| s.to_string()),
    });
}
```

- [ ] **Step 4.11: Run tests — expect PASS**

Run: `cargo test -p forge-daemon --lib test_extractor_success_emits test_extractor_error_emits test_record_extraction`
Expected: PASS.

- [ ] **Step 4.12: Wire `emit_extraction_metric` into extractor success + error paths**

Open `crates/daemon/src/workers/extractor.rs`. Find:
1. The success path that calls `ops::remember()` (around line 744 per earlier audit) after a successful extraction batch. After the memory is stored, call:
   ```rust
   if let Some(tx) = &writer_tx {
       emit_extraction_metric(tx, &session_id, memories_created, tokens_in, tokens_out, cost_cents, None);
   }
   ```
2. The error path (around line 793-799 per earlier audit) that logs a stderr message. After the log, call:
   ```rust
   if let Some(tx) = &writer_tx {
       emit_extraction_metric(tx, &session_id, 0, 0, 0, 0, Some(&e.to_string()));
   }
   ```

**Field resolution**:
- `memories_created` — length of the memory vec returned from successful extraction.
- `tokens_in` / `tokens_out` / `cost_cents` — grab from the extraction result struct. If the struct doesn't carry them (yet), pass `0` and file a follow-up ticket. The spec prioritizes counter movement; token accuracy can come later.
- `session_id` — already in scope in the extractor batch loop.
- `writer_tx` — the extractor's handle. If the extractor doesn't currently own a writer_tx handle, we need to plumb one in (check the spawn call in `main.rs` — likely around the `new_writer` call).

If `writer_tx` plumbing is a big lift, **stop** and scope it as a separate task before continuing.

- [ ] **Step 4.13: Run full test suite**

Run: `cargo test --workspace`
Expected: all pass. If extractor integration tests fail due to the added emit, check that the test harness sets up a writer channel.

- [ ] **Step 4.14: `simplify`**

```
Skill: simplify
```

- [ ] **Step 4.15: Clippy**

Run: `cargo clippy --workspace -- -W clippy::all -D warnings`
Expected: 0 warnings.

- [ ] **Step 4.16: Stage**

```bash
git add crates/daemon/src/server/writer.rs \
        crates/daemon/src/db/metrics.rs \
        crates/daemon/src/db/mod.rs \
        crates/daemon/src/workers/extractor.rs
git status --short
```

- [ ] **Step 4.17: Adversarial Codex review**

Use the template. Specific probes:
- Is the `metrics` table schema assumption safe? Could another table migration break this?
- What happens if the extractor fires `emit_extraction_metric` before the writer actor has started? (Channel may not exist yet — `try_send` returns err, tick lost.)
- Is the error string sanitized before storage? (Potential log-poisoning vector if extraction errors include user input.)

Address BLOCKER + HIGH.

- [ ] **Step 4.18: Commit**

```bash
git commit -m "$(cat <<'EOF'
fix(extraction): record extraction metrics via writer channel (#53)

forge-next stats showed Extractions: 0 despite 244 messages + active
sessions because the extractor never wrote to the metrics table. ops.rs
already queries WHERE metric_type='extraction', but nothing INSERTed
those rows.

Added WriteCommand::RecordExtraction variant, writer match arm
(dispatches to new db::metrics::record_extraction), and emit_extraction_metric
helper wired into both success and error paths in workers/extractor.rs.
Error path writes with error=Some(e), so crash-loops are visible in 24h
counters instead of silently reading 0.

Closes SESSION-GAPS #53.
Spec: docs/superpowers/specs/2026-04-20-dark-loops-sp1-design.md §3.4

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Fix 5: Integration Test `e2e_sp1_dark_loops`

**One composite test that drives all 4 loops end-to-end on a real daemon harness.**

### Task 5: Integration test

**Files:**
- Create: `crates/daemon/tests/e2e_sp1_dark_loops.rs`.

- [ ] **Step 5.1: Write the integration test**

Create `crates/daemon/tests/e2e_sp1_dark_loops.rs`:

```rust
//! e2e_sp1_dark_loops — validate all 4 dark-loop fixes land together.
//!
//! Drives:
//!  - #55 skill registry auto-populate from tempdir
//!  - #45 proactive-context RecordInjection
//!  - #54 per-tool use_count increment
//!  - #53 extraction metric row insertion
//!
//! Uses the in-process DaemonState harness (same pattern as test_e2e_lifecycle.rs).

use std::fs;
use tempfile::tempdir;

#[test]
fn e2e_sp1_dark_loops_all_counters_advance() {
    // ===== Arrange =====

    // Temp skills dir with 3 fixtures
    let skills_dir = tempdir().unwrap();
    for name in ["loop-a", "loop-b", "loop-c"] {
        let p = skills_dir.path().join(name);
        fs::create_dir_all(&p).unwrap();
        fs::write(
            p.join("SKILL.md"),
            format!("---\nname: {name}\ndescription: test\ncategory: test\n---\n\n# {name}\n"),
        ).unwrap();
    }

    let db = tempdir().unwrap();
    let db_path = db.path().join("f.db").to_string_lossy().to_string();

    // Boot DaemonState (adapt to real constructor)
    let state = forge_daemon::server::handler::DaemonState::new(&db_path).unwrap();

    // ===== Act — #55: auto-populate skills =====
    let n = forge_daemon::skills::auto_populate_on_start(&state.conn, skills_dir.path()).unwrap();
    assert_eq!(n, 3, "#55: should auto-index 3 fixtures");

    // ===== Assert — #55 =====
    let skills = forge_daemon::skills::list_skills(&state.conn, None).unwrap();
    assert_eq!(skills.len(), 3, "#55: skills-list should return 3");

    // ===== Act — #45: proactive injection =====
    // Call record_proactive_injection helper with a fake session + fake injections
    // (the helper writes to the writer channel; under test we bypass via direct DB write,
    //  or via the test harness if available)
    //
    // Simplification: drive via DaemonState directly if the helper is pub(crate).
    // Otherwise use a minimal synthetic ProactiveInjection vec:
    let session_id = "test-sess-1";
    state.conn.execute(
        "INSERT INTO session (id, agent, started_at) VALUES (?1, 'test-agent', datetime('now'))",
        rusqlite::params![session_id],
    ).unwrap();

    // Simulate the RecordInjection that the helper would emit
    forge_daemon::db::effectiveness::record_injection_with_size(
        &state.conn,
        session_id,
        "PreBashChecked",
        "proactive",
        "skill:42",
        42,
    ).unwrap();

    // ===== Assert — #45 =====
    let inj_count: i64 = state.conn.query_row(
        "SELECT COUNT(*) FROM context_effectiveness WHERE context_type = 'proactive'",
        [], |r| r.get(0)
    ).unwrap();
    assert!(inj_count >= 1, "#45: proactive injection should be recorded");

    // ===== Act — #54: per-tool counter =====
    // Seed registry
    for (id, name) in [("bash", "Bash"), ("read", "Read")] {
        state.conn.execute(
            "INSERT INTO tool (id, name, kind, use_count) VALUES (?1, ?2, 'Cli', 0)",
            rusqlite::params![id, name],
        ).unwrap();
    }
    let transcript = "<tool_use name=\"Bash\"></tool_use><tool_use name=\"Bash\"></tool_use><tool_use name=\"Read\"></tool_use>";
    forge_daemon::workers::extractor::record_tool_uses_from_transcript(&state.conn, transcript).unwrap();

    // ===== Assert — #54 =====
    let bash_count: i64 = state.conn.query_row("SELECT use_count FROM tool WHERE id='bash'", [], |r| r.get(0)).unwrap();
    let read_count: i64 = state.conn.query_row("SELECT use_count FROM tool WHERE id='read'", [], |r| r.get(0)).unwrap();
    assert_eq!(bash_count, 2, "#54: Bash should be incremented to 2");
    assert_eq!(read_count, 1, "#54: Read should be incremented to 1");

    // ===== Act — #53: extraction metric =====
    forge_daemon::db::metrics::record_extraction(
        &state.conn, session_id, 7, 2000, 1000, 25, None,
    ).unwrap();

    // ===== Assert — #53 =====
    let extraction_rows: i64 = state.conn.query_row(
        "SELECT COUNT(*) FROM metrics WHERE metric_type='extraction' AND session_id = ?1",
        rusqlite::params![session_id], |r| r.get(0)
    ).unwrap();
    assert_eq!(extraction_rows, 1, "#53: extraction metric row should exist");
}
```

**Note**: depending on visibility, `forge_daemon::workers::extractor::record_tool_uses_from_transcript` may need `pub(crate)` exposure. Adjust accordingly when writing the Fix 3 helper.

- [ ] **Step 5.2: Run the test — expect PASS**

Run: `cargo test -p forge-daemon --test e2e_sp1_dark_loops`
Expected: PASS (all 4 fixes landed; test is pure validation).

If any assert fails, diagnose which fix didn't land correctly; amend the corresponding earlier fix commit (or add a hotfix commit) before proceeding.

- [ ] **Step 5.3: Full test suite**

Run: `cargo test --workspace`
Expected: all pass.

- [ ] **Step 5.4: `simplify` + Clippy + Adversarial Codex**

```
Skill: simplify
```

Run: `cargo clippy --workspace -- -W clippy::all -D warnings`

Adversarial Codex review using template:
```bash
codex exec <<'PROMPT'
Adversarial reviewer: this integration test claims to validate 4 dark-loop fixes.
What does it MISS?
1. Does it actually exercise the handler, or only the helpers?
2. Would a regression in writer-channel wiring be caught?
3. Are there concurrent-invocation scenarios untested?
4. Is the test flaky under parallel execution (shared DB name, shared tempdir)?
5. Does it verify Response shape or only DB state?

Diff:
$(git diff --staged)
PROMPT
```

Address BLOCKER+HIGH.

- [ ] **Step 5.5: Stage + commit**

```bash
git add crates/daemon/tests/e2e_sp1_dark_loops.rs
git commit -m "$(cat <<'EOF'
test(sp1): e2e_sp1_dark_loops integration test

Composite integration test validating all 4 SP1 fixes land together:
- #55 skill registry auto-populate (3 fixtures indexed)
- #45 proactive injection RecordInjection row
- #54 per-tool use_count increment (2 Bash + 1 Read)
- #53 extraction metric row with correct session_id + memory count

Uses in-process DaemonState harness (pattern from test_e2e_lifecycle.rs).
Deliberately bypasses full hook plumbing (that lives in Phase 2A-4c2);
validates the DB state shape that each fix commits produce.

Spec: docs/superpowers/specs/2026-04-20-dark-loops-sp1-design.md §6.2

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 6: Pre-PR Dogfood Validation

**Files:** none modified. Live-daemon verification.

- [ ] **Step 6.1: Rebuild daemon from HEAD of sp1/dark-loops**

Run:
```bash
cargo build --release --bin forge-daemon
cargo install --path crates/daemon --force
```

- [ ] **Step 6.2: Graceful restart**

Run: `forge-next restart`
Expected: drain in-flight, clean shutdown, auto-restart.

- [ ] **Step 6.3: Confirm skills registry populated**

Run: `forge-next skills-list`
Expected: non-zero count. Ideally 15+ (matching `forge-app-private/skills/` if that's the resolved dir).

If still 0, check daemon logs for the skill-auto-index log line; verify the env/cwd/home cascade resolved to a real path.

- [ ] **Step 6.4: Fire a proactive hook**

In a Claude Code session (or via curl to the daemon):
```bash
curl -sS -X POST http://127.0.0.1:8420/api -d '{
  "method": "pre_bash_checked",
  "params": { /* actual request shape — match handler */ }
}' | jq .
```

Then check:
```bash
forge-next context-stats
```
Expected: `Injections: >0` with at least one proactive entry.

- [ ] **Step 6.5: Trigger tool usage + wait for extractor cycle**

Run any real tool via Claude Code (or insert a synthetic transcript via a test endpoint). Wait for the extractor's next cycle (check logs for extractor tick).

Then:
```bash
forge-next tools | head -20
forge-next stats
```
Expected: at least one tool with `used: >0x`; `Extractions: N (E errors)` with N > 0.

- [ ] **Step 6.6: Record post-fix counter values**

Save output to PR description:
```bash
echo "=== POST-SP1 DOGFOOD ===" > /tmp/sp1-dogfood.txt
{
  echo "context-stats:"; forge-next context-stats
  echo; echo "stats:"; forge-next stats
  echo; echo "tools (first 10):"; forge-next tools | head -10
  echo; echo "skills-list:"; forge-next skills-list
} >> /tmp/sp1-dogfood.txt
cat /tmp/sp1-dogfood.txt
```

Compare against baseline from Task 0.7. All 4 counters should have moved.

- [ ] **Step 6.7: Decision gate**

If any counter still reads 0 despite fix + rebuild:
- **Do not open PR.** Debug the specific dark loop. Likely cause: wiring missed a call site, or the hook path being tested doesn't flow through our modified code.
- If counter > 0 for all 4: proceed to Task 7.

---

## Task 7: Final `simplify` pass on full PR diff

- [ ] **Step 7.1: Diff full PR**

Run: `git diff master..HEAD -- crates/ docs/`
Expected: see full PR diff across all 5 commits.

- [ ] **Step 7.2: Run `simplify` on full diff**

```
Skill: simplify
```
(Accept or reject suggestions that span commits. Only commit additional changes if truly beneficial; otherwise note in PR description.)

- [ ] **Step 7.3: If changes made, commit as a "polish" commit**

```bash
git add -u
git diff --staged --stat
git commit -m "chore(sp1): simplify polish pass on full PR diff"
```

Otherwise skip.

---

## Task 8: PR-level Adversarial Codex Review

- [ ] **Step 8.1: Run PR-level Codex review**

```bash
codex exec <<'PROMPT'
You are an adversarial reviewer. This is a 5-commit PR closing 4 bugs in
a production Rust daemon. Full diff below.

Probe these angles specifically:
1. What makes this regress in 3 months?
2. Are the counter fixes tick-accurate under load (no race where a hook
   fires + counter lost)?
3. Is the test genuinely e2e or does it shortcut the critical boundary?
4. Could the write-channel under-backpressure pattern cause silent data loss
   that masks extractor problems?
5. Is anything in this PR out of spec (compare against
   docs/superpowers/specs/2026-04-20-dark-loops-sp1-design.md)?
6. Are any deferred items (doctor probes, ops.rs changes, 2A-4c1 territory)
   accidentally included here?

Output: 5-10 concrete issues ranked BLOCKER > HIGH > MEDIUM > LOW. For each,
cite file:line and explain the failure mode. If you find no blockers, say so.

The full PR diff:
$(git diff master..HEAD)
PROMPT
```

- [ ] **Step 8.2: Triage findings**

- BLOCKER: must address or explicitly justify rejection in PR description. If addressed, new commit.
- HIGH: address or document as follow-up ticket.
- MEDIUM/LOW: log in PR description.

If any code changed, re-run `cargo test --workspace` + `cargo clippy` + commit.

---

## Task 9: Open PR

- [ ] **Step 9.1: Rebase on origin/master one more time**

Run:
```bash
git fetch origin master
git rebase origin/master
```

If conflicts: **stop**, coordinate manually.

- [ ] **Step 9.2: Push branch**

Run: `git push -u origin sp1/dark-loops`
Expected: push succeeds.

- [ ] **Step 9.3: Open PR with `gh`**

Run:
```bash
gh pr create --base master --head sp1/dark-loops \
  --title "SP1: Dark-loop closure (#45, #53, #54, #55)" \
  --body "$(cat <<'EOF'
## Summary

Closes 4 dark feedback loops in the daemon:
- **#45** proactive context injections recorded (was: 0 ever)
- **#53** extraction metrics row on every batch (was: 0 in 24h)
- **#54** per-tool `use_count` increments (was: 42 tools all 0x)
- **#55** skill_registry auto-populates on boot (was: empty table)

## Methodology

- Spec: `docs/superpowers/specs/2026-04-20-dark-loops-sp1-design.md`
- TDD red → green → simplify → Codex adversarial review → commit, per commit
- Full-PR `simplify` pass + PR-level adversarial Codex review before push
- Dogfood verified on live daemon (see below)

## Deferred from SP1

- Doctor probes (§6.3) — separate follow-up PR coordinated with 2026-04-16
  housekeeping + doctor-observability plan.
- #46 multi-tenant workers, #56 healing scheduler, #57 domain entities — in
  later sub-projects.

## Coordination

- **Phase 2A-4c1** active in parallel. No code conflict; touches different
  surfaces. See spec §11.1.
- **Did not touch** `crates/daemon/src/db/ops.rs` (2A-4c1 T3 in-flight).

## Dogfood

(paste /tmp/sp1-dogfood.txt from Task 6.6)

## Adversarial review

(paste highlights + actions from Task 8.2)

## Test plan

- [x] `cargo test --workspace` passing
- [x] `cargo clippy --workspace -- -W clippy::all -D warnings` clean
- [x] Integration test `e2e_sp1_dark_loops` passing
- [x] Dogfood on live daemon: all 4 counters moved

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
```

- [ ] **Step 9.4: Paste dogfood + adversarial sections into the PR body**

Edit the PR on GitHub or via `gh pr edit --body-file` with the real content from Tasks 6.6 + 8.2.

- [ ] **Step 9.5: Return PR URL**

Print the URL from `gh pr create` output so the user can review.

---

## Completion Criteria

SP1 is done when ALL of the following hold:

- [ ] 5 commits on `sp1/dark-loops` branch (fix #55, fix #45, fix #54, fix #53, integration test) + any review-fix commits.
- [ ] `cargo test --workspace` passing.
- [ ] `cargo clippy --workspace -- -W clippy::all -D warnings` clean.
- [ ] `e2e_sp1_dark_loops` integration test passing in isolation.
- [ ] Dogfood verified: `context-stats`, `stats`, `tools`, `skills-list` all non-zero on live daemon built from branch HEAD.
- [ ] PR opened against master with dogfood output + adversarial Codex summary in description.
- [ ] Zero modifications to `crates/daemon/src/db/ops.rs`, `crates/daemon/src/db/schema.rs`, or 2A-4c1 surfaces.
- [ ] Doctor probes NOT in this PR (deferred per spec §11.2).

## Notes for executor

- **If you hit a schema assumption that doesn't hold** (e.g., `metrics` table shape, `tool.id` slug convention): stop, verify against the live DB via `sqlite3 ~/.forge/forge.db '.schema <table>'`, then adjust the plan. Do NOT silently adapt — note the deviation in the commit message.
- **If 2A-4c1 lands while this is in flight**: rebase, resolve, re-run tests. Preserve both sets of changes.
- **If a Codex review surfaces a BLOCKER you cannot fix in 30 minutes**: stop, surface to the user, do not commit.
- **Test-harness limitations**: some tests above use in-process DaemonState shortcuts. If the real handler path requires more setup (e.g., writer actor spawn), adapt by following the precedent in `crates/daemon/tests/test_e2e_lifecycle.rs`.
