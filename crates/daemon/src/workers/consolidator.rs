// workers/consolidator.rs — Memory consolidator (15 phases)
//
// Periodically runs: exact dedup, semantic dedup, link related, confidence decay,
// episodic->semantic promotion, reconsolidation, embedding merge,
// edge strengthening, contradiction detection, activation decay,
// entity detection, contradiction synthesis, knowledge gap detection,
// memory reweave, and quality scoring.
// Memories that fall below 0.1 effective confidence are marked "faded".

use crate::db::ops;
use crate::events;
use forge_core::types::memory::{Memory, MemoryType};
use forge_core::types::manas::{Perception, PerceptionKind, Severity};
use rusqlite::Connection;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{watch, Mutex};

// Interval is now configurable via ForgeConfig.workers.consolidation_interval_secs
// (default: 1800 = 30 minutes)

/// Stats returned by a consolidation run.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ConsolidationStats {
    pub exact_dedup: usize,
    pub semantic_dedup: usize,
    pub linked: usize,
    pub faded: usize,
    pub promoted: usize,
    pub reconsolidated: usize,
    pub embedding_merged: usize,
    pub strengthened: usize,
    pub contradictions: usize,
    pub entities_detected: usize,
    pub synthesized: usize,
    pub gaps_detected: usize,
    pub reweaved: usize,
    pub scored: usize,
    pub protocols_extracted: usize,
    pub antipatterns_tagged: usize,
    pub healed_superseded: usize,
    pub healed_faded: usize,
    pub healed_quality_adjusted: usize,
}

/// Stats from a healing cycle.
#[derive(Debug, Default, Clone)]
pub struct HealingStats {
    pub topic_superseded: usize,
    pub session_faded: usize,
    pub quality_adjusted: usize,
    pub candidates_found: usize,
    pub false_positives_skipped: usize,
}

/// Run all consolidation phases synchronously. Used by:
/// - The periodic consolidator worker (every 30 min)
/// - The ForceConsolidate handler (on demand)
/// - Daemon startup (once)
pub fn run_all_phases(conn: &Connection, config: &crate::config::ConsolidationConfig) -> ConsolidationStats {
    let mut stats = ConsolidationStats::default();

    // Phase 1: Exact dedup (fast)
    match ops::dedup_memories(conn) {
        Ok(removed) => {
            stats.exact_dedup = removed;
            if removed > 0 {
                eprintln!("[consolidator] dedup removed {} duplicate memories", removed);
            }
        }
        Err(e) => eprintln!("[consolidator] dedup error: {}", e),
    }

    // Phase 2: Semantic dedup (slow O(n^2), bounded by batch_limit)
    match ops::semantic_dedup(conn, config.batch_limit) {
        Ok(merged) => {
            stats.semantic_dedup = merged;
            if merged > 0 {
                eprintln!("[consolidator] semantic dedup merged {} near-duplicates", merged);
            }
        }
        Err(e) => eprintln!("[consolidator] semantic dedup error: {}", e),
    }

    // Phase 3: Link related memories (bounded by batch_limit)
    match ops::link_related_memories(conn, config.batch_limit) {
        Ok(linked) => {
            stats.linked = linked;
            if linked > 0 {
                eprintln!("[consolidator] linked {} related memory pairs", linked);
            }
        }
        Err(e) => eprintln!("[consolidator] link error: {}", e),
    }

    // Phase 4: Decay (bounded by batch_limit)
    match ops::decay_memories(conn, config.batch_limit) {
        Ok((_decayed, faded)) => {
            stats.faded = faded;
            if faded > 0 {
                eprintln!("[consolidator] faded {}", faded);
            }
        }
        Err(e) => eprintln!("[consolidator] decay error: {}", e),
    }

    // Phase 5: Episodic -> Semantic promotion (bounded by batch_limit)
    match ops::promote_recurring_lessons(conn, config.batch_limit) {
        Ok(promoted) => {
            stats.promoted = promoted;
            if promoted > 0 {
                eprintln!("[consolidator] promoted {} recurring lessons to patterns", promoted);
            }
        }
        Err(e) => eprintln!("[consolidator] promotion error: {}", e),
    }

    // Phase 6: Reconsolidation — boost confidence of heavily-accessed memories
    match ops::find_reconsolidation_candidates(conn) {
        Ok(candidates) => {
            for mem in &candidates {
                let new_confidence = (mem.confidence + 0.05).min(1.0);
                if let Err(e) = conn.execute(
                    "UPDATE memory SET confidence = ?1 WHERE id = ?2",
                    rusqlite::params![new_confidence, mem.id],
                ) {
                    eprintln!("[consolidator] failed to reconsolidate memory {}: {e}", mem.id);
                }
            }
            stats.reconsolidated = candidates.len();
            if !candidates.is_empty() {
                eprintln!("[consolidator] reconsolidated {} memories", candidates.len());
            }
        }
        Err(e) => eprintln!("[consolidator] reconsolidation error: {}", e),
    }

    // Phase 7: Embedding-based merge (sleep cycle — deep structural cleanup)
    match ops::embedding_merge(conn) {
        Ok(merged) => {
            stats.embedding_merged = merged;
            if merged > 0 {
                eprintln!("[consolidator] embedding merge: {} similar memories merged", merged);
            }
        }
        Err(e) => eprintln!("[consolidator] embedding merge error: {}", e),
    }

    // Phase 8: Strengthen active edges
    match ops::strengthen_active_edges(conn) {
        Ok(strengthened) => {
            stats.strengthened = strengthened;
            if strengthened > 0 {
                eprintln!("[consolidator] strengthened {} active edges", strengthened);
            }
        }
        Err(e) => eprintln!("[consolidator] edge strengthening error: {}", e),
    }

    // Phase 9: Contradiction detection
    match ops::detect_contradictions(conn) {
        Ok(found) => {
            stats.contradictions = found;
            if found > 0 {
                eprintln!("[consolidator] detected {} contradictory memory pairs", found);
            }
        }
        Err(e) => eprintln!("[consolidator] contradiction detection error: {}", e),
    }

    // Phase 10: Decay activation levels (fast — single UPDATE)
    match ops::decay_activation_levels(conn) {
        Ok(n) => {
            if n > 0 {
                eprintln!("[consolidator] decayed {} activation levels", n);
            }
        }
        Err(e) => eprintln!("[consolidator] activation decay error: {e}"),
    }

    // Phase 11: Entity detection (Knowledge Intelligence)
    match crate::db::manas::detect_entities(conn) {
        Ok(detected) => {
            stats.entities_detected = detected;
            if detected > 0 {
                eprintln!("[consolidator] detected/updated {} entities from memory titles", detected);
            }
        }
        Err(e) => eprintln!("[consolidator] entity detection error: {e}"),
    }

    // Phase 12: Contradiction synthesis — resolve detected contradictions
    let synthesized = synthesize_contradictions(conn, config.batch_limit);
    stats.synthesized = synthesized;
    if synthesized > 0 {
        eprintln!("[consolidator] synthesized {} contradiction resolutions", synthesized);
    }

    // Phase 13: Knowledge gap detection — surface concepts without entities
    let gaps = detect_and_surface_gaps(conn);
    stats.gaps_detected = gaps;
    if gaps > 0 {
        eprintln!("[consolidator] detected {} knowledge gaps", gaps);
    }

    // Phase 14: Memory reweave — enrich older memories with newer context sharing tags
    let reweaved = reweave_memories(conn, config.batch_limit, config.reweave_limit);
    stats.reweaved = reweaved;
    if reweaved > 0 {
        eprintln!("[consolidator] reweaved {} memory pairs", reweaved);
    }

    // Phase 15: Quality scoring — compute quality scores for active memories
    let scored = score_memory_quality(conn, config.batch_limit);
    stats.scored = scored;
    if scored > 0 {
        eprintln!("[consolidator] scored {} memories", scored);
    }

    // Phase 16: Portability classification — classify unknown memories
    match ops::classify_portability(conn, config.batch_limit) {
        Ok(classified) => {
            if classified > 0 {
                eprintln!("[consolidator] classified portability for {} memories", classified);
            }
        }
        Err(e) => eprintln!("[consolidator] portability classification failed: {e}"),
    }

    // Phase 17: Protocol extraction — promote recurring process patterns to protocols
    let protocols = extract_protocols(conn, config.batch_limit);
    stats.protocols_extracted = protocols;
    if protocols > 0 {
        eprintln!("[consolidator] extracted {} protocols from behavior patterns", protocols);
    }

    // Phase 18: Anti-pattern tagging — tag lessons with negative signals
    let antipatterns = tag_antipatterns(conn, config.batch_limit);
    stats.antipatterns_tagged = antipatterns;
    if antipatterns > 0 {
        eprintln!("[consolidator] tagged {} anti-patterns from lessons", antipatterns);
    }

    // Phase 19: Generate notifications from consolidation findings
    let mut notifs_generated = 0;

    // 19a: Protocol suggestion notifications
    if protocols > 0
        && !crate::notifications::check_throttle(conn, "protocol_suggestion", "local", 3600)
            .unwrap_or(true)
    {
            if let Err(e) = crate::notifications::NotificationBuilder::new(
                "confirmation", "medium",
                &format!("Forge extracted {} new protocol(s) from behavior patterns", protocols),
                "Review the new protocols with: forge-next recall --type protocol. Approve or dismiss.",
                "consolidator",
            )
            .topic("protocol_suggestion")
            .action("review_protocols", "{}")
            .build(conn) { eprintln!("[consolidator] notification failed: {e}"); }
        notifs_generated += 1;
    }

    // 19b: Contradiction notifications
    if stats.contradictions > 0
        && !crate::notifications::check_throttle(conn, "contradiction", "local", 1800)
            .unwrap_or(true)
    {
        let _ = crate::notifications::NotificationBuilder::new(
            "insight", "high",
            &format!("{} contradiction(s) detected between active decisions", stats.contradictions),
            "Review with: forge-next recall --type decision. Resolve conflicting decisions.",
            "consolidator",
        )
        .topic("contradiction")
        .build(conn);
        notifs_generated += 1;
    }

    // 19c: Quality decline check
    {
        let avg_quality: f64 = conn
            .query_row(
                "SELECT COALESCE(AVG(quality_score), 0.5) FROM memory
                 WHERE status='active' AND created_at > datetime('now', '-7 days')",
                [], |row| row.get(0),
            )
            .unwrap_or(0.5);

        if avg_quality < 0.3
            && !crate::notifications::check_throttle(conn, "quality_decline", "local", 86400)
                .unwrap_or(true)
        {
            if let Err(e) = crate::notifications::NotificationBuilder::new(
                "insight", "medium",
                "Memory quality declining",
                &format!("Average quality score for recent memories is {:.2}. Consider reviewing and cleaning up low-quality entries.", avg_quality),
                "consolidator",
            )
            .topic("quality_decline")
            .build(conn) { eprintln!("[consolidator] notification failed: {e}"); }
            notifs_generated += 1;
        }
    }

    // 19d: Meeting timeout detection
    {
        let timeout_secs = crate::config::load_config().meeting.timeout_secs;
        let timeout_modifier = format!("-{} seconds", timeout_secs);
        let timed_out: Vec<(String, String)> = conn
            .prepare(
                "SELECT id, topic FROM meeting
                 WHERE status IN ('open', 'collecting')
                 AND created_at < datetime('now', ?1)",
            )
            .ok()
            .and_then(|mut stmt| {
                stmt.query_map(rusqlite::params![timeout_modifier], |row| Ok((row.get(0)?, row.get(1)?)))
                    .ok()
                    .map(|rows| rows.filter_map(|r| r.ok()).collect())
            })
            .unwrap_or_default();

        for (meeting_id, topic) in &timed_out {
            let _ = conn.execute(
                "UPDATE meeting SET status = 'timed_out' WHERE id = ?1",
                rusqlite::params![meeting_id],
            );
            if let Err(e) = crate::notifications::NotificationBuilder::new(
                "alert", "high",
                &format!("Meeting '{}' timed out", topic),
                &format!("Meeting {} exceeded timeout. Partial responses may be available.", meeting_id),
                "meeting_engine",
            )
            .topic("meeting_timeout")
            .source_id(meeting_id)
            .build(conn) { eprintln!("[consolidator] notification failed: {e}"); }
            notifs_generated += 1;
        }
    }

    if notifs_generated > 0 {
        eprintln!("[consolidator] generated {} notifications", notifs_generated);
    }

    // ── Memory Self-Healing (Phases 20-22) ──

    // Phase 20: Topic-aware auto-supersede
    let healing_config = crate::config::load_config().healing;
    let healing_stats = heal_topic_supersedes(conn, &healing_config);
    stats.healed_superseded = healing_stats.topic_superseded;
    if healing_stats.topic_superseded > 0 {
        eprintln!("[consolidator] healing: auto-superseded {} topic-evolved memories ({} candidates, {} skipped)",
            healing_stats.topic_superseded, healing_stats.candidates_found, healing_stats.false_positives_skipped);
    }

    // Phase 21: Session staleness fade
    let healed_faded = heal_session_staleness(conn, &healing_config);
    stats.healed_faded = healed_faded;
    if healed_faded > 0 {
        eprintln!("[consolidator] healing: auto-faded {} stale memories", healed_faded);
    }

    // Phase 22: Quality pressure (natural selection)
    let quality_adjusted = apply_quality_pressure(conn, &healing_config);
    stats.healed_quality_adjusted = quality_adjusted;
    if quality_adjusted > 0 {
        eprintln!("[consolidator] healing: adjusted quality for {} memories", quality_adjusted);
    }

    // Healing notification (throttled: max once per hour)
    if (healing_stats.topic_superseded > 0 || healed_faded > 0)
        && !crate::notifications::check_throttle(conn, "healing", "local", 3600).unwrap_or(true)
    {
        if let Err(e) = crate::notifications::NotificationBuilder::new(
            "insight", "medium",
            &format!("Memory healing: {} superseded, {} faded", healing_stats.topic_superseded, healed_faded),
            &format!("Auto-superseded {} same-topic decisions, faded {} stale memories. Review: forge-next healing-log",
                healing_stats.topic_superseded, healed_faded),
            "consolidator",
        )
        .topic("healing")
        .build(conn) {
            eprintln!("[consolidator] healing notification failed: {e}");
        }
    }

    stats
}

/// Synthesize contradictions: find pairs of conflicting memories (same tags,
/// opposite valence, both active), create a resolution memory, and mark
/// originals as "superseded". Returns count of resolutions created.
pub fn synthesize_contradictions(conn: &Connection, batch_limit: usize) -> usize {
    // Find pairs of active memories with opposite valence, shared tags, high intensity
    let mut stmt = match conn.prepare(&format!(
        "SELECT id, title, content, tags, valence, intensity, confidence, project FROM memory
         WHERE status = 'active' AND valence IN ('positive', 'negative') AND intensity > 0.5
         LIMIT {batch_limit}"
    )) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("[consolidator] synthesize query error: {e}");
            return 0;
        }
    };

    struct ConflictRow {
        id: String,
        title: String,
        content: String,
        tags: Vec<String>,
        valence: String,
        confidence: f64,
        project: Option<String>,
    }

    let rows: Vec<ConflictRow> = match stmt.query_map([], |row| {
        let tags_json: String = row.get(3)?;
        Ok(ConflictRow {
            id: row.get(0)?,
            title: row.get(1)?,
            content: row.get(2)?,
            tags: serde_json::from_str(&tags_json).unwrap_or_default(),
            valence: row.get(4)?,
            confidence: row.get(6)?,
            project: row.get(7)?,
        })
    }) {
        Ok(mapped) => mapped.filter_map(|r| r.ok()).collect(),
        Err(e) => {
            eprintln!("[consolidator] synthesize row error: {e}");
            return 0;
        }
    };

    let mut synthesized = 0usize;
    // Track which memory IDs have already been superseded in this run
    let mut superseded_ids: std::collections::HashSet<String> = std::collections::HashSet::new();

    for i in 0..rows.len() {
        if superseded_ids.contains(&rows[i].id) {
            continue;
        }
        if rows[i].tags.len() < 2 {
            continue;
        }

        for j in (i + 1)..rows.len() {
            if superseded_ids.contains(&rows[j].id) {
                continue;
            }
            if rows[j].tags.len() < 2 {
                continue;
            }

            // Must have opposite valence
            if rows[i].valence == rows[j].valence {
                continue;
            }

            // Count shared tags (HashSet for O(n) instead of O(n^2))
            let tags_i: std::collections::HashSet<&str> = rows[i].tags.iter().map(|s| s.as_str()).collect();
            let shared: usize = rows[j].tags.iter().filter(|t| tags_i.contains(t.as_str())).count();
            if shared < 2 {
                continue;
            }

            // Reference both conflict rows
            let (a, b) = (&rows[i], &rows[j]);

            // Create resolution memory
            let resolution_title = format!("Resolution: {} vs {}", a.title, b.title);
            let resolution_content = format!(
                "Previously: {}. Later: {}. The later decision supersedes the earlier one.",
                a.content, b.content
            );

            // Tags: union + "resolution"
            let mut union_tags: Vec<String> = a.tags.clone();
            for t in &b.tags {
                if !union_tags.contains(t) {
                    union_tags.push(t.clone());
                }
            }
            union_tags.push("resolution".to_string());

            let conf = a.confidence.max(b.confidence);

            let resolution = Memory::new(MemoryType::Decision, &resolution_title, &resolution_content)
                .with_tags(union_tags);
            // Set confidence manually
            let mut resolution = resolution;
            resolution.confidence = conf;
            resolution.project = a.project.clone();

            // Transaction: resolution insert + supersede originals (atomic)
            if let Err(e) = conn.execute_batch("BEGIN IMMEDIATE") {
                eprintln!("[consolidator] failed to begin transaction: {e}");
                continue;
            }
            if let Err(e) = ops::remember(conn, &resolution) {
                eprintln!("[consolidator] failed to store resolution: {e}");
                let _ = conn.execute_batch("ROLLBACK");
                continue;
            }
            if conn.execute("UPDATE memory SET status = 'superseded' WHERE id = ?1", rusqlite::params![a.id]).is_err()
                || conn.execute("UPDATE memory SET status = 'superseded' WHERE id = ?1", rusqlite::params![b.id]).is_err()
            {
                eprintln!("[consolidator] failed to supersede originals — rolling back");
                let _ = conn.execute_batch("ROLLBACK");
                continue;
            }
            let _ = conn.execute_batch("COMMIT");

            superseded_ids.insert(a.id.clone());
            superseded_ids.insert(b.id.clone());
            synthesized += 1;
        }
    }

    synthesized
}

/// Detect knowledge gaps and surface them as perceptions.
/// A knowledge gap is a word appearing in 3+ memory titles but with no entity.
/// Returns count of gap perceptions created.
pub fn detect_and_surface_gaps(conn: &Connection) -> usize {
    let gaps = match crate::db::manas::detect_knowledge_gaps(conn, None) {
        Ok(g) => g,
        Err(e) => {
            eprintln!("[consolidator] knowledge gap detection error: {e}");
            return 0;
        }
    };

    let mut count = 0;
    for word in &gaps {
        // Count how many titles reference this word
        let freq: usize = conn
            .query_row(
                "SELECT COUNT(*) FROM memory WHERE status = 'active' AND LOWER(title) LIKE ?1",
                rusqlite::params![format!("%{}%", word)],
                |row| row.get(0),
            )
            .unwrap_or(0);

        let perception_id = format!("gap-{}", ulid::Ulid::new());
        let p = Perception {
            id: perception_id,
            kind: PerceptionKind::KnowledgeGap,
            data: format!("Knowledge gap: no entity for '{}' despite {} references", word, freq),
            severity: Severity::Info,
            project: None,
            created_at: forge_core::time::now_iso(),
            expires_at: Some(forge_core::time::now_offset(86400)), // 24 hours
            consumed: false,
        };

        if let Err(e) = crate::db::manas::store_perception(conn, &p) {
            eprintln!("[consolidator] failed to store gap perception: {e}");
            continue;
        }
        count += 1;
    }

    count
}

/// Reweave memories: when a newer memory shares 2+ tags with an older memory
/// and both are active with the same project and memory_type, enrich the older
/// memory by appending the newer content and mark the newer one as "merged".
/// Returns count of reweaves performed.
pub fn reweave_memories(conn: &Connection, batch_limit: usize, reweave_limit: usize) -> usize {
    // Find candidate pairs: newer memory shares 2+ tags with older memory,
    // same project, same memory_type, both active
    let mut stmt = match conn.prepare(&format!(
        "SELECT id, title, content, tags, memory_type, project, created_at FROM memory
         WHERE status = 'active' AND tags != '[]'
         ORDER BY created_at ASC
         LIMIT {batch_limit}"
    )) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("[consolidator] reweave query error: {e}");
            return 0;
        }
    };

    struct ReweaveRow {
        id: String,
        content: String,
        tags: Vec<String>,
        memory_type: String,
        project: Option<String>,
        created_at: String,
    }

    let rows: Vec<ReweaveRow> = match stmt.query_map([], |row| {
        let tags_json: String = row.get(3)?;
        Ok(ReweaveRow {
            id: row.get(0)?,
            content: row.get(2)?,
            tags: serde_json::from_str(&tags_json).unwrap_or_default(),
            memory_type: row.get(4)?,
            project: row.get(5)?,
            created_at: row.get(6)?,
        })
    }) {
        Ok(mapped) => mapped.filter_map(|r| r.ok()).collect(),
        Err(e) => {
            eprintln!("[consolidator] reweave row error: {e}");
            return 0;
        }
    };

    let mut reweaved = 0usize;
    let mut merged_ids: std::collections::HashSet<String> = std::collections::HashSet::new();
    let limit = reweave_limit;

    for i in 0..rows.len() {
        if reweaved >= limit {
            break;
        }
        if merged_ids.contains(&rows[i].id) {
            continue;
        }
        if rows[i].tags.len() < 2 {
            continue;
        }

        for j in (i + 1)..rows.len() {
            if reweaved >= limit {
                break;
            }
            if merged_ids.contains(&rows[j].id) {
                continue;
            }
            if rows[j].tags.len() < 2 {
                continue;
            }

            // Must have same memory_type
            if rows[i].memory_type != rows[j].memory_type {
                continue;
            }

            // Must have same project (both None or both same value)
            if rows[i].project != rows[j].project {
                continue;
            }

            // Count shared tags
            let tags_i: std::collections::HashSet<&str> = rows[i].tags.iter().map(|s| s.as_str()).collect();
            let shared: usize = rows[j].tags.iter().filter(|t| tags_i.contains(t.as_str())).count();
            if shared < 2 {
                continue;
            }

            // rows[i] is older (ordered by created_at ASC), rows[j] is newer
            // Verify j is indeed newer (or at least not the same)
            if rows[j].created_at <= rows[i].created_at {
                continue;
            }

            // Transaction: read current content, enrich, mark newer as merged (atomic)
            if let Err(e) = conn.execute_batch("BEGIN IMMEDIATE") {
                eprintln!("[consolidator] reweave begin error: {e}");
                continue;
            }
            // Re-read content inside transaction to avoid TOCTOU race
            let current_content: String = match conn.query_row(
                "SELECT content FROM memory WHERE id = ?1 AND status = 'active'",
                rusqlite::params![rows[i].id],
                |row| row.get(0),
            ) {
                Ok(c) => c,
                Err(_) => { let _ = conn.execute_batch("ROLLBACK"); continue; }
            };
            let enriched_content = format!("{}\n\n[Update]: {}", current_content, rows[j].content);

            let update1 = conn.execute(
                "UPDATE memory SET content = ?1 WHERE id = ?2 AND status = 'active'",
                rusqlite::params![enriched_content, rows[i].id],
            );
            let update2 = conn.execute(
                "UPDATE memory SET status = 'merged' WHERE id = ?1 AND status = 'active'",
                rusqlite::params![rows[j].id],
            );
            if update1.is_err() || update2.is_err()
                || update1.unwrap_or(0) != 1 || update2.unwrap_or(0) != 1
            {
                eprintln!("[consolidator] reweave update error — rolling back");
                let _ = conn.execute_batch("ROLLBACK");
                continue;
            }
            if let Err(e) = conn.execute_batch("COMMIT") {
                eprintln!("[consolidator] reweave commit error: {e}");
                continue;
            }

            merged_ids.insert(rows[j].id.clone());
            reweaved += 1;
        }
    }

    reweaved
}

/// Score memory quality for active memories. Computes a quality score (0.0 to 1.0)
/// based on freshness, utility (access_count), completeness (content length),
/// and activation_level. Stores the result in the quality_score column.
/// Returns count of memories scored.
pub fn score_memory_quality(conn: &Connection, batch_limit: usize) -> usize {
    let mut stmt = match conn.prepare(&format!(
        "SELECT id, content, access_count, activation_level,
                julianday('now') - julianday(created_at) as age_days
         FROM memory WHERE status = 'active'
         LIMIT {batch_limit}"
    )) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("[consolidator] quality score query error: {e}");
            return 0;
        }
    };

    struct ScoreRow {
        id: String,
        content_len: usize,
        access_count: i64,
        activation_level: f64,
        age_days: f64,
    }

    let rows: Vec<ScoreRow> = match stmt.query_map([], |row| {
        let content: String = row.get(1)?;
        Ok(ScoreRow {
            id: row.get(0)?,
            content_len: content.len(),
            access_count: row.get(2)?,
            activation_level: row.get::<_, f64>(3).unwrap_or(0.0),
            age_days: row.get::<_, f64>(4).unwrap_or(0.0),
        })
    }) {
        Ok(mapped) => mapped.filter_map(|r| r.ok()).collect(),
        Err(e) => {
            eprintln!("[consolidator] quality score row error: {e}");
            return 0;
        }
    };

    let mut scored = 0usize;
    for row in &rows {
        // freshness (0-1): 1.0 for today, decays by 0.1 per week, min 0.1
        let weeks = row.age_days / 7.0;
        let freshness = (1.0 - weeks * 0.1).clamp(0.1, 1.0);

        // utility (0-1): min(access_count / 10.0, 1.0)
        let utility = (row.access_count as f64 / 10.0).clamp(0.0, 1.0);

        // completeness (0-1): min(content.len() / 200.0, 1.0)
        let completeness = (row.content_len as f64 / 200.0).min(1.0);

        // activation (0-1): activation_level (already 0-1)
        let activation = row.activation_level.clamp(0.0, 1.0);

        let quality_score = freshness * 0.3 + utility * 0.3 + completeness * 0.2 + activation * 0.2;

        if let Err(e) = conn.execute(
            "UPDATE memory SET quality_score = ?1 WHERE id = ?2",
            rusqlite::params![quality_score, row.id],
        ) {
            eprintln!("[consolidator] quality score update error for {}: {e}", row.id);
            continue;
        }
        scored += 1;
    }

    scored
}

/// Phase 17: Protocol Extraction — promote recurring process patterns to protocols.
///
/// Only promotes memories that are clearly process-level (HOW to work):
/// - Preferences are user-declared process rules → always promote (high signal)
/// - Patterns with "Behavioral:" prefix → promote (extracted behavioral patterns)
/// - Other patterns need strong process signals in title to qualify
///
/// Avoids promoting facts, decisions, or observations that happen to
/// mention process-adjacent words.
pub fn extract_protocols(conn: &Connection, batch_limit: usize) -> usize {
    // Two-tier extraction:
    // Tier 1: ALL preferences → these are explicit user process preferences
    // Tier 2: Patterns with "Behavioral:" prefix or strong title signals
    let sql = format!(
        "SELECT id, title, content, memory_type FROM memory
         WHERE status = 'active'
           AND (
             (memory_type = 'preference')
             OR (memory_type = 'pattern' AND (
               LOWER(title) LIKE 'behavioral:%'
               OR LOWER(title) LIKE '%always %'
               OR LOWER(title) LIKE '%never %'
               OR LOWER(title) LIKE '%before every%'
               OR LOWER(title) LIKE '%after every%'
             ))
           )
         LIMIT {batch_limit}"
    );

    let mut stmt = match conn.prepare(&sql) {
        Ok(s) => s,
        Err(_) => return 0,
    };

    let candidates: Vec<(String, String, String, String)> = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?, // id
                row.get::<_, String>(1)?, // title
                row.get::<_, String>(2)?, // content
                row.get::<_, String>(3)?, // memory_type
            ))
        })
        .ok()
        .map(|rows| rows.filter_map(|r| r.ok()).collect())
        .unwrap_or_default();

    let mut promoted = 0;
    for (source_id, title, content, _memory_type) in &candidates {
        // Check if a protocol with this exact source title already exists
        let protocol_title = format!("Protocol: {}", title);
        let exists: bool = conn
            .query_row(
                "SELECT COUNT(*) FROM memory WHERE memory_type = 'protocol' AND status = 'active'
                 AND title = ?1",
                rusqlite::params![protocol_title],
                |row| row.get::<_, i64>(0),
            )
            .unwrap_or(0) > 0;

        if exists {
            continue;
        }

        // Rust-side validation: content must describe a process (not just mention a keyword)
        let content_lower = content.to_lowercase();
        let title_lower = title.to_lowercase();

        // Positive: imperative verbs, workflow instructions
        let has_process_signal = content_lower.contains("always ")
            || content_lower.contains("never ")
            || content_lower.contains("must ")
            || content_lower.contains("require")
            || content_lower.contains("workflow")
            || content_lower.contains("rule:")
            || title_lower.starts_with("behavioral:");

        // Negative: observations, facts, goals — these are NOT process rules
        let is_observation = content_lower.contains("discovered")
            || content_lower.contains("observed that")
            || content_lower.contains("validates")
            || content_lower.contains("proved that")
            || title_lower.contains("user goal")
            || title_lower.contains("user dogfoods")
            || title_lower.contains("pipeline")
            || title_lower.contains("test pattern:");

        if !has_process_signal || is_observation {
            continue;
        }

        let protocol_id = ulid::Ulid::new().to_string();
        let now = forge_core::time::now_iso();

        if conn.execute(
            "INSERT INTO memory (id, memory_type, title, content, confidence, status, created_at, accessed_at, quality_score)
             VALUES (?1, 'protocol', ?2, ?3, 0.8, 'active', ?4, ?5, 0.7)",
            rusqlite::params![protocol_id, protocol_title, content, now, now],
        ).is_ok() {
            let _ = crate::db::ops::store_edge(conn, source_id, &protocol_id, "promoted_to", "{}");
            promoted += 1;
        }
    }

    promoted
}

/// Phase 18: Anti-pattern tagging — identify lessons that describe what NOT to do.
///
/// Scans lessons for negative signals ("don't", "avoid", "caused problems",
/// "broke", "reverted", "never") and tags them with "anti-pattern" in the tags
/// JSON array. These are then surfaced in context as guardrails.
pub fn tag_antipatterns(conn: &Connection, batch_limit: usize) -> usize {
    let sql = format!(
        "SELECT id, tags FROM memory
         WHERE status = 'active'
           AND memory_type = 'lesson'
           AND tags NOT LIKE '%anti-pattern%'
           AND (
             LOWER(content) LIKE '%don''t %'
             OR LOWER(content) LIKE '%avoid %'
             OR LOWER(content) LIKE '%caused problem%'
             OR LOWER(content) LIKE '%broke %'
             OR LOWER(content) LIKE '%revert%'
             OR LOWER(content) LIKE '%never %'
             OR LOWER(content) LIKE '%bug%found%'
             OR LOWER(content) LIKE '%fail%'
             OR LOWER(title) LIKE '%don''t %'
             OR LOWER(title) LIKE '%avoid %'
             OR LOWER(title) LIKE '%pitfall%'
           )
         LIMIT {batch_limit}"
    );

    let mut stmt = match conn.prepare(&sql) {
        Ok(s) => s,
        Err(_) => return 0,
    };

    let candidates: Vec<(String, String)> = stmt
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .ok()
        .map(|rows| rows.filter_map(|r| r.ok()).collect())
        .unwrap_or_default();

    let mut tagged = 0;
    for (id, tags_json) in &candidates {
        // Parse existing tags, add "anti-pattern"
        let mut tags: Vec<String> = serde_json::from_str(tags_json).unwrap_or_default();
        if tags.iter().any(|t| t == "anti-pattern") {
            continue;
        }
        tags.push("anti-pattern".into());
        let new_tags = serde_json::to_string(&tags).unwrap_or_else(|_| "[]".into());

        if conn.execute(
            "UPDATE memory SET tags = ?1 WHERE id = ?2",
            rusqlite::params![new_tags, id],
        ).is_ok() {
            tagged += 1;
        }
    }

    tagged
}

/// Phase 20: Topic supersede — detect memories on the same topic that have been
/// superseded by newer, more complete memories.
///
/// Algorithm:
/// 1. Get active decision/lesson/pattern memories that have embeddings
/// 2. For each, KNN search for 5 nearest same-type neighbors
/// 3. Check cosine similarity > threshold (distance < 1 - threshold)
/// 4. Compute word overlap on combined title+content
/// 5. If overlap is in [overlap_low, overlap_high) — same topic, different content — SUPERSEDE older
/// 6. Skip if overlap >= overlap_high (dedup territory) or < overlap_low (false positive)
/// 7. Skip if old memory confidence >= 0.95 (user explicitly set high)
/// 8. On supersede: UPDATE status, INSERT edge, INSERT healing_log
pub fn heal_topic_supersedes(conn: &Connection, config: &crate::config::HealingConfig) -> HealingStats {
    let mut stats = HealingStats::default();

    if !config.enabled {
        return stats;
    }

    // Get active decision/lesson/pattern memories that have embeddings
    let sql = format!(
        "SELECT m.id, m.memory_type, m.title, m.content, m.confidence, m.created_at
         FROM memory m
         INNER JOIN memory_vec mv ON m.id = mv.id
         WHERE m.status = 'active'
           AND m.memory_type IN ('decision', 'lesson', 'pattern')
         LIMIT {}",
        config.batch_limit
    );

    let mut stmt = match conn.prepare(&sql) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("[healing] failed to prepare candidate query: {e}");
            return stats;
        }
    };

    struct Candidate {
        id: String,
        memory_type: String,
        title: String,
        content: String,
        confidence: f64,
        created_at: String,
    }

    let candidates: Vec<Candidate> = stmt
        .query_map([], |row| {
            Ok(Candidate {
                id: row.get(0)?,
                memory_type: row.get(1)?,
                title: row.get(2)?,
                content: row.get(3)?,
                confidence: row.get(4)?,
                created_at: row.get(5)?,
            })
        })
        .ok()
        .map(|rows| rows.filter_map(|r| r.ok()).collect())
        .unwrap_or_default();

    stats.candidates_found = candidates.len();

    // Track which memories we've already superseded this cycle to avoid double-processing
    let mut already_superseded = std::collections::HashSet::new();

    for candidate in &candidates {
        if already_superseded.contains(&candidate.id) {
            continue;
        }

        // Get the embedding for this candidate from memory_vec (raw bytes -> f32)
        let embedding: Vec<f32> = match conn.query_row(
            "SELECT embedding FROM memory_vec WHERE id = ?1",
            rusqlite::params![&candidate.id],
            |row| {
                let bytes: Vec<u8> = row.get(0)?;
                let floats: Vec<f32> = bytes
                    .chunks_exact(4)
                    .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
                    .collect();
                Ok(floats)
            },
        ) {
            Ok(emb) => emb,
            Err(_) => continue,
        };

        // KNN search for 5 nearest neighbors
        let neighbors = match crate::db::vec::search_vectors(conn, &embedding, 6) {
            Ok(n) => n,
            Err(_) => continue,
        };

        for (neighbor_id, distance) in &neighbors {
            // Skip self
            if neighbor_id == &candidate.id {
                continue;
            }
            // Skip already superseded
            if already_superseded.contains(neighbor_id) {
                continue;
            }

            // Check cosine similarity: similarity = 1.0 - distance
            // Threshold check: distance < (1.0 - cosine_threshold)
            let similarity = 1.0 - distance;
            if similarity < config.cosine_threshold {
                continue;
            }

            // Get neighbor details
            let neighbor = match conn.query_row(
                "SELECT memory_type, title, content, confidence, created_at FROM memory WHERE id = ?1 AND status = 'active'",
                rusqlite::params![neighbor_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, f64>(3)?,
                        row.get::<_, String>(4)?,
                    ))
                },
            ) {
                Ok(n) => n,
                Err(_) => continue,
            };

            let (n_type, n_title, n_content, n_confidence, n_created_at) = neighbor;

            // Must be same type
            if n_type != candidate.memory_type {
                stats.false_positives_skipped += 1;
                continue;
            }

            // Compute word overlap on combined title+content
            let cand_text = format!("{} {}", candidate.title, candidate.content);
            let neigh_text = format!("{} {}", n_title, n_content);
            let cand_words = ops::meaningful_words_pub(&cand_text);
            let neigh_words = ops::meaningful_words_pub(&neigh_text);

            let intersection = cand_words.intersection(&neigh_words).count();
            let union = cand_words.union(&neigh_words).count();
            let overlap = if union > 0 { intersection as f64 / union as f64 } else { 0.0 };

            // Skip if overlap >= overlap_high (dedup territory)
            if overlap >= config.overlap_high {
                stats.false_positives_skipped += 1;
                continue;
            }

            // Skip if overlap < overlap_low (unrelated)
            if overlap < config.overlap_low {
                stats.false_positives_skipped += 1;
                continue;
            }

            // Determine which is older — older gets superseded
            let (old_id, new_id, old_confidence) = if candidate.created_at <= n_created_at {
                (&candidate.id, neighbor_id, candidate.confidence)
            } else {
                (neighbor_id, &candidate.id, n_confidence)
            };

            // Skip if old memory confidence >= 0.95 (user explicitly set high)
            if old_confidence >= 0.95 {
                stats.false_positives_skipped += 1;
                continue;
            }

            // Supersede: update status + superseded_by
            if conn.execute(
                "UPDATE memory SET status = 'superseded', superseded_by = ?1 WHERE id = ?2",
                rusqlite::params![new_id, old_id],
            ).is_err() {
                continue;
            }

            // Insert edge
            if let Err(e) = ops::store_edge(conn, new_id, old_id, "supersedes", "{}") {
                eprintln!("[healing] edge insert failed for {new_id} -> {old_id}: {e}");
            }

            // Insert healing_log
            let log_id = ulid::Ulid::new().to_string();
            if let Err(e) = conn.execute(
                "INSERT INTO healing_log (id, action, old_memory_id, new_memory_id, similarity_score, overlap_score, reason, created_at)
                 VALUES (?1, 'auto_superseded', ?2, ?3, ?4, ?5, ?6, datetime('now'))",
                rusqlite::params![
                    log_id,
                    old_id,
                    new_id,
                    similarity,
                    overlap,
                    format!("Same topic (overlap={:.2}), newer memory supersedes older", overlap),
                ],
            ) {
                eprintln!("[healing] healing_log insert failed: {e}");
            }

            already_superseded.insert(old_id.clone());
            stats.topic_superseded += 1;
        }
    }

    if stats.topic_superseded > 0 {
        eprintln!(
            "[healing] topic supersede: {} superseded, {} candidates, {} false positives skipped",
            stats.topic_superseded, stats.candidates_found, stats.false_positives_skipped
        );
    }

    stats
}

/// Phase 21: Fade old unaccessed low-quality memories.
/// Memories with quality_score < threshold AND zero accesses AND older than N days -> faded.
pub fn heal_session_staleness(conn: &Connection, config: &crate::config::HealingConfig) -> usize {
    if !config.enabled { return 0; }

    let days = config.staleness_days;
    let min_quality = config.staleness_min_quality;

    // Fade stale memories
    let faded: usize = conn.execute(
        "UPDATE memory SET status = 'faded'
         WHERE status = 'active'
         AND COALESCE(quality_score, 0.5) < ?1
         AND access_count = 0
         AND created_at < datetime('now', ?2)",
        rusqlite::params![min_quality, format!("-{days} days")],
    ).unwrap_or(0);

    // Log each faded memory
    if faded > 0 {
        let now = forge_core::time::now_iso();
        // Query the just-faded memories to log them
        let ids: Vec<String> = conn.prepare(
            "SELECT id FROM memory WHERE status = 'faded'
             AND COALESCE(quality_score, 0.5) < ?1
             AND access_count = 0"
        ).and_then(|mut stmt| {
            stmt.query_map(rusqlite::params![min_quality], |row| row.get(0))?.collect()
        }).unwrap_or_default();

        for id in ids.iter().take(config.batch_limit) {
            let log_id = ulid::Ulid::new().to_string();
            if let Err(e) = conn.execute(
                "INSERT INTO healing_log (id, action, old_memory_id, reason, created_at)
                 VALUES (?1, 'auto_faded', ?2, ?3, ?4)",
                rusqlite::params![log_id, id,
                    format!("Stale: quality < {min_quality}, 0 accesses, > {days} days old"),
                    now],
            ) {
                eprintln!("[healing] healing_log insert failed: {e}");
            }
        }
    }

    faded
}

/// Phase 22: Natural selection — decay unused memories' quality, boost accessed ones.
pub fn apply_quality_pressure(conn: &Connection, config: &crate::config::HealingConfig) -> usize {
    if !config.enabled { return 0; }

    let decay = config.quality_decay_per_cycle;
    let boost = config.quality_boost_per_access;

    // Decay: reduce quality for unaccessed active memories (floor at 0.0)
    let decayed: usize = conn.execute(
        "UPDATE memory SET quality_score = MAX(0.0, COALESCE(quality_score, 0.5) - ?1)
         WHERE status = 'active' AND access_count = 0 AND COALESCE(quality_score, 0.5) > 0.0",
        rusqlite::params![decay],
    ).unwrap_or(0);

    // Boost: increase quality for recently accessed active memories (cap at 1.0)
    let boosted: usize = conn.execute(
        "UPDATE memory SET quality_score = MIN(1.0, COALESCE(quality_score, 0.5) + ?1)
         WHERE status = 'active' AND access_count > 0
         AND accessed_at > datetime('now', '-1 day')",
        rusqlite::params![boost],
    ).unwrap_or(0);

    decayed + boosted
}

pub async fn run_consolidator(
    state: Arc<Mutex<crate::server::handler::DaemonState>>,
    mut shutdown_rx: watch::Receiver<bool>,
    interval_secs: u64,
) {
    let interval = Duration::from_secs(interval_secs);
    eprintln!("[consolidator] started, interval = {:?}", interval);

    loop {
        tokio::select! {
            _ = tokio::time::sleep(interval) => {
                // Clone event sender before any phase
                let event_tx = {
                    let locked = state.lock().await;
                    locked.events.clone()
                };

                // Run all 15 phases (acquires conn from state)
                let stats = {
                    let consol_config = crate::config::load_config().consolidation.validated();
                    let locked = state.lock().await;
                    run_all_phases(&locked.conn, &consol_config)
                };

                // Emit consolidation event with stats
                events::emit(&event_tx, "consolidation", serde_json::json!({
                    "exact_dedup": stats.exact_dedup,
                    "semantic_dedup": stats.semantic_dedup,
                    "linked": stats.linked,
                    "faded": stats.faded,
                    "promoted": stats.promoted,
                    "reconsolidated": stats.reconsolidated,
                    "embedding_merged": stats.embedding_merged,
                    "strengthened": stats.strengthened,
                    "contradictions": stats.contradictions,
                    "entities_detected": stats.entities_detected,
                    "synthesized": stats.synthesized,
                    "gaps_detected": stats.gaps_detected,
                    "reweaved": stats.reweaved,
                    "scored": stats.scored,
                }));

                // Emit contradiction_detected event if any contradictions were found
                if stats.contradictions > 0 {
                    events::emit(&event_tx, "contradiction_detected", serde_json::json!({
                        "count": stats.contradictions,
                    }));
                }
            }
            _ = shutdown_rx.changed() => {
                eprintln!("[consolidator] shutting down");
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_run_all_phases_returns_stats() {
        // Initialize sqlite-vec extension before opening connection
        crate::db::vec::init_sqlite_vec();
        let conn = Connection::open_in_memory().unwrap();
        crate::db::schema::create_schema(&conn).unwrap();

        // On an empty DB, all stats should be 0
        let config = crate::config::ConsolidationConfig::default();
        let stats = run_all_phases(&conn, &config);
        assert_eq!(stats.exact_dedup, 0);
        assert_eq!(stats.semantic_dedup, 0);
        assert_eq!(stats.linked, 0);
        assert_eq!(stats.faded, 0);
        assert_eq!(stats.promoted, 0);
        assert_eq!(stats.reconsolidated, 0);
        assert_eq!(stats.embedding_merged, 0);
        assert_eq!(stats.strengthened, 0);
        assert_eq!(stats.contradictions, 0);
    }

    #[test]
    fn test_reweave_memories() {
        crate::db::vec::init_sqlite_vec();
        let conn = Connection::open_in_memory().unwrap();
        crate::db::schema::create_schema(&conn).unwrap();

        // Create an older memory with tags
        let older = Memory::new(MemoryType::Decision, "Use JWT auth", "We chose JWT for authentication")
            .with_tags(vec!["auth".to_string(), "security".to_string(), "jwt".to_string()]);
        ops::remember(&conn, &older).unwrap();

        // Create a newer memory with shared tags (same project, same type)
        // Need a slight delay in created_at to ensure ordering
        let newer = Memory::new(MemoryType::Decision, "JWT rotation policy", "Rotate JWT tokens every 24h")
            .with_tags(vec!["auth".to_string(), "security".to_string(), "rotation".to_string()]);
        // Manually set a later created_at
        conn.execute(
            "INSERT INTO memory (id, memory_type, title, content, confidence, status, tags, created_at, accessed_at, project)
             VALUES (?1, 'decision', 'JWT rotation policy', 'Rotate JWT tokens every 24h', 0.9, 'active',
                     '[\"auth\",\"security\",\"rotation\"]', datetime('now', '+1 second'), datetime('now'), NULL)",
            rusqlite::params![newer.id],
        ).unwrap();

        let count = reweave_memories(&conn, 200, 50);
        assert_eq!(count, 1, "should reweave 1 pair");

        // Verify older memory was enriched
        let content: String = conn.query_row(
            "SELECT content FROM memory WHERE id = ?1",
            rusqlite::params![older.id],
            |row| row.get(0),
        ).unwrap();
        assert!(content.contains("[Update]:"), "older memory should contain [Update] marker");
        assert!(content.contains("Rotate JWT tokens every 24h"), "older memory should contain newer content");

        // Verify newer memory was marked as merged
        let status: String = conn.query_row(
            "SELECT status FROM memory WHERE id = ?1",
            rusqlite::params![newer.id],
            |row| row.get(0),
        ).unwrap();
        assert_eq!(status, "merged", "newer memory should be marked as merged");
    }

    #[test]
    fn test_reweave_different_types_skipped() {
        crate::db::vec::init_sqlite_vec();
        let conn = Connection::open_in_memory().unwrap();
        crate::db::schema::create_schema(&conn).unwrap();

        // Create a decision memory
        let decision = Memory::new(MemoryType::Decision, "Use JWT auth", "JWT for authentication")
            .with_tags(vec!["auth".to_string(), "security".to_string()]);
        ops::remember(&conn, &decision).unwrap();

        // Create a lesson memory with same tags — different type should NOT reweave
        conn.execute(
            "INSERT INTO memory (id, memory_type, title, content, confidence, status, tags, created_at, accessed_at, project)
             VALUES ('lesson-1', 'lesson', 'Auth lesson', 'Learned about auth', 0.9, 'active',
                     '[\"auth\",\"security\"]', datetime('now', '+1 second'), datetime('now'), NULL)",
            [],
        ).unwrap();

        let count = reweave_memories(&conn, 200, 50);
        assert_eq!(count, 0, "should not reweave memories of different types");
    }

    #[test]
    fn test_quality_score_computation() {
        crate::db::vec::init_sqlite_vec();
        let conn = Connection::open_in_memory().unwrap();
        crate::db::schema::create_schema(&conn).unwrap();

        // Create a memory with known parameters
        conn.execute(
            "INSERT INTO memory (id, memory_type, title, content, confidence, status, tags,
                                 created_at, accessed_at, access_count, activation_level, project)
             VALUES ('qs-1', 'decision', 'Test quality', ?1, 0.9, 'active', '[]',
                     datetime('now'), datetime('now'), 5, 0.5, NULL)",
            rusqlite::params!["A".repeat(200)], // content_len = 200 -> completeness = 1.0
        ).unwrap();

        let count = score_memory_quality(&conn, 200);
        assert_eq!(count, 1, "should score 1 memory");

        let score: f64 = conn.query_row(
            "SELECT quality_score FROM memory WHERE id = 'qs-1'",
            [],
            |row| row.get(0),
        ).unwrap();

        // freshness: created today = 1.0
        // utility: 5/10 = 0.5
        // completeness: 200/200 = 1.0
        // activation: 0.5
        // expected = 1.0*0.3 + 0.5*0.3 + 1.0*0.2 + 0.5*0.2 = 0.3 + 0.15 + 0.2 + 0.1 = 0.75
        assert!((score - 0.75).abs() < 0.05, "score should be ~0.75, got {}", score);
    }

    #[test]
    fn test_quality_score_fresh_vs_old() {
        crate::db::vec::init_sqlite_vec();
        let conn = Connection::open_in_memory().unwrap();
        crate::db::schema::create_schema(&conn).unwrap();

        // Fresh memory — created now
        conn.execute(
            "INSERT INTO memory (id, memory_type, title, content, confidence, status, tags,
                                 created_at, accessed_at, access_count, activation_level, project)
             VALUES ('fresh-1', 'decision', 'Fresh memory', 'Some content here', 0.9, 'active', '[]',
                     datetime('now'), datetime('now'), 0, 0.0, NULL)",
            [],
        ).unwrap();

        // Old memory — created 70 days ago (10 weeks = freshness decayed to 0.1)
        conn.execute(
            "INSERT INTO memory (id, memory_type, title, content, confidence, status, tags,
                                 created_at, accessed_at, access_count, activation_level, project)
             VALUES ('old-1', 'decision', 'Old memory', 'Some content here', 0.9, 'active', '[]',
                     datetime('now', '-70 days'), datetime('now'), 0, 0.0, NULL)",
            [],
        ).unwrap();

        score_memory_quality(&conn, 200);

        let fresh_score: f64 = conn.query_row(
            "SELECT quality_score FROM memory WHERE id = 'fresh-1'",
            [],
            |row| row.get(0),
        ).unwrap();
        let old_score: f64 = conn.query_row(
            "SELECT quality_score FROM memory WHERE id = 'old-1'",
            [],
            |row| row.get(0),
        ).unwrap();

        assert!(fresh_score > old_score, "fresh memory score ({}) should be higher than old ({})", fresh_score, old_score);
    }

    /// Simple deterministic text embedding for tests.
    /// Creates a 768-dim vector from word hashes — same words = similar vectors.
    fn simple_text_embedding(text: &str) -> Vec<f32> {
        let mut emb = vec![0.0f32; 768];
        for word in text.to_lowercase().split_whitespace() {
            let hash = word.bytes().fold(0u64, |acc, b| acc.wrapping_mul(31).wrapping_add(b as u64));
            let idx = (hash % 768) as usize;
            emb[idx] += 1.0;
        }
        let norm: f32 = emb.iter().map(|x| x * x).sum::<f32>().sqrt();
        if norm > 0.0 { emb.iter_mut().for_each(|x| *x /= norm); }
        emb
    }

    #[test]
    fn test_heal_topic_supersedes_same_subject_different_content() {
        crate::db::vec::init_sqlite_vec();
        let conn = Connection::open_in_memory().unwrap();
        crate::db::schema::create_schema(&conn).unwrap();

        // Create two decisions about storage — same topic, different content
        // Texts crafted so cosine similarity > 0.8 and word overlap in [0.3, 0.7)
        let old = Memory::new(
            MemoryType::Decision,
            "Use SurrealDB for the primary storage backend",
            "We evaluated options and chose SurrealDB for the primary storage backend due to its graph query capabilities",
        );
        ops::remember(&conn, &old).unwrap();
        // Backdate old memory by 10 days
        conn.execute(
            "UPDATE memory SET created_at = datetime('now', '-10 days') WHERE id = ?1",
            rusqlite::params![old.id],
        ).unwrap();

        let new_mem = Memory::new(
            MemoryType::Decision,
            "Use SQLite for the primary storage backend",
            "We evaluated options and chose SQLite for the primary storage backend due to its simplicity",
        );
        ops::remember(&conn, &new_mem).unwrap();

        // Store embeddings for both (embedding text = title + content)
        let old_emb = simple_text_embedding(
            "Use SurrealDB for the primary storage backend We evaluated options and chose SurrealDB for the primary storage backend due to its graph query capabilities",
        );
        let new_emb = simple_text_embedding(
            "Use SQLite for the primary storage backend We evaluated options and chose SQLite for the primary storage backend due to its simplicity",
        );
        crate::db::vec::store_embedding(&conn, &old.id, &old_emb).unwrap();
        crate::db::vec::store_embedding(&conn, &new_mem.id, &new_emb).unwrap();

        let config = crate::config::HealingConfig::default();
        let stats = heal_topic_supersedes(&conn, &config);

        assert!(stats.candidates_found > 0, "should find candidates");
        assert_eq!(stats.topic_superseded, 1, "should supersede the old decision");

        // Verify old memory is superseded
        let status: String = conn.query_row(
            "SELECT status FROM memory WHERE id = ?1",
            rusqlite::params![old.id],
            |row| row.get(0),
        ).unwrap();
        assert_eq!(status, "superseded", "old memory should be superseded");

        // Verify healing_log entry
        let log_count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM healing_log WHERE action = 'auto_superseded'",
            [],
            |row| row.get(0),
        ).unwrap();
        assert_eq!(log_count, 1, "should have 1 healing log entry");
    }

    #[test]
    fn test_heal_topic_supersedes_skips_high_overlap() {
        crate::db::vec::init_sqlite_vec();
        let conn = Connection::open_in_memory().unwrap();
        crate::db::schema::create_schema(&conn).unwrap();

        // Two nearly identical decisions — high overlap (>= 0.7), dedup territory
        // Use different IDs but nearly identical text so overlap >= 0.7
        let old = Memory::new(
            MemoryType::Decision,
            "Use SQLite for storage backend",
            "We chose SQLite for the storage backend persistence layer",
        );
        ops::remember(&conn, &old).unwrap();
        conn.execute(
            "UPDATE memory SET created_at = datetime('now', '-10 days') WHERE id = ?1",
            rusqlite::params![old.id],
        ).unwrap();

        // Insert new directly with raw SQL to bypass title dedup in remember()
        let new_id = ulid::Ulid::new().to_string();
        conn.execute(
            "INSERT INTO memory (id, memory_type, title, content, confidence, status, tags, created_at, accessed_at, project)
             VALUES (?1, 'decision', 'Use SQLite for storage backend layer', 'We chose SQLite for the storage backend persistence layer', 0.9, 'active', '[]', datetime('now'), datetime('now'), NULL)",
            rusqlite::params![new_id],
        ).unwrap();

        // Use identical embeddings (cosine distance = 0, similarity = 1.0)
        let emb = simple_text_embedding("Use SQLite for storage backend We chose SQLite for the storage backend persistence layer");
        crate::db::vec::store_embedding(&conn, &old.id, &emb).unwrap();
        crate::db::vec::store_embedding(&conn, &new_id, &emb).unwrap();

        let config = crate::config::HealingConfig::default();
        let stats = heal_topic_supersedes(&conn, &config);

        assert_eq!(stats.topic_superseded, 0, "should NOT supersede nearly identical memories (dedup handles)");
    }

    #[test]
    fn test_heal_topic_supersedes_skips_different_type() {
        crate::db::vec::init_sqlite_vec();
        let conn = Connection::open_in_memory().unwrap();
        crate::db::schema::create_schema(&conn).unwrap();

        // One decision and one lesson with similar embeddings but different types
        let decision = Memory::new(
            MemoryType::Decision,
            "Use SQLite for the primary storage backend",
            "We evaluated options and chose SQLite for the primary storage backend due to its simplicity",
        );
        ops::remember(&conn, &decision).unwrap();
        conn.execute(
            "UPDATE memory SET created_at = datetime('now', '-10 days') WHERE id = ?1",
            rusqlite::params![decision.id],
        ).unwrap();

        let lesson = Memory::new(
            MemoryType::Lesson,
            "SQLite primary storage backend lesson",
            "We evaluated options and learned SQLite for the primary storage backend due to its simplicity",
        );
        ops::remember(&conn, &lesson).unwrap();

        // Use identical embeddings so they are KNN neighbors
        let emb = simple_text_embedding("SQLite primary storage backend evaluated options simplicity");
        crate::db::vec::store_embedding(&conn, &decision.id, &emb).unwrap();
        crate::db::vec::store_embedding(&conn, &lesson.id, &emb).unwrap();

        let config = crate::config::HealingConfig::default();
        let stats = heal_topic_supersedes(&conn, &config);

        assert_eq!(stats.topic_superseded, 0, "should NOT supersede across different types");
    }

    #[test]
    fn test_heal_topic_supersedes_newer_wins() {
        crate::db::vec::init_sqlite_vec();
        let conn = Connection::open_in_memory().unwrap();
        crate::db::schema::create_schema(&conn).unwrap();

        // Old decision backdated 30 days
        let old = Memory::new(
            MemoryType::Decision,
            "Use SurrealDB for the primary storage backend",
            "We evaluated options and chose SurrealDB for the primary storage backend due to its graph query capabilities",
        );
        ops::remember(&conn, &old).unwrap();
        conn.execute(
            "UPDATE memory SET created_at = datetime('now', '-30 days') WHERE id = ?1",
            rusqlite::params![old.id],
        ).unwrap();

        // New decision (now)
        let new_mem = Memory::new(
            MemoryType::Decision,
            "Use SQLite for the primary storage backend",
            "We evaluated options and chose SQLite for the primary storage backend due to its simplicity",
        );
        ops::remember(&conn, &new_mem).unwrap();

        let old_emb = simple_text_embedding(
            "Use SurrealDB for the primary storage backend We evaluated options and chose SurrealDB for the primary storage backend due to its graph query capabilities",
        );
        let new_emb = simple_text_embedding(
            "Use SQLite for the primary storage backend We evaluated options and chose SQLite for the primary storage backend due to its simplicity",
        );
        crate::db::vec::store_embedding(&conn, &old.id, &old_emb).unwrap();
        crate::db::vec::store_embedding(&conn, &new_mem.id, &new_emb).unwrap();

        let config = crate::config::HealingConfig::default();
        let stats = heal_topic_supersedes(&conn, &config);

        assert!(stats.topic_superseded >= 1, "should supersede at least one memory");

        // Verify the OLD one is superseded, not the new one
        let old_status: String = conn.query_row(
            "SELECT status FROM memory WHERE id = ?1",
            rusqlite::params![old.id],
            |row| row.get(0),
        ).unwrap();
        assert_eq!(old_status, "superseded", "OLD memory should be superseded");

        let new_status: String = conn.query_row(
            "SELECT status FROM memory WHERE id = ?1",
            rusqlite::params![new_mem.id],
            |row| row.get(0),
        ).unwrap();
        assert_eq!(new_status, "active", "NEW memory should remain active");

        // Verify superseded_by points to new memory
        let superseded_by: Option<String> = conn.query_row(
            "SELECT superseded_by FROM memory WHERE id = ?1",
            rusqlite::params![old.id],
            |row| row.get(0),
        ).unwrap();
        assert_eq!(superseded_by, Some(new_mem.id.clone()), "superseded_by should point to new memory");
    }

    #[test]
    fn test_heal_session_staleness_fades_old_unaccessed() {
        crate::db::vec::init_sqlite_vec();
        let conn = Connection::open_in_memory().unwrap();
        crate::db::schema::create_schema(&conn).unwrap();

        let m = Memory::new(MemoryType::Decision, "Ancient unused decision", "Very old")
            .with_confidence(0.5);
        ops::remember(&conn, &m).unwrap();
        // Backdate + set low quality + zero access
        conn.execute(
            "UPDATE memory SET created_at = datetime('now', '-14 days'), quality_score = 0.1, access_count = 0 WHERE id = ?1",
            rusqlite::params![m.id],
        ).unwrap();

        let config = crate::config::HealingConfig::default();
        let faded = heal_session_staleness(&conn, &config);
        assert!(faded > 0, "old unaccessed low-quality memory should be faded");

        let status: String = conn.query_row(
            "SELECT status FROM memory WHERE id = ?1", rusqlite::params![m.id], |row| row.get(0),
        ).unwrap();
        assert_eq!(status, "faded");

        // Verify healing_log entry
        let log_count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM healing_log WHERE action = 'auto_faded'", [], |row| row.get(0),
        ).unwrap();
        assert!(log_count > 0, "should have healing_log entry for faded memory");
    }

    #[test]
    fn test_heal_session_staleness_preserves_accessed() {
        crate::db::vec::init_sqlite_vec();
        let conn = Connection::open_in_memory().unwrap();
        crate::db::schema::create_schema(&conn).unwrap();

        let m = Memory::new(MemoryType::Decision, "Old but accessed decision", "Still useful")
            .with_confidence(0.9);
        ops::remember(&conn, &m).unwrap();
        // Backdate but give it access count
        conn.execute(
            "UPDATE memory SET created_at = datetime('now', '-14 days'), quality_score = 0.1, access_count = 5 WHERE id = ?1",
            rusqlite::params![m.id],
        ).unwrap();

        let config = crate::config::HealingConfig::default();
        let faded = heal_session_staleness(&conn, &config);
        assert_eq!(faded, 0, "accessed memory should not be faded regardless of age/quality");

        let status: String = conn.query_row(
            "SELECT status FROM memory WHERE id = ?1", rusqlite::params![m.id], |row| row.get(0),
        ).unwrap();
        assert_eq!(status, "active");
    }

    #[test]
    fn test_quality_pressure_decays_unused() {
        crate::db::vec::init_sqlite_vec();
        let conn = Connection::open_in_memory().unwrap();
        crate::db::schema::create_schema(&conn).unwrap();

        let m = Memory::new(MemoryType::Decision, "Unused decision for decay test", "Never accessed")
            .with_confidence(0.9);
        ops::remember(&conn, &m).unwrap();
        conn.execute(
            "UPDATE memory SET quality_score = 0.5, access_count = 0 WHERE id = ?1",
            rusqlite::params![m.id],
        ).unwrap();

        let config = crate::config::HealingConfig::default();
        let adjusted = apply_quality_pressure(&conn, &config);
        assert!(adjusted > 0, "should adjust at least one memory");

        let quality: f64 = conn.query_row(
            "SELECT quality_score FROM memory WHERE id = ?1", rusqlite::params![m.id], |row| row.get(0),
        ).unwrap();
        assert!(quality < 0.5, "quality should have decayed from 0.5");
        assert!(quality >= 0.35, "quality should decay by ~0.1, not more");
    }

    #[test]
    fn test_run_all_phases_includes_healing_stats() {
        crate::db::vec::init_sqlite_vec();
        let conn = Connection::open_in_memory().unwrap();
        crate::db::schema::create_schema(&conn).unwrap();

        // Create a stale memory that should be faded by Phase 21.
        // Must survive Phase 4 decay (confidence * exp(-0.03 * days) >= 0.1)
        // but get low quality from Phase 15 scoring (< staleness_min_quality = 0.2).
        // At 30 days: Phase 4 effective = 0.5 * exp(-0.9) ≈ 0.20 (survives),
        // Phase 15 quality ≈ 0.57 * 0.3 + 0 + 0.015 + 0 ≈ 0.19 (below 0.2 threshold).
        let m = Memory::new(MemoryType::Decision, "Very stale memory for consolidation test", "Ancient content")
            .with_confidence(0.5);
        ops::remember(&conn, &m).unwrap();
        conn.execute(
            "UPDATE memory SET created_at = datetime('now', '-30 days'), quality_score = 0.1, access_count = 0 WHERE id = ?1",
            rusqlite::params![m.id],
        ).unwrap();

        let config = crate::config::ConsolidationConfig::default();
        let stats = run_all_phases(&conn, &config);

        // Healing phases should have run
        assert!(stats.healed_faded > 0, "Phase 21 should have faded the stale memory");
    }
}
