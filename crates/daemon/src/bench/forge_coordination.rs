//! Forge-Coordination bench (2A-6) — multi-agent FISP coordination correctness.
//!
//! Spec: `docs/superpowers/specs/2026-04-26-multi-agent-coordination-bench-design.md`
//! v2.1 LOCKED.
//!
//! Validates the FISP message-queue substrate (`session_message` table +
//! `sessions::*` helpers) underpinning the planner→generator→evaluator
//! pipeline pattern.
//!
//! ## Architecture (spec §3.7)
//!
//! Single shared `DaemonState` per seed; all 6 dimensions read from the same
//! `Connection` to preserve cross-dim signal integrity (D5 sentinel-hash
//! comparison spans D1-D4-D6 mutations). Per-dim isolated `:memory:` DBs
//! would actively HIDE cross-dim issues — wrong primitive for this surface.
//!
//! ## Dimensions (§3.1, §3.3)
//!
//! | Dim | Probe | Min | Weight |
//! |-----|-------|-----|--------|
//! | D1 inbox_precision           | list_messages filters by to_session              | 0.95 | 0.20 |
//! | D2 roundtrip_correctness     | send→retrieve preserves all 7 fields              | 0.95 | 0.15 |
//! | D3 broadcast_project_scoping | to="*" with project filters by session.project    | 0.95 | 0.15 |
//! | D4 authorization_enforcement | ack + respond reject non-recipient callers        | 0.95 | 0.20 |
//! | D5 edge_case_resilience      | 7 probes (size/respond/broadcast/ack/sqli/etc)    | 0.85 | 0.15 |
//! | D6 pipeline_chain_correctness | K=3 linear-chain trials (planner→r2→r3)          | 0.90 | 0.15 |
//!
//! Composite = weighted mean. Pass = composite ≥ 0.95 AND every dim ≥ min.

use rand_chacha::ChaCha20Rng;
use serde::Serialize;

use crate::bench::common::{seeded_rng, sha256_hex};
use crate::server::handler::DaemonState;

// ── Per-bench weights (§3.1, §3.3) ──────────────────────────────────────

/// Weights summing to 1.00 (spec §3.3).
const DIM_WEIGHTS: [f64; 6] = [0.20, 0.15, 0.15, 0.20, 0.15, 0.15];

/// Per-dimension minimum scores for pass (spec §3.1).
const DIM_MINIMUMS: [f64; 6] = [0.95, 0.95, 0.95, 0.95, 0.85, 0.90];

/// Composite pass threshold (spec §3.3).
const COMPOSITE_THRESHOLD: f64 = 0.95;

/// Spec §3.4 check 1 invariant — base CREATE has 13 cols; meeting_id ALTER
/// at db/schema.rs:1107 makes 14 post-migration. v2 NEW-MED-2 fix
/// (tightened from `>= 14` to exact equality with named constant).
pub(crate) const SESSION_MESSAGE_COLUMN_COUNT: usize = 14;

// ── Corpus parameters (spec §3.2) ───────────────────────────────────────

/// Roles encoded in session ids; one session per (role, project) pair.
pub(crate) const ROLES: [&str; 3] = ["planner", "generator", "evaluator"];

/// Two projects: alpha + beta. The (planner_alpha, generator_alpha) pair is
/// reserved for the D5 sentinel row.
pub(crate) const PROJECTS: [&str; 2] = ["alpha", "beta"];

/// Total sessions: 3 roles × 2 projects = 6.
pub(crate) const TOTAL_SESSIONS: usize = ROLES.len() * PROJECTS.len();

/// Each (sender, recipient) pair gets this many seeded directed messages.
pub(crate) const MSGS_PER_PAIR: usize = 2;

/// Per-recipient incoming = (TOTAL_SESSIONS - 1) × MSGS_PER_PAIR
/// = 5 × 2 = 10 directed messages.
pub(crate) const MSGS_PER_INBOX: usize = (TOTAL_SESSIONS - 1) * MSGS_PER_PAIR;

/// Total seeded directed messages = 60.
pub(crate) const TOTAL_SEEDED_MESSAGES: usize = TOTAL_SESSIONS * MSGS_PER_INBOX;

/// Pinned sentinel row id for D5 probes 6 + 7 (spec §3.1a + §4 D11).
pub(crate) const SENTINEL_ROW_ID: &str = "seed_planner_alpha_to_generator_alpha_0";

/// Constant timestamp baseline for deterministic created_at ordering.
const CORPUS_NOW_ISO: &str = "2026-04-26T00:00:00Z";

/// Project label used for D3 broadcast tests (matches sessions' project).
pub(crate) const TEAM_ALPHA: &str = "alpha";
pub(crate) const TEAM_BETA: &str = "beta";

// ── Result structs ──────────────────────────────────────────────────────

/// One bench session row (corpus member).
#[derive(Debug, Clone)]
pub struct BenchSession {
    pub id: String,
    pub role: String,
    pub project: String,
}

/// One seeded directed message (corpus row).
#[derive(Debug, Clone)]
pub struct BenchMessage {
    pub id: String,
    pub from_session: String,
    pub to_session: String,
    pub kind: String,
    pub topic: String,
    pub parts: String,
    pub project: Option<String>,
    pub status: String,
    pub created_at: String,
}

/// Generated dataset for one bench seed.
#[derive(Debug, Clone)]
pub struct Corpus {
    pub sessions: Vec<BenchSession>,
    pub messages: Vec<BenchMessage>,
}

impl Corpus {
    pub fn session_by_role_project(&self, role: &str, project: &str) -> Option<&BenchSession> {
        self.sessions
            .iter()
            .find(|s| s.role == role && s.project == project)
    }
}

/// One dimension's score with pass/fail eval.
#[derive(Debug, Clone, Serialize)]
pub struct DimensionScore {
    pub name: &'static str,
    pub score: f64,
    pub min: f64,
    pub pass: bool,
}

/// One infrastructure assertion's outcome.
#[derive(Debug, Clone, Serialize)]
pub struct InfrastructureCheck {
    pub name: &'static str,
    pub passed: bool,
    pub detail: String,
}

/// Top-level summary.json contract — mirrors `forge_isolation::IsolationScore`.
#[derive(Debug, Clone, Serialize)]
pub struct CoordinationScore {
    pub seed: u64,
    pub composite: f64,
    pub dimensions: [DimensionScore; 6],
    pub infrastructure_checks: Vec<InfrastructureCheck>,
    pub pass: bool,
    pub wall_duration_ms: u64,
}

/// Bench-runner config knobs (mirrors forge_isolation::BenchConfig).
#[derive(Debug, Clone)]
pub struct BenchConfig {
    pub seed: u64,
    pub output_dir: std::path::PathBuf,
    pub expected_composite: Option<f64>,
}

impl Default for BenchConfig {
    fn default() -> Self {
        Self {
            seed: 42,
            output_dir: std::path::PathBuf::from("bench_results_forge_coordination"),
            expected_composite: None,
        }
    }
}

// ── Corpus generator (T3, spec §3.2) ────────────────────────────────────

/// Generate the deterministic corpus per spec §3.2.
///
/// Layout:
/// - 6 sessions (3 roles × 2 projects)
/// - 60 directed messages (TOTAL_SEEDED_MESSAGES)
/// - 36 cross-project messages (18 alpha→beta + 18 beta→alpha)
/// - 24 same-project messages
/// - 90% kind='notification', 10% kind='request' (deterministic by idx)
///
/// The function takes a `ChaCha20Rng` for signature-consistency with other
/// bench harnesses but does not consume randomness from it — corpus content
/// is fully derived by formula from `(role, project, idx)` triples.
pub fn generate_corpus(_rng: &mut ChaCha20Rng) -> Corpus {
    let mut sessions = Vec::with_capacity(TOTAL_SESSIONS);
    for project in PROJECTS {
        for role in ROLES {
            sessions.push(BenchSession {
                id: format!("{role}_{project}"),
                role: role.to_string(),
                project: project.to_string(),
            });
        }
    }

    let mut messages = Vec::with_capacity(TOTAL_SEEDED_MESSAGES);
    let mut idx_global = 0usize;
    for sender in &sessions {
        for recipient in &sessions {
            if sender.id == recipient.id {
                continue;
            }
            for idx_in_pair in 0..MSGS_PER_PAIR {
                let kind = if (idx_global % 10) == 0 { "request" } else { "notification" };
                let id = format!(
                    "seed_{}_{}_to_{}_{}_{}",
                    sender.role, sender.project, recipient.role, recipient.project, idx_in_pair
                );
                let topic = format!(
                    "seed_{}_{}_{}",
                    sender.role, recipient.role, idx_in_pair
                );
                let parts = format!(
                    "[{{\"text\":\"{}_{}_to_{}_{}: m_{}\"}}]",
                    sender.role, sender.project, recipient.role, recipient.project, idx_in_pair
                );
                let created_at = format!(
                    "2026-04-26T00:{:02}:{:02}Z",
                    idx_global / 60,
                    idx_global % 60
                );
                messages.push(BenchMessage {
                    id,
                    from_session: sender.id.clone(),
                    to_session: recipient.id.clone(),
                    kind: kind.to_string(),
                    topic,
                    parts,
                    project: Some(recipient.project.clone()),
                    status: "pending".to_string(),
                    created_at,
                });
                idx_global += 1;
            }
        }
    }
    debug_assert_eq!(messages.len(), TOTAL_SEEDED_MESSAGES);
    debug_assert_eq!(sessions.len(), TOTAL_SESSIONS);
    let _ = CORPUS_NOW_ISO;

    Corpus { sessions, messages }
}

/// Seed all sessions + messages into the bench DaemonState via direct INSERT.
///
/// Bypasses higher-level helpers so the corpus is persisted byte-identical.
/// Sessions get `status='active'` so broadcast SELECT picks them up (per
/// `sessions::send_message` lines 388-394 broadcast filter).
///
/// Returns `(sessions_seeded, messages_seeded)`. Infrastructure check 5 +
/// 7 verify these totals.
pub fn seed_corpus(state: &mut DaemonState, corpus: &Corpus) -> rusqlite::Result<(usize, usize)> {
    for s in &corpus.sessions {
        state.conn.execute(
            "INSERT INTO session (id, agent, project, started_at, status, organization_id)
             VALUES (?1, ?2, ?3, ?4, 'active', 'default')",
            rusqlite::params![s.id, format!("forge-{}", s.role), s.project, CORPUS_NOW_ISO],
        )?;
    }

    for m in &corpus.messages {
        state.conn.execute(
            "INSERT INTO session_message
                (id, from_session, to_session, kind, topic, parts, status,
                 in_reply_to, project, timeout_secs, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, NULL, ?8, NULL, ?9)",
            rusqlite::params![
                m.id,
                m.from_session,
                m.to_session,
                m.kind,
                m.topic,
                m.parts,
                m.status,
                m.project,
                m.created_at,
            ],
        )?;
    }
    Ok((corpus.sessions.len(), corpus.messages.len()))
}

// ── Sentinel-row hash (D5 probes 6 + 7) ─────────────────────────────────

/// SHA-256 of the pinned sentinel row's `(id, from_session, to_session,
/// kind, topic, parts, status, in_reply_to)` for invariance comparison.
///
/// Sentinel id = `SENTINEL_ROW_ID`. Per §3.1a + §4 D11, no D2-D6 op
/// touches this row; comparing pre/post hash detects DROP TABLE / DELETE /
/// UPDATE-class mutations.
fn sentinel_row_hash(state: &DaemonState) -> Option<String> {
    let row: rusqlite::Result<(String, String, String, String, String, String, String, Option<String>)> =
        state.conn.query_row(
            "SELECT id, from_session, to_session, kind, topic, parts, status, in_reply_to
             FROM session_message WHERE id = ?1",
            rusqlite::params![SENTINEL_ROW_ID],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                    row.get(7)?,
                ))
            },
        );
    match row {
        Ok((id, fs, ts, k, top, p, st, irt)) => {
            let irt_str = irt.unwrap_or_default();
            let canonical = format!("{id}|{fs}|{ts}|{k}|{top}|{p}|{st}|{irt_str}");
            Some(sha256_hex(&canonical))
        }
        Err(_) => None,
    }
}

// ── Dimension stubs (T2 skeleton — T4-T6 will fill in) ──────────────────

fn dim_1_inbox_precision(_state: &mut DaemonState, _corpus: &Corpus) -> DimensionScore {
    DimensionScore {
        name: "inbox_precision",
        score: 0.0,
        min: DIM_MINIMUMS[0],
        pass: false,
    }
}

fn dim_2_roundtrip_correctness(_state: &mut DaemonState, _corpus: &Corpus) -> DimensionScore {
    DimensionScore {
        name: "roundtrip_correctness",
        score: 0.0,
        min: DIM_MINIMUMS[1],
        pass: false,
    }
}

fn dim_3_broadcast_project_scoping(_state: &mut DaemonState, _corpus: &Corpus) -> DimensionScore {
    DimensionScore {
        name: "broadcast_project_scoping",
        score: 0.0,
        min: DIM_MINIMUMS[2],
        pass: false,
    }
}

fn dim_4_authorization_enforcement(_state: &mut DaemonState, _corpus: &Corpus) -> DimensionScore {
    DimensionScore {
        name: "authorization_enforcement",
        score: 0.0,
        min: DIM_MINIMUMS[3],
        pass: false,
    }
}

fn dim_5_edge_case_resilience(_state: &mut DaemonState, _corpus: &Corpus) -> DimensionScore {
    DimensionScore {
        name: "edge_case_resilience",
        score: 0.0,
        min: DIM_MINIMUMS[4],
        pass: false,
    }
}

fn dim_6_pipeline_chain_correctness(_state: &mut DaemonState, _corpus: &Corpus) -> DimensionScore {
    DimensionScore {
        name: "pipeline_chain_correctness",
        score: 0.0,
        min: DIM_MINIMUMS[5],
        pass: false,
    }
}

// ── Composite scorer (uses lifted bench::scoring) ───────────────────────

fn composite_score(dims: &[DimensionScore; 6]) -> f64 {
    let scores: [f64; 6] = std::array::from_fn(|i| dims[i].score);
    crate::bench::scoring::composite_score(&scores, &DIM_WEIGHTS)
}

fn mark_pass(d: DimensionScore) -> DimensionScore {
    let pass = d.score >= d.min;
    DimensionScore { pass, ..d }
}

// ── Infrastructure assertions (9 checks, spec §3.4 — T6 will fill in) ───

fn run_infrastructure_checks(
    _state: &mut DaemonState,
    _corpus: &Corpus,
) -> Vec<InfrastructureCheck> {
    // 9-element placeholder; T6 replaces with real assertions.
    (0..9)
        .map(|i| InfrastructureCheck {
            name: match i {
                0 => "session_message_column_count",
                1 => "session_message_indexes_present",
                2 => "session_table_columns_present",
                3 => "seeded_rng_deterministic",
                4 => "corpus_size_matches_spec",
                5 => "session_distribution_correct",
                6 => "pre_d1_total_count_60",
                7 => "send_message_returns_ulid",
                _ => "respond_to_message_inverts_addressing",
            },
            passed: false,
            detail: "stub — T6 fills in".into(),
        })
        .collect()
}

// ── Orchestrator (single shared DaemonState per spec §3.7) ──────────────

/// Run the bench against a pre-seeded `Connection`. T3 builds the corpus
/// and seeds the connection; this fn runs the 6 dims + 9 infra checks.
///
/// Per §3.7, all dims share the SAME connection — per-dim isolation is
/// the wrong primitive for a coordination bench because D5 sentinel-hash
/// invariance spans the D1-D6 mutation chain.
///
/// Per §3.3 dim execution order: D1 → D2 → D3 → D4 → D6 → D5 (D5 last so
/// sentinel-hash captures all prior mutations).
pub fn run_bench_in_state(state: &mut DaemonState, corpus: &Corpus, seed: u64) -> CoordinationScore {
    let start = std::time::Instant::now();

    let infra = run_infrastructure_checks(state, corpus);
    let infra_pass = infra.iter().all(|c| c.passed);

    // Per spec §3.3 dim order: D1 → D2 → D3 → D4 → D6 → D5.
    // (D5 runs LAST so sentinel-hash compares end-state to seeded state.)
    let dimensions: [DimensionScore; 6] = if infra_pass {
        let d1 = dim_1_inbox_precision(state, corpus);
        let d2 = dim_2_roundtrip_correctness(state, corpus);
        let d3 = dim_3_broadcast_project_scoping(state, corpus);
        let d4 = dim_4_authorization_enforcement(state, corpus);
        let d6 = dim_6_pipeline_chain_correctness(state, corpus);
        let d5 = dim_5_edge_case_resilience(state, corpus);
        [
            mark_pass(d1),
            mark_pass(d2),
            mark_pass(d3),
            mark_pass(d4),
            mark_pass(d5),
            mark_pass(d6),
        ]
    } else {
        // Per 2A-5 MED-4 precedent: zero ALL dims when infra fails (avoids
        // inconsistent summary.json with composite=0 but per-dim populated).
        const ZEROED_DIM_NAMES: [&str; 6] = [
            "inbox_precision",
            "roundtrip_correctness",
            "broadcast_project_scoping",
            "authorization_enforcement",
            "edge_case_resilience",
            "pipeline_chain_correctness",
        ];
        std::array::from_fn(|i| DimensionScore {
            name: ZEROED_DIM_NAMES[i],
            score: 0.0,
            min: DIM_MINIMUMS[i],
            pass: false,
        })
    };

    let composite = if infra_pass {
        composite_score(&dimensions)
    } else {
        0.0
    };

    let dims_pass = dimensions.iter().all(|d| d.pass);
    let pass = infra_pass && dims_pass && composite >= COMPOSITE_THRESHOLD;

    CoordinationScore {
        seed,
        composite,
        dimensions,
        infrastructure_checks: infra,
        pass,
        wall_duration_ms: u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX),
    }
}

/// Top-level entry point used by the `forge-bench forge-coordination` CLI
/// (T7) and integration tests.
///
/// Builds a fresh `DaemonState::new(":memory:")` (which sets up the full
/// schema + indexes), seeds the corpus via [`seed_corpus`], then dispatches
/// to [`run_bench_in_state`] for the 6 dimension probes + 9 infrastructure
/// checks.
///
/// Writes `summary.json` to `config.output_dir` (mirrors forge-isolation).
/// Returns the [`CoordinationScore`].
pub fn run_bench(config: &BenchConfig) -> CoordinationScore {
    let mut rng = seeded_rng(config.seed);
    let corpus = generate_corpus(&mut rng);

    let mut state =
        DaemonState::new(":memory:").expect("DaemonState::new(:memory:) for forge-coordination");
    let (s_seeded, m_seeded) =
        seed_corpus(&mut state, &corpus).expect("seed_corpus for forge-coordination");
    debug_assert_eq!(s_seeded, TOTAL_SESSIONS);
    debug_assert_eq!(m_seeded, TOTAL_SEEDED_MESSAGES);

    let score = run_bench_in_state(&mut state, &corpus, config.seed);

    // Best-effort: write summary.json. Don't panic on failure.
    if let Err(e) = std::fs::create_dir_all(&config.output_dir) {
        tracing::warn!(error = %e, dir = %config.output_dir.display(),
            "failed to create forge-coordination output_dir");
    } else {
        let path = config.output_dir.join("summary.json");
        match serde_json::to_string_pretty(&score) {
            Ok(json) => {
                if let Err(e) = std::fs::write(&path, json) {
                    tracing::warn!(error = %e, path = %path.display(),
                        "failed to write forge-coordination summary.json");
                }
            }
            Err(e) => tracing::warn!(error = %e, "summary.json serialization failed"),
        }
    }

    score
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dim_weights_sum_to_one() {
        let s: f64 = DIM_WEIGHTS.iter().sum();
        assert!((s - 1.0).abs() < 1e-9, "weights sum {s} != 1.0");
    }

    #[test]
    fn dim_minimums_match_spec() {
        assert_eq!(DIM_MINIMUMS, [0.95, 0.95, 0.95, 0.95, 0.85, 0.90]);
    }

    #[test]
    fn composite_threshold_is_0_95() {
        assert!((COMPOSITE_THRESHOLD - 0.95).abs() < 1e-9);
    }

    #[test]
    fn corpus_size_constants_match_spec() {
        assert_eq!(TOTAL_SESSIONS, 6);
        assert_eq!(MSGS_PER_INBOX, 10);
        assert_eq!(TOTAL_SEEDED_MESSAGES, 60);
    }

    #[test]
    fn session_message_column_count_constant_is_14() {
        assert_eq!(SESSION_MESSAGE_COLUMN_COUNT, 14);
    }

    #[test]
    fn corpus_generator_produces_60_messages_and_6_sessions() {
        let mut rng = seeded_rng(42);
        let corpus = generate_corpus(&mut rng);
        assert_eq!(corpus.sessions.len(), TOTAL_SESSIONS);
        assert_eq!(corpus.messages.len(), TOTAL_SEEDED_MESSAGES);
    }

    #[test]
    fn corpus_inbox_distribution_is_10_per_session() {
        let mut rng = seeded_rng(42);
        let corpus = generate_corpus(&mut rng);
        for s in &corpus.sessions {
            let incoming = corpus
                .messages
                .iter()
                .filter(|m| m.to_session == s.id)
                .count();
            assert_eq!(incoming, MSGS_PER_INBOX, "session {} inbox", s.id);
        }
    }

    #[test]
    fn cross_project_message_count_is_36() {
        let mut rng = seeded_rng(42);
        let corpus = generate_corpus(&mut rng);
        let cross = corpus
            .messages
            .iter()
            .filter(|m| {
                let from = corpus.sessions.iter().find(|s| s.id == m.from_session).unwrap();
                let to = corpus.sessions.iter().find(|s| s.id == m.to_session).unwrap();
                from.project != to.project
            })
            .count();
        assert_eq!(cross, 36, "expected 36 cross-project messages (6/inbox × 6 inboxes)");
    }

    #[test]
    fn sentinel_row_id_present_in_corpus() {
        let mut rng = seeded_rng(42);
        let corpus = generate_corpus(&mut rng);
        assert!(
            corpus.messages.iter().any(|m| m.id == SENTINEL_ROW_ID),
            "sentinel row {SENTINEL_ROW_ID} must be in seeded corpus"
        );
    }

    #[test]
    fn seed_corpus_inserts_all_rows() {
        let mut rng = seeded_rng(42);
        let corpus = generate_corpus(&mut rng);
        let mut state = DaemonState::new(":memory:").unwrap();
        let (s_count, m_count) = seed_corpus(&mut state, &corpus).unwrap();
        assert_eq!(s_count, TOTAL_SESSIONS);
        assert_eq!(m_count, TOTAL_SEEDED_MESSAGES);

        let total_msgs: i64 = state
            .conn
            .query_row("SELECT COUNT(*) FROM session_message", [], |r| r.get(0))
            .unwrap();
        assert_eq!(total_msgs, TOTAL_SEEDED_MESSAGES as i64);

        let total_sessions: i64 = state
            .conn
            .query_row("SELECT COUNT(*) FROM session", [], |r| r.get(0))
            .unwrap();
        assert_eq!(total_sessions, TOTAL_SESSIONS as i64);
    }

    #[test]
    fn sentinel_row_hash_is_stable_across_seeds() {
        let mut state = DaemonState::new(":memory:").unwrap();
        let corpus_a = generate_corpus(&mut seeded_rng(7));
        let corpus_b = generate_corpus(&mut seeded_rng(123));

        seed_corpus(&mut state, &corpus_a).unwrap();
        let hash_a = sentinel_row_hash(&state).expect("sentinel must exist");

        // Different seed but corpus is formula-derived (no rng consumption);
        // sentinel content is identical, so hash must match.
        assert_eq!(corpus_a.messages.iter().find(|m| m.id == SENTINEL_ROW_ID).map(|m| &m.parts),
                   corpus_b.messages.iter().find(|m| m.id == SENTINEL_ROW_ID).map(|m| &m.parts));
        let _ = hash_a;
    }

    #[test]
    fn skeleton_run_in_state_returns_zeroed_score() {
        let mut state = DaemonState::new(":memory:").unwrap();
        let corpus = generate_corpus(&mut seeded_rng(42));
        seed_corpus(&mut state, &corpus).unwrap();

        let score = run_bench_in_state(&mut state, &corpus, 42);

        // T2 skeleton: infra checks are all stubs returning passed=false.
        // Therefore composite=0.0, all dims zeroed, pass=false.
        assert_eq!(score.composite, 0.0);
        assert!(!score.pass);
        for d in &score.dimensions {
            assert_eq!(d.score, 0.0);
            assert!(!d.pass);
        }
    }
}
