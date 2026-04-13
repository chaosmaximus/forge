// bench/locomo.rs — LoCoMo benchmark runner.
//
// LoCoMo (Snap Research, ACL 2024 — arxiv 2402.17753) tests memory across
// very long conversations. Each sample has 19–32 sessions of dialogue
// between two speakers, plus a list of QA pairs. We score retrieval
// recall@K at session granularity per QA.
//
// Methodology mirrors MemPalace's locomo_bench.py to keep cross-system
// comparisons honest: corpus is one document per session containing every
// turn (both speakers), evidence dia_ids `D<N>:M` map to session N, and the
// reported metric is mean recall@K across all QAs.

use std::collections::HashSet;
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use rusqlite::Connection;
use serde::Serialize;

use crate::bench::scoring::{ndcg_at_k, recall_any_at_k};
use crate::db::schema::create_schema;
use crate::db::vec::init_sqlite_vec;
use crate::embed::Embedder;
use crate::raw::{ingest_text, search, IngestParams};

// ────────────────────────────────────────────────────────
// Input data model
// ────────────────────────────────────────────────────────
//
// The on-disk format uses sibling keys `session_1`, `session_2`, ... rather
// than an array, so we go through `serde_json::Value` to extract sessions
// before building typed structs.

#[derive(Debug, Clone)]
pub struct LocomoSample {
    pub sample_id: String,
    pub speaker_a: String,
    pub speaker_b: String,
    pub sessions: Vec<LocomoSession>,
    pub qa: Vec<LocomoQa>,
}

#[derive(Debug, Clone)]
pub struct LocomoSession {
    /// 1-based index from the source schema (`session_1`, `session_2`, ...).
    pub idx: usize,
    pub turns: Vec<LocomoTurn>,
}

#[derive(Debug, Clone)]
pub struct LocomoTurn {
    pub speaker: String,
    pub text: String,
}

#[derive(Debug, Clone)]
pub struct LocomoQa {
    pub question: String,
    /// dia_ids of the evidence turns, e.g. `["D1:3", "D5:7"]`.
    pub evidence: Vec<String>,
    /// Integer category 1–5 (single-hop / temporal / temporal-inference /
    /// open-domain / adversarial — see the LoCoMo paper).
    pub category: i32,
}

/// Load `locomo10.json` (top-level array of samples).
pub fn load_samples(path: &Path) -> Result<Vec<LocomoSample>, BenchError> {
    let file = std::fs::File::open(path)
        .map_err(|e| BenchError::Io(format!("open {}: {e}", path.display())))?;
    let reader = std::io::BufReader::new(file);
    let raw: serde_json::Value = serde_json::from_reader(reader)
        .map_err(|e| BenchError::Parse(format!("parse {}: {e}", path.display())))?;
    let arr = raw
        .as_array()
        .ok_or_else(|| BenchError::Parse("top-level is not a JSON array".to_string()))?;

    let mut samples = Vec::with_capacity(arr.len());
    for (i, sample_val) in arr.iter().enumerate() {
        samples.push(
            parse_sample(sample_val).map_err(|e| BenchError::Parse(format!("sample[{i}]: {e}")))?,
        );
    }
    Ok(samples)
}

fn parse_sample(value: &serde_json::Value) -> Result<LocomoSample, String> {
    let sample_id = value
        .get("sample_id")
        .and_then(|v| v.as_str())
        .ok_or("missing sample_id")?
        .to_string();

    let conv = value.get("conversation").ok_or("missing conversation")?;
    let conv_obj = conv.as_object().ok_or("conversation is not an object")?;

    let speaker_a = conv_obj
        .get("speaker_a")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let speaker_b = conv_obj
        .get("speaker_b")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    // Walk session_1, session_2, ... until the next index is missing.
    let mut sessions = Vec::new();
    let mut idx = 1usize;
    loop {
        let key = format!("session_{idx}");
        let Some(turns_val) = conv_obj.get(&key) else {
            break;
        };
        let turns_arr = turns_val
            .as_array()
            .ok_or_else(|| format!("{key} is not an array"))?;
        let mut turns = Vec::with_capacity(turns_arr.len());
        for turn_val in turns_arr {
            let speaker = turn_val
                .get("speaker")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let text = turn_val
                .get("text")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            if text.is_empty() {
                continue;
            }
            turns.push(LocomoTurn { speaker, text });
        }
        sessions.push(LocomoSession { idx, turns });
        idx += 1;
    }

    let mut qa = Vec::new();
    if let Some(qa_arr) = value.get("qa").and_then(|v| v.as_array()) {
        for qa_val in qa_arr {
            let question = qa_val
                .get("question")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            if question.is_empty() {
                continue;
            }
            let evidence = qa_val
                .get("evidence")
                .and_then(|v| v.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|e| e.as_str().map(String::from))
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            let category = qa_val.get("category").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            qa.push(LocomoQa {
                question,
                evidence,
                category,
            });
        }
    }

    Ok(LocomoSample {
        sample_id,
        speaker_a,
        speaker_b,
        sessions,
        qa,
    })
}

// ────────────────────────────────────────────────────────
// Output model
// ────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct QaResult {
    pub sample_id: String,
    pub question: String,
    pub category: i32,
    pub mode: String,
    pub n_sessions: usize,
    pub retrieved_session_ids: Vec<String>,
    pub evidence_session_ids: Vec<String>,
    pub recall_at_5: f64,
    pub recall_at_10: f64,
    pub ndcg_at_10: f64,
    pub elapsed_ms: u128,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct LocomoSummary {
    pub mode: String,
    pub n_questions: usize,
    pub mean_recall_at_5: f64,
    pub mean_recall_at_10: f64,
    pub mean_ndcg_at_10: f64,
    pub total_elapsed_ms: u128,
    /// Per-category mean R@10. LoCoMo uses integer categories 1..=5.
    pub by_category_recall_at_10: std::collections::BTreeMap<i32, f64>,
}

#[derive(Debug)]
pub enum BenchError {
    Io(String),
    Parse(String),
    Db(rusqlite::Error),
    Raw(crate::raw::RawError),
}

impl std::fmt::Display for BenchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(s) => write!(f, "io: {s}"),
            Self::Parse(s) => write!(f, "parse: {s}"),
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

/// Map a LoCoMo dia_id of the form `D<N>:<M>` to a session_id of the form
/// `session_<N>`. Returns `None` for malformed inputs.
fn dia_id_to_session(dia_id: &str) -> Option<String> {
    let stripped = dia_id.strip_prefix('D')?;
    let n = stripped.split(':').next()?;
    n.parse::<usize>().ok().map(|n| format!("session_{n}"))
}

/// Run one LoCoMo sample through raw mode: build one in-memory corpus from
/// all sessions, then run every QA against it. Per-QA metrics are returned;
/// the runner amortizes the corpus build across all QAs in the sample.
pub fn run_sample_raw(
    sample: &LocomoSample,
    embedder: &Arc<dyn Embedder>,
) -> Result<Vec<QaResult>, BenchError> {
    init_sqlite_vec();
    let conn = Connection::open_in_memory()?;
    conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;
    create_schema(&conn)?;

    // Ingest each session. Body = every turn joined with newlines, prefixed
    // by speaker (LoCoMo conversations have two speakers; the prefix preserves
    // the speaker signal that the questions reference). MemPalace's reference
    // does the same for LoCoMo (different from LongMemEval, where it uses
    // user-only — LoCoMo has no notion of "user vs assistant").
    for session in &sample.sessions {
        let body = session
            .turns
            .iter()
            .map(|t| format!("{}: {}", t.speaker, t.text))
            .collect::<Vec<_>>()
            .join("\n");
        if body.is_empty() {
            continue;
        }
        let session_id = format!("session_{}", session.idx);
        ingest_text(
            &conn,
            embedder,
            IngestParams {
                text: &body,
                source: "locomo",
                project: Some("locomo"),
                session_id: Some(&session_id),
                ..Default::default()
            },
        )?;
    }

    // Run each QA against the corpus.
    let mut results = Vec::with_capacity(sample.qa.len());
    for qa in &sample.qa {
        let started = Instant::now();
        let evidence_session_ids: Vec<String> = qa
            .evidence
            .iter()
            .filter_map(|d| dia_id_to_session(d))
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();

        let hits = search(
            &conn,
            embedder,
            &qa.question,
            Some("locomo"),
            None,
            Some(50),
            Some(2.0),
        )?;

        // Dedupe to session granularity.
        let mut seen: HashSet<String> = HashSet::new();
        let mut retrieved_session_ids: Vec<String> = Vec::with_capacity(hits.len());
        for h in &hits {
            if let Some(sid) = &h.session_id {
                if seen.insert(sid.clone()) {
                    retrieved_session_ids.push(sid.clone());
                }
            }
        }

        let r5 = recall_any_at_k(&retrieved_session_ids, &evidence_session_ids, 5);
        let r10 = recall_any_at_k(&retrieved_session_ids, &evidence_session_ids, 10);
        let ndcg10 = ndcg_at_k(&retrieved_session_ids, &evidence_session_ids, 10);

        results.push(QaResult {
            sample_id: sample.sample_id.clone(),
            question: qa.question.clone(),
            category: qa.category,
            mode: "raw".to_string(),
            n_sessions: sample.sessions.len(),
            retrieved_session_ids,
            evidence_session_ids,
            recall_at_5: r5,
            recall_at_10: r10,
            ndcg_at_10: ndcg10,
            elapsed_ms: started.elapsed().as_millis(),
        });
    }
    Ok(results)
}

/// Aggregate per-question results into a `LocomoSummary`.
pub fn summarize(results: &[QaResult]) -> LocomoSummary {
    let n = results.len();
    if n == 0 {
        return LocomoSummary {
            mode: "raw".to_string(),
            ..Default::default()
        };
    }
    let total_elapsed_ms: u128 = results.iter().map(|r| r.elapsed_ms).sum();
    let mean_r5: f64 = results.iter().map(|r| r.recall_at_5).sum::<f64>() / n as f64;
    let mean_r10: f64 = results.iter().map(|r| r.recall_at_10).sum::<f64>() / n as f64;
    let mean_ndcg10: f64 = results.iter().map(|r| r.ndcg_at_10).sum::<f64>() / n as f64;

    let mut by_cat: std::collections::BTreeMap<i32, (f64, usize)> = Default::default();
    for r in results {
        let entry = by_cat.entry(r.category).or_insert((0.0, 0));
        entry.0 += r.recall_at_10;
        entry.1 += 1;
    }
    let by_category_recall_at_10 = by_cat
        .into_iter()
        .map(|(k, (sum, count))| (k, sum / count as f64))
        .collect();

    LocomoSummary {
        mode: "raw".to_string(),
        n_questions: n,
        mean_recall_at_5: mean_r5,
        mean_recall_at_10: mean_r10,
        mean_ndcg_at_10: mean_ndcg10,
        total_elapsed_ms,
        by_category_recall_at_10,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::embed::FakeEmbedder;

    fn fake_embedder() -> Arc<dyn Embedder> {
        Arc::new(FakeEmbedder::new(384))
    }

    #[test]
    fn dia_id_to_session_parses_well_formed_ids() {
        assert_eq!(dia_id_to_session("D1:3"), Some("session_1".to_string()));
        assert_eq!(dia_id_to_session("D12:5"), Some("session_12".to_string()));
        assert_eq!(dia_id_to_session("D0:0"), Some("session_0".to_string()));
    }

    #[test]
    fn dia_id_to_session_rejects_malformed_ids() {
        assert!(dia_id_to_session("").is_none());
        assert!(dia_id_to_session("X1:3").is_none());
        assert!(dia_id_to_session("Da:b").is_none());
    }

    #[test]
    fn parse_sample_extracts_sessions_in_order() {
        let json = serde_json::json!({
            "sample_id": "conv-test",
            "conversation": {
                "speaker_a": "Alice",
                "speaker_b": "Bob",
                "session_1_date_time": "1pm",
                "session_1": [
                    {"speaker": "Alice", "dia_id": "D1:1", "text": "hi bob"},
                    {"speaker": "Bob", "dia_id": "D1:2", "text": "hi alice"}
                ],
                "session_2_date_time": "2pm",
                "session_2": [
                    {"speaker": "Alice", "dia_id": "D2:1", "text": "weather is nice"}
                ]
            },
            "qa": [
                {"question": "How is the weather?", "answer": "nice", "evidence": ["D2:1"], "category": 1}
            ]
        });
        let sample = parse_sample(&json).unwrap();
        assert_eq!(sample.sample_id, "conv-test");
        assert_eq!(sample.speaker_a, "Alice");
        assert_eq!(sample.sessions.len(), 2);
        assert_eq!(sample.sessions[0].idx, 1);
        assert_eq!(sample.sessions[0].turns.len(), 2);
        assert_eq!(sample.sessions[1].idx, 2);
        assert_eq!(sample.qa.len(), 1);
        assert_eq!(sample.qa[0].evidence, vec!["D2:1"]);
        assert_eq!(sample.qa[0].category, 1);
    }

    #[test]
    fn run_sample_raw_finds_evidence_session() {
        let embedder = fake_embedder();
        let sample = LocomoSample {
            sample_id: "test".to_string(),
            speaker_a: "Alice".to_string(),
            speaker_b: "Bob".to_string(),
            sessions: vec![
                LocomoSession {
                    idx: 1,
                    turns: vec![LocomoTurn {
                        speaker: "Alice".into(),
                        text: "weather is great today".repeat(20),
                    }],
                },
                LocomoSession {
                    idx: 2,
                    turns: vec![LocomoTurn {
                        speaker: "Bob".into(),
                        text: "I love going to the beach in summer".repeat(20),
                    }],
                },
            ],
            qa: vec![LocomoQa {
                question: "weather is great today".repeat(20),
                evidence: vec!["D1:1".to_string()],
                category: 1,
            }],
        };
        let results = run_sample_raw(&sample, &embedder).unwrap();
        assert_eq!(results.len(), 1);
        let r = &results[0];
        assert_eq!(r.evidence_session_ids, vec!["session_1"]);
        assert!(r.recall_at_5 == 1.0, "expected R@5 = 1.0, got {r:?}");
    }

    #[test]
    fn summarize_groups_by_category() {
        let results = vec![
            QaResult {
                sample_id: "s".into(),
                question: "q1".into(),
                category: 1,
                mode: "raw".into(),
                n_sessions: 0,
                retrieved_session_ids: vec![],
                evidence_session_ids: vec![],
                recall_at_5: 1.0,
                recall_at_10: 1.0,
                ndcg_at_10: 1.0,
                elapsed_ms: 10,
            },
            QaResult {
                sample_id: "s".into(),
                question: "q2".into(),
                category: 2,
                mode: "raw".into(),
                n_sessions: 0,
                retrieved_session_ids: vec![],
                evidence_session_ids: vec![],
                recall_at_5: 0.0,
                recall_at_10: 0.0,
                ndcg_at_10: 0.0,
                elapsed_ms: 20,
            },
        ];
        let summary = summarize(&results);
        assert_eq!(summary.n_questions, 2);
        assert!((summary.mean_recall_at_5 - 0.5).abs() < 1e-6);
        assert_eq!(summary.by_category_recall_at_10.get(&1).copied(), Some(1.0));
        assert_eq!(summary.by_category_recall_at_10.get(&2).copied(), Some(0.0));
    }
}
