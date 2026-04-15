//! Forge-Persist benchmark harness — Phase 2A-1 of the Phase 2+ roadmap.
//!
//! Spawns a real `forge-daemon` subprocess, issues a scripted seeded
//! workload, SIGKILLs mid-run, restarts, and verifies that every
//! HTTP-200-acked operation survived with bit-exact consistency.
//!
//! See `docs/benchmarks/forge-persist-design.md` for the full design,
//! scoring rubric, and reproduction contract.

use forge_core::protocol::{MessagePart, Request, Response, ResponseData};
use forge_core::types::memory::MemoryType;
use rand::seq::SliceRandom;
use rand::SeedableRng;
use rand_chacha::ChaCha20Rng;
use sha2::{Digest, Sha256};
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};
use tempfile::TempDir;

/// Configuration for a Forge-Persist workload.
///
/// The workload is a deterministic sequence of `Operation`s derived
/// from `seed`. Calling `generate_workload` twice with the same
/// `WorkloadConfig` produces byte-identical output on any machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkloadConfig {
    /// ChaCha20 PRNG seed controlling interleaving order and any
    /// future per-op content randomness.
    pub seed: u64,
    /// Number of `Remember` (memory insertion) ops.
    pub memories: usize,
    /// Number of `IngestRaw` (raw-document ingest) ops.
    pub chunks: usize,
    /// Number of `FispSend` (inter-session message) ops.
    pub fisp_messages: usize,
}

/// Size of the pre-created FISP session pool. Five sessions give
/// enough permutations for `FispSend` routing while keeping the
/// pre-workload setup cheap. See design doc §5.3.
pub const SESSION_POOL_SIZE: usize = 5;

/// Fixed `memory_type` vocabulary rotated across `Remember` ops.
/// Covers every variant of `MemoryType` defined in
/// `crates/core/src/types/memory.rs:7` so the workload exercises all
/// five memory kinds the daemon can store. The order mirrors the
/// enum declaration order.
const MEMORY_TYPES: [&str; 5] = ["decision", "lesson", "pattern", "preference", "protocol"];

/// Fixed tag vocabulary — each `Remember` op draws 2 tags via a
/// deterministic rotation keyed on the op index.
const TAG_POOL: [&str; 5] = ["persist", "bench", "forge", "durability", "harness"];

/// `source` string attached to every `RawIngest` request this harness
/// issues. Also serves as a filter during post-restart verification
/// so the ground-truth matcher in cycle (g) can isolate this bench's
/// documents from any other state that may leak into the daemon.
pub const HARNESS_SOURCE: &str = "forge-persist";

/// `topic` string on every `SessionSend` (FISP) request. Same role
/// as `HARNESS_SOURCE` — lets cycle (g) filter message recovery by
/// bench origin.
pub const HARNESS_TOPIC: &str = "forge-persist";

/// A single workload operation, fully populated with the payload
/// the HTTP client will marshal into a daemon API request.
///
/// `index` uniquely identifies the op within its kind's sub-sequence
/// and seeds the deterministic content strings. Two calls to
/// `generate_workload` with the same `WorkloadConfig` produce
/// byte-identical `Operation` values.
///
/// The field shape mirrors the corresponding `Request` variant in
/// `crates/core/src/protocol/request.rs` so cycle (e) can marshal
/// `Operation` into HTTP payloads without further enum changes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Operation {
    /// Store a memory via `POST /api method=Remember`.
    /// Matches `Request::Remember` minus the optional `confidence`,
    /// `project`, and `metadata` fields (Forge-Persist keeps those
    /// at defaults to minimize the content-hash canonical surface).
    Remember {
        index: usize,
        memory_type: String,
        title: String,
        content: String,
        tags: Vec<String>,
    },
    /// Ingest a raw document chunk via `POST /api method=RawIngest`.
    IngestRaw { index: usize, content: String },
    /// Send a FISP inter-session message via `POST /api method=SessionSend`.
    /// `from_session` and `to_session` are drawn from the pre-created
    /// session pool (`persist_session_0` .. `persist_session_{N-1}`).
    FispSend {
        index: usize,
        from_session: String,
        to_session: String,
        content: String,
    },
}

/// Build the canonical session name for pool slot `i`. Internal helper —
/// only called by `generate_workload`. `pub(crate)` because the
/// integration test may need to reconstruct the pool names later.
pub(crate) fn pool_session_name(i: usize) -> String {
    format!("persist_session_{i}")
}

fn remember_memory_type(index: usize) -> String {
    MEMORY_TYPES[index % MEMORY_TYPES.len()].to_string()
}

fn remember_title(index: usize) -> String {
    format!("persist_memory_{index}")
}

fn remember_content(index: usize) -> String {
    format!("persist_memory_{index}: deterministic memory body for Forge-Persist bench")
}

fn remember_tags(index: usize) -> Vec<String> {
    // 2 tags per op, drawn from TAG_POOL by rotating on index. The
    // design doc §5.5 says "2-3 tags"; two is the minimum that still
    // exercises the tags vec path in the daemon and keeps the
    // content hash stable for consistency checks.
    vec![
        TAG_POOL[index % TAG_POOL.len()].to_string(),
        TAG_POOL[(index + 1) % TAG_POOL.len()].to_string(),
    ]
}

fn ingest_content(index: usize) -> String {
    // ~200-char lorem-ipsum-derivative body prefixed with the index so
    // every document in the workload is unique while still exercising
    // the raw-layer chunker / BM25 / vec pipeline.
    format!(
        "persist_doc_{index}: lorem ipsum dolor sit amet consectetur adipiscing \
         elit sed do eiusmod tempor incididunt ut labore et dolore magna aliqua \
         ut enim ad minim veniam quis nostrud exercitation ullamco laboris"
    )
}

fn fisp_content(index: usize) -> String {
    format!("persist_fisp_{index}: deterministic inter-session message body")
}

/// Single source of truth for the `Vec<MessagePart>` shape a FISP
/// workload message marshals into. Used by BOTH [`op_to_request`] and
/// [`canonical_hash`] so that the hashed payload is, by construction,
/// byte-identical to the payload actually sent over the wire. If this
/// shape ever changes (new `MessagePart` fields, different `kind`,
/// multi-part message), both the request and the hash follow in lockstep
/// automatically — no more silent divergence at the refactor boundary.
///
/// Extracted in cycle (g1) adversarial review to fix HIGH 95/100
/// ("silent hash divergence on FispSend when `op_to_request` is
/// modified"). A cross-check test in `mod tests` verifies the two
/// paths remain in agreement even after future refactors.
fn fisp_parts(content: &str) -> Vec<MessagePart> {
    vec![MessagePart {
        kind: "text".to_string(),
        text: Some(content.to_string()),
        path: None,
        data: None,
        memory_id: None,
    }]
}

/// Generate a deterministic workload from `config`.
///
/// The returned `Vec<Operation>` has exactly
/// `config.memories + config.chunks + config.fisp_messages` elements.
/// Within a single config, the output is byte-identical across calls
/// and platforms. Two configs that differ only in `seed` produce
/// different interleaving orders.
///
/// **Algorithm:** build the sequential op vector (all `Remember` then
/// all `IngestRaw` then all `FispSend`), then shuffle in place using a
/// ChaCha20 PRNG seeded from `config.seed`. ChaCha20 is
/// platform-independent and deterministic.
pub fn generate_workload(config: &WorkloadConfig) -> Vec<Operation> {
    let total = config.memories + config.chunks + config.fisp_messages;
    let mut ops = Vec::with_capacity(total);
    for i in 0..config.memories {
        ops.push(Operation::Remember {
            index: i,
            memory_type: remember_memory_type(i),
            title: remember_title(i),
            content: remember_content(i),
            tags: remember_tags(i),
        });
    }
    for i in 0..config.chunks {
        ops.push(Operation::IngestRaw {
            index: i,
            content: ingest_content(i),
        });
    }
    for i in 0..config.fisp_messages {
        // Round-robin routing through the pool keeps from != to (pool
        // size is always > 1) while remaining fully deterministic.
        let from_idx = i % SESSION_POOL_SIZE;
        let to_idx = (i + 1) % SESSION_POOL_SIZE;
        ops.push(Operation::FispSend {
            index: i,
            from_session: pool_session_name(from_idx),
            to_session: pool_session_name(to_idx),
            content: fisp_content(i),
        });
    }
    let mut rng = ChaCha20Rng::seed_from_u64(config.seed);
    ops.shuffle(&mut rng);
    ops
}

/// Translate a `Forge-Persist` [`Operation`] into the wire-level
/// [`forge_core::protocol::Request`] the daemon accepts.
///
/// Used by the HTTP client wrapper in later cycles to marshal a
/// workload into actual `POST /api` requests. Pure function — no IO,
/// no error path: the input is under our control (generator-local)
/// and any unknown `memory_type` string is coerced to
/// `MemoryType::Decision` as the safe default.
pub fn op_to_request(op: &Operation) -> Request {
    match op {
        Operation::Remember {
            memory_type,
            title,
            content,
            tags,
            ..
        } => Request::Remember {
            memory_type: match memory_type.as_str() {
                "decision" => MemoryType::Decision,
                "lesson" => MemoryType::Lesson,
                "pattern" => MemoryType::Pattern,
                "preference" => MemoryType::Preference,
                "protocol" => MemoryType::Protocol,
                // Generator-produced ops always hit an explicit arm
                // above, so the wildcard is a defensive default that
                // should be unreachable in practice.
                _ => MemoryType::Decision,
            },
            title: title.clone(),
            content: content.clone(),
            confidence: None,
            tags: Some(tags.clone()),
            project: None,
            metadata: None,
        },
        Operation::IngestRaw { content, .. } => Request::RawIngest {
            text: content.clone(),
            project: None,
            session_id: None,
            source: HARNESS_SOURCE.to_string(),
            timestamp: None,
            metadata: None,
        },
        Operation::FispSend {
            from_session,
            to_session,
            content,
            ..
        } => Request::SessionSend {
            to: to_session.clone(),
            kind: "notification".to_string(),
            topic: HARNESS_TOPIC.to_string(),
            parts: fisp_parts(content),
            project: None,
            timeout_secs: None,
            meeting_id: None,
            from_session: Some(from_session.clone()),
        },
    }
}

// ---------------------------------------------------------------------------
// Canonical content hashing (cycle g1)
// ---------------------------------------------------------------------------

/// Compute the canonical content hash for a Forge-Persist workload
/// [`Operation`] per design doc §6.2.
///
/// `canonical_hash` is a pure function of the op's payload, returning a
/// lowercase hex-encoded SHA-256 digest (64 chars). The exact canonical
/// bytes depend on the op variant:
///
/// - `Remember` → UTF-8 bytes of the `content` field, unchanged
/// - `IngestRaw` → UTF-8 bytes of the `content` field, unchanged
/// - `FispSend` → UTF-8 bytes of `serde_json::to_string(&parts)`, where
///   `parts` is the `Vec<MessagePart>` reconstructed exactly as
///   [`op_to_request`] builds it for `SessionSend`. `serde_json` preserves
///   struct field declaration order for `#[derive(Serialize)]` types, so
///   the output is deterministic across machines and runs.
///
/// **Invariant:** two `Operation` values that are `PartialEq`-equal MUST
/// produce the same hash. The content-only hashing scheme for Remember
/// and IngestRaw means two ops of different kinds but the same body
/// collide — that's expected: recovery verification looks up each ack
/// by ID (which is kind-specific), so cross-kind collisions have no
/// effect on scoring.
///
/// **Hash scheme version:** SHA-256 + per-variant canonical payload as
/// above. Any change to this function must bump the `hash_scheme` field
/// of `summary.json` output in cycle (i).
pub fn canonical_hash(op: &Operation) -> String {
    let canonical_bytes: Vec<u8> = match op {
        Operation::Remember { content, .. } | Operation::IngestRaw { content, .. } => {
            content.as_bytes().to_vec()
        }
        Operation::FispSend { content, .. } => {
            // `fisp_parts` is the single source of truth — `op_to_request`
            // uses the same helper, so the hashed payload is by
            // construction byte-identical to the payload actually sent
            // over the wire. A cross-check test verifies this invariant
            // holds under future refactors.
            let parts = fisp_parts(content);
            serde_json::to_string(&parts)
                .expect("MessagePart serialization is infallible for generator-local data")
                .into_bytes()
        }
    };
    bytes_to_hex(&Sha256::digest(&canonical_bytes))
}

/// Lowercase-hex encoding of a byte slice. Inlined to avoid taking a
/// direct dependency on the `hex` crate — SHA-256 digests are only 32
/// bytes, so the alloc cost is negligible and the helper stays private
/// to this module.
fn bytes_to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

// ---------------------------------------------------------------------------
// Error taxonomy
// ---------------------------------------------------------------------------

/// Errors the Forge-Persist harness can report. Preserves the underlying
/// `std::io::Error` / `reqwest::Error` / `serde_json::Error` cause for
/// spawn/kill/IO/HTTP failures so callers (the integration test or the
/// `forge-bench forge-persist` CLI) can include it in diagnostics.
#[derive(Debug)]
pub enum HarnessError {
    /// Failed to bind a free port while preparing the harness.
    BindFailed(std::io::Error),
    /// `Command::spawn` for the daemon binary failed.
    SpawnFailed(std::io::Error),
    /// Daemon spawned but did not answer HTTP Health within
    /// `recovery_timeout`. `elapsed` is measured from `Command::spawn`.
    SpawnTimeout { elapsed: Duration, port: u16 },
    /// `Child::kill` or `Child::wait` failed during termination.
    KillFailed(std::io::Error),
    /// Attempted to spawn a daemon on a harness that already has
    /// an active child process.
    AlreadySpawned,
    /// Miscellaneous IO failure (temp-dir creation, dir setup, etc.).
    Io(std::io::Error),
    /// HTTP transport error — DNS resolution, connection refused, TLS,
    /// or reqwest-internal failures. Wraps the underlying `reqwest::Error`.
    NetworkError(reqwest::Error),
    /// Response body could not be parsed as a `forge_core::protocol::Response`.
    /// Wraps the `serde_json::Error` so callers can see the exact column.
    JsonError(serde_json::Error),
    /// Daemon responded but with a non-2xx HTTP status code.
    BadStatus(u16),
    /// Daemon returned a `Response::Error { message }` — application-level
    /// failure rather than transport.
    DaemonError(String),
    /// Daemon returned `Response::Ok` but with a `ResponseData` variant
    /// the caller did not expect (e.g., `execute_op` expected
    /// `Stored` but received `Memories`).
    UnexpectedResponse(String),
}

impl std::fmt::Display for HarnessError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HarnessError::BindFailed(e) => write!(f, "failed to bind a free port: {e}"),
            HarnessError::SpawnFailed(e) => write!(f, "failed to spawn forge-daemon: {e}"),
            HarnessError::SpawnTimeout { elapsed, port } => write!(
                f,
                "forge-daemon did not answer HTTP Health on port {port} within {elapsed:?} (spawn timeout)"
            ),
            HarnessError::KillFailed(e) => write!(f, "failed to kill forge-daemon child: {e}"),
            HarnessError::AlreadySpawned => {
                write!(f, "harness already has an active child process")
            }
            HarnessError::Io(e) => write!(f, "harness IO error: {e}"),
            HarnessError::NetworkError(e) => write!(f, "HTTP network error: {e}"),
            HarnessError::JsonError(e) => write!(f, "response JSON decode error: {e}"),
            HarnessError::BadStatus(code) => write!(f, "daemon returned HTTP {code}"),
            HarnessError::DaemonError(msg) => write!(f, "daemon returned error response: {msg}"),
            HarnessError::UnexpectedResponse(msg) => {
                write!(f, "unexpected daemon response: {msg}")
            }
        }
    }
}

impl std::error::Error for HarnessError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            HarnessError::BindFailed(e)
            | HarnessError::SpawnFailed(e)
            | HarnessError::KillFailed(e)
            | HarnessError::Io(e) => Some(e),
            HarnessError::NetworkError(e) => Some(e),
            HarnessError::JsonError(e) => Some(e),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// HTTP client wrapper + acked-op record (cycle f2)
// ---------------------------------------------------------------------------

/// Record of a successfully-acked workload operation. Stored by the
/// ground-truth tracker in cycle (g) and compared against the daemon's
/// post-restart state to score recovery.
///
/// `content_hash` is a placeholder (empty string) in cycle (f2). Cycle
/// (g) introduces a SHA-256 canonical-payload hash per design doc §6.2
/// so the tracker can verify bit-exact consistency after restart.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AckedOp {
    /// The daemon-assigned identifier. The exact shape depends on the
    /// op kind, which matters for cycle (g)'s ground-truth tracker:
    ///
    /// - `Remember` → the ULID from `ResponseData::Stored { id }`.
    ///   Recovery verification looks this up in the `memories` table.
    /// - `IngestRaw` → the **document-level** `document_id` from
    ///   `ResponseData::RawIngest { document_id, .. }`. Recovery
    ///   verification must query `raw_documents.id`, NOT
    ///   `raw_chunks.id` — the harness never sees chunk ids. (The
    ///   daemon assigns chunk ids lazily during ingest and they are
    ///   not exposed via the HTTP response shape.)
    /// - `FispSend` → the message id from
    ///   `ResponseData::MessageSent { id, .. }`. Recovery verification
    ///   looks this up in the `session_messages` table.
    pub id: String,
    /// SHA-256 canonical content hash. Empty in cycle (f2); populated
    /// by the ground-truth tracker in cycle (g).
    pub content_hash: String,
}

/// Blocking HTTP client that marshals Forge-Persist workload operations
/// into `POST /api` requests against a running daemon and parses the
/// JSON response into `forge_core::protocol::Response`.
///
/// Wraps `reqwest::blocking::Client` with a 5-second total per-request
/// timeout. The timeout is tuned so that (a) during the spawn wait
/// loop, connection-refused failures still bypass the timeout and
/// return near-instantly, and (b) actual daemon writes (Remember,
/// RawIngest) have enough headroom for embedder-backed work.
pub struct HttpClient {
    client: reqwest::blocking::Client,
    base_url: String,
}

impl HttpClient {
    /// Build a new client bound to `base_url` (e.g.,
    /// `"http://127.0.0.1:8420"`). Construction is fallible because
    /// `reqwest::blocking::Client::builder().build()` can fail during
    /// TLS stack initialization on some platforms.
    ///
    /// **Timeout strategy:** the total per-request timeout is 5 s
    /// (generous enough for a real `Remember` op that touches the
    /// embedder), but the TCP-level **connect** timeout is pinned to
    /// 200 ms. That split matters during the `PersistHarness::spawn`
    /// wait loop: `ECONNREFUSED` returns immediately when the daemon
    /// has not yet bound its port, but a stalled connection (e.g.,
    /// the SYN got dropped on a contended CI runner) would otherwise
    /// burn the full 5-second budget per probe. 200 ms keeps the
    /// overrun tight without starving legitimate localhost traffic,
    /// which should complete in sub-millisecond.
    pub fn new(base_url: String) -> Result<Self, HarnessError> {
        let client = reqwest::blocking::Client::builder()
            .connect_timeout(Duration::from_millis(200))
            .timeout(Duration::from_secs(5))
            .build()
            .map_err(HarnessError::NetworkError)?;
        Ok(Self { client, base_url })
    }

    /// POST the given [`Request`] as JSON to `{base_url}/api` and parse
    /// the body as a [`Response`].
    ///
    /// Error taxonomy:
    /// - `NetworkError` — DNS/connect/TLS/timeout failure before a
    ///   response body can be read.
    /// - `BadStatus(code)` — daemon responded with a non-2xx code
    ///   (we do not try to parse the body as JSON in that case).
    /// - `JsonError` — body could not be deserialized into `Response`.
    pub fn execute(&self, req: &Request) -> Result<Response, HarnessError> {
        let url = format!("{}/api", self.base_url);
        let http_resp = self
            .client
            .post(&url)
            .json(req)
            .send()
            .map_err(HarnessError::NetworkError)?;

        let status = http_resp.status();
        if !status.is_success() {
            return Err(HarnessError::BadStatus(status.as_u16()));
        }

        let body = http_resp.text().map_err(HarnessError::NetworkError)?;
        serde_json::from_str::<Response>(&body).map_err(HarnessError::JsonError)
    }

    /// Issue a `Request::Health` and require `Response::Ok { data:
    /// ResponseData::Health { .. } }`. Any other response shape is a
    /// bug, so we surface it as `UnexpectedResponse` rather than silently
    /// returning `Ok(())`. The spawn wait loop and `is_daemon_alive` use
    /// this as their liveness predicate.
    pub fn health(&self) -> Result<(), HarnessError> {
        match self.execute(&Request::Health)? {
            Response::Ok {
                data: ResponseData::Health { .. },
            } => Ok(()),
            Response::Ok { data: other } => Err(HarnessError::UnexpectedResponse(format!(
                "expected Health data, got {other:?}"
            ))),
            Response::Error { message } => Err(HarnessError::DaemonError(message)),
        }
    }

    /// Marshal a Forge-Persist workload [`Operation`] into its daemon
    /// [`Request`], POST it, and extract the ack id from whichever
    /// `ResponseData` variant the daemon returns.
    ///
    /// Accepted ack shapes:
    /// - `ResponseData::Stored { id }` — from `Remember`
    /// - `ResponseData::RawIngest { document_id, .. }` — from `RawIngest`
    /// - `ResponseData::MessageSent { id, .. }` — from `SessionSend`
    ///
    /// Any other `ResponseData` variant is `UnexpectedResponse`.
    /// `content_hash` is left empty; cycle (g) will populate it via the
    /// ground-truth tracker's canonical-payload SHA-256.
    pub fn execute_op(&self, op: &Operation) -> Result<AckedOp, HarnessError> {
        let req = op_to_request(op);
        match self.execute(&req)? {
            Response::Ok { data } => match data {
                ResponseData::Stored { id } => Ok(AckedOp {
                    id,
                    content_hash: String::new(),
                }),
                ResponseData::RawIngest { document_id, .. } => Ok(AckedOp {
                    id: document_id,
                    content_hash: String::new(),
                }),
                ResponseData::MessageSent { id, .. } => Ok(AckedOp {
                    id,
                    content_hash: String::new(),
                }),
                other => Err(HarnessError::UnexpectedResponse(format!(
                    "expected Stored/RawIngest/MessageSent, got {other:?}"
                ))),
            },
            Response::Error { message } => Err(HarnessError::DaemonError(message)),
        }
    }
}

// ---------------------------------------------------------------------------
// Harness config + handle
// ---------------------------------------------------------------------------

/// Full Forge-Persist run configuration. Mirrors the CLI flags defined
/// in `forge-bench forge-persist` (cycle (i)) and the integration test
/// at `crates/daemon/tests/forge_persist_harness.rs`.
#[derive(Debug, Clone)]
pub struct PersistConfig {
    /// Path to a built `forge-daemon` binary — usually
    /// `env!("CARGO_BIN_EXE_forge-daemon")` in the integration test or
    /// `--daemon-bin` in the CLI.
    pub daemon_bin: PathBuf,
    /// Number of `Remember` operations in the workload.
    pub memories: usize,
    /// Number of `RawIngest` operations in the workload.
    pub chunks: usize,
    /// Number of `SessionSend` operations in the workload.
    pub fisp_messages: usize,
    /// Seed for the ChaCha20 workload interleaver.
    pub seed: u64,
    /// Fraction of total acked operations at which SIGKILL fires.
    pub kill_after: f64,
    /// Maximum time to wait for the daemon to bind its HTTP port
    /// after spawn. Also reused as the post-restart health-poll
    /// timeout in later cycles.
    pub recovery_timeout: Duration,
    /// Time to wait after restart for asynchronous worker writes
    /// (e.g., embedder) to catch up before scoring. Not exercised
    /// by cycle (f1) — reserved for cycle (g) ground-truth verification.
    pub worker_catchup: Duration,
    /// Optional output directory for results. `None` means "in-memory
    /// only, don't write files" — used by the integration test.
    pub output_dir: Option<PathBuf>,
}

/// Owning handle for a Forge-Persist benchmark run. Owns the TempDir
/// that holds the daemon's isolated state (via `FORGE_DIR`), the
/// bench's free port, the HTTP client used to probe health and issue
/// workload ops, the daemon's running child process (when spawned),
/// and the config describing the workload.
///
/// Drop kills any live child to prevent orphaned daemon processes.
pub struct PersistHarness {
    config: PersistConfig,
    port: u16,
    client: HttpClient,
    child: Option<Child>,
    /// Kept last so it is dropped AFTER `child` — removing the
    /// TempDir before killing the daemon would yank its data
    /// directory while it is still writing.
    tempdir: TempDir,
}

impl PersistHarness {
    /// Construct a harness. Allocates a fresh TempDir, a free port,
    /// and a pre-bound HTTP client targeting that port, but does NOT
    /// spawn the daemon — call [`Self::spawn`] for that.
    pub fn new(config: PersistConfig) -> Result<Self, HarnessError> {
        let tempdir = TempDir::new().map_err(HarnessError::Io)?;
        let port = find_free_port()?;
        let client = HttpClient::new(format!("http://127.0.0.1:{port}"))?;
        Ok(Self {
            config,
            port,
            client,
            child: None,
            tempdir,
        })
    }

    /// The TCP port the daemon will bind (or has bound) for its
    /// HTTP server.
    pub fn port(&self) -> u16 {
        self.port
    }

    /// Borrow the harness's HTTP client. Used by the integration test
    /// (cycle f2) and later by the benchmark driver loop (cycles g-j)
    /// to issue workload ops against the running daemon.
    pub fn client(&self) -> &HttpClient {
        &self.client
    }

    /// Spawn the `forge-daemon` subprocess and wait for it to answer
    /// an HTTP `Health` request. Returns `Ok(())` once the daemon is
    /// serving HTTP; returns `SpawnTimeout` if the Health endpoint
    /// does not succeed within `config.recovery_timeout`.
    ///
    /// The spawned daemon has its state isolated via
    /// `FORGE_DIR=<tempdir>/.forge` and its HTTP server enabled on
    /// a random free port on loopback. stdout/stderr are discarded
    /// to keep test output clean.
    pub fn spawn(&mut self) -> Result<(), HarnessError> {
        if self.child.is_some() {
            return Err(HarnessError::AlreadySpawned);
        }

        let forge_dir = self.tempdir.path().join(".forge");
        std::fs::create_dir_all(&forge_dir).map_err(HarnessError::Io)?;

        let spawn_instant = Instant::now();
        let child = Command::new(&self.config.daemon_bin)
            .env("FORGE_DIR", &forge_dir)
            .env("FORGE_HTTP_ENABLED", "true")
            .env("FORGE_HTTP_BIND", "127.0.0.1")
            .env("FORGE_HTTP_PORT", self.port.to_string())
            .env("RUST_LOG", "forge_daemon=warn")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(HarnessError::SpawnFailed)?;

        self.child = Some(child);

        let deadline = spawn_instant + self.config.recovery_timeout;
        while Instant::now() < deadline {
            if self.client.health().is_ok() {
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(50));
        }

        // Timeout — best-effort kill so we don't leak the child, then
        // report the timeout without shadowing it.
        let _ = self.kill();
        Err(HarnessError::SpawnTimeout {
            elapsed: spawn_instant.elapsed(),
            port: self.port,
        })
    }

    /// Send SIGKILL to the child process (via `Child::kill` which on
    /// Unix maps to `SIGKILL`), reap the zombie, and wait for the HTTP
    /// endpoint to stop responding. When no child is active this is a
    /// true no-op: it returns immediately without issuing any HTTP
    /// probes (adversarial review finding from cycle f2 — the previous
    /// implementation still ran the post-kill wait loop even when
    /// there was nothing to kill).
    pub fn kill(&mut self) -> Result<(), HarnessError> {
        let Some(mut child) = self.child.take() else {
            return Ok(());
        };
        child.kill().map_err(HarnessError::KillFailed)?;
        child.wait().map_err(HarnessError::KillFailed)?;

        // Brief wait loop bounded at 5 s. In practice `Child::wait`
        // already blocks until the kernel reaps the process, so by
        // the time we get here the port is usually already released
        // and the first Health probe fails immediately with
        // connection-refused.
        let deadline = Instant::now() + Duration::from_secs(5);
        while self.client.health().is_ok() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(50));
        }

        Ok(())
    }

    /// True iff the daemon is currently answering HTTP `Health`. Used
    /// by the integration test as a liveness predicate before and
    /// after SIGKILL. Upgraded in cycle (f2) from a raw TCP probe so
    /// that "alive" means the HTTP handler is actually serving, not
    /// just that the kernel listener is bound.
    pub fn is_daemon_alive(&self) -> bool {
        self.client.health().is_ok()
    }
}

impl Drop for PersistHarness {
    fn drop(&mut self) {
        // Best-effort cleanup — never panic in Drop. If the child is
        // still running we SIGKILL it; any errors are swallowed because
        // there is no sensible recovery from a Drop-time failure.
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

/// Discover a free TCP port on loopback. Binds a listener to port 0,
/// reads the kernel-assigned port, then drops the listener. The port
/// is RACE-PRONE between `drop` and the next `bind` call — the
/// daemon's eventual `bind` may lose the race if another process
/// grabs the port. In practice this is rare enough to ignore; the
/// caller can retry spawn if it happens.
fn find_free_port() -> Result<u16, HarnessError> {
    let listener = TcpListener::bind("127.0.0.1:0").map_err(HarnessError::BindFailed)?;
    let port = listener.local_addr().map_err(HarnessError::Io)?.port();
    drop(listener);
    Ok(port)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_workload_has_expected_op_count() {
        let config = WorkloadConfig {
            seed: 42,
            memories: 3,
            chunks: 0,
            fisp_messages: 0,
        };
        let ops = generate_workload(&config);
        assert_eq!(ops.len(), 3, "workload should contain N+K+J operations");
    }

    #[test]
    fn test_workload_order_differs_for_different_seeds() {
        // With a mixed workload (memories + chunks + fisp) and two
        // different seeds, the interleaving order MUST differ. Without
        // seed-driven shuffling, two configs that differ only in seed
        // would produce identical outputs — this test drives the need
        // for a ChaCha20 (or equivalent) seeded permutation.
        let config_a = WorkloadConfig {
            seed: 1,
            memories: 5,
            chunks: 5,
            fisp_messages: 5,
        };
        let config_b = WorkloadConfig {
            seed: 2,
            memories: 5,
            chunks: 5,
            fisp_messages: 5,
        };
        let ops_a = generate_workload(&config_a);
        let ops_b = generate_workload(&config_b);
        assert_ne!(
            ops_a, ops_b,
            "different seeds must produce different op orderings"
        );
    }

    #[test]
    fn test_workload_ops_have_populated_payload_fields() {
        // Each op must carry the data the harness will later marshal
        // into an HTTP request. Remember needs memory_type + title +
        // content + tags. IngestRaw needs content. FispSend needs
        // from_session + to_session + content.
        let config = WorkloadConfig {
            seed: 7,
            memories: 2,
            chunks: 2,
            fisp_messages: 2,
        };
        let ops = generate_workload(&config);
        assert_eq!(ops.len(), 6);
        for op in &ops {
            match op {
                Operation::Remember {
                    memory_type,
                    title,
                    content,
                    tags,
                    ..
                } => {
                    assert!(
                        MEMORY_TYPES.contains(&memory_type.as_str()),
                        "memory_type must be from the fixed vocabulary"
                    );
                    assert!(!title.is_empty(), "Remember title must be non-empty");
                    assert!(!content.is_empty(), "Remember content must be non-empty");
                    assert!(!tags.is_empty(), "Remember must carry at least one tag");
                    assert!(tags.len() >= 2, "§5.5 requires at least 2 tags per memory");
                }
                Operation::IngestRaw { content, .. } => {
                    assert!(!content.is_empty(), "IngestRaw content must be non-empty");
                }
                Operation::FispSend {
                    from_session,
                    to_session,
                    content,
                    ..
                } => {
                    assert!(!from_session.is_empty(), "FispSend from_session required");
                    assert!(!to_session.is_empty(), "FispSend to_session required");
                    assert!(!content.is_empty(), "FispSend content must be non-empty");
                    assert_ne!(from_session, to_session, "FispSend should not self-route");
                }
            }
        }
    }

    #[test]
    fn test_workload_zero_ops_is_empty_vec() {
        // Edge case: empty workload. Cycle (f)'s kill-offset calculation
        // (floor(F * total)) collapses to 0 when total is 0 — the harness
        // should handle this gracefully (SIGKILL fires before any op).
        // This test locks the empty-input contract before the harness
        // builds on top of it.
        let config = WorkloadConfig {
            seed: 0,
            memories: 0,
            chunks: 0,
            fisp_messages: 0,
        };
        let ops = generate_workload(&config);
        assert!(ops.is_empty(), "zero-op config must produce empty vec");
    }

    #[test]
    fn test_workload_content_varies_across_indexes() {
        // Each op within a kind should have unique content so the harness
        // can distinguish between ops during post-restart verification.
        // Without index-based content, all three Remember ops would share
        // the same payload and be indistinguishable.
        let config = WorkloadConfig {
            seed: 42,
            memories: 3,
            chunks: 0,
            fisp_messages: 0,
        };
        let ops = generate_workload(&config);
        let contents: std::collections::HashSet<_> = ops
            .iter()
            .map(|op| match op {
                Operation::Remember { content, .. } => content.clone(),
                _ => unreachable!(),
            })
            .collect();
        assert_eq!(
            contents.len(),
            3,
            "each memory should have distinct content"
        );
    }

    #[test]
    fn test_workload_is_deterministic_for_same_seed() {
        // Guard: generating twice with the same config produces
        // byte-identical output. ChaCha20 + index-based content make
        // this trivially true, but the test locks the guarantee.
        let config = WorkloadConfig {
            seed: 42,
            memories: 5,
            chunks: 5,
            fisp_messages: 5,
        };
        let ops_a = generate_workload(&config);
        let ops_b = generate_workload(&config);
        assert_eq!(ops_a, ops_b, "same seed must produce identical workload");
    }

    #[test]
    fn test_op_to_request_remember_produces_correct_request() {
        // Pure helper: translate Operation::Remember into the wire-level
        // Request::Remember variant. Drives the helper's existence and
        // the memory_type string → MemoryType enum conversion.
        let op = Operation::Remember {
            index: 0,
            memory_type: "decision".to_string(),
            title: "t".to_string(),
            content: "c".to_string(),
            tags: vec!["a".to_string(), "b".to_string()],
        };
        let req = op_to_request(&op);
        match req {
            Request::Remember {
                memory_type,
                title,
                content,
                tags,
                confidence,
                project,
                metadata,
            } => {
                assert_eq!(memory_type, MemoryType::Decision);
                assert_eq!(title, "t");
                assert_eq!(content, "c");
                assert_eq!(tags, Some(vec!["a".to_string(), "b".to_string()]));
                assert!(confidence.is_none());
                assert!(project.is_none());
                assert!(metadata.is_none());
            }
            other => panic!("expected Request::Remember, got {other:?}"),
        }
    }

    #[test]
    fn test_op_to_request_handles_all_memory_types() {
        // Guard: every vocab entry in MEMORY_TYPES must map to a real
        // MemoryType variant (no silent fallback to Decision). Protocol
        // specifically covers a prior adversarial-review concern where
        // an expanded MEMORY_TYPES vocab would have silently coerced
        // unknown strings to Decision.
        for (i, expected_mt) in [
            (0usize, MemoryType::Decision),
            (1, MemoryType::Lesson),
            (2, MemoryType::Pattern),
            (3, MemoryType::Preference),
            (4, MemoryType::Protocol),
        ] {
            let op = Operation::Remember {
                index: i,
                memory_type: MEMORY_TYPES[i].to_string(),
                title: "t".to_string(),
                content: "c".to_string(),
                tags: vec!["x".to_string()],
            };
            match op_to_request(&op) {
                Request::Remember { memory_type, .. } => {
                    assert_eq!(
                        memory_type, expected_mt,
                        "MEMORY_TYPES[{i}] should map to {expected_mt:?}"
                    );
                }
                other => panic!("expected Remember, got {other:?}"),
            }
        }
    }

    #[test]
    fn test_op_to_request_ingest_raw_produces_raw_ingest() {
        // IngestRaw wire shape: text body into the raw storage layer
        // with a fixed "forge-persist" source tag so the daemon can
        // attribute the ingest to this benchmark.
        let op = Operation::IngestRaw {
            index: 0,
            content: "a deterministic document body".to_string(),
        };
        match op_to_request(&op) {
            Request::RawIngest {
                text,
                source,
                project,
                session_id,
                ..
            } => {
                assert_eq!(text, "a deterministic document body");
                assert_eq!(source, HARNESS_SOURCE);
                assert!(project.is_none());
                assert!(session_id.is_none());
            }
            other => panic!("expected Request::RawIngest, got {other:?}"),
        }
    }

    #[test]
    fn test_canonical_hash_remember_uses_content_field() {
        // §6.2: Remember hash = sha256(content bytes), nothing else.
        // Known SHA-256 of the UTF-8 bytes of "hello world" (no trailing
        // newline) is the classic test vector — hardcoded here so a
        // broken hash function (different algorithm, wrong encoding,
        // off-by-one on the byte range) fails loudly instead of silently
        // round-tripping its own mistake.
        let op = Operation::Remember {
            index: 0,
            memory_type: "decision".to_string(),
            title: "anything".to_string(),
            content: "hello world".to_string(),
            tags: vec!["ignored".to_string()],
        };
        let hash = canonical_hash(&op);
        assert_eq!(
            hash, "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9",
            "hash must equal the known SHA-256 of 'hello world' bytes"
        );
    }

    #[test]
    fn test_canonical_hash_ingest_raw_uses_content_field() {
        // §6.2: IngestRaw hash = sha256(content bytes), same scheme as
        // Remember. Since "hello world" has a well-known SHA-256, we
        // reuse it here as the KAT. The fact that Remember and IngestRaw
        // share a hash for identical content is intentional: ids are
        // kind-scoped, so collisions cannot confuse verify_matches.
        let op = Operation::IngestRaw {
            index: 0,
            content: "hello world".to_string(),
        };
        let hash = canonical_hash(&op);
        assert_eq!(
            hash,
            "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9"
        );
    }

    #[test]
    fn test_canonical_hash_fisp_send_matches_serde_json_compact() {
        // §6.2: FispSend hash = sha256(serde_json::to_string(&parts)
        // bytes). For an op with content="hello", parts serializes as
        // exactly: [{"kind":"text","text":"hello"}]
        // (compact, no whitespace, struct field order preserved by
        // serde's derive, None fields elided via skip_serializing_if).
        //
        // Pre-computed offline with python3's hashlib.sha256 over those
        // exact bytes. If serde_json ever changes how it orders or
        // elides fields, this KAT fails loudly — exactly the tripwire
        // the design doc §6.2 version-bump requirement is protecting.
        let op = Operation::FispSend {
            index: 0,
            from_session: "persist_session_0".to_string(),
            to_session: "persist_session_1".to_string(),
            content: "hello".to_string(),
        };
        let hash = canonical_hash(&op);
        assert_eq!(
            hash,
            "20ae1a900410d7f6f6a0ad4a944d7b62c52f230016c80ab37603d2e4e130f390"
        );
    }

    #[test]
    fn test_canonical_hash_is_deterministic_for_same_op() {
        // Locks the "pure function" contract: the same op must always
        // produce the same hash, no matter how many times it is called.
        // If canonical_hash ever grows state (e.g., a timestamp or a
        // session-specific salt), this test fails.
        let op = Operation::Remember {
            index: 7,
            memory_type: "lesson".to_string(),
            title: "stable".to_string(),
            content: "stable body".to_string(),
            tags: vec!["a".to_string(), "b".to_string()],
        };
        let first = canonical_hash(&op);
        let second = canonical_hash(&op);
        assert_eq!(first, second, "canonical_hash must be deterministic");
        assert_eq!(first.len(), 64, "SHA-256 hex output is exactly 64 chars");
    }

    #[test]
    fn test_canonical_hash_differs_for_different_content() {
        // Guard against a trivial impl that returns a constant or hashes
        // something unrelated. Two ops with different body strings MUST
        // produce different hashes.
        let a = Operation::Remember {
            index: 0,
            memory_type: "decision".to_string(),
            title: "t".to_string(),
            content: "body A".to_string(),
            tags: vec![],
        };
        let b = Operation::Remember {
            index: 0,
            memory_type: "decision".to_string(),
            title: "t".to_string(),
            content: "body B".to_string(),
            tags: vec![],
        };
        assert_ne!(canonical_hash(&a), canonical_hash(&b));
    }

    #[test]
    fn test_canonical_hash_fisp_send_differs_for_different_content() {
        // FispSend content change → different hash. Parallels the
        // Remember test above; closes the gap where a constant-output
        // bug in the FispSend code path would slip past the single
        // FispSend KAT that only tests one content value.
        let a = Operation::FispSend {
            index: 0,
            from_session: "persist_session_0".to_string(),
            to_session: "persist_session_1".to_string(),
            content: "body A".to_string(),
        };
        let b = Operation::FispSend {
            index: 0,
            from_session: "persist_session_0".to_string(),
            to_session: "persist_session_1".to_string(),
            content: "body B".to_string(),
        };
        assert_ne!(canonical_hash(&a), canonical_hash(&b));
    }

    #[test]
    fn test_canonical_hash_fisp_matches_op_to_request_parts() {
        // Tripwire test — the critical invariant between `op_to_request`
        // and `canonical_hash`. Both paths MUST agree on the exact
        // `Vec<MessagePart>` shape that gets sent + hashed, because
        // cycle (h)'s consistency_rate == 1.00 requires byte-exact hash
        // match against the daemon's stored content.
        //
        // This test takes the REAL output of `op_to_request` for a
        // FispSend op, extracts its `parts` vec, recomputes the hash
        // from it, and asserts equality with `canonical_hash(&op)`.
        // If either side ever refactors the MessagePart shape without
        // updating the other (or, more importantly, without updating
        // the shared `fisp_parts` helper), this test fails loudly.
        let op = Operation::FispSend {
            index: 0,
            from_session: "persist_session_0".to_string(),
            to_session: "persist_session_1".to_string(),
            content: "tripwire payload".to_string(),
        };
        let expected_hash = match op_to_request(&op) {
            Request::SessionSend { parts, .. } => {
                let json = serde_json::to_string(&parts)
                    .expect("parts must serialize for the tripwire check");
                let digest = Sha256::digest(json.as_bytes());
                bytes_to_hex(&digest)
            }
            other => panic!("expected SessionSend from op_to_request, got {other:?}"),
        };
        assert_eq!(
            canonical_hash(&op),
            expected_hash,
            "canonical_hash must agree with the parts shape op_to_request actually sends"
        );
    }

    #[test]
    fn test_canonical_hash_remember_ignores_non_content_fields() {
        // §6.2: Remember hash = sha256(content bytes), nothing else.
        // Changing title, memory_type, tags, or index MUST NOT affect
        // the hash. This locks the content-only scheme against an
        // over-eager refactor that starts folding extra fields into
        // the canonical payload.
        let base = Operation::Remember {
            index: 0,
            memory_type: "decision".to_string(),
            title: "original".to_string(),
            content: "same body".to_string(),
            tags: vec!["a".to_string()],
        };
        let changed = Operation::Remember {
            index: 99,
            memory_type: "protocol".to_string(),
            title: "different title".to_string(),
            content: "same body".to_string(),
            tags: vec!["x".to_string(), "y".to_string(), "z".to_string()],
        };
        assert_eq!(
            canonical_hash(&base),
            canonical_hash(&changed),
            "Remember hash must depend only on the content field"
        );
    }

    #[test]
    fn test_op_to_request_fisp_send_produces_session_send() {
        // FispSend maps to Request::SessionSend with a single
        // MessagePart { kind: "text", text: Some(content) }. The
        // `to` field receives the to_session name, `from_session`
        // goes into the explicit field so it isn't defaulted to "api".
        let op = Operation::FispSend {
            index: 0,
            from_session: "persist_session_0".to_string(),
            to_session: "persist_session_1".to_string(),
            content: "hello".to_string(),
        };
        match op_to_request(&op) {
            Request::SessionSend {
                to,
                kind,
                topic,
                parts,
                from_session,
                ..
            } => {
                assert_eq!(to, "persist_session_1");
                assert_eq!(kind, "notification");
                assert_eq!(topic, HARNESS_TOPIC);
                assert_eq!(parts.len(), 1);
                assert_eq!(parts[0].kind, "text");
                assert_eq!(parts[0].text, Some("hello".to_string()));
                assert_eq!(from_session, Some("persist_session_0".to_string()));
            }
            other => panic!("expected Request::SessionSend, got {other:?}"),
        }
    }
}
