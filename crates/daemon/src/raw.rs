// raw.rs — raw layer ingest + search orchestration.
//
// Thin glue layer above the `db::raw` CRUD and the `embed::Embedder` trait.
// Callers (HTTP handlers, bench CLI, background worker) pass a transcript
// body and receive a synchronous result. No LLM is invoked at any point;
// this path is fast, deterministic, and always-on.

use std::sync::Arc;

use rusqlite::Connection;

use crate::chunk_raw::chunk_text_default;
use crate::db::raw::{
    insert_chunk, insert_document, search_chunks, search_chunks_bm25, store_chunk_embedding,
    RawChunk, RawDocument, RawHit,
};
use crate::embed::{EmbedError, Embedder};

/// Default top-K when the caller doesn't specify one. Matches MemPalace's
/// LongMemEval bench default and our publishing plan.
pub const DEFAULT_SEARCH_K: usize = 50;

/// Default cosine-distance cutoff for raw search. Pairs with MemPalace's
/// empirical `max_distance ~= 0.6` threshold on LongMemEval. Disabled when
/// the caller passes `None`.
pub const DEFAULT_MAX_DISTANCE: f64 = 0.6;

/// Error type for the raw ingest + search pipeline.
#[derive(Debug)]
pub enum RawError {
    Db(rusqlite::Error),
    Embed(EmbedError),
}

impl std::fmt::Display for RawError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RawError::Db(e) => write!(f, "raw layer db error: {e}"),
            RawError::Embed(e) => write!(f, "raw layer embed error: {e}"),
        }
    }
}

impl std::error::Error for RawError {}

impl From<rusqlite::Error> for RawError {
    fn from(e: rusqlite::Error) -> Self {
        RawError::Db(e)
    }
}

impl From<EmbedError> for RawError {
    fn from(e: EmbedError) -> Self {
        RawError::Embed(e)
    }
}

/// Report from a successful raw ingest call.
#[derive(Debug, Clone)]
pub struct IngestReport {
    pub document_id: String,
    pub chunk_count: usize,
    pub total_chars: usize,
}

/// Parameters describing one raw-ingest call. Grouping the optional metadata
/// fields keeps `ingest_text` below the clippy 7-argument threshold and
/// documents the natural call shape for callers (handlers, bench CLI, workers).
#[derive(Debug, Clone, Default)]
pub struct IngestParams<'a> {
    pub text: &'a str,
    pub source: &'a str,
    pub project: Option<&'a str>,
    pub session_id: Option<&'a str>,
    pub timestamp: Option<&'a str>,
    pub metadata_json: Option<&'a str>,
}

/// Ingest one body of text into the raw layer.
///
/// Does the full pipeline in a single SQLite transaction:
///   1. Allocate a new document ID.
///   2. Chunk the text (800/100/50 defaults).
///   3. Embed every chunk via the provided embedder.
///   4. Insert the document row, the chunk rows, and the vec0 embeddings.
///
/// If any step fails the transaction rolls back and no state is left in the
/// database. Safe to call repeatedly — IDs are fresh ULIDs per call.
pub fn ingest_text(
    conn: &Connection,
    embedder: &Arc<dyn Embedder>,
    params: IngestParams<'_>,
) -> Result<IngestReport, RawError> {
    let IngestParams {
        text,
        source,
        project,
        session_id,
        timestamp,
        metadata_json,
    } = params;
    // Chunk first so we can bail out early on empty / sub-threshold inputs
    // without paying for the embedding call.
    let chunks = chunk_text_default(text);
    if chunks.is_empty() {
        return Ok(IngestReport {
            document_id: String::new(),
            chunk_count: 0,
            total_chars: text.chars().count(),
        });
    }

    // Embed outside the DB transaction — embedding is the slow part and there
    // is no reason to hold a SQLite write lock across model inference.
    let embeddings = embedder.embed(&chunks)?;
    if embeddings.len() != chunks.len() {
        return Err(RawError::Embed(EmbedError::Inference(format!(
            "embedder returned {} vectors for {} chunks",
            embeddings.len(),
            chunks.len()
        ))));
    }
    let expected_dim = embedder.dim();
    for (idx, v) in embeddings.iter().enumerate() {
        if v.len() != expected_dim {
            return Err(RawError::Embed(EmbedError::DimensionMismatch {
                expected: expected_dim,
                actual: v.len(),
            }));
        }
        // Guard against empty vectors that would silently write as zero bytes.
        if v.is_empty() {
            return Err(RawError::Embed(EmbedError::Inference(format!(
                "embedder returned empty vector for chunk {idx}"
            ))));
        }
    }

    let document_id = ulid::Ulid::new().to_string();
    // Input character count — chunk overlaps would otherwise double-count.
    let total_chars: usize = text.chars().count();
    let ts = timestamp
        .map(String::from)
        .unwrap_or_else(forge_core::time::now_iso);
    let meta = metadata_json.unwrap_or("{}");

    // Single transaction: doc + all chunks + all embeddings.
    let tx = conn.unchecked_transaction()?;

    insert_document(
        &tx,
        &RawDocument {
            id: document_id.clone(),
            project: project.map(String::from),
            session_id: session_id.map(String::from),
            source: source.to_string(),
            text: text.to_string(),
            timestamp: ts,
            metadata_json: meta.to_string(),
        },
    )?;

    for (idx, (chunk_text, embedding)) in chunks.iter().zip(embeddings.iter()).enumerate() {
        let chunk_id = ulid::Ulid::new().to_string();
        insert_chunk(
            &tx,
            &RawChunk {
                id: chunk_id.clone(),
                document_id: document_id.clone(),
                chunk_index: idx,
                text: chunk_text.clone(),
                metadata_json: "{}".to_string(),
            },
        )?;
        store_chunk_embedding(&tx, &chunk_id, embedding)?;
    }

    tx.commit()?;

    Ok(IngestReport {
        document_id,
        chunk_count: chunks.len(),
        total_chars,
    })
}

/// Search the raw layer. Embeds the query, runs KNN, filters by project /
/// session / max-distance. Pass `None` for `max_distance` to use the default.
pub fn search(
    conn: &Connection,
    embedder: &Arc<dyn Embedder>,
    query: &str,
    project: Option<&str>,
    session_id: Option<&str>,
    k: Option<usize>,
    max_distance: Option<f64>,
) -> Result<Vec<RawHit>, RawError> {
    let k = k.unwrap_or(DEFAULT_SEARCH_K);
    let cutoff = max_distance.or(Some(DEFAULT_MAX_DISTANCE));
    let embeddings = embedder.embed(&[query.to_string()])?;
    let Some(query_vec) = embeddings.into_iter().next() else {
        return Ok(Vec::new());
    };
    let hits = search_chunks(conn, &query_vec, project, session_id, k, cutoff)?;
    Ok(hits)
}

/// Hybrid raw search — fuses the KNN leg (`search_chunks`) and the BM25
/// leg (`search_chunks_bm25`) via RRF on chunk IDs, then rebuilds a
/// `Vec<RawHit>` preferring KNN metadata when a chunk appears on both legs.
///
/// Fetches `max(50, 10*k)` candidates from each leg before merging — the
/// larger pool gives downstream feature-engineering waves (bge-large,
/// preference sidecars, LLM rerank) headroom to reorder without losing
/// the relevant chunk. No `max_distance` cutoff is applied on the KNN leg
/// because RRF weighs by rank position, not absolute score — a wider pool
/// with low-quality tail items is harmless since they get low fused score.
///
/// Returns up to `k` chunks sorted by fused RRF score descending.
///
/// # Caveat: mixed distance convention
///
/// The `RawHit.distance` field on returned hits is NOT comparable across
/// hits. A chunk retrieved from the KNN leg carries cosine distance; one
/// retrieved from the BM25 leg carries the SQLite `bm25()` score. This
/// function never reads `distance` — merging happens by rank position
/// only. Downstream code must treat `distance` as opaque when iterating
/// hybrid results. See the `RawHit.distance` doc comment for details.
///
/// # Error behavior
///
/// Fail-fast: if either leg (embed, KNN, or BM25) fails, the error is
/// propagated immediately and no partial results are returned. The KNN
/// work is wasted when BM25 errors after KNN succeeds; this is an
/// acceptable trade-off versus the complexity of partial-leg degradation.
pub fn hybrid_search(
    conn: &Connection,
    embedder: &Arc<dyn Embedder>,
    query: &str,
    project: Option<&str>,
    session_id: Option<&str>,
    k: Option<usize>,
) -> Result<Vec<RawHit>, RawError> {
    // Short-circuit for queries that would resolve to zero tokens after
    // FTS5 sanitization (empty string, whitespace, all-punctuation). The
    // BM25 leg would return empty anyway, and running the KNN leg on a
    // degenerate query risks an embedder error on pathological inputs.
    // Return an empty Vec to match the contract of `search_chunks_bm25`.
    if crate::db::ops::sanitize_fts5_query(query).is_empty() {
        return Ok(Vec::new());
    }

    let final_k = k.unwrap_or(DEFAULT_SEARCH_K);
    if final_k == 0 {
        tracing::warn!("hybrid_search called with k=0; returning empty result");
        return Ok(Vec::new());
    }
    let pool_k = std::cmp::max(50, final_k * 10);

    // KNN leg — embed the query once, run vec0 MATCH with no distance
    // cutoff so RRF sees the full pool.
    let embeddings = embedder.embed(&[query.to_string()])?;
    let knn_hits: Vec<RawHit> = if let Some(query_vec) = embeddings.into_iter().next() {
        search_chunks(conn, &query_vec, project, session_id, pool_k, None)?
    } else {
        Vec::new()
    };

    // BM25 leg — sanitize + FTS5 MATCH. Returns empty when the sanitized
    // query has no surviving tokens (all-punctuation input).
    let bm25_hits: Vec<RawHit> = search_chunks_bm25(conn, query, project, session_id, pool_k)?;

    // Collect chunk IDs in ranked order (rank 0 = best per leg) for RRF.
    // Pre-sized to match the pool; avoids a reallocation on large pools.
    let mut knn_ids: Vec<String> = Vec::with_capacity(knn_hits.len());
    knn_ids.extend(knn_hits.iter().map(|h| h.chunk_id.clone()));
    let mut bm25_ids: Vec<String> = Vec::with_capacity(bm25_hits.len());
    bm25_ids.extend(bm25_hits.iter().map(|h| h.chunk_id.clone()));

    // Pure RRF merge, k=60 convention, capped at `final_k` items.
    let merged_ids = rrf_merge_raw(&[knn_ids, bm25_ids], 60.0, final_k);

    // Rebuild `Vec<RawHit>` in fused order. KNN metadata takes precedence
    // when a chunk appears on both legs — we insert BM25 first, then let
    // KNN overwrite. This ensures `hit.distance` reflects cosine when
    // available, and only falls back to BM25 score when a chunk is
    // BM25-only.
    let mut hit_map: std::collections::HashMap<String, RawHit> =
        std::collections::HashMap::with_capacity(knn_hits.len() + bm25_hits.len());
    for h in bm25_hits {
        hit_map.insert(h.chunk_id.clone(), h);
    }
    for h in knn_hits {
        hit_map.insert(h.chunk_id.clone(), h);
    }

    Ok(merged_ids
        .into_iter()
        .filter_map(|id| hit_map.remove(&id))
        .collect())
}

/// Pure Reciprocal Rank Fusion for the raw layer. Merges already-ranked
/// chunk-ID lists (from KNN and BM25 legs) by rank position only — no
/// score blending, because cosine distance and SQLite `bm25()` scores
/// aren't comparable. Each input list is assumed sorted with rank 0 = best.
///
///   score(id) = Σ 1 / (k + rank_in_list_i(id) + 1)
///
/// Returns up to `limit` distinct chunk IDs sorted by fused score descending.
fn rrf_merge_raw(ranked_lists: &[Vec<String>], k: f64, limit: usize) -> Vec<String> {
    let mut scores: std::collections::HashMap<String, f64> = std::collections::HashMap::new();
    for list in ranked_lists {
        for (rank, id) in list.iter().enumerate() {
            *scores.entry(id.clone()).or_insert(0.0) += 1.0 / (k + rank as f64 + 1.0);
        }
    }
    let mut sorted: Vec<(String, f64)> = scores.into_iter().collect();
    sorted.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    sorted.into_iter().take(limit).map(|(id, _)| id).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{schema::create_schema, vec::init_sqlite_vec};
    use crate::embed::FakeEmbedder;

    fn setup() -> (Connection, Arc<dyn Embedder>) {
        init_sqlite_vec();
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")
            .unwrap();
        create_schema(&conn).unwrap();
        let embedder: Arc<dyn Embedder> = Arc::new(FakeEmbedder::new(384));
        (conn, embedder)
    }

    #[test]
    fn ingest_empty_text_returns_no_chunks() {
        let (conn, embedder) = setup();
        let report = ingest_text(
            &conn,
            &embedder,
            IngestParams {
                text: "",
                source: "claude-code",
                project: Some("p1"),
                session_id: Some("s1"),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(report.chunk_count, 0);
        assert!(report.document_id.is_empty());
    }

    #[test]
    fn ingest_short_text_produces_single_chunk() {
        let (conn, embedder) = setup();
        let text = "a".repeat(200);
        let report = ingest_text(
            &conn,
            &embedder,
            IngestParams {
                text: &text,
                source: "claude-code",
                project: Some("p1"),
                session_id: Some("s1"),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(report.chunk_count, 1);
        assert!(!report.document_id.is_empty());
        assert_eq!(report.total_chars, 200);
    }

    #[test]
    fn ingest_long_text_produces_multiple_chunks() {
        let (conn, embedder) = setup();
        let text = "hello world ".repeat(200); // ~2400 chars
        let report = ingest_text(
            &conn,
            &embedder,
            IngestParams {
                text: &text,
                source: "claude-code",
                project: Some("p1"),
                session_id: Some("s1"),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(report.chunk_count >= 3);
    }

    #[test]
    fn search_returns_chunks_from_same_document() {
        let (conn, embedder) = setup();
        // Two distinct documents, one in project p1 and one in p2.
        let rust_text = "rust is fast ".repeat(100);
        ingest_text(
            &conn,
            &embedder,
            IngestParams {
                text: &rust_text,
                source: "claude-code",
                project: Some("p1"),
                session_id: Some("s_rust"),
                ..Default::default()
            },
        )
        .unwrap();
        let panda_text = "pandas are cute ".repeat(100);
        ingest_text(
            &conn,
            &embedder,
            IngestParams {
                text: &panda_text,
                source: "claude-code",
                project: Some("p2"),
                session_id: Some("s_panda"),
                ..Default::default()
            },
        )
        .unwrap();

        // Query with the same text as doc 1 — fake embedder produces identical
        // vectors for identical inputs, so the closest hit must be from p1.
        let hits = search(
            &conn,
            &embedder,
            &"rust is fast ".repeat(100),
            None,
            None,
            Some(10),
            Some(2.0), // disable cutoff to get everything
        )
        .unwrap();
        assert!(!hits.is_empty(), "expected at least one hit");
        // Top hit must come from p1 (the project whose text we echoed).
        assert_eq!(hits[0].project.as_deref(), Some("p1"));
    }

    #[test]
    fn search_filters_by_project() {
        let (conn, embedder) = setup();
        let body = "x".repeat(300);
        ingest_text(
            &conn,
            &embedder,
            IngestParams {
                text: &body,
                source: "claude-code",
                project: Some("p1"),
                session_id: Some("s1"),
                ..Default::default()
            },
        )
        .unwrap();
        ingest_text(
            &conn,
            &embedder,
            IngestParams {
                text: &body,
                source: "claude-code",
                project: Some("p2"),
                session_id: Some("s2"),
                ..Default::default()
            },
        )
        .unwrap();

        let hits = search(
            &conn,
            &embedder,
            &"x".repeat(300),
            Some("p1"),
            None,
            Some(10),
            Some(2.0),
        )
        .unwrap();
        for h in &hits {
            assert_eq!(h.project.as_deref(), Some("p1"));
        }
    }

    #[test]
    fn ingest_is_transactional() {
        let (conn, embedder) = setup();
        // Successful ingest → rows visible.
        let body = "x".repeat(300);
        let report = ingest_text(
            &conn,
            &embedder,
            IngestParams {
                text: &body,
                source: "test",
                ..Default::default()
            },
        )
        .unwrap();
        let doc_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM raw_documents", [], |r| r.get(0))
            .unwrap();
        assert_eq!(doc_count, 1);
        let chunk_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM raw_chunks", [], |r| r.get(0))
            .unwrap();
        assert_eq!(chunk_count, report.chunk_count as i64);
    }

    // ──────────────────────────────────────────────────────────
    // rrf_merge_raw — pure Reciprocal Rank Fusion for the raw layer.
    //
    // Merges two already-ranked chunk-ID lists (from KNN and BM25) into
    // one fused ranking. No score blending; rank position only, because
    // cosine distance and SQLite bm25() scores aren't comparable.
    // Rank 0 = best in every input list.
    // ──────────────────────────────────────────────────────────

    #[test]
    fn rrf_merge_raw_single_list_preserves_order() {
        // One list in, same order out — nothing to fuse.
        let list = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        let merged = rrf_merge_raw(&[list], 60.0, 10);
        let ids: Vec<&str> = merged.iter().map(|s| s.as_str()).collect();
        assert_eq!(ids, vec!["a", "b", "c"]);
    }

    #[test]
    fn rrf_merge_raw_empty_lists_returns_empty() {
        // Empty slice of lists: no panic, returns empty Vec.
        let merged = rrf_merge_raw(&[], 60.0, 10);
        assert!(
            merged.is_empty(),
            "empty input slice must produce empty result"
        );

        // Slice of empty lists: same.
        let merged2 = rrf_merge_raw(&[vec![], vec![]], 60.0, 10);
        assert!(
            merged2.is_empty(),
            "slice of empty lists must produce empty result"
        );
    }

    #[test]
    fn rrf_merge_raw_two_lists_reranks_by_overlap() {
        // "b" appears at rank 1 in both lists. Its fused score is the
        // sum of two 1/(60+2) contributions ≈ 0.0323. "a" and "c" each
        // appear once at rank 0 with score 1/(60+1) ≈ 0.0164. So "b"
        // must rank first after fusion even though it was second in
        // both individual lists.
        let list_knn = vec!["a".to_string(), "b".to_string()];
        let list_bm25 = vec!["c".to_string(), "b".to_string()];
        let merged = rrf_merge_raw(&[list_knn, list_bm25], 60.0, 10);
        let ids: Vec<&str> = merged.iter().map(|s| s.as_str()).collect();
        assert_eq!(
            ids[0], "b",
            "overlapping item must rank first after RRF, got {ids:?}"
        );
        assert_eq!(ids.len(), 3);
    }

    // ──────────────────────────────────────────────────────────
    // hybrid_search — KNN + BM25 fused via RRF.
    //
    // End-to-end tests verify the plumbing (both legs invoked, filter
    // pass-through, edge cases). With FakeEmbedder we cannot construct
    // a test that strictly REQUIRES BM25 over pure KNN — real-data
    // verification happens in the day-3 smoke bench.
    // ──────────────────────────────────────────────────────────

    #[test]
    fn hybrid_search_finds_matching_chunk() {
        let (conn, embedder) = setup();
        ingest_text(
            &conn,
            &embedder,
            IngestParams {
                text: &"rust programming is fast ".repeat(100),
                source: "test",
                project: Some("p1"),
                session_id: Some("s1"),
                ..Default::default()
            },
        )
        .unwrap();
        ingest_text(
            &conn,
            &embedder,
            IngestParams {
                text: &"pandas are cute ".repeat(100),
                source: "test",
                project: Some("p1"),
                session_id: Some("s2"),
                ..Default::default()
            },
        )
        .unwrap();

        let hits = hybrid_search(
            &conn,
            &embedder,
            &"rust programming is fast ".repeat(100),
            Some("p1"),
            None,
            Some(5),
        )
        .unwrap();

        assert!(!hits.is_empty(), "expected at least one hybrid hit");
        assert!(
            hits[0].text.contains("rust"),
            "top hit must contain 'rust', got {:?}",
            hits[0].text
        );
    }

    #[test]
    fn hybrid_search_preserves_project_filter() {
        // Two projects with matching chunks. Filter to p1 — no p2 hits allowed.
        let (conn, embedder) = setup();
        ingest_text(
            &conn,
            &embedder,
            IngestParams {
                text: &"rust programming ".repeat(100),
                source: "test",
                project: Some("p1"),
                ..Default::default()
            },
        )
        .unwrap();
        ingest_text(
            &conn,
            &embedder,
            IngestParams {
                text: &"rust programming ".repeat(100),
                source: "test",
                project: Some("p2"),
                ..Default::default()
            },
        )
        .unwrap();

        let hits = hybrid_search(
            &conn,
            &embedder,
            "rust programming",
            Some("p1"),
            None,
            Some(50),
        )
        .unwrap();
        assert!(!hits.is_empty(), "expected p1 hits");
        for h in &hits {
            assert_eq!(
                h.project.as_deref(),
                Some("p1"),
                "hybrid leaked a p2 chunk: {h:?}"
            );
        }
    }

    #[test]
    fn hybrid_search_handles_empty_query() {
        // Empty / all-punctuation queries sanitize to empty for BM25
        // and produce a (harmless) vector for KNN. Must return `Ok`
        // without panicking regardless of what KNN picks up.
        let (conn, embedder) = setup();
        ingest_text(
            &conn,
            &embedder,
            IngestParams {
                text: &"x".repeat(300),
                source: "test",
                ..Default::default()
            },
        )
        .unwrap();

        assert!(hybrid_search(&conn, &embedder, "", None, None, Some(5)).is_ok());
        assert!(hybrid_search(&conn, &embedder, "!!!", None, None, Some(5)).is_ok());
    }

    #[test]
    fn hybrid_search_respects_k_limit() {
        // 30 chunks, all matching the query → must cap at k=5 after merge.
        let (conn, embedder) = setup();
        for i in 0..30 {
            let sid = format!("s{i}");
            ingest_text(
                &conn,
                &embedder,
                IngestParams {
                    text: &format!("rust programming {i} ").repeat(50),
                    source: "test",
                    project: Some("p1"),
                    session_id: Some(&sid),
                    ..Default::default()
                },
            )
            .unwrap();
        }

        let hits = hybrid_search(
            &conn,
            &embedder,
            "rust programming",
            Some("p1"),
            None,
            Some(5),
        )
        .unwrap();
        assert!(
            hits.len() <= 5,
            "k=5 must cap result count, got {} hits",
            hits.len()
        );
    }

    #[test]
    fn hybrid_search_zero_k_returns_empty() {
        // k=0 is semantically degenerate but shouldn't crash or silently
        // do work. Contract: return Ok(vec![]) and log a warning. The
        // adversarial review flagged this as a latent footgun.
        let (conn, embedder) = setup();
        ingest_text(
            &conn,
            &embedder,
            IngestParams {
                text: &"rust programming ".repeat(100),
                source: "test",
                project: Some("p1"),
                ..Default::default()
            },
        )
        .unwrap();

        let hits = hybrid_search(&conn, &embedder, "rust", Some("p1"), None, Some(0)).unwrap();
        assert!(hits.is_empty(), "k=0 must return empty Vec, got {hits:?}");
    }

    #[test]
    fn hybrid_search_prefers_knn_distance_when_chunk_on_both_legs() {
        // A chunk that both KNN and BM25 surface: the returned
        // `hit.distance` must be the KNN cosine distance (non-negative,
        // in [0, 2]), NOT the BM25 score (negative). Locks in the
        // metadata-precedence design decision so a future refactor
        // can't silently flip it.
        let (conn, embedder) = setup();
        ingest_text(
            &conn,
            &embedder,
            IngestParams {
                text: &"rust programming is fast ".repeat(100),
                source: "test",
                project: Some("p1"),
                ..Default::default()
            },
        )
        .unwrap();

        let hits = hybrid_search(
            &conn,
            &embedder,
            &"rust programming is fast ".repeat(100),
            Some("p1"),
            None,
            Some(5),
        )
        .unwrap();
        assert!(!hits.is_empty());
        assert!(
            hits[0].distance >= 0.0,
            "top hit's distance must be KNN cosine (>=0), got {} (looks like a negative BM25 score)",
            hits[0].distance
        );
    }

    #[test]
    fn rrf_merge_raw_input_rank_zero_is_best() {
        // Guarantee: callers MUST pass lists sorted so rank 0 = best.
        // This test catches a sort-direction inversion in rrf_merge_raw
        // itself: if the final sort were accidentally ascending, the
        // lowest-scoring (worst) items would rank at the top.
        //
        // Adversarial review added this test explicitly — without it,
        // a single-character typo in the sort comparator would silently
        // destroy recall and pass every other cycle.
        //
        // Setup: two disjoint lists of 3 items each, winner at rank 0.
        // After merge, the two winners must sit in the top 2 (either
        // order, since both have the same fused score).
        let list_knn = vec![
            "win_knn".to_string(),
            "lose_knn".to_string(),
            "trash_knn".to_string(),
        ];
        let list_bm25 = vec![
            "win_bm25".to_string(),
            "lose_bm25".to_string(),
            "trash_bm25".to_string(),
        ];
        let merged = rrf_merge_raw(&[list_knn, list_bm25], 60.0, 10);
        let ids: Vec<&str> = merged.iter().map(|s| s.as_str()).collect();

        let top_two: std::collections::HashSet<&str> = ids.iter().take(2).copied().collect();
        let expected: std::collections::HashSet<&str> =
            ["win_knn", "win_bm25"].iter().copied().collect();
        assert_eq!(
            top_two, expected,
            "rank-0 items from each list must end up in the top 2 of the merge, got {ids:?}"
        );
    }
}
