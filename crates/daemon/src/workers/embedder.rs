// workers/embedder.rs — Batch embedding via Ollama
//
// Periodically checks for memories without embeddings and generates them
// via Ollama's /api/embed endpoint.

use crate::config::ForgeConfig;
use crate::extraction::ollama;
use crate::vector::VectorIndex;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{watch, Mutex};

/// Periodically checks for memories missing embeddings and generates them via Ollama.
///
/// Runs every 30 seconds. For each batch of up to 32 memories, calls Ollama embed
/// and inserts the resulting vectors into the VectorIndex.
/// If Ollama is unavailable, logs and retries next interval.
pub async fn run_embedder(
    state: Arc<Mutex<crate::server::handler::DaemonState>>,
    config: ForgeConfig,
    mut shutdown_rx: watch::Receiver<bool>,
) {
    eprintln!("[embedder] started, interval = 30s");

    loop {
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_secs(30)) => {}
            _ = shutdown_rx.changed() => {
                if *shutdown_rx.borrow() {
                    eprintln!("[embedder] shutdown received");
                    return;
                }
            }
        }

        // Get unembedded memories (under lock, but fast — just a SQLite query)
        let to_embed: Vec<(String, String)> = {
            let locked = state.lock().await;
            get_unembedded_memories(&locked.conn, &locked.vector_idx)
        };

        if to_embed.is_empty() {
            continue;
        }

        // Process in batches of 32
        for batch in to_embed.chunks(32) {
            let texts: Vec<String> = batch.iter().map(|(_, text)| text.clone()).collect();

            let result = ollama::embed(
                &config.extraction.ollama.endpoint,
                &config.embedding.model,
                &texts,
            )
            .await;

            match result {
                Ok(embeddings) => {
                    let mut locked = state.lock().await;
                    let mut inserted = 0usize;

                    for (i, (id, _)) in batch.iter().enumerate() {
                        if let Some(emb) = embeddings.get(i) {
                            match locked.vector_idx.insert(id, emb) {
                                Ok(_) => inserted += 1,
                                Err(e) => {
                                    eprintln!("[embedder] insert error for {id}: {e}");
                                }
                            }
                        }
                    }

                    eprintln!("[embedder] embedded {inserted}/{} memories", batch.len());
                }
                Err(e) => {
                    eprintln!("[embedder] ollama embed failed: {e}, will retry next interval");
                    break; // skip remaining batches, retry next interval
                }
            }
        }
    }
}

/// Query SQLite for memories not yet in the vector index.
///
/// Returns (id, combined_text) pairs where combined_text = title + ' ' + content.
/// Only returns active memories. Limited to 100 results per call.
fn get_unembedded_memories(
    conn: &rusqlite::Connection,
    vector_idx: &VectorIndex,
) -> Vec<(String, String)> {
    let mut stmt = match conn.prepare(
        "SELECT id, title || ' ' || content FROM memory WHERE status = 'active' LIMIT 100",
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
        Ok((id, text))
    }) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("[embedder] query_map error: {e}");
            return Vec::new();
        }
    };

    rows.filter_map(|r| r.ok())
        .filter(|(id, _)| !vector_idx.contains(id))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{ops, schema};
    use forge_v2_core::types::{Memory, MemoryType};

    #[test]
    fn test_get_unembedded_memories() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        schema::create_schema(&conn).unwrap();
        let vi = VectorIndex::new(768);

        // Insert 2 memories
        let m1 = Memory::new(MemoryType::Decision, "Use JWT", "For auth");
        let m2 = Memory::new(MemoryType::Lesson, "Test first", "TDD works");
        let _m1_id = m1.id.clone();
        ops::remember(&conn, &m1).unwrap();
        ops::remember(&conn, &m2).unwrap();

        // Both should be unembedded
        let unembedded = get_unembedded_memories(&conn, &vi);
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
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        schema::create_schema(&conn).unwrap();
        let mut vi = VectorIndex::new(4); // small dim for test

        let m1 = Memory::new(MemoryType::Decision, "Embedded", "Already done");
        let m2 = Memory::new(MemoryType::Lesson, "Not embedded", "Still pending");
        let m1_id = m1.id.clone();
        ops::remember(&conn, &m1).unwrap();
        ops::remember(&conn, &m2).unwrap();

        // Simulate m1 being already embedded
        vi.insert(&m1_id, &[0.1, 0.2, 0.3, 0.4]).unwrap();

        let unembedded = get_unembedded_memories(&conn, &vi);
        assert_eq!(unembedded.len(), 1);
        assert_eq!(unembedded[0].0, m2.id);
    }

    #[test]
    fn test_get_unembedded_empty_db() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        schema::create_schema(&conn).unwrap();
        let vi = VectorIndex::new(768);

        let unembedded = get_unembedded_memories(&conn, &vi);
        assert!(unembedded.is_empty());
    }
}
