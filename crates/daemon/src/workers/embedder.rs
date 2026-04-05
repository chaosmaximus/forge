// workers/embedder.rs — Batch embedding via Ollama → sqlite-vec
//
// Periodically checks for memories without embeddings and generates them
// via Ollama's /api/embed endpoint. Stores results in sqlite-vec.

use crate::config::ForgeConfig;
use crate::db::vec;
use crate::extraction::ollama;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{watch, Mutex};

/// Periodically checks for memories missing embeddings and generates them via Ollama.
///
/// Runs every 30 seconds. For each batch of up to 32 memories, calls Ollama embed
/// and inserts the resulting vectors into sqlite-vec (persistent storage).
/// If Ollama is unavailable, logs and retries next interval.
pub async fn run_embedder(
    state: Arc<Mutex<crate::server::handler::DaemonState>>,
    config: ForgeConfig,
    mut shutdown_rx: watch::Receiver<bool>,
    db_path: String,
) {
    eprintln!("[embedder] started, interval = 60s");

    loop {
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_secs(60)) => {}
            _ = shutdown_rx.changed() => {
                if *shutdown_rx.borrow() {
                    eprintln!("[embedder] shutdown received");
                    return;
                }
            }
        }

        // Get unembedded memories using read-only connection (no mutex contention)
        let to_embed: Vec<(String, String)> = if let Some(rc) = super::open_read_conn(&db_path) {
            get_unembedded_memories(&rc)
        } else {
            let locked = state.lock().await;
            get_unembedded_memories(&locked.conn)
        };

        if to_embed.is_empty() {
            continue;
        }

        // Process in batches of 32
        for batch in to_embed.chunks(32) {
            let batch_start = std::time::Instant::now();
            let texts: Vec<String> = batch.iter().map(|(_, text)| text.clone()).collect();

            let result = ollama::embed(
                &config.extraction.ollama.endpoint,
                &config.embedding.model,
                &texts,
            )
            .await;

            match result {
                Ok(embeddings) => {
                    let locked = state.lock().await;
                    let mut inserted = 0usize;

                    for (i, (id, _)) in batch.iter().enumerate() {
                        if let Some(emb) = embeddings.get(i) {
                            match vec::store_embedding(&locked.conn, id, emb) {
                                Ok(()) => inserted += 1,
                                Err(e) => {
                                    eprintln!("[embedder] store error for {id}: {e}");
                                }
                            }
                        }
                    }

                    eprintln!("[embedder] embedded {inserted}/{} memories ({}ms)", batch.len(), batch_start.elapsed().as_millis());
                }
                Err(e) => {
                    eprintln!("[embedder] ollama embed failed: {e}, will retry next interval");
                    break; // skip remaining batches, retry next interval
                }
            }
        }
    }
}

/// Query SQLite for memories not yet in the sqlite-vec table.
///
/// Returns (id, combined_text) pairs where combined_text = title + ' ' + content.
/// Only returns active memories. Limited to 100 results per call.
fn get_unembedded_memories(conn: &rusqlite::Connection) -> Vec<(String, String)> {
    let mut stmt = match conn.prepare(
        "SELECT m.id, m.title || ' ' || m.content
         FROM memory m
         LEFT JOIN memory_vec v ON v.id = m.id
         WHERE m.status = 'active' AND v.id IS NULL
         LIMIT 100",
    ) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("[embedder] query error: {e}");
            return Vec::new();
        }
    };

    let rows = match stmt.query_map([], |row| {
        let id: String = row.get(0)?;
        let text: String = row.get(1)?;
        // Truncate to 4000 chars to stay within nomic-embed-text 2048-token context
        let text = if text.len() > 4000 {
            let mut end = 4000;
            while !text.is_char_boundary(end) && end > 0 { end -= 1; }
            text[..end].to_string()
        } else {
            text
        };
        Ok((id, text))
    }) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("[embedder] query_map error: {e}");
            return Vec::new();
        }
    };

    rows.filter_map(|r| r.ok()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{ops, schema};
    use forge_core::types::{Memory, MemoryType};

    fn open_db() -> rusqlite::Connection {
        crate::db::vec::init_sqlite_vec();
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        schema::create_schema(&conn).unwrap();
        conn
    }

    #[test]
    fn test_get_unembedded_memories() {
        let conn = open_db();

        // Insert 2 memories
        let m1 = Memory::new(MemoryType::Decision, "Use JWT", "For auth");
        let m2 = Memory::new(MemoryType::Lesson, "Test first", "TDD works");
        ops::remember(&conn, &m1).unwrap();
        ops::remember(&conn, &m2).unwrap();

        // Both should be unembedded
        let unembedded = get_unembedded_memories(&conn);
        assert_eq!(unembedded.len(), 2);

        // Verify the returned data contains the right text
        let ids: Vec<&str> = unembedded.iter().map(|(id, _)| id.as_str()).collect();
        assert!(ids.contains(&m1.id.as_str()));
        assert!(ids.contains(&m2.id.as_str()));

        // Verify combined text format (title + ' ' + content)
        for (id, text) in &unembedded {
            if id == &m1.id {
                assert!(text.contains("Use JWT"));
                assert!(text.contains("For auth"));
            } else if id == &m2.id {
                assert!(text.contains("Test first"));
                assert!(text.contains("TDD works"));
            }
        }
    }

    #[test]
    fn test_get_unembedded_skips_embedded() {
        let conn = open_db();

        let m1 = Memory::new(MemoryType::Decision, "Embedded", "Already done");
        let m2 = Memory::new(MemoryType::Lesson, "Not embedded", "Still pending");
        let m1_id = m1.id.clone();
        ops::remember(&conn, &m1).unwrap();
        ops::remember(&conn, &m2).unwrap();

        // Simulate m1 being already embedded in sqlite-vec
        let emb: Vec<f32> = (0..768).map(|j| (j as f32 * 0.001).sin()).collect();
        vec::store_embedding(&conn, &m1_id, &emb).unwrap();

        let unembedded = get_unembedded_memories(&conn);
        assert_eq!(unembedded.len(), 1);
        assert_eq!(unembedded[0].0, m2.id);
    }

    #[test]
    fn test_get_unembedded_empty_db() {
        let conn = open_db();
        let unembedded = get_unembedded_memories(&conn);
        assert!(unembedded.is_empty());
    }
}
