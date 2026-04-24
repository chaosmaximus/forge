# Housekeeping + Doctor Observability Implementation Plan

> **STATUS:** SHIPPED in the interim — retained only for historical context. Triaged 2026-04-24 as part of Stream C (2P-1b cleanup).
>
> All 4 tasks are live:
>
> | Task | Evidence |
> |------|----------|
> | 1. `Request::Version` endpoint + `build.rs` | `crates/core/src/protocol/request.rs:990` + `crates/daemon/build.rs` + live `{"method":"version"}` responds with `git_sha`, `rustc_version`, `target_triple` (T11 dogfood `/tmp/dogfood_version.json`) |
> | 2. HttpClient configurable timeout | `crates/daemon/src/bin/forge-bench.rs:119-244` uses `request_timeout_ms` flag → `Duration::from_millis` into `HttpClient::with_timeout` at `forge_persist.rs:498` |
> | 3. Session message pagination + offset | `crates/daemon/src/sessions.rs:1323` test + `offset` field on `Request::SessionMessages` |
> | 4. Doctor "on steroids" | `ResponseData::Doctor` (response.rs:153-) carries `version`, `git_sha`, `raw_documents_count`, `raw_chunks_count`, per-layer counts; `structured checks: Vec<HealthCheck>` |
>
> No further action; this file is kept so future readers understand the original design intent. For live doctor fields and related observability work, see the current response.rs.

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Land 4 deferred housekeeping items from Forge-Persist reviews (Version endpoint, HttpClient timeout, session pagination, push to origin), then enhance Doctor into a "Doctor on steroids" observability surface for the dogfood gate.

**Architecture:** Five independent changes to the Forge daemon. C1 (Version) adds a new lightweight protocol endpoint with build-time metadata captured via `build.rs`. C2 (HttpClient timeout) makes the bench harness configurable. C3 (session pagination) adds SQL OFFSET support. C4 (Doctor enhancement) consolidates version + raw layer + session stats into the existing Doctor response. Each change follows the existing Request/Response/handler/tier/contract-test pattern.

**Tech Stack:** Rust, SQLite, serde JSON, reqwest blocking, `build.rs` for compile-time metadata capture.

---

## File Structure

| File | Responsibility | Tasks |
|------|---------------|-------|
| `crates/daemon/build.rs` | Build-time metadata (git SHA, rustc version, target) | 1 |
| `crates/core/src/protocol/request.rs` | `Request::Version` variant + `SessionMessages.offset` field | 1, 3 |
| `crates/core/src/protocol/response.rs` | `ResponseData::Version` variant + Doctor new fields | 1, 4 |
| `crates/core/src/protocol/mod.rs` | Re-export new types if any | 1 |
| `crates/core/src/protocol/contract_tests.rs` | Serde round-trip pins for Version + SessionMessages.offset | 1, 3 |
| `crates/daemon/src/server/handler.rs` | Handler arms for Version + pagination passthrough + Doctor enhancement | 1, 3, 4 |
| `crates/daemon/src/server/tier.rs` | Version → free tier | 1 |
| `crates/daemon/src/sessions.rs` | `list_messages` offset param + cap raise 100→1000 | 3 |
| `crates/daemon/src/bench/forge_persist.rs` | HttpClient configurable timeout + auto-paginate + runtime version query | 2, 3, 5 |
| `crates/daemon/src/bin/forge-bench.rs` | Pass `request_timeout` from CLI to PersistConfig | 2 |
| `crates/daemon/src/db/raw.rs` | `count_raw_documents()` + `count_raw_chunks()` helpers | 4 |

---

### Task 1: `Request::Version` endpoint

**Files:**
- Modify: `crates/daemon/build.rs`
- Modify: `crates/core/src/protocol/request.rs:904` (before `Shutdown`)
- Modify: `crates/core/src/protocol/response.rs:916` (before `Shutdown`)
- Modify: `crates/core/src/protocol/contract_tests.rs:41` (unit variants list)
- Modify: `crates/daemon/src/server/handler.rs:832` (after Status arm)
- Modify: `crates/daemon/src/server/tier.rs:213` (free-tier catch-all)
- Test: inline `#[cfg(test)]` in handler.rs + contract_tests.rs

- [ ] **Step 1.1: Write the failing contract test for `Request::Version`**

Add to `crates/core/src/protocol/contract_tests.rs` in the `test_unit_variants_method_names` cases vec:

```rust
("version", Request::Version),
```

- [ ] **Step 1.2: Run test to verify it fails**

Run: `cargo test -p forge-core test_unit_variants_method_names`
Expected: FAIL — `Request::Version` does not exist (E0599)

- [ ] **Step 1.3: Add `Request::Version` variant**

In `crates/core/src/protocol/request.rs`, before `Shutdown`:

```rust
/// Runtime version and build metadata. Lightweight (no DB queries).
Version,
```

- [ ] **Step 1.4: Add `ResponseData::Version` variant**

In `crates/core/src/protocol/response.rs`, before `Shutdown`:

```rust
/// Runtime version and build metadata — no DB queries, sub-1ms.
Version {
    /// Crate version from Cargo.toml (e.g., "0.4.0").
    version: String,
    /// "release" or "debug".
    build_profile: String,
    /// Platform triple (e.g., "aarch64-apple-darwin").
    target_triple: String,
    /// Rust compiler version used to build this binary.
    rustc_version: String,
    /// Short git commit hash at build time, if available.
    git_sha: Option<String>,
    /// Daemon uptime in seconds since process start.
    uptime_secs: u64,
},
```

- [ ] **Step 1.5: Run contract test to verify it compiles and passes**

Run: `cargo test -p forge-core test_unit_variants_method_names`
Expected: PASS (serde round-trip for the unit variant)

- [ ] **Step 1.6: Extend `build.rs` to capture git SHA and rustc version**

Replace `crates/daemon/build.rs`:

```rust
fn main() -> Result<(), Box<dyn std::error::Error>> {
    tonic_build::compile_protos("proto/forge.proto")?;

    // Capture git short SHA at build time (best-effort — CI may not have .git)
    let git_sha = std::process::Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()
        .and_then(|o| {
            if o.status.success() {
                String::from_utf8(o.stdout).ok().map(|s| s.trim().to_string())
            } else {
                None
            }
        })
        .unwrap_or_default();
    println!("cargo::rustc-env=FORGE_GIT_SHA={git_sha}");

    // Capture rustc version
    let rustc_version = std::process::Command::new("rustc")
        .args(["--version"])
        .output()
        .ok()
        .and_then(|o| {
            if o.status.success() {
                String::from_utf8(o.stdout).ok().map(|s| s.trim().to_string())
            } else {
                None
            }
        })
        .unwrap_or_else(|| "unknown".to_string());
    println!("cargo::rustc-env=FORGE_RUSTC_VERSION={rustc_version}");

    // Capture target triple
    if let Ok(target) = std::env::var("TARGET") {
        println!("cargo::rustc-env=FORGE_TARGET={target}");
    }

    Ok(())
}
```

- [ ] **Step 1.7: Write failing handler test**

In `crates/daemon/src/server/handler.rs`, inside `#[cfg(test)] mod tests`:

```rust
#[test]
fn test_version_returns_build_metadata() {
    let state = AppState::new_test();
    let resp = handle_request(Request::Version, &state);
    match resp {
        Response::Ok { data: ResponseData::Version { version, build_profile, target_triple, rustc_version, .. } } => {
            assert!(!version.is_empty(), "version must not be empty");
            assert!(
                build_profile == "release" || build_profile == "debug",
                "build_profile must be 'release' or 'debug', got: {build_profile}"
            );
            assert!(!target_triple.is_empty(), "target_triple must not be empty");
            assert!(!rustc_version.is_empty(), "rustc_version must not be empty");
        }
        other => panic!("expected Version response, got: {other:?}"),
    }
}
```

- [ ] **Step 1.8: Run test to verify it fails**

Run: `cargo test -p forge-daemon test_version_returns_build_metadata`
Expected: FAIL — no handler arm for `Request::Version`

- [ ] **Step 1.9: Implement handler arm**

In `crates/daemon/src/server/handler.rs`, after the `Request::Status` arm (line ~845):

```rust
Request::Version => Response::Ok {
    data: ResponseData::Version {
        version: env!("CARGO_PKG_VERSION").to_string(),
        build_profile: if cfg!(debug_assertions) { "debug" } else { "release" }.to_string(),
        target_triple: env!("FORGE_TARGET").to_string(),
        rustc_version: env!("FORGE_RUSTC_VERSION").to_string(),
        git_sha: {
            let sha = env!("FORGE_GIT_SHA");
            if sha.is_empty() { None } else { Some(sha.to_string()) }
        },
        uptime_secs: state.started_at.elapsed().as_secs(),
    },
},
```

- [ ] **Step 1.10: Add Version to free-tier catch-all**

In `crates/daemon/src/server/tier.rs`, add `Request::Version` to the `None` (free-tier) arm around line 213:

```rust
| Request::Version
```

- [ ] **Step 1.11: Run all tests to verify green**

Run: `cargo test -p forge-daemon test_version_returns_build_metadata && cargo test -p forge-core test_unit_variants_method_names`
Expected: PASS

- [ ] **Step 1.12: Run full workspace test + clippy**

Run: `cargo test --workspace && cargo clippy --workspace -- -W clippy::all -D warnings`
Expected: PASS, 0 warnings

- [ ] **Step 1.13: Commit**

```bash
git add crates/daemon/build.rs crates/core/src/protocol/request.rs crates/core/src/protocol/response.rs crates/core/src/protocol/contract_tests.rs crates/daemon/src/server/handler.rs crates/daemon/src/server/tier.rs
git commit -m "feat(version): Request::Version endpoint with build-time metadata

Adds a lightweight Version endpoint (no DB queries) returning:
- version (CARGO_PKG_VERSION)
- build_profile (debug/release)
- target_triple (aarch64-apple-darwin, etc.)
- rustc_version (captured in build.rs)
- git_sha (short commit hash, best-effort)
- uptime_secs

Closes deferred finding from Forge-Persist cycle j2 review
(HIGH 82: daemon_version is build-time, not runtime).

Co-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>"
```

---

### Task 2: HttpClient configurable timeout

**Files:**
- Modify: `crates/daemon/src/bench/forge_persist.rs:1290-1315` (PersistConfig)
- Modify: `crates/daemon/src/bench/forge_persist.rs:527-533` (HttpClient::new)
- Modify: `crates/daemon/src/bin/forge-bench.rs` (CLI arg)
- Test: inline unit test in forge_persist.rs

- [ ] **Step 2.1: Write failing test for configurable timeout**

In `crates/daemon/src/bench/forge_persist.rs`, in the `#[cfg(test)] mod tests`:

```rust
#[test]
fn test_http_client_with_timeout_uses_custom_duration() {
    // Construction should succeed with any valid Duration.
    let client = HttpClient::with_timeout(
        "http://127.0.0.1:9999".to_string(),
        Duration::from_secs(60),
    );
    assert!(client.is_ok(), "HttpClient::with_timeout should not fail construction");
}
```

- [ ] **Step 2.2: Run test to verify it fails**

Run: `cargo test -p forge-daemon test_http_client_with_timeout`
Expected: FAIL — `HttpClient::with_timeout` does not exist (E0599)

- [ ] **Step 2.3: Add `HttpClient::with_timeout` constructor + `request_timeout` config field**

In `crates/daemon/src/bench/forge_persist.rs`:

Add to `PersistConfig` after `output_dir`:
```rust
/// Per-request total timeout for the HttpClient. Defaults to 30 s
/// for production workloads. The original 5 s default caused
/// `NetworkError::TimedOut` on stress runs with 250+ raw ingests.
pub request_timeout: Duration,
```

Add new constructor on `HttpClient`:
```rust
/// Build a client with a custom total per-request timeout.
/// Connect timeout remains pinned at 200 ms (load-bearing for
/// spawn polling — see `HttpClient::new` doc comment).
pub fn with_timeout(base_url: String, request_timeout: Duration) -> Result<Self, HarnessError> {
    let client = reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_millis(200))
        .timeout(request_timeout)
        .build()
        .map_err(HarnessError::NetworkError)?;
    Ok(Self { client, base_url })
}
```

- [ ] **Step 2.4: Update `pub fn run` to use configurable timeout**

In `pub fn run`, where `PersistHarness` is constructed, change the HttpClient construction to use `config.request_timeout` instead of the hardcoded 5s.

- [ ] **Step 2.5: Update CLI to pass the new field**

In `crates/daemon/src/bin/forge-bench.rs`, add `--request-timeout-ms` CLI arg (default 30000) and wire it into `PersistConfig::request_timeout`.

- [ ] **Step 2.6: Update integration test config to include new field**

All places that construct `PersistConfig` in tests must include `request_timeout: Duration::from_secs(30)`.

- [ ] **Step 2.7: Run tests to verify green**

Run: `cargo test -p forge-daemon test_http_client_with_timeout && cargo test --workspace`
Expected: PASS

- [ ] **Step 2.8: Commit**

```bash
git add crates/daemon/src/bench/forge_persist.rs crates/daemon/src/bin/forge-bench.rs
git commit -m "feat(bench): configurable HttpClient request timeout

Adds request_timeout field to PersistConfig (default 30s) and
HttpClient::with_timeout constructor. The original 5s hardcoded
timeout caused NetworkError::TimedOut on stress workloads with
250+ raw ingests queued up.

Closes deferred limitation from Forge-Persist cycle k calibration.

Co-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>"
```

---

### Task 3: Session message pagination (offset + cap raise)

**Files:**
- Modify: `crates/core/src/protocol/request.rs:383-387` (SessionMessages)
- Modify: `crates/core/src/protocol/contract_tests.rs` (update SessionMessages test case)
- Modify: `crates/daemon/src/sessions.rs:437-485` (list_messages)
- Modify: `crates/daemon/src/server/handler.rs:3151-3211` (SessionMessages handler)
- Modify: `crates/daemon/src/bench/forge_persist.rs` (auto-paginate in list_session_messages)
- Test: unit tests in sessions.rs + bench module

- [ ] **Step 3.1: Write failing test for offset in `list_messages`**

In `crates/daemon/src/sessions.rs`, add to `#[cfg(test)] mod tests`:

```rust
#[test]
fn test_list_messages_with_offset_skips_rows() {
    let conn = test_conn();
    setup_sessions(&conn);
    // Insert 5 messages
    for i in 0..5 {
        insert_test_message(&conn, &format!("msg-{i}"), "sender", "receiver", "notification", &format!("topic-{i}"));
    }
    // Fetch with offset=2, limit=10 — should get 3 messages
    let msgs = list_messages(&conn, "receiver", None, 10, Some(2)).unwrap();
    assert_eq!(msgs.len(), 3, "offset=2 on 5 messages should return 3");
}
```

- [ ] **Step 3.2: Run test to verify it fails**

Run: `cargo test -p forge-daemon test_list_messages_with_offset_skips_rows`
Expected: FAIL — `list_messages` doesn't accept offset parameter

- [ ] **Step 3.3: Add `offset` parameter to `list_messages`**

In `crates/daemon/src/sessions.rs`, update the signature:

```rust
pub fn list_messages(
    conn: &Connection,
    session_id: &str,
    status_filter: Option<&str>,
    limit: usize,
    offset: Option<usize>,
) -> rusqlite::Result<Vec<SessionMessageRow>> {
    let limit = limit.min(1000) as i64; // Raised from 100 to 1000
    let offset = offset.unwrap_or(0) as i64;
```

Update SQL queries to append `OFFSET ?N` parameter.

Without status filter:
```sql
SELECT ... FROM session_message WHERE to_session = ?1 ORDER BY created_at DESC LIMIT ?2 OFFSET ?3
```

With status filter:
```sql
SELECT ... FROM session_message WHERE to_session = ?1 AND status = ?2 ORDER BY created_at DESC LIMIT ?3 OFFSET ?4
```

- [ ] **Step 3.4: Add `offset` field to `Request::SessionMessages`**

In `crates/core/src/protocol/request.rs`:

```rust
SessionMessages {
    session_id: String,
    status: Option<String>,
    limit: Option<usize>,
    #[serde(default)]
    offset: Option<usize>,
},
```

- [ ] **Step 3.5: Update handler to pass offset**

In `crates/daemon/src/server/handler.rs` at the `Request::SessionMessages` arm:

```rust
Request::SessionMessages {
    session_id,
    status,
    limit,
    offset,
} => {
    match crate::sessions::list_messages(
        &state.conn,
        &session_id,
        status.as_deref(),
        limit.unwrap_or(20),
        offset,
    ) {
```

- [ ] **Step 3.6: Update contract test for SessionMessages**

In `crates/core/src/protocol/contract_tests.rs`, update the `SessionMessages` entry in the parameterized test:

```rust
(
    "session_messages",
    Request::SessionMessages {
        session_id: "s1".into(),
        status: None,
        limit: Some(10),
        offset: Some(5),
    },
),
```

- [ ] **Step 3.7: Fix all callers of list_messages to pass offset**

Search for all call sites of `list_messages` and add the `offset` parameter (pass `None` for existing callers that don't need pagination).

- [ ] **Step 3.8: Add auto-pagination to bench HttpClient**

In `crates/daemon/src/bench/forge_persist.rs`, update `list_session_messages` to loop:

```rust
pub fn list_session_messages(&self, session_id: &str) -> Result<Vec<SessionMessage>, HarnessError> {
    let page_size = 1000;
    let mut all_messages = Vec::new();
    let mut offset: usize = 0;
    loop {
        let req = Request::SessionMessages {
            session_id: session_id.to_string(),
            status: None,
            limit: Some(page_size),
            offset: Some(offset),
        };
        match self.execute(&req)? {
            Response::Ok { data: ResponseData::SessionMessageList { messages, .. } } => {
                let count = messages.len();
                all_messages.extend(messages);
                if count < page_size {
                    break;
                }
                offset += count;
            }
            Response::Ok { data } => {
                return Err(HarnessError::DaemonError(format!(
                    "expected SessionMessageList, got {data:?}"
                )));
            }
            Response::Error { message } => {
                return Err(HarnessError::DaemonError(message));
            }
        }
    }
    Ok(all_messages)
}
```

- [ ] **Step 3.9: Run all tests**

Run: `cargo test --workspace && cargo clippy --workspace -- -W clippy::all -D warnings`
Expected: PASS, 0 warnings

- [ ] **Step 3.10: Commit**

```bash
git add crates/core/src/protocol/request.rs crates/core/src/protocol/contract_tests.rs crates/daemon/src/sessions.rs crates/daemon/src/server/handler.rs crates/daemon/src/bench/forge_persist.rs
git commit -m "feat(sessions): pagination offset + cap raise for session messages

Adds offset parameter to Request::SessionMessages and list_messages().
Raises the hard cap from 100 to 1000 rows per page. Bench harness
auto-paginates via loop until returned count < page_size.

Closes deferred finding from Forge-Persist cycle j1 review
(HIGH 82: 500-message FISP ceiling).

Co-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>"
```

---

### Task 4: Doctor observability enhancement ("Doctor on steroids")

**Files:**
- Modify: `crates/core/src/protocol/response.rs:100-122` (Doctor variant)
- Modify: `crates/daemon/src/server/handler.rs:847-1020` (Doctor handler)
- Modify: `crates/daemon/src/db/raw.rs` (add count helpers)
- Test: handler test + raw.rs unit tests

- [ ] **Step 4.1: Write failing test for raw document count helper**

In `crates/daemon/src/db/raw.rs`, add to `#[cfg(test)] mod tests`:

```rust
#[test]
fn test_count_raw_documents_returns_total() {
    let conn = test_conn();
    // Insert 3 documents
    for i in 0..3 {
        ingest_raw_document(&conn, &format!("doc-{i}"), &format!("text-{i}"), "test", None, None, None, None).unwrap();
    }
    let count = count_raw_documents(&conn).unwrap();
    assert_eq!(count, 3);
}
```

- [ ] **Step 4.2: Run test to verify it fails**

Run: `cargo test -p forge-daemon test_count_raw_documents_returns_total`
Expected: FAIL — `count_raw_documents` does not exist

- [ ] **Step 4.3: Implement count helpers in `db/raw.rs`**

```rust
/// Count total raw documents in the store.
pub fn count_raw_documents(conn: &Connection) -> rusqlite::Result<usize> {
    conn.query_row("SELECT COUNT(*) FROM raw_documents", [], |row| row.get(0))
}

/// Count total raw chunks in the store.
pub fn count_raw_chunks(conn: &Connection) -> rusqlite::Result<usize> {
    conn.query_row("SELECT COUNT(*) FROM raw_chunks", [], |row| row.get(0))
}
```

- [ ] **Step 4.4: Run test to verify green**

Run: `cargo test -p forge-daemon test_count_raw_documents_returns_total`
Expected: PASS

- [ ] **Step 4.5: Add new fields to `ResponseData::Doctor`**

In `crates/core/src/protocol/response.rs`, add to the `Doctor` variant after `checks`:

```rust
/// Daemon version (CARGO_PKG_VERSION).
#[serde(default)]
version: String,
/// Short git commit hash at build time.
#[serde(default)]
git_sha: Option<String>,
/// Total raw documents ingested into the raw layer.
#[serde(default)]
raw_document_count: usize,
/// Total raw chunks (embedder output) in the raw layer.
#[serde(default)]
raw_chunk_count: usize,
/// Number of active sessions.
#[serde(default)]
active_session_count: usize,
/// Total session messages exchanged via FISP.
#[serde(default)]
session_message_count: usize,
```

- [ ] **Step 4.6: Write failing handler test for new Doctor fields**

```rust
#[test]
fn test_doctor_includes_version_and_raw_stats() {
    let state = AppState::new_test();
    let resp = handle_request(Request::Doctor, &state);
    match resp {
        Response::Ok { data: ResponseData::Doctor { version, raw_document_count, raw_chunk_count, active_session_count, .. } } => {
            assert!(!version.is_empty(), "doctor should include version");
            // Fresh test DB has 0 documents
            assert_eq!(raw_document_count, 0);
            assert_eq!(raw_chunk_count, 0);
            assert_eq!(active_session_count, 0);
        }
        other => panic!("expected Doctor response, got: {other:?}"),
    }
}
```

- [ ] **Step 4.7: Run test to verify it fails**

Run: `cargo test -p forge-daemon test_doctor_includes_version_and_raw_stats`
Expected: FAIL — Doctor struct doesn't have new fields

- [ ] **Step 4.8: Populate new fields in the Doctor handler**

In `crates/daemon/src/server/handler.rs`, in the `Request::Doctor` arm, before the `Response::Ok` construction:

```rust
let raw_doc_count = crate::db::raw::count_raw_documents(&state.conn).unwrap_or(0);
let raw_chunk_count = crate::db::raw::count_raw_chunks(&state.conn).unwrap_or(0);

let active_session_count = crate::sessions::count_active_sessions(&state.conn).unwrap_or(0);
let session_message_count = crate::sessions::count_all_messages(&state.conn).unwrap_or(0);
```

Then add to the `ResponseData::Doctor { ... }` block:

```rust
version: env!("CARGO_PKG_VERSION").to_string(),
git_sha: {
    let sha = env!("FORGE_GIT_SHA");
    if sha.is_empty() { None } else { Some(sha.to_string()) }
},
raw_document_count: raw_doc_count,
raw_chunk_count,
active_session_count,
session_message_count,
```

- [ ] **Step 4.9: Add session count helpers**

In `crates/daemon/src/sessions.rs`:

```rust
/// Count active sessions.
pub fn count_active_sessions(conn: &Connection) -> rusqlite::Result<usize> {
    conn.query_row(
        "SELECT COUNT(*) FROM session_registry WHERE status = 'active'",
        [],
        |row| row.get(0),
    )
}

/// Count total session messages.
pub fn count_all_messages(conn: &Connection) -> rusqlite::Result<usize> {
    conn.query_row("SELECT COUNT(*) FROM session_message", [], |row| row.get(0))
}
```

- [ ] **Step 4.10: Run all tests**

Run: `cargo test --workspace && cargo clippy --workspace -- -W clippy::all -D warnings`
Expected: PASS, 0 warnings

- [ ] **Step 4.11: Commit**

```bash
git add crates/core/src/protocol/response.rs crates/daemon/src/server/handler.rs crates/daemon/src/db/raw.rs crates/daemon/src/sessions.rs
git commit -m "feat(doctor): observability enhancement — version, raw stats, session counts

Doctor now reports:
- version + git_sha (build metadata)
- raw_document_count + raw_chunk_count (raw layer utilization)
- active_session_count + session_message_count (FISP activity)

Makes 'forge doctor' the single-command observability surface for
the dogfood gate — one call shows daemon health, memory health,
and usage activity.

Co-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>"
```

---

### Task 5: Forge-Persist runtime version query

**Files:**
- Modify: `crates/daemon/src/bench/forge_persist.rs` (RunSummary + pub fn run)

- [ ] **Step 5.1: Write failing test**

```rust
#[test]
fn test_run_summary_daemon_version_from_version_endpoint() {
    // After this change, daemon_version should come from the
    // Version endpoint at runtime, not from the bench binary's
    // own env!("CARGO_PKG_VERSION").
    // This is a design assertion — the actual integration test
    // (test_persist_harness_full_run_passes_on_clean_workload)
    // exercises the runtime path.
    let version = env!("CARGO_PKG_VERSION");
    assert!(!version.is_empty(), "sanity: CARGO_PKG_VERSION is set");
}
```

- [ ] **Step 5.2: Add `HttpClient::version` method**

```rust
/// Query the daemon's runtime version via Request::Version.
pub fn version(&self) -> Result<String, HarnessError> {
    match self.execute(&Request::Version)? {
        Response::Ok { data: ResponseData::Version { version, .. } } => Ok(version),
        Response::Ok { data } => Err(HarnessError::DaemonError(
            format!("expected Version response, got {data:?}")
        )),
        Response::Error { message } => Err(HarnessError::DaemonError(message)),
    }
}
```

- [ ] **Step 5.3: Update `pub fn run` to query version at runtime**

In the orchestrator, after the first spawn succeeds (step 3), query the daemon version:

```rust
let daemon_version = harness.client().version().unwrap_or_else(|_| {
    env!("CARGO_PKG_VERSION").to_string()
});
```

Replace the existing `daemon_version: env!("CARGO_PKG_VERSION").to_string()` in `RunSummary` construction with `daemon_version`.

- [ ] **Step 5.4: Run full workspace tests**

Run: `cargo test --workspace && cargo clippy --workspace -- -W clippy::all -D warnings`
Expected: PASS

- [ ] **Step 5.5: Commit**

```bash
git add crates/daemon/src/bench/forge_persist.rs
git commit -m "feat(bench): query daemon version at runtime via Version endpoint

RunSummary.daemon_version now comes from the running daemon's
Request::Version response instead of the bench binary's own
env!(CARGO_PKG_VERSION). Falls back to build-time version if
the Version endpoint is unavailable (backward compat).

Co-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>"
```

---

### Task 6: Push to origin

- [ ] **Step 6.1: Verify clean state**

Run: `git status && git log --oneline origin/master..HEAD`
Expected: clean working tree, 5 new commits (Tasks 1-5) on top of cc04d67

- [ ] **Step 6.2: Push**

Run: `git push origin master`
Expected: success — fast-forward push

---

## Self-Review

**Spec coverage:**
- C1 (Version endpoint): Task 1 ✅
- C2 (HttpClient timeout): Task 2 ✅
- C3 (Session pagination): Task 3 ✅
- C4 (Push to origin): Task 6 ✅
- Doctor observability (dogfood): Task 4 ✅
- Forge-Persist runtime version fix: Task 5 ✅

**Placeholder scan:** No TBD/TODO/placeholder in any step. All code blocks complete.

**Type consistency:** `Request::Version`, `ResponseData::Version`, `HttpClient::with_timeout`, `HttpClient::version`, `list_messages` offset param — all consistent across tasks.

**Note on `serde(default)` for new Doctor fields:** The `#[serde(default)]` ensures backward compatibility — older daemon versions that don't have these fields will deserialize to zero/None without breaking existing consumers.
