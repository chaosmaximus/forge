// workers/consolidator.rs — Memory consolidator (22 phases)
//
// Runs every 30 minutes (configurable). 22 phases in 5 groups:
//
// Core (1-10): exact dedup, semantic dedup, link related, confidence decay,
//   episodic→semantic promotion, reconsolidation, embedding merge,
//   edge strengthening, contradiction detection (9a: valence-based, 9b: content-based),
//   activation decay.
// Knowledge Intelligence (11-15): entity detection, contradiction synthesis,
//   knowledge gap detection, memory reweave, quality scoring.
// Additional (16-18): portability classification, protocol extraction, anti-pattern tagging.
// Notifications (19a-d): protocol suggestions, contradiction alerts, quality decline, meeting timeout.
// Healing (20-22): topic supersede, session staleness fade, quality pressure.
//
// Memories that fall below 0.1 effective confidence are marked "faded".

use crate::db::ops;
use crate::events;
use forge_core::types::manas::{Perception, PerceptionKind, Severity};
use forge_core::types::memory::{Memory, MemoryType};
use rusqlite::Connection;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{watch, Mutex};

// Interval is now configurable via ForgeConfig.workers.consolidation_interval_secs
// (default: 1800 = 30 minutes)

/// Registry of phases the consolidator executes, in execution order.
/// Used by `Request::ProbePhase` to answer master-design assertion 9
/// (Phase 23 executes after Phase 17).
///
/// `fn_name` matches the Rust function called for that phase.
/// `phase_number` is the 1-based doc numbering ("Phase N") — independent
/// of array position. 2A-4c2 only requires these two entries; future
/// assertions can extend the array without breaking anything.
#[cfg(any(test, feature = "bench"))]
pub const PHASE_ORDER: &[(&str, usize)] = &[
    ("extract_protocols", 17),
    ("infer_skills_from_behavior", 23),
];

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
    pub skills_inferred: usize,
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
pub fn run_all_phases(
    conn: &Connection,
    config: &crate::config::ConsolidationConfig,
    metrics: Option<&crate::server::metrics::ForgeMetrics>,
) -> ConsolidationStats {
    use crate::workers::instrumentation::{record, PhaseOutcome};

    let mut stats = ConsolidationStats::default();
    let run_id = ulid::Ulid::new().to_string();
    let _pass_span = tracing::info_span!("consolidate_pass", run_id = %run_id).entered();

    // Phase 1: Exact dedup (fast)
    {
        let _span = tracing::info_span!("phase_1_dedup_memories").entered();
        let t0 = std::time::Instant::now();
        let (output, err) = match ops::dedup_memories(conn) {
            Ok(removed) => {
                stats.exact_dedup = removed;
                if removed > 0 {
                    tracing::info!(removed, "phase_1: dedup removed duplicate memories");
                }
                (removed as u64, 0u64)
            }
            Err(e) => {
                tracing::error!(error = %e, "phase_1: dedup error");
                (0, 1)
            }
        };
        record(
            conn,
            metrics,
            &PhaseOutcome {
                phase: "phase_1_dedup_memories",
                run_id: &run_id,
                correlation_id: &run_id,
                trace_id: None,
                output_count: output,
                error_count: err,
                duration_ms: t0.elapsed().as_millis() as u64,
                extra: serde_json::json!({}),
            },
        );
    }

    // Phase 2: Semantic dedup (slow O(n^2), bounded by batch_limit)
    {
        let _span = tracing::info_span!("phase_2_semantic_dedup").entered();
        let t0 = std::time::Instant::now();
        let (output, err) = match ops::semantic_dedup(conn, config.batch_limit) {
            Ok(merged) => {
                stats.semantic_dedup = merged;
                if merged > 0 {
                    tracing::info!(merged, "phase_2: semantic dedup merged near-duplicates");
                }
                (merged as u64, 0u64)
            }
            Err(e) => {
                tracing::error!(error = %e, "phase_2: semantic dedup error");
                (0, 1)
            }
        };
        record(
            conn,
            metrics,
            &PhaseOutcome {
                phase: "phase_2_semantic_dedup",
                run_id: &run_id,
                correlation_id: &run_id,
                trace_id: None,
                output_count: output,
                error_count: err,
                duration_ms: t0.elapsed().as_millis() as u64,
                extra: serde_json::json!({}),
            },
        );
    }

    // Phase 3: Link related memories (bounded by batch_limit)
    {
        let _span = tracing::info_span!("phase_3_link_memories").entered();
        let t0 = std::time::Instant::now();
        let (output, err) = match ops::link_related_memories(conn, config.batch_limit) {
            Ok(linked) => {
                stats.linked = linked;
                if linked > 0 {
                    tracing::info!(linked, "phase_3: linked related memory pairs");
                }
                (linked as u64, 0u64)
            }
            Err(e) => {
                tracing::error!(error = %e, "phase_3: link error");
                (0, 1)
            }
        };
        record(
            conn,
            metrics,
            &PhaseOutcome {
                phase: "phase_3_link_memories",
                run_id: &run_id,
                correlation_id: &run_id,
                trace_id: None,
                output_count: output,
                error_count: err,
                duration_ms: t0.elapsed().as_millis() as u64,
                extra: serde_json::json!({}),
            },
        );
    }

    // Phase 4: Decay (bounded by batch_limit). output_count = faded (NOT
    // checked + faded — per 2A-4d.1 spec §3.1a, faded ⊆ checked so summing
    // would double-count). Checked count preserved in extra for operator
    // drill-down.
    let preference_half_life_days = crate::config::load_config()
        .recall
        .validated()
        .preference_half_life_days;
    {
        let _span = tracing::info_span!("phase_4_decay_memories").entered();
        let t0 = std::time::Instant::now();
        let (output, err, checked_count) =
            match ops::decay_memories(conn, config.batch_limit, preference_half_life_days) {
                Ok((checked, faded)) => {
                    stats.faded = faded;
                    if faded > 0 {
                        tracing::info!(faded, checked, "phase_4: decay faded memories");
                    }
                    (faded as u64, 0u64, checked as u64)
                }
                Err(e) => {
                    tracing::error!(error = %e, "phase_4: decay error");
                    (0, 1, 0)
                }
            };
        record(
            conn,
            metrics,
            &PhaseOutcome {
                phase: "phase_4_decay_memories",
                run_id: &run_id,
                correlation_id: &run_id,
                trace_id: None,
                output_count: output,
                error_count: err,
                duration_ms: t0.elapsed().as_millis() as u64,
                extra: serde_json::json!({ "checked_count": checked_count }),
            },
        );
    }

    // Phase 5: Episodic -> Semantic promotion (bounded by batch_limit)
    {
        let _span = tracing::info_span!("phase_5_promote_patterns").entered();
        let t0 = std::time::Instant::now();
        let (output, err) = match ops::promote_recurring_lessons(conn, config.batch_limit) {
            Ok(promoted) => {
                stats.promoted = promoted;
                if promoted > 0 {
                    tracing::info!(promoted, "phase_5: promoted recurring lessons to patterns");
                }
                (promoted as u64, 0u64)
            }
            Err(e) => {
                tracing::error!(error = %e, "phase_5: promotion error");
                (0, 1)
            }
        };
        record(
            conn,
            metrics,
            &PhaseOutcome {
                phase: "phase_5_promote_patterns",
                run_id: &run_id,
                correlation_id: &run_id,
                trace_id: None,
                output_count: output,
                error_count: err,
                duration_ms: t0.elapsed().as_millis() as u64,
                extra: serde_json::json!({}),
            },
        );
    }

    // Phase 6: Reconsolidation — boost confidence of heavily-accessed memories.
    // output_count = candidates.len() per spec §3.1a (inner UPDATE loop may
    // swallow errors; those surface as tracing::warn!).
    {
        let _span = tracing::info_span!("phase_6_reconsolidate_contradicting").entered();
        let t0 = std::time::Instant::now();
        let (output, err) = match ops::find_reconsolidation_candidates(conn) {
            Ok(candidates) => {
                for mem in &candidates {
                    let new_confidence = (mem.confidence + 0.05).min(1.0);
                    if let Err(e) = conn.execute(
                        "UPDATE memory SET confidence = ?1 WHERE id = ?2",
                        rusqlite::params![new_confidence, mem.id],
                    ) {
                        tracing::warn!(error = %e, memory_id = %mem.id, "phase_6: reconsolidate update failed");
                    }
                }
                stats.reconsolidated = candidates.len();
                if !candidates.is_empty() {
                    tracing::info!(count = candidates.len(), "phase_6: reconsolidated memories");
                }
                (candidates.len() as u64, 0u64)
            }
            Err(e) => {
                tracing::error!(error = %e, "phase_6: reconsolidation error");
                (0, 1)
            }
        };
        record(
            conn,
            metrics,
            &PhaseOutcome {
                phase: "phase_6_reconsolidate_contradicting",
                run_id: &run_id,
                correlation_id: &run_id,
                trace_id: None,
                output_count: output,
                error_count: err,
                duration_ms: t0.elapsed().as_millis() as u64,
                extra: serde_json::json!({}),
            },
        );
    }

    // Phase 7: Embedding-based merge (sleep cycle — deep structural cleanup)
    {
        let _span = tracing::info_span!("phase_7_merge_embedding_duplicates").entered();
        let t0 = std::time::Instant::now();
        let (output, err) = match ops::embedding_merge(conn) {
            Ok(merged) => {
                stats.embedding_merged = merged;
                if merged > 0 {
                    tracing::info!(merged, "phase_7: embedding merge merged similar memories");
                }
                (merged as u64, 0u64)
            }
            Err(e) => {
                tracing::error!(error = %e, "phase_7: embedding merge error");
                (0, 1)
            }
        };
        record(
            conn,
            metrics,
            &PhaseOutcome {
                phase: "phase_7_merge_embedding_duplicates",
                run_id: &run_id,
                correlation_id: &run_id,
                trace_id: None,
                output_count: output,
                error_count: err,
                duration_ms: t0.elapsed().as_millis() as u64,
                extra: serde_json::json!({}),
            },
        );
    }

    // Phase 8: Strengthen active edges
    {
        let _span = tracing::info_span!("phase_8_strengthen_by_access").entered();
        let t0 = std::time::Instant::now();
        let (output, err) = match ops::strengthen_active_edges(conn) {
            Ok(strengthened) => {
                stats.strengthened = strengthened;
                if strengthened > 0 {
                    tracing::info!(strengthened, "phase_8: strengthened active edges");
                }
                (strengthened as u64, 0u64)
            }
            Err(e) => {
                tracing::error!(error = %e, "phase_8: edge strengthening error");
                (0, 1)
            }
        };
        record(
            conn,
            metrics,
            &PhaseOutcome {
                phase: "phase_8_strengthen_by_access",
                run_id: &run_id,
                correlation_id: &run_id,
                trace_id: None,
                output_count: output,
                error_count: err,
                duration_ms: t0.elapsed().as_millis() as u64,
                extra: serde_json::json!({}),
            },
        );
    }

    // Phase 9: Contradiction detection (two strategies — 9a valence, 9b content)
    {
        let _span = tracing::info_span!("phase_9_detect_contradictions").entered();
        let t0 = std::time::Instant::now();
        let (output, err, valence_summary) = match ops::detect_contradictions(conn) {
            Ok(found) => {
                stats.contradictions = found;
                let mut summary = String::new();
                if found > 0 {
                    tracing::info!(found, "phase_9a: valence-based contradictions detected");
                } else {
                    let valence_counts: Vec<(String, i64)> = conn
                        .prepare("SELECT valence, COUNT(*) FROM memory WHERE status = 'active' GROUP BY valence")
                        .and_then(|mut s| s.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?.collect())
                        .unwrap_or_default();
                    let joined: Vec<String> = valence_counts.iter().map(|(v, c)| format!("{v}={c}")).collect();
                    summary = if joined.is_empty() { "none".to_string() } else { joined.join(", ") };
                    tracing::info!(valence_distribution = %summary, "phase_9a: 0 valence-based contradictions");
                }
                (found as u64, 0u64, summary)
            }
            Err(e) => {
                tracing::error!(error = %e, "phase_9a: contradiction detection error");
                (0, 1, String::new())
            }
        };
        let content_contradictions = detect_content_contradictions(conn);
        stats.contradictions += content_contradictions;
        if content_contradictions > 0 {
            tracing::info!(content_contradictions, "phase_9b: content-based contradictions detected");
        }
        record(
            conn,
            metrics,
            &PhaseOutcome {
                phase: "phase_9_detect_contradictions",
                run_id: &run_id,
                correlation_id: &run_id,
                trace_id: None,
                output_count: output + content_contradictions as u64,
                error_count: err,
                duration_ms: t0.elapsed().as_millis() as u64,
                extra: serde_json::json!({ "valence_distribution": valence_summary, "content_contradictions": content_contradictions }),
            },
        );
    }

    // Phase 10: Decay activation levels (fast — single UPDATE)
    {
        let _span = tracing::info_span!("phase_10_decay_activation").entered();
        let t0 = std::time::Instant::now();
        let (output, err) = match ops::decay_activation_levels(conn) {
            Ok(n) => {
                if n > 0 {
                    tracing::info!(n, "phase_10: decayed activation levels");
                }
                (n as u64, 0u64)
            }
            Err(e) => {
                tracing::error!(error = %e, "phase_10: activation decay error");
                (0, 1)
            }
        };
        record(
            conn,
            metrics,
            &PhaseOutcome {
                phase: "phase_10_decay_activation",
                run_id: &run_id,
                correlation_id: &run_id,
                trace_id: None,
                output_count: output,
                error_count: err,
                duration_ms: t0.elapsed().as_millis() as u64,
                extra: serde_json::json!({}),
            },
        );
    }

    // Phase 11: Entity detection (Knowledge Intelligence)
    {
        let _span = tracing::info_span!("phase_11_entity_detection").entered();
        let t0 = std::time::Instant::now();
        let (output, err) = match crate::db::manas::detect_entities(conn) {
            Ok(detected) => {
                stats.entities_detected = detected;
                if detected > 0 {
                    tracing::info!(detected, "phase_11: entity detection");
                }
                (detected as u64, 0u64)
            }
            Err(e) => {
                tracing::error!(error = %e, "phase_11: entity detection error");
                (0, 1)
            }
        };
        record(
            conn,
            metrics,
            &PhaseOutcome {
                phase: "phase_11_entity_detection",
                run_id: &run_id,
                correlation_id: &run_id,
                trace_id: None,
                output_count: output,
                error_count: err,
                duration_ms: t0.elapsed().as_millis() as u64,
                extra: serde_json::json!({}),
            },
        );
    }

    // Phase 12: Contradiction synthesis — resolve detected contradictions
    {
        let _span = tracing::info_span!("phase_12_synthesize_contradictions").entered();
        let t0 = std::time::Instant::now();
        let synthesized = synthesize_contradictions(conn, config.batch_limit);
        stats.synthesized = synthesized;
        if synthesized > 0 {
            tracing::info!(synthesized, "phase_12: contradiction resolutions synthesized");
        }
        record(
            conn,
            metrics,
            &PhaseOutcome {
                phase: "phase_12_synthesize_contradictions",
                run_id: &run_id,
                correlation_id: &run_id,
                trace_id: None,
                output_count: synthesized as u64,
                error_count: 0,
                duration_ms: t0.elapsed().as_millis() as u64,
                extra: serde_json::json!({}),
            },
        );
    }

    // Phase 13: Knowledge gap detection — surface concepts without entities
    {
        let _span = tracing::info_span!("phase_13_detect_gaps").entered();
        let t0 = std::time::Instant::now();
        let gaps = detect_and_surface_gaps(conn);
        stats.gaps_detected = gaps;
        if gaps > 0 {
            tracing::info!(gaps, "phase_13: knowledge gaps detected");
        }
        record(
            conn,
            metrics,
            &PhaseOutcome {
                phase: "phase_13_detect_gaps",
                run_id: &run_id,
                correlation_id: &run_id,
                trace_id: None,
                output_count: gaps as u64,
                error_count: 0,
                duration_ms: t0.elapsed().as_millis() as u64,
                extra: serde_json::json!({}),
            },
        );
    }

    // Phase 14: Memory reweave — enrich older memories with newer context sharing tags
    {
        let _span = tracing::info_span!("phase_14_reweave_memories").entered();
        let t0 = std::time::Instant::now();
        let reweaved = reweave_memories(conn, config.batch_limit, config.reweave_limit);
        stats.reweaved = reweaved;
        if reweaved > 0 {
            tracing::info!(reweaved, "phase_14: memory pairs reweaved");
        }
        record(
            conn,
            metrics,
            &PhaseOutcome {
                phase: "phase_14_reweave_memories",
                run_id: &run_id,
                correlation_id: &run_id,
                trace_id: None,
                output_count: reweaved as u64,
                error_count: 0,
                duration_ms: t0.elapsed().as_millis() as u64,
                extra: serde_json::json!({}),
            },
        );
    }

    // Phase 15: Quality scoring — compute quality scores for active memories
    {
        let _span = tracing::info_span!("phase_15_quality_scoring").entered();
        let t0 = std::time::Instant::now();
        let scored = score_memory_quality(conn, config.batch_limit);
        stats.scored = scored;
        if scored > 0 {
            tracing::info!(scored, "phase_15: memories scored");
        }
        record(
            conn,
            metrics,
            &PhaseOutcome {
                phase: "phase_15_quality_scoring",
                run_id: &run_id,
                correlation_id: &run_id,
                trace_id: None,
                output_count: scored as u64,
                error_count: 0,
                duration_ms: t0.elapsed().as_millis() as u64,
                extra: serde_json::json!({}),
            },
        );
    }

    // Phase 16: Portability classification — classify unknown memories
    {
        let _span = tracing::info_span!("phase_16_portability_classification").entered();
        let t0 = std::time::Instant::now();
        let (output, err) = match ops::classify_portability(conn, config.batch_limit) {
            Ok(classified) => {
                if classified > 0 {
                    tracing::info!(classified, "phase_16: portability classified");
                }
                (classified as u64, 0u64)
            }
            Err(e) => {
                tracing::error!(error = %e, "phase_16: portability classification failed");
                (0, 1)
            }
        };
        record(
            conn,
            metrics,
            &PhaseOutcome {
                phase: "phase_16_portability_classification",
                run_id: &run_id,
                correlation_id: &run_id,
                trace_id: None,
                output_count: output,
                error_count: err,
                duration_ms: t0.elapsed().as_millis() as u64,
                extra: serde_json::json!({}),
            },
        );
    }

    // Phase 17: Protocol extraction — promote recurring process patterns to protocols
    let protocols;
    {
        let _span = tracing::info_span!("phase_17_extract_protocols").entered();
        let t0 = std::time::Instant::now();
        protocols = extract_protocols(conn, config.batch_limit);
        stats.protocols_extracted = protocols;
        if protocols > 0 {
            tracing::info!(protocols, "phase_17: protocols extracted");
        }
        record(
            conn,
            metrics,
            &PhaseOutcome {
                phase: "phase_17_extract_protocols",
                run_id: &run_id,
                correlation_id: &run_id,
                trace_id: None,
                output_count: protocols as u64,
                error_count: 0,
                duration_ms: t0.elapsed().as_millis() as u64,
                extra: serde_json::json!({}),
            },
        );
    }

    // Phase 23: Behavioral skill inference — elevate recurring clean tool-use
    // patterns from session_tool_call to the skill table. Physically runs
    // between Phase 17 and Phase 18 per 2A-4c2 design.
    {
        let _span = tracing::info_span!("phase_23_infer_skills_from_behavior").entered();
        let t0 = std::time::Instant::now();
        let skills_inferred = infer_skills_from_behavior(
            conn,
            config.skill_inference_min_sessions,
            config.skill_inference_window_days,
        );
        stats.skills_inferred = skills_inferred;
        if skills_inferred > 0 {
            tracing::info!(
                skills_inferred,
                "phase_23: inferred skills from tool-use patterns"
            );
        }
        record(
            conn,
            metrics,
            &PhaseOutcome {
                phase: "phase_23_infer_skills_from_behavior",
                run_id: &run_id,
                correlation_id: &run_id,
                trace_id: None,
                output_count: skills_inferred as u64,
                error_count: 0,
                duration_ms: t0.elapsed().as_millis() as u64,
                extra: serde_json::json!({}),
            },
        );
    }

    // Phase 18: Anti-pattern tagging — tag lessons with negative signals
    {
        let _span = tracing::info_span!("phase_18_tag_antipatterns").entered();
        let t0 = std::time::Instant::now();
        let antipatterns = tag_antipatterns(conn, config.batch_limit);
        stats.antipatterns_tagged = antipatterns;
        if antipatterns > 0 {
            tracing::info!(antipatterns, "phase_18: anti-patterns tagged from lessons");
        }
        record(
            conn,
            metrics,
            &PhaseOutcome {
                phase: "phase_18_tag_antipatterns",
                run_id: &run_id,
                correlation_id: &run_id,
                trace_id: None,
                output_count: antipatterns as u64,
                error_count: 0,
                duration_ms: t0.elapsed().as_millis() as u64,
                extra: serde_json::json!({}),
            },
        );
    }

    // Phase 19: Generate notifications from consolidation findings
    let _span_19 = tracing::info_span!("phase_19_emit_notifications").entered();
    let t0_19 = std::time::Instant::now();
    let mut notifs_generated = 0;

    // 19a: Protocol suggestion notifications
    if protocols > 0
        && !crate::notifications::check_throttle(conn, "protocol_suggestion", "local", 3600)
            .unwrap_or(true)
    {
        if let Err(e) = crate::notifications::NotificationBuilder::new(
            "confirmation",
            "medium",
            &format!("Forge extracted {protocols} new protocol(s) from behavior patterns"),
            "Review the new protocols with: forge-next recall --type protocol. Approve or dismiss.",
            "consolidator",
        )
        .topic("protocol_suggestion")
        .action("review_protocols", "{}")
        .build(conn)
        {
            eprintln!("[consolidator] notification failed: {e}");
        }
        notifs_generated += 1;
    }

    // 19b: Contradiction notifications
    if stats.contradictions > 0
        && !crate::notifications::check_throttle(conn, "contradiction", "local", 1800)
            .unwrap_or(true)
    {
        let _ = crate::notifications::NotificationBuilder::new(
            "insight",
            "high",
            &format!(
                "{} contradiction(s) detected between active decisions",
                stats.contradictions
            ),
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
                [],
                |row| row.get(0),
            )
            .unwrap_or(0.5);

        if avg_quality < 0.3
            && !crate::notifications::check_throttle(conn, "quality_decline", "local", 86400)
                .unwrap_or(true)
        {
            if let Err(e) = crate::notifications::NotificationBuilder::new(
                "insight", "medium",
                "Memory quality declining",
                &format!("Average quality score for recent memories is {avg_quality:.2}. Consider reviewing and cleaning up low-quality entries."),
                "consolidator",
            )
            .topic("quality_decline")
            .build(conn) { eprintln!("[consolidator] notification failed: {e}"); }
            notifs_generated += 1;
        } else if avg_quality >= 0.3 {
            // Auto-dismiss existing quality decline notifications — condition resolved
            let _ = conn.execute(
                "UPDATE notification SET status = 'dismissed'
                 WHERE category = 'insight' AND title LIKE '%quality%' AND status = 'pending'",
                [],
            );
        }
    }

    // 19d: Meeting timeout detection
    {
        let timeout_secs = crate::config::load_config().meeting.timeout_secs;
        let timeout_modifier = format!("-{timeout_secs} seconds");
        let timed_out: Vec<(String, String)> = conn
            .prepare(
                "SELECT id, topic FROM meeting
                 WHERE status IN ('open', 'collecting')
                 AND created_at < datetime('now', ?1)",
            )
            .ok()
            .and_then(|mut stmt| {
                stmt.query_map(rusqlite::params![timeout_modifier], |row| {
                    Ok((row.get(0)?, row.get(1)?))
                })
                .ok()
                .map(|rows| rows.filter_map(|r| r.ok()).collect())
            })
            .unwrap_or_default();

        for (meeting_id, topic) in &timed_out {
            // Collect partial responses for auto-synthesis
            let responses: Vec<(String, String)> = conn.prepare(
                "SELECT session_id, COALESCE(response, '') FROM meeting_participant WHERE meeting_id = ?1 AND response IS NOT NULL ORDER BY responded_at"
            ).and_then(|mut stmt| {
                stmt.query_map(rusqlite::params![meeting_id], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })?.collect()
            }).unwrap_or_default();

            // Auto-synthesize from partial responses
            let synthesis = if responses.is_empty() {
                format!("Meeting '{topic}' timed out with no responses.")
            } else {
                let parts: Vec<String> = responses
                    .iter()
                    .map(|(sid, resp)| {
                        format!("- {}: {}", sid, resp.chars().take(200).collect::<String>())
                    })
                    .collect();
                format!(
                    "Meeting '{}' timed out with {} partial response(s):\n{}",
                    topic,
                    responses.len(),
                    parts.join("\n")
                )
            };

            // Store synthesis as a decision memory
            let decision_mem = Memory::new(
                MemoryType::Decision,
                format!("Meeting timed out: {topic}"),
                synthesis.clone(),
            )
            .with_confidence(0.6);
            if let Err(e) = ops::remember(conn, &decision_mem) {
                eprintln!(
                    "[consolidator] auto-synthesis store failed for meeting {meeting_id}: {e}"
                );
            }

            // Update meeting status + store synthesis
            if let Err(e) = conn.execute(
                "UPDATE meeting SET status = 'timed_out', synthesis = ?2 WHERE id = ?1",
                rusqlite::params![meeting_id, synthesis],
            ) {
                eprintln!("[consolidator] meeting timeout update failed: {e}");
            }

            if let Err(e) = crate::notifications::NotificationBuilder::new(
                "alert", "high",
                &format!("Meeting '{topic}' timed out — auto-synthesized"),
                &format!("Meeting {} timed out with {} response(s). Auto-synthesis stored as decision. Review: forge-next meeting transcript {}",
                    meeting_id, responses.len(), meeting_id),
                "meeting_engine",
            )
            .topic("meeting_timeout")
            .source_id(meeting_id)
            .build(conn) { eprintln!("[consolidator] notification failed: {e}"); }
            notifs_generated += 1;
        }
    }

    if notifs_generated > 0 {
        tracing::info!(notifs_generated, "phase_19: notifications generated");
    }
    record(
        conn,
        metrics,
        &PhaseOutcome {
            phase: "phase_19_emit_notifications",
            run_id: &run_id,
            correlation_id: &run_id,
            trace_id: None,
            output_count: notifs_generated as u64,
            error_count: 0,
            duration_ms: t0_19.elapsed().as_millis() as u64,
            extra: serde_json::json!({}),
        },
    );
    drop(_span_19);

    // ── Memory Self-Healing (Phases 20-22) ──

    let healing_config = crate::config::load_config().healing;

    // Phase 20: Topic-aware auto-supersede
    {
        let _span = tracing::info_span!("phase_20_auto_supersede").entered();
        let t0 = std::time::Instant::now();
        let healing_stats = heal_topic_supersedes(conn, &healing_config);
        stats.healed_superseded = healing_stats.topic_superseded;
        if healing_stats.topic_superseded > 0 {
            tracing::info!(
                superseded = healing_stats.topic_superseded,
                candidates = healing_stats.candidates_found,
                skipped = healing_stats.false_positives_skipped,
                "phase_20: topic-evolved memories auto-superseded"
            );
        }
        record(
            conn,
            metrics,
            &PhaseOutcome {
                phase: "phase_20_auto_supersede",
                run_id: &run_id,
                correlation_id: &run_id,
                trace_id: None,
                output_count: healing_stats.topic_superseded as u64,
                error_count: 0,
                duration_ms: t0.elapsed().as_millis() as u64,
                extra: serde_json::json!({
                    "candidates_found": healing_stats.candidates_found,
                    "false_positives_skipped": healing_stats.false_positives_skipped,
                }),
            },
        );
    }
    let healing_stats_topic_superseded = stats.healed_superseded;

    // Phase 21: Session staleness fade
    let healed_faded;
    {
        let _span = tracing::info_span!("phase_21_session_staleness_fade").entered();
        let t0 = std::time::Instant::now();
        healed_faded = heal_session_staleness(conn, &healing_config);
        stats.healed_faded = healed_faded;
        if healed_faded > 0 {
            tracing::info!(healed_faded, "phase_21: stale memories auto-faded");
        }
        record(
            conn,
            metrics,
            &PhaseOutcome {
                phase: "phase_21_session_staleness_fade",
                run_id: &run_id,
                correlation_id: &run_id,
                trace_id: None,
                output_count: healed_faded as u64,
                error_count: 0,
                duration_ms: t0.elapsed().as_millis() as u64,
                extra: serde_json::json!({}),
            },
        );
    }

    // Phase 22: Quality pressure (natural selection)
    {
        let _span = tracing::info_span!("phase_22_apply_quality_pressure").entered();
        let t0 = std::time::Instant::now();
        let quality_adjusted = apply_quality_pressure(conn, &healing_config);
        stats.healed_quality_adjusted = quality_adjusted;
        if quality_adjusted > 0 {
            tracing::info!(quality_adjusted, "phase_22: quality pressure applied");
        }
        record(
            conn,
            metrics,
            &PhaseOutcome {
                phase: "phase_22_apply_quality_pressure",
                run_id: &run_id,
                correlation_id: &run_id,
                trace_id: None,
                output_count: quality_adjusted as u64,
                error_count: 0,
                duration_ms: t0.elapsed().as_millis() as u64,
                extra: serde_json::json!({}),
            },
        );
    }

    // Healing notification (throttled: max once per hour). Uses healing stats
    // captured in Phase 20+21 scopes (healed_superseded via saved local,
    // healed_faded via the surrounding `let`).
    let healing_superseded = healing_stats_topic_superseded;
    if (healing_superseded > 0 || healed_faded > 0)
        && !crate::notifications::check_throttle(conn, "healing", "local", 3600).unwrap_or(true)
    {
        if let Err(e) = crate::notifications::NotificationBuilder::new(
            "insight", "medium",
            &format!("Memory healing: {healing_superseded} superseded, {healed_faded} faded"),
            &format!("Auto-superseded {healing_superseded} same-topic decisions, faded {healed_faded} stale memories. Review: forge-next healing-log"),
            "consolidator",
        )
        .topic("healing")
        .build(conn) {
            tracing::error!(error = %e, "healing notification failed");
        }
    }

    stats
}

/// Content-based contradiction detection: finds same-type active memories
/// with high title word overlap (>= 50%) but divergent content (< 30% overlap).
/// This complements `ops::detect_contradictions` which only catches
/// valence-based contradictions (positive vs negative with shared tags).
/// Most memories have valence='neutral' and intensity=0.0, so the valence-based
/// detector produces zero results in practice.
///
/// Returns number of contradiction pairs found.
pub fn detect_content_contradictions(conn: &Connection) -> usize {
    use crate::db::diagnostics::{store_diagnostic, Diagnostic};

    // Fetch active memories grouped by type — only types that can contradict
    let mut stmt = match conn.prepare(
        "SELECT id, memory_type, title, content FROM memory
         WHERE status = 'active' AND memory_type IN ('decision', 'pattern', 'protocol')
         ORDER BY memory_type, created_at DESC",
    ) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("[consolidator] content contradiction query error: {e}");
            return 0;
        }
    };

    struct Row {
        id: String,
        memory_type: String,
        title: String,
        content: String,
    }

    let rows: Vec<Row> = match stmt.query_map([], |row| {
        Ok(Row {
            id: row.get(0)?,
            memory_type: row.get(1)?,
            title: row.get(2)?,
            content: row.get(3)?,
        })
    }) {
        Ok(mapped) => mapped.filter_map(|r| r.ok()).collect(),
        Err(e) => {
            eprintln!("[consolidator] content contradiction row error: {e}");
            return 0;
        }
    };

    if rows.is_empty() {
        eprintln!("[consolidator] content contradiction: 0 candidate memories");
        return 0;
    }

    /// Extract lowercase word set from text (alphanumeric words with 3+ chars).
    fn word_set(text: &str) -> std::collections::HashSet<String> {
        text.to_lowercase()
            .split(|c: char| !c.is_alphanumeric())
            .filter(|w| w.len() >= 3)
            .map(|w| w.to_string())
            .collect()
    }

    /// Jaccard overlap between two word sets.
    fn jaccard(
        a: &std::collections::HashSet<String>,
        b: &std::collections::HashSet<String>,
    ) -> f64 {
        if a.is_empty() || b.is_empty() {
            return 0.0;
        }
        let intersection = a.intersection(b).count() as f64;
        let union = a.union(b).count() as f64;
        intersection / union
    }

    let mut found = 0usize;
    // Limit total comparisons to avoid O(n^2) blowup on large memory sets
    let max_comparisons = 5000usize;
    let mut comparisons = 0usize;

    for i in 0..rows.len() {
        if comparisons >= max_comparisons {
            break;
        }
        let a = &rows[i];
        let title_words_a = word_set(&a.title);
        if title_words_a.len() < 2 {
            continue;
        }

        for b in rows.iter().skip(i + 1) {
            if comparisons >= max_comparisons {
                break;
            }

            // Must be same type
            if a.memory_type != b.memory_type {
                continue;
            }

            comparisons += 1;

            let title_words_b = word_set(&b.title);
            if title_words_b.len() < 2 {
                continue;
            }

            // High title overlap (>= 50% Jaccard) means they're about the same topic
            let title_overlap = jaccard(&title_words_a, &title_words_b);
            if title_overlap < 0.5 {
                continue;
            }

            // Low content overlap (< 30%) means they say different things
            let content_words_a = word_set(&a.content);
            let content_words_b = word_set(&b.content);
            let content_overlap = jaccard(&content_words_a, &content_words_b);
            if content_overlap >= 0.3 {
                continue;
            }

            // Check if this contradiction edge already exists (either direction)
            let diag_id = format!("contradiction-{}-{}", a.id, b.id);
            let diag_id_rev = format!("contradiction-{}-{}", b.id, a.id);
            let exists: bool = conn
                .query_row(
                    "SELECT COUNT(*) > 0 FROM diagnostic WHERE id IN (?1, ?2)",
                    rusqlite::params![diag_id, diag_id_rev],
                    |row| row.get(0),
                )
                .unwrap_or(false);
            if exists {
                continue;
            }

            // Also check if edge already exists
            let edge_exists: bool = conn
                .query_row(
                    "SELECT COUNT(*) > 0 FROM edge WHERE edge_type = 'contradicts'
                     AND ((from_id = ?1 AND to_id = ?2) OR (from_id = ?2 AND to_id = ?1))",
                    rusqlite::params![a.id, b.id],
                    |row| row.get(0),
                )
                .unwrap_or(false);
            if edge_exists {
                continue;
            }

            // Create diagnostic warning
            let message = format!(
                "Content contradiction: \"{}\" vs \"{}\". Title overlap {:.0}%, content overlap {:.0}%.",
                a.title, b.title, title_overlap * 100.0, content_overlap * 100.0
            );
            let diag = Diagnostic {
                id: diag_id.clone(),
                file_path: "memory://contradictions".to_string(),
                severity: "warning".to_string(),
                message,
                source: "forge-consolidator".to_string(),
                line: None,
                column: None,
                created_at: forge_core::time::now_iso(),
                expires_at: forge_core::time::now_offset(86400),
            };
            if let Err(e) = store_diagnostic(conn, &diag) {
                eprintln!("[consolidator] content contradiction diagnostic error: {e}");
                continue;
            }

            // Create 'contradicts' edge
            let edge_id = format!("edge-contradiction-{}-{}", a.id, b.id);
            let _ = conn.execute(
                "INSERT OR IGNORE INTO edge (id, from_id, to_id, edge_type, properties, created_at, valid_from)
                 VALUES (?1, ?2, ?3, 'contradicts', ?4, ?5, ?5)",
                rusqlite::params![
                    edge_id,
                    a.id,
                    b.id,
                    format!("{{\"detection\":\"content\",\"title_overlap\":{:.2},\"content_overlap\":{:.2}}}", title_overlap, content_overlap),
                    forge_core::time::now_iso(),
                ],
            );
            found += 1;
        }
    }

    found
}

/// Synthesize contradictions: find pairs of conflicting memories (same tags,
/// opposite valence, both active), create a resolution memory, and mark
/// originals as "superseded". Returns count of resolutions created.
pub fn synthesize_contradictions(conn: &Connection, batch_limit: usize) -> usize {
    // Find pairs of active memories with opposite valence, shared tags, high intensity
    let mut stmt = match conn.prepare(&format!(
        "SELECT id, title, content, tags, valence, intensity, confidence, project, organization_id FROM memory
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
        organization_id: String,
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
            organization_id: row
                .get::<_, String>(8)
                .unwrap_or_else(|_| "default".to_string()),
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

            // Must be in the same organization (multi-tenant isolation)
            if rows[i].organization_id != rows[j].organization_id {
                continue;
            }

            // Must have opposite valence
            if rows[i].valence == rows[j].valence {
                continue;
            }

            // Count shared tags (HashSet for O(n) instead of O(n^2))
            let tags_i: std::collections::HashSet<&str> =
                rows[i].tags.iter().map(|s| s.as_str()).collect();
            let shared: usize = rows[j]
                .tags
                .iter()
                .filter(|t| tags_i.contains(t.as_str()))
                .count();
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

            let resolution =
                Memory::new(MemoryType::Decision, &resolution_title, &resolution_content)
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
            if conn
                .execute(
                    "UPDATE memory SET status = 'superseded' WHERE id = ?1",
                    rusqlite::params![a.id],
                )
                .is_err()
                || conn
                    .execute(
                        "UPDATE memory SET status = 'superseded' WHERE id = ?1",
                        rusqlite::params![b.id],
                    )
                    .is_err()
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
        // Check if an unconsumed perception for this gap already exists
        let already_exists: bool = conn
            .query_row(
                "SELECT COUNT(*) > 0 FROM perception WHERE consumed = 0 AND kind = 'knowledge_gap' AND data LIKE 'Knowledge gap: no entity for ''' || ?1 || '''%'",
                rusqlite::params![word],
                |row| row.get(0),
            )
            .unwrap_or(false);

        if already_exists {
            continue;
        }

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
            data: format!("Knowledge gap: no entity for '{word}' despite {freq} references"),
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
        "SELECT id, title, content, tags, memory_type, project, created_at, organization_id FROM memory
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
        organization_id: String,
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
            organization_id: row
                .get::<_, String>(7)
                .unwrap_or_else(|_| "default".to_string()),
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

            // Must be in the same organization (multi-tenant isolation)
            if rows[i].organization_id != rows[j].organization_id {
                continue;
            }

            // Count shared tags
            let tags_i: std::collections::HashSet<&str> =
                rows[i].tags.iter().map(|s| s.as_str()).collect();
            let shared: usize = rows[j]
                .tags
                .iter()
                .filter(|t| tags_i.contains(t.as_str()))
                .count();
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
                Err(_) => {
                    let _ = conn.execute_batch("ROLLBACK");
                    continue;
                }
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
            if update1.is_err()
                || update2.is_err()
                || update1.unwrap_or(0) != 1
                || update2.unwrap_or(0) != 1
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
            eprintln!(
                "[consolidator] quality score update error for {}: {e}",
                row.id
            );
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
        let protocol_title = format!("Protocol: {title}");
        let exists: bool = conn
            .query_row(
                "SELECT COUNT(*) FROM memory WHERE memory_type = 'protocol' AND status = 'active'
                 AND title = ?1",
                rusqlite::params![protocol_title],
                |row| row.get::<_, i64>(0),
            )
            .unwrap_or(0)
            > 0;

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

/// Phase 23: Behavioral Skill Inference — elevate recurring clean tool-use
/// patterns from `session_tool_call` to the `skill` table.
///
/// Detection signal: tool-call rows with `success=1 AND user_correction_flag=0`,
/// grouped by `(agent, session_id)`, canonicalized via
/// `skill_inference::canonical_fingerprint`, elevated when the fingerprint
/// appears in ≥ `min_sessions` distinct sessions within the last
/// `window_days` days.
///
/// Idempotent: re-running with no new data merges `inferred_from` without
/// creating duplicate rows (ON CONFLICT (agent, fingerprint) DO UPDATE).
///
/// Returns the number of rows affected (INSERTs + upsert-UPDATEs).
pub fn infer_skills_from_behavior(
    conn: &Connection,
    min_sessions: usize,
    window_days: u32,
) -> usize {
    use crate::workers::skill_inference::{
        canonical_fingerprint, format_skill_name, infer_domain, ToolCall,
    };
    use std::collections::{BTreeMap, BTreeSet};

    // Step 1: SELECT clean rows within the window. Join the session table to
    // carry project through — otherwise inferred skills are project-NULL and
    // leak across every project on recall (T10 review Codex-H2).
    let sql = format!(
        "SELECT stc.agent, stc.session_id, stc.tool_name, stc.tool_args,
                COALESCE(s.project, '') AS project
         FROM session_tool_call AS stc
         LEFT JOIN session AS s ON s.id = stc.session_id
         WHERE stc.success = 1 AND stc.user_correction_flag = 0
           AND stc.created_at > datetime('now', '-{window_days} days')
         ORDER BY stc.agent, COALESCE(s.project, ''), stc.session_id, stc.created_at"
    );
    let mut stmt = match conn.prepare(&sql) {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!(error = %e, "Phase 23: prepare failed, skipping");
            return 0;
        }
    };
    let rows: Vec<(String, String, String, String, String)> = match stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, String>(4)?,
        ))
    }) {
        Ok(iter) => iter.filter_map(|r| r.ok()).collect(),
        Err(e) => {
            tracing::warn!(error = %e, "Phase 23: query failed, skipping");
            return 0;
        }
    };

    // Step 2: group by (agent, project, session_id), build per-session fingerprint.
    let mut per_session: BTreeMap<(String, String, String), Vec<ToolCall>> = BTreeMap::new();
    for (agent, session_id, tool_name, tool_args_json, project) in rows {
        let arg_keys: Vec<String> = match serde_json::from_str::<serde_json::Value>(&tool_args_json)
        {
            Ok(serde_json::Value::Object(map)) => {
                let mut ks: Vec<String> = map.keys().cloned().collect();
                ks.sort();
                ks
            }
            Ok(_) => Vec::new(),
            Err(_) => {
                tracing::warn!(
                    session_id = %session_id,
                    "Phase 23: tool_args not valid JSON, skipping row"
                );
                continue;
            }
        };
        per_session
            .entry((agent, project, session_id))
            .or_default()
            .push(ToolCall {
                tool_name,
                arg_keys,
            });
    }

    // Step 3: aggregate fingerprints across sessions, scoped per project.
    //   (agent, project, fingerprint) -> (sessions, last-seen tool_names_sorted)
    type FpKey = (String, String, String);
    type FpBucket = (BTreeSet<String>, Vec<String>);
    let mut fp_sessions: BTreeMap<FpKey, FpBucket> = BTreeMap::new();
    for ((agent, project, session_id), calls) in per_session {
        let fp = canonical_fingerprint(&calls);
        let mut names: Vec<String> = calls.iter().map(|c| c.tool_name.clone()).collect();
        names.sort();
        names.dedup();
        let entry = fp_sessions
            .entry((agent, project, fp))
            .or_insert_with(|| (BTreeSet::new(), names.clone()));
        entry.0.insert(session_id);
        entry.1 = names;
    }

    // Step 4: filter ≥ min_sessions + elevate.
    let now_iso = forge_core::time::now_iso();
    let mut affected = 0_usize;
    for ((agent, project, fingerprint), (sessions, tool_names_sorted)) in fp_sessions {
        if sessions.len() < min_sessions {
            continue;
        }
        let name = format_skill_name(&tool_names_sorted, &fingerprint);
        let domain = infer_domain(&tool_names_sorted);
        let inferred_from = serde_json::to_string(&sessions.into_iter().collect::<Vec<String>>())
            .unwrap_or_else(|_| "[]".to_string());
        let id = ulid::Ulid::new().to_string();
        // Always write project as a TEXT value (possibly empty '') — never
        // NULL. SQLite treats each NULL as distinct in unique indexes, so a
        // NULL project would break idempotency on the partial unique index
        // (agent, project, fingerprint). Recall.rs already maps Some("") to
        // "global" semantics alongside None.
        let project_value: &str = project.as_str();

        // SQLite partial-index workaround: the unique index on (agent, project, fingerprint)
        // is partial (WHERE fingerprint != ''), so ON CONFLICT(cols) DO UPDATE
        // is not usable. Use INSERT OR IGNORE + UPDATE instead.
        //
        // skill_type='behavioral' is explicit — without it the column default
        // 'procedural' matches prune_junk_skills()'s delete predicate and every
        // inferred row gets wiped at next daemon startup (T10 review Codex-H1).
        let insert_res = conn.execute(
            "INSERT OR IGNORE INTO skill
             (id, name, domain, description, steps, source, skill_type, project,
              agent, fingerprint, inferred_from, inferred_at, success_count)
             VALUES (?1, ?2, ?3, '', '[]', 'inferred', 'behavioral', ?4, ?5, ?6, ?7, ?8, 0)",
            rusqlite::params![
                id,
                name,
                domain,
                project_value,
                agent,
                fingerprint,
                inferred_from,
                now_iso,
            ],
        );
        match insert_res {
            Err(e) => {
                tracing::error!(error = %e, "Phase 23: INSERT OR IGNORE failed, skipping fingerprint");
                continue;
            }
            Ok(inserted) if inserted > 0 => {
                affected += 1;
            }
            Ok(_) => {
                // Row already exists — REPLACE inferred_from with the current
                // window's session set (not UNION). The pre-2P-1b merge path
                // accumulated session IDs forever, so long-lived patterns grew
                // inferred_sessions="N" unbounded even after most contributing
                // sessions aged out of the window (Codex-MED / 2P-1b §14). The
                // window filter at the SELECT already restricts `sessions` to
                // the live set, so binding it directly is the correct value.
                //
                // json_valid is no longer needed on the right side — we rewrite
                // with known-good JSON — so the CASE guard is gone.
                let update_res = conn.execute(
                    "UPDATE skill SET
                        inferred_from = ?1,
                        inferred_at = ?2
                     WHERE agent = ?3 AND fingerprint = ?4
                       AND COALESCE(project, '') = ?5",
                    rusqlite::params![inferred_from, now_iso, agent, fingerprint, project_value],
                );
                match update_res {
                    Ok(n) => affected += n,
                    Err(e) => {
                        tracing::error!(error = %e, "Phase 23: UPDATE failed for fingerprint");
                    }
                }
            }
        }
    }

    affected
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

        if conn
            .execute(
                "UPDATE memory SET tags = ?1 WHERE id = ?2",
                rusqlite::params![new_tags, id],
            )
            .is_ok()
        {
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
pub fn heal_topic_supersedes(
    conn: &Connection,
    config: &crate::config::HealingConfig,
) -> HealingStats {
    let mut stats = HealingStats::default();

    if !config.enabled {
        return stats;
    }

    // Get active decision/lesson/pattern memories that have embeddings
    let sql = format!(
        "SELECT m.id, m.memory_type, m.title, m.content, m.confidence, m.created_at, m.organization_id
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
        organization_id: String,
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
                organization_id: row
                    .get::<_, String>(6)
                    .unwrap_or_else(|_| "default".to_string()),
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
                "SELECT memory_type, title, content, confidence, created_at, organization_id FROM memory WHERE id = ?1 AND status = 'active'",
                rusqlite::params![neighbor_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, f64>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, String>(5).unwrap_or_else(|_| "default".to_string()),
                    ))
                },
            ) {
                Ok(n) => n,
                Err(_) => continue,
            };

            let (n_type, n_title, n_content, n_confidence, n_created_at, n_org_id) = neighbor;

            // Must be in the same organization (multi-tenant isolation)
            if n_org_id != candidate.organization_id {
                stats.false_positives_skipped += 1;
                continue;
            }

            // Must be same type
            if n_type != candidate.memory_type {
                stats.false_positives_skipped += 1;
                continue;
            }

            // Compute word overlap on combined title+content
            let cand_text = format!("{} {}", candidate.title, candidate.content);
            let neigh_text = format!("{n_title} {n_content}");
            let cand_words = ops::meaningful_words_pub(&cand_text);
            let neigh_words = ops::meaningful_words_pub(&neigh_text);

            let intersection = cand_words.intersection(&neigh_words).count();
            let union = cand_words.union(&neigh_words).count();
            let overlap = if union > 0 {
                intersection as f64 / union as f64
            } else {
                0.0
            };

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

            // TODO(2A-4+): migrate to ops::supersede_memory_impl() — see docs/superpowers/specs/2026-04-17-forge-valence-flipping-design.md §14 R1.
            // Supersede: update status + superseded_by (org-scoped for multi-tenant safety)
            if conn.execute(
                "UPDATE memory SET status = 'superseded', superseded_by = ?1 WHERE id = ?2 AND organization_id = ?3",
                rusqlite::params![new_id, old_id, &candidate.organization_id],
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
/// Two tiers: aggressive (quality < 0.1, 3 days) and normal (quality < threshold, N days).
pub fn heal_session_staleness(conn: &Connection, config: &crate::config::HealingConfig) -> usize {
    if !config.enabled {
        return 0;
    }

    let days = config.staleness_days;
    let min_quality = config.staleness_min_quality;

    // Aggressive tier: near-garbage memories (quality < 0.1) fade after 3 days
    let aggressive_days = 3u64;
    let aggressive_quality = 0.1;
    let aggressive_faded: usize = conn
        .execute(
            "UPDATE memory SET status = 'faded'
         WHERE status = 'active'
         AND COALESCE(quality_score, 0.5) < ?1
         AND access_count = 0
         AND created_at < datetime('now', ?2)",
            rusqlite::params![aggressive_quality, format!("-{aggressive_days} days")],
        )
        .unwrap_or(0);

    // Normal tier: low-quality memories fade after configured days
    let normal_faded: usize = conn
        .execute(
            "UPDATE memory SET status = 'faded'
         WHERE status = 'active'
         AND COALESCE(quality_score, 0.5) < ?1
         AND access_count = 0
         AND created_at < datetime('now', ?2)",
            rusqlite::params![min_quality, format!("-{days} days")],
        )
        .unwrap_or(0);

    let faded = aggressive_faded + normal_faded;

    // Log each faded memory
    if faded > 0 {
        let now = forge_core::time::now_iso();
        // Query the just-faded memories to log them
        let ids: Vec<String> = conn
            .prepare(
                "SELECT id FROM memory WHERE status = 'faded'
             AND COALESCE(quality_score, 0.5) < ?1
             AND access_count = 0",
            )
            .and_then(|mut stmt| {
                stmt.query_map(rusqlite::params![min_quality], |row| row.get(0))?
                    .collect()
            })
            .unwrap_or_default();

        for id in ids.iter().take(config.batch_limit) {
            let log_id = ulid::Ulid::new().to_string();
            if let Err(e) = conn.execute(
                "INSERT INTO healing_log (id, action, old_memory_id, reason, created_at)
                 VALUES (?1, 'auto_faded', ?2, ?3, ?4)",
                rusqlite::params![
                    log_id,
                    id,
                    format!("Stale: quality < {min_quality}, 0 accesses, > {days} days old"),
                    now
                ],
            ) {
                eprintln!("[healing] healing_log insert failed: {e}");
            }
        }
    }

    faded
}

/// Phase 22: Natural selection — decay unused memories' quality, boost accessed ones.
/// Two-tier decay: accelerated decay (0.15) for quality < 0.3, normal decay for the rest.
pub fn apply_quality_pressure(conn: &Connection, config: &crate::config::HealingConfig) -> usize {
    if !config.enabled {
        return 0;
    }

    let decay = config.quality_decay_per_cycle;
    let boost = config.quality_boost_per_access;
    let accelerated_decay = 0.15_f64.max(decay); // at least 0.15 for low-quality

    // Accelerated decay: faster decay for low-quality memories (quality < 0.3)
    let accel_decayed: usize = conn
        .execute(
            "UPDATE memory SET quality_score = MAX(0.0, COALESCE(quality_score, 0.5) - ?1)
         WHERE status = 'active' AND access_count = 0
         AND COALESCE(quality_score, 0.5) > 0.0
         AND COALESCE(quality_score, 0.5) < 0.3",
            rusqlite::params![accelerated_decay],
        )
        .unwrap_or(0);

    // Normal decay: reduce quality for unaccessed active memories with quality >= 0.3 (floor at 0.0)
    let normal_decayed: usize = conn
        .execute(
            "UPDATE memory SET quality_score = MAX(0.0, COALESCE(quality_score, 0.5) - ?1)
         WHERE status = 'active' AND access_count = 0
         AND COALESCE(quality_score, 0.5) >= 0.3",
            rusqlite::params![decay],
        )
        .unwrap_or(0);

    // Boost: increase quality for recently accessed active memories (cap at 1.0)
    let boosted: usize = conn
        .execute(
            "UPDATE memory SET quality_score = MIN(1.0, COALESCE(quality_score, 0.5) + ?1)
         WHERE status = 'active' AND access_count > 0
         AND accessed_at > datetime('now', '-1 day')",
            rusqlite::params![boost],
        )
        .unwrap_or(0);

    accel_decayed + normal_decayed + boosted
}

pub async fn run_consolidator(
    state: Arc<Mutex<crate::server::handler::DaemonState>>,
    mut shutdown_rx: watch::Receiver<bool>,
    interval_secs: u64,
) {
    let interval = Duration::from_secs(interval_secs);
    eprintln!("[consolidator] started, interval = {interval:?}");

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
                    run_all_phases(&locked.conn, &consol_config, None)
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
        let stats = run_all_phases(&conn, &config, None);
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
        let older = Memory::new(
            MemoryType::Decision,
            "Use JWT auth",
            "We chose JWT for authentication",
        )
        .with_tags(vec![
            "auth".to_string(),
            "security".to_string(),
            "jwt".to_string(),
        ]);
        ops::remember(&conn, &older).unwrap();

        // Create a newer memory with shared tags (same project, same type)
        // Need a slight delay in created_at to ensure ordering
        let newer = Memory::new(
            MemoryType::Decision,
            "JWT rotation policy",
            "Rotate JWT tokens every 24h",
        )
        .with_tags(vec![
            "auth".to_string(),
            "security".to_string(),
            "rotation".to_string(),
        ]);
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
        let content: String = conn
            .query_row(
                "SELECT content FROM memory WHERE id = ?1",
                rusqlite::params![older.id],
                |row| row.get(0),
            )
            .unwrap();
        assert!(
            content.contains("[Update]:"),
            "older memory should contain [Update] marker"
        );
        assert!(
            content.contains("Rotate JWT tokens every 24h"),
            "older memory should contain newer content"
        );

        // Verify newer memory was marked as merged
        let status: String = conn
            .query_row(
                "SELECT status FROM memory WHERE id = ?1",
                rusqlite::params![newer.id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(status, "merged", "newer memory should be marked as merged");
    }

    #[test]
    fn test_reweave_different_types_skipped() {
        crate::db::vec::init_sqlite_vec();
        let conn = Connection::open_in_memory().unwrap();
        crate::db::schema::create_schema(&conn).unwrap();

        // Create a decision memory
        let decision = Memory::new(
            MemoryType::Decision,
            "Use JWT auth",
            "JWT for authentication",
        )
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
        )
        .unwrap();

        let count = score_memory_quality(&conn, 200);
        assert_eq!(count, 1, "should score 1 memory");

        let score: f64 = conn
            .query_row(
                "SELECT quality_score FROM memory WHERE id = 'qs-1'",
                [],
                |row| row.get(0),
            )
            .unwrap();

        // freshness: created today = 1.0
        // utility: 5/10 = 0.5
        // completeness: 200/200 = 1.0
        // activation: 0.5
        // expected = 1.0*0.3 + 0.5*0.3 + 1.0*0.2 + 0.5*0.2 = 0.3 + 0.15 + 0.2 + 0.1 = 0.75
        assert!(
            (score - 0.75).abs() < 0.05,
            "score should be ~0.75, got {score}"
        );
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
        )
        .unwrap();

        score_memory_quality(&conn, 200);

        let fresh_score: f64 = conn
            .query_row(
                "SELECT quality_score FROM memory WHERE id = 'fresh-1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let old_score: f64 = conn
            .query_row(
                "SELECT quality_score FROM memory WHERE id = 'old-1'",
                [],
                |row| row.get(0),
            )
            .unwrap();

        assert!(
            fresh_score > old_score,
            "fresh memory score ({fresh_score}) should be higher than old ({old_score})"
        );
    }

    /// Simple deterministic text embedding for tests.
    /// Creates a 768-dim vector from word hashes — same words = similar vectors.
    fn simple_text_embedding(text: &str) -> Vec<f32> {
        let mut emb = vec![0.0f32; 768];
        for word in text.to_lowercase().split_whitespace() {
            let hash = word
                .bytes()
                .fold(0u64, |acc, b| acc.wrapping_mul(31).wrapping_add(b as u64));
            let idx = (hash % 768) as usize;
            emb[idx] += 1.0;
        }
        let norm: f32 = emb.iter().map(|x| x * x).sum::<f32>().sqrt();
        if norm > 0.0 {
            emb.iter_mut().for_each(|x| *x /= norm);
        }
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
        )
        .unwrap();

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
        assert_eq!(
            stats.topic_superseded, 1,
            "should supersede the old decision"
        );

        // Verify old memory is superseded
        let status: String = conn
            .query_row(
                "SELECT status FROM memory WHERE id = ?1",
                rusqlite::params![old.id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(status, "superseded", "old memory should be superseded");

        // Verify healing_log entry
        let log_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM healing_log WHERE action = 'auto_superseded'",
                [],
                |row| row.get(0),
            )
            .unwrap();
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
        )
        .unwrap();

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

        assert_eq!(
            stats.topic_superseded, 0,
            "should NOT supersede nearly identical memories (dedup handles)"
        );
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
        )
        .unwrap();

        let lesson = Memory::new(
            MemoryType::Lesson,
            "SQLite primary storage backend lesson",
            "We evaluated options and learned SQLite for the primary storage backend due to its simplicity",
        );
        ops::remember(&conn, &lesson).unwrap();

        // Use identical embeddings so they are KNN neighbors
        let emb =
            simple_text_embedding("SQLite primary storage backend evaluated options simplicity");
        crate::db::vec::store_embedding(&conn, &decision.id, &emb).unwrap();
        crate::db::vec::store_embedding(&conn, &lesson.id, &emb).unwrap();

        let config = crate::config::HealingConfig::default();
        let stats = heal_topic_supersedes(&conn, &config);

        assert_eq!(
            stats.topic_superseded, 0,
            "should NOT supersede across different types"
        );
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
        )
        .unwrap();

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

        assert!(
            stats.topic_superseded >= 1,
            "should supersede at least one memory"
        );

        // Verify the OLD one is superseded, not the new one
        let old_status: String = conn
            .query_row(
                "SELECT status FROM memory WHERE id = ?1",
                rusqlite::params![old.id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(old_status, "superseded", "OLD memory should be superseded");

        let new_status: String = conn
            .query_row(
                "SELECT status FROM memory WHERE id = ?1",
                rusqlite::params![new_mem.id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(new_status, "active", "NEW memory should remain active");

        // Verify superseded_by points to new memory
        let superseded_by: Option<String> = conn
            .query_row(
                "SELECT superseded_by FROM memory WHERE id = ?1",
                rusqlite::params![old.id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            superseded_by,
            Some(new_mem.id.clone()),
            "superseded_by should point to new memory"
        );
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
        assert!(
            faded > 0,
            "old unaccessed low-quality memory should be faded"
        );

        let status: String = conn
            .query_row(
                "SELECT status FROM memory WHERE id = ?1",
                rusqlite::params![m.id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(status, "faded");

        // Verify healing_log entry
        let log_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM healing_log WHERE action = 'auto_faded'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(
            log_count > 0,
            "should have healing_log entry for faded memory"
        );
    }

    #[test]
    fn test_heal_session_staleness_preserves_accessed() {
        crate::db::vec::init_sqlite_vec();
        let conn = Connection::open_in_memory().unwrap();
        crate::db::schema::create_schema(&conn).unwrap();

        let m = Memory::new(
            MemoryType::Decision,
            "Old but accessed decision",
            "Still useful",
        )
        .with_confidence(0.9);
        ops::remember(&conn, &m).unwrap();
        // Backdate but give it access count
        conn.execute(
            "UPDATE memory SET created_at = datetime('now', '-14 days'), quality_score = 0.1, access_count = 5 WHERE id = ?1",
            rusqlite::params![m.id],
        ).unwrap();

        let config = crate::config::HealingConfig::default();
        let faded = heal_session_staleness(&conn, &config);
        assert_eq!(
            faded, 0,
            "accessed memory should not be faded regardless of age/quality"
        );

        let status: String = conn
            .query_row(
                "SELECT status FROM memory WHERE id = ?1",
                rusqlite::params![m.id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(status, "active");
    }

    #[test]
    fn test_quality_pressure_decays_unused() {
        crate::db::vec::init_sqlite_vec();
        let conn = Connection::open_in_memory().unwrap();
        crate::db::schema::create_schema(&conn).unwrap();

        let m = Memory::new(
            MemoryType::Decision,
            "Unused decision for decay test",
            "Never accessed",
        )
        .with_confidence(0.9);
        ops::remember(&conn, &m).unwrap();
        conn.execute(
            "UPDATE memory SET quality_score = 0.5, access_count = 0 WHERE id = ?1",
            rusqlite::params![m.id],
        )
        .unwrap();

        let config = crate::config::HealingConfig::default();
        let adjusted = apply_quality_pressure(&conn, &config);
        assert!(adjusted > 0, "should adjust at least one memory");

        let quality: f64 = conn
            .query_row(
                "SELECT quality_score FROM memory WHERE id = ?1",
                rusqlite::params![m.id],
                |row| row.get(0),
            )
            .unwrap();
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
        let m = Memory::new(
            MemoryType::Decision,
            "Very stale memory for consolidation test",
            "Ancient content",
        )
        .with_confidence(0.5);
        ops::remember(&conn, &m).unwrap();
        conn.execute(
            "UPDATE memory SET created_at = datetime('now', '-30 days'), quality_score = 0.1, access_count = 0 WHERE id = ?1",
            rusqlite::params![m.id],
        ).unwrap();

        let config = crate::config::ConsolidationConfig::default();
        let stats = run_all_phases(&conn, &config, None);

        // Healing phases should have run
        assert!(
            stats.healed_faded > 0,
            "Phase 21 should have faded the stale memory"
        );
    }

    #[test]
    fn test_no_cascade_supersede() {
        crate::db::vec::init_sqlite_vec();
        let conn = Connection::open_in_memory().unwrap();
        crate::db::schema::create_schema(&conn).unwrap();

        let config = crate::config::HealingConfig::default();

        // A: Old storage decision
        let a = Memory::new(
            MemoryType::Decision,
            "Old storage approach using Redis cluster",
            "Redis for distributed caching",
        )
        .with_confidence(0.8);
        ops::remember(&conn, &a).unwrap();
        conn.execute(
            "UPDATE memory SET created_at = datetime('now', '-30 days') WHERE id = ?1",
            rusqlite::params![a.id],
        )
        .unwrap();

        // C: Completely unrelated auth decision (should NOT be affected)
        let c = Memory::new(
            MemoryType::Decision,
            "Use OAuth2 for third-party authentication",
            "OAuth2 with PKCE flow for security",
        )
        .with_confidence(0.9);
        ops::remember(&conn, &c).unwrap();

        // D: New storage decision that supersedes A
        let d = Memory::new(
            MemoryType::Decision,
            "New storage approach using SQLite cache",
            "SQLite for local caching instead of Redis",
        )
        .with_confidence(0.9);
        ops::remember(&conn, &d).unwrap();

        // Store embeddings for all three
        let emb_a = simple_text_embedding(&format!("{} {}", a.title, a.content));
        let emb_c = simple_text_embedding(&format!("{} {}", c.title, c.content));
        let emb_d = simple_text_embedding(&format!("{} {}", d.title, d.content));
        crate::db::vec::store_embedding(&conn, &a.id, &emb_a).unwrap();
        crate::db::vec::store_embedding(&conn, &c.id, &emb_c).unwrap();
        crate::db::vec::store_embedding(&conn, &d.id, &emb_d).unwrap();

        heal_topic_supersedes(&conn, &config);

        // C (unrelated auth) MUST remain active
        let c_status: String = conn
            .query_row(
                "SELECT status FROM memory WHERE id = ?1",
                rusqlite::params![c.id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            c_status, "active",
            "unrelated memory MUST NOT be cascade-superseded"
        );
    }

    #[test]
    fn test_healing_idempotent() {
        crate::db::vec::init_sqlite_vec();
        let conn = Connection::open_in_memory().unwrap();
        crate::db::schema::create_schema(&conn).unwrap();

        let config = crate::config::HealingConfig::default();

        let old = Memory::new(
            MemoryType::Decision,
            "Old approach to distributed caching",
            "Use Redis cluster with sentinel",
        )
        .with_confidence(0.8);
        ops::remember(&conn, &old).unwrap();
        conn.execute(
            "UPDATE memory SET created_at = datetime('now', '-30 days') WHERE id = ?1",
            rusqlite::params![old.id],
        )
        .unwrap();

        let new_mem = Memory::new(
            MemoryType::Decision,
            "New approach to distributed caching strategy",
            "Use local SQLite cache instead",
        )
        .with_confidence(0.9);
        ops::remember(&conn, &new_mem).unwrap();

        let emb_old = simple_text_embedding(&format!("{} {}", old.title, old.content));
        let emb_new = simple_text_embedding(&format!("{} {}", new_mem.title, new_mem.content));
        crate::db::vec::store_embedding(&conn, &old.id, &emb_old).unwrap();
        crate::db::vec::store_embedding(&conn, &new_mem.id, &emb_new).unwrap();

        let stats1 = heal_topic_supersedes(&conn, &config);
        let stats2 = heal_topic_supersedes(&conn, &config);

        // Second run should find nothing new (already superseded)
        assert_eq!(
            stats2.topic_superseded, 0,
            "second healing run must be idempotent — nothing new to supersede"
        );
        // First run may or may not have superseded depending on similarity
        // but second run must always be zero regardless
        let _ = stats1;
    }

    #[test]
    fn test_healing_with_no_embeddings() {
        crate::db::vec::init_sqlite_vec();
        let conn = Connection::open_in_memory().unwrap();
        crate::db::schema::create_schema(&conn).unwrap();

        let config = crate::config::HealingConfig::default();

        // Memory without embedding — healing should skip gracefully
        let m = Memory::new(
            MemoryType::Decision,
            "Decision without any embedding vector",
            "No embedding stored for this",
        )
        .with_confidence(0.9);
        ops::remember(&conn, &m).unwrap();
        // Don't store embedding

        let stats = heal_topic_supersedes(&conn, &config);
        assert_eq!(stats.topic_superseded, 0, "no embeddings = no healing");
        assert_eq!(
            stats.candidates_found, 0,
            "no candidates without embeddings"
        );
    }

    #[test]
    fn test_meeting_timeout_auto_synthesis() {
        crate::db::vec::init_sqlite_vec();
        let conn = Connection::open_in_memory().unwrap();
        crate::db::schema::create_schema(&conn).unwrap();

        // Create a team
        conn.execute(
            "INSERT INTO team (id, name, organization_id, created_by, status, created_at)
             VALUES ('t1', 'eng', 'default', 'system', 'active', datetime('now'))",
            [],
        )
        .unwrap();

        // Create a meeting that's already past timeout (backdate it)
        conn.execute(
            "INSERT INTO meeting (id, team_id, topic, status, orchestrator_session_id, created_at)
             VALUES ('m1', 't1', 'Architecture review', 'collecting', 'orch-1', datetime('now', '-600 seconds'))",
            [],
        ).unwrap();

        // Add a partial response
        conn.execute(
            "INSERT INTO meeting_participant (id, meeting_id, session_id, status, response, responded_at)
             VALUES ('mp1', 'm1', 'cto-1', 'responded', 'Use microservices for scalability', datetime('now', '-300 seconds'))",
            [],
        ).unwrap();

        // Run full consolidation (which includes meeting timeout in phase 19d)
        let config = crate::config::ConsolidationConfig::default();
        let _stats = run_all_phases(&conn, &config, None);

        // Verify meeting is timed_out
        let status: String = conn
            .query_row(
                "SELECT status FROM meeting WHERE id = 'm1'",
                [],
                |row: &rusqlite::Row| row.get(0),
            )
            .unwrap();
        assert_eq!(status, "timed_out");

        // Verify synthesis was stored
        let synthesis: Option<String> = conn
            .query_row(
                "SELECT synthesis FROM meeting WHERE id = 'm1'",
                [],
                |row: &rusqlite::Row| row.get(0),
            )
            .unwrap();
        assert!(synthesis.is_some(), "synthesis should be stored on timeout");
        let syn = synthesis.unwrap();
        assert!(
            syn.contains("Architecture review"),
            "synthesis should mention the topic"
        );
        assert!(
            syn.contains("microservices"),
            "synthesis should include partial response"
        );

        // Verify a decision memory was created
        let decision_count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM memory WHERE title LIKE '%Architecture review%' AND memory_type = 'decision'",
            [], |row: &rusqlite::Row| row.get(0),
        ).unwrap();
        assert!(
            decision_count > 0,
            "auto-synthesis should create a decision memory"
        );
    }

    #[test]
    fn test_detect_and_surface_gaps_no_duplicates() {
        crate::db::vec::init_sqlite_vec();
        let conn = Connection::open_in_memory().unwrap();
        crate::db::schema::create_schema(&conn).unwrap();

        // Create 3 memories that reference "consolidator" so it becomes a gap word
        for i in 0..3 {
            let m = Memory::new(
                MemoryType::Lesson,
                format!("consolidator tuning attempt {i}"),
                "some content",
            );
            ops::remember(&conn, &m).unwrap();
        }

        // First call should create perception(s)
        let first_count = detect_and_surface_gaps(&conn);
        assert!(first_count > 0, "first call should create gap perceptions");

        // Count perceptions
        let perception_count_after_first: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM perception WHERE kind = 'knowledge_gap'",
                [],
                |row| row.get(0),
            )
            .unwrap();

        // Second call should NOT create duplicates
        let second_count = detect_and_surface_gaps(&conn);
        assert_eq!(
            second_count, 0,
            "second call should skip existing gap perceptions"
        );

        let perception_count_after_second: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM perception WHERE kind = 'knowledge_gap'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            perception_count_after_first, perception_count_after_second,
            "perception count should not increase on second call"
        );
    }

    // ── Fix 1: Notification auto-dismiss ──

    #[test]
    fn test_quality_notification_auto_dismissed_when_quality_improves() {
        crate::db::vec::init_sqlite_vec();
        let conn = Connection::open_in_memory().unwrap();
        crate::db::schema::create_schema(&conn).unwrap();

        // Create a quality decline notification manually (simulating a prior consolidation run)
        let notif_id = ulid::Ulid::new().to_string();
        let now = forge_core::time::now_iso();
        conn.execute(
            "INSERT INTO notification (id, category, priority, title, content, source, status, created_at, topic)
             VALUES (?1, 'insight', 'medium', 'Memory quality declining', 'Average quality score is 0.20', 'consolidator', 'pending', ?2, 'quality_decline')",
            rusqlite::params![notif_id, now],
        ).unwrap();

        // Verify notification exists as pending
        let pending_count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM notification WHERE status = 'pending' AND title LIKE '%quality%'",
            [], |row| row.get(0),
        ).unwrap();
        assert_eq!(
            pending_count, 1,
            "should have 1 pending quality notification"
        );

        // Create memories with GOOD quality scores (avg >= 0.3)
        for i in 0..5 {
            let m = Memory::new(
                MemoryType::Decision,
                format!("Good decision {i}"),
                "High quality",
            );
            ops::remember(&conn, &m).unwrap();
            conn.execute(
                "UPDATE memory SET quality_score = 0.8 WHERE id = ?1",
                rusqlite::params![m.id],
            )
            .unwrap();
        }

        // Run all phases — should auto-dismiss the notification since quality is good
        let config = crate::config::ConsolidationConfig::default();
        run_all_phases(&conn, &config, None);

        // Verify the notification is dismissed
        let dismissed_count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM notification WHERE status = 'dismissed' AND title LIKE '%quality%'",
            [], |row| row.get(0),
        ).unwrap();
        assert_eq!(
            dismissed_count, 1,
            "quality notification should be auto-dismissed when quality improves"
        );

        let remaining_pending: i64 = conn.query_row(
            "SELECT COUNT(*) FROM notification WHERE status = 'pending' AND title LIKE '%quality%'",
            [], |row| row.get(0),
        ).unwrap();
        assert_eq!(
            remaining_pending, 0,
            "no pending quality notifications should remain"
        );
    }

    // ── Fix 4: Healing tuning ──

    #[test]
    fn test_heal_session_staleness_aggressive_tier() {
        crate::db::vec::init_sqlite_vec();
        let conn = Connection::open_in_memory().unwrap();
        crate::db::schema::create_schema(&conn).unwrap();

        // Create a near-garbage memory (quality 0.05) that is only 4 days old
        let m = Memory::new(
            MemoryType::Decision,
            "Near garbage decision",
            "Very low quality",
        )
        .with_confidence(0.5);
        ops::remember(&conn, &m).unwrap();
        conn.execute(
            "UPDATE memory SET created_at = datetime('now', '-4 days'), quality_score = 0.05, access_count = 0 WHERE id = ?1",
            rusqlite::params![m.id],
        ).unwrap();

        // This memory should NOT be faded by the old logic (requires 7 days for quality < 0.2)
        // but SHOULD be faded by the aggressive tier (3 days for quality < 0.1)
        let config = crate::config::HealingConfig::default();
        let faded = heal_session_staleness(&conn, &config);
        assert!(
            faded > 0,
            "near-garbage memory (quality 0.05, 4 days old) should be faded by aggressive tier"
        );

        let status: String = conn
            .query_row(
                "SELECT status FROM memory WHERE id = ?1",
                rusqlite::params![m.id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(status, "faded");
    }

    #[test]
    fn test_heal_session_staleness_normal_tier_still_works() {
        crate::db::vec::init_sqlite_vec();
        let conn = Connection::open_in_memory().unwrap();
        crate::db::schema::create_schema(&conn).unwrap();

        // Create a low-quality memory (quality 0.15) that is 10 days old
        // This should be caught by the normal tier (quality < 0.2, > 7 days)
        let m = Memory::new(MemoryType::Decision, "Old low quality decision", "Stale")
            .with_confidence(0.5);
        ops::remember(&conn, &m).unwrap();
        conn.execute(
            "UPDATE memory SET created_at = datetime('now', '-10 days'), quality_score = 0.15, access_count = 0 WHERE id = ?1",
            rusqlite::params![m.id],
        ).unwrap();

        let config = crate::config::HealingConfig::default();
        let faded = heal_session_staleness(&conn, &config);
        assert!(
            faded > 0,
            "low-quality memory (0.15, 10 days old) should be faded by normal tier"
        );

        let status: String = conn
            .query_row(
                "SELECT status FROM memory WHERE id = ?1",
                rusqlite::params![m.id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(status, "faded");
    }

    #[test]
    fn test_quality_pressure_accelerated_decay_for_low_quality() {
        crate::db::vec::init_sqlite_vec();
        let conn = Connection::open_in_memory().unwrap();
        crate::db::schema::create_schema(&conn).unwrap();

        // Create a low-quality memory (quality 0.25, below 0.3 threshold)
        let m_low = Memory::new(
            MemoryType::Decision,
            "Low quality accelerated decay",
            "Should decay faster",
        );
        ops::remember(&conn, &m_low).unwrap();
        conn.execute(
            "UPDATE memory SET quality_score = 0.25, access_count = 0 WHERE id = ?1",
            rusqlite::params![m_low.id],
        )
        .unwrap();

        // Create a normal-quality memory (quality 0.5, above 0.3 threshold)
        let m_normal = Memory::new(
            MemoryType::Decision,
            "Normal quality standard decay",
            "Should decay normally",
        );
        ops::remember(&conn, &m_normal).unwrap();
        conn.execute(
            "UPDATE memory SET quality_score = 0.5, access_count = 0 WHERE id = ?1",
            rusqlite::params![m_normal.id],
        )
        .unwrap();

        let config = crate::config::HealingConfig::default();
        apply_quality_pressure(&conn, &config);

        let low_quality: f64 = conn
            .query_row(
                "SELECT quality_score FROM memory WHERE id = ?1",
                rusqlite::params![m_low.id],
                |row| row.get(0),
            )
            .unwrap();
        let normal_quality: f64 = conn
            .query_row(
                "SELECT quality_score FROM memory WHERE id = ?1",
                rusqlite::params![m_normal.id],
                |row| row.get(0),
            )
            .unwrap();

        // Low-quality should decay by 0.15 (accelerated): 0.25 - 0.15 = 0.10
        assert!(
            (low_quality - 0.10).abs() < 0.01,
            "low-quality memory should decay by 0.15 (accelerated), got {low_quality}"
        );

        // Normal-quality should decay by 0.1 (standard): 0.5 - 0.1 = 0.4
        assert!(
            (normal_quality - 0.4).abs() < 0.01,
            "normal-quality memory should decay by 0.1 (standard), got {normal_quality}"
        );
    }

    // ── Content-based contradiction detection tests ──

    #[test]
    fn test_content_contradiction_detects_same_topic_different_conclusions() {
        crate::db::vec::init_sqlite_vec();
        let conn = Connection::open_in_memory().unwrap();
        crate::db::schema::create_schema(&conn).unwrap();

        // Two decisions about the same topic ("primary storage backend") but
        // completely different content conclusions.
        let a = Memory::new(
            MemoryType::Decision,
            "Use Redis for the primary storage backend",
            "Redis provides fast in-memory caching with persistence options and cluster support",
        );
        ops::remember(&conn, &a).unwrap();

        let b = Memory::new(
            MemoryType::Decision,
            "Use SQLite for the primary storage backend",
            "SQLite is a lightweight embedded database that requires no separate server process",
        );
        ops::remember(&conn, &b).unwrap();

        let found = detect_content_contradictions(&conn);
        assert!(
            found >= 1,
            "should detect content contradiction, got {found}"
        );

        // Verify edge was created
        let edge_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM edge WHERE edge_type = 'contradicts'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(
            edge_count >= 1,
            "should have a contradicts edge, got {edge_count}"
        );

        // Verify diagnostic was created
        let diag_count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM diagnostic WHERE source = 'forge-consolidator' AND message LIKE '%Content contradiction%'",
            [],
            |row| row.get(0),
        ).unwrap();
        assert!(
            diag_count >= 1,
            "should have a contradiction diagnostic, got {diag_count}"
        );
    }

    #[test]
    fn test_content_contradiction_skips_similar_content() {
        crate::db::vec::init_sqlite_vec();
        let conn = Connection::open_in_memory().unwrap();
        crate::db::schema::create_schema(&conn).unwrap();

        // Two decisions about same topic with very similar content — not a contradiction
        let a = Memory::new(
            MemoryType::Decision,
            "Use SQLite for the primary storage backend",
            "We chose SQLite for the primary storage backend because it is lightweight and embedded",
        );
        ops::remember(&conn, &a).unwrap();

        let b = Memory::new(
            MemoryType::Decision,
            "Use SQLite for the primary storage backend layer",
            "We chose SQLite for the primary storage backend because it is lightweight and requires no server",
        );
        // Use raw SQL to bypass title dedup
        conn.execute(
            "INSERT INTO memory (id, memory_type, title, content, confidence, status, tags, created_at, accessed_at, project)
             VALUES (?1, 'decision', 'Use SQLite for the primary storage backend layer',
                     'We chose SQLite for the primary storage backend because it is lightweight and requires no server',
                     0.9, 'active', '[]', datetime('now'), datetime('now'), NULL)",
            rusqlite::params![b.id],
        ).unwrap();

        let found = detect_content_contradictions(&conn);
        assert_eq!(
            found, 0,
            "should not detect contradiction for similar content"
        );
    }

    #[test]
    fn test_content_contradiction_skips_different_types() {
        crate::db::vec::init_sqlite_vec();
        let conn = Connection::open_in_memory().unwrap();
        crate::db::schema::create_schema(&conn).unwrap();

        // A decision and a lesson with same title pattern — different types, no contradiction
        let a = Memory::new(
            MemoryType::Decision,
            "Use Redis for the primary storage backend",
            "Redis provides fast in-memory caching with persistence options",
        );
        ops::remember(&conn, &a).unwrap();

        let b = Memory::new(
            MemoryType::Lesson,
            "Use SQLite for the primary storage backend",
            "SQLite is a lightweight embedded database that requires no server",
        );
        ops::remember(&conn, &b).unwrap();

        let found = detect_content_contradictions(&conn);
        assert_eq!(
            found, 0,
            "should not detect contradiction across different types"
        );
    }

    #[test]
    fn test_content_contradiction_idempotent() {
        crate::db::vec::init_sqlite_vec();
        let conn = Connection::open_in_memory().unwrap();
        crate::db::schema::create_schema(&conn).unwrap();

        let a = Memory::new(
            MemoryType::Decision,
            "Use Redis for the primary storage backend",
            "Redis provides fast in-memory caching with persistence options and cluster support",
        );
        ops::remember(&conn, &a).unwrap();

        let b = Memory::new(
            MemoryType::Decision,
            "Use SQLite for the primary storage backend",
            "SQLite is a lightweight embedded database that requires no separate server process",
        );
        ops::remember(&conn, &b).unwrap();

        let found1 = detect_content_contradictions(&conn);
        assert!(found1 >= 1, "first run should find contradictions");

        let found2 = detect_content_contradictions(&conn);
        assert_eq!(
            found2, 0,
            "second run should be idempotent — already detected"
        );
    }

    #[test]
    fn test_content_contradiction_integrated_in_all_phases() {
        crate::db::vec::init_sqlite_vec();
        let conn = Connection::open_in_memory().unwrap();
        crate::db::schema::create_schema(&conn).unwrap();

        // Create contradicting decisions with neutral valence (default).
        // Titles must survive semantic dedup (Phase 2) which uses meaningful_words
        // with stop words filtered and threshold 0.65 on max(weighted, title_score, content_score).
        // Title meaningful overlap: 5/8 = 0.625 < 0.65 → survives dedup.
        // Content meaningful overlap: ~0 → combined = max(0.3125, 0.625, 0) = 0.625 < 0.65.
        // My Jaccard (3+ char words, no stop filter): 6/12 = 0.50 >= 0.50 threshold.
        let a = Memory::new(
            MemoryType::Decision,
            "Adopt Redis cluster caching for backend data layer solution",
            "Redis provides blazing fast distributed cache with cluster failover and replication",
        );
        ops::remember(&conn, &a).unwrap();

        let b = Memory::new(
            MemoryType::Decision,
            "Adopt SQLite embedded storage for backend data layer solution",
            "SQLite offers zero-configuration embedded database with ACID transactions and WAL journaling",
        );
        ops::remember(&conn, &b).unwrap();

        // Run full consolidation — Phase 9b should pick these up
        let config = crate::config::ConsolidationConfig::default();
        let stats = run_all_phases(&conn, &config, None);

        assert!(
            stats.contradictions >= 1,
            "run_all_phases should detect content contradictions even with neutral valence, got {}",
            stats.contradictions
        );

        // Verify edge was created
        let edge_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM edge WHERE edge_type = 'contradicts'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(
            edge_count >= 1,
            "should have contradicts edge after full consolidation"
        );
    }

    // ── Phase 2A-4c2 T4: infer_skills_from_behavior tests ────────────────────

    fn seed_session_tool_call_row(
        conn: &Connection,
        id: &str,
        session_id: &str,
        agent: &str,
        tool_name: &str,
        tool_args_json: &str,
        success: i64,
        corr: i64,
        created_at_offset_days: i64,
    ) {
        let sql = format!(
            "INSERT INTO session_tool_call
             (id, session_id, agent, tool_name, tool_args, tool_result_summary,
              success, user_correction_flag, organization_id, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, '', ?6, ?7, 'default',
                     datetime('now', '-{days} days'))",
            days = created_at_offset_days
        );
        conn.execute(
            &sql,
            rusqlite::params![
                id,
                session_id,
                agent,
                tool_name,
                tool_args_json,
                success,
                corr
            ],
        )
        .unwrap();
    }

    fn seed_session(conn: &Connection, id: &str, agent: &str) {
        conn.execute(
            "INSERT INTO session (id, agent, started_at, status, organization_id)
             VALUES (?1, ?2, '2026-04-19 10:00:00', 'active', 'default')",
            rusqlite::params![id, agent],
        )
        .unwrap();
    }

    /// Seed one session with the standard Read+Edit+Bash pattern, clean rows.
    fn seed_clean_sess(conn: &Connection, sid: &str) {
        seed_session(conn, sid, "claude-code");
        seed_session_tool_call_row(
            conn,
            &format!("{sid}-01"),
            sid,
            "claude-code",
            "Read",
            r#"{"file_path":"/tmp/a"}"#,
            1,
            0,
            0,
        );
        seed_session_tool_call_row(
            conn,
            &format!("{sid}-02"),
            sid,
            "claude-code",
            "Edit",
            r#"{"file_path":"/tmp/a","old_string":"x","new_string":"y"}"#,
            1,
            0,
            0,
        );
        seed_session_tool_call_row(
            conn,
            &format!("{sid}-03"),
            sid,
            "claude-code",
            "Bash",
            r#"{"cmd":"cargo test"}"#,
            1,
            0,
            0,
        );
    }

    fn fresh_schema_conn() -> Connection {
        crate::db::vec::init_sqlite_vec();
        let conn = Connection::open_in_memory().unwrap();
        crate::db::schema::create_schema(&conn).unwrap();
        conn
    }

    #[test]
    fn infer_skills_from_behavior_elevates_at_three_sessions() {
        let conn = fresh_schema_conn();
        seed_clean_sess(&conn, "SA");
        seed_clean_sess(&conn, "SB");
        seed_clean_sess(&conn, "SC");

        let elevated = infer_skills_from_behavior(&conn, 3, 30);
        assert_eq!(elevated, 1, "3 matching sessions → 1 skill row elevated");

        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM skill WHERE inferred_at IS NOT NULL",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);

        let inferred_from: String = conn
            .query_row(
                "SELECT inferred_from FROM skill WHERE inferred_at IS NOT NULL",
                [],
                |row| row.get(0),
            )
            .unwrap();
        for sid in ["SA", "SB", "SC"] {
            assert!(
                inferred_from.contains(&format!("\"{sid}\"")),
                "inferred_from missing {sid}: {inferred_from}"
            );
        }
    }

    #[test]
    fn infer_skills_from_behavior_skips_at_two_sessions() {
        let conn = fresh_schema_conn();
        seed_clean_sess(&conn, "SA");
        seed_clean_sess(&conn, "SB");

        let elevated = infer_skills_from_behavior(&conn, 3, 30);
        assert_eq!(elevated, 0);

        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM skill WHERE inferred_at IS NOT NULL",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn infer_skills_from_behavior_skips_corrected_rows() {
        let conn = fresh_schema_conn();
        // SA has a correction on its Edit row — its 3-tool fingerprint won't match SB/SC's.
        seed_session(&conn, "SA", "claude-code");
        seed_session_tool_call_row(
            &conn,
            "SA-01",
            "SA",
            "claude-code",
            "Read",
            r#"{"file_path":"/a"}"#,
            1,
            0,
            0,
        );
        seed_session_tool_call_row(
            &conn,
            "SA-02",
            "SA",
            "claude-code",
            "Edit",
            r#"{"file_path":"/a"}"#,
            1,
            1,
            0,
        ); // corrected
        seed_session_tool_call_row(
            &conn,
            "SA-03",
            "SA",
            "claude-code",
            "Bash",
            r#"{"cmd":"x"}"#,
            1,
            0,
            0,
        );
        seed_clean_sess(&conn, "SB");
        seed_clean_sess(&conn, "SC");

        let elevated = infer_skills_from_behavior(&conn, 3, 30);
        assert_eq!(elevated, 0, "correction taints SA's matching fingerprint");
    }

    #[test]
    fn infer_skills_from_behavior_skips_failed_rows() {
        let conn = fresh_schema_conn();
        seed_session(&conn, "SA", "claude-code");
        seed_session_tool_call_row(
            &conn,
            "SA-01",
            "SA",
            "claude-code",
            "Read",
            r#"{"file_path":"/a"}"#,
            1,
            0,
            0,
        );
        seed_session_tool_call_row(
            &conn,
            "SA-02",
            "SA",
            "claude-code",
            "Edit",
            r#"{"file_path":"/a"}"#,
            0,
            0,
            0,
        ); // failure
        seed_session_tool_call_row(
            &conn,
            "SA-03",
            "SA",
            "claude-code",
            "Bash",
            r#"{"cmd":"x"}"#,
            1,
            0,
            0,
        );
        seed_clean_sess(&conn, "SB");
        seed_clean_sess(&conn, "SC");

        let elevated = infer_skills_from_behavior(&conn, 3, 30);
        assert_eq!(
            elevated, 0,
            "failed rows drop out of clean-filter, SA fingerprint diverges"
        );
    }

    #[test]
    fn infer_skills_from_behavior_skips_rows_outside_window() {
        let conn = fresh_schema_conn();
        seed_session(&conn, "SA", "claude-code");
        seed_session_tool_call_row(
            &conn,
            "SA-01",
            "SA",
            "claude-code",
            "Read",
            r#"{"file_path":"/a"}"#,
            1,
            0,
            60,
        );
        seed_session_tool_call_row(
            &conn,
            "SA-02",
            "SA",
            "claude-code",
            "Edit",
            r#"{"file_path":"/a"}"#,
            1,
            0,
            60,
        );
        seed_session_tool_call_row(
            &conn,
            "SA-03",
            "SA",
            "claude-code",
            "Bash",
            r#"{"cmd":"x"}"#,
            1,
            0,
            60,
        );
        seed_clean_sess(&conn, "SB");
        seed_clean_sess(&conn, "SC");

        let elevated = infer_skills_from_behavior(&conn, 3, 30);
        assert_eq!(
            elevated, 0,
            "SA outside window → only SB+SC match, below threshold"
        );
    }

    #[test]
    fn infer_skills_from_behavior_merges_inferred_from_on_conflict() {
        let conn = fresh_schema_conn();
        seed_clean_sess(&conn, "SA");
        seed_clean_sess(&conn, "SB");
        seed_clean_sess(&conn, "SC");
        assert_eq!(infer_skills_from_behavior(&conn, 3, 30), 1);

        seed_clean_sess(&conn, "SD");
        let second = infer_skills_from_behavior(&conn, 3, 30);
        assert_eq!(second, 1, "upsert returns 1 affected row");

        let inferred_from: String = conn
            .query_row(
                "SELECT inferred_from FROM skill WHERE inferred_at IS NOT NULL",
                [],
                |row| row.get(0),
            )
            .unwrap();
        for sid in ["SA", "SB", "SC", "SD"] {
            assert!(
                inferred_from.contains(&format!("\"{sid}\"")),
                "merged inferred_from missing {sid}: {inferred_from}"
            );
        }

        let total_rows: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM skill WHERE inferred_at IS NOT NULL",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(total_rows, 1, "upsert must not create a duplicate row");
    }

    #[test]
    fn infer_skills_from_behavior_idempotent_on_rerun() {
        let conn = fresh_schema_conn();
        seed_clean_sess(&conn, "SA");
        seed_clean_sess(&conn, "SB");
        seed_clean_sess(&conn, "SC");
        let first = infer_skills_from_behavior(&conn, 3, 30);
        let second = infer_skills_from_behavior(&conn, 3, 30);
        assert_eq!(first, 1);
        assert_eq!(second, 1, "re-run upserts same row, no duplicate");

        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM skill WHERE inferred_at IS NOT NULL",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn infer_skills_from_behavior_separates_fingerprints() {
        let conn = fresh_schema_conn();
        seed_clean_sess(&conn, "A1");
        seed_clean_sess(&conn, "A2");
        seed_clean_sess(&conn, "A3");
        for sid in ["B1", "B2", "B3"] {
            seed_session(&conn, sid, "claude-code");
            seed_session_tool_call_row(
                &conn,
                &format!("{sid}-01"),
                sid,
                "claude-code",
                "Grep",
                r#"{"pattern":"x"}"#,
                1,
                0,
                0,
            );
            seed_session_tool_call_row(
                &conn,
                &format!("{sid}-02"),
                sid,
                "claude-code",
                "Write",
                r#"{"file_path":"/tmp/z","content":"q"}"#,
                1,
                0,
                0,
            );
        }

        let elevated = infer_skills_from_behavior(&conn, 3, 30);
        assert_eq!(elevated, 2, "two distinct fingerprints each elevate");
    }

    #[test]
    fn infer_skills_from_behavior_separates_agents() {
        let conn = fresh_schema_conn();
        seed_clean_sess(&conn, "C1");
        seed_clean_sess(&conn, "C2");
        seed_clean_sess(&conn, "C3");
        for sid in ["X1", "X2", "X3"] {
            seed_session(&conn, sid, "codex-cli");
            seed_session_tool_call_row(
                &conn,
                &format!("{sid}-01"),
                sid,
                "codex-cli",
                "Read",
                r#"{"file_path":"/tmp/a"}"#,
                1,
                0,
                0,
            );
            seed_session_tool_call_row(
                &conn,
                &format!("{sid}-02"),
                sid,
                "codex-cli",
                "Edit",
                r#"{"file_path":"/tmp/a","old_string":"x","new_string":"y"}"#,
                1,
                0,
                0,
            );
            seed_session_tool_call_row(
                &conn,
                &format!("{sid}-03"),
                sid,
                "codex-cli",
                "Bash",
                r#"{"cmd":"cargo test"}"#,
                1,
                0,
                0,
            );
        }

        let elevated = infer_skills_from_behavior(&conn, 3, 30);
        assert_eq!(
            elevated, 2,
            "same fingerprint on two different agents → two rows"
        );

        let rows: Vec<String> = conn
            .prepare("SELECT agent FROM skill WHERE inferred_at IS NOT NULL ORDER BY agent")
            .unwrap()
            .query_map([], |row| row.get::<_, String>(0))
            .unwrap()
            .filter_map(|r| r.ok())
            .collect();
        assert_eq!(
            rows,
            vec!["claude-code".to_string(), "codex-cli".to_string()]
        );
    }

    // ── Phase 2A-4c2 T10 Codex-H2 regression guard ───────────────────────────

    fn seed_session_with_project(conn: &Connection, id: &str, agent: &str, project: &str) {
        conn.execute(
            "INSERT INTO session (id, agent, project, started_at, status, organization_id)
             VALUES (?1, ?2, ?3, '2026-04-19 10:00:00', 'active', 'default')",
            rusqlite::params![id, agent, project],
        )
        .unwrap();
    }

    fn seed_clean_tool_calls(conn: &Connection, sid: &str, agent: &str) {
        seed_session_tool_call_row(
            conn,
            &format!("{sid}-01"),
            sid,
            agent,
            "Read",
            r#"{"file_path":"/tmp/a"}"#,
            1,
            0,
            0,
        );
        seed_session_tool_call_row(
            conn,
            &format!("{sid}-02"),
            sid,
            agent,
            "Edit",
            r#"{"file_path":"/tmp/a","old_string":"x","new_string":"y"}"#,
            1,
            0,
            0,
        );
        seed_session_tool_call_row(
            conn,
            &format!("{sid}-03"),
            sid,
            agent,
            "Bash",
            r#"{"cmd":"cargo test"}"#,
            1,
            0,
            0,
        );
    }

    #[test]
    fn infer_skills_from_behavior_scopes_per_project() {
        // T10 Codex-H2: the same tool-call pattern used in three sessions of
        // project "alpha" plus three sessions of project "beta" must elevate
        // into two rows (one per project), not a single global row. Otherwise
        // project alpha's pattern leaks into beta's compiled context.
        let conn = fresh_schema_conn();
        for sid in ["A1", "A2", "A3"] {
            seed_session_with_project(&conn, sid, "claude-code", "alpha");
            seed_clean_tool_calls(&conn, sid, "claude-code");
        }
        for sid in ["B1", "B2", "B3"] {
            seed_session_with_project(&conn, sid, "claude-code", "beta");
            seed_clean_tool_calls(&conn, sid, "claude-code");
        }

        let elevated = infer_skills_from_behavior(&conn, 3, 30);
        assert_eq!(elevated, 2, "same fingerprint in two projects → two rows");

        let projects: Vec<String> = conn
            .prepare("SELECT project FROM skill WHERE inferred_at IS NOT NULL ORDER BY project")
            .unwrap()
            .query_map([], |row| row.get::<_, Option<String>>(0))
            .unwrap()
            .filter_map(|r| r.ok())
            .map(|p| p.unwrap_or_default())
            .collect();
        assert_eq!(projects, vec!["alpha".to_string(), "beta".to_string()]);
    }

    #[test]
    fn infer_skills_from_behavior_prunes_aged_out_session_ids() {
        // 2P-1b §14 (Codex-MED): on re-run, inferred_from must reflect only
        // sessions in the current window — old IDs that aged out of the
        // window should NOT accumulate forever.
        let conn = fresh_schema_conn();

        // Seed 3 sessions 60 days ago.
        for sid in ["OLD1", "OLD2", "OLD3"] {
            seed_session(&conn, sid, "claude-code");
            seed_session_tool_call_row(
                &conn, &format!("{sid}-01"), sid, "claude-code",
                "Read", r#"{"file_path":"/tmp/a"}"#, 1, 0, 60,
            );
            seed_session_tool_call_row(
                &conn, &format!("{sid}-02"), sid, "claude-code",
                "Edit", r#"{"file_path":"/tmp/a","old_string":"x","new_string":"y"}"#,
                1, 0, 60,
            );
            seed_session_tool_call_row(
                &conn, &format!("{sid}-03"), sid, "claude-code",
                "Bash", r#"{"cmd":"cargo test"}"#, 1, 0, 60,
            );
        }
        // Run with a 90-day window — all 3 old sessions qualify.
        assert_eq!(infer_skills_from_behavior(&conn, 3, 90), 1);

        // Now add 3 fresh sessions (today) and re-run with 30-day window.
        // OLD* should NOT appear in inferred_from anymore.
        seed_clean_sess(&conn, "NEW1");
        seed_clean_sess(&conn, "NEW2");
        seed_clean_sess(&conn, "NEW3");
        let _ = infer_skills_from_behavior(&conn, 3, 30);

        let inferred_from: String = conn
            .query_row(
                "SELECT inferred_from FROM skill WHERE inferred_at IS NOT NULL",
                [],
                |row| row.get(0),
            )
            .unwrap();
        for sid in ["NEW1", "NEW2", "NEW3"] {
            assert!(
                inferred_from.contains(&format!("\"{sid}\"")),
                "inferred_from missing current-window session {sid}: {inferred_from}"
            );
        }
        for sid in ["OLD1", "OLD2", "OLD3"] {
            assert!(
                !inferred_from.contains(&format!("\"{sid}\"")),
                "inferred_from leaked aged-out session {sid}: {inferred_from}"
            );
        }
    }

    #[test]
    fn infer_skills_from_behavior_survives_malformed_inferred_from_on_merge() {
        // T10 Claude-H2 regression guard (2P-1b §17): if a pre-existing skill
        // row has a non-JSON inferred_from (manual DB edit, pre-Phase-23
        // writer), the UPDATE merge must not explode. Fall back to the new
        // value rather than crashing json_each().
        let conn = fresh_schema_conn();
        seed_clean_sess(&conn, "SA");
        seed_clean_sess(&conn, "SB");
        seed_clean_sess(&conn, "SC");
        let elevated = infer_skills_from_behavior(&conn, 3, 30);
        assert_eq!(elevated, 1);

        // Corrupt inferred_from out-of-band.
        conn.execute(
            "UPDATE skill SET inferred_from = 'not-json-at-all' \
             WHERE inferred_at IS NOT NULL",
            [],
        )
        .unwrap();

        // Add a fourth matching session + re-run. Should not panic/error;
        // row gets refreshed with the new JSON value.
        seed_clean_sess(&conn, "SD");
        let _ = infer_skills_from_behavior(&conn, 3, 30);

        let inferred_from: String = conn
            .query_row(
                "SELECT inferred_from FROM skill WHERE inferred_at IS NOT NULL",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(
            serde_json::from_str::<serde_json::Value>(&inferred_from).is_ok(),
            "inferred_from should be valid JSON after recovery; got {inferred_from}"
        );
    }

    #[test]
    fn infer_skills_from_behavior_inserts_behavioral_skill_type() {
        // T10 Codex-H1: inferred skills must be stored with skill_type='behavioral'
        // so prune_junk_skills does not wipe them on daemon restart.
        let conn = fresh_schema_conn();
        seed_clean_sess(&conn, "SA");
        seed_clean_sess(&conn, "SB");
        seed_clean_sess(&conn, "SC");

        let elevated = infer_skills_from_behavior(&conn, 3, 30);
        assert_eq!(elevated, 1);

        let skill_type: String = conn
            .query_row(
                "SELECT skill_type FROM skill WHERE inferred_at IS NOT NULL",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(skill_type, "behavioral");
    }
}
