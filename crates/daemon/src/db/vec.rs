// db/vec.rs — sqlite-vec vector operations
//
// Persistent vector storage using sqlite-vec extension.
// Replaces the in-memory hnsw_rs VectorIndex.

use rusqlite::{params, Connection};
use zerocopy::AsBytes;

/// Register sqlite-vec as an auto-extension. Must be called before opening any connection.
/// Safe to call multiple times (uses std::sync::Once internally).
pub fn init_sqlite_vec() {
    use std::sync::Once;
    static INIT: Once = Once::new();
    INIT.call_once(|| {
        unsafe {
            #[allow(clippy::missing_transmute_annotations)]
            rusqlite::ffi::sqlite3_auto_extension(Some(std::mem::transmute(
                sqlite_vec::sqlite3_vec_init as *const (),
            )));
        }
    });
}

/// Store a vector embedding for a memory ID.
/// Idempotent: deletes any existing embedding for this ID first
/// (vec0 virtual tables don't support INSERT OR REPLACE).
/// Wrapped in a transaction so DELETE+INSERT is atomic — if INSERT fails,
/// the DELETE is rolled back and the old embedding is preserved.
pub fn store_embedding(conn: &Connection, id: &str, embedding: &[f32]) -> rusqlite::Result<()> {
    let tx = conn.unchecked_transaction()?;
    tx.execute("DELETE FROM memory_vec WHERE id = ?1", params![id])?;
    tx.execute(
        "INSERT INTO memory_vec(id, embedding) VALUES (?1, ?2)",
        params![id, embedding.as_bytes()],
    )?;
    tx.commit()
}

/// KNN search: find the k nearest vectors to the query embedding.
/// Returns (memory_id, distance) pairs sorted by ascending distance.
/// Distance is cosine distance (0 = identical, 2 = opposite).
pub fn search_vectors(
    conn: &Connection,
    query_embedding: &[f32],
    limit: usize,
) -> rusqlite::Result<Vec<(String, f64)>> {
    let mut stmt = conn.prepare(
        "SELECT id, distance FROM memory_vec
         WHERE embedding MATCH ?1 AND k = ?2",
    )?;
    let results = stmt
        .query_map(params![query_embedding.as_bytes(), limit as i64], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, f64>(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(results)
}

/// Check if a memory ID already has an embedding stored.
pub fn has_embedding(conn: &Connection, id: &str) -> rusqlite::Result<bool> {
    // Query the vec table for this ID. If it returns a row, the embedding exists.
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM memory_vec WHERE id = ?1",
        params![id],
        |row| row.get(0),
    )?;
    Ok(count > 0)
}

/// Count total embeddings stored.
pub fn count_embeddings(conn: &Connection) -> rusqlite::Result<usize> {
    // vec0 tables support count(*) via a shadow table
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM memory_vec",
        [],
        |row| row.get(0),
    )?;
    Ok(count as usize)
}

/// Delete an embedding by memory ID.
pub fn delete_embedding(conn: &Connection, id: &str) -> rusqlite::Result<()> {
    conn.execute("DELETE FROM memory_vec WHERE id = ?1", params![id])?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Code embeddings (code_vec table)
// ---------------------------------------------------------------------------

/// Create the code_vec virtual table for code embeddings.
/// Separate from memory_vec to keep code and memory vectors independent.
pub fn create_code_vec_table(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE VIRTUAL TABLE IF NOT EXISTS code_vec USING vec0(
            id TEXT PRIMARY KEY,
            embedding float[768] distance_metric=cosine
        );"
    )
}

/// Store a code embedding for a symbol/file ID.
/// Idempotent: deletes any existing embedding for this ID first.
/// Validates embedding dimension is 768.
pub fn store_code_embedding(conn: &Connection, id: &str, embedding: &[f32]) -> rusqlite::Result<()> {
    if embedding.len() != 768 {
        return Err(rusqlite::Error::InvalidParameterName(
            format!("code embedding must be 768-dim, got {}", embedding.len()),
        ));
    }
    let tx = conn.unchecked_transaction()?;
    tx.execute("DELETE FROM code_vec WHERE id = ?1", params![id])?;
    tx.execute(
        "INSERT INTO code_vec(id, embedding) VALUES (?1, ?2)",
        params![id, embedding.as_bytes()],
    )?;
    tx.commit()
}

/// KNN search on code embeddings: find the k nearest code vectors to the query embedding.
/// Returns (code_id, distance) pairs sorted by ascending distance.
pub fn search_code_vectors(
    conn: &Connection,
    query_embedding: &[f32],
    k: usize,
) -> rusqlite::Result<Vec<(String, f64)>> {
    let mut stmt = conn.prepare(
        "SELECT id, distance FROM code_vec
         WHERE embedding MATCH ?1 AND k = ?2",
    )?;
    let results = stmt
        .query_map(params![query_embedding.as_bytes(), k as i64], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, f64>(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(results)
}

/// Count total code embeddings stored.
pub fn count_code_embeddings(conn: &Connection) -> rusqlite::Result<i64> {
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM code_vec",
        [],
        |row| row.get(0),
    )?;
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::schema::create_schema;

    fn setup() -> Connection {
        init_sqlite_vec();
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA journal_mode=WAL;").unwrap();
        create_schema(&conn).unwrap();
        conn
    }

    fn make_embedding(dim: usize, seed: f32) -> Vec<f32> {
        (0..dim).map(|j| (j as f32 * 0.001 + seed).sin()).collect()
    }

    #[test]
    fn test_store_and_search_vector() {
        let conn = setup();

        // Store 3 embeddings with different seeds
        let emb0 = make_embedding(768, 0.0);
        let emb1 = make_embedding(768, 1.0);
        let emb2 = make_embedding(768, 2.0);
        store_embedding(&conn, "m0", &emb0).unwrap();
        store_embedding(&conn, "m1", &emb1).unwrap();
        store_embedding(&conn, "m2", &emb2).unwrap();

        // Search for nearest to emb0
        let results = search_vectors(&conn, &emb0, 3).unwrap();
        assert_eq!(results.len(), 3);
        // m0 should be nearest (distance ~ 0)
        assert_eq!(results[0].0, "m0");
        assert!(results[0].1.abs() < 0.001, "self-distance should be ~0");
        // All results should be in ascending distance order
        for w in results.windows(2) {
            assert!(w[0].1 <= w[1].1, "results should be sorted by distance");
        }
    }

    #[test]
    fn test_vector_persists_in_file() {
        init_sqlite_vec();
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test.db");

        // Store embedding, close connection
        {
            let conn = Connection::open(&db_path).unwrap();
            conn.execute_batch("PRAGMA journal_mode=WAL;").unwrap();
            create_schema(&conn).unwrap();
            let emb = make_embedding(768, 1.0);
            store_embedding(&conn, "persist_1", &emb).unwrap();
        }

        // Reopen and verify
        {
            let conn = Connection::open(&db_path).unwrap();
            assert!(has_embedding(&conn, "persist_1").unwrap());
            let emb = make_embedding(768, 1.0);
            let results = search_vectors(&conn, &emb, 5).unwrap();
            assert!(!results.is_empty());
            assert_eq!(results[0].0, "persist_1");
        }
    }

    #[test]
    fn test_empty_vector_search() {
        let conn = setup();
        let query = make_embedding(768, 0.0);
        let results = search_vectors(&conn, &query, 5).unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn test_has_embedding() {
        let conn = setup();
        assert!(!has_embedding(&conn, "missing").unwrap());

        let emb = make_embedding(768, 0.0);
        store_embedding(&conn, "exists", &emb).unwrap();
        assert!(has_embedding(&conn, "exists").unwrap());
        assert!(!has_embedding(&conn, "missing").unwrap());
    }

    #[test]
    fn test_count_embeddings() {
        let conn = setup();
        assert_eq!(count_embeddings(&conn).unwrap(), 0);

        let emb = make_embedding(768, 0.0);
        store_embedding(&conn, "a", &emb).unwrap();
        store_embedding(&conn, "b", &emb).unwrap();
        assert_eq!(count_embeddings(&conn).unwrap(), 2);
    }

    #[test]
    fn test_store_embedding_idempotent() {
        let conn = setup();
        let emb1 = make_embedding(768, 1.0);
        let emb2 = make_embedding(768, 2.0);

        store_embedding(&conn, "m1", &emb1).unwrap();
        assert_eq!(count_embeddings(&conn).unwrap(), 1);

        // Re-store with different embedding — should replace, not duplicate
        store_embedding(&conn, "m1", &emb2).unwrap();
        assert_eq!(count_embeddings(&conn).unwrap(), 1);

        // Verify it returns the updated embedding (nearest to emb2, not emb1)
        let results = search_vectors(&conn, &emb2, 1).unwrap();
        assert_eq!(results[0].0, "m1");
        assert!(results[0].1.abs() < 0.001);
    }

    #[test]
    fn test_delete_embedding() {
        let conn = setup();
        let emb = make_embedding(768, 0.0);
        store_embedding(&conn, "del_me", &emb).unwrap();
        assert!(has_embedding(&conn, "del_me").unwrap());

        delete_embedding(&conn, "del_me").unwrap();
        assert!(!has_embedding(&conn, "del_me").unwrap());
        assert_eq!(count_embeddings(&conn).unwrap(), 0);
    }

    // -----------------------------------------------------------------------
    // Code embedding tests (code_vec table)
    // -----------------------------------------------------------------------

    #[test]
    fn test_create_code_vec_table() {
        init_sqlite_vec();
        let conn = Connection::open_in_memory().unwrap();
        create_code_vec_table(&conn).unwrap();
        // Calling again should be idempotent
        create_code_vec_table(&conn).unwrap();
    }

    #[test]
    fn test_store_search_code_embedding() {
        let conn = setup();

        let emb1 = make_embedding(768, 1.0);
        let emb2 = make_embedding(768, 2.0);
        store_code_embedding(&conn, "file:src/main.rs", &emb1).unwrap();
        store_code_embedding(&conn, "file:src/lib.rs", &emb2).unwrap();

        let results = search_code_vectors(&conn, &emb1, 2).unwrap();
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].0, "file:src/main.rs");
        assert!(results[0].1.abs() < 0.001, "self-distance should be ~0");
    }

    #[test]
    fn test_count_code_embeddings() {
        let conn = setup();
        assert_eq!(count_code_embeddings(&conn).unwrap(), 0);

        let emb = make_embedding(768, 0.0);
        store_code_embedding(&conn, "sym:a", &emb).unwrap();
        store_code_embedding(&conn, "sym:b", &emb).unwrap();
        assert_eq!(count_code_embeddings(&conn).unwrap(), 2);
    }

    #[test]
    fn test_vector_join_with_memory_table() {
        let conn = setup();

        // Insert memories
        conn.execute(
            "INSERT INTO memory (id, memory_type, title, content, confidence, status, tags, created_at, accessed_at)
             VALUES ('m1', 'decision', 'Use JWT', 'For auth', 0.9, 'active', '[]', '2026-01-01', '2026-01-01')",
            [],
        ).unwrap();
        conn.execute(
            "INSERT INTO memory (id, memory_type, title, content, confidence, status, tags, created_at, accessed_at)
             VALUES ('m2', 'lesson', 'Test first', 'TDD', 0.8, 'active', '[]', '2026-01-01', '2026-01-01')",
            [],
        ).unwrap();

        // Insert embeddings
        let emb1 = make_embedding(768, 1.0);
        let emb2 = make_embedding(768, 2.0);
        store_embedding(&conn, "m1", &emb1).unwrap();
        store_embedding(&conn, "m2", &emb2).unwrap();

        // Hybrid: vector KNN JOIN to memory table
        let query = make_embedding(768, 1.0);
        let mut stmt = conn.prepare(
            "SELECT m.id, m.title, v.distance
             FROM memory_vec v
             JOIN memory m ON m.id = v.id
             WHERE v.embedding MATCH ?1 AND k = 5
             AND m.status = 'active'"
        ).unwrap();
        let results: Vec<(String, String, f64)> = stmt
            .query_map([query.as_bytes()], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();

        assert_eq!(results.len(), 2);
        assert_eq!(results[0].0, "m1"); // nearest to seed 1.0
        assert_eq!(results[0].1, "Use JWT");
    }
}
