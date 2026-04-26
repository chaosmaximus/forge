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
                let kind = if idx_global.is_multiple_of(10) {
                    "request"
                } else {
                    "notification"
                };
                let id = format!(
                    "seed_{}_{}_to_{}_{}_{}",
                    sender.role, sender.project, recipient.role, recipient.project, idx_in_pair
                );
                let topic = format!("seed_{}_{}_{}", sender.role, recipient.role, idx_in_pair);
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
/// Tuple alias for the 8 canonical sentinel-row columns.
type SentinelRow = (
    String,
    String,
    String,
    String,
    String,
    String,
    String,
    Option<String>,
);

fn sentinel_row_hash(state: &DaemonState) -> Option<String> {
    let row: rusqlite::Result<SentinelRow> = state.conn.query_row(
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

// ── Dimension implementations (T4-T6) ───────────────────────────────────

/// **D1 — inbox_precision** (T4, weight 0.20, min 0.95).
///
/// For each session S: `list_messages(conn, &S.id, None, 1000, None)` —
/// every returned row must have `to_session=S.id`. Foreign-message
/// denominator computed at runtime per spec §3.3 + v1 H1 fix:
/// `pre_d1_total - (pre_d1_total / num_inboxes)`.
fn dim_1_inbox_precision(state: &mut DaemonState, corpus: &Corpus) -> DimensionScore {
    use crate::sessions::list_messages;

    let pre_d1_total: i64 = state
        .conn
        .query_row("SELECT COUNT(*) FROM session_message", [], |r| r.get(0))
        .unwrap_or(0);
    let num_inboxes = corpus.sessions.len() as i64;
    debug_assert!(num_inboxes > 0, "corpus must have ≥1 session");
    debug_assert_eq!(
        pre_d1_total % num_inboxes,
        0,
        "spec §3.3 D1 invariant: corpus must distribute messages evenly across inboxes \
         (pre_d1_total {pre_d1_total} not divisible by num_inboxes {num_inboxes})"
    );
    let max_possible_foreign = pre_d1_total - (pre_d1_total / num_inboxes);
    debug_assert!(max_possible_foreign > 0);

    let mut sum_score = 0.0;
    for s in &corpus.sessions {
        let inbox = list_messages(&state.conn, &s.id, None, 1000, None).unwrap_or_default();
        let foreign_count = inbox.iter().filter(|m| m.to_session != s.id).count();
        let score_s = 1.0 - (foreign_count as f64 / max_possible_foreign as f64);
        sum_score += score_s;
    }
    let score = sum_score / corpus.sessions.len() as f64;

    DimensionScore {
        name: "inbox_precision",
        score,
        min: DIM_MINIMUMS[0],
        pass: false,
    }
}

/// **D2 — roundtrip_correctness** (T4, weight 0.15, min 0.95).
///
/// For K=10 trials, send a fresh message via `sessions::send_message` then
/// retrieve via `list_messages` and verify all 7 fields round-trip.
/// Score = pass_count / (K × 7) = pass_count / 70.
fn dim_2_roundtrip_correctness(state: &mut DaemonState, corpus: &Corpus) -> DimensionScore {
    use crate::sessions::{list_messages, send_message};

    const K: usize = 10;
    const SUB_ASSERTIONS: usize = 7;
    let mut pass = 0u32;

    // Pick deterministic from→to pair: planner_alpha → evaluator_alpha
    // (avoids the sentinel pair planner_alpha → generator_alpha).
    let from = corpus
        .session_by_role_project("planner", TEAM_ALPHA)
        .expect("planner_alpha");
    let to = corpus
        .session_by_role_project("evaluator", TEAM_ALPHA)
        .expect("evaluator_alpha");

    for idx in 0..K {
        let topic = format!("d2_trial_{idx}");
        let parts = format!("[{{\"text\":\"d2_p_{idx}\"}}]");
        let project = Some(TEAM_ALPHA);
        let msg_id = match send_message(
            &state.conn,
            &from.id,
            &to.id,
            "notification",
            &topic,
            &parts,
            project,
            None,
            None,
        ) {
            Ok(id) => id,
            Err(_) => continue,
        };

        // Retrieve and find by id.
        let inbox =
            list_messages(&state.conn, &to.id, Some("pending"), 1000, None).unwrap_or_default();
        let row = inbox.iter().find(|m| m.id == msg_id);

        if let Some(r) = row {
            pass += 1; // (a) row found
            if r.from_session == from.id {
                pass += 1;
            } // (b) from
            if r.to_session == to.id {
                pass += 1;
            } // (c) to
            if r.topic == topic {
                pass += 1;
            } // (d) topic
            if r.parts == parts {
                pass += 1;
            } // (e) parts
            if r.kind == "notification" {
                pass += 1;
            } // (f) kind
            if r.project.as_deref() == Some(TEAM_ALPHA) {
                pass += 1;
            } // (g) project
        }
    }

    let score = f64::from(pass) / (K * SUB_ASSERTIONS) as f64;
    DimensionScore {
        name: "roundtrip_correctness",
        score,
        min: DIM_MINIMUMS[1],
        pass: false,
    }
}

/// **D3 — broadcast_project_scoping** (T5, weight 0.15, min 0.95).
///
/// For K=4 trials (one per role × project combo), broadcast and verify:
/// (a) delta = 2 (2 same-project peers excluding sender)
/// (b) all delta-rows have project = sender's project
/// (c) zero delta-rows addressed to other-project sessions
/// Score = pass_count / (K × 3) = pass / 12.
fn dim_3_broadcast_project_scoping(state: &mut DaemonState, corpus: &Corpus) -> DimensionScore {
    use crate::sessions::send_message;

    const SUB_ASSERTIONS: usize = 3;
    let trials: [(&str, &str); 4] = [
        ("planner", TEAM_ALPHA),
        ("generator", TEAM_ALPHA),
        ("planner", TEAM_BETA),
        ("generator", TEAM_BETA),
    ];
    let k = trials.len();
    let mut pass = 0u32;

    for (idx, (role, project)) in trials.iter().enumerate() {
        let sender = match corpus.session_by_role_project(role, project) {
            Some(s) => s,
            None => continue,
        };

        let pre: i64 = state
            .conn
            .query_row("SELECT COUNT(*) FROM session_message", [], |r| r.get(0))
            .unwrap_or(0);

        let topic = format!("d3_broadcast_{idx}");
        let parts = format!("[{{\"text\":\"d3_b_{idx}\"}}]");

        if send_message(
            &state.conn,
            &sender.id,
            "*",
            "notification",
            &topic,
            &parts,
            Some(project),
            None,
            None,
        )
        .is_err()
        {
            continue;
        }

        let post: i64 = state
            .conn
            .query_row("SELECT COUNT(*) FROM session_message", [], |r| r.get(0))
            .unwrap_or(0);
        let delta = post - pre;

        // (a) delta == 2 (sender excluded by `id != ?2`; 2 same-project peers remain).
        if delta == 2 {
            pass += 1;
        }

        // Find the new rows from this broadcast (latest 2 by topic match).
        let new_rows: Vec<(String, Option<String>)> = state
            .conn
            .prepare(
                "SELECT to_session, project FROM session_message
                 WHERE from_session = ?1 AND topic = ?2",
            )
            .ok()
            .and_then(|mut stmt| {
                stmt.query_map(rusqlite::params![sender.id, topic], |r| {
                    Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?))
                })
                .ok()
                .map(|rs| rs.flatten().collect())
            })
            .unwrap_or_default();

        // (b) all new rows have project = sender's project
        if !new_rows.is_empty() && new_rows.iter().all(|(_, p)| p.as_deref() == Some(*project)) {
            pass += 1;
        }

        // (c) zero new rows addressed to other-project sessions
        let other_project = if *project == TEAM_ALPHA {
            TEAM_BETA
        } else {
            TEAM_ALPHA
        };
        let other_session_ids: Vec<&String> = corpus
            .sessions
            .iter()
            .filter(|s| s.project == other_project)
            .map(|s| &s.id)
            .collect();
        let leak_count = new_rows
            .iter()
            .filter(|(to, _)| other_session_ids.iter().any(|oid| **oid == *to))
            .count();
        if leak_count == 0 {
            pass += 1;
        }
    }

    let score = f64::from(pass) / (k * SUB_ASSERTIONS) as f64;
    DimensionScore {
        name: "broadcast_project_scoping",
        score,
        min: DIM_MINIMUMS[2],
        pass: false,
    }
}

/// **D4 — authorization_enforcement** (T5, weight 0.20, min 0.95).
///
/// Two sub-classes:
/// - Ack ownership (K=3): non-recipient calling ack_messages must affect 0 rows + leave status pending.
/// - Respond authorization (K=3): non-recipient calling respond_to_message must return false + leave status unchanged + insert no response row.
///
/// Score = (ack_pass + respond_pass) / (3*2 + 3*3) = pass / 15.
fn dim_4_authorization_enforcement(state: &mut DaemonState, corpus: &Corpus) -> DimensionScore {
    use crate::sessions::{ack_messages, respond_to_message, send_message};

    const K_ACK: usize = 3;
    const K_RESPOND: usize = 3;
    const ACK_ASSERTIONS: usize = 2;
    const RESPOND_ASSERTIONS: usize = 3;

    // Pick stable session triplet in team-alpha avoiding sentinel pair (planner_alpha, generator_alpha):
    // sender = generator_alpha, recipient = evaluator_alpha, attacker = planner_alpha.
    let a = corpus
        .session_by_role_project("generator", TEAM_ALPHA)
        .expect("generator_alpha");
    let b = corpus
        .session_by_role_project("evaluator", TEAM_ALPHA)
        .expect("evaluator_alpha");
    let c = corpus
        .session_by_role_project("planner", TEAM_ALPHA)
        .expect("planner_alpha");

    let mut pass = 0u32;

    // ── Ack ownership probes ─────────────────────────────────────────────
    for idx in 0..K_ACK {
        let topic = format!("d4_ack_{idx}");
        let parts = format!("[{{\"text\":\"d4_a_{idx}\"}}]");
        let m_id = match send_message(
            &state.conn,
            &a.id,
            &b.id,
            "notification",
            &topic,
            &parts,
            Some(TEAM_ALPHA),
            None,
            None,
        ) {
            Ok(id) => id,
            Err(_) => continue,
        };

        let count =
            ack_messages(&state.conn, std::slice::from_ref(&m_id), &c.id).unwrap_or(usize::MAX);
        if count == 0 {
            pass += 1;
        }

        let status: String = state
            .conn
            .query_row(
                "SELECT status FROM session_message WHERE id = ?1",
                rusqlite::params![m_id],
                |r| r.get(0),
            )
            .unwrap_or_else(|_| "missing".to_string());
        if status == "pending" {
            pass += 1;
        }
    }

    // ── Respond authorization probes ─────────────────────────────────────
    for idx in 0..K_RESPOND {
        let topic = format!("d4_respond_{idx}");
        let parts = format!("[{{\"text\":\"d4_r_{idx}\"}}]");
        let m_id = match send_message(
            &state.conn,
            &a.id,
            &b.id,
            "request",
            &topic,
            &parts,
            Some(TEAM_ALPHA),
            None,
            None,
        ) {
            Ok(id) => id,
            Err(_) => continue,
        };

        // c (non-recipient) tries to respond
        let result = respond_to_message(&state.conn, &m_id, &c.id, "completed", "[]");
        if matches!(result, Ok(false)) {
            pass += 1;
        }

        let status: String = state
            .conn
            .query_row(
                "SELECT status FROM session_message WHERE id = ?1",
                rusqlite::params![m_id],
                |r| r.get(0),
            )
            .unwrap_or_else(|_| "missing".to_string());
        if status == "pending" {
            pass += 1;
        }

        // No response row was inserted (no row with in_reply_to=m_id)
        let reply_count: i64 = state
            .conn
            .query_row(
                "SELECT COUNT(*) FROM session_message WHERE in_reply_to = ?1",
                rusqlite::params![m_id],
                |r| r.get(0),
            )
            .unwrap_or(-1);
        if reply_count == 0 {
            pass += 1;
        }
    }

    let max = K_ACK * ACK_ASSERTIONS + K_RESPOND * RESPOND_ASSERTIONS; // = 15
    let score = f64::from(pass) / max as f64;
    DimensionScore {
        name: "authorization_enforcement",
        score,
        min: DIM_MINIMUMS[3],
        pass: false,
    }
}

/// **D6 — pipeline_chain_correctness** (T6, weight 0.15, min 0.90).
///
/// K=3 linear-chain trials per spec §3.1 + §4 D11. Each trial: r1→r2→r3
/// with reverse responses. Trial 3 in team-alpha skips the sentinel pair
/// `(planner_alpha, generator_alpha)` by using planner→evaluator→generator.
///
/// 6 sub-assertions per trial; total 18.
fn dim_6_pipeline_chain_correctness(state: &mut DaemonState, corpus: &Corpus) -> DimensionScore {
    use crate::sessions::{list_messages, respond_to_message, send_message};

    // (project, [r1_role, r2_role, r3_role], outer_resp_status, inner_resp_status)
    let trials: [(&str, [&str; 3], &str, &str); 3] = [
        // Trial 1: team-beta forward chain (planner → generator → evaluator).
        (
            TEAM_BETA,
            ["planner", "generator", "evaluator"],
            "accepted",
            "completed",
        ),
        // Trial 2: team-beta reverse-role chain (planner → evaluator → generator).
        (
            TEAM_BETA,
            ["planner", "evaluator", "generator"],
            "rejected",
            "failed",
        ),
        // Trial 3: team-alpha sentinel-disjoint chain (planner → evaluator → generator).
        // Skips the (planner_alpha, generator_alpha) sentinel pair entirely.
        (
            TEAM_ALPHA,
            ["planner", "evaluator", "generator"],
            "accepted",
            "completed",
        ),
    ];

    const ASSERTIONS_PER_TRIAL: usize = 6;
    let mut pass = 0u32;

    for (trial_idx, (project, roles, outer_status, inner_status)) in trials.iter().enumerate() {
        let r1 = corpus.session_by_role_project(roles[0], project).unwrap();
        let r2 = corpus.session_by_role_project(roles[1], project).unwrap();
        let r3 = corpus.session_by_role_project(roles[2], project).unwrap();

        // Step 1: r1 → r2 (M_outer, kind=request)
        let m_outer_id = match send_message(
            &state.conn,
            &r1.id,
            &r2.id,
            "request",
            &format!("d6_t{trial_idx}_outer"),
            &format!("[{{\"text\":\"d6_t{trial_idx}_outer\"}}]"),
            Some(project),
            None,
            None,
        ) {
            Ok(id) => id,
            Err(_) => continue,
        };
        // Sentinel preservation: orig_id passed to respond_to_message must NEVER be sentinel.
        debug_assert_ne!(
            m_outer_id, SENTINEL_ROW_ID,
            "d6 outer must not equal sentinel"
        );

        // Step 2: r2 → r3 (M_inner, kind=request)
        let m_inner_id = match send_message(
            &state.conn,
            &r2.id,
            &r3.id,
            "request",
            &format!("d6_t{trial_idx}_inner"),
            &format!("[{{\"text\":\"d6_t{trial_idx}_inner\"}}]"),
            Some(project),
            None,
            None,
        ) {
            Ok(id) => id,
            Err(_) => continue,
        };
        debug_assert_ne!(
            m_inner_id, SENTINEL_ROW_ID,
            "d6 inner must not equal sentinel"
        );

        // Step 3: r3 responds to M_inner → creates M_inner_resp
        if respond_to_message(&state.conn, &m_inner_id, &r3.id, inner_status, "[]").is_err() {
            continue;
        }

        // Step 4: r2 responds to M_outer → creates M_outer_resp
        if respond_to_message(&state.conn, &m_outer_id, &r2.id, outer_status, "[]").is_err() {
            continue;
        }

        // (a) M_outer.status post-respond
        let outer_actual: String = state
            .conn
            .query_row(
                "SELECT status FROM session_message WHERE id = ?1",
                rusqlite::params![m_outer_id],
                |r| r.get(0),
            )
            .unwrap_or_default();
        if outer_actual == *outer_status {
            pass += 1;
        }

        // (b) M_outer_resp shape: from=r2, to=r1, kind=response, in_reply_to=m_outer, status=outer_status
        let outer_resp: Option<(String, String, String, Option<String>, String)> = state
            .conn
            .query_row(
                "SELECT from_session, to_session, kind, in_reply_to, status FROM session_message
                 WHERE in_reply_to = ?1",
                rusqlite::params![m_outer_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
            )
            .ok();
        if let Some((fs, ts, k, irt, st)) = &outer_resp {
            if fs == &r2.id
                && ts == &r1.id
                && k == "response"
                && irt.as_deref() == Some(m_outer_id.as_str())
                && st == outer_status
            {
                pass += 1;
            }
        }

        // (c) M_inner.status post-respond
        let inner_actual: String = state
            .conn
            .query_row(
                "SELECT status FROM session_message WHERE id = ?1",
                rusqlite::params![m_inner_id],
                |r| r.get(0),
            )
            .unwrap_or_default();
        if inner_actual == *inner_status {
            pass += 1;
        }

        // (d) M_inner_resp shape: from=r3, to=r2, kind=response, in_reply_to=m_inner, status=inner_status
        let inner_resp: Option<(String, String, String, Option<String>, String)> = state
            .conn
            .query_row(
                "SELECT from_session, to_session, kind, in_reply_to, status FROM session_message
                 WHERE in_reply_to = ?1",
                rusqlite::params![m_inner_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
            )
            .ok();
        if let Some((fs, ts, k, irt, st)) = &inner_resp {
            if fs == &r3.id
                && ts == &r2.id
                && k == "response"
                && irt.as_deref() == Some(m_inner_id.as_str())
                && st == inner_status
            {
                pass += 1;
            }
        }

        // (e) M_outer_resp retrievable via list_messages(r1)
        let r1_inbox = list_messages(&state.conn, &r1.id, None, 1000, None).unwrap_or_default();
        if r1_inbox
            .iter()
            .any(|m| m.in_reply_to.as_deref() == Some(&m_outer_id))
        {
            pass += 1;
        }

        // (f) M_inner_resp retrievable via list_messages(r2)
        let r2_inbox = list_messages(&state.conn, &r2.id, None, 1000, None).unwrap_or_default();
        if r2_inbox
            .iter()
            .any(|m| m.in_reply_to.as_deref() == Some(&m_inner_id))
        {
            pass += 1;
        }
    }

    let max = trials.len() * ASSERTIONS_PER_TRIAL; // = 18
    let score = f64::from(pass) / max as f64;
    DimensionScore {
        name: "pipeline_chain_correctness",
        score,
        min: DIM_MINIMUMS[5],
        pass: false,
    }
}

/// **D5 — edge_case_resilience** (T6, weight 0.15, min 0.85).
///
/// 7 probes per spec §3.1a. D5 runs LAST per §3.3 dim execution order so
/// its sentinel-row hash captures the cumulative state from D1-D4-D6.
/// Spec §4 D11: no prior dim mutates `SENTINEL_ROW_ID`.
///
/// 1. payload_size_limit_enforced (65537-byte → Err containing "exceed 64KB limit")
/// 2. payload_at_limit_succeeds (65536-byte → Ok, row inserted)
/// 3. send_to_nonexistent_session_no_panic (no recipient validation per fact 9)
/// 4. respond_to_nonexistent_message_returns_false
/// 5. empty_broadcast_zero_inserts (project with no active sessions)
/// 6. empty_ack_returns_zero (sentinel-hash unchanged)
/// 7. sql_injection_in_topic_inert (sentinel-hash unchanged + table still queryable)
fn dim_5_edge_case_resilience(state: &mut DaemonState, corpus: &Corpus) -> DimensionScore {
    use crate::sessions::{ack_messages, respond_to_message, send_message};

    let from = &corpus.sessions[0].id;
    let to = &corpus.sessions[1].id;

    let mut passes = 0u32;

    // Probe 1: 65537-byte parts_json → Err containing "exceed 64KB limit"
    let oversize_parts = "x".repeat(65537);
    match send_message(
        &state.conn,
        from,
        to,
        "notification",
        "d5_oversize",
        &oversize_parts,
        Some(TEAM_ALPHA),
        None,
        None,
    ) {
        Err(rusqlite::Error::InvalidParameterName(msg)) if msg.contains("exceed 64KB limit") => {
            passes += 1;
        }
        _ => {}
    }

    // Probe 2: exactly 65536-byte parts_json → Ok + row inserted
    let boundary_parts = "x".repeat(65536);
    if let Ok(msg_id) = send_message(
        &state.conn,
        from,
        to,
        "notification",
        "d5_boundary",
        &boundary_parts,
        Some(TEAM_ALPHA),
        None,
        None,
    ) {
        let exists: bool = state
            .conn
            .query_row(
                "SELECT 1 FROM session_message WHERE id = ?1",
                rusqlite::params![msg_id],
                |_r| Ok(true),
            )
            .unwrap_or(false);
        if exists {
            passes += 1;
        }
    }

    // Probe 3: send to nonexistent session → Ok + row inserted (no recipient validation)
    if let Ok(msg_id) = send_message(
        &state.conn,
        from,
        "zzz_nonexistent_session_xxx",
        "notification",
        "d5_nonexistent",
        "[]",
        Some(TEAM_ALPHA),
        None,
        None,
    ) {
        let exists: bool = state
            .conn
            .query_row(
                "SELECT 1 FROM session_message WHERE id = ?1",
                rusqlite::params![msg_id],
                |_r| Ok(true),
            )
            .unwrap_or(false);
        if exists {
            passes += 1;
        }
    }

    // Probe 4: respond to nonexistent message_id → Ok(false)
    if matches!(
        respond_to_message(
            &state.conn,
            "zzz_nonexistent_msg_xxx",
            from,
            "completed",
            "[]"
        ),
        Ok(false)
    ) {
        passes += 1;
    }

    // Probe 5: empty broadcast (project with no active sessions) → Ok + 0 INSERTs
    let pre: i64 = state
        .conn
        .query_row("SELECT COUNT(*) FROM session_message", [], |r| r.get(0))
        .unwrap_or(0);
    let bcast_result = send_message(
        &state.conn,
        from,
        "*",
        "notification",
        "d5_empty_bcast",
        "[]",
        Some("zzz_no_active_sessions"),
        None,
        None,
    );
    let post: i64 = state
        .conn
        .query_row("SELECT COUNT(*) FROM session_message", [], |r| r.get(0))
        .unwrap_or(0);
    if bcast_result.is_ok() && (post - pre) == 0 {
        passes += 1;
    }

    // Probe 6: empty ack list → Ok(0); sentinel-hash unchanged
    let pre_hash = sentinel_row_hash(state);
    if matches!(ack_messages(&state.conn, &[], "any_caller"), Ok(0)) {
        let post_hash = sentinel_row_hash(state);
        if pre_hash == post_hash && pre_hash.is_some() {
            passes += 1;
        }
    }

    // Probe 7: SQL injection via topic → Ok + table still queryable + sentinel-hash unchanged
    let pre_hash = sentinel_row_hash(state);
    let evil_topic = "alpha'; DROP TABLE session_message;--";
    let send_ok = send_message(
        &state.conn,
        from,
        to,
        "notification",
        evil_topic,
        "[]",
        Some(TEAM_ALPHA),
        None,
        None,
    )
    .is_ok();
    let table_alive: bool = state
        .conn
        .query_row("SELECT 1 FROM session_message LIMIT 1", [], |_r| Ok(true))
        .unwrap_or(false);
    let post_hash = sentinel_row_hash(state);
    if send_ok && table_alive && pre_hash == post_hash && pre_hash.is_some() {
        passes += 1;
    }

    let score = f64::from(passes) / 7.0;
    DimensionScore {
        name: "edge_case_resilience",
        score,
        min: DIM_MINIMUMS[4],
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

// ── Infrastructure assertions (9 checks, spec §3.4) ─────────────────────

/// 9 fail-fast checks before dimensions run.
fn run_infrastructure_checks(state: &mut DaemonState, corpus: &Corpus) -> Vec<InfrastructureCheck> {
    use crate::sessions::{respond_to_message, send_message};

    let mut out = Vec::with_capacity(9);

    // 1. session_message column count == 14 (per v2.1 NM2 fix; tight equality).
    let cols: usize = state
        .conn
        .prepare("SELECT * FROM session_message LIMIT 0")
        .map(|stmt| stmt.column_count())
        .unwrap_or(0);
    let cols_ok = cols == SESSION_MESSAGE_COLUMN_COUNT;
    out.push(InfrastructureCheck {
        name: "session_message_column_count",
        passed: cols_ok,
        detail: format!(
            "session_message has {cols} columns (expected {SESSION_MESSAGE_COLUMN_COUNT})"
        ),
    });

    // 2. All 4 indexes present.
    let mut idx_ok = true;
    let mut idx_detail = String::new();
    for idx_name in [
        "idx_msg_to",
        "idx_msg_from",
        "idx_msg_reply",
        "idx_msg_meeting",
    ] {
        let exists: bool = state
            .conn
            .query_row(
                "SELECT 1 FROM sqlite_master WHERE type='index' AND name=?1",
                rusqlite::params![idx_name],
                |_r| Ok(true),
            )
            .unwrap_or(false);
        if !exists {
            idx_ok = false;
            idx_detail.push_str(&format!("{idx_name} MISSING; "));
        }
    }
    out.push(InfrastructureCheck {
        name: "session_message_indexes_present",
        passed: idx_ok,
        detail: if idx_ok {
            "idx_msg_to + idx_msg_from + idx_msg_reply + idx_msg_meeting all present".into()
        } else {
            idx_detail
        },
    });

    // 3. session table relevant columns present.
    let mut sess_cols_ok = true;
    let mut sess_detail = String::new();
    for col in [
        "id",
        "agent",
        "project",
        "status",
        "started_at",
        "organization_id",
    ] {
        let probe = state
            .conn
            .prepare(&format!("SELECT {col} FROM session LIMIT 0"))
            .is_ok();
        if !probe {
            sess_cols_ok = false;
            sess_detail.push_str(&format!("{col} MISSING; "));
        }
    }
    out.push(InfrastructureCheck {
        name: "session_table_columns_present",
        passed: sess_cols_ok,
        detail: if sess_cols_ok {
            "session has id, agent, project, status, started_at, organization_id".into()
        } else {
            sess_detail
        },
    });

    // 4. seeded_rng deterministic.
    use rand::RngExt;
    let mut a = seeded_rng(42);
    let mut b = seeded_rng(42);
    let v_a: u64 = a.random();
    let v_b: u64 = b.random();
    let det = v_a == v_b;
    out.push(InfrastructureCheck {
        name: "seeded_rng_deterministic",
        passed: det,
        detail: if det {
            "seeded_rng(42) produces same u64 twice".into()
        } else {
            format!("seeded_rng diverged: {v_a} != {v_b}")
        },
    });

    // 5. Corpus struct shape matches spec (in-memory).
    let size_ok =
        corpus.sessions.len() == TOTAL_SESSIONS && corpus.messages.len() == TOTAL_SEEDED_MESSAGES;
    out.push(InfrastructureCheck {
        name: "corpus_size_matches_spec",
        passed: size_ok,
        detail: format!(
            "corpus: {} sessions (expected {}), {} messages (expected {})",
            corpus.sessions.len(),
            TOTAL_SESSIONS,
            corpus.messages.len(),
            TOTAL_SEEDED_MESSAGES,
        ),
    });

    // 6. Session distribution + per-recipient-inbox count (in-memory cross-check).
    let mut dist_ok = true;
    let mut dist_detail = String::new();
    for project in PROJECTS {
        for role in ROLES {
            let n = corpus
                .sessions
                .iter()
                .filter(|s| s.role == role && s.project == project)
                .count();
            if n != 1 {
                dist_ok = false;
                dist_detail.push_str(&format!("({role},{project})={n}; "));
            }
        }
    }
    for s in &corpus.sessions {
        let inbox = corpus
            .messages
            .iter()
            .filter(|m| m.to_session == s.id)
            .count();
        if inbox != MSGS_PER_INBOX {
            dist_ok = false;
            dist_detail.push_str(&format!("inbox({})={inbox}; ", s.id));
        }
    }
    out.push(InfrastructureCheck {
        name: "session_distribution_correct",
        passed: dist_ok,
        detail: if dist_ok {
            format!("3 roles × 2 projects = 6 sessions; {MSGS_PER_INBOX} incoming each")
        } else {
            format!("distribution drift: {dist_detail}")
        },
    });

    // 7. pre-D1 count == 60 (DB-level).
    let total: i64 = state
        .conn
        .query_row("SELECT COUNT(*) FROM session_message", [], |r| r.get(0))
        .unwrap_or(-1);
    let total_ok = total == TOTAL_SEEDED_MESSAGES as i64;
    out.push(InfrastructureCheck {
        name: "pre_d1_total_count_60",
        passed: total_ok,
        detail: format!(
            "post-seed_corpus session_message count = {total} (expected {TOTAL_SEEDED_MESSAGES})"
        ),
    });

    // Probes 8 + 9 are wrapped in a SAVEPOINT and ROLLBACK-ed after
    // verification so the synthetic rows do NOT pollute the canonical
    // pre-D1 corpus shape (post-rollback session_message count == 60 still).
    // This preserves spec §3.3 D1 invariant `pre_d1_total % num_inboxes == 0`.
    let savepoint_ok = state
        .conn
        .execute_batch("SAVEPOINT infra_probes_8_9")
        .is_ok();

    // 8. send_message returns ULID (26 chars).
    let probe_id = send_message(
        &state.conn,
        "infra_check_from",
        "infra_check_to",
        "notification",
        "infra_probe_8",
        "[]",
        None,
        None,
        None,
    );
    let ulid_ok = matches!(&probe_id, Ok(id) if id.len() == 26);
    out.push(InfrastructureCheck {
        name: "send_message_returns_ulid",
        passed: ulid_ok,
        detail: match &probe_id {
            Ok(id) => format!("send_message returned id len={} (expected 26)", id.len()),
            Err(e) => format!("send_message errored: {e}"),
        },
    });

    // 9. respond_to_message inverts addressing — synthetic ids per v1 H2 +
    //    v2.1 NH1 (sentinel-row contract preservation).
    let probe_orig_id = send_message(
        &state.conn,
        "infra_check_from",
        "infra_check_to",
        "request",
        "infra_probe_9_orig",
        "[]",
        None,
        None,
        None,
    )
    .unwrap_or_default();
    let resp_ok = if probe_orig_id.is_empty() {
        false
    } else {
        let resp_call = respond_to_message(
            &state.conn,
            &probe_orig_id,
            "infra_check_to",
            "completed",
            "[]",
        );
        let row: Option<(String, String, Option<String>)> = state
            .conn
            .query_row(
                "SELECT from_session, to_session, in_reply_to FROM session_message
                 WHERE in_reply_to = ?1",
                rusqlite::params![probe_orig_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .ok();
        matches!(resp_call, Ok(true))
            && matches!(row, Some((ref f, ref t, ref irt))
                if f == "infra_check_to"
                && t == "infra_check_from"
                && irt.as_deref() == Some(probe_orig_id.as_str()))
    };
    out.push(InfrastructureCheck {
        name: "respond_to_message_inverts_addressing",
        passed: resp_ok,
        detail: if resp_ok {
            "respond_to_message inverts (from↔to) and sets in_reply_to to orig_id".into()
        } else {
            "respond_to_message did NOT invert addressing correctly".into()
        },
    });

    // Roll back probes 8 + 9 so the bench corpus is exactly 60 rows when
    // D1 runs. If the SAVEPOINT didn't open (rare), DELETE by id as fallback.
    if savepoint_ok {
        let _ = state.conn.execute_batch(
            "ROLLBACK TO SAVEPOINT infra_probes_8_9; RELEASE SAVEPOINT infra_probes_8_9",
        );
    } else {
        // Fallback: explicit DELETEs by infra-check session-id markers.
        let _ = state.conn.execute(
            "DELETE FROM session_message
             WHERE from_session IN ('infra_check_from', 'infra_check_to')
                OR to_session IN ('infra_check_from', 'infra_check_to')",
            [],
        );
    }

    out
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
pub fn run_bench_in_state(
    state: &mut DaemonState,
    corpus: &Corpus,
    seed: u64,
) -> CoordinationScore {
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
                let from = corpus
                    .sessions
                    .iter()
                    .find(|s| s.id == m.from_session)
                    .unwrap();
                let to = corpus
                    .sessions
                    .iter()
                    .find(|s| s.id == m.to_session)
                    .unwrap();
                from.project != to.project
            })
            .count();
        assert_eq!(
            cross, 36,
            "expected 36 cross-project messages (6/inbox × 6 inboxes)"
        );
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
        assert_eq!(
            corpus_a
                .messages
                .iter()
                .find(|m| m.id == SENTINEL_ROW_ID)
                .map(|m| &m.parts),
            corpus_b
                .messages
                .iter()
                .find(|m| m.id == SENTINEL_ROW_ID)
                .map(|m| &m.parts)
        );
        let _ = hash_a;
    }

    #[test]
    fn end_to_end_run_passes_on_seed_42() {
        let mut state = DaemonState::new(":memory:").unwrap();
        let corpus = generate_corpus(&mut seeded_rng(42));
        seed_corpus(&mut state, &corpus).unwrap();

        let score = run_bench_in_state(&mut state, &corpus, 42);

        // All 9 infra checks must pass.
        for c in &score.infrastructure_checks {
            assert!(c.passed, "infra check {} failed: {}", c.name, c.detail);
        }
        // All 6 dims must meet their min.
        for d in &score.dimensions {
            assert!(
                d.pass,
                "dim {} score={} below min={}",
                d.name, d.score, d.min
            );
        }
        assert!(
            score.composite >= 0.95,
            "composite {} below threshold 0.95",
            score.composite
        );
        assert!(score.pass, "overall bench did not pass on seed=42");
    }

    #[test]
    fn d1_inbox_precision_perfect_on_seeded_corpus() {
        let mut state = DaemonState::new(":memory:").unwrap();
        let corpus = generate_corpus(&mut seeded_rng(42));
        seed_corpus(&mut state, &corpus).unwrap();

        let d1 = dim_1_inbox_precision(&mut state, &corpus);
        // Pre-mutation corpus: every message has correct to_session, no foreign rows.
        assert!(
            d1.score >= 0.99,
            "D1 score should be ~1.0 on clean corpus, got {}",
            d1.score
        );
    }

    #[test]
    fn d6_pipeline_chain_is_correct_on_clean_state() {
        let mut state = DaemonState::new(":memory:").unwrap();
        let corpus = generate_corpus(&mut seeded_rng(42));
        seed_corpus(&mut state, &corpus).unwrap();

        let d6 = dim_6_pipeline_chain_correctness(&mut state, &corpus);
        assert!(
            d6.score >= 0.99,
            "D6 should be ~1.0 on green system, got {}",
            d6.score
        );
    }

    #[test]
    fn d5_edge_case_resilience_passes_all_7_probes() {
        let mut state = DaemonState::new(":memory:").unwrap();
        let corpus = generate_corpus(&mut seeded_rng(42));
        seed_corpus(&mut state, &corpus).unwrap();

        let d5 = dim_5_edge_case_resilience(&mut state, &corpus);
        assert!(
            d5.score >= 0.99,
            "D5 score should be ~1.0 (7/7), got {}",
            d5.score
        );
    }
}
