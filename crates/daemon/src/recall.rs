use crate::db::{ops, vec};
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
            embedding: None,
            created_at: row.get(8)?,
            accessed_at: row.get(9)?,
        }))
    } else {
        Ok(None)
    }
}

/// Get 1-hop outgoing neighbors from the edge table via SQL.
/// Replaces in-memory petgraph GraphStore.neighbors().
/// Limited to 10 neighbors per node to prevent fan-out explosion
/// (e.g., a decision that `affects` hundreds of files).
fn sql_neighbors(conn: &Connection, id: &str) -> Vec<String> {
    let mut stmt = match conn.prepare("SELECT to_id FROM edge WHERE from_id = ?1 LIMIT 10") {
        Ok(s) => s,
        Err(_) => return vec![],
    };
    let rows = match stmt.query_map(params![id], |row| row.get(0)) {
        Ok(r) => r,
        Err(_) => return vec![],
    };
    rows.filter_map(|r| r.ok()).collect()
}

/// Hybrid recall combining BM25 full-text search, vector similarity search,
/// and graph expansion via Reciprocal Rank Fusion.
///
/// All data comes from SQLite — no in-memory indexes required.
///
/// Steps:
/// 1. BM25 search via FTS5
/// 2. Vector search via sqlite-vec (if embedding provided)
/// 3. RRF merge of both result lists (k=60)
/// 4. Graph expansion: 1-hop neighbors of top 5 via SQL edge table
/// 5. Fetch full Memory records from SQLite
/// 6. Filter by memory_type if specified
/// 7. Touch accessed_at for returned IDs
/// 8. Return Vec<MemoryResult> with score and source="hybrid"
pub fn hybrid_recall(
    conn: &Connection,
    query: &str,
    query_embedding: Option<&[f32]>,
    memory_type: Option<&MemoryType>,
    project: Option<&str>,
    limit: usize,
) -> Vec<MemoryResult> {
    let mut ranked_lists: Vec<Vec<(String, f64)>> = Vec::new();

    // 1. BM25 search (project-scoped: includes global memories)
    match ops::recall_bm25_project(conn, query, project, limit * 3) {
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
        }
    }

    // 2. Vector search via sqlite-vec (if embedding provided)
    if let Some(emb) = query_embedding {
        match vec::search_vectors(conn, emb, limit * 3) {
            Ok(vec_results) => {
                // Convert cosine distance to similarity score (1 - distance)
                let vec_list: Vec<(String, f64)> = vec_results
                    .into_iter()
                    .map(|(id, distance)| (id, 1.0 - distance))
                    .collect();
                if !vec_list.is_empty() {
                    ranked_lists.push(vec_list);
                }
            }
            Err(e) => {
                eprintln!("[recall] vector search error: {}", e);
            }
        }
    }

    // 3. RRF merge
    let merged = rrf_merge(&ranked_lists, 60.0, limit);

    // 4. Graph expansion via SQL: 1-hop neighbors of top 5 results
    let mut all_ids: Vec<String> = merged.iter().map(|(id, _)| id.clone()).collect();
    let mut seen_ids: HashSet<String> = all_ids.iter().cloned().collect();
    let top_for_expansion: Vec<String> = merged.iter().take(5).map(|(id, _)| id.clone()).collect();
    for id in &top_for_expansion {
        let neighbors = sql_neighbors(conn, id);
        for neighbor_id in neighbors {
            if seen_ids.insert(neighbor_id.clone()) {
                all_ids.push(neighbor_id);
            }
        }
    }

    // Build score map; graph-expanded items get minimal score
    let score_map: HashMap<String, f64> = merged.iter().cloned().collect();

    // 5. Fetch full Memory records from SQLite
    let mut results: Vec<MemoryResult> = Vec::new();
    for id in &all_ids {
        if let Ok(Some(memory)) = fetch_memory_by_id(conn, id) {
            let score = score_map.get(id).copied().unwrap_or(0.001);
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

/// Cross-layer recall across Manas layers (declared knowledge + domain DNA).
///
/// Searches declared knowledge (Layer 5) via LIKE and domain DNA (Layer 3)
/// by pattern keyword match. Returns results as MemoryResult with lower
/// scores than direct memory matches.
pub fn manas_recall(
    conn: &Connection,
    query: &str,
    project: Option<&str>,
    limit: usize,
) -> Vec<MemoryResult> {
    let mut results = Vec::new();

    // Search declared knowledge (LIKE search on content/source)
    if let Ok(declared_list) = crate::db::manas::search_declared(conn, query, project) {
        for d in declared_list.into_iter().take(limit) {
            results.push(MemoryResult {
                memory: Memory::new(
                    MemoryType::Lesson,
                    format!("[declared:{}] {}", d.source, d.id),
                    d.content.chars().take(500).collect::<String>(),
                )
                .with_confidence(0.7),
                score: 0.5, // Lower score than direct memory matches
                source: "declared".to_string(),
            });
        }
    }

    // Search skills (Layer 2 — procedural memory)
    if let Ok(skills) = crate::db::manas::search_skills(conn, query, project) {
        for skill in skills.into_iter().take(3) {
            results.push(MemoryResult {
                memory: Memory::new(
                    MemoryType::Pattern,
                    format!("[skill:{}] {}", skill.domain, skill.name),
                    skill.description.clone(),
                )
                .with_confidence(
                    // Higher confidence for skills with more successful uses
                    (0.5 + (skill.success_count as f64 * 0.1)).min(1.0),
                ),
                score: 0.6, // Skills rank between experience and domain DNA
                source: "skill".to_string(),
            });
        }
    }

    // Search domain DNA for the project
    if let Some(proj) = project {
        if let Ok(dna_list) = crate::db::manas::list_domain_dna(conn, Some(proj)) {
            for dna in dna_list.into_iter().take(3) {
                if dna.pattern.to_lowercase().contains(&query.to_lowercase()) {
                    results.push(MemoryResult {
                        memory: Memory::new(
                            MemoryType::Pattern,
                            format!("[dna:{}] {}", dna.aspect, dna.pattern),
                            format!(
                                "Project convention: {} (confidence: {:.0}%)",
                                dna.pattern,
                                dna.confidence * 100.0
                            ),
                        )
                        .with_confidence(dna.confidence),
                        score: 0.4,
                        source: "domain_dna".to_string(),
                    });
                }
            }
        }
    }

    results.truncate(limit);
    results
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::schema::create_schema;

    fn setup() -> Connection {
        crate::db::vec::init_sqlite_vec();
        let conn = Connection::open_in_memory().unwrap();
        create_schema(&conn).unwrap();
        conn
    }

    #[test]
    fn test_hybrid_recall_bm25_only() {
        let conn = setup();

        let m = Memory::new(
            MemoryType::Decision,
            "Use JWT",
            "For authentication across microservices",
        );
        ops::remember(&conn, &m).unwrap();

        let results = hybrid_recall(&conn, "JWT authentication", None, None, None, 10);

        assert!(!results.is_empty(), "should find at least one result");
        assert!(
            results[0].memory.title.contains("JWT"),
            "first result title should contain JWT"
        );
        assert_eq!(results[0].source, "hybrid");
    }

    #[test]
    fn test_hybrid_recall_with_vector() {
        let conn = setup();

        let m = Memory::new(
            MemoryType::Decision,
            "Use JWT",
            "For authentication across microservices",
        );
        let mem_id = m.id.clone();
        ops::remember(&conn, &m).unwrap();

        // Store embedding in sqlite-vec
        let dim = 768;
        let emb: Vec<f32> = (0..dim).map(|j| (j as f32 * 0.001).sin()).collect();
        vec::store_embedding(&conn, &mem_id, &emb).unwrap();

        // Use a slightly different embedding as the query
        let query_emb: Vec<f32> = (0..dim).map(|j| (j as f32 * 0.001 + 0.01).sin()).collect();

        let results = hybrid_recall(&conn, "JWT", Some(&query_emb), None, None, 10);

        assert!(!results.is_empty(), "should find results with both BM25 and vector");
    }

    #[test]
    fn test_hybrid_recall_graph_expansion() {
        let conn = setup();

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

        // Add edge: A -[motivated_by]-> B in the SQL edge table
        ops::store_edge(&conn, &id_a, &id_b, "motivated_by", "{}").unwrap();

        // Recall "JWT" — should find A directly via BM25
        // B should appear in results via graph expansion (1-hop neighbor of A)
        let results = hybrid_recall(&conn, "JWT authentication", None, None, None, 10);

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
        let conn = setup();

        let results = hybrid_recall(&conn, "xyzzy nonexistent gibberish", None, None, None, 10);

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

        assert_eq!(merged[0].0, "b", "b should be ranked #1 (appears in both lists)");

        let ids: Vec<&str> = merged.iter().map(|x| x.0.as_str()).collect();
        assert!(ids.contains(&"a"));
        assert!(ids.contains(&"b"));
        assert!(ids.contains(&"c"));
        assert!(ids.contains(&"d"));
    }

    #[test]
    fn test_hybrid_recall_with_persistent_vector() {
        let conn = setup();

        // Store memory + embedding
        let m = Memory::new(MemoryType::Decision, "Use SQLite", "For persistent storage");
        let mem_id = m.id.clone();
        ops::remember(&conn, &m).unwrap();

        let emb: Vec<f32> = (0..768).map(|j| (j as f32 * 0.002).sin()).collect();
        vec::store_embedding(&conn, &mem_id, &emb).unwrap();

        // Verify vector is in sqlite-vec
        assert!(vec::has_embedding(&conn, &mem_id).unwrap());

        // Recall via vector similarity
        let query_emb: Vec<f32> = (0..768).map(|j| (j as f32 * 0.002 + 0.001).sin()).collect();
        let results = hybrid_recall(&conn, "SQLite storage", Some(&query_emb), None, None, 10);
        assert!(!results.is_empty());
        assert_eq!(results[0].memory.id, mem_id);
    }

    #[test]
    fn test_manas_recall_empty() {
        let conn = setup();

        // On empty DB, manas_recall should return empty vec
        let results = manas_recall(&conn, "anything", None, 10);
        assert!(results.is_empty(), "manas_recall on empty DB should return empty");
    }

    #[test]
    fn test_manas_recall_with_declared() {
        let conn = setup();

        // Store declared knowledge
        let d = forge_core::types::manas::Declared {
            id: "dk1".into(),
            source: "CLAUDE.md".into(),
            path: Some("/project/CLAUDE.md".into()),
            content: "Always use parameterized SQL queries for security".into(),
            hash: "abc123".into(),
            project: Some("forge".into()),
            ingested_at: "2026-04-03 12:00:00".into(),
        };
        crate::db::manas::store_declared(&conn, &d).unwrap();

        // Search for it via manas_recall
        let results = manas_recall(&conn, "parameterized", Some("forge"), 10);
        assert!(!results.is_empty(), "should find declared knowledge");
        assert_eq!(results[0].source, "declared");
        assert!(results[0].memory.title.contains("[declared:CLAUDE.md]"));
        assert!(results[0].score > 0.0);
    }

    #[test]
    fn test_manas_recall_with_dna() {
        let conn = setup();

        // Store domain DNA
        let dna = forge_core::types::manas::DomainDna {
            id: "d1".into(),
            project: "forge".into(),
            aspect: "naming".into(),
            pattern: "snake_case for all functions".into(),
            confidence: 0.9,
            evidence: vec!["src/main.rs".into()],
            detected_at: "2026-04-03 12:00:00".into(),
        };
        crate::db::manas::store_domain_dna(&conn, &dna).unwrap();

        // Search by pattern keyword — should find it
        let results = manas_recall(&conn, "snake_case", Some("forge"), 10);
        assert!(!results.is_empty(), "should find domain DNA by pattern keyword");
        assert_eq!(results[0].source, "domain_dna");
        assert!(results[0].memory.title.contains("[dna:naming]"));

        // Search without project — DNA should not appear (requires project)
        let results = manas_recall(&conn, "snake_case", None, 10);
        assert!(results.is_empty(), "domain DNA should not appear without project");
    }

    #[test]
    fn test_manas_recall_with_skill() {
        let conn = setup();

        // Store a skill
        let skill = forge_core::types::Skill {
            id: "s1".into(),
            name: "Deploy Rust".into(),
            domain: "devops".into(),
            description: "cargo build --release then scp binary".into(),
            steps: vec!["cargo build --release".into(), "scp binary".into()],
            success_count: 5,
            fail_count: 0,
            last_used: None,
            source: "extracted".into(),
            version: 1,
            project: None,
        };
        crate::db::manas::store_skill(&conn, &skill).unwrap();

        // Search for it via manas_recall
        let results = manas_recall(&conn, "deploy", None, 10);
        assert!(!results.is_empty(), "should find skill via manas_recall");
        assert_eq!(results[0].source, "skill");
        assert!(results[0].memory.title.contains("[skill:devops]"));
        assert!(results[0].score > 0.0);

        // Confidence should be boosted by success_count (5 * 0.1 + 0.5 = 1.0)
        assert!((results[0].memory.confidence - 1.0).abs() < f64::EPSILON,
            "5 successes should give max confidence, got {}", results[0].memory.confidence);
    }

    #[test]
    fn test_manas_recall_skill_no_match() {
        let conn = setup();

        let skill = forge_core::types::Skill {
            id: "s1".into(),
            name: "Deploy Rust".into(),
            domain: "devops".into(),
            description: "cargo build".into(),
            steps: vec![],
            success_count: 1,
            fail_count: 0,
            last_used: None,
            source: "extracted".into(),
            version: 1,
            project: None,
        };
        crate::db::manas::store_skill(&conn, &skill).unwrap();

        // Non-matching query should not return the skill
        let results = manas_recall(&conn, "xyzzy_nonexistent", None, 10);
        assert!(results.is_empty(), "non-matching query should return empty");
    }
}
