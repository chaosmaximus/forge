// db/raw.rs — raw verbatim storage layer
//
// Stores full session text as chunks with 384-dim embeddings in raw_chunks_vec.
// Sits alongside the existing extraction pipeline — both ingest paths fire on
// the same transcript. Raw is LLM-free and exists for benchmark parity with
// published retrieval systems (see docs/benchmarks/plan.md).

use rusqlite::{params, Connection};
use zerocopy::AsBytes;

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

/// A hit returned by `search_chunks` — joins a chunk row with its parent document.
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
}
