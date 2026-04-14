// db/raw.rs — raw verbatim storage layer
//
// Stores full session text as chunks with 384-dim embeddings in raw_chunks_vec.
// Sits alongside the existing extraction pipeline — both ingest paths fire on
// the same transcript. Raw is LLM-free and exists for benchmark parity with
// published retrieval systems (see docs/benchmarks/plan.md).

use rusqlite::{params, Connection};
use zerocopy::AsBytes;

use crate::db::ops;

/// Embedding dimension for the raw layer. Matches `all-MiniLM-L6-v2` (fastembed).
/// This is a compile-time constant — do NOT change without a schema migration.
pub const RAW_EMBEDDING_DIM: usize = 384;

/// A raw document as persisted in `raw_documents`.
#[derive(Debug, Clone)]
pub struct RawDocument {
    pub id: String,
    pub project: Option<String>,
    pub session_id: Option<String>,
    pub source: String,
    pub text: String,
    pub timestamp: String,
    pub metadata_json: String,
}

/// A raw chunk row (without the embedding, which lives in `raw_chunks_vec`).
#[derive(Debug, Clone)]
pub struct RawChunk {
    pub id: String,
    pub document_id: String,
    pub chunk_index: usize,
    pub text: String,
    pub metadata_json: String,
}

/// A hit returned by `search_chunks` or `search_chunks_bm25` — joins a chunk
/// row with its parent document.
///
/// # Distance field convention (MIXED — read this)
///
/// `distance` is "lower is better" in both search paths, but the numeric
/// ranges are different:
///
/// - `search_chunks` (KNN) sets `distance` to the cosine distance from
///   sqlite-vec — a non-negative value in `[0, 2]` where 0 is identical.
/// - `search_chunks_bm25` sets `distance` to SQLite's `bm25(raw_chunks_fts)`
///   score — **negative**, where more negative means more relevant.
///
/// Downstream code MUST NOT compare `distance` across hits from different
/// search paths. The hybrid path in `raw::hybrid_search` merges by RRF rank
/// position only; it never reads `distance` for any merge decision.
#[derive(Debug, Clone)]
pub struct RawHit {
    pub chunk_id: String,
    pub document_id: String,
    pub chunk_index: usize,
    pub text: String,
    pub project: Option<String>,
    pub session_id: Option<String>,
    pub source: String,
    pub timestamp: String,
    pub distance: f64,
}

/// Insert a raw document row. Caller supplies the ID (ULID recommended).
pub fn insert_document(conn: &Connection, doc: &RawDocument) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO raw_documents (id, project, session_id, source, text, timestamp, metadata_json)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            doc.id,
            doc.project,
            doc.session_id,
            doc.source,
            doc.text,
            doc.timestamp,
            doc.metadata_json,
        ],
    )?;
    Ok(())
}

/// Insert a raw chunk row (no embedding yet — use `store_chunk_embedding` after).
pub fn insert_chunk(conn: &Connection, chunk: &RawChunk) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO raw_chunks (id, document_id, chunk_index, text, metadata_json)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            chunk.id,
            chunk.document_id,
            chunk.chunk_index as i64,
            chunk.text,
            chunk.metadata_json,
        ],
    )?;
    Ok(())
}

/// Store a chunk embedding in `raw_chunks_vec`.
/// Idempotent: deletes any existing row for this ID first (vec0 does not support REPLACE).
/// Validates embedding dimension is `RAW_EMBEDDING_DIM` (384).
///
/// This function does NOT open its own transaction — it issues two statements
/// directly so it is safe to call from inside a caller-managed transaction
/// (e.g. `raw::ingest_text`). Standalone callers should wrap in a transaction
/// themselves if they need DELETE+INSERT atomicity.
pub fn store_chunk_embedding(
    conn: &Connection,
    chunk_id: &str,
    embedding: &[f32],
) -> rusqlite::Result<()> {
    if embedding.len() != RAW_EMBEDDING_DIM {
        return Err(rusqlite::Error::InvalidParameterName(format!(
            "raw chunk embedding must be {RAW_EMBEDDING_DIM}-dim, got {}",
            embedding.len()
        )));
    }
    conn.execute(
        "DELETE FROM raw_chunks_vec WHERE id = ?1",
        params![chunk_id],
    )?;
    conn.execute(
        "INSERT INTO raw_chunks_vec(id, embedding) VALUES (?1, ?2)",
        params![chunk_id, embedding.as_bytes()],
    )?;
    Ok(())
}

/// KNN search on raw chunks.
///
/// Optionally filters by `project` and/or `session_id` (applied after the vec0 KNN
/// so the JOIN doesn't interfere with the MATCH constraint). `max_distance` is a
/// post-filter — rows with cosine distance > max are dropped. Pass `None` to disable.
pub fn search_chunks(
    conn: &Connection,
    query_embedding: &[f32],
    project: Option<&str>,
    session_id: Option<&str>,
    k: usize,
    max_distance: Option<f64>,
) -> rusqlite::Result<Vec<RawHit>> {
    if query_embedding.len() != RAW_EMBEDDING_DIM {
        return Err(rusqlite::Error::InvalidParameterName(format!(
            "raw query embedding must be {RAW_EMBEDDING_DIM}-dim, got {}",
            query_embedding.len()
        )));
    }

    // vec0 KNN requires the MATCH constraint to be the sole filter on the vec table;
    // we pull top-k raw hits first, then join/filter in application code.
    let mut stmt = conn.prepare(
        "SELECT v.id, v.distance, c.document_id, c.chunk_index, c.text,
                d.project, d.session_id, d.source, d.timestamp
         FROM raw_chunks_vec v
         JOIN raw_chunks c ON c.id = v.id
         JOIN raw_documents d ON d.id = c.document_id
         WHERE v.embedding MATCH ?1 AND k = ?2",
    )?;

    let rows = stmt
        .query_map(params![query_embedding.as_bytes(), k as i64], |row| {
            Ok(RawHit {
                chunk_id: row.get(0)?,
                distance: row.get(1)?,
                document_id: row.get(2)?,
                chunk_index: row.get::<_, i64>(3)? as usize,
                text: row.get(4)?,
                project: row.get(5)?,
                session_id: row.get(6)?,
                source: row.get(7)?,
                timestamp: row.get(8)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    let hits = rows
        .into_iter()
        .filter(|hit| {
            if let Some(p) = project {
                if hit.project.as_deref() != Some(p) {
                    return false;
                }
            }
            if let Some(s) = session_id {
                if hit.session_id.as_deref() != Some(s) {
                    return false;
                }
            }
            if let Some(max) = max_distance {
                if hit.distance > max {
                    return false;
                }
            }
            true
        })
        .collect();

    Ok(hits)
}

/// Full-text BM25 search over raw chunks via the `raw_chunks_fts` virtual table.
///
/// Sibling to `search_chunks` (KNN). The FTS5 contentless table is populated
/// automatically on every chunk insert by triggers declared in `schema.rs`, so
/// this function just runs a MATCH query and joins back to `raw_chunks` /
/// `raw_documents` to build the same `RawHit` shape as KNN.
///
/// The `distance` field on each hit carries the SQLite `bm25(raw_chunks_fts)`
/// score, which is **negative** and **lower-is-better** (SQLite convention —
/// unlike cosine distance). Callers that combine this with KNN hits must
/// merge by rank, not by raw score.
///
/// Query is sanitized via `ops::sanitize_fts5_query` before being passed to
/// FTS5 so user input cannot inject operators. Returns `Ok(vec![])` if the
/// sanitized query is empty (all punctuation / stopwords dropped).
pub fn search_chunks_bm25(
    conn: &Connection,
    query: &str,
    project: Option<&str>,
    session_id: Option<&str>,
    k: usize,
) -> rusqlite::Result<Vec<RawHit>> {
    let safe_query = ops::sanitize_fts5_query(query);
    if safe_query.is_empty() {
        return Ok(Vec::new());
    }

    // Unlike vec0 (whose MATCH constraint must be the sole filter on the vec
    // table), FTS5 JOINs accept additional WHERE clauses. Pushing the
    // project/session filters into SQL avoids the top-K truncation bug: if
    // we instead post-filtered in Rust, a query whose top-K BM25 hits were
    // all in the wrong project would drop every hit and return zero results
    // even when matching chunks exist in the target project. The
    // `(?N IS NULL OR col = ?N)` pattern is a no-op when the caller passes
    // `None` and a strict equality match otherwise; SQL NULL semantics
    // correctly exclude rows whose column is itself NULL when a concrete
    // filter is supplied.
    let mut stmt = conn.prepare(
        "SELECT c.id, bm25(raw_chunks_fts) AS score, c.document_id, c.chunk_index, c.text,
                d.project, d.session_id, d.source, d.timestamp
         FROM raw_chunks_fts
         JOIN raw_chunks c ON c.rowid = raw_chunks_fts.rowid
         JOIN raw_documents d ON d.id = c.document_id
         WHERE raw_chunks_fts MATCH ?1
           AND (?3 IS NULL OR d.project = ?3)
           AND (?4 IS NULL OR d.session_id = ?4)
         ORDER BY score
         LIMIT ?2",
    )?;

    let rows = stmt
        .query_map(params![safe_query, k as i64, project, session_id], |row| {
            Ok(RawHit {
                chunk_id: row.get(0)?,
                distance: row.get(1)?,
                document_id: row.get(2)?,
                chunk_index: row.get::<_, i64>(3)? as usize,
                text: row.get(4)?,
                project: row.get(5)?,
                session_id: row.get(6)?,
                source: row.get(7)?,
                timestamp: row.get(8)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    Ok(rows)
}

/// Delete a raw document and all its chunks (cascade via the FK).
/// Also removes the corresponding embeddings from `raw_chunks_vec` since vec0
/// tables don't honor SQLite FK cascades.
///
/// Safe to call from inside a caller-managed transaction — does not open its own.
pub fn delete_document(conn: &Connection, document_id: &str) -> rusqlite::Result<()> {
    // Collect chunk IDs first, then delete their embeddings + chunk rows + doc row.
    let mut stmt = conn.prepare("SELECT id FROM raw_chunks WHERE document_id = ?1")?;
    let chunk_ids: Vec<String> = stmt
        .query_map(params![document_id], |row| row.get(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    drop(stmt);

    for id in &chunk_ids {
        conn.execute("DELETE FROM raw_chunks_vec WHERE id = ?1", params![id])?;
    }
    conn.execute(
        "DELETE FROM raw_documents WHERE id = ?1",
        params![document_id],
    )?;
    Ok(())
}

/// Total count of raw chunks across all documents.
pub fn count_chunks(conn: &Connection) -> rusqlite::Result<usize> {
    let count: i64 = conn.query_row("SELECT COUNT(*) FROM raw_chunks", [], |row| row.get(0))?;
    Ok(count as usize)
}

/// Total count of raw documents.
pub fn count_documents(conn: &Connection) -> rusqlite::Result<usize> {
    let count: i64 = conn.query_row("SELECT COUNT(*) FROM raw_documents", [], |row| row.get(0))?;
    Ok(count as usize)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{schema::create_schema, vec::init_sqlite_vec};

    fn setup() -> Connection {
        init_sqlite_vec();
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA journal_mode=WAL;").unwrap();
        create_schema(&conn).unwrap();
        conn
    }

    fn make_embedding(seed: f32) -> Vec<f32> {
        (0..RAW_EMBEDDING_DIM)
            .map(|j| (j as f32 * 0.001 + seed).sin())
            .collect()
    }

    fn sample_document(id: &str, project: Option<&str>, text: &str) -> RawDocument {
        RawDocument {
            id: id.to_string(),
            project: project.map(String::from),
            session_id: Some("sess-1".to_string()),
            source: "claude-code".to_string(),
            text: text.to_string(),
            timestamp: "2026-04-13T00:00:00Z".to_string(),
            metadata_json: "{}".to_string(),
        }
    }

    #[test]
    fn test_insert_and_count_documents() {
        let conn = setup();
        insert_document(&conn, &sample_document("doc1", Some("p1"), "hello world")).unwrap();
        insert_document(&conn, &sample_document("doc2", Some("p2"), "goodbye world")).unwrap();
        assert_eq!(count_documents(&conn).unwrap(), 2);
    }

    #[test]
    fn test_insert_chunk_enforces_unique_index() {
        let conn = setup();
        insert_document(&conn, &sample_document("doc1", None, "x")).unwrap();
        let chunk = RawChunk {
            id: "c1".to_string(),
            document_id: "doc1".to_string(),
            chunk_index: 0,
            text: "first".to_string(),
            metadata_json: "{}".to_string(),
        };
        insert_chunk(&conn, &chunk).unwrap();

        // Same (document_id, chunk_index) must fail the UNIQUE constraint.
        let dup = RawChunk {
            id: "c2".to_string(),
            ..chunk.clone()
        };
        assert!(insert_chunk(&conn, &dup).is_err());
    }

    #[test]
    fn test_store_embedding_dim_guard() {
        let conn = setup();
        insert_document(&conn, &sample_document("doc1", None, "x")).unwrap();
        insert_chunk(
            &conn,
            &RawChunk {
                id: "c1".to_string(),
                document_id: "doc1".to_string(),
                chunk_index: 0,
                text: "x".to_string(),
                metadata_json: "{}".to_string(),
            },
        )
        .unwrap();

        // Wrong dimension → error.
        let bad = vec![0.0f32; 768];
        assert!(store_chunk_embedding(&conn, "c1", &bad).is_err());

        // Right dimension → ok.
        let good = make_embedding(0.0);
        store_chunk_embedding(&conn, "c1", &good).unwrap();
    }

    #[test]
    fn test_search_chunks_ranks_by_cosine() {
        let conn = setup();
        insert_document(&conn, &sample_document("doc1", Some("p1"), "body")).unwrap();

        for (idx, seed) in [0.0f32, 1.0, 2.0].iter().enumerate() {
            let chunk_id = format!("c{idx}");
            insert_chunk(
                &conn,
                &RawChunk {
                    id: chunk_id.clone(),
                    document_id: "doc1".to_string(),
                    chunk_index: idx,
                    text: format!("chunk {idx}"),
                    metadata_json: "{}".to_string(),
                },
            )
            .unwrap();
            store_chunk_embedding(&conn, &chunk_id, &make_embedding(*seed)).unwrap();
        }

        let query = make_embedding(0.0);
        let hits = search_chunks(&conn, &query, None, None, 3, None).unwrap();
        assert_eq!(hits.len(), 3);
        assert_eq!(hits[0].chunk_id, "c0");
        assert!(hits[0].distance.abs() < 0.001);
        // Hits sorted ascending by distance.
        for w in hits.windows(2) {
            assert!(w[0].distance <= w[1].distance);
        }
    }

    #[test]
    fn test_search_chunks_filters_project() {
        let conn = setup();
        insert_document(&conn, &sample_document("doc_p1", Some("p1"), "body")).unwrap();
        insert_document(&conn, &sample_document("doc_p2", Some("p2"), "body")).unwrap();

        for (doc_id, chunk_id, seed) in [("doc_p1", "c_p1", 0.0f32), ("doc_p2", "c_p2", 0.1f32)] {
            insert_chunk(
                &conn,
                &RawChunk {
                    id: chunk_id.to_string(),
                    document_id: doc_id.to_string(),
                    chunk_index: 0,
                    text: "x".to_string(),
                    metadata_json: "{}".to_string(),
                },
            )
            .unwrap();
            store_chunk_embedding(&conn, chunk_id, &make_embedding(seed)).unwrap();
        }

        let query = make_embedding(0.0);
        let hits = search_chunks(&conn, &query, Some("p1"), None, 10, None).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].chunk_id, "c_p1");
    }

    #[test]
    fn test_search_chunks_max_distance_filter() {
        let conn = setup();
        insert_document(&conn, &sample_document("doc1", None, "body")).unwrap();

        // Two chunks at different similarity levels.
        for (idx, seed) in [0.0f32, 5.0].iter().enumerate() {
            let chunk_id = format!("c{idx}");
            insert_chunk(
                &conn,
                &RawChunk {
                    id: chunk_id.clone(),
                    document_id: "doc1".to_string(),
                    chunk_index: idx,
                    text: "x".to_string(),
                    metadata_json: "{}".to_string(),
                },
            )
            .unwrap();
            store_chunk_embedding(&conn, &chunk_id, &make_embedding(*seed)).unwrap();
        }

        let query = make_embedding(0.0);
        // Tight cutoff drops the far chunk.
        let tight = search_chunks(&conn, &query, None, None, 10, Some(0.01)).unwrap();
        assert_eq!(tight.len(), 1);
        assert_eq!(tight[0].chunk_id, "c0");
    }

    #[test]
    fn test_delete_document_cascades_chunks_and_embeddings() {
        let conn = setup();
        insert_document(&conn, &sample_document("doc1", None, "body")).unwrap();
        insert_chunk(
            &conn,
            &RawChunk {
                id: "c1".to_string(),
                document_id: "doc1".to_string(),
                chunk_index: 0,
                text: "x".to_string(),
                metadata_json: "{}".to_string(),
            },
        )
        .unwrap();
        store_chunk_embedding(&conn, "c1", &make_embedding(0.0)).unwrap();

        // FK cascades need to be enabled — default SQLite behavior is off. The raw
        // layer only works correctly if the daemon runs `PRAGMA foreign_keys=ON`.
        // Enable here for the test.
        conn.execute_batch("PRAGMA foreign_keys=ON;").unwrap();

        delete_document(&conn, "doc1").unwrap();
        assert_eq!(count_documents(&conn).unwrap(), 0);
        assert_eq!(count_chunks(&conn).unwrap(), 0);
        let vec_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM raw_chunks_vec", [], |r| r.get(0))
            .unwrap();
        assert_eq!(vec_count, 0);
    }

    #[test]
    fn test_fts_search_raw_chunks() {
        let conn = setup();
        insert_document(&conn, &sample_document("doc1", None, "body")).unwrap();
        insert_chunk(
            &conn,
            &RawChunk {
                id: "c1".to_string(),
                document_id: "doc1".to_string(),
                chunk_index: 0,
                text: "ferris the crab eats rust".to_string(),
                metadata_json: "{}".to_string(),
            },
        )
        .unwrap();

        let mut stmt = conn
            .prepare("SELECT rowid FROM raw_chunks_fts WHERE raw_chunks_fts MATCH 'ferris'")
            .unwrap();
        let rows: Vec<i64> = stmt
            .query_map([], |r| r.get::<_, i64>(0))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        assert_eq!(rows.len(), 1);
    }

    // ──────────────────────────────────────────────────────────
    // search_chunks_bm25 — FTS5 MATCH keyword retrieval.
    //
    // Sibling to `search_chunks` (KNN). Together they form the two legs of
    // the hybrid raw search path. Written test-first; one cycle per behavior.
    // ──────────────────────────────────────────────────────────

    fn insert_named_chunk(conn: &Connection, doc_id: &str, chunk_id: &str, idx: usize, text: &str) {
        insert_chunk(
            conn,
            &RawChunk {
                id: chunk_id.to_string(),
                document_id: doc_id.to_string(),
                chunk_index: idx,
                text: text.to_string(),
                metadata_json: "{}".to_string(),
            },
        )
        .unwrap();
    }

    #[test]
    fn search_chunks_bm25_matches_single_term() {
        let conn = setup();
        insert_document(&conn, &sample_document("doc1", None, "body")).unwrap();
        insert_named_chunk(&conn, "doc1", "c_rust", 0, "rust programming is fast");
        insert_named_chunk(&conn, "doc1", "c_python", 1, "python code is expressive");
        insert_named_chunk(&conn, "doc1", "c_js", 2, "javascript runs in browsers");

        let hits = search_chunks_bm25(&conn, "rust", None, None, 10).unwrap();
        assert!(
            hits.iter().any(|h| h.chunk_id == "c_rust"),
            "expected c_rust in hits, got: {hits:?}"
        );
        assert_eq!(
            hits[0].chunk_id, "c_rust",
            "expected c_rust to rank first (only chunk containing the term)"
        );
    }

    #[test]
    fn search_chunks_bm25_filters_project() {
        let conn = setup();
        insert_document(&conn, &sample_document("doc_p1", Some("p1"), "body")).unwrap();
        insert_document(&conn, &sample_document("doc_p2", Some("p2"), "body")).unwrap();
        insert_named_chunk(&conn, "doc_p1", "c_p1", 0, "rust programming is fast");
        insert_named_chunk(&conn, "doc_p2", "c_p2", 0, "rust programming is fast");

        let hits = search_chunks_bm25(&conn, "rust", Some("p1"), None, 10).unwrap();
        assert_eq!(
            hits.len(),
            1,
            "expected exactly one hit after project filter, got {:?}",
            hits.iter().map(|h| &h.chunk_id).collect::<Vec<_>>()
        );
        assert_eq!(hits[0].chunk_id, "c_p1");
        assert_eq!(hits[0].project.as_deref(), Some("p1"));
    }

    #[test]
    fn search_chunks_bm25_combined_project_and_session_filter() {
        // 2×2 matrix of (project, session) — all chunks contain the same
        // keyword. Querying with both filters must return ONLY the
        // (p1, s1) intersection, not the project-only or session-only
        // supersets. Catches the bug where one filter silently overrides
        // or masks the other inside the WHERE clause.
        let conn = setup();
        let doc = |id: &str, p: &str, s: &str| RawDocument {
            id: id.to_string(),
            project: Some(p.to_string()),
            session_id: Some(s.to_string()),
            source: "test".to_string(),
            text: "body".to_string(),
            timestamp: "2026-04-14T00:00:00Z".to_string(),
            metadata_json: "{}".to_string(),
        };
        insert_document(&conn, &doc("doc_11", "p1", "s1")).unwrap();
        insert_document(&conn, &doc("doc_12", "p1", "s2")).unwrap();
        insert_document(&conn, &doc("doc_21", "p2", "s1")).unwrap();
        insert_document(&conn, &doc("doc_22", "p2", "s2")).unwrap();
        insert_named_chunk(&conn, "doc_11", "c_11", 0, "rust programming");
        insert_named_chunk(&conn, "doc_12", "c_12", 0, "rust programming");
        insert_named_chunk(&conn, "doc_21", "c_21", 0, "rust programming");
        insert_named_chunk(&conn, "doc_22", "c_22", 0, "rust programming");

        let hits = search_chunks_bm25(&conn, "rust", Some("p1"), Some("s1"), 10).unwrap();
        let ids: Vec<_> = hits.iter().map(|h| h.chunk_id.as_str()).collect();
        assert_eq!(
            hits.len(),
            1,
            "combined filter must return exactly (p1,s1), got {ids:?}"
        );
        assert_eq!(hits[0].chunk_id, "c_11");
        assert_eq!(hits[0].project.as_deref(), Some("p1"));
        assert_eq!(hits[0].session_id.as_deref(), Some("s1"));
    }

    #[test]
    fn search_chunks_bm25_project_filter_does_not_truncate_top_k() {
        // The adversarial reviewer found a real bug: if the top-K BM25
        // hits are all in the wrong project, a post-SQL filter drops them
        // and returns zero results even when matching chunks exist in the
        // right project. Unlike vec0, FTS5 JOINs accept WHERE clauses —
        // the fix is to push the filter into SQL.
        //
        // Setup: 5 high-frequency chunks in `p_other` (dominate BM25
        // ranking), 2 lower-frequency chunks in `p_target`. k=3. With the
        // current post-filter impl, SQL returns top-3 (all p_other),
        // post-filter drops them, we get 0 hits — BUG. With the fix, SQL
        // pre-filters to p_target, returns both c_target chunks, we get 2.
        let conn = setup();
        insert_document(
            &conn,
            &sample_document("doc_other", Some("p_other"), "body"),
        )
        .unwrap();
        insert_document(
            &conn,
            &sample_document("doc_target", Some("p_target"), "body"),
        )
        .unwrap();

        for i in 0..5 {
            insert_named_chunk(
                &conn,
                "doc_other",
                &format!("c_other_{i}"),
                i,
                "rust rust rust rust rust",
            );
        }
        for i in 0..2 {
            insert_named_chunk(
                &conn,
                "doc_target",
                &format!("c_target_{i}"),
                i,
                "rust programming",
            );
        }

        let hits = search_chunks_bm25(&conn, "rust", Some("p_target"), None, 3).unwrap();
        let ids: Vec<_> = hits.iter().map(|h| h.chunk_id.as_str()).collect();
        assert_eq!(
            hits.len(),
            2,
            "expected 2 p_target hits (not truncated by higher-frequency p_other chunks), got {ids:?}"
        );
        for h in &hits {
            assert_eq!(h.project.as_deref(), Some("p_target"));
        }
    }

    #[test]
    fn search_chunks_bm25_ranks_by_relevance() {
        // BM25 must rank chunks by term frequency + inverse document length.
        // A chunk with 5 occurrences of "rust" in a short body must rank
        // above a chunk with 1 occurrence of "rust" in a longer body.
        //
        // Without this test a sort-direction inversion in the FTS5 ORDER BY
        // clause would still pass cycles 1–3 (which only check matching-vs-
        // non-matching) but silently destroy real relevance. This is the
        // load-bearing ordering gate — flagged explicitly by the adversarial
        // review as the missing check in the day-1 plan.
        let conn = setup();
        insert_document(&conn, &sample_document("doc1", None, "body")).unwrap();
        insert_named_chunk(&conn, "doc1", "c_high", 0, "rust rust rust rust rust");
        insert_named_chunk(
            &conn,
            "doc1",
            "c_low",
            1,
            "i once heard someone mention rust briefly in passing",
        );
        insert_named_chunk(&conn, "doc1", "c_none", 2, "python is expressive");

        let hits = search_chunks_bm25(&conn, "rust", None, None, 10).unwrap();
        let ids: Vec<_> = hits.iter().map(|h| h.chunk_id.as_str()).collect();
        assert_eq!(
            hits.len(),
            2,
            "only 2 chunks should match 'rust', got {ids:?}"
        );
        assert_eq!(
            hits[0].chunk_id, "c_high",
            "c_high (5× term frequency) must rank above c_low (1× in longer body) — got {ids:?}"
        );
        assert_eq!(hits[1].chunk_id, "c_low");

        // SQLite bm25() returns negative scores; ORDER BY ASC → lowest first.
        // Assert the ranking field is monotone ascending.
        assert!(
            hits[0].distance <= hits[1].distance,
            "hits must be sorted ascending by BM25 score (lower=better), got {} then {}",
            hits[0].distance,
            hits[1].distance
        );
    }

    #[test]
    fn search_chunks_bm25_empty_query_returns_empty() {
        // Two flavors of "empty": literal "" and all-punctuation "!!!".
        // Both must resolve to the sanitized empty string and return an
        // empty Vec — never propagate an FTS5 syntax error to the caller.
        let conn = setup();
        insert_document(&conn, &sample_document("doc1", None, "body")).unwrap();
        insert_named_chunk(&conn, "doc1", "c1", 0, "rust programming is fast");

        let hits_punct = search_chunks_bm25(&conn, "!!!", None, None, 10).unwrap();
        assert!(
            hits_punct.is_empty(),
            "all-punctuation query must return no hits, got {hits_punct:?}"
        );

        let hits_empty = search_chunks_bm25(&conn, "", None, None, 10).unwrap();
        assert!(
            hits_empty.is_empty(),
            "literal empty query must return no hits, got {hits_empty:?}"
        );
    }

    #[test]
    fn search_chunks_bm25_filters_session() {
        // Two documents sharing the same project but in different sessions.
        // The session filter must reject chunks from the non-matching session
        // even when the keyword matches — this closes the cross-tenant leak
        // the adversarial review flagged as CRITICAL.
        let conn = setup();
        let doc_a = RawDocument {
            id: "doc_a".to_string(),
            project: Some("p1".to_string()),
            session_id: Some("sess_a".to_string()),
            source: "test".to_string(),
            text: "body".to_string(),
            timestamp: "2026-04-14T00:00:00Z".to_string(),
            metadata_json: "{}".to_string(),
        };
        let doc_b = RawDocument {
            id: "doc_b".to_string(),
            project: Some("p1".to_string()),
            session_id: Some("sess_b".to_string()),
            source: "test".to_string(),
            text: "body".to_string(),
            timestamp: "2026-04-14T00:00:00Z".to_string(),
            metadata_json: "{}".to_string(),
        };
        insert_document(&conn, &doc_a).unwrap();
        insert_document(&conn, &doc_b).unwrap();
        insert_named_chunk(&conn, "doc_a", "c_a", 0, "rust programming is fast");
        insert_named_chunk(&conn, "doc_b", "c_b", 0, "rust programming is fast");

        let hits = search_chunks_bm25(&conn, "rust", None, Some("sess_a"), 10).unwrap();
        assert_eq!(
            hits.len(),
            1,
            "expected exactly one hit after session filter, got {:?}",
            hits.iter().map(|h| &h.chunk_id).collect::<Vec<_>>()
        );
        assert_eq!(hits[0].chunk_id, "c_a");
        assert_eq!(hits[0].session_id.as_deref(), Some("sess_a"));
    }
}
