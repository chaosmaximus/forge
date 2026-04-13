// bench/longmemeval.rs — LongMemEval benchmark runner.
//
// Reproduces the methodology from the LongMemEval paper (arxiv 2410.10813):
// for each of 500 questions, build a fresh in-memory corpus from the haystack
// sessions, query with the question text, and score top-K retrieval against
// the ground-truth `answer_session_ids`.
//
// We support four modes (see docs/benchmarks/plan.md §3):
//   - Raw: chunk + embed + KNN (the MemPalace 96.6% recipe).
//   - Extract: TODO (out of scope for the foundations commit).
//   - Consolidate: TODO.
//   - Hybrid: TODO.
//
// Only `raw` is implemented in this commit. Adding the other modes is
// straightforward once we have a runner trait — captured as follow-up work.

use std::collections::HashSet;
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use rusqlite::Connection;
use serde::{Deserialize, Serialize};

use crate::bench::scoring::{ndcg_at_k, recall_all_at_k, recall_any_at_k};
use crate::db::schema::create_schema;
use crate::db::vec::init_sqlite_vec;
use crate::embed::Embedder;
use crate::raw::{ingest_text, search, IngestParams};

// ────────────────────────────────────────────────────────
// Input data model — matches longmemeval_s_cleaned.json
// ────────────────────────────────────────────────────────

/// One LongMemEval question entry. Field names match the on-disk schema
/// exactly so `serde_json::from_reader` works without any rename glue.
#[derive(Debug, Clone, Deserialize)]
pub struct LongMemEvalEntry {
    pub question_id: String,
    pub question_type: String,
    pub question: String,
    #[serde(default)]
    pub question_date: String,
    pub answer: String,
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
pub fn run_raw(
    entry: &LongMemEvalEntry,
    embedder: &Arc<dyn Embedder>,
) -> Result<QuestionResult, BenchError> {
    let started = Instant::now();

    init_sqlite_vec();
    let conn = Connection::open_in_memory()?;
    conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;
    create_schema(&conn)?;

    // One raw_documents row per haystack session. Concatenate user + assistant
    // turns into a single body — MemPalace's bench script joins user turns
    // only, but indexing assistant turns lets us answer the
    // `single-session-assistant` category (which is one of the six types).
    for (sid, session) in entry
        .haystack_session_ids
        .iter()
        .zip(entry.haystack_sessions.iter())
    {
        let body = session
            .iter()
            .map(|t| format!("[{}] {}", t.role, t.content))
            .collect::<Vec<_>>()
            .join("\n");
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
    // same query. Use max_distance: None to disable cutoff — bench scoring
    // wants the full ranked list, not a quality filter.
    let hits = search(
        &conn,
        embedder,
        &entry.question,
        Some("longmemeval"),
        None,
        Some(50),
        Some(2.0),
    )?;

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
            answer: "42".to_string(),
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
        let result = run_raw(&entry, &embedder).expect("run_raw");
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
        let result = run_raw(&entry, &embedder).unwrap();
        assert!(
            result.recall_at_5 == 1.0,
            "expected R@5 = 1.0 on a near-perfect FakeEmbedder match, got {result:?}"
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
}
