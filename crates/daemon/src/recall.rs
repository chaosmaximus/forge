use crate::db::{ops, vec};
use forge_core::protocol::MemoryResult;
use forge_core::types::{Memory, MemoryType};
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
        "SELECT id, memory_type, title, content, confidence, status, project, tags, created_at, accessed_at, valence, intensity, hlc_timestamp, node_id, session_id, access_count
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

        let status = ops::status_from_str(&status_str);

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
            valence: row.get(10)?,
            intensity: row.get(11)?,
            hlc_timestamp: row.get(12)?,
            node_id: row.get(13)?,
            session_id: row.get::<_, String>(14).unwrap_or_default(),
            access_count: row.get::<_, i64>(15).unwrap_or(0) as u64,
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

    // Temporal recency boost: recent memories get up to 1.5x score
    let now_secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as f64;
    for result in &mut results {
        let created_secs = ops::parse_timestamp_to_epoch(&result.memory.created_at).unwrap_or(0.0);
        let days_old = (now_secs - created_secs).max(0.0) / 86400.0;
        let recency_boost = (-0.1 * days_old).exp();
        result.score *= 1.0 + recency_boost * 0.5;
    }
    results.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));

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

/// Escape special XML characters to prevent injection.
fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

/// Static prefix — things that don't change within a session.
/// Generated once at session-start, cached by the hook.
/// DETERMINISTIC: same input = same output, every time.
/// All XML sections always present (masking, not removal) for KV-cache stability.
pub fn compile_static_prefix(conn: &Connection, agent: &str) -> String {
    let mut xml = String::from("<forge-static>\n");

    // Platform (never changes within a session)
    {
        let platform = crate::db::manas::list_platform(conn).unwrap_or_default();
        if platform.is_empty() {
            xml.push_str("<platform/>\n");
        } else {
            xml.push_str("<platform");
            for entry in &platform {
                xml.push_str(&format!(
                    " {}=\"{}\"",
                    xml_escape(&entry.key),
                    xml_escape(&entry.value)
                ));
            }
            xml.push_str("/>\n");
        }
    }

    // Identity facets (changes rarely — user-declared)
    {
        let facets = crate::db::manas::list_identity(conn, agent, true).unwrap_or_default();
        if facets.is_empty() {
            xml.push_str(&format!(
                "<identity agent=\"{}\"/>\n",
                xml_escape(agent)
            ));
        } else {
            xml.push_str(&format!(
                "<identity agent=\"{}\">\n",
                xml_escape(agent)
            ));
            for f in &facets {
                xml.push_str(&format!(
                    "  <facet type=\"{}\" strength=\"{:.1}\">{}</facet>\n",
                    xml_escape(&f.facet),
                    f.strength,
                    xml_escape(&f.description)
                ));
            }
            xml.push_str("</identity>\n");
        }
    }

    // Disposition (changes slowly — 15min intervals)
    {
        let traits = crate::db::manas::list_dispositions(conn, agent).unwrap_or_default();
        if traits.is_empty() {
            xml.push_str("<disposition/>\n");
        } else {
            xml.push_str("<disposition");
            for t in &traits {
                xml.push_str(&format!(
                    " {}=\"{:.2}({:?})\"",
                    format!("{:?}", t.disposition_trait).to_lowercase(),
                    t.value,
                    t.trend
                ));
            }
            xml.push_str("/>\n");
        }
    }

    // Tool summary (changes only on restart)
    {
        let tools = crate::db::manas::list_tools(conn, None).unwrap_or_default();
        if tools.is_empty() {
            xml.push_str("<tools/>\n");
        } else {
            xml.push_str("<tools");
            xml.push_str(&format!(" count=\"{}\"", tools.len()));
            let names: Vec<String> = tools.iter().take(10).map(|t| t.name.clone()).collect();
            xml.push_str(&format!(" available=\"{}\"", names.join(",")));
            xml.push_str("/>\n");
        }
    }

    xml.push_str("</forge-static>");
    xml
}

/// Dynamic suffix — things that change per-turn or accumulate.
/// Regenerated on each compile_context call.
/// All XML sections always present (masking, not removal) for KV-cache stability.
pub fn compile_dynamic_suffix(
    conn: &Connection,
    agent: &str,
    project: Option<&str>,
    budget: usize,
) -> String {
    let mut xml = String::from("<forge-dynamic>\n");
    let mut used = 0usize;

    // Decisions (accumulate — always show ALL, masking with empty tag if none)
    {
        let decisions: Vec<(String, f64, String, f64)> = if let Some(proj) = project {
            conn.prepare(
                "SELECT title, confidence, valence, intensity FROM memory
                 WHERE memory_type = 'decision' AND status = 'active'
                 AND (project = ?1 OR project IS NULL OR project = '')
                 ORDER BY confidence DESC, accessed_at DESC LIMIT 10",
            )
            .and_then(|mut stmt| {
                stmt.query_map(params![proj], |row| {
                    Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
                })?
                .collect()
            })
            .unwrap_or_default()
        } else {
            conn.prepare(
                "SELECT title, confidence, valence, intensity FROM memory
                 WHERE memory_type = 'decision' AND status = 'active'
                 ORDER BY confidence DESC, accessed_at DESC LIMIT 10",
            )
            .and_then(|mut stmt| {
                stmt.query_map([], |row| {
                    Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
                })?
                .collect()
            })
            .unwrap_or_default()
        };
        if decisions.is_empty() {
            xml.push_str("<decisions/>\n");
        } else {
            let mut dec_xml = String::from("<decisions>");
            for (title, confidence, _valence, intensity) in &decisions {
                let display_confidence = if *intensity > 0.5 {
                    (confidence * (1.0 + intensity * 0.5)).min(1.0)
                } else {
                    *confidence
                };
                let entry = format!(
                    "\n  <decision confidence=\"{:.1}\">{}</decision>",
                    display_confidence,
                    xml_escape(title)
                );
                if used + dec_xml.len() + entry.len() < budget {
                    dec_xml.push_str(&entry);
                }
            }
            dec_xml.push_str("\n</decisions>\n");
            used += dec_xml.len();
            xml.push_str(&dec_xml);
        }
    }

    // Lessons (accumulate — always present)
    {
        let lessons: Vec<(String, f64, String, f64)> = if let Some(proj) = project {
            conn.prepare(
                "SELECT title, confidence, valence, intensity FROM memory
                 WHERE memory_type = 'lesson' AND status = 'active'
                 AND (project = ?1 OR project IS NULL OR project = '')
                 ORDER BY confidence DESC, accessed_at DESC LIMIT 5",
            )
            .and_then(|mut stmt| {
                stmt.query_map(params![proj], |row| {
                    Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
                })?
                .collect()
            })
            .unwrap_or_default()
        } else {
            conn.prepare(
                "SELECT title, confidence, valence, intensity FROM memory
                 WHERE memory_type = 'lesson' AND status = 'active'
                 ORDER BY confidence DESC, accessed_at DESC LIMIT 5",
            )
            .and_then(|mut stmt| {
                stmt.query_map([], |row| {
                    Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
                })?
                .collect()
            })
            .unwrap_or_default()
        };
        if lessons.is_empty() {
            xml.push_str("<lessons/>\n");
        } else {
            let mut les_xml = String::from("<lessons>");
            for (title, _confidence, _valence, _intensity) in &lessons {
                let entry = format!("\n  <lesson>{}</lesson>", xml_escape(title));
                if used + les_xml.len() + entry.len() < budget {
                    les_xml.push_str(&entry);
                }
            }
            les_xml.push_str("\n</lessons>\n");
            used += les_xml.len();
            xml.push_str(&les_xml);
        }
    }

    // Skill summaries (lazy loading — 1-line each, agent pulls details on demand)
    // Skills: project-scoped AND tool-validated
    {
        let available_tools = crate::db::manas::available_tool_names(conn).unwrap_or_default();
        let active_skills: Vec<_> = crate::db::manas::list_skills(conn, None)
            .unwrap_or_default()
            .into_iter()
            .filter(|s| {
                s.success_count > 0
                    && (s.project.is_none()
                        || s.project.as_deref() == Some("")
                        || s.project.as_deref() == project)
            })
            .filter(|s| {
                if available_tools.is_empty() {
                    return true;
                }
                let skill_text =
                    format!("{} {} {}", s.name, s.description, s.domain).to_lowercase();
                let tool_keywords: &[(&str, &str)] = &[
                    ("docker", "docker"),
                    ("kubectl", "kubectl"),
                    ("terraform", "terraform"),
                    ("npm", "npm"),
                    ("cargo", "cargo"),
                    ("pip", "pip"),
                    ("gcloud", "gcloud"),
                    ("aws", "aws"),
                    ("ssh", "ssh"),
                    ("make", "make"),
                    ("scp", "scp"),
                    ("rsync", "rsync"),
                ];
                for (keyword, tool_name) in tool_keywords {
                    if skill_text.contains(keyword) && !available_tools.contains(*tool_name) {
                        return false;
                    }
                }
                true
            })
            .take(5)
            .collect();
        if active_skills.is_empty() {
            xml.push_str("<skills/>\n");
        } else {
            let mut skill_xml = String::from(
                "<skills hint=\"use 'forge recall --layer skill &lt;keyword&gt;' for full steps\">",
            );
            for s in &active_skills {
                let entry = format!(
                    "\n  <skill domain=\"{}\" uses=\"{}\">{}</skill>",
                    xml_escape(&s.domain),
                    s.success_count,
                    xml_escape(&s.name)
                );
                if used + skill_xml.len() + entry.len() < budget {
                    skill_xml.push_str(&entry);
                }
            }
            skill_xml.push_str("\n</skills>\n");
            used += skill_xml.len();
            xml.push_str(&skill_xml);
        }
    }

    // Critical perceptions only (warnings/errors, unconsumed, project-scoped)
    {
        let critical: Vec<_> = crate::db::manas::list_unconsumed_perceptions(conn, None)
            .unwrap_or_default()
            .into_iter()
            .filter(|p| {
                let project_ok = match (&p.project, project) {
                    (Some(pp), Some(proj)) => pp == proj,
                    (None, _) => true,
                    (_, None) => true,
                };
                project_ok
                    && matches!(
                        p.severity,
                        forge_core::types::manas::Severity::Warning
                            | forge_core::types::manas::Severity::Error
                            | forge_core::types::manas::Severity::Critical
                    )
            })
            .take(3)
            .collect();
        if critical.is_empty() {
            xml.push_str("<perceptions/>\n");
        } else {
            let mut perc_xml = String::from("<perceptions>");
            for p in &critical {
                let snippet: String = xml_escape(&p.data.chars().take(100).collect::<String>());
                let sev = format!("{:?}", p.severity);
                let sev_lower = sev.to_lowercase();
                let entry = format!("\n  <{sev_lower}>{snippet}</{sev_lower}>");
                if used + perc_xml.len() + entry.len() < budget {
                    perc_xml.push_str(&entry);
                }
            }
            perc_xml.push_str("\n</perceptions>\n");
            used += perc_xml.len();
            xml.push_str(&perc_xml);
        }
    }

    // Working set from last session (pick up where you left off)
    {
        let ws = crate::sessions::get_last_working_set(conn, agent, project)
            .unwrap_or_default();
        if ws.is_empty() {
            xml.push_str("<working-set/>\n");
        } else {
            let mut ws_xml = String::from("<working-set hint=\"from your last session\">");
            ws_xml.push_str(&xml_escape(&ws));
            ws_xml.push_str("</working-set>\n");
            if used + ws_xml.len() < budget {
                xml.push_str(&ws_xml);
                // used += ws_xml.len(); // last item
            }
        }
    }

    xml.push_str("</forge-dynamic>");
    xml
}

/// Compile optimized context from all Manas layers for session-start injection.
///
/// Returns an XML string with the most relevant information, budget-limited
/// to ~4000 chars (~1000 tokens). Uses lazy loading pattern: summaries in
/// context, full content on demand via `forge recall --layer <layer>`.
/// All user-controlled strings are XML-escaped to prevent injection.
///
/// This is backward compatible: calls compile_static_prefix + compile_dynamic_suffix
/// and wraps them in a single `<forge-context>` envelope.
pub fn compile_context(
    conn: &Connection,
    agent: &str,
    project: Option<&str>,
) -> String {
    let prefix = compile_static_prefix(conn, agent);
    let suffix = compile_dynamic_suffix(conn, agent, project, 3000);
    format!(
        "<forge-context version=\"0.7.0\">\n{}\n{}\n</forge-context>",
        prefix, suffix
    )
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

    // ── compile_static_prefix tests ──

    #[test]
    fn test_compile_static_prefix_is_stable() {
        let conn = setup();

        // Store some platform data so prefix is non-trivial
        let pe1 = forge_core::types::manas::PlatformEntry {
            key: "os".into(),
            value: "linux".into(),
            detected_at: "2026-04-03".into(),
        };
        let pe2 = forge_core::types::manas::PlatformEntry {
            key: "arch".into(),
            value: "x86_64".into(),
            detected_at: "2026-04-03".into(),
        };
        crate::db::manas::store_platform(&conn, &pe1).unwrap();
        crate::db::manas::store_platform(&conn, &pe2).unwrap();

        let prefix1 = compile_static_prefix(&conn, "claude-code");
        let prefix2 = compile_static_prefix(&conn, "claude-code");
        assert_eq!(
            prefix1, prefix2,
            "static prefix should be identical across calls"
        );
    }

    #[test]
    fn test_compile_static_prefix_all_sections_present_empty_db() {
        let conn = setup();

        let prefix = compile_static_prefix(&conn, "claude-code");
        assert!(prefix.contains("<forge-static>"), "should contain opening tag");
        assert!(prefix.contains("</forge-static>"), "should contain closing tag");
        assert!(prefix.contains("<platform"), "platform always present");
        assert!(prefix.contains("<identity"), "identity always present");
        assert!(prefix.contains("<disposition"), "disposition always present");
        assert!(prefix.contains("<tools"), "tools always present");
    }

    #[test]
    fn test_compile_static_prefix_with_data() {
        let conn = setup();

        let facet = forge_core::types::manas::IdentityFacet {
            id: "f1".into(),
            agent: "claude-code".into(),
            facet: "role".into(),
            description: "Senior Rust engineer".into(),
            strength: 0.9,
            source: "user_defined".into(),
            active: true,
            created_at: "2026-04-03".into(),
        };
        crate::db::manas::store_identity(&conn, &facet).unwrap();

        let prefix = compile_static_prefix(&conn, "claude-code");
        assert!(
            prefix.contains("Senior Rust engineer"),
            "should contain identity facet"
        );
    }

    // ── compile_dynamic_suffix tests ──

    #[test]
    fn test_compile_dynamic_suffix_all_sections_present_empty_db() {
        let conn = setup();

        let suffix = compile_dynamic_suffix(&conn, "claude-code", None, 3000);
        assert!(suffix.contains("<forge-dynamic>"), "should contain opening tag");
        assert!(suffix.contains("</forge-dynamic>"), "should contain closing tag");
        assert!(suffix.contains("<decisions"), "decisions always present");
        assert!(suffix.contains("<lessons"), "lessons always present");
        assert!(suffix.contains("<skills"), "skills always present");
        assert!(suffix.contains("<perceptions"), "perceptions always present");
        assert!(suffix.contains("<working-set"), "working-set always present");
    }

    #[test]
    fn test_compile_dynamic_suffix_changes_with_new_data() {
        let conn = setup();

        let suffix1 = compile_dynamic_suffix(&conn, "claude-code", None, 3000);
        assert!(suffix1.contains("<decisions/>"), "no decisions yet");

        // Store a decision
        let mem = Memory::new(MemoryType::Decision, "Use JWT for auth", "Security decision")
            .with_confidence(0.9);
        ops::remember(&conn, &mem).unwrap();

        let suffix2 = compile_dynamic_suffix(&conn, "claude-code", None, 3000);
        assert_ne!(suffix1, suffix2, "suffix should change when data is added");
        assert!(suffix2.contains("JWT"), "should contain the new decision");
    }

    // ── compile_context tests (backward compat) ──

    #[test]
    fn test_compile_context_empty_db() {
        let conn = setup();

        let ctx = compile_context(&conn, "claude-code", None);
        assert!(ctx.contains("<forge-context"), "should contain opening tag");
        assert!(ctx.contains("</forge-context>"), "should contain closing tag");
        assert!(ctx.contains("<platform"), "should always include platform");
        // All sections always present (masking)
        assert!(ctx.contains("<decisions"), "decisions always present");
        assert!(ctx.contains("<lessons"), "lessons always present");
        assert!(ctx.contains("<skills"), "skills always present");
        assert!(ctx.contains("<perceptions"), "perceptions always present");
        assert!(ctx.contains("<working-set"), "working-set always present");
        assert!(ctx.contains("<identity"), "identity always present");
        assert!(ctx.contains("<disposition"), "disposition always present");
        assert!(ctx.contains("<tools"), "tools always present");
    }

    #[test]
    fn test_compile_context_all_sections_always_present() {
        let conn = setup();

        // Even with completely empty DB, all XML sections exist
        let ctx = compile_context(&conn, "claude-code", None);
        assert!(ctx.contains("<platform"), "platform always present");
        assert!(ctx.contains("<identity"), "identity always present");
        assert!(ctx.contains("<disposition"), "disposition always present");
        assert!(ctx.contains("<tools"), "tools always present");
        assert!(ctx.contains("<decisions"), "decisions always present");
        assert!(ctx.contains("<lessons"), "lessons always present");
        assert!(ctx.contains("<skills"), "skills always present");
        assert!(ctx.contains("<perceptions"), "perceptions always present");
        assert!(ctx.contains("<working-set"), "working-set always present");
    }

    #[test]
    fn test_compile_context_with_data() {
        let conn = setup();

        // Store a decision
        let mem = Memory::new(MemoryType::Decision, "Use JWT for auth", "Security decision")
            .with_confidence(0.9);
        ops::remember(&conn, &mem).unwrap();

        // Store an identity facet
        let facet = forge_core::types::manas::IdentityFacet {
            id: "f1".into(),
            agent: "claude-code".into(),
            facet: "role".into(),
            description: "Senior Rust engineer".into(),
            strength: 0.9,
            source: "user_defined".into(),
            active: true,
            created_at: "2026-04-03".into(),
        };
        crate::db::manas::store_identity(&conn, &facet).unwrap();

        let ctx = compile_context(&conn, "claude-code", None);
        assert!(ctx.contains("JWT"), "should contain decision about JWT");
        assert!(
            ctx.contains("Senior Rust engineer"),
            "should contain identity facet"
        );
    }

    #[test]
    fn test_compile_context_contains_both_static_and_dynamic() {
        let conn = setup();

        let ctx = compile_context(&conn, "claude-code", None);
        assert!(ctx.contains("<forge-static>"), "should contain static prefix");
        assert!(ctx.contains("</forge-static>"), "should contain static prefix closing");
        assert!(ctx.contains("<forge-dynamic>"), "should contain dynamic suffix");
        assert!(ctx.contains("</forge-dynamic>"), "should contain dynamic suffix closing");
    }

    #[test]
    fn test_compile_context_version_updated() {
        let conn = setup();

        let ctx = compile_context(&conn, "claude-code", None);
        assert!(
            ctx.contains("version=\"0.7.0\""),
            "version should be 0.7.0"
        );
    }

    #[test]
    fn test_compile_context_respects_budget() {
        let conn = setup();

        // Store 50 long decisions
        for i in 0..50 {
            let mem = Memory::new(
                MemoryType::Decision,
                &format!("Decision {} about architecture and design patterns", i),
                &"x".repeat(200),
            )
            .with_confidence(0.9);
            ops::remember(&conn, &mem).unwrap();
        }

        let ctx = compile_context(&conn, "claude-code", None);
        assert!(
            ctx.len() <= 5000,
            "context should be budget-limited, got {} chars",
            ctx.len()
        );
    }

    #[test]
    fn test_compile_context_filters_unavailable_skill_tools() {
        let conn = setup();

        // Store a skill that requires "kubectl"
        let skill = forge_core::types::Skill {
            id: "s-kubectl".into(),
            name: "Deploy to k8s".into(),
            domain: "kubernetes".into(),
            description: "kubectl apply -f deployment.yaml".into(),
            steps: vec!["kubectl apply".into()],
            success_count: 5,
            fail_count: 0,
            last_used: None,
            source: "extracted".into(),
            version: 1,
            project: None,
        };
        crate::db::manas::store_skill(&conn, &skill).unwrap();

        // Store available tools (only git, not kubectl)
        let git_tool = forge_core::types::manas::Tool {
            id: "cli:git".into(),
            name: "git".into(),
            kind: forge_core::types::manas::ToolKind::Cli,
            capabilities: vec!["version-control".into()],
            config: None,
            health: forge_core::types::manas::ToolHealth::Healthy,
            last_used: None,
            use_count: 0,
            discovered_at: "2026-04-03".into(),
        };
        crate::db::manas::store_tool(&conn, &git_tool).unwrap();

        let ctx = compile_context(&conn, "claude-code", None);
        // The k8s skill should NOT appear because kubectl is not in the tool table
        assert!(!ctx.contains("Deploy to k8s"), "skill requiring unavailable tool should be filtered");
    }

    #[test]
    fn test_compile_context_keeps_skill_with_available_tool() {
        let conn = setup();

        // Store a skill that requires "cargo"
        let skill = forge_core::types::Skill {
            id: "s-cargo".into(),
            name: "Build Rust project".into(),
            domain: "build".into(),
            description: "cargo build --release".into(),
            steps: vec!["cargo build --release".into()],
            success_count: 3,
            fail_count: 0,
            last_used: None,
            source: "extracted".into(),
            version: 1,
            project: None,
        };
        crate::db::manas::store_skill(&conn, &skill).unwrap();

        // Store cargo as available tool
        let cargo_tool = forge_core::types::manas::Tool {
            id: "cli:cargo".into(),
            name: "cargo".into(),
            kind: forge_core::types::manas::ToolKind::Cli,
            capabilities: vec!["rust-build".into()],
            config: None,
            health: forge_core::types::manas::ToolHealth::Healthy,
            last_used: None,
            use_count: 0,
            discovered_at: "2026-04-03".into(),
        };
        crate::db::manas::store_tool(&conn, &cargo_tool).unwrap();

        let ctx = compile_context(&conn, "claude-code", None);
        // The cargo skill SHOULD appear because cargo is available
        assert!(ctx.contains("Build Rust project"), "skill with available tool should be kept");
    }

    #[test]
    fn test_compile_context_no_tools_graceful_degradation() {
        let conn = setup();

        // Store a skill referencing docker but don't store any tools
        let skill = forge_core::types::Skill {
            id: "s-docker".into(),
            name: "Docker deploy".into(),
            domain: "devops".into(),
            description: "docker build and push".into(),
            steps: vec!["docker build".into()],
            success_count: 2,
            fail_count: 0,
            last_used: None,
            source: "extracted".into(),
            version: 1,
            project: None,
        };
        crate::db::manas::store_skill(&conn, &skill).unwrap();

        // No tools stored at all — graceful degradation: show all skills
        let ctx = compile_context(&conn, "claude-code", None);
        assert!(ctx.contains("Docker deploy"), "with no tools registered, all skills should pass through");
    }
}
