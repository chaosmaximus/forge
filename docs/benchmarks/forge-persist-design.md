# Forge-Persist — design gate document

**Status:** DRAFT — design gate, pending founder approval AND adversarial review. No implementation begins until both gates pass.

**Scope:** Phase 2A-1 of [phase-2-plan.md](./phase-2-plan.md) — first of five custom Forge-* benchmarks.

**Predecessor work:** Wave 1 benchmark initiative (hybrid BM25+KNN raw search) landed 2026-04-14. See [improvement-roadmap-2026-04-13.md](./improvement-roadmap-2026-04-13.md).

---

## 1. Thesis

"Local-first cognitive infrastructure is meaningless if the daemon cannot survive abrupt termination and replay correctly."

Forge-Persist is the narrowest of the five Forge-* benchmarks. It does NOT test retrieval quality, extraction pipeline, or multi-agent coordination. It tests one property: **after a SIGKILL-restart cycle, does the daemon recover every operation that was acknowledged before the kill, without corruption, without loss, and within a bounded time?**

**Why this is first:** it validates the subprocess-spawn + daemon-lifecycle pattern that Forge-Multi and Forge-Transfer will reuse. The plan doc flags this as the framework shakedown: *"if the Forge-Persist harness is painful to build, the framework design needs rework before 2A-2 begins."* Important scope narrowing: the shakedown validates the subprocess lifecycle primitive, NOT a reusable shared-module framework. Module extraction happens in Forge-Tool (2A-2) only after we have a second call site to inform the boundary.

---

## 2. Reconnaissance summary

Empirical facts from a reconnaissance pass over `crates/daemon/` that shape every design decision below. File:line citations are load-bearing — any disagreement between this summary and the code means this document is wrong and must be revised.

1. **Daemon entrypoint:** `crates/daemon/src/main.rs`. Tokio runtime. Spawns 9 background workers, opens SQLite, acquires PID lock, binds socket + optional HTTP server.
2. **Signal handling:** Ctrl+C only (main.rs ~line 316). No SIGTERM handler. SIGKILL cannot be handled — that is the test surface.
3. **PID lock:** `fs2::FileExt::try_lock_exclusive` at `crates/daemon/src/main.rs:17-84`. File path is `default_pid_path()` which resolves to `forge_dir() + "/forge.pid"`. Stale-lock cleanup at `main.rs:46-70` is gated on `#[cfg(unix)]` and uses `/proc/{pid}` existence for liveness. **This is a latent bug on macOS** — `/proc` does not exist on macOS, so `Path::new("/proc/{pid}").exists()` always returns `false`, meaning the stale-cleanup path treats any lock-held PID as dead and unconditionally unlinks the file. For Forge-Persist's happy path this is harmless (the second daemon spawn acquires the lock immediately because SIGKILL releases the kernel advisory lock, so the cleanup path is never entered). But it is a real daemon bug that should be fixed opportunistically via `kill(pid, 0)` (signal-0 probe, portable across Unix). See §13. **Also:** PID path is NOT overridable by env var — `forge_dir()` in `crates/core/src/paths.rs:5-8` hardcodes `$HOME/.forge`.
4. **DB + socket paths ARE overridable:** `FORGE_DB` and `FORGE_SOCKET` env vars, read in `main.rs:175-176`.
5. **HTTP server knobs:** `FORGE_HTTP_ENABLED`, `FORGE_HTTP_BIND`, `FORGE_HTTP_PORT` (see `crates/daemon/src/config.rs:918-930`). Toggling HTTP on gives us `POST /api` with `{method, params}` JSON per CLAUDE.md.
6. **WAL config:** `PRAGMA journal_mode=WAL` applied at `crates/daemon/src/db/schema.rs:41,141`. `synchronous` not set explicitly → defaults to `FULL` in WAL mode. `wal_autocheckpoint` not overridden → defaults to 1000 pages. Standard SQLite crash-safety semantics apply.
7. **Background workers:** 9 of them (watcher, extractor, embedder, consolidator, indexer, perception, disposition, diagnostics, reaper). **No persistent task queues.** All rebuild state from SQL on restart (embedder re-scans `memory WHERE embedding IS NULL`, etc.). Implication: worker in-memory state is NOT ground truth; only DB rows are.
8. **sqlite-vec KNN indices:** `memory_vec`, `raw_chunks_vec`, `code_vec` are stored inside the SQLite file. Fully persistent via standard WAL recovery — no separate rebuild path.
9. **FISP messages:** `session_message` table with status enum (`pending` → `delivered`). Persistent across restarts. Undelivered messages survive.
10. **Existing bench harnesses** (`crates/daemon/src/bench/longmemeval.rs`, `locomo.rs`) are **pure in-process SQLite benches** — no daemon subprocess spawn. Forge-Persist **cannot reuse this pattern**; it requires new infrastructure.
11. **Existing integration tests** (`crates/daemon/tests/*.rs`) use `DaemonState::new(":memory:")` and call request handlers directly. No existing pattern for spawning the real `forge-daemon` binary from a test. Forge-Persist **introduces this pattern for the codebase**.
12. **Core protocol** (`crates/core/src/protocol/request.rs`) defines the Request enum. Verified variants for this bench: `Remember` (line 43), `Recall` (line 54), `SessionSend` (line 360), `RawIngest` (line 858), `Health` (response.rs:85). Session pre-creation uses `RegisterSession` (NOT a non-existent `SessionCreate`). Read-back of FISP messages uses `SessionMessages`.
13. **Option B isolation confirmation via socket.rs:** `crates/daemon/src/server/socket.rs:72-73` constructs the PID path via `forge_core::forge_dir()` directly. After the proposed `FORGE_DIR` patch, this call site picks up the override automatically — confirming Option B propagates through the daemon's socket server.
14. **TLS isolation gap** — `crates/daemon/src/server/tls.rs:152-157` defines a separate `dirs_for_forge_home()` that resolves `HOME + "/.forge"` directly, NOT through `forge_core::paths::forge_dir()`. Adding `FORGE_DIR` env var support to `paths.rs` alone leaves `tls.rs` hardcoded. **This is a required co-change**, not optional.
15. **Synchronous writes confirmed:**
   - `SessionSend` is synchronous: `crates/daemon/src/sessions.rs:323` does a direct `INSERT` before returning `Ok(id)`, and the handler calls it inline (not via WriterActor). HTTP 200 implies persisted row.
   - `Remember` writes the `memory` row synchronously before HTTP 200. The embedder worker writes `memory_vec` asynchronously AFTER the handler returns — see §6.1 scope limitation.
16. **`Recall` does NOT query `memory_vec`.** The HTTP `Recall` handler passes `query_embedding: None` to `hybrid_recall`, which only consults `memory_vec` when an embedding is provided (`crates/daemon/src/recall.rs:187-203`). A `Recall` call therefore reads only the BM25 + `memory` table surfaces — it does NOT verify that the vector row for a given memory id exists. This shapes the scoring scope: Forge-Persist verifies `memory` table durability, not `memory_vec` durability. See §6.1 and §11.

---

## 3. Core architectural commitment: real subprocess

**The single most important design decision:** Forge-Persist spawns the real `forge-daemon` binary as a child process and kills it with real SIGKILL via `Child::kill()`. No in-process `DaemonState::drop()` simulation. No library-only test that exits via `std::process::exit`. No "abort the tokio runtime" pattern.

**Rationale:**
- SIGKILL bypasses all Rust destructors. An in-process "drop the runtime" test exercises destructors in some order. These are fundamentally different code paths; the test surface we care about is the one destructors never touch.
- WAL corruption, stale PID lock files, half-committed transactions, and half-flushed OS page cache pages are only reproducible with a real OS-level process kill.
- The audience (open-source evaluators, paying customers, investor deck) must find the bench believable. Simulation gives weaker evidence and invites nit-picks.
- Forge-Multi and Forge-Transfer will reuse this harness infrastructure. Building it now on the simplest bench is the correct place to absorb the framework cost.

**The cost we are accepting:** we build new test infrastructure that does not exist in the codebase today. That is what the plan doc called the framework shakedown.

---

## 4. Isolation strategy (how to run a test daemon without stomping on the user's real daemon)

**The problem:** a second `forge-daemon` will (a) collide on the PID lock at `~/.forge/forge.pid`, (b) stomp `~/.forge/forge.db`, (c) race on the default socket, (d) conflict on port 8420, and (e) potentially leak state back to the user's real setup. All five are unacceptable. All must be isolated.

**Overridable via existing env vars:** DB path (`FORGE_DB`), socket path (`FORGE_SOCKET`), HTTP port (`FORGE_HTTP_PORT`), HTTP bind (`FORGE_HTTP_BIND`), HTTP enable (`FORGE_HTTP_ENABLED`).

**Not overridable:** PID file path. `main.rs:197` calls `default_pid_path()` with no env check; `forge_core::paths::forge_dir()` in turn hardcodes `HOME + /.forge`.

### Three isolation options

**Option A — Override HOME to a TempDir.** Set `HOME=<tempdir>` in the spawned daemon's environment. `forge_dir()` resolves to `<tempdir>/.forge`; PID file lands there. Zero daemon code changes.
- Possible side effect: the fastembed cache may redirect into the TempDir, forcing a fresh weight download (~90 MB, ~10 s) per run. **Not verified** — adversarial review flagged that fastembed-rs uses the `dirs` crate's `cache_dir()`, which on macOS is `~/Library/Caches` (derived from HOME via `dirs`, but whether it responds to a runtime HOME change depends on the library's resolution path). If Option A is chosen, trace the actual fastembed cache resolution before locking a mitigation.
- Possible mitigation (if cache redirects): symlink the real cache into the TempDir before spawn, OR use `FORGE_TEST_FASTEMBED=1` (mentioned in `embed/minilm.rs:88`).
- Risk: other HOME-derived paths (KUBECONFIG, etc.) quietly redirect with unknown side effects on worker behavior.
- Verdict: feasible but leaky. Not recommended.

**Option B — Add `FORGE_DIR` env var to `forge_core::paths::forge_dir()` AND update `crates/daemon/src/server/tls.rs:dirs_for_forge_home()` to flow through the same resolver.** Change 1: `pub fn forge_dir() -> String { std::env::var("FORGE_DIR").unwrap_or_else(|_| format!("{}/.forge", std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string()))) }`. Change 2: `tls.rs:dirs_for_forge_home()` must be rewritten to call `forge_core::paths::forge_dir()` (or read `FORGE_DIR` itself). The `default_pid_path`, `default_db_path`, `default_socket_path`, and `socket.rs:72-73` call site all flow through `forge_core::forge_dir()` already — those are fine after change 1. TLS path is the exception and requires change 2.
- Scope: ~5 LoC in `crates/core/src/paths.rs` + test + ~10 LoC in `tls.rs` + test. Both changes land in ONE commit so isolation is complete atomically — partial landing leaves a broken state.
- Side effect: none. HOME stays untouched. Fastembed cache stays warm.
- Verdict: cleanest. The daemon should have had this env var from day one for testability; adding it is a legitimate feature driven by the first Forge-* bench's requirements.

**Option C — Accept PID stomp.** Not viable. The fs2 advisory lock will reject the second daemon instance. Non-starter.

**Recommendation: Option B.** Ship the `FORGE_DIR` env var as its own commit BEFORE the Forge-Persist harness commits, so the isolation mechanism is a clean, independently testable primitive. TDD cycle for the `forge_dir()` change lives in `crates/core/src/paths.rs` tests (there is already a `#[cfg(test)] mod tests` at line 26).

**Fallback if founder rejects Option B:** Option A with `FORGE_TEST_FASTEMBED=1` set explicitly for the subprocess.

---

## 5. Dataset shape

A Forge-Persist run executes a seeded deterministic workload against an isolated daemon instance, then kills and restarts.

### 5.1 Seed and RNG

- **PRNG:** ChaCha20 from the `rand_chacha` crate, seeded from a `u64` CLI flag. Deterministic across runs, machines, and platforms.
- **Governs:** operation generation (content), interleaving order, and kill offset.

### 5.2 Operation types

Three categories that exercise different persistence surfaces:

| Op kind     | Method      | Tables touched                                                      | Ack criterion             |
|-------------|-------------|---------------------------------------------------------------------|---------------------------|
| `Remember`  | `Remember`  | `memory`, `memory_vec` (via embedder worker, async)                 | HTTP 200 with memory id   |
| `IngestRaw` | `RawIngest` | `raw_documents`, `raw_chunks`, `raw_chunks_fts`, `raw_chunks_vec`   | HTTP 200 with document id |
| `FispSend`  | `SessionSend` | `session_message`, `session` (updated_at)                         | HTTP 200 with message id  |

### 5.3 Workload parameters (CLI flags with defaults)

- `--memories N` (default 100) — number of `Remember` ops
- `--chunks K` (default 50) — number of `IngestRaw` ops
- `--fisp-messages J` (default 20) — number of `SessionSend` ops
- `--seed S` (default 42) — PRNG seed
- `--kill-after F` (default 0.5) — fraction of HTTP-200-acked ops at which SIGKILL fires

**Total ops = N + K + J.** Interleaving is a ChaCha-shuffled order over the total op set. Kill fires after `floor(F * (N+K+J))` ops have returned HTTP 200.

**Pre-session setup:** before the workload begins, the harness calls `RegisterSession` five times to create a pool of named sessions. This is the correct protocol variant (verified in `crates/core/src/protocol/request.rs`). `SessionSend` then has valid `from_session` / `to_session` targets.

**Read-back method:** post-restart verification for FISP messages uses `SessionMessages` (verified in the Request enum — not a `list_session_messages` placeholder). Memory read-back uses `Recall` with an appropriate query. Raw document read-back uses the existing raw-document listing method (exact variant to be pinned during TDD — first cycle).

### 5.4 Ground-truth tracking

The harness maintains an in-process `ExpectedState` struct as ops are acked:

```rust
struct ExpectedState {
    acked_memories: HashMap<String, MemoryExpectation>,
    acked_documents: HashMap<String, DocumentExpectation>,
    acked_messages: HashMap<String, MessageExpectation>,
    acked_order: Vec<AckedOp>,
}
```

Each expectation captures the id + a content hash (defined precisely in §6.2) + (for FISP) the expected `status`. An op is added to ground truth ONLY after the daemon returns HTTP 200. Operations in-flight at kill time are NOT expected to survive — that is correct behavior (no ack = no durability contract).

**Single ack criterion, not two-step.** An earlier draft proposed a "two-step ack" that polled `Recall` after HTTP 200 to confirm the embedding row was written. The adversarial review caught that `Recall` does not query `memory_vec` (it passes `query_embedding: None` to `hybrid_recall`), so the poll would add nothing. The single HTTP-200 ack is sufficient for the synchronous writes (`memory`, `raw_documents`, `raw_chunks`, `session_message`). Asynchronous worker writes (`memory_vec`, `raw_chunks_vec` via the embedder) are explicitly scoped OUT — see §6.1 and §11.

### 5.5 Content generation

Each op generates deterministic content from the seeded RNG:

- `Remember`: a dummy text body (~100 characters) + a category from a fixed vocabulary + 2-3 tags
- `IngestRaw`: a 200-character Lorem ipsum-derivative passage as a raw document
- `SessionSend`: from-session and to-session ids from the pre-created pool; a deterministic string body

Content is NOT designed for semantic retrieval quality. It exists only so the harness can verify bit-exact recall after restart.

---

## 6. Scoring rubric

Three metrics, all required to pass.

### 6.1 Recovery rate
```
recovery_rate = |acked ∩ post_restart_visible| / |acked|
```
Where `acked` is the union of the three acked-id sets and `post_restart_visible` is the same union after querying the restarted daemon via public read methods: `Recall` (for memories), `SessionMessages` (for FISP messages), and the existing raw-document listing method (for ingested documents).

**Scope limitation:** The recovery rate only verifies the synchronously-written tables: `memory`, `raw_documents`, `raw_chunks`, `session_message`. It does NOT verify `memory_vec` or `raw_chunks_vec` rows written by the async embedder worker. See §11. If the founder wants embedding-row durability verified, the harness adds a post-restart direct SQL probe as an additional metric (out of first-iteration scope; tracked as Q9).

**Pass threshold:** `recovery_rate >= 0.99`. The 1% tolerance is reserved for HTTP-client-level transient failures (e.g., connection reset races during the kill transition) that are neither the daemon's nor the harness's fault. A run whose recovery rate drops below 1.0 without such a root cause is investigated per-run, not accepted silently.

### 6.2 Consistency rate
```
consistency_rate = |correctly_matched| / |post_restart_visible|
```
Where `correctly_matched` means the recovered row has the same id AND the same content hash as recorded pre-kill. No tolerance for orphan rows (ids present post-restart that weren't acked pre-kill) — they count against consistency.

**Content hash definition (precise).** `content_hash = hex(sha256(canonical_payload))` where `canonical_payload` is the UTF-8 byte representation of the op-kind-specific canonical string:

- `Remember` — the `content` string from the request body, unchanged, as UTF-8 bytes
- `IngestRaw` — the full document body string from the request body, unchanged, as UTF-8 bytes
- `SessionSend` — `serde_json::to_string(&parts)` where `parts` is the `Vec<MessagePart>` from the request (canonical serialization — no trailing whitespace, deterministic key ordering)

Post-restart verification recomputes the hash from the daemon's returned content and compares as a byte-exact string match. The hash algorithm (SHA-256) and the exact field selection are pinned in the design — any future change requires a design-doc amendment and a version bump of `summary.json`'s `hash_scheme` field.

**Pass threshold:** `consistency_rate == 1.00`. Anything less is corruption or phantom-write — unconditional fail.

### 6.3 Recovery time
```
recovery_time_ms = first_health_ok_timestamp - second_daemon_spawn_timestamp
```

Harness records wall clock at `Command::spawn()`, polls `POST /api method=Health` at 50 ms intervals until it receives HTTP 200, subtracts.

**Pass threshold (provisional):** `recovery_time_ms < 5000`. See §12 Q4 — the threshold is calibrated empirically on the first run, then locked in a decision log entry.

### 6.4 Composite result

A run passes iff all three metrics meet their thresholds. A run that passes recovery but fails consistency is a FAIL — corruption is worse than loss. This rule is surfaced in the CLI output.

---

## 7. Harness architecture

### 7.1 Module layout

```
crates/daemon/src/bench/forge_persist.rs       ← single file for first iteration
crates/daemon/src/bin/forge-bench.rs           ← new ForgePersist subcommand
crates/daemon/tests/forge_persist_harness.rs   ← integration test
```

The plan doc §2A-1 proposed `crates/daemon/src/bench/datasets/persist.rs` for the dataset generator. **I am recommending against that split for the first iteration** — keep everything in one file, extract modules in Forge-Tool (2A-2) once we know which boundaries are load-bearing. This honors the "no speculative builds" principle. See §12 Q7.

**What the framework shakedown actually validates (scope narrowing):** the single-file approach validates the *subprocess lifecycle primitive* — spawn the daemon, poll health, execute ops, kill, restart, re-poll. This is the pattern Forge-Multi and Forge-Transfer will reuse. What it does NOT validate is a reusable shared-module framework (dataset generators, scoring helpers, ground-truth trackers as library types). Those emerge in Forge-Tool when there's a second call site to inform the boundary.

Additionally, `crates/daemon/src/bench/mod.rs` gains one line: `pub mod forge_persist;`.

### 7.2 Daemon subprocess lifecycle

The harness defines a `PersistHarness` struct that owns:
- `tempdir: TempDir` — the daemon's isolated state root (`$FORGE_DIR` target)
- `port: u16` — a free port discovered via `TcpListener::bind(("127.0.0.1", 0))` + `local_addr()` + drop
- `base_url: String` — `http://127.0.0.1:<port>`
- `daemon_bin: PathBuf` — path to the `forge-daemon` binary
- `child: Option<Child>` — current daemon process handle (None between kill and restart)
- `env: Vec<(String, String)>` — env vars applied on every spawn

Lifecycle methods:
- `start()` — spawn the daemon with the env vars + base command; poll `Health` until 200 or timeout
- `execute_op(op: &Op) -> Result<AckedOp>` — issue the op via `reqwest::blocking::Client::post`, return id + content hash on HTTP 200
- `kill()` — `child.kill()` (Unix: SIGKILL via signal 9), then `child.wait()` to reap the zombie
- `restart()` — start() again; measure elapsed time from spawn to first Health 200
- `verify_state(expected: &ExpectedState) -> RecoveryReport` — query all three surfaces, produce metrics
- `Drop` — if child is alive, kill it; TempDir is cleaned automatically by its own Drop

### 7.3 HTTP client choice

**`reqwest::blocking::Client`** — simple, synchronous, well-documented. The harness is correctness-focused, not latency-sensitive; synchronous code is easier to reason about in tests. Per-op timeout set to something generous (say, 10 s) to avoid false negatives on slow machines.

**Cargo.toml change required.** The daemon crate's current `reqwest` dependency at `crates/daemon/Cargo.toml` does NOT enable the `blocking` feature — it's currently `default-features = false, features = ["json", "rustls-tls"]`. The design requires adding `"blocking"` to that feature list as part of the TDD prerequisite commits. This is a one-line Cargo.toml change. Adversarial review caught this — I initially assumed blocking was available.

**Alternative rejected:** `ureq` (synchronous, no feature flag needed, smaller dep tree) was considered but requires adding a net-new dependency. Since `reqwest` is already in the workspace, enabling its `blocking` feature is the smaller surface-area change. If the founder prefers the smaller dep tree, flip to `ureq` at the design gate.

Not `tokio::reqwest` — we do not need async concurrency here.

### 7.4 Daemon binary discovery

- **In the `forge-bench` CLI:** `--daemon-bin <PATH>` flag. Default discovery order: (1) explicit `$FORGE_BENCH_DAEMON_BIN` env var, (2) `which("forge-daemon")` on PATH, (3) error with a helpful message.
- **In the integration test:** `env!("CARGO_BIN_EXE_forge-daemon")` — Cargo automatically builds the binary and injects this env var for any integration test in the `daemon` crate. No manual setup required.

### 7.5 Isolation guarantees

Each harness invocation creates a fresh TempDir and fresh port. Multiple concurrent harness runs are safe — no shared state, no PID conflict, no port conflict. The user's running daemon at `~/.forge/forge.pid` + port 8420 is untouched.

The TempDir is cleaned up via `Drop` at the end of each run. If the harness crashes, the TempDir persists intentionally (helps post-mortem debugging). A bench-cleanup task could prune old TempDirs; that is out of scope for the first iteration.

---

## 8. CLI subcommand

Added to `crates/daemon/src/bin/forge-bench.rs` `Commands` enum:

```rust
ForgePersist {
    #[arg(long, default_value = "100")]
    memories: usize,

    #[arg(long, default_value = "50")]
    chunks: usize,

    #[arg(long, default_value = "20")]
    fisp_messages: usize,

    #[arg(long, default_value = "42")]
    seed: u64,

    #[arg(long, default_value = "0.5")]
    kill_after: f64,

    #[arg(long, default_value = "bench_results")]
    output: PathBuf,

    #[arg(long)]
    daemon_bin: Option<PathBuf>,

    #[arg(long, default_value = "5000")]
    recovery_timeout_ms: u64,

    #[arg(long, default_value = "10000")]
    worker_catchup_ms: u64,
},
```

Dispatch in `main()` follows the same pattern as `Longmemeval` / `Locomo` — a single `run_forge_persist` function call. `#[allow(clippy::too_many_arguments)]` applies consistently with the other subcommands.

### 8.1 Outputs

On completion, the harness writes to `bench_results/forge_persist_<unix_secs>/`:

- `summary.json` — scalar metrics + config echo + daemon version + wall time + pass bit
- `pre_kill_state.jsonl` — one row per acked op: id, kind, content_hash, acked_at_ms
- `post_restart_state.jsonl` — one row per recovered row: id, kind, content_hash, matched_flag
- `repro.sh` — exact command to reproduce (sibling to existing bench repro scripts)
- `daemon_stderr.log` — daemon's stderr captured pre-kill and post-restart, for post-mortem

Example `summary.json`:

```json
{
  "seed": 42,
  "memories": 100,
  "chunks": 50,
  "fisp_messages": 20,
  "kill_after": 0.5,
  "total_ops": 170,
  "acked_pre_kill": 85,
  "recovered": 85,
  "matched": 85,
  "recovery_rate": 1.0,
  "consistency_rate": 1.0,
  "recovery_time_ms": 1420,
  "pass": true,
  "wall_time_ms": 14360,
  "daemon_version": "forge-daemon 0.7.0"
}
```

---

## 9. Integration test shape

`crates/daemon/tests/forge_persist_harness.rs`:

```rust
use std::time::Duration;
use forge_daemon::bench::forge_persist::{run, Config};

#[test]
fn forge_persist_recovers_small_workload() {
    let bin = env!("CARGO_BIN_EXE_forge-daemon");
    let result = run(Config {
        daemon_bin: std::path::PathBuf::from(bin),
        memories: 10,
        chunks: 5,
        fisp_messages: 3,
        seed: 1,
        kill_after: 0.5,
        recovery_timeout: Duration::from_secs(30),
        worker_catchup: Duration::from_secs(5),
        output_dir: None,
    })
    .expect("harness should complete without crashing");

    assert!(
        result.recovery_rate >= 0.99,
        "recovery_rate {} < 0.99",
        result.recovery_rate
    );
    assert_eq!(result.consistency_rate, 1.0);
    assert!(result.recovery_time < Duration::from_secs(10));
}
```

Three properties:

1. **Uses the real daemon binary** via `CARGO_BIN_EXE_forge-daemon`.
2. **Small workload** (10+5+3 ops, seed 1) for fast test runs (~5 s target).
3. **Looser recovery threshold** (10 s vs 5 s) because CI timing is noisier than a dev machine. The CLI run enforces the tighter production threshold.

`output_dir: None` means "do not write files to disk" — the test mode returns metrics in memory only.

**Daemon startup cost that affects this threshold:** `main.rs` startup includes tokio runtime init, PID lock, SQLite open + schema creation (idempotent but still real work), 9 worker spawns, embedder init (fastembed weight load from cache or disk, small if warm), HTTP server binding, project ingestion, and domain DNA detection. On a warm M1 Pro this totals well under a second; on a cold GitHub Actions macOS runner it may be 3-8 seconds. The 10 s integration-test threshold is ASPIRATIONAL and must be validated on the target CI platform before the test is enabled in CI. First landing: test runs locally only (no CI inclusion) until the threshold is calibrated against a real CI run. This is documented in the test file itself with a TODO comment citing this design doc section.

---

## 10. Reproduction contract

Every run produces a `repro.sh` matching the existing bench repro convention:

```bash
#!/usr/bin/env bash
# Generated by forge-bench forge-persist on <timestamp>
set -euo pipefail
cargo build --release --bin forge-bench --bin forge-daemon
./target/release/forge-bench forge-persist \
  --memories 100 \
  --chunks 50 \
  --fisp-messages 20 \
  --seed 42 \
  --kill-after 0.5 \
  --output bench_results
```

The founder runs `repro.sh` once manually as part of the reproduction gate.

---

## 11. Non-goals (explicit exclusions)

What Forge-Persist deliberately does NOT test:

- **Retrieval quality** — tested by LongMemEval, LoCoMo, and the other Forge-* benches
- **Extraction pipeline correctness** — separate concern; tested indirectly by Forge-Tool
- **Multi-process coordination semantics** — Forge-Multi's territory
- **Network partition tolerance** — not relevant to a local-first daemon
- **Disk corruption below SQLite** — trusts SQLite WAL guarantees; not a SQLite reliability test
- **Performance throughput** — correctness benchmark, not ops/sec
- **Long-running workload** — hundreds to low thousands of ops; enough to stress WAL but not enough to stress auto-checkpoint or vacuum
- **Worker in-memory queue recovery** — workers rebuild from DB state, so there is no separate worker-state to test
- **`memory_vec` / `raw_chunks_vec` embedding row durability** — these rows are written asynchronously by the embedder worker AFTER HTTP 200 is returned for `Remember` / `RawIngest`. The ack criterion does not wait for vector writes, and `Recall` does not query `memory_vec` in the HTTP path. SQLite WAL guarantees persist these rows once written, but the harness does not wait for or verify them. If the founder wants this verified, see Q9.
- **Cluster / distributed recovery** — Forge is single-node
- **Litestream / backup replication** — out of scope; tested separately in ops polish (Phase 2C-6)

---

## 12. Open questions for founder (DESIGN GATE INPUT REQUIRED)

These block design-gate approval. Each has a recommended answer. Total: Q1-Q9.

**Q1. Isolation strategy — Option A (HOME override) or Option B (add `FORGE_DIR` env var)?**
*Recommended:* Option B. Scope: `crates/core/src/paths.rs` + `crates/daemon/src/server/tls.rs:dirs_for_forge_home()` in a single atomic commit (both must land together per BLOCKER-1 from adversarial review). Cleaner than HOME override, doesn't leak HOME into the subprocess, keeps fastembed cache warm.

**Q2. Real-subprocess commitment — spawn `forge-daemon` as a child process, or simulate in-process?**
*Recommended:* Real subprocess. See §3 rationale. This is a non-negotiable from my side but stated for explicit approval.

**Q3. Default workload size — 100 memories + 50 chunks + 20 FISP messages acceptable?**
*Recommended:* Yes for default. CLI flags allow scaling up for stress runs. A stress variant with 10k memories is a future run, not part of the first landing.

**Q4. Recovery time threshold — hard-lock 5s now, or calibrate empirically after first run?**
*Recommended:* Calibrate first. Run once with no threshold enforcement; measure actual recovery time on M1 Pro; then lock threshold based on real data + 50% margin. Document in the decision log. No silent lowering.

**Q5. Embedder for the bench — real MiniLM or fake embedder?**
*Recommended:* Real MiniLM. The persistence contract covers embedding rows in `memory_vec`; testing with a fake embedder skips that surface. Option A isolation forces us into fake embedder (cache miss) — another reason to prefer Option B.

**Q6. Worker catch-up window — post-restart, how long to wait before scoring?**
*Recommended:* 10 seconds default, overridable via CLI. The embedder may still be processing pre-kill memories; scoring during catch-up would spuriously depress recovery_rate.

**Q7. Framework extraction scope — single file now, extract modules in Forge-Tool?**
*Recommended:* Yes. Build `forge_persist.rs` as one file for the first iteration. Extract shared helpers (daemon subprocess harness, seeded content generator, ground-truth tracker) in Forge-Tool (2A-2) when we have two call sites and can see the real extraction boundary.

**Q8. macOS stale-PID cleanup is inverted (confirmed latent daemon bug).**
Adversarial review confirmed the behavior: `main.rs:50-71` guards stale-PID cleanup behind `#[cfg(unix)]`, but the liveness check uses `Path::new("/proc/{pid}").exists()`. On macOS `/proc` does not exist, so the check always returns `false`, so the cleanup path unconditionally unlinks the PID file whenever the lock-held branch is entered. This is a real daemon bug — two concurrent live daemons could theoretically coexist if one's lock state ever got into the lock-held-but-unknown branch.

**For Forge-Persist specifically, this is NOT a blocker** because the SIGKILL-restart flow doesn't enter the lock-held branch (the kernel releases advisory locks on process death, so the new daemon's `try_lock_exclusive` succeeds on first attempt). But the bug IS a real correctness gap and should be fixed opportunistically. **Recommended fix:** replace the `/proc/{pid}` probe with `libc::kill(pid, 0)` (signal-0 liveness probe, portable across Unix). Ship as a prerequisite commit BEFORE the Forge-Persist harness, on its own TDD cycle. See §14 step (b).

**Q9. `memory_vec` durability scope — add direct SQL probe to verify embedding rows survive?**
The design currently scopes out `memory_vec` / `raw_chunks_vec` durability (see §6.1 and §11) because `Recall` does not query `memory_vec` and the ack criterion doesn't wait for async embedder writes. Adding a post-restart direct SQL probe (query `memory_vec WHERE id = ?` for each acked memory id) would close this gap. Recommended: **defer to second iteration**. The WAL guarantees are strong; the first iteration's scope is already broad enough to prove the framework.

---

## 13. Risks and mitigations

| Risk | Impact | Mitigation |
|------|--------|------------|
| macOS stale-PID cleanup inverted (`/proc/{pid}` always false on macOS) | Not a blocker for Forge-Persist's happy path (SIGKILL releases kernel lock cleanly), but a latent daemon bug — two live daemons could theoretically coexist on macOS | Fix: replace `/proc/{pid}` probe with `libc::kill(pid, 0)` (signal-0 liveness check, portable across Unix). Ship as its own commit before Forge-Persist work begins, on its own TDD cycle. See Q8. |
| Subprocess spawn fails in CI (sandboxing, display lock) | Integration test flakes | Test uses `CARGO_BIN_EXE_*` which works in any Cargo test env |
| Free-port race between `bind(0) + drop` and daemon `start()` | Daemon fails to bind | Retry loop: 3 attempts; each allocates a fresh port |
| Daemon startup > 5 s on founder's machine | Recovery time metric fails | Calibrate first run; update threshold + document in decision log |
| Embedder async lag — `Remember` HTTP-200-acked but `memory_vec` row not yet written at kill time | Would depress `consistency_rate` for embedding rows | **Scoped OUT** per §11 non-goals and §6.1 scope limitation. Ack criterion is the `memory` table row only (written synchronously). `memory_vec` durability is trusted to SQLite WAL + future Q9 SQL probe. Two-step ack via `Recall` was proposed and rejected — `Recall` does not query `memory_vec`. |
| ~~FISP `SessionSend` might not write to `session_message` synchronously~~ **CLOSED** by adversarial review | n/a | Confirmed synchronous: `crates/daemon/src/sessions.rs:323` does a direct `INSERT` before returning `Ok(id)`, handler calls inline. HTTP 200 implies persisted row. |
| TempDir cleanup leaves orphans on crash | Disk fills slowly over many runs | Documented behavior. Future cleanup cron. |
| PID lock race between two concurrent harness runs | Second run fails spawn | Each run has its own TempDir + `FORGE_DIR` override → no lock conflict by construction |
| Daemon version drift — old binary on PATH vs current code | Wrong binary tested | Test uses `CARGO_BIN_EXE_forge-daemon`; CLI prefers `--daemon-bin` explicit. Summary.json logs the version. |
| `Child::kill()` semantics differ across platforms | Wrong signal | Docs confirm Unix = SIGKILL. Windows not supported for v1. |
| Framework over-engineering | Wastes Phase 2A velocity | Single-file first iteration; no extraction until Forge-Tool |
| `HOME` override (Option A fallback) breaks workers that read HOME | Worker misbehavior | Option B avoids this entirely. Option A only used if Q1 answered differently. |

---

## 14. Quality gate progression for this deliverable

Per the Phase 2 quality gate definition in `phase-2-plan.md` §"Quality gate definition":

1. **Design gate** ← THIS DOCUMENT. Pending founder review AND adversarial subagent review.
2. **TDD gate** — every new function has a failing test first. RED → GREEN → REFACTOR. TDD cycle ordering (prerequisite commits first, then harness commits):

   **Prerequisite commits (each its own atomic commit):**
   - (a) `FORGE_DIR` env var in `forge_core::paths::forge_dir()` + `crates/daemon/src/server/tls.rs:dirs_for_forge_home()` flowing through the same resolver. Tests in both crates. One commit, because partial landing breaks isolation.
   - (b) Stale-PID cleanup cross-platform fix: replace `/proc/{pid}` probe at `main.rs:50-71` with `libc::kill(pid, 0)` signal-0 liveness probe. New test that exercises the stale-cleanup path on macOS (currently inverted — this is a confirmed latent bug; fix is non-conditional). One commit.
   - (c) `crates/daemon/Cargo.toml` — add `"blocking"` to the `reqwest` features list. One-line change + a trivial smoke test that `reqwest::blocking::Client::new()` compiles. One commit.

   **Harness commits (after prerequisites land):**
   - (d) Seeded workload generator (content generation, interleaving, ordering) — `bench/forge_persist.rs` first module
   - (e) HTTP client wrapper (request construction, response parsing, error taxonomy)
   - (f) Subprocess lifecycle (spawn, health-poll, kill, restart with elapsed measurement)
   - (g) Ground-truth tracker (add-on-ack, verify-matches, content hash per §6.2)
   - (h) Scoring functions (recovery, consistency, time composition)
   - (i) `forge-bench forge-persist` CLI subcommand wiring in `bin/forge-bench.rs`
   - (j) End-to-end integration test in `crates/daemon/tests/forge_persist_harness.rs`
   - (k) Results doc at `docs/benchmarks/results/forge-persist-<date>.md` populated from the first calibration run
3. **Clippy + fmt gate** — `cargo fmt --all` clean, `cargo clippy --workspace -- -W clippy::all -D warnings` zero warnings.
4. **Adversarial review gate** — `feature-dev:code-reviewer` subagent on the diff. All ≥ 80 confidence findings addressed or explicitly accepted.
5. **Documentation gate** — `docs/benchmarks/results/forge-persist-<date>.md` published with honest numbers, per-run setup, reproduction command, limitations, comparison to "no public baseline" (this is a Forge-specific bench by design).
6. **Reproduction gate** — founder runs `repro.sh` once manually.
7. **Dogfood gate** — founder runs the bench against their dev daemon setup for ≥ 1 calendar day.

Forge-Persist is not "done" until all 7 gates pass. No shortcuts.

---

## 15. What happens next

1. **Founder reviews this design doc.** Answers Q1-Q9. Approves or redirects.
2. **Adversarial review.** `feature-dev:code-reviewer` subagent dispatched on this draft BEFORE any code. All ≥ 80 confidence findings fixed or explicitly accepted with rationale.
3. **Only then:** TDD cycles begin. First cycle is the `FORGE_DIR` env var patch as its own commit.
4. **First Forge-Persist run** on the founder's machine produces the calibration number for the 5 s recovery time threshold.
5. **Results doc** published at `docs/benchmarks/results/forge-persist-<date>.md` per the honesty rail.
6. **Memory updated** — `MEMORY.md` index entry, Forge daemon memory entry (via `forge-next remember`).
7. **Phase 2A-2 Forge-Tool design gate** begins only after Forge-Persist has passed all 7 quality gates.

---

**END DESIGN DOC.** No implementation begins until founder explicitly approves this design AND adversarial review finds no blocking issues. This document itself is the first gate — it is the artifact that is reviewed, not the code.

---

## Appendix A. Adversarial review findings (2026-04-14) — addressed in this revision

This design doc has passed one round of adversarial review via the `feature-dev:code-reviewer` subagent. Findings below were either fixed in-document or explicitly accepted with rationale. The review's grading scale: BLOCKER ≥ 90 confidence, HIGH 80-89, MEDIUM 60-79.

**BLOCKER findings (all fixed):**

- **B1 — TLS path isolation leak (confidence 95).** `tls.rs:dirs_for_forge_home()` at `crates/daemon/src/server/tls.rs:152` resolves `HOME + /.forge` directly, not through `forge_core::paths::forge_dir()`. Adding `FORGE_DIR` to `paths.rs` alone would leave TLS leaking. **Fixed:** §4 Option B now requires both `paths.rs` and `tls.rs` changes in a single atomic commit; §14 TDD ordering step (a) names both files explicitly.
- **B2 — `reqwest::blocking` feature absent (confidence 98).** `crates/daemon/Cargo.toml` does not enable the `"blocking"` feature on reqwest. Design claim that the dependency was already available was wrong. **Fixed:** §7.3 acknowledges the Cargo.toml change as a required prerequisite; §14 TDD ordering step (c) adds the Cargo.toml patch as its own commit.
- **B3 — macOS PID stale check is inverted (confidence 96).** `main.rs:50-71` uses `/proc/{pid}` existence check, which always returns false on macOS, so stale-cleanup fires unconditionally. This is a latent daemon bug. **Partial fix:** §2 item 3 and §13 risk table now describe the actual behavior correctly. The bug is a prerequisite commit for Forge-Persist (§14 step (b)) via `libc::kill(pid, 0)` liveness probe. Scope clarified: not strictly a blocker for Forge-Persist's happy path (kernel releases locks on SIGKILL cleanly), but a correctness gap worth fixing opportunistically.
- **B4 — `SessionCreate` variant does not exist (confidence 90).** Correct variant is `RegisterSession`; read-back is `SessionMessages`. **Fixed:** §5.3 now names both correctly; §2 item 12 verifies the variant names against `crates/core/src/protocol/request.rs`.

**HIGH findings (all addressed):**

- **H1 — Two-step ack via `Recall` is a no-op (confidence 87).** `Recall` handler passes `query_embedding: None` to `hybrid_recall`, which only consults `memory_vec` when an embedding is provided. The proposed "poll `Recall` after HTTP 200" fix does not verify `memory_vec` row existence. **Fixed:** §5.2 drops the two-step ack framing; §6.1 adds a scope limitation explicitly excluding `memory_vec`/`raw_chunks_vec` from recovery rate; §11 non-goals adds embedding row durability as an explicit exclusion; §13 risk table updated.
- **H2 — Fastembed cache redirect claim unverified (confidence 85).** Option A's "HOME override redirects the fastembed cache" claim was not traced to the library source. **Addressed:** §4 Option A now marks the claim as unverified and instructs any Option A implementation to trace the real resolution path first. Option B is the recommendation and sidesteps this entirely.
- **H3 — socket.rs is a second Option B confirmation, not a leak (confidence 82).** `crates/daemon/src/server/socket.rs:72-73` constructs PID path via `forge_core::forge_dir()` directly; after `FORGE_DIR` patch, this picks up the override automatically. **Addressed:** §2 item 13 cites this as confirmation that Option B propagates through all socket server call sites.
- **H4 — "Content hash" was undefined (confidence 84).** Consistency metric required a hash match without specifying which bytes were hashed. **Fixed:** §6.2 now defines `content_hash = hex(sha256(canonical_payload))` with per-op-kind canonical payloads, pinning SHA-256 and exact field selection.

**MEDIUM findings (addressed):**

- **M1 — CI recovery time threshold uncalibrated (confidence 72).** Integration test's 10 s threshold has no real data point; daemon startup may take 3-8 s on cold CI runners. **Fixed:** §9 now states the 10 s threshold is aspirational, requires validation on the target CI platform before enabling, and the first landing runs locally only with a TODO comment in the test file.
- **M2 — `SessionSend` write IS synchronous (confidence 76).** Verified at `crates/daemon/src/sessions.rs:323`. **Fixed:** §2 item 15 cites this explicitly; §13 risk table marks the FISP write row CLOSED.
- **M3 — Framework shakedown claim oversells single-file approach (confidence 68).** Building as a single file validates the subprocess lifecycle but not a reusable module framework. **Fixed:** §1 and §7.1 now explicitly scope the framework validation to the subprocess lifecycle primitive only, deferring module extraction to Forge-Tool (2A-2).

**No findings rejected.** All BLOCKER and HIGH findings were fixed in the document; all MEDIUM findings were either fixed or addressed with explicit rationale.

**Ready for founder review.**
