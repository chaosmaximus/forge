// bench/longmemeval.rs — LongMemEval benchmark runner.
//
// Reproduces the methodology from the LongMemEval paper (arxiv 2410.10813):
// for each of 500 questions, build a fresh in-memory corpus from the haystack
// sessions, query with the question text, and score top-K retrieval against
// the ground-truth `answer_session_ids`.
//
// We support four modes (see docs/benchmarks/plan.md §3):
//   - Raw:         chunk + embed + KNN (the MemPalace 96.6% recipe). LLM-free.
//   - Extract:     run the Forge LLM extraction pipeline (Claude Haiku via the
//                  local `claude` CLI), store memories with session_id, query
//                  via BM25.
//   - Consolidate: TODO — extract + run consolidation phases before query.
//   - Hybrid:      TODO — RRF-merge raw chunks + extracted memories.
//
// Raw + Extract are implemented in this commit. Consolidate + Hybrid are the
// next targets; both reuse the run_extract corpus build.

use std::collections::HashSet;
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use futures_util::stream::{self, StreamExt};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

use crate::bench::scoring::{ndcg_at_k, recall_all_at_k, recall_any_at_k};
use crate::config::ConsolidationConfig;
use crate::db::ops;
use crate::db::schema::create_schema;
use crate::db::vec::init_sqlite_vec;
use crate::embed::Embedder;
use crate::extraction::backend::ExtractionResult;
use crate::extraction::gemini;
use crate::raw::{hybrid_search, ingest_text, search, IngestParams};
use crate::workers::consolidator;
use forge_core::types::{Memory, MemoryType};

// ────────────────────────────────────────────────────────
// Input data model — matches longmemeval_s_cleaned.json
// ────────────────────────────────────────────────────────

/// One LongMemEval question entry. Field names match the on-disk schema
/// exactly so `serde_json::from_reader` works without any rename glue.
///
/// `answer` is `serde_json::Value` because the upstream data is mixed: most
/// answers are strings, but "how many" questions store integers (e.g.
/// `"answer": 3`). The retrieval scoring path doesn't read `answer` at all —
/// only `answer_session_ids` matters — so we just pass it through.
#[derive(Debug, Clone, Deserialize)]
pub struct LongMemEvalEntry {
    pub question_id: String,
    pub question_type: String,
    pub question: String,
    #[serde(default)]
    pub question_date: String,
    #[serde(default)]
    pub answer: serde_json::Value,
    /// Ground-truth session IDs (plural — note the trailing `s`).
    pub answer_session_ids: Vec<String>,
    #[serde(default)]
    pub haystack_dates: Vec<String>,
    pub haystack_session_ids: Vec<String>,
    pub haystack_sessions: Vec<Vec<Turn>>,
}

/// One conversation turn inside a haystack session.
#[derive(Debug, Clone, Deserialize)]
pub struct Turn {
    pub role: String,
    pub content: String,
}

/// Load `longmemeval_s_cleaned.json` (top-level array of entries).
pub fn load_entries(path: &Path) -> Result<Vec<LongMemEvalEntry>, BenchError> {
    let file = std::fs::File::open(path)
        .map_err(|e| BenchError::Io(format!("open {}: {e}", path.display())))?;
    let reader = std::io::BufReader::new(file);
    let entries: Vec<LongMemEvalEntry> = serde_json::from_reader(reader)
        .map_err(|e| BenchError::Parse(format!("parse {}: {e}", path.display())))?;
    Ok(entries)
}

// ────────────────────────────────────────────────────────
// Output data model — JSONL records + summary
// ────────────────────────────────────────────────────────

/// One row of the per-question JSONL output. Matches the shape of MemPalace's
/// per-question records so downstream comparison tooling can ingest both.
#[derive(Debug, Clone, Serialize)]
pub struct QuestionResult {
    pub question_id: String,
    pub question_type: String,
    pub mode: String,
    pub n_haystack_sessions: usize,
    pub retrieved_session_ids: Vec<String>,
    pub answer_session_ids: Vec<String>,
    pub recall_at_5: f64,
    pub recall_at_10: f64,
    pub recall_all_at_10: f64,
    pub ndcg_at_10: f64,
    pub elapsed_ms: u128,
}

/// Aggregated summary across an entire benchmark run.
#[derive(Debug, Clone, Serialize, Default)]
pub struct RunSummary {
    pub mode: String,
    pub n_questions: usize,
    pub mean_recall_at_5: f64,
    pub mean_recall_at_10: f64,
    pub mean_recall_all_at_10: f64,
    pub mean_ndcg_at_10: f64,
    pub total_elapsed_ms: u128,
    /// Per-question-type breakdown (mean R@5).
    pub by_type_recall_at_5: std::collections::BTreeMap<String, f64>,
}

/// All four bench modes the harness understands. Only `Raw` is wired in this
/// commit; the others return a `BenchError::Unsupported`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BenchMode {
    Raw,
    Extract,
    Consolidate,
    Hybrid,
}

impl BenchMode {
    pub fn parse(s: &str) -> Result<Self, BenchError> {
        match s {
            "raw" => Ok(Self::Raw),
            "extract" => Ok(Self::Extract),
            "consolidate" => Ok(Self::Consolidate),
            "hybrid" => Ok(Self::Hybrid),
            other => Err(BenchError::Parse(format!("unknown mode: {other}"))),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Raw => "raw",
            Self::Extract => "extract",
            Self::Consolidate => "consolidate",
            Self::Hybrid => "hybrid",
        }
    }
}

/// Dispatch enum for the raw-search path inside `BenchMode::Raw`.
///
/// - `Knn` → pure cosine-KNN via `raw::search` (the published 0.9520 baseline).
/// - `Hybrid` → BM25 + KNN fused via RRF through `raw::hybrid_search` (wave 1).
///
/// Default for new bench runs is `Hybrid`. Pure KNN stays reachable behind
/// the `--raw-mode knn` CLI flag so the parity number against MemPalace can
/// still be reproduced.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RawStrategy {
    Knn,
    Hybrid,
}

impl RawStrategy {
    pub fn parse(s: &str) -> Result<Self, &'static str> {
        match s {
            "knn" => Ok(Self::Knn),
            "hybrid" => Ok(Self::Hybrid),
            _ => Err("invalid raw strategy (expected 'knn' or 'hybrid')"),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Knn => "knn",
            Self::Hybrid => "hybrid",
        }
    }
}

#[derive(Debug)]
pub enum BenchError {
    Io(String),
    Parse(String),
    Unsupported(String),
    Db(rusqlite::Error),
    Raw(crate::raw::RawError),
}

impl std::fmt::Display for BenchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(s) => write!(f, "io: {s}"),
            Self::Parse(s) => write!(f, "parse: {s}"),
            Self::Unsupported(s) => write!(f, "unsupported: {s}"),
            Self::Db(e) => write!(f, "db: {e}"),
            Self::Raw(e) => write!(f, "raw layer: {e}"),
        }
    }
}

impl std::error::Error for BenchError {}

impl From<rusqlite::Error> for BenchError {
    fn from(e: rusqlite::Error) -> Self {
        Self::Db(e)
    }
}

impl From<crate::raw::RawError> for BenchError {
    fn from(e: crate::raw::RawError) -> Self {
        Self::Raw(e)
    }
}

// ────────────────────────────────────────────────────────
// Runner
// ────────────────────────────────────────────────────────

/// Run one entry through the raw mode pipeline.
///
/// Allocates a fresh in-memory SQLite per call, ingests every haystack session
/// as a `raw_documents` row, queries with the question, and computes the four
/// retrieval metrics against `answer_session_ids`.
///
/// The `strategy` parameter selects between pure KNN (the published 0.9520
/// baseline) and hybrid BM25+KNN via RRF. Dispatched inside the function
/// rather than at the caller so the corpus-build path stays shared across
/// both strategies — they only differ at the retrieval step.
pub fn run_raw(
    entry: &LongMemEvalEntry,
    embedder: &Arc<dyn Embedder>,
    strategy: RawStrategy,
) -> Result<QuestionResult, BenchError> {
    let started = Instant::now();

    init_sqlite_vec();
    let conn = Connection::open_in_memory()?;
    conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;
    create_schema(&conn)?;

    // One raw_documents row per haystack session. Match MemPalace's reference
    // bench exactly so the comparison to their 96.6% is apples-to-apples:
    // index ONLY user turns, joined with a single newline, no role prefix.
    // (See longmemeval_bench.py lines 188–192.)
    //
    // Sessions whose haystack has zero user turns are skipped — same
    // behaviour as the reference. This affects ~0 sessions in practice but
    // keeps the corpus-build logic identical.
    //
    // Variant ideas (all out of scope for the baseline run):
    //   - Index assistant turns too → may improve `single-session-assistant`
    //   - Add role markers → may improve `single-session-preference`
    //   - Swap embedder to bge-large → known to add ~3 points
    // Each of these deserves its own published row.
    for (sid, session) in entry
        .haystack_session_ids
        .iter()
        .zip(entry.haystack_sessions.iter())
    {
        let user_turns: Vec<&str> = session
            .iter()
            .filter(|t| t.role == "user")
            .map(|t| t.content.as_str())
            .collect();
        if user_turns.is_empty() {
            continue;
        }
        let body = user_turns.join("\n");
        ingest_text(
            &conn,
            embedder,
            IngestParams {
                text: &body,
                source: "longmemeval",
                project: Some("longmemeval"),
                session_id: Some(sid),
                ..Default::default()
            },
        )?;
    }

    // Top-50 results so we can compute Recall@5, @10, and NDCG@10 from the
    // same query. Dispatch on strategy:
    //   - Knn → raw::search with cutoff disabled (Some(2.0)), matches
    //     the exact published 0.9520 baseline.
    //   - Hybrid → raw::hybrid_search, which uses pool = max(50, 10*k)
    //     internally and has no `max_distance` parameter.
    let hits = match strategy {
        RawStrategy::Knn => search(
            &conn,
            embedder,
            &entry.question,
            Some("longmemeval"),
            None,
            Some(50),
            Some(2.0),
        )?,
        RawStrategy::Hybrid => hybrid_search(
            &conn,
            embedder,
            &entry.question,
            Some("longmemeval"),
            None,
            Some(50),
        )?,
    };

    // De-duplicate retrieved session IDs while preserving order — multiple
    // chunks from the same session count as one hit at session granularity.
    let mut seen: HashSet<String> = HashSet::new();
    let mut retrieved_session_ids: Vec<String> = Vec::with_capacity(hits.len());
    for h in &hits {
        if let Some(sid) = &h.session_id {
            if seen.insert(sid.clone()) {
                retrieved_session_ids.push(sid.clone());
            }
        }
    }

    let r5 = recall_any_at_k(&retrieved_session_ids, &entry.answer_session_ids, 5);
    let r10 = recall_any_at_k(&retrieved_session_ids, &entry.answer_session_ids, 10);
    let r_all_10 = recall_all_at_k(&retrieved_session_ids, &entry.answer_session_ids, 10);
    let ndcg10 = ndcg_at_k(&retrieved_session_ids, &entry.answer_session_ids, 10);

    Ok(QuestionResult {
        question_id: entry.question_id.clone(),
        question_type: entry.question_type.clone(),
        mode: "raw".to_string(),
        n_haystack_sessions: entry.haystack_sessions.len(),
        retrieved_session_ids,
        answer_session_ids: entry.answer_session_ids.clone(),
        recall_at_5: r5,
        recall_at_10: r10,
        recall_all_at_10: r_all_10,
        ndcg_at_10: ndcg10,
        elapsed_ms: started.elapsed().as_millis(),
    })
}

/// Run one entry through the extract mode pipeline.
///
/// For each haystack session: invoke the Forge LLM extraction backend
/// (`extraction::gemini::extract`) with the configured model — defaults to
/// `"gemini-2.0-flash"`, which matches the production Forge default. The
/// bench uses Gemini rather than the local `claude` CLI for three reasons:
///
///   1. Every `claude -p` subprocess leaks sync-cli child processes on the
///      user's system — unacceptable at bench scale.
///   2. Gemini Flash is faster (~2–4 s/call vs ~16 s for the CLI), so even
///      a 20-question run stays under 10 minutes.
///   3. `GEMINI_API_KEY` is typically present in the Forge developer
///      environment already (the project ships with a Gemini default); the
///      Anthropic API key is not.
///
/// Extracted memories are stored in a fresh in-memory `memory` table with
/// `session_id` set to the source haystack session ID. Retrieval uses BM25
/// over `memory_fts` (no vector search, since `memory_vec` is 768-dim and
/// would require a separate Ollama embedder that we don't want to make the
/// bench depend on).
///
/// Concurrency: extractions for a single question's haystack run in parallel
/// with bounded `concurrency` (default 8 — see `forge-bench`'s
/// `--extract-concurrency` flag). Wall time is roughly
/// `(haystack_size / concurrency) * per_call_seconds`.
pub async fn run_extract(
    entry: &LongMemEvalEntry,
    api_key: &str,
    model: &str,
    concurrency: usize,
) -> Result<QuestionResult, BenchError> {
    let started = Instant::now();
    let conn = open_bench_conn()?;
    build_extract_corpus(&conn, entry, api_key, model, concurrency).await?;
    let retrieved_session_ids = bm25_query_session_ids(&conn, &entry.question)?;
    Ok(score_question(
        entry,
        "extract",
        retrieved_session_ids,
        started,
    ))
}

/// Run one entry through the `extract + consolidate` mode pipeline.
///
/// Identical to `run_extract` but runs `consolidator::run_all_phases` after
/// the memories are stored and before the BM25 query. The consolidator
/// performs exact dedup, semantic dedup, linking, decay, episodic→semantic
/// promotion, and reconsolidation — all the operations Forge runs
/// periodically in production. The bench question is: does running
/// consolidation on extracted memories recover any retrieval recall that
/// extraction alone lost?
pub async fn run_consolidate(
    entry: &LongMemEvalEntry,
    api_key: &str,
    model: &str,
    concurrency: usize,
) -> Result<QuestionResult, BenchError> {
    let started = Instant::now();
    let conn = open_bench_conn()?;
    build_extract_corpus(&conn, entry, api_key, model, concurrency).await?;
    consolidator::run_all_phases(&conn, &ConsolidationConfig::default(), None, None);
    let retrieved_session_ids = bm25_query_session_ids(&conn, &entry.question)?;
    Ok(score_question(
        entry,
        "consolidate",
        retrieved_session_ids,
        started,
    ))
}

/// Run one entry through the `hybrid` mode pipeline.
///
/// Populates BOTH the raw layer (chunks + embeddings in `raw_chunks_vec`)
/// AND the extraction memory table from the same haystack sessions, then
/// queries both paths and RRF-merges the two ranked session-id lists.
/// Scores the merged list against ground truth.
///
/// This is the headline number in the benchmark plan: "does Forge's
/// extraction add value on top of raw storage for retrieval tasks?" If
/// hybrid > raw, extraction helps. If hybrid ≈ raw, extraction is
/// architectural weight that must be justified by non-retrieval axes
/// (tools, identity, behavioral learning).
pub async fn run_hybrid(
    entry: &LongMemEvalEntry,
    embedder: &Arc<dyn Embedder>,
    api_key: &str,
    model: &str,
    concurrency: usize,
) -> Result<QuestionResult, BenchError> {
    let started = Instant::now();
    let conn = open_bench_conn()?;

    // Build raw corpus.
    build_raw_corpus(&conn, embedder, entry)?;

    // Build extract corpus (same `memory` table as extract mode, with
    // session_id set). Runs in parallel over haystack sessions via the
    // same helper.
    build_extract_corpus(&conn, entry, api_key, model, concurrency).await?;

    // Query both paths independently, then RRF-merge.
    let raw_hits = search(
        &conn,
        embedder,
        &entry.question,
        Some("longmemeval"),
        None,
        Some(50),
        Some(2.0),
    )?;
    let raw_session_ids: Vec<String> = {
        let mut seen: HashSet<String> = HashSet::new();
        raw_hits
            .into_iter()
            .filter_map(|h| h.session_id)
            .filter(|sid| seen.insert(sid.clone()))
            .collect()
    };

    let extract_session_ids = bm25_query_session_ids(&conn, &entry.question)?;

    let retrieved_session_ids = rrf_merge(&[&raw_session_ids, &extract_session_ids], 60);

    Ok(score_question(
        entry,
        "hybrid",
        retrieved_session_ids,
        started,
    ))
}

// ────────────────────────────────────────────────────────
// Shared helpers
// ────────────────────────────────────────────────────────

/// Open a fresh in-memory SQLite with the Forge schema installed and
/// sqlite-vec ready for both the raw layer's 384-dim vec table and the
/// extraction layer's BM25 index.
fn open_bench_conn() -> Result<Connection, BenchError> {
    init_sqlite_vec();
    let conn = Connection::open_in_memory()?;
    conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;
    create_schema(&conn)?;
    Ok(conn)
}

/// Ingest every haystack session into the raw layer. Matches `run_raw`'s
/// corpus-build exactly (user turns only, joined with `\n`).
fn build_raw_corpus(
    conn: &Connection,
    embedder: &Arc<dyn Embedder>,
    entry: &LongMemEvalEntry,
) -> Result<(), BenchError> {
    for (sid, session) in entry
        .haystack_session_ids
        .iter()
        .zip(entry.haystack_sessions.iter())
    {
        let user_turns: Vec<&str> = session
            .iter()
            .filter(|t| t.role == "user")
            .map(|t| t.content.as_str())
            .collect();
        if user_turns.is_empty() {
            continue;
        }
        let body = user_turns.join("\n");
        ingest_text(
            conn,
            embedder,
            IngestParams {
                text: &body,
                source: "longmemeval",
                project: Some("longmemeval"),
                session_id: Some(sid),
                ..Default::default()
            },
        )?;
    }
    Ok(())
}

/// Ingest every haystack session through the Forge LLM extraction pipeline
/// (Gemini HTTP API, bounded concurrency), storing the produced memories in
/// the `memory` table with `session_id` set. Shared by extract / consolidate
/// / hybrid modes.
async fn build_extract_corpus(
    conn: &Connection,
    entry: &LongMemEvalEntry,
    api_key: &str,
    model: &str,
    concurrency: usize,
) -> Result<(), BenchError> {
    let session_bodies: Vec<(String, String)> = entry
        .haystack_session_ids
        .iter()
        .zip(entry.haystack_sessions.iter())
        .filter_map(|(sid, session)| {
            let user_turns: Vec<&str> = session
                .iter()
                .filter(|t| t.role == "user")
                .map(|t| t.content.as_str())
                .collect();
            if user_turns.is_empty() {
                None
            } else {
                Some((sid.clone(), user_turns.join("\n")))
            }
        })
        .collect();

    // Parallel extraction with bounded concurrency via Gemini's HTTP API.
    // ~2–4 s per call dominated by network + model inference. No subprocess
    // spawning, no child-process leaks.
    let model_owned = model.to_string();
    let api_key_owned = api_key.to_string();
    let extraction_results: Vec<(String, ExtractionResult)> =
        stream::iter(session_bodies.into_iter().map(|(sid, body)| {
            let model_for_call = model_owned.clone();
            let key_for_call = api_key_owned.clone();
            async move {
                let result = gemini::extract(&key_for_call, &model_for_call, &body).await;
                (sid, result)
            }
        }))
        .buffer_unordered(concurrency.max(1))
        .collect()
        .await;

    let mut total_extracted = 0usize;
    let mut sample_error: Option<String> = None;
    for (sid, result) in extraction_results {
        match result {
            ExtractionResult::Success(memories) if !memories.is_empty() => {
                for em in memories {
                    if !em.is_valid_type() {
                        continue;
                    }
                    let Some(memory_type) = parse_memory_type(&em.memory_type) else {
                        continue;
                    };
                    let mut memory = Memory::new(memory_type, &em.title, &em.content)
                        .with_confidence(em.confidence)
                        .with_tags(em.tags.clone())
                        .with_project("longmemeval");
                    memory.session_id = sid.clone();
                    if !em.valence.is_empty() {
                        memory.valence = em.valence.clone();
                    }
                    memory.intensity = em.intensity;
                    memory.alternatives = em.alternatives.clone();
                    memory.participants = em.participants.clone();
                    ops::remember(conn, &memory)?;
                    total_extracted += 1;
                }
            }
            ExtractionResult::Success(_) => {}
            ExtractionResult::Error(msg) | ExtractionResult::Unavailable(msg) => {
                if sample_error.is_none() {
                    sample_error = Some(msg);
                }
            }
        }
    }

    if total_extracted == 0 && sample_error.is_some() {
        eprintln!(
            "[bench][{}] WARN: 0 memories stored — sample error: {}",
            entry.question_id,
            sample_error
                .as_deref()
                .unwrap_or("")
                .chars()
                .take(160)
                .collect::<String>(),
        );
    }
    Ok(())
}

/// Run a BM25 query against the `memory` table and return deduped session IDs
/// in BM25-rank order. Used by extract / consolidate / hybrid modes.
fn bm25_query_session_ids(conn: &Connection, query: &str) -> Result<Vec<String>, BenchError> {
    let safe_query = ops::sanitize_fts5_query(query);
    if safe_query.is_empty() {
        return Ok(Vec::new());
    }
    let sql = "SELECT m.session_id, bm25(memory_fts) AS score
               FROM memory_fts
               JOIN memory m ON memory_fts.rowid = m.rowid
               WHERE memory_fts MATCH ?1
                 AND m.status = 'active'
               ORDER BY score
               LIMIT 50";
    let mut stmt = conn.prepare(sql)?;
    let rows: Vec<String> = stmt
        .query_map(params![safe_query], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut seen: HashSet<String> = HashSet::new();
    Ok(rows
        .into_iter()
        .filter(|sid| !sid.is_empty() && seen.insert(sid.clone()))
        .collect())
}

/// Reciprocal Rank Fusion — merges multiple ranked lists into one.
/// `score(doc) = Σ 1 / (k + rank_in_list_i(doc))` where k = 60 by convention.
/// Ties broken by insertion order. Returns a deduped ranked list.
fn rrf_merge(lists: &[&[String]], k: usize) -> Vec<String> {
    let k = k as f64;
    let mut scores: std::collections::HashMap<String, f64> = std::collections::HashMap::new();
    for list in lists {
        for (rank, sid) in list.iter().enumerate() {
            *scores.entry(sid.clone()).or_insert(0.0) += 1.0 / (k + rank as f64 + 1.0);
        }
    }
    let mut sorted: Vec<(String, f64)> = scores.into_iter().collect();
    sorted.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    sorted.into_iter().map(|(sid, _)| sid).collect()
}

/// Common scoring path — computes R@5, R@10, R_all@10, NDCG@10 from a
/// retrieved-session-id list and wraps everything in a `QuestionResult`.
fn score_question(
    entry: &LongMemEvalEntry,
    mode: &str,
    retrieved_session_ids: Vec<String>,
    started: Instant,
) -> QuestionResult {
    let r5 = recall_any_at_k(&retrieved_session_ids, &entry.answer_session_ids, 5);
    let r10 = recall_any_at_k(&retrieved_session_ids, &entry.answer_session_ids, 10);
    let r_all_10 = recall_all_at_k(&retrieved_session_ids, &entry.answer_session_ids, 10);
    let ndcg10 = ndcg_at_k(&retrieved_session_ids, &entry.answer_session_ids, 10);

    QuestionResult {
        question_id: entry.question_id.clone(),
        question_type: entry.question_type.clone(),
        mode: mode.to_string(),
        n_haystack_sessions: entry.haystack_sessions.len(),
        retrieved_session_ids,
        answer_session_ids: entry.answer_session_ids.clone(),
        recall_at_5: r5,
        recall_at_10: r10,
        recall_all_at_10: r_all_10,
        ndcg_at_10: ndcg10,
        elapsed_ms: started.elapsed().as_millis(),
    }
}

/// Map an `ExtractedMemory.memory_type` string to a typed `MemoryType` enum.
/// Returns `None` for skill / identity (which extract mode does not yet
/// surface in the bench retrieval scorer — we treat them as out-of-scope).
fn parse_memory_type(s: &str) -> Option<MemoryType> {
    match s {
        "decision" => Some(MemoryType::Decision),
        "lesson" => Some(MemoryType::Lesson),
        "pattern" => Some(MemoryType::Pattern),
        "preference" => Some(MemoryType::Preference),
        // Skills and identity facets aren't first-class memories in the
        // simplified Memory enum — drop them here. A future commit can add
        // dedicated tables for both.
        "skill" | "identity" | "protocol" => None,
        _ => None,
    }
}

/// Aggregate per-question results into a `RunSummary`.
pub fn summarize(results: &[QuestionResult], mode: BenchMode) -> RunSummary {
    let n = results.len();
    if n == 0 {
        return RunSummary {
            mode: mode.as_str().to_string(),
            ..Default::default()
        };
    }
    let total_elapsed_ms: u128 = results.iter().map(|r| r.elapsed_ms).sum();
    let mean_r5: f64 = results.iter().map(|r| r.recall_at_5).sum::<f64>() / n as f64;
    let mean_r10: f64 = results.iter().map(|r| r.recall_at_10).sum::<f64>() / n as f64;
    let mean_r_all_10: f64 = results.iter().map(|r| r.recall_all_at_10).sum::<f64>() / n as f64;
    let mean_ndcg10: f64 = results.iter().map(|r| r.ndcg_at_10).sum::<f64>() / n as f64;

    let mut by_type: std::collections::BTreeMap<String, (f64, usize)> =
        std::collections::BTreeMap::new();
    for r in results {
        let entry = by_type.entry(r.question_type.clone()).or_insert((0.0, 0));
        entry.0 += r.recall_at_5;
        entry.1 += 1;
    }
    let by_type_recall_at_5 = by_type
        .into_iter()
        .map(|(k, (sum, count))| (k, sum / count as f64))
        .collect();

    RunSummary {
        mode: mode.as_str().to_string(),
        n_questions: n,
        mean_recall_at_5: mean_r5,
        mean_recall_at_10: mean_r10,
        mean_recall_all_at_10: mean_r_all_10,
        mean_ndcg_at_10: mean_ndcg10,
        total_elapsed_ms,
        by_type_recall_at_5,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::embed::FakeEmbedder;

    fn fake_embedder() -> Arc<dyn Embedder> {
        Arc::new(FakeEmbedder::new(384))
    }

    fn entry_with(answer_sids: &[&str]) -> LongMemEvalEntry {
        LongMemEvalEntry {
            question_id: "q1".to_string(),
            question_type: "single-session-user".to_string(),
            question: "What is the answer?".to_string(),
            question_date: "2026-04-13".to_string(),
            answer: serde_json::json!("42"),
            answer_session_ids: answer_sids.iter().map(|s| s.to_string()).collect(),
            haystack_dates: vec!["2026-04-13".to_string(); 3],
            haystack_session_ids: vec![
                "sess-a".to_string(),
                "sess-b".to_string(),
                "sess-c".to_string(),
            ],
            haystack_sessions: vec![
                vec![Turn {
                    role: "user".to_string(),
                    content: "What is the answer? The answer is 42 obviously.".repeat(20),
                }],
                vec![Turn {
                    role: "user".to_string(),
                    content: "completely unrelated chitchat about the weather ".repeat(20),
                }],
                vec![Turn {
                    role: "user".to_string(),
                    content: "more unrelated content about cooking recipes ".repeat(20),
                }],
            ],
        }
    }

    #[test]
    fn parse_bench_mode() {
        assert_eq!(BenchMode::parse("raw").unwrap(), BenchMode::Raw);
        assert_eq!(BenchMode::parse("extract").unwrap(), BenchMode::Extract);
        assert_eq!(
            BenchMode::parse("consolidate").unwrap(),
            BenchMode::Consolidate
        );
        assert_eq!(BenchMode::parse("hybrid").unwrap(), BenchMode::Hybrid);
        assert!(BenchMode::parse("nope").is_err());
    }

    #[test]
    fn run_raw_returns_result_for_each_question() {
        let embedder = fake_embedder();
        let entry = entry_with(&["sess-a"]);
        let result = run_raw(&entry, &embedder, RawStrategy::Knn).expect("run_raw");
        assert_eq!(result.question_id, "q1");
        assert_eq!(result.question_type, "single-session-user");
        assert_eq!(result.mode, "raw");
        assert_eq!(result.n_haystack_sessions, 3);
        assert!(!result.retrieved_session_ids.is_empty());
        assert_eq!(result.answer_session_ids, vec!["sess-a"]);
    }

    #[test]
    fn run_raw_finds_self_match_with_fake_embedder() {
        // The first haystack session contains the literal question text,
        // hashed identically by the FakeEmbedder. It must rank in the top 5.
        let embedder = fake_embedder();
        let entry = entry_with(&["sess-a"]);
        let result = run_raw(&entry, &embedder, RawStrategy::Knn).unwrap();
        assert!(
            result.recall_at_5 == 1.0,
            "expected R@5 = 1.0 on a near-perfect FakeEmbedder match, got {result:?}"
        );
    }

    #[test]
    fn run_raw_finds_self_match_with_hybrid_strategy() {
        // Same fixture, but via the new hybrid dispatch. Both legs see the
        // self-match so hybrid's fused ranking must also land it in top 5.
        // This is the day-3 TDD test that drives the run_raw signature
        // change to take a RawStrategy parameter.
        let embedder = fake_embedder();
        let entry = entry_with(&["sess-a"]);
        let result = run_raw(&entry, &embedder, RawStrategy::Hybrid).unwrap();
        assert!(
            result.recall_at_5 == 1.0,
            "expected R@5 = 1.0 on hybrid dispatch with FakeEmbedder, got {result:?}"
        );
    }

    #[test]
    fn summarize_averages_metrics() {
        let results = vec![
            QuestionResult {
                question_id: "q1".to_string(),
                question_type: "type-a".to_string(),
                mode: "raw".to_string(),
                n_haystack_sessions: 3,
                retrieved_session_ids: vec![],
                answer_session_ids: vec![],
                recall_at_5: 1.0,
                recall_at_10: 1.0,
                recall_all_at_10: 1.0,
                ndcg_at_10: 0.9,
                elapsed_ms: 100,
            },
            QuestionResult {
                question_id: "q2".to_string(),
                question_type: "type-b".to_string(),
                mode: "raw".to_string(),
                n_haystack_sessions: 3,
                retrieved_session_ids: vec![],
                answer_session_ids: vec![],
                recall_at_5: 0.0,
                recall_at_10: 1.0,
                recall_all_at_10: 0.0,
                ndcg_at_10: 0.5,
                elapsed_ms: 200,
            },
        ];
        let summary = summarize(&results, BenchMode::Raw);
        assert_eq!(summary.n_questions, 2);
        assert!((summary.mean_recall_at_5 - 0.5).abs() < 1e-6);
        assert!((summary.mean_recall_at_10 - 1.0).abs() < 1e-6);
        assert!((summary.mean_ndcg_at_10 - 0.7).abs() < 1e-6);
        assert_eq!(summary.total_elapsed_ms, 300);
        assert_eq!(
            summary.by_type_recall_at_5.get("type-a").copied(),
            Some(1.0)
        );
        assert_eq!(
            summary.by_type_recall_at_5.get("type-b").copied(),
            Some(0.0)
        );
    }

    #[test]
    fn load_entries_parses_minimal_fixture() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("fixture.json");
        let body = serde_json::json!([
            {
                "question_id": "qfix",
                "question_type": "single-session-user",
                "question": "test?",
                "answer": "yes",
                "answer_session_ids": ["s1"],
                "haystack_session_ids": ["s1", "s2"],
                "haystack_sessions": [
                    [{"role": "user", "content": "yes"}],
                    [{"role": "user", "content": "no"}]
                ]
            }
        ]);
        std::fs::write(&path, body.to_string()).unwrap();
        let entries = load_entries(&path).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].question_id, "qfix");
        assert_eq!(entries[0].haystack_sessions.len(), 2);
    }

    // ──────────────────────────────────────────────────────────
    // RawStrategy — dispatch enum for the bench raw-search path.
    //
    // Wave 1 T4: selects between pure KNN (the published 0.9520 baseline)
    // and hybrid BM25+KNN (the new default). The CLI flag --raw-mode
    // accepts "knn" or "hybrid" and parses into this enum.
    // ──────────────────────────────────────────────────────────

    #[test]
    fn raw_strategy_parses_knn() {
        assert_eq!(RawStrategy::parse("knn").unwrap(), RawStrategy::Knn);
    }

    #[test]
    fn raw_strategy_parses_hybrid() {
        assert_eq!(RawStrategy::parse("hybrid").unwrap(), RawStrategy::Hybrid);
    }

    #[test]
    fn raw_strategy_rejects_bad_input() {
        assert!(RawStrategy::parse("").is_err());
        assert!(RawStrategy::parse("KNN").is_err(), "case-sensitive");
        assert!(RawStrategy::parse("hybrid ").is_err(), "no trimming");
        assert!(RawStrategy::parse("garbage").is_err());
    }

    #[test]
    fn raw_strategy_as_str_roundtrip() {
        // Every variant must roundtrip through parse ∘ as_str.
        for strat in [RawStrategy::Knn, RawStrategy::Hybrid] {
            let s = strat.as_str();
            assert_eq!(RawStrategy::parse(s).unwrap(), strat);
        }
    }
}
