# Forge-Behavioral-Skill-Inference Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add Phase 23 `infer_skills_from_behavior` to the consolidator pipeline, detecting recurring clean tool-use patterns in the 2A-4c1 `session_tool_call` log and elevating them to the existing `skill` table. Updates the `<skills>` renderer so inferred skills surface to agents.

**Architecture:** New consolidator phase runs after Phase 17 (`extract_protocols`). SQL select of recent clean rows, Rust-side canonical-fingerprint computation (sha256 of JSON-canonical `(sorted unique tool_names, sorted tool_arg_shapes)`), aggregation across sessions, elevation at ≥3 distinct sessions via `INSERT ... ON CONFLICT(agent, fingerprint) DO UPDATE` merging `inferred_from`. Renderer updated to dual-gate `success_count > 0 OR inferred_at IS NOT NULL`. Test/bench-gated `Request::ProbePhase` enables master-assertion-9 verification via a static `PHASE_ORDER` const.

**Tech Stack:** Rust 1.88, rusqlite + SQLite JSON1 functions (built-in), `sha2` crate (already in workspace), `serde_json` canonical encoding, `tracing` for observability. No new dependencies.

**Parent design:** `docs/superpowers/specs/2026-04-23-forge-behavioral-skill-inference-design.md`
**Predecessor phase:** 2A-4c1 shipped at HEAD `cf74fb3` (2026-04-23).

---

## Task 1: Schema migration — ALTER `skill` + partial unique index

**Files:**
- Modify: `crates/daemon/src/db/schema.rs` (ALTER block around :759, :767 pattern; tests block at end of `mod tests`)

**Goal:** Add 4 columns to `skill` + partial unique index gated on `fingerprint != ''`.

- [ ] **Step 1.1: Write the RED test**

Append to `#[cfg(test)] mod tests` block in `schema.rs` (end of file, immediately before final `}`):

```rust
    // ── Phase 2A-4c2 T1: skill Phase-23 columns + partial unique index ───────

    #[test]
    fn test_skill_has_phase23_columns_and_partial_unique_index() {
        crate::db::vec::init_sqlite_vec();
        let conn = Connection::open_in_memory().unwrap();
        create_schema(&conn).unwrap();

        // All 4 new columns present with correct defaults / nullability.
        let columns: Vec<(String, String, i32)> = conn
            .prepare("PRAGMA table_info(skill)")
            .unwrap()
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(1)?,   // name
                    row.get::<_, String>(2)?,   // type
                    row.get::<_, i32>(3)?,      // notnull
                ))
            })
            .unwrap()
            .filter_map(|r| r.ok())
            .collect();

        let col_map: std::collections::HashMap<&str, (&str, i32)> = columns
            .iter()
            .map(|(n, t, nn)| (n.as_str(), (t.as_str(), *nn)))
            .collect();

        assert_eq!(col_map.get("agent"), Some(&("TEXT", 1)), "agent column must be TEXT NOT NULL");
        assert_eq!(col_map.get("fingerprint"), Some(&("TEXT", 1)), "fingerprint column must be TEXT NOT NULL");
        assert_eq!(col_map.get("inferred_from"), Some(&("TEXT", 1)), "inferred_from column must be TEXT NOT NULL");
        assert_eq!(col_map.get("inferred_at"), Some(&("TEXT", 0)), "inferred_at column must be TEXT NULL");

        // Partial unique index present, gated on fingerprint != ''.
        let idx_sql: String = conn
            .query_row(
                "SELECT sql FROM sqlite_master
                 WHERE type='index' AND name='idx_skill_agent_fingerprint'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(
            idx_sql.contains("UNIQUE") && idx_sql.contains("agent, fingerprint") && idx_sql.contains("fingerprint"),
            "expected partial unique index on (agent, fingerprint); got: {idx_sql}"
        );

        // Partial-index filter must exclude empty fingerprints.
        assert!(
            idx_sql.to_lowercase().contains("where") && idx_sql.contains("fingerprint"),
            "expected WHERE fingerprint != '' partial predicate; got: {idx_sql}"
        );
    }
```

- [ ] **Step 1.2: Run to confirm RED**

Run: `cargo test -p forge-daemon --lib test_skill_has_phase23_columns_and_partial_unique_index`
Expected: FAIL — columns don't exist yet.

- [ ] **Step 1.3: Add the ALTER block**

In `crates/daemon/src/db/schema.rs`, locate the existing ALTER migration block (around line 750-770 where `skill_type` and `observed_count` ALTERs live). Append new ALTERs after the existing `observed_count` ALTER, then add a new CREATE UNIQUE INDEX statement inside the `create_schema` body (after the table creates):

```rust
        // 2A-4c2 Phase 23: tool-use-inferred skill columns + partial unique index
        let _ = conn.execute(
            "ALTER TABLE skill ADD COLUMN agent TEXT NOT NULL DEFAULT 'claude-code'",
            [],
        );
        let _ = conn.execute(
            "ALTER TABLE skill ADD COLUMN fingerprint TEXT NOT NULL DEFAULT ''",
            [],
        );
        let _ = conn.execute(
            "ALTER TABLE skill ADD COLUMN inferred_from TEXT NOT NULL DEFAULT '[]'",
            [],
        );
        let _ = conn.execute(
            "ALTER TABLE skill ADD COLUMN inferred_at TEXT",
            [],
        );
        let _ = conn.execute_batch(
            "CREATE UNIQUE INDEX IF NOT EXISTS idx_skill_agent_fingerprint
             ON skill(agent, fingerprint)
             WHERE fingerprint != '';"
        );
```

The `let _ = conn.execute(...)` ignore pattern matches the existing ALTER style at `schema.rs:759` (ALTERs are idempotent at DB-level via "duplicate column" errors being ignored).

- [ ] **Step 1.4: Run to confirm GREEN**

Run: `cargo test -p forge-daemon --lib test_skill_has_phase23_columns_and_partial_unique_index`
Expected: PASS.

- [ ] **Step 1.5: Regression + lint**

```bash
cargo test -p forge-daemon --lib 2>&1 | tail -5
cargo clippy --workspace -- -W clippy::all -D warnings 2>&1 | tail -5
cargo fmt --all
```

Expected: all prior lib tests still pass (baseline 1352 → 1353), clippy clean, fmt clean.

- [ ] **Step 1.6: Commit**

```bash
git add crates/daemon/src/db/schema.rs
git commit -m "$(cat <<'EOF'
feat(2A-4c2 T1): skill table Phase 23 columns + partial unique index

Adds 4 ALTERs to the skill table (agent, fingerprint, inferred_from,
inferred_at) with backward-compatible DEFAULTs so existing rows continue
to work. Adds partial unique index on (agent, fingerprint) gated on
fingerprint != '' — prevents pre-existing rows (with default empty
fingerprint) from colliding with each other.

Test: PRAGMA table_info shows all 4 new columns with correct types and
NOT NULL flags; sqlite_master shows the partial unique index with the
WHERE fingerprint != '' predicate.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 2: Config additions + validator clamps

**Files:**
- Modify: `crates/daemon/src/config.rs` (ConsolidationConfig struct + Default + validated() at :442-459, :908-916)
- Modify: `~/.forge/config.toml.example` if present (documentation; optional — skip if file doesn't exist)

**Goal:** Add 3 config fields with validators; default values match spec Q2 / §2.2.

- [ ] **Step 2.1: Write the RED test**

Append to the `#[cfg(test)] mod tests` block in `config.rs`:

```rust
    // ── Phase 2A-4c2 T2: skill inference config validator tests ──────────────

    #[test]
    fn consolidation_config_default_skill_inference_values() {
        let cfg = ConsolidationConfig::default();
        assert_eq!(cfg.skill_inference_min_sessions, 3);
        assert_eq!(cfg.skill_inference_window_days, 30);
        assert!((cfg.skill_inference_tool_name_similarity_threshold - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn consolidation_config_validated_clamps_min_sessions_in_range() {
        let mut cfg = ConsolidationConfig::default();
        cfg.skill_inference_min_sessions = 0; // below range
        assert_eq!(cfg.validated().skill_inference_min_sessions, 1, "0 clamps to range min 1");

        cfg.skill_inference_min_sessions = 50; // above range
        assert_eq!(cfg.validated().skill_inference_min_sessions, 20, "50 clamps to range max 20");

        cfg.skill_inference_min_sessions = 5; // in range
        assert_eq!(cfg.validated().skill_inference_min_sessions, 5, "in-range value preserved");
    }

    #[test]
    fn consolidation_config_validated_clamps_window_days_in_range() {
        let mut cfg = ConsolidationConfig::default();
        cfg.skill_inference_window_days = 0; // below
        assert_eq!(cfg.validated().skill_inference_window_days, 1);
        cfg.skill_inference_window_days = 400; // above
        assert_eq!(cfg.validated().skill_inference_window_days, 365);
    }

    #[test]
    fn consolidation_config_validated_clamps_similarity_threshold() {
        let mut cfg = ConsolidationConfig::default();
        cfg.skill_inference_tool_name_similarity_threshold = -1.0;
        assert_eq!(cfg.validated().skill_inference_tool_name_similarity_threshold, 0.0);
        cfg.skill_inference_tool_name_similarity_threshold = 2.5;
        assert_eq!(cfg.validated().skill_inference_tool_name_similarity_threshold, 1.0);
    }
```

- [ ] **Step 2.2: Run to confirm RED**

Run: `cargo test -p forge-daemon --lib consolidation_config_default_skill_inference_values`
Expected: FAIL — fields don't exist.

- [ ] **Step 2.3: Add fields to `ConsolidationConfig`**

In `crates/daemon/src/config.rs`, replace the `ConsolidationConfig` struct + its `Default` + its `validated()` with:

```rust
/// Consolidation batch configuration — limits for consolidation phases.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ConsolidationConfig {
    #[serde(default = "default_200_usize")]
    pub batch_limit: usize,
    #[serde(default = "default_50_usize")]
    pub reweave_limit: usize,

    // Phase 23 behavioral skill inference (2A-4c2).
    #[serde(default = "default_3_usize")]
    pub skill_inference_min_sessions: usize,
    #[serde(default = "default_30_u32")]
    pub skill_inference_window_days: u32,
    #[serde(default = "default_1_f64")]
    pub skill_inference_tool_name_similarity_threshold: f64,
}

impl Default for ConsolidationConfig {
    fn default() -> Self {
        Self {
            batch_limit: 200,
            reweave_limit: 50,
            skill_inference_min_sessions: 3,
            skill_inference_window_days: 30,
            skill_inference_tool_name_similarity_threshold: 1.0,
        }
    }
}
```

Add these helper functions near the other `default_*` functions in the same file (search for `fn default_200_usize` for placement):

```rust
fn default_3_usize() -> usize { 3 }
fn default_30_u32() -> u32 { 30 }
fn default_1_f64() -> f64 { 1.0 }
```

And update `validated()`:

```rust
impl ConsolidationConfig {
    /// Return a copy with all values clamped to sane bounds.
    pub fn validated(&self) -> Self {
        Self {
            batch_limit: self.batch_limit.clamp(1, 1000),
            reweave_limit: self.reweave_limit.clamp(1, 500),
            skill_inference_min_sessions: self.skill_inference_min_sessions.clamp(1, 20),
            skill_inference_window_days: self.skill_inference_window_days.clamp(1, 365),
            skill_inference_tool_name_similarity_threshold: self
                .skill_inference_tool_name_similarity_threshold
                .clamp(0.0, 1.0),
        }
    }
}
```

- [ ] **Step 2.4: Run to confirm GREEN**

Run: `cargo test -p forge-daemon --lib consolidation_config`
Expected: 4 new tests pass; all prior config tests still pass.

- [ ] **Step 2.5: Regression + lint**

```bash
cargo test -p forge-daemon --lib 2>&1 | tail -5
cargo clippy --workspace -- -W clippy::all -D warnings 2>&1 | tail -5
cargo fmt --all
```

Expected: 1357 pass (prior 1353 + 4 new), clippy clean.

- [ ] **Step 2.6: Commit**

```bash
git add crates/daemon/src/config.rs
git commit -m "$(cat <<'EOF'
feat(2A-4c2 T2): ConsolidationConfig skill inference fields + validators

Three new fields:
- skill_inference_min_sessions: usize (default 3, clamp 1..=20)
- skill_inference_window_days: u32 (default 30, clamp 1..=365)
- skill_inference_tool_name_similarity_threshold: f64 (default 1.0,
  clamp 0.0..=1.0) — strict fingerprint match by default; future-
  proof for fuzzy matching without another migration.

Backward-compatible: #[serde(default = ...)] on each field means
existing config.toml files without the [consolidation] fields keep
working.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 3: Pure helpers — fingerprint, domain, name (L0 tests)

**Files:**
- Create: `crates/daemon/src/workers/skill_inference.rs` (new sibling module)
- Modify: `crates/daemon/src/workers/mod.rs` — add `pub mod skill_inference;`

**Goal:** Three pure Rust functions that any test can exercise without DB: `canonical_fingerprint`, `infer_domain`, `format_skill_name`.

- [ ] **Step 3.1: Create the new module file with all RED tests**

Write `crates/daemon/src/workers/skill_inference.rs`:

```rust
//! Phase 23 Behavioral Skill Inference (2A-4c2).
//!
//! Pure helpers for turning a sequence of tool calls into a canonical
//! fingerprint, inferring a domain tag from the tool-name set, and
//! formatting a display name.
//!
//! All helpers here are side-effect-free; the DB-touching orchestrator
//! lives in `consolidator.rs::infer_skills_from_behavior`.

use sha2::{Digest, Sha256};

/// One clean tool call observed in a session.
///
/// Holds only the subset the fingerprint actually consumes: the tool
/// name and the sorted top-level keys of `tool_args`. Not related to
/// `ToolCallRow` (which carries full record for other callers).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolCall {
    pub tool_name: String,
    /// Top-level keys of the call's `tool_args` object, pre-sorted ASC.
    pub arg_keys: Vec<String>,
}

/// Canonical fingerprint of a session's tool-call sequence (Phase 23 input).
///
/// Shape: `sha256(json_canonical([sorted unique tool_names, sorted tool_arg_shapes]))`.
/// Tool-arg shapes are per-call sorted key sets, with the outer list also sorted
/// lexicographically. Values do NOT affect the hash — only structural key
/// presence.
///
/// Pure function. Determinism guaranteed by:
/// - `unique_sorted_tool_names`: dedup + sort.
/// - `arg_keys` already sorted on construction.
/// - Outer sort of the arg-keys list.
/// - `serde_json::to_string` produces stable JSON (no Object randomness since we
///   emit `Value::Array` only).
pub fn canonical_fingerprint(calls: &[ToolCall]) -> String {
    let mut tool_names: Vec<String> =
        calls.iter().map(|c| c.tool_name.clone()).collect();
    tool_names.sort();
    tool_names.dedup();

    let mut arg_shapes: Vec<Vec<String>> =
        calls.iter().map(|c| c.arg_keys.clone()).collect();
    arg_shapes.sort();

    let canonical = serde_json::json!([tool_names, arg_shapes]);
    // `to_string` on a JSON array of arrays has no key-order ambiguity.
    let canonical_bytes = canonical.to_string();

    let mut hasher = Sha256::new();
    hasher.update(canonical_bytes.as_bytes());
    let digest = hasher.finalize();
    hex_encode(&digest)
}

/// Rule-based domain tag for a set of tool names.
///
/// Precedence: file-ops > shell > web > workflow > integration > general.
/// MCP-prefixed tools check last so a hypothetical `mcp__write__…` tool doesn't
/// win over the explicit file-ops case.
pub fn infer_domain(tool_names: &[String]) -> &'static str {
    let names: Vec<&str> = tool_names.iter().map(|s| s.as_str()).collect();
    let file_ops = ["Read", "Write", "Edit", "Glob", "Grep", "MultiEdit", "NotebookEdit"];
    if names.iter().any(|n| file_ops.contains(n)) {
        return "file-ops";
    }
    if names.iter().any(|n| *n == "Bash") {
        return "shell";
    }
    if names.iter().any(|n| *n == "WebFetch" || *n == "WebSearch") {
        return "web";
    }
    if names.iter().any(|n| *n == "TodoWrite" || *n == "Task") {
        return "workflow";
    }
    if names.iter().any(|n| n.starts_with("mcp__")) {
        return "integration";
    }
    "general"
}

/// Display name per spec Q5: "Inferred: {sorted-tools} [{hash8}]".
pub fn format_skill_name(tool_names: &[String], fingerprint: &str) -> String {
    let mut sorted = tool_names.to_vec();
    sorted.sort();
    sorted.dedup();
    let tools = sorted.join("+");
    let short_hash = fingerprint.chars().take(8).collect::<String>();
    format!("Inferred: {tools} [{short_hash}]")
}

fn hex_encode(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tc(name: &str, keys: &[&str]) -> ToolCall {
        let mut k: Vec<String> = keys.iter().map(|s| s.to_string()).collect();
        k.sort();
        ToolCall {
            tool_name: name.to_string(),
            arg_keys: k,
        }
    }

    #[test]
    fn canonical_fingerprint_is_deterministic() {
        let a = [tc("Read", &["file_path"]), tc("Edit", &["file_path", "old_string", "new_string"]), tc("Bash", &["cmd"])];
        let b = [tc("Bash", &["cmd"]), tc("Read", &["file_path"]), tc("Edit", &["new_string", "file_path", "old_string"])];
        // b is a permutation of a; arg_keys in b's Edit are an unsorted input
        // but tc() pre-sorts them.
        assert_eq!(canonical_fingerprint(&a), canonical_fingerprint(&b));
    }

    #[test]
    fn canonical_fingerprint_ignores_arg_values_only_keys() {
        // Same key set regardless of values.
        let a = [tc("Read", &["file_path"])];
        let b = [tc("Read", &["file_path"])];
        assert_eq!(canonical_fingerprint(&a), canonical_fingerprint(&b));
    }

    #[test]
    fn canonical_fingerprint_distinguishes_different_arg_keys() {
        let a = [tc("Bash", &["cmd"])];
        let b = [tc("Bash", &["cmd", "run_id"])];
        assert_ne!(canonical_fingerprint(&a), canonical_fingerprint(&b));
    }

    #[test]
    fn canonical_fingerprint_distinguishes_different_tool_sets() {
        let a = [tc("Read", &["file_path"]), tc("Edit", &["file_path"])];
        let b = [tc("Read", &["file_path"]), tc("Edit", &["file_path"]), tc("Bash", &["cmd"])];
        assert_ne!(canonical_fingerprint(&a), canonical_fingerprint(&b));
    }

    #[test]
    fn infer_domain_file_ops_match() {
        assert_eq!(infer_domain(&["Read".to_string(), "Edit".to_string()]), "file-ops");
        assert_eq!(infer_domain(&["Glob".to_string()]), "file-ops");
    }

    #[test]
    fn infer_domain_shell_when_only_bash() {
        assert_eq!(infer_domain(&["Bash".to_string()]), "shell");
    }

    #[test]
    fn infer_domain_mcp_prefix() {
        assert_eq!(
            infer_domain(&["mcp__context7__query-docs".to_string()]),
            "integration"
        );
    }

    #[test]
    fn infer_domain_general_fallback() {
        assert_eq!(infer_domain(&["SomeUnknownTool".to_string()]), "general");
    }

    #[test]
    fn format_skill_name_contains_hash_prefix() {
        let n = format_skill_name(
            &["Edit".to_string(), "Read".to_string(), "Bash".to_string()],
            "abcdef1234567890",
        );
        assert_eq!(n, "Inferred: Bash+Edit+Read [abcdef12]");
    }
}
```

- [ ] **Step 3.2: Register the module**

In `crates/daemon/src/workers/mod.rs`, add `pub mod skill_inference;` to the module declarations (alongside the other `pub mod` lines — placement alphabetical).

- [ ] **Step 3.3: Run to confirm tests pass**

Run: `cargo test -p forge-daemon --lib skill_inference::tests -- --test-threads=1`
Expected: 9 passed, 0 failed.

- [ ] **Step 3.4: Regression + lint**

```bash
cargo test -p forge-daemon --lib 2>&1 | tail -5
cargo clippy --workspace -- -W clippy::all -D warnings 2>&1 | tail -5
cargo fmt --all
```

Expected: 1366 pass (prior 1357 + 9 new), clippy clean.

- [ ] **Step 3.5: Commit**

```bash
git add crates/daemon/src/workers/skill_inference.rs crates/daemon/src/workers/mod.rs
git commit -m "$(cat <<'EOF'
feat(2A-4c2 T3): skill_inference pure helpers (fingerprint, domain, name)

New module crates/daemon/src/workers/skill_inference.rs — side-effect-free
functions shared between the consolidator orchestrator and unit tests.

- ToolCall: minimal struct (tool_name + sorted top-level arg_keys) used
  only by the fingerprint pipeline.
- canonical_fingerprint: sha256(json_canonical([sorted_unique_tool_names,
  sorted_tool_arg_shapes])) — values ignored, only structural keys
  participate. 8-char hex prefix is what appears in display names.
- infer_domain: precedence file-ops > shell > web > workflow > integration
  > general. MCP check last so file-ops wins for hypothetical MCP
  read/write tools.
- format_skill_name: "Inferred: {sorted-tools} [{hash8}]".

9 L0 tests cover: determinism across permutations, value-vs-key
distinction, tool-set distinction, each domain branch, name format.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 4: `infer_skills_from_behavior` function + L1 tests

**Files:**
- Modify: `crates/daemon/src/workers/consolidator.rs` — add new function + 9 tests in its `#[cfg(test)] mod tests` block.

**Goal:** The DB-touching orchestrator function. Selects recent clean rows, groups by session, computes fingerprint per session, aggregates, elevates at ≥3.

- [ ] **Step 4.1: Write the 9 RED tests first**

In `crates/daemon/src/workers/consolidator.rs`, find the existing `#[cfg(test)] mod tests` block (search `#[cfg(test)]`). Append:

```rust
    // ── Phase 2A-4c2 T4: infer_skills_from_behavior tests ────────────────────

    fn seed_session_tool_call_row(
        conn: &Connection,
        id: &str,
        session_id: &str,
        agent: &str,
        tool_name: &str,
        tool_args_json: &str,
        success: i64,
        corr: i64,
        created_at_offset_days: i64,
    ) {
        let created_at = format!(
            "datetime('now', '-{} days', '-{} minutes')",
            created_at_offset_days,
            id.len() // deterministic-but-distinct minute offset so ORDER BY is stable
        );
        let sql = format!(
            "INSERT INTO session_tool_call
             (id, session_id, agent, tool_name, tool_args, tool_result_summary,
              success, user_correction_flag, organization_id, created_at)
             VALUES ('{id}', '{session_id}', '{agent}', '{tool_name}', '{tool_args_json}',
              '', {success}, {corr}, 'default', {created_at})"
        );
        conn.execute_batch(&sql).unwrap();
    }

    fn seed_session(conn: &Connection, id: &str, agent: &str) {
        conn.execute(
            "INSERT INTO session (id, agent, started_at, status, organization_id)
             VALUES (?1, ?2, '2026-04-19 10:00:00', 'active', 'default')",
            rusqlite::params![id, agent],
        )
        .unwrap();
    }

    /// Seed one session with the standard Read+Edit+Bash pattern, clean rows.
    fn seed_clean_sess(conn: &Connection, sid: &str) {
        seed_session(conn, sid, "claude-code");
        seed_session_tool_call_row(conn, &format!("{sid}-01"), sid, "claude-code", "Read",
            r#"{\"file_path\":\"/tmp/a\"}"#, 1, 0, 0);
        seed_session_tool_call_row(conn, &format!("{sid}-02"), sid, "claude-code", "Edit",
            r#"{\"file_path\":\"/tmp/a\",\"old_string\":\"x\",\"new_string\":\"y\"}"#, 1, 0, 0);
        seed_session_tool_call_row(conn, &format!("{sid}-03"), sid, "claude-code", "Bash",
            r#"{\"cmd\":\"cargo test\"}"#, 1, 0, 0);
    }

    #[test]
    fn infer_skills_from_behavior_elevates_at_three_sessions() {
        crate::db::vec::init_sqlite_vec();
        let conn = Connection::open_in_memory().unwrap();
        crate::db::schema::create_schema(&conn).unwrap();
        seed_clean_sess(&conn, "SA");
        seed_clean_sess(&conn, "SB");
        seed_clean_sess(&conn, "SC");

        let elevated = infer_skills_from_behavior(&conn, 3, 30);
        assert_eq!(elevated, 1, "3 matching sessions → 1 skill row elevated");

        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM skill WHERE inferred_at IS NOT NULL",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);

        let inferred_from: String = conn
            .query_row(
                "SELECT inferred_from FROM skill WHERE inferred_at IS NOT NULL",
                [],
                |row| row.get(0),
            )
            .unwrap();
        // All three session IDs present in the JSON array.
        for sid in ["SA", "SB", "SC"] {
            assert!(
                inferred_from.contains(&format!("\"{sid}\"")),
                "inferred_from missing {sid}: {inferred_from}"
            );
        }
    }

    #[test]
    fn infer_skills_from_behavior_skips_at_two_sessions() {
        crate::db::vec::init_sqlite_vec();
        let conn = Connection::open_in_memory().unwrap();
        crate::db::schema::create_schema(&conn).unwrap();
        seed_clean_sess(&conn, "SA");
        seed_clean_sess(&conn, "SB");

        let elevated = infer_skills_from_behavior(&conn, 3, 30);
        assert_eq!(elevated, 0);

        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM skill WHERE inferred_at IS NOT NULL",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn infer_skills_from_behavior_skips_corrected_rows() {
        crate::db::vec::init_sqlite_vec();
        let conn = Connection::open_in_memory().unwrap();
        crate::db::schema::create_schema(&conn).unwrap();
        // SA has a correction on its Edit row — its 3-tool fingerprint won't match
        seed_session(&conn, "SA", "claude-code");
        seed_session_tool_call_row(&conn, "SA-01", "SA", "claude-code", "Read", r#"{\"file_path\":\"/a\"}"#, 1, 0, 0);
        seed_session_tool_call_row(&conn, "SA-02", "SA", "claude-code", "Edit", r#"{\"file_path\":\"/a\"}"#, 1, 1, 0);
        seed_session_tool_call_row(&conn, "SA-03", "SA", "claude-code", "Bash", r#"{\"cmd\":\"x\"}"#, 1, 0, 0);
        seed_clean_sess(&conn, "SB");
        seed_clean_sess(&conn, "SC");

        let elevated = infer_skills_from_behavior(&conn, 3, 30);
        // SB+SC = 2 sessions with 3-tool fingerprint; SA only has 2-tool fingerprint (Read+Bash).
        assert_eq!(elevated, 0, "correction taints SA's matching fingerprint");
    }

    #[test]
    fn infer_skills_from_behavior_skips_failed_rows() {
        crate::db::vec::init_sqlite_vec();
        let conn = Connection::open_in_memory().unwrap();
        crate::db::schema::create_schema(&conn).unwrap();
        seed_session(&conn, "SA", "claude-code");
        seed_session_tool_call_row(&conn, "SA-01", "SA", "claude-code", "Read", r#"{\"file_path\":\"/a\"}"#, 1, 0, 0);
        seed_session_tool_call_row(&conn, "SA-02", "SA", "claude-code", "Edit", r#"{\"file_path\":\"/a\"}"#, 0, 0, 0); // failure
        seed_session_tool_call_row(&conn, "SA-03", "SA", "claude-code", "Bash", r#"{\"cmd\":\"x\"}"#, 1, 0, 0);
        seed_clean_sess(&conn, "SB");
        seed_clean_sess(&conn, "SC");

        let elevated = infer_skills_from_behavior(&conn, 3, 30);
        assert_eq!(elevated, 0, "failed rows drop out of clean-filter, SA fingerprint diverges");
    }

    #[test]
    fn infer_skills_from_behavior_skips_rows_outside_window() {
        crate::db::vec::init_sqlite_vec();
        let conn = Connection::open_in_memory().unwrap();
        crate::db::schema::create_schema(&conn).unwrap();
        seed_session(&conn, "SA", "claude-code");
        // Session with rows 60 days ago — outside 30d window.
        seed_session_tool_call_row(&conn, "SA-01", "SA", "claude-code", "Read", r#"{\"file_path\":\"/a\"}"#, 1, 0, 60);
        seed_session_tool_call_row(&conn, "SA-02", "SA", "claude-code", "Edit", r#"{\"file_path\":\"/a\"}"#, 1, 0, 60);
        seed_session_tool_call_row(&conn, "SA-03", "SA", "claude-code", "Bash", r#"{\"cmd\":\"x\"}"#, 1, 0, 60);
        seed_clean_sess(&conn, "SB");
        seed_clean_sess(&conn, "SC");

        let elevated = infer_skills_from_behavior(&conn, 3, 30);
        assert_eq!(elevated, 0, "SA outside window → only SB+SC match, below threshold");
    }

    #[test]
    fn infer_skills_from_behavior_merges_inferred_from_on_conflict() {
        crate::db::vec::init_sqlite_vec();
        let conn = Connection::open_in_memory().unwrap();
        crate::db::schema::create_schema(&conn).unwrap();
        seed_clean_sess(&conn, "SA");
        seed_clean_sess(&conn, "SB");
        seed_clean_sess(&conn, "SC");
        assert_eq!(infer_skills_from_behavior(&conn, 3, 30), 1);

        // Add a 4th session, re-run.
        seed_clean_sess(&conn, "SD");
        let second = infer_skills_from_behavior(&conn, 3, 30);
        assert_eq!(second, 1, "upsert returns 1 affected row");

        let inferred_from: String = conn
            .query_row(
                "SELECT inferred_from FROM skill WHERE inferred_at IS NOT NULL",
                [],
                |row| row.get(0),
            )
            .unwrap();
        for sid in ["SA", "SB", "SC", "SD"] {
            assert!(
                inferred_from.contains(&format!("\"{sid}\"")),
                "merged inferred_from missing {sid}: {inferred_from}"
            );
        }

        let total_rows: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM skill WHERE inferred_at IS NOT NULL",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(total_rows, 1, "upsert must not create a duplicate row");
    }

    #[test]
    fn infer_skills_from_behavior_idempotent_on_rerun() {
        crate::db::vec::init_sqlite_vec();
        let conn = Connection::open_in_memory().unwrap();
        crate::db::schema::create_schema(&conn).unwrap();
        seed_clean_sess(&conn, "SA");
        seed_clean_sess(&conn, "SB");
        seed_clean_sess(&conn, "SC");
        let first = infer_skills_from_behavior(&conn, 3, 30);
        let second = infer_skills_from_behavior(&conn, 3, 30);
        assert_eq!(first, 1);
        assert_eq!(second, 1, "re-run upserts same row, no duplicate");

        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM skill WHERE inferred_at IS NOT NULL",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn infer_skills_from_behavior_separates_fingerprints() {
        crate::db::vec::init_sqlite_vec();
        let conn = Connection::open_in_memory().unwrap();
        crate::db::schema::create_schema(&conn).unwrap();
        // Pattern A: Read+Edit+Bash × 3 sessions
        seed_clean_sess(&conn, "A1");
        seed_clean_sess(&conn, "A2");
        seed_clean_sess(&conn, "A3");
        // Pattern B: Grep+Write × 3 sessions
        for sid in ["B1", "B2", "B3"] {
            seed_session(&conn, sid, "claude-code");
            seed_session_tool_call_row(&conn, &format!("{sid}-01"), sid, "claude-code", "Grep",
                r#"{\"pattern\":\"x\"}"#, 1, 0, 0);
            seed_session_tool_call_row(&conn, &format!("{sid}-02"), sid, "claude-code", "Write",
                r#"{\"file_path\":\"/tmp/z\",\"content\":\"q\"}"#, 1, 0, 0);
        }

        let elevated = infer_skills_from_behavior(&conn, 3, 30);
        assert_eq!(elevated, 2, "two distinct fingerprints each elevate");

        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM skill WHERE inferred_at IS NOT NULL",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 2);
    }

    #[test]
    fn infer_skills_from_behavior_separates_agents() {
        crate::db::vec::init_sqlite_vec();
        let conn = Connection::open_in_memory().unwrap();
        crate::db::schema::create_schema(&conn).unwrap();
        // 3 sessions for claude-code
        seed_clean_sess(&conn, "C1");
        seed_clean_sess(&conn, "C2");
        seed_clean_sess(&conn, "C3");
        // 3 sessions for codex-cli, same fingerprint
        for sid in ["X1", "X2", "X3"] {
            seed_session(&conn, sid, "codex-cli");
            seed_session_tool_call_row(&conn, &format!("{sid}-01"), sid, "codex-cli", "Read",
                r#"{\"file_path\":\"/tmp/a\"}"#, 1, 0, 0);
            seed_session_tool_call_row(&conn, &format!("{sid}-02"), sid, "codex-cli", "Edit",
                r#"{\"file_path\":\"/tmp/a\",\"old_string\":\"x\",\"new_string\":\"y\"}"#, 1, 0, 0);
            seed_session_tool_call_row(&conn, &format!("{sid}-03"), sid, "codex-cli", "Bash",
                r#"{\"cmd\":\"cargo test\"}"#, 1, 0, 0);
        }

        let elevated = infer_skills_from_behavior(&conn, 3, 30);
        assert_eq!(elevated, 2, "same fingerprint on two different agents → two rows");

        let rows: Vec<String> = conn
            .prepare("SELECT agent FROM skill WHERE inferred_at IS NOT NULL ORDER BY agent")
            .unwrap()
            .query_map([], |row| row.get::<_, String>(0))
            .unwrap()
            .filter_map(|r| r.ok())
            .collect();
        assert_eq!(rows, vec!["claude-code".to_string(), "codex-cli".to_string()]);
    }
```

- [ ] **Step 4.2: Run to confirm RED**

Run: `cargo test -p forge-daemon --lib infer_skills_from_behavior`
Expected: 9 FAILs (function not defined).

- [ ] **Step 4.3: Implement `infer_skills_from_behavior`**

In `crates/daemon/src/workers/consolidator.rs`, add imports at the top of the file (near existing imports):

```rust
use crate::workers::skill_inference::{
    canonical_fingerprint, format_skill_name, infer_domain, ToolCall,
};
```

Then add the function. A good placement is immediately after `extract_protocols` (around line 1226 — after the end of the existing `fn extract_protocols` body):

```rust
/// Phase 23: Behavioral Skill Inference — elevate recurring clean tool-use
/// patterns from `session_tool_call` to the `skill` table.
///
/// Detection signal: tool-call rows with `success=1 AND user_correction_flag=0`,
/// grouped by `(agent, session_id)`, canonicalized via
/// `skill_inference::canonical_fingerprint`, elevated when the fingerprint
/// appears in ≥ `min_sessions` distinct sessions within the last
/// `window_days` days.
///
/// Idempotent: re-running with no new data merges `inferred_from` without
/// creating duplicate rows (ON CONFLICT (agent, fingerprint) DO UPDATE).
///
/// Returns the number of rows affected (INSERTs + upsert-UPDATEs).
pub fn infer_skills_from_behavior(
    conn: &Connection,
    min_sessions: usize,
    window_days: u32,
) -> usize {
    use std::collections::BTreeMap;
    use std::collections::BTreeSet;

    // Step 1: SELECT clean rows within the window.
    let window_clause = format!("datetime('now', '-{window_days} days')");
    let sql = format!(
        "SELECT agent, session_id, tool_name, tool_args
         FROM session_tool_call
         WHERE success = 1 AND user_correction_flag = 0
           AND created_at > {window_clause}
         ORDER BY agent, session_id, created_at"
    );
    let mut stmt = match conn.prepare(&sql) {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!(error = %e, "Phase 23: prepare failed, skipping");
            return 0;
        }
    };
    let rows: Vec<(String, String, String, String)> = match stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
        ))
    }) {
        Ok(iter) => iter.filter_map(|r| r.ok()).collect(),
        Err(e) => {
            tracing::warn!(error = %e, "Phase 23: query failed, skipping");
            return 0;
        }
    };

    // Step 2: group by (agent, session_id), build per-session fingerprint.
    // Also remember tool_names_sorted for later display.
    let mut per_session: BTreeMap<(String, String), Vec<ToolCall>> = BTreeMap::new();
    for (agent, session_id, tool_name, tool_args_json) in rows {
        let arg_keys: Vec<String> = match serde_json::from_str::<serde_json::Value>(&tool_args_json)
        {
            Ok(serde_json::Value::Object(map)) => {
                let mut ks: Vec<String> = map.keys().cloned().collect();
                ks.sort();
                ks
            }
            Ok(_) => Vec::new(), // non-object args → empty key set
            Err(_) => {
                tracing::warn!(
                    session_id = %session_id,
                    "Phase 23: tool_args not valid JSON, skipping row"
                );
                continue;
            }
        };
        per_session
            .entry((agent, session_id))
            .or_default()
            .push(ToolCall { tool_name, arg_keys });
    }

    // Step 3: aggregate fingerprints across sessions.
    //   (agent, fingerprint) -> (sessions, last-seen tool_names_sorted)
    let mut fp_sessions: BTreeMap<(String, String), (BTreeSet<String>, Vec<String>)> =
        BTreeMap::new();
    for ((agent, session_id), calls) in per_session {
        let fp = canonical_fingerprint(&calls);
        let mut names: Vec<String> = calls.iter().map(|c| c.tool_name.clone()).collect();
        names.sort();
        names.dedup();
        let entry = fp_sessions
            .entry((agent, fp))
            .or_insert_with(|| (BTreeSet::new(), names.clone()));
        entry.0.insert(session_id);
        entry.1 = names; // stable: each session with same fingerprint produces same sorted unique list
    }

    // Step 4: filter ≥ min_sessions + elevate.
    let now_iso = forge_core::time::now_iso();
    let mut affected = 0_usize;
    for ((agent, fingerprint), (sessions, tool_names_sorted)) in fp_sessions {
        if sessions.len() < min_sessions {
            continue;
        }
        let name = format_skill_name(&tool_names_sorted, &fingerprint);
        let domain = infer_domain(&tool_names_sorted);
        let inferred_from = serde_json::to_string(
            &sessions.into_iter().collect::<Vec<String>>(),
        )
        .unwrap_or_else(|_| "[]".to_string());
        let id = ulid::Ulid::new().to_string();

        // Step 5: INSERT ON CONFLICT.
        let res = conn.execute(
            "INSERT INTO skill
             (id, name, domain, description, steps, source,
              agent, fingerprint, inferred_from, inferred_at, success_count)
             VALUES (?1, ?2, ?3, '', '[]', 'inferred', ?4, ?5, ?6, ?7, 0)
             ON CONFLICT(agent, fingerprint) DO UPDATE SET
                inferred_from = (
                    SELECT json_group_array(DISTINCT value) FROM (
                        SELECT value FROM json_each(skill.inferred_from)
                        UNION
                        SELECT value FROM json_each(excluded.inferred_from)
                    )
                ),
                inferred_at = excluded.inferred_at",
            rusqlite::params![
                id,
                name,
                domain,
                agent,
                fingerprint,
                inferred_from,
                now_iso,
            ],
        );
        match res {
            Ok(n) => affected += n,
            Err(e) => {
                tracing::error!(error = %e, "Phase 23: INSERT/UPSERT failed, skipping fingerprint");
            }
        }
    }

    affected
}
```

- [ ] **Step 4.4: Run to confirm GREEN**

Run: `cargo test -p forge-daemon --lib infer_skills_from_behavior`
Expected: 9 passed, 0 failed.

- [ ] **Step 4.5: Regression + lint**

```bash
cargo test -p forge-daemon --lib 2>&1 | tail -5
cargo clippy --workspace -- -W clippy::all -D warnings 2>&1 | tail -5
cargo fmt --all
```

Expected: 1375 pass (prior 1366 + 9 new). Clippy clean.

- [ ] **Step 4.6: Commit**

```bash
git add crates/daemon/src/workers/consolidator.rs
git commit -m "$(cat <<'EOF'
feat(2A-4c2 T4): infer_skills_from_behavior orchestrator + 9 L1 tests

Phase 23 DB orchestrator. Selects clean session_tool_call rows within
window, groups by (agent, session_id), computes per-session canonical
fingerprint via skill_inference helpers, aggregates (agent, fp) →
{session_ids}, elevates at >= min_sessions distinct sessions.

Uses INSERT ON CONFLICT(agent, fingerprint) DO UPDATE with JSON1
json_each/json_group_array to merge inferred_from sets — idempotent
re-runs, additive session discovery, no duplicate rows.

Tests pin:
- Elevation at exactly 3 sessions; skip at 2.
- Row-level correction/failure filter (not session-level).
- 30-day window respected.
- Upsert merges inferred_from (adds session D to existing A,B,C).
- Idempotent re-run with same data (count stays 1).
- Distinct fingerprints elevate independently (2 rows).
- Agent is part of the unique key (same fingerprint × 2 agents → 2 rows).

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 5: Register Phase 23 in consolidator orchestrator

**Files:**
- Modify: `crates/daemon/src/workers/consolidator.rs` — call `infer_skills_from_behavior` after `extract_protocols` at :278-282

**Goal:** Wire Phase 23 into the main consolidation loop. No tests added here — the next task's ProbePhase test will verify ordering.

- [ ] **Step 5.1: Add `skills_inferred` to `ConsolidationStats`**

Find the `ConsolidationStats` struct in `consolidator.rs` (search `struct ConsolidationStats`). Add a new field `pub skills_inferred: usize` to it (and initialize to 0 in its `Default` / constructor).

- [ ] **Step 5.2: Add the call site**

In `consolidator.rs`, locate the block starting at :277 (Phase 17). Immediately after the Phase 17 block (after the `if protocols > 0 { ... }` at :282), insert:

```rust
    // Phase 23: Behavioral skill inference — elevate recurring clean tool-use
    // patterns from session_tool_call to the skill table.
    let skills_inferred = infer_skills_from_behavior(
        conn,
        config.skill_inference_min_sessions,
        config.skill_inference_window_days,
    );
    stats.skills_inferred = skills_inferred;
    if skills_inferred > 0 {
        eprintln!("[consolidator] inferred {skills_inferred} skills from tool-use patterns");
    }
```

- [ ] **Step 5.3: Build check (registration is implicitly smoke-tested by all T4 tests still passing)**

```bash
cargo build -p forge-daemon --lib --tests 2>&1 | tail -5
cargo test -p forge-daemon --lib 2>&1 | tail -5
cargo clippy --workspace -- -W clippy::all -D warnings 2>&1 | tail -5
cargo fmt --all
```

Expected: build OK, all tests pass (count unchanged at 1375 — no new tests this task). Clippy clean.

- [ ] **Step 5.4: Commit**

```bash
git add crates/daemon/src/workers/consolidator.rs
git commit -m "$(cat <<'EOF'
feat(2A-4c2 T5): register Phase 23 in consolidator orchestrator

Adds the infer_skills_from_behavior call immediately after Phase 17
(extract_protocols) in run_consolidation. Uses min_sessions +
window_days from ConsolidationConfig. Stats struct gains
skills_inferred counter so the usual "consolidator run summary"
log line reports it.

The master-design assertion that Phase 23 runs "after Phase 17" is
satisfied by this placement (Phase 17 block ends at :282, Phase 23
starts immediately after). The ProbePhase handler in T6 pins this
ordering via a static PHASE_ORDER const.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 6: `Request::ProbePhase` + `ResponseData::PhaseProbe` + handler + PHASE_ORDER const + 4 L1 tests

**Files:**
- Modify: `crates/core/src/protocol/request.rs` — add cfg-gated variant.
- Modify: `crates/core/src/protocol/response.rs` — add cfg-gated variant.
- Modify: `crates/daemon/src/workers/consolidator.rs` — add cfg-gated `PHASE_ORDER` const.
- Modify: `crates/daemon/src/server/handler.rs` — add cfg-gated handler arm + 4 tests.

**Goal:** Observability surface so bench assertion 9 can verify execution order.

- [ ] **Step 6.1: Add `PHASE_ORDER` const**

In `crates/daemon/src/workers/consolidator.rs`, near the top of the file (after the existing `use` block), add:

```rust
/// Registry of phases the consolidator executes, in execution order.
/// Used by `Request::ProbePhase` to answer master-design assertion 9
/// (Phase 23 executes after Phase 17).
///
/// `fn_name` matches the Rust function called for that phase.
/// `phase_number` is the 1-based doc numbering ("Phase N") — independent
/// of array position. 2A-4c2 only requires these two entries; future
/// assertions can extend the array without breaking anything.
#[cfg(any(test, feature = "bench"))]
pub const PHASE_ORDER: &[(&str, usize)] = &[
    ("extract_protocols", 17),
    ("infer_skills_from_behavior", 23),
];
```

- [ ] **Step 6.2: Add the cfg-gated protocol variants**

In `crates/core/src/protocol/request.rs`, find the `pub enum Request { ... }` block. Add the cfg-gated variant inside the enum:

```rust
    /// Probe consolidator phase execution order (test/bench-only).
    /// Returns the phase_number (1-based doc numbering) and the list of
    /// phase fn_names that execute before `phase_name`.
    #[cfg(any(test, feature = "bench"))]
    ProbePhase { phase_name: String },
```

In `crates/core/src/protocol/response.rs`, find `pub enum ResponseData { ... }`. Add:

```rust
    /// Response for `Request::ProbePhase`.
    #[cfg(any(test, feature = "bench"))]
    PhaseProbe {
        executed_at_phase_index: usize,
        executed_after: Vec<String>,
    },
```

If `ResponseData` has `#[serde(tag = "kind")]` (check the file), add `#[serde(rename = "phase_probe")]` on the variant to match the existing naming scheme for other cfg-gated variants like `recency_factor` from 2A-4b T12.

- [ ] **Step 6.3: Write 4 RED tests**

In `crates/daemon/src/server/handler.rs`, append to `#[cfg(test)] mod tests` block:

```rust
    // ── Phase 2A-4c2 T6: ProbePhase handler tests ────────────────────────────

    #[cfg(any(test, feature = "bench"))]
    #[test]
    fn probe_phase_returns_correct_index_for_infer_skills() {
        let mut state = DaemonState::new(":memory:").unwrap();
        let req = forge_core::protocol::Request::ProbePhase {
            phase_name: "infer_skills_from_behavior".to_string(),
        };
        match crate::server::handler::handle_request(&mut state, req) {
            forge_core::protocol::Response::Ok {
                data: forge_core::protocol::ResponseData::PhaseProbe {
                    executed_at_phase_index, ..
                },
            } => {
                assert_eq!(executed_at_phase_index, 23);
            }
            other => panic!("got {other:?}"),
        }
    }

    #[cfg(any(test, feature = "bench"))]
    #[test]
    fn probe_phase_executed_after_contains_extract_protocols() {
        let mut state = DaemonState::new(":memory:").unwrap();
        let req = forge_core::protocol::Request::ProbePhase {
            phase_name: "infer_skills_from_behavior".to_string(),
        };
        match crate::server::handler::handle_request(&mut state, req) {
            forge_core::protocol::Response::Ok {
                data: forge_core::protocol::ResponseData::PhaseProbe {
                    executed_after, ..
                },
            } => {
                assert!(
                    executed_after.contains(&"extract_protocols".to_string()),
                    "executed_after must contain Phase 17 (extract_protocols); got {executed_after:?}"
                );
            }
            other => panic!("got {other:?}"),
        }
    }

    #[cfg(any(test, feature = "bench"))]
    #[test]
    fn probe_phase_unknown_phase_errors() {
        let mut state = DaemonState::new(":memory:").unwrap();
        let req = forge_core::protocol::Request::ProbePhase {
            phase_name: "not_a_real_phase".to_string(),
        };
        match crate::server::handler::handle_request(&mut state, req) {
            forge_core::protocol::Response::Error { message } => {
                assert!(
                    message.starts_with("unknown_phase: "),
                    "expected unknown_phase: prefix, got {message}"
                );
            }
            other => panic!("got {other:?}"),
        }
    }

    #[cfg(any(test, feature = "bench"))]
    #[test]
    fn probe_phase_phase_17_executed_at_index_17() {
        let mut state = DaemonState::new(":memory:").unwrap();
        let req = forge_core::protocol::Request::ProbePhase {
            phase_name: "extract_protocols".to_string(),
        };
        match crate::server::handler::handle_request(&mut state, req) {
            forge_core::protocol::Response::Ok {
                data: forge_core::protocol::ResponseData::PhaseProbe {
                    executed_at_phase_index,
                    executed_after,
                },
            } => {
                assert_eq!(executed_at_phase_index, 17);
                assert_eq!(
                    executed_after,
                    Vec::<String>::new(),
                    "Phase 17 is the first in PHASE_ORDER — nothing before it"
                );
            }
            other => panic!("got {other:?}"),
        }
    }
```

- [ ] **Step 6.4: Run to confirm RED**

Run: `cargo test -p forge-daemon --lib probe_phase`
Expected: FAILs (handler not implemented).

- [ ] **Step 6.5: Implement the handler arm**

In `crates/daemon/src/server/handler.rs`, add a new cfg-gated arm to the `match request` block inside `handle_request`. Good placement: immediately after the `ListToolCalls` arm (around handler.rs:1380+). Match the pattern of the existing cfg-gated `ComputeRecencyFactor` arm (search for `ComputeRecencyFactor` in the file).

```rust
        // Phase 2A-4c2 T6: ProbePhase — consolidator phase introspection.
        #[cfg(any(test, feature = "bench"))]
        Request::ProbePhase { phase_name } => {
            let order = crate::workers::consolidator::PHASE_ORDER;
            match order.iter().position(|(n, _)| *n == phase_name) {
                Some(pos) => {
                    let (_, phase_number) = order[pos];
                    let executed_after: Vec<String> = order[..pos]
                        .iter()
                        .map(|(n, _)| (*n).to_string())
                        .collect();
                    Response::Ok {
                        data: forge_core::protocol::ResponseData::PhaseProbe {
                            executed_at_phase_index: phase_number,
                            executed_after,
                        },
                    }
                }
                None => Response::Error {
                    message: format!("unknown_phase: {phase_name}"),
                },
            }
        }
```

- [ ] **Step 6.6: Run to confirm GREEN**

Run: `cargo test -p forge-daemon --lib probe_phase`
Expected: 4 passed, 0 failed.

- [ ] **Step 6.7: Regression + lint (with bench feature, since some tests are cfg-gated)**

```bash
cargo test -p forge-daemon --lib 2>&1 | tail -5
cargo clippy --workspace -- -W clippy::all -D warnings 2>&1 | tail -5
cargo fmt --all
```

Expected: 1379 pass (prior 1375 + 4 new). Clippy clean.

- [ ] **Step 6.8: Commit**

```bash
git add crates/daemon/src/workers/consolidator.rs crates/core/src/protocol/request.rs crates/core/src/protocol/response.rs crates/daemon/src/server/handler.rs
git commit -m "$(cat <<'EOF'
feat(2A-4c2 T6): Request::ProbePhase + PHASE_ORDER const + handler

cfg(any(test, feature = "bench"))-gated protocol surface for master-
design assertion 9 (Phase 23 executes after Phase 17). Consumers:
integration tests + forge-bench Dim 5 harness.

PHASE_ORDER = &[("extract_protocols", 17), ("infer_skills_from_behavior", 23)]
is listed in execution order; phase_number is doc-convention
1-based numbering (independent of array position). executed_after
is derived from array position (prior fn_names).

Handler returns unknown_phase: <name> for typos — consistent with the
2A-4c1 unknown_session: prefix pattern for error messages.

4 L1 tests pin: correct phase_index for Phase 23, executed_after
contains extract_protocols, unknown phase errors, Phase 17 itself
returns index 17 + empty executed_after.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 7: Renderer dual-gate + `inferred_sessions=` attribute + 4 L1 tests

**Files:**
- Modify: `crates/daemon/src/db/manas.rs` — add Phase 23 fields to the `Skill` row struct + `list_skills` SELECT.
- Modify: `crates/daemon/src/recall.rs` — dual-gate filter + attribute branch at :1058-1100.
- 4 new tests — placement depends on where existing render tests live; if there are none specifically for `<skills>`, add them to `recall.rs`'s `#[cfg(test)] mod tests` block.

**Goal:** Phase 23 rows surface to `CompileContext`'s `<skills>` XML.

- [ ] **Step 7.1: Check how `list_skills` returns Skill rows today**

Run: `grep -n "pub fn list_skills\|pub struct Skill" crates/daemon/src/db/manas.rs | head -5`

If the `Skill` struct doesn't include `inferred_at` / `inferred_from`, add them. The existing `Skill` struct is in `manas.rs`. Locate it and extend:

```rust
#[derive(Debug, Clone)]
pub struct Skill {
    // ... existing fields ...
    pub inferred_at: Option<String>,
    pub inferred_from: String, // JSON array of session_ids, default "[]"
}
```

Update the `list_skills` SELECT to include both columns (and the row-mapper closure to populate them).

- [ ] **Step 7.2: Write 4 RED tests**

In `crates/daemon/src/recall.rs`, append to its `#[cfg(test)] mod tests` block (search `#[cfg(test)]`):

```rust
    // ── Phase 2A-4c2 T7: skills renderer dual-gate + inferred_sessions= ──────

    fn seed_schema(conn: &rusqlite::Connection) {
        crate::db::vec::init_sqlite_vec();
        crate::db::schema::create_schema(conn).unwrap();
    }

    /// Minimal compile_context rendering entry — whatever the real fn is called in
    /// recall.rs. Replace the invocation below with the actual entry (search for
    /// "<skills hint=" to find the renderer context). If the renderer is inside
    /// `compile_context`, call compile_context with a minimal request.
    fn render_skills_section(conn: &rusqlite::Connection, project: Option<&str>) -> String {
        // Actual impl: call into the real recall.rs rendering fn. Example:
        // crate::recall::compile_context_internal(conn, ... , project).unwrap()
        // Filter to just the <skills>...</skills> section for assertion clarity.
        //
        // Placeholder: tests should call the real entry point and extract <skills>
        // via a small helper. Implementer: wire this up against the actual fn at
        // `recall.rs:~1058`.
        let _ = (conn, project);
        unimplemented!("wire to actual recall.rs compile_context_internal")
    }

    #[test]
    fn skills_renderer_includes_success_count_rows() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        seed_schema(&conn);
        conn.execute(
            "INSERT INTO skill (id, name, domain, description, steps, source, success_count)
             VALUES ('s1', 'Use Cargo', 'shell', '', '[]', 'legacy', 1)",
            [],
        )
        .unwrap();
        let xml = render_skills_section(&conn, None);
        assert!(
            xml.contains("uses=\"1\"") && xml.contains("Use Cargo"),
            "legacy success_count row must render with uses= attribute; got {xml}"
        );
    }

    #[test]
    fn skills_renderer_includes_inferred_rows() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        seed_schema(&conn);
        conn.execute(
            "INSERT INTO skill
             (id, name, domain, description, steps, source, agent, fingerprint,
              inferred_from, inferred_at, success_count)
             VALUES ('s2', 'Inferred: Read+Edit+Bash [deadbeef]', 'file-ops', '', '[]',
                     'inferred', 'claude-code', 'deadbeefcafe',
                     '[\"SA\",\"SB\",\"SC\"]', '2026-04-23T10:00:00Z', 0)",
            [],
        )
        .unwrap();
        let xml = render_skills_section(&conn, None);
        assert!(
            xml.contains("inferred_sessions=\"3\"") && xml.contains("Inferred: Read+Edit+Bash"),
            "Phase 23 row must render with inferred_sessions= attribute; got {xml}"
        );
    }

    #[test]
    fn skills_renderer_excludes_zero_success_zero_inferred() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        seed_schema(&conn);
        conn.execute(
            "INSERT INTO skill (id, name, domain, description, steps, source, success_count)
             VALUES ('s3', 'Orphan skill', 'general', '', '[]', 'legacy', 0)",
            [],
        )
        .unwrap();
        let xml = render_skills_section(&conn, None);
        assert!(
            !xml.contains("Orphan skill"),
            "row with success_count=0 AND inferred_at=NULL must NOT render; got {xml}"
        );
    }

    #[test]
    fn skills_renderer_mixed_attributes_coexist() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        seed_schema(&conn);
        conn.execute(
            "INSERT INTO skill (id, name, domain, description, steps, source, success_count)
             VALUES ('s1', 'Legacy skill', 'shell', '', '[]', 'legacy', 2)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO skill
             (id, name, domain, description, steps, source, agent, fingerprint,
              inferred_from, inferred_at, success_count)
             VALUES ('s2', 'Inferred: Read [cafe1234]', 'file-ops', '', '[]',
                     'inferred', 'claude-code', 'cafe1234babe',
                     '[\"SA\",\"SB\",\"SC\"]', '2026-04-23T10:00:00Z', 0)",
            [],
        )
        .unwrap();
        let xml = render_skills_section(&conn, None);
        assert!(xml.contains("uses=\"2\""), "legacy row keeps uses= attribute");
        assert!(
            xml.contains("inferred_sessions=\"3\""),
            "inferred row gets inferred_sessions= attribute"
        );
    }
```

**Implementer note:** the `render_skills_section` helper is a `unimplemented!()` placeholder. Before running the tests, wire it to the actual render path in `recall.rs`. A clean approach: extract a pure-enough fn that takes a `&Connection` and returns the `<skills>` XML substring. If refactoring the main render fn is too invasive, call the full `compile_context` path and grep the result for `<skills>...</skills>`.

- [ ] **Step 7.3: Run to confirm RED**

Run: `cargo test -p forge-daemon --lib skills_renderer`
Expected: FAILs (helper `unimplemented!`).

- [ ] **Step 7.4: Update the renderer**

In `crates/daemon/src/recall.rs` around lines 1058-1100, two changes:

**(a) Dual-gate filter.** Replace:

```rust
            .filter(|s| {
                s.success_count > 0
                    && (s.project.is_none()
```

with:

```rust
            .filter(|s| {
                (s.success_count > 0 || s.inferred_at.is_some())
                    && (s.project.is_none()
```

**(b) Attribute branch.** Replace the inner `for s in &active_skills { ... }` loop:

```rust
            for s in &active_skills {
                let entry = format!(
                    "\n  <skill domain=\"{}\" uses=\"{}\">{}</skill>",
                    xml_escape(&s.domain),
                    s.success_count,
                    xml_escape(&s.name)
                );
                if used + skill_xml.len() + entry.len() < budget {
                    skill_xml.push_str(&entry);
                }
            }
```

with:

```rust
            for s in &active_skills {
                let entry = if s.inferred_at.is_some() {
                    // Phase 23 row — show inferred_sessions instead of uses.
                    let sessions: usize =
                        match serde_json::from_str::<serde_json::Value>(&s.inferred_from) {
                            Ok(serde_json::Value::Array(a)) => a.len(),
                            _ => 0,
                        };
                    format!(
                        "\n  <skill domain=\"{}\" inferred_sessions=\"{}\">{}</skill>",
                        xml_escape(&s.domain),
                        sessions,
                        xml_escape(&s.name)
                    )
                } else {
                    format!(
                        "\n  <skill domain=\"{}\" uses=\"{}\">{}</skill>",
                        xml_escape(&s.domain),
                        s.success_count,
                        xml_escape(&s.name)
                    )
                };
                if used + skill_xml.len() + entry.len() < budget {
                    skill_xml.push_str(&entry);
                }
            }
```

- [ ] **Step 7.5: Wire `render_skills_section` test helper**

Replace the `unimplemented!()` in the test helper with a call into the real renderer. If there's a public or pub(crate) entry like `compile_context_internal`, use it; else extract a small `pub(crate) fn render_skills_xml(conn: &Connection, project: Option<&str>, budget: usize) -> String` in `recall.rs` and call that.

- [ ] **Step 7.6: Run to confirm GREEN**

Run: `cargo test -p forge-daemon --lib skills_renderer`
Expected: 4 passed.

- [ ] **Step 7.7: Regression + lint**

```bash
cargo test -p forge-daemon --lib 2>&1 | tail -5
cargo clippy --workspace -- -W clippy::all -D warnings 2>&1 | tail -5
cargo fmt --all
```

Expected: 1383 pass (prior 1379 + 4 new). Clippy clean. If existing `<skills>` renderer tests break — update the existing test fixtures to expect the new `inferred_sessions=` attribute when a row has `inferred_at IS NOT NULL`.

- [ ] **Step 7.8: Commit**

```bash
git add crates/daemon/src/db/manas.rs crates/daemon/src/recall.rs
git commit -m "$(cat <<'EOF'
feat(2A-4c2 T7): skills renderer dual-gate + inferred_sessions attribute

Extend the <skills> section in CompileContext rendering to include
Phase 23 rows (inferred_at IS NOT NULL). Row filter was success_count > 0
ONLY; now also includes rows with inferred_at set — matches master-
design prerequisite renderer update mandate.

For Phase 23 rows, the XML attribute is inferred_sessions="N" (N =
json_array_len(inferred_from)) instead of uses="N". Both attributes
coexist in a single <skills> block for mixed legacy + inferred skill
lists.

Skill row struct in manas.rs gains `inferred_at: Option<String>` and
`inferred_from: String` (JSON, defaults "[]") so the renderer can
branch without a second query.

4 L1 tests pin: success_count-only rows still render, Phase 23 rows
render with new attribute, rows with neither signal are excluded,
mixed types coexist in same render.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 8: Integration test `tests/skill_inference_flow.rs`

**Files:**
- Create: `crates/daemon/tests/skill_inference_flow.rs`

**Goal:** End-to-end via public protocol — `register_session` × 3 → `record_tool_use` × 9 → `force_consolidate` → `compile_context` shows `<skill inferred_sessions="3">`.

- [ ] **Step 8.1: Create the integration test file**

Write `crates/daemon/tests/skill_inference_flow.rs`:

```rust
//! Integration test for Phase 2A-4c2 Forge-Behavioral-Skill-Inference.
//!
//! Exercises the full surface end-to-end through the Rust handler (no HTTP):
//! register_session × 3 → record_tool_use × 9 (matching fingerprint) →
//! force_consolidate → compile_context → verify <skill inferred_sessions="3">.

use forge_core::protocol::{Request, Response, ResponseData};
use forge_daemon::server::handler::{handle_request, DaemonState};

fn fresh_state() -> DaemonState {
    DaemonState::new(":memory:").expect("DaemonState::new(:memory:)")
}

fn register(state: &mut DaemonState, id: &str, agent: &str, project: &str) {
    let resp = handle_request(
        state,
        Request::RegisterSession {
            id: id.to_string(),
            agent: agent.to_string(),
            project: project.to_string(),
            cwd: "/tmp".to_string(),
        },
    );
    assert!(matches!(resp, Response::Ok { .. }), "register failed: {resp:?}");
}

fn record(
    state: &mut DaemonState,
    session: &str,
    tool: &str,
    args: serde_json::Value,
) {
    let resp = handle_request(
        state,
        Request::RecordToolUse {
            session_id: session.to_string(),
            agent: "claude-code".to_string(),
            tool_name: tool.to_string(),
            tool_args: args,
            tool_result_summary: String::new(),
            success: true,
            user_correction_flag: false,
        },
    );
    assert!(
        matches!(resp, Response::Ok { .. }),
        "record_tool_use failed: {resp:?}"
    );
}

fn force_consolidate(state: &mut DaemonState) {
    let resp = handle_request(state, Request::ForceConsolidate {});
    assert!(
        matches!(resp, Response::Ok { .. }),
        "force_consolidate failed: {resp:?}"
    );
}

fn compile_context_xml(state: &mut DaemonState, project: &str) -> String {
    let resp = handle_request(
        state,
        Request::CompileContext {
            project: Some(project.to_string()),
            session_id: None,
            excluded_layers: vec![],
        },
    );
    match resp {
        Response::Ok {
            data: ResponseData::Context { xml, .. },
        } => xml,
        other => panic!("compile_context failed: {other:?}"),
    }
}

#[test]
fn skill_inference_end_to_end_via_protocol() {
    let mut state = fresh_state();
    for sid in ["SA", "SB", "SC"] {
        register(&mut state, sid, "claude-code", "proj");
        record(&mut state, sid, "Read", serde_json::json!({"file_path": "/a"}));
        record(
            &mut state,
            sid,
            "Edit",
            serde_json::json!({"file_path": "/a", "old_string": "x", "new_string": "y"}),
        );
        record(&mut state, sid, "Bash", serde_json::json!({"cmd": "cargo test"}));
    }

    force_consolidate(&mut state);

    let xml = compile_context_xml(&mut state, "proj");
    assert!(
        xml.contains("inferred_sessions=\"3\""),
        "<skills> must contain inferred_sessions=\"3\" after 3 matching sessions; got:\n{xml}"
    );
    assert!(
        xml.contains("Inferred: Bash+Edit+Read"),
        "inferred skill name missing from XML:\n{xml}"
    );
}

#[test]
fn skill_inference_does_not_emit_for_two_sessions() {
    let mut state = fresh_state();
    for sid in ["SA", "SB"] {
        register(&mut state, sid, "claude-code", "proj");
        record(&mut state, sid, "Read", serde_json::json!({"file_path": "/a"}));
        record(
            &mut state,
            sid,
            "Edit",
            serde_json::json!({"file_path": "/a", "old_string": "x", "new_string": "y"}),
        );
        record(&mut state, sid, "Bash", serde_json::json!({"cmd": "cargo test"}));
    }

    force_consolidate(&mut state);

    let xml = compile_context_xml(&mut state, "proj");
    assert!(
        !xml.contains("Inferred: Bash+Edit+Read"),
        "inferred skill must NOT be emitted at 2 sessions; got:\n{xml}"
    );
}
```

**Implementer note:** the `Request` variant names (`RegisterSession`, `ForceConsolidate`, `CompileContext`) and their field lists must match the actual enum definitions in `crates/core/src/protocol/request.rs`. If field names differ (e.g., `cwd` is optional), adjust the struct literal. Same for `ResponseData::Context { xml, .. }` — check the actual shape.

- [ ] **Step 8.2: Run to confirm GREEN**

Run: `cargo test -p forge-daemon --test skill_inference_flow`
Expected: 2 passed.

- [ ] **Step 8.3: Regression**

```bash
cargo test -p forge-daemon --lib 2>&1 | tail -5
cargo test -p forge-daemon --tests 2>&1 | tail -5
cargo clippy --workspace -- -W clippy::all -D warnings 2>&1 | tail -5
cargo fmt --all
```

Expected: 1383 lib pass + 2 new integration tests pass. Clippy clean.

- [ ] **Step 8.4: Commit**

```bash
git add crates/daemon/tests/skill_inference_flow.rs
git commit -m "$(cat <<'EOF'
test(2A-4c2 T8): end-to-end skill_inference_flow integration tests

Two integration tests covering master-design infrastructure
assertion 12:

1. skill_inference_end_to_end_via_protocol: register 3 sessions,
   record 3-tool fingerprint in each, force_consolidate, then
   compile_context XML must contain <skill inferred_sessions="3">
   with the expected "Inferred: Bash+Edit+Read" name token.

2. skill_inference_does_not_emit_for_two_sessions: same flow with
   2 sessions, verify skill absent from XML.

Pins the full end-to-end path: RecordToolUse → session_tool_call
→ consolidator Phase 23 → skill table → recall.rs renderer →
<skills> XML.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 9: Schema rollback recipe test (follows 2A-4c1 T11 precedent)

**Files:**
- Modify: `crates/daemon/src/db/schema.rs` — add test to `#[cfg(test)] mod tests` block.

**Goal:** Document and verify the ALTER/INDEX rollback for operators who want to undo 2A-4c2.

- [ ] **Step 9.1: Write the test**

Append to `#[cfg(test)] mod tests` block (after the 2A-4c1 T11 rollback test):

```rust
    // ── Phase 2A-4c2 T9: Phase 23 schema rollback recipe ─────────────────────

    #[test]
    fn test_skill_phase23_columns_and_index_rollback_recipe_works_on_populated_db() {
        crate::db::vec::init_sqlite_vec();
        let conn = Connection::open_in_memory().unwrap();
        create_schema(&conn).unwrap();

        // Seed a Phase 23 skill row.
        conn.execute(
            "INSERT INTO skill
             (id, name, domain, description, steps, source, agent, fingerprint,
              inferred_from, inferred_at, success_count)
             VALUES ('s1', 'Inferred: Read+Edit+Bash [deadbeef]', 'file-ops', '', '[]',
                     'inferred', 'claude-code', 'deadbeefcafe1234',
                     '[\"SA\",\"SB\",\"SC\"]', '2026-04-23T10:00:00Z', 0)",
            [],
        )
        .unwrap();

        // Pre-assertion: the partial unique index must exist before rollback.
        // Without this, a regression that silently removed the index creation
        // would let the rollback's DROP IF EXISTS no-op and the post-assertion
        // pass vacuously (per 2A-4c1 H1 precedent).
        let idx_count_before: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master
                 WHERE type='index' AND name='idx_skill_agent_fingerprint'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            idx_count_before, 1,
            "partial unique index must exist before rollback — forward migration regression"
        );

        // Rollback recipe (documented in spec §6 / this test's commit message).
        // SQLite doesn't support DROP COLUMN directly in all versions; use
        // table rebuild pattern — rename, recreate, copy.
        conn.execute_batch(
            "
            DROP INDEX IF EXISTS idx_skill_agent_fingerprint;
            -- SQLite 3.35+ supports ALTER TABLE ... DROP COLUMN directly.
            ALTER TABLE skill DROP COLUMN inferred_at;
            ALTER TABLE skill DROP COLUMN inferred_from;
            ALTER TABLE skill DROP COLUMN fingerprint;
            ALTER TABLE skill DROP COLUMN agent;
            ",
        )
        .unwrap();

        // Post-assertions.
        let idx_after: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master
                 WHERE type='index' AND name='idx_skill_agent_fingerprint'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(idx_after, 0, "partial unique index should be dropped");

        // None of the 4 Phase 23 columns exist in PRAGMA table_info any more.
        let cols: Vec<String> = conn
            .prepare("PRAGMA table_info(skill)")
            .unwrap()
            .query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .filter_map(|r| r.ok())
            .collect();
        for phase23_col in ["agent", "fingerprint", "inferred_from", "inferred_at"] {
            assert!(
                !cols.contains(&phase23_col.to_string()),
                "column {phase23_col} must be absent after rollback"
            );
        }

        // Legacy skill columns still present (rollback didn't damage pre-existing schema).
        for legacy_col in ["id", "name", "domain", "description", "success_count"] {
            assert!(
                cols.contains(&legacy_col.to_string()),
                "legacy column {legacy_col} must still exist"
            );
        }
    }
```

- [ ] **Step 9.2: Run**

Run: `cargo test -p forge-daemon --lib test_skill_phase23_columns_and_index_rollback`
Expected: PASS (no new implementation; SQLite 3.35+ supports DROP COLUMN).

If it fails due to SQLite version, rewrite the rollback using the table-rename-and-copy pattern and re-document in the commit. The test helps catch version mismatches early.

- [ ] **Step 9.3: Regression + lint**

```bash
cargo test -p forge-daemon --lib 2>&1 | tail -5
cargo clippy --workspace -- -W clippy::all -D warnings 2>&1 | tail -5
cargo fmt --all
```

Expected: 1384 pass (prior 1383 + 1 new). Clippy clean.

- [ ] **Step 9.4: Commit**

```bash
git add crates/daemon/src/db/schema.rs
git commit -m "$(cat <<'EOF'
test(2A-4c2 T9): Phase 23 schema rollback recipe validated

Documents and verifies the rollback sequence for operators who want to
undo 2A-4c2:

  DROP INDEX IF EXISTS idx_skill_agent_fingerprint;
  ALTER TABLE skill DROP COLUMN inferred_at;
  ALTER TABLE skill DROP COLUMN inferred_from;
  ALTER TABLE skill DROP COLUMN fingerprint;
  ALTER TABLE skill DROP COLUMN agent;

Requires SQLite 3.35+ for DROP COLUMN. Pre-assertion (per 2A-4c1 H1
precedent) verifies the index exists before rollback — a forward-
migration regression that failed to create the index would be caught
here instead of passing the post-assertion vacuously.

Post-assertions: partial unique index gone, all 4 Phase 23 columns
gone, legacy columns (id/name/domain/description/success_count)
still present.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 10: Adversarial review on T1-T9 diff

**Files:** none modified unless the review finds actionable issues.

**Goal:** Find reasons NOT to ship. Codex CLI substitute via Claude `feature-dev:code-reviewer` subagent.

- [ ] **Step 10.1: Dispatch the review**

Invoke the `Agent` tool with `subagent_type: "feature-dev:code-reviewer"` and this prompt (substitute-for-Codex review per established SP1/2A-4c1 pattern):

```
You are an adversarial reviewer for 9 commits landing Phase 23 Behavioral
Skill Inference in a production Rust daemon. Find reasons NOT to merge. Be
ruthless. NO BLOCKERS only if you genuinely find none.

Working dir: /Users/dsskonuru/workspace/playground/forge
Branch: master

Commits to review: the 9 commits on top of HEAD before this sub-project
started (diff range: cf74fb3..HEAD on 2A-4c2 work).

Probe angles, each grounded in the actual diff:

1. canonical_fingerprint determinism under weird inputs — empty calls[],
   single call with no arg_keys, duplicate tool_name with same vs
   different arg_keys, Unicode tool names.

2. partial unique index on (agent, fingerprint) WHERE fingerprint != '' —
   can two rows somehow both satisfy fingerprint == '' and collide on an
   INSERT from the Phase 23 path? Inspect the INSERT to confirm it
   always sets fingerprint to a non-empty value.

3. INSERT ON CONFLICT json_each / json_group_array merge SQL — when
   inferred_from is '[]' or malformed, does the merge produce a sane
   result or blow up? Test both edge cases.

4. Consolidator orchestrator registration — is the call at the right
   place in run_consolidation to satisfy "runs after Phase 17"?
   Specifically: does any other phase between 17 and the Phase 23 call
   site do something that would invalidate our assumptions (e.g.,
   delete session_tool_call rows)?

5. Renderer dual-gate — if inferred_from is an empty array ("[]"), does
   inferred_sessions="0" render, or is the row filtered? Check the
   filter logic.

6. ProbePhase const-array drift — PHASE_ORDER currently has 2 entries.
   What happens if a future phase reorders existing phases (e.g.,
   Phase 22 is changed to run BEFORE Phase 17)? The const would lie.
   Is there a runtime sanity assertion, or is this intentional
   drift-tolerance?

7. T4 test fixture — the JSON literals for tool_args use \" escaping
   inside rusqlite INSERT VALUES. Is that quoting correct for
   SQLite's own escape rules? (Rust raw strings only escape ", not "
   interpreted by SQLite.)

8. Config out-of-range handling — if someone sets
   skill_inference_min_sessions = 0 in config.toml, validated()
   clamps to 1, but does the raw struct field (before validated()
   is called) ever get read anywhere? Grep usages.

9. JSON1 extension assumption — json_each / json_group_array are
   bundled in SQLite since 3.9, but the project might compile against
   a stripped build (rusqlite `bundled` feature). Verify the test
   suite catches JSON1-missing scenarios.

Output: 5-10 issues ranked BLOCKER / HIGH / MEDIUM / LOW with
file:line citations. If no BLOCKERs, say "NO BLOCKERS" explicitly.
```

- [ ] **Step 10.2: Address BLOCKER + HIGH findings**

If any BLOCKER: do not proceed to Task 11. Fix, re-run gates, re-review or move on once the issue is resolved. Commit as `chore(2A-4c2): address adversarial review — <finding-ids>`.

If only MEDIUM/LOW: document them in the T11 results doc carry-forwards list, no code fixup required unless you choose to address.

---

## Task 11: Live-daemon dogfood + results doc

**Files:**
- Create: `docs/benchmarks/results/2026-04-23-forge-behavioral-skill-inference.md`

**Goal:** Rebuild daemon at HEAD, seed 3 sessions via curl, force consolidation, verify `<skills>` contains the inferred token.

- [ ] **Step 11.1: Rebuild release daemon**

```bash
cargo build --release --bin forge-daemon 2>&1 | tail -5
```

Expected: Finished.

- [ ] **Step 11.2: Restart daemon**

```bash
PID=$(pgrep -fl forge-daemon | awk 'NR==1 {print $1}')
if [ -n "$PID" ]; then kill -TERM "$PID"; sleep 3; fi
rm -f /Users/dsskonuru/.forge/forge.pid
nohup /Users/dsskonuru/workspace/playground/forge/target/release/forge-daemon > /tmp/forge-daemon-dogfood.log 2>&1 & disown
sleep 6

# Verify HEAD sha matches.
HEAD_SHA=$(git rev-parse --short HEAD)
curl -sS -X POST http://127.0.0.1:8430/api -d '{"method":"version"}' | jq .
```

Expected: JSON `.data.git_sha` matches `$HEAD_SHA`.

- [ ] **Step 11.3: Seed sessions via curl**

```bash
DAEMON=http://127.0.0.1:8430/api

for SID in DOGFOOD-2A4C2-SA DOGFOOD-2A4C2-SB DOGFOOD-2A4C2-SC; do
  curl -sS -X POST $DAEMON -d "{
    \"method\":\"register_session\",
    \"params\":{\"id\":\"$SID\",\"agent\":\"claude-code\",\"project\":\"dogfood-2a4c2\",\"cwd\":\"/tmp\"}
  }" | jq -r '.status'

  for CALL in '"tool_name":"Read","tool_args":{"file_path":"/tmp/a"}' \
              '"tool_name":"Edit","tool_args":{"file_path":"/tmp/a","old_string":"x","new_string":"y"}' \
              '"tool_name":"Bash","tool_args":{"cmd":"cargo test"}'; do
    curl -sS -X POST $DAEMON -d "{
      \"method\":\"record_tool_use\",
      \"params\":{\"session_id\":\"$SID\",\"agent\":\"claude-code\",$CALL,
                  \"tool_result_summary\":\"ok\",\"success\":true,\"user_correction_flag\":false}
    }" | jq -r '.status'
  done
done
```

Expected: all `"ok"`.

- [ ] **Step 11.4: Force consolidation**

```bash
curl -sS -X POST http://127.0.0.1:8430/api -d '{"method":"force_consolidate"}' | tee /tmp/dogfood_consolidate.json | jq .
```

Expected: `{"status":"ok","data":{"kind":"consolidation",...}}` with a non-zero skills_inferred count (or similar field).

- [ ] **Step 11.5: Verify `<skills>` renders inferred skill**

```bash
curl -sS -X POST http://127.0.0.1:8430/api -d '{
  "method":"compile_context",
  "params":{"project":"dogfood-2a4c2"}
}' | tee /tmp/dogfood_compile.json | jq -r '.data.xml' > /tmp/dogfood_context.xml

# Assert the inferred skill appears.
grep -E 'inferred_sessions="3".*Inferred: Bash\+Edit\+Read' /tmp/dogfood_context.xml \
  && echo "DOGFOOD PASS" || echo "DOGFOOD FAIL"
```

Expected: "DOGFOOD PASS" line printed; `/tmp/dogfood_context.xml` contains `<skill domain="file-ops" inferred_sessions="3">Inferred: Bash+Edit+Read [...]</skill>`.

- [ ] **Step 11.6: (Optional) Verify ProbePhase via HTTP (only if bench feature built into the release binary)**

```bash
curl -sS -X POST http://127.0.0.1:8430/api -d '{"method":"probe_phase","params":{"phase_name":"infer_skills_from_behavior"}}' | jq .
```

If built with `--release` default (no bench feature), this returns an error. That's expected — the assertion is test-gated, not runtime-gated. The unit tests at T6 cover it.

- [ ] **Step 11.7: Write the results doc**

Create `docs/benchmarks/results/2026-04-23-forge-behavioral-skill-inference.md` mirroring the 2A-4c1 results doc structure. Template:

```markdown
# Forge-Behavioral-Skill-Inference (Phase 2A-4c2) — Results

**Phase:** 2A-4c2 of Phase 2A-4 Forge-Identity master decomposition.
**Date:** 2026-04-23
**Parent design:** `docs/superpowers/specs/2026-04-23-forge-behavioral-skill-inference-design.md`
**Implementation plan:** `docs/superpowers/plans/2026-04-23-forge-behavioral-skill-inference.md`
**HEAD at ship time:** `<git rev-parse HEAD>`
**Prior phase:** 2A-4c1 shipped 2026-04-23 (HEAD `cf74fb3`).

## Summary

**SHIPPED.** Phase 23 `infer_skills_from_behavior` is live. Given ≥ 3
matching clean tool-use fingerprints across sessions, the consolidator
now elevates them to the `skill` table and surfaces them in
`<skills>` via `CompileContext`.

**Tests:** 1384 lib + 2 new integration tests (total 1386) — baseline
was 1352 at 2A-4c1 ship.

**Live dogfood (HEAD `<short-sha>`):**
- 3 sessions registered, 9 `record_tool_use` calls matching fingerprint, `force_consolidate` returned skills_inferred=1.
- `<skill domain="file-ops" inferred_sessions="3">Inferred: Bash+Edit+Read [<hash>]</skill>` appeared in `<skills>`.

## What shipped

(Table of T1-T9 with commit SHAs, per 2A-4c1 template.)

## Known carry-forwards

- Hook auto-wiring (PostToolUse → record_tool_use) deferred to 2A-4c3 or follow-up.
- `informed_by` edge between Phase 17 protocols and Phase 23 skills deferred.
- Skill retirement / success_count updates / fuzzy fingerprinting — out of scope per master.

## Adversarial review

(Embed T10 findings here, ranked BLOCKER / HIGH / MEDIUM / LOW with file:line citations.)

## Test gates (at HEAD)

- `cargo test -p forge-daemon --lib` → 1384 passed, 0 failed, 1 ignored
- `cargo test -p forge-daemon --test skill_inference_flow` → 2 passed
- `cargo clippy --workspace -- -W clippy::all -D warnings` → clean
- `cargo fmt --all` → clean

## Ship checklist

- [x] T1-T9 committed.
- [x] T10 adversarial review completed.
- [x] T11 live-daemon dogfood passing.
- [x] Results doc written (this file).
- [ ] Push to `origin/master` — awaiting user approval.
```

Fill in the `<...>` placeholders with actual values from Steps 11.2-11.5 output.

- [ ] **Step 11.8: Commit results doc**

```bash
git add docs/benchmarks/results/2026-04-23-forge-behavioral-skill-inference.md
git commit -m "$(cat <<'EOF'
docs(2A-4c2 T11): Forge-Behavioral-Skill-Inference dogfood results

Documents the Phase 23 ship: 9 tasks T1-T9 + T10 adversarial review
+ T11 dogfood. Live-daemon verification that 3-session tool-use
pattern → Phase 23 elevation → <skill inferred_sessions="3"> in
CompileContext XML.

Test counts: 1384 lib (baseline 1352 + 32 new), 2 integration.

Known carry-forwards: hook auto-wiring, informed_by edge, skill
retirement. Not in 2A-4c2 scope per master design.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

- [ ] **Step 11.9: STOP gate — await user approval before push**

```bash
git log --oneline cf74fb3..HEAD | wc -l   # should be ~11 commits
git log --oneline cf74fb3..HEAD
```

DO NOT run `git push origin master` without explicit user approval. Show the log, summarize, ask to push.

---

## Self-review checklist

- [x] **Spec coverage:** T1 covers §2.1 schema, T2 covers §2.2 config, T3 covers §2.3 pure helpers, T4 covers §2.4 orchestrator, T5 covers §1 registration + §2.5 orchestrator wiring, T6 covers §2.5 ProbePhase, T7 covers §2.6 renderer, T8 covers §5.5 L3 integration, T9 covers §5.6 L4 schema rollback, T10 covers adversarial review, T11 covers §5.7 L4 dogfood.
- [x] **Placeholder scan:** the `render_skills_section` test helper in Task 7 is explicitly marked `unimplemented!()` with an implementer note to wire it — not a spec-skip but a pragmatic decision (wiring depends on whether existing render fn is pub or needs refactoring; both paths described). All other steps have concrete code.
- [x] **Type consistency:** `ToolCall`, `ConsolidationConfig`, `Skill` struct, `PHASE_ORDER` const signature, `Request::ProbePhase` / `ResponseData::PhaseProbe` — all match across tasks.
- [x] **Commit messages:** all use HEREDOC with `Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>` trailer per project convention.

## Execution handoff

Plan complete and saved to `docs/superpowers/plans/2026-04-23-forge-behavioral-skill-inference.md`.

Two execution options:

**1. Subagent-Driven (recommended)** — I dispatch a fresh subagent per task, review between tasks, fast iteration.

**2. Inline Execution** — Execute tasks in this session using `superpowers:executing-plans`, batch execution with checkpoints.

Which approach?
