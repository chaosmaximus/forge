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
    insert_chunk, insert_document, search_chunks, store_chunk_embedding, RawChunk, RawDocument,
    RawHit,
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
}
