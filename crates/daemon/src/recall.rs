use crate::db::ops;
use crate::graph::GraphStore;
use crate::vector::VectorIndex;
use forge_core::protocol::MemoryResult;
use forge_core::types::{Memory, MemoryStatus, MemoryType};
use rusqlite::{params, Connection};
use std::collections::{HashMap, HashSet};

/// Reciprocal Rank Fusion merges multiple ranked lists.
///
/// Score = sum(1 / (k + rank_i + 1)) across lists where the item appears.
/// k=60 is the standard constant. Higher k gives more weight to lower-ranked items.
fn rrf_merge(lists: &[Vec<(String, f64)>], k: f64, limit: usize) -> Vec<(String, f64)> {
    let mut scores: HashMap<String, f64> = HashMap::new();

    for list in lists {
        for (rank, (id, _original_score)) in list.iter().enumerate() {
            *scores.entry(id.clone()).or_insert(0.0) += 1.0 / (k + rank as f64 + 1.0);
        }
    }

    let mut merged: Vec<(String, f64)> = scores.into_iter().collect();
    merged.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    merged.truncate(limit);
    merged
}

/// Fetch a single Memory record from SQLite by its ID.
fn fetch_memory_by_id(conn: &Connection, id: &str) -> rusqlite::Result<Option<Memory>> {
    let mut stmt = conn.prepare(
        "SELECT id, memory_type, title, content, confidence, status, project, tags, created_at, accessed_at
         FROM memory WHERE id = ?1",
    )?;

    let mut rows = stmt.query(params![id])?;

    if let Some(row) = rows.next()? {
        let type_str: String = row.get(1)?;
        let status_str: String = row.get(5)?;
        let project: Option<String> = row.get(6)?;
        let tags_json: String = row.get(7)?;

        let memory_type = match type_str.as_str() {
            "decision" => MemoryType::Decision,
            "lesson" => MemoryType::Lesson,
            "pattern" => MemoryType::Pattern,
            "preference" => MemoryType::Preference,
            _ => MemoryType::Decision,
        };

        let status = match status_str.as_str() {
            "active" => MemoryStatus::Active,
            "superseded" => MemoryStatus::Superseded,
            "reverted" => MemoryStatus::Reverted,
            "faded" => MemoryStatus::Faded,
            _ => MemoryStatus::Active,
        };

        let tags: Vec<String> =
            serde_json::from_str(&tags_json).unwrap_or_default();

        Ok(Some(Memory {
            id: row.get(0)?,
            memory_type,
            title: row.get(2)?,
            content: row.get(3)?,
            confidence: row.get(4)?,
            status,
            project,
            tags,
            embedding: None, // embeddings not stored in SQLite
            created_at: row.get(8)?,
            accessed_at: row.get(9)?,
        }))
    } else {
        Ok(None)
    }
}

/// Hybrid recall combining BM25 full-text search, vector similarity search,
/// and graph expansion via Reciprocal Rank Fusion.
///
/// Steps:
/// 1. BM25 search via FTS5
/// 2. Vector search if an embedding is provided and the index is non-empty
/// 3. RRF merge of both result lists (k=60)
/// 4. Graph expansion: 1-hop neighbors of top 5 results
/// 5. Fetch full Memory records from SQLite
/// 6. Filter by memory_type if specified
/// 7. Touch accessed_at for returned IDs
/// 8. Return Vec<MemoryResult> with score and source="hybrid"
pub fn hybrid_recall(
    conn: &Connection,
    vector_idx: &VectorIndex,
    graph: &GraphStore,
    query: &str,
    query_embedding: Option<&[f32]>,
    memory_type: Option<&MemoryType>,
    limit: usize,
) -> Vec<MemoryResult> {
    let mut ranked_lists: Vec<Vec<(String, f64)>> = Vec::new();

    // 1. BM25 search
    match ops::recall_bm25(conn, query, limit * 3) {
        Ok(bm25_results) => {
            let bm25_list: Vec<(String, f64)> = bm25_results
                .into_iter()
                .map(|r| (r.id, r.score))
                .collect();
            if !bm25_list.is_empty() {
                ranked_lists.push(bm25_list);
            }
        }
        Err(e) => {
            eprintln!("[recall] BM25 search error: {}", e);
            // Continue with empty BM25 list — vector search may still work
        }
    }

    // 2. Vector search (only if embedding provided and index non-empty)
    // NEW-4: handle Result from search (no panic on dimension mismatch)
    if let Some(emb) = query_embedding {
        if !vector_idx.is_empty() {
            match vector_idx.search(emb, limit * 3) {
                Ok(vec_results) => {
                    let vec_list: Vec<(String, f64)> = vec_results
                        .into_iter()
                        .map(|(id, distance)| (id, 1.0 - distance as f64))
                        .collect();
                    if !vec_list.is_empty() {
                        ranked_lists.push(vec_list);
                    }
                }
                Err(e) => {
                    eprintln!("[recall] vector search error: {}", e);
                    // Continue without vector results
                }
            }
        }
    }

    // 3. RRF merge
    let merged = rrf_merge(&ranked_lists, 60.0, limit);

    // 4. Graph expansion: for top 5 merged results, get 1-hop neighbors
    // NEW-5: Use HashSet for O(1) dedup instead of O(n) Vec::contains
    let mut all_ids: Vec<String> = merged.iter().map(|(id, _)| id.clone()).collect();
    let mut seen_ids: HashSet<String> = all_ids.iter().cloned().collect();
    let top_for_expansion = merged.iter().take(5).map(|(id, _)| id.clone()).collect::<Vec<_>>();
    for id in &top_for_expansion {
        let neighbors = graph.neighbors(id);
        for (neighbor_id, _edge_type) in neighbors {
            if seen_ids.insert(neighbor_id.clone()) {
                all_ids.push(neighbor_id);
            }
        }
    }

    // Build a score map from merged results; graph-expanded items get a small bonus score
    let score_map: HashMap<String, f64> = merged.iter().cloned().collect();

    // 5. Fetch full Memory records from SQLite
    let mut results: Vec<MemoryResult> = Vec::new();
    for id in &all_ids {
        if let Ok(Some(memory)) = fetch_memory_by_id(conn, id) {
            let score = score_map.get(id).copied().unwrap_or(0.001); // graph-expanded get minimal score
            results.push(MemoryResult {
                memory,
                score,
                source: "hybrid".to_string(),
            });
        }
    }

    // Filter by memory_type if specified
    if let Some(mt) = memory_type {
        results.retain(|r| &r.memory.memory_type == mt);
    }

    // Sort by score descending
    results.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
    results.truncate(limit);

    // 6. Touch accessed_at for returned IDs
    let returned_ids: Vec<&str> = results.iter().map(|r| r.memory.id.as_str()).collect();
    ops::touch(conn, &returned_ids);

    results
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::schema::create_schema;

    fn setup() -> (Connection, VectorIndex, GraphStore) {
        let conn = Connection::open_in_memory().unwrap();
        create_schema(&conn).unwrap();
        let vi = VectorIndex::new(768);
        let gs = GraphStore::new();
        (conn, vi, gs)
    }

    #[test]
    fn test_hybrid_recall_bm25_only() {
        let (conn, vi, gs) = setup();

        let m = Memory::new(
            MemoryType::Decision,
            "Use JWT",
            "For authentication across microservices",
        );
        ops::remember(&conn, &m).unwrap();

        let results = hybrid_recall(&conn, &vi, &gs, "JWT authentication", None, None, 10);

        assert!(!results.is_empty(), "should find at least one result");
        assert!(
            results[0].memory.title.contains("JWT"),
            "first result title should contain JWT"
        );
        assert_eq!(results[0].source, "hybrid");
    }

    #[test]
    fn test_hybrid_recall_with_vector() {
        let (conn, mut vi, gs) = setup();

        let m = Memory::new(
            MemoryType::Decision,
            "Use JWT",
            "For authentication across microservices",
        );
        let mem_id = m.id.clone();
        ops::remember(&conn, &m).unwrap();

        // Create a fake embedding and insert into vector index
        let dim = 768;
        let emb: Vec<f32> = (0..dim).map(|j| (j as f32 * 0.001).sin()).collect();
        vi.insert(&mem_id, &emb).unwrap();

        // Use a slightly different embedding as the query
        let query_emb: Vec<f32> = (0..dim).map(|j| (j as f32 * 0.001 + 0.01).sin()).collect();

        let results = hybrid_recall(
            &conn,
            &vi,
            &gs,
            "JWT",
            Some(&query_emb),
            None,
            10,
        );

        assert!(!results.is_empty(), "should find results with both BM25 and vector");
    }

    #[test]
    fn test_hybrid_recall_graph_expansion() {
        let (conn, vi, mut gs) = setup();

        // Insert memory A ("JWT auth") and memory B ("Token rotation")
        let m_a = Memory::new(
            MemoryType::Decision,
            "Use JWT",
            "For authentication across microservices",
        );
        let m_b = Memory::new(
            MemoryType::Decision,
            "Token rotation policy",
            "Rotate refresh tokens every 7 days for security compliance",
        );
        let id_a = m_a.id.clone();
        let id_b = m_b.id.clone();
        ops::remember(&conn, &m_a).unwrap();
        ops::remember(&conn, &m_b).unwrap();

        // Add edge: A -[motivated_by]-> B in the graph
        gs.add_edge(&id_a, &id_b, "motivated_by", serde_json::json!({}));

        // Recall "JWT" — should find A directly via BM25
        // B should appear in results via graph expansion (1-hop neighbor of A)
        let results = hybrid_recall(&conn, &vi, &gs, "JWT authentication", None, None, 10);

        assert!(!results.is_empty(), "should find at least one result");

        let has_a = results.iter().any(|r| r.memory.id == id_a);
        assert!(has_a, "memory A (JWT) should be found via BM25");

        let has_b = results.iter().any(|r| r.memory.id == id_b);
        assert!(
            has_b,
            "memory B (Token rotation) should appear via graph expansion"
        );

        // B should have a lower score than A (graph-expanded gets minimal score)
        let score_a = results.iter().find(|r| r.memory.id == id_a).unwrap().score;
        let score_b = results.iter().find(|r| r.memory.id == id_b).unwrap().score;
        assert!(
            score_a > score_b,
            "directly matched A ({score_a}) should score higher than graph-expanded B ({score_b})"
        );
    }

    #[test]
    fn test_hybrid_recall_no_matches() {
        let (conn, vi, gs) = setup();

        // Recall a query that matches nothing in an empty DB
        let results = hybrid_recall(
            &conn,
            &vi,
            &gs,
            "xyzzy nonexistent gibberish",
            None,
            None,
            10,
        );

        assert!(
            results.is_empty(),
            "should return empty results for a query matching nothing, got {} results",
            results.len()
        );
    }

    #[test]
    fn test_rrf_merge() {
        let list1 = vec![
            ("a".to_string(), 1.0),
            ("b".to_string(), 0.9),
            ("c".to_string(), 0.8),
        ];
        let list2 = vec![
            ("b".to_string(), 1.0),
            ("c".to_string(), 0.9),
            ("d".to_string(), 0.8),
        ];

        let merged = rrf_merge(&[list1, list2], 60.0, 10);

        // "b" should be #1: it appears at rank 1 in list1 and rank 0 in list2
        // b_score = 1/(60+1+1) + 1/(60+0+1) = 1/62 + 1/61
        // "a" at rank 0 in list1 only: 1/61
        // "c" at rank 2 in list1, rank 1 in list2: 1/63 + 1/62
        // "d" at rank 2 in list2 only: 1/63
        assert_eq!(merged[0].0, "b", "b should be ranked #1 (appears in both lists)");

        // Verify all 4 items are present
        let ids: Vec<&str> = merged.iter().map(|x| x.0.as_str()).collect();
        assert!(ids.contains(&"a"));
        assert!(ids.contains(&"b"));
        assert!(ids.contains(&"c"));
        assert!(ids.contains(&"d"));
    }
}
