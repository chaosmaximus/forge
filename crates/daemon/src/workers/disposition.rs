// workers/disposition.rs — Disposition trait updater
//
// Analyzes recent session history and slowly adjusts agent disposition traits
// (Manas Layer 7). Changes are capped at +/-0.05 per cycle — the "wide turning
// arc" from Sadhguru's teaching: character changes slowly, through evidence.

use crate::db::manas;
use forge_core::types::{Disposition, DispositionTrait, Trend};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{watch, Mutex};

// Interval is now configurable via ForgeConfig.workers.disposition_interval_secs
// (default: 900 = 15 minutes)

/// Maximum change per cycle.
const MAX_DELTA: f64 = 0.05;

/// Default trait value for new dispositions.
const DEFAULT_VALUE: f64 = 0.5;

/// Default agent name for disposition tracking (fallback when no sessions exist).
/// Uses "claude-code" as the fallback since it's the most common agent type.
const DEFAULT_AGENT_NAME: &str = "claude-code";

/// Sessions shorter than this (seconds) are considered "short" — may indicate errors/restarts.
const SHORT_SESSION_THRESHOLD_SECS: i64 = 60;

/// Sessions longer than this (seconds) are considered "long" — thorough work.
const LONG_SESSION_THRESHOLD_SECS: i64 = 600;

pub async fn run_disposition(
    state: Arc<Mutex<crate::server::handler::DaemonState>>,
    mut shutdown_rx: watch::Receiver<bool>,
    db_path: String,
    interval_secs: u64,
) {
    let interval = Duration::from_secs(interval_secs);
    eprintln!(
        "[disposition] started, interval = {:?}",
        interval
    );

    loop {
        tokio::select! {
            _ = tokio::time::sleep(interval) => {
                tick(&state, &db_path).await;
            }
            _ = shutdown_rx.changed() => {
                eprintln!("[disposition] shutting down");
                return;
            }
        }
    }
}

async fn tick(state: &Arc<Mutex<crate::server::handler::DaemonState>>, db_path: &str) {
    // Use read-only connection for agent discovery (SELECT queries)
    let active_agents = if let Some(rc) = super::open_read_conn(db_path) {
        let agents = query_active_agents(&rc);
        drop(rc);
        agents
    } else {
        let locked = state.lock().await;
        let agents = query_active_agents(&locked.conn);
        drop(locked);
        agents
    };

    // query_active_agents always returns at least DEFAULT_AGENT_NAME, so this is defensive
    if active_agents.is_empty() {
        eprintln!("[disposition] WARN: no agents found at all — this should not happen");
        return;
    }

    // Lock state for writes (tick_for_agent does both reads and writes — store_disposition)
    let locked = state.lock().await;
    for agent_name in &active_agents {
        tick_for_agent(&locked.conn, agent_name);
    }
}

/// Compute and store disposition traits for a single agent.
fn tick_for_agent(conn: &rusqlite::Connection, agent_name: &str) {
    // 1. Query recent sessions (last 24 hours) for this agent
    let sessions = match query_recent_sessions_for_agent(conn, agent_name) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("[disposition] session query error for {}: {}", agent_name, e);
            return;
        }
    };

    if sessions.is_empty() {
        eprintln!("[disposition] no sessions found for agent '{}' — cannot compute traits", agent_name);
        return;
    }

    // 2. Compute heuristics
    let total = sessions.len();
    let short_count = sessions.iter().filter(|s| s.duration_secs < SHORT_SESSION_THRESHOLD_SECS).count();
    let long_count = sessions.iter().filter(|s| s.duration_secs >= LONG_SESSION_THRESHOLD_SECS).count();
    let short_ratio = short_count as f64 / total as f64;
    let long_ratio = long_count as f64 / total as f64;

    // 3. Compute deltas (capped at MAX_DELTA)
    //    Caution: rises when many short sessions (errors/restarts), falls when stable
    let caution_delta = if short_ratio > 0.5 {
        MAX_DELTA * short_ratio
    } else {
        -MAX_DELTA * 0.5
    }
    .clamp(-MAX_DELTA, MAX_DELTA);

    //    Thoroughness: rises with long sessions, falls with short ones
    let thoroughness_delta = if long_ratio > 0.3 {
        MAX_DELTA * long_ratio
    } else if short_ratio > 0.5 {
        -MAX_DELTA * short_ratio
    } else {
        0.0
    }
    .clamp(-MAX_DELTA, MAX_DELTA);

    // 4. Load current values and update
    let caution_current = get_current_value(conn, agent_name, DispositionTrait::Caution);
    let thoroughness_current = get_current_value(conn, agent_name, DispositionTrait::Thoroughness);

    let new_caution = (caution_current + caution_delta).clamp(0.0, 1.0);
    let new_thoroughness = (thoroughness_current + thoroughness_delta).clamp(0.0, 1.0);

    let evidence = vec![format!(
        "agent={} sessions={} short={} long={} short_ratio={:.2} long_ratio={:.2}",
        agent_name, total, short_count, long_count, short_ratio, long_ratio
    )];

    // 5. Store updated dispositions
    let now = manas::now_offset(0);

    let caution = Disposition {
        id: format!("{}-caution", agent_name),
        agent: agent_name.to_string(),
        disposition_trait: DispositionTrait::Caution,
        domain: None,
        value: new_caution,
        trend: compute_trend(caution_current, new_caution),
        updated_at: now.clone(),
        evidence: evidence.clone(),
    };

    let thoroughness = Disposition {
        id: format!("{}-thoroughness", agent_name),
        agent: agent_name.to_string(),
        disposition_trait: DispositionTrait::Thoroughness,
        domain: None,
        value: new_thoroughness,
        trend: compute_trend(thoroughness_current, new_thoroughness),
        updated_at: now,
        evidence,
    };

    if let Err(e) = manas::store_disposition(conn, &caution) {
        eprintln!("[disposition] store caution error for {}: {}", agent_name, e);
    }
    if let Err(e) = manas::store_disposition(conn, &thoroughness) {
        eprintln!("[disposition] store thoroughness error for {}: {}", agent_name, e);
    }

    eprintln!(
        "[disposition] updated {}: caution={:.3} ({:?}), thoroughness={:.3} ({:?})",
        agent_name, new_caution, caution.trend, new_thoroughness, thoroughness.trend
    );
}

/// A minimal session summary for heuristic analysis.
struct SessionSummary {
    duration_secs: i64,
}

/// Query distinct agent names from sessions in the last 24 hours (active or recently ended).
fn query_active_agents(conn: &rusqlite::Connection) -> Vec<String> {
    let cutoff = manas::now_offset(-86400);
    let mut stmt = match conn.prepare(
        "SELECT DISTINCT agent FROM session WHERE status = 'active' OR started_at > ?1"
    ) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("[disposition] WARN: failed to prepare agent query: {e}");
            return vec![DEFAULT_AGENT_NAME.to_string()];
        }
    };
    let rows = match stmt.query_map(rusqlite::params![cutoff], |row| row.get(0)) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("[disposition] WARN: failed to query agents: {e}");
            return vec![DEFAULT_AGENT_NAME.to_string()];
        }
    };
    let agents: Vec<String> = rows.filter_map(|r| r.ok()).collect();
    if agents.is_empty() {
        eprintln!("[disposition] no active agents found, using default: {}", DEFAULT_AGENT_NAME);
        vec![DEFAULT_AGENT_NAME.to_string()]
    } else {
        agents
    }
}

/// Query recent sessions for a specific agent.
/// Includes: sessions from last 24h OR still-active sessions (regardless of age).
fn query_recent_sessions_for_agent(
    conn: &rusqlite::Connection,
    agent: &str,
) -> rusqlite::Result<Vec<SessionSummary>> {
    let cutoff = manas::now_offset(-86400); // 24 hours ago

    let mut stmt = conn.prepare(
        "SELECT started_at, ended_at FROM session
         WHERE agent = ?1 AND (started_at > ?2 OR (status = 'active' AND ended_at IS NULL))
         ORDER BY started_at DESC",
    )?;

    let rows = stmt.query_map(rusqlite::params![agent, cutoff], |row| {
        let started: String = row.get(0)?;
        let ended: Option<String> = row.get(1)?;
        Ok((started, ended))
    })?;

    let mut sessions = Vec::new();
    for row in rows {
        let (started, ended) = row?;
        let duration = match ended {
            Some(ref e) => estimate_duration_secs(&started, e),
            None => {
                // Active session — estimate from started_at to now
                let now = manas::now_offset(0);
                estimate_duration_secs(&started, &now)
            }
        };
        sessions.push(SessionSummary {
            duration_secs: duration,
        });
    }
    Ok(sessions)
}

/// Estimate session duration in seconds from two ISO timestamp strings.
/// Falls back to 0 if parsing fails.
fn estimate_duration_secs(start: &str, end: &str) -> i64 {
    // Simple approach: parse "YYYY-MM-DD HH:MM:SS" to epoch-ish comparison
    // We only need relative difference, so just compare the raw strings
    // as they're in ISO format and lexicographically comparable.
    // For more precise duration we'd need full parsing.
    // Use a simple heuristic: parse HH:MM:SS from both and compute difference.
    let start_secs = parse_time_component(start);
    let end_secs = parse_time_component(end);

    // If dates differ, we can detect by comparing date portions
    let start_date = start.get(..10).unwrap_or("");
    let end_date = end.get(..10).unwrap_or("");

    if start_date == end_date {
        (end_secs - start_secs).max(0)
    } else {
        // Cross-day: estimate by adding full-day seconds
        // Simple: parse date diff roughly
        let date_diff_days = estimate_date_diff(start_date, end_date);
        (date_diff_days * 86400 + end_secs - start_secs).max(0)
    }
}

/// Parse the time component (HH:MM:SS) from an ISO datetime string to seconds since midnight.
fn parse_time_component(s: &str) -> i64 {
    // Format: "YYYY-MM-DD HH:MM:SS"
    let time_part = if s.len() >= 19 {
        &s[11..19]
    } else {
        return 0;
    };
    let parts: Vec<&str> = time_part.split(':').collect();
    if parts.len() != 3 {
        return 0;
    }
    let h: i64 = parts[0].parse().unwrap_or(0);
    let m: i64 = parts[1].parse().unwrap_or(0);
    let sec: i64 = parts[2].parse().unwrap_or(0);
    h * 3600 + m * 60 + sec
}

/// Rough date difference in days between two "YYYY-MM-DD" strings.
fn estimate_date_diff(start: &str, end: &str) -> i64 {
    // Simple: just use ordinal day-of-year approximation
    let s_day = rough_day_number(start);
    let e_day = rough_day_number(end);
    (e_day - s_day).max(0)
}

fn rough_day_number(date_str: &str) -> i64 {
    if date_str.len() < 10 {
        return 0;
    }
    let y: i64 = date_str[..4].parse().unwrap_or(0);
    let m: i64 = date_str[5..7].parse().unwrap_or(1);
    let d: i64 = date_str[8..10].parse().unwrap_or(1);
    y * 365 + m * 30 + d
}

/// Get current value for a disposition trait, defaulting to DEFAULT_VALUE.
fn get_current_value(
    conn: &rusqlite::Connection,
    agent_name: &str,
    trait_type: DispositionTrait,
) -> f64 {
    let id = match trait_type {
        DispositionTrait::Caution => format!("{}-caution", agent_name),
        DispositionTrait::Thoroughness => format!("{}-thoroughness", agent_name),
        DispositionTrait::Autonomy => format!("{}-autonomy", agent_name),
        DispositionTrait::Verbosity => format!("{}-verbosity", agent_name),
        DispositionTrait::Creativity => format!("{}-creativity", agent_name),
    };

    match conn.query_row(
        "SELECT value FROM disposition WHERE id = ?1",
        rusqlite::params![id],
        |row| row.get::<_, f64>(0),
    ) {
        Ok(v) => v,
        Err(rusqlite::Error::QueryReturnedNoRows) => DEFAULT_VALUE, // Expected: first-time trait
        Err(e) => {
            eprintln!("[disposition] WARN: failed to read {:?} for {}: {e}", trait_type, agent_name);
            DEFAULT_VALUE
        }
    }
}

/// Determine trend from old→new value.
fn compute_trend(old: f64, new: f64) -> Trend {
    let diff = new - old;
    if diff > 0.001 {
        Trend::Rising
    } else if diff < -0.001 {
        Trend::Falling
    } else {
        Trend::Stable
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::schema;

    fn open_db() -> rusqlite::Connection {
        crate::db::vec::init_sqlite_vec();
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        schema::create_schema(&conn).unwrap();
        conn
    }

    #[test]
    fn test_disposition_initial_values() {
        let conn = open_db();
        // Before any disposition is stored, get_current_value should return DEFAULT_VALUE (0.5)
        let caution = get_current_value(&conn, DEFAULT_AGENT_NAME, DispositionTrait::Caution);
        assert!(
            (caution - DEFAULT_VALUE).abs() < f64::EPSILON,
            "initial caution should be {}",
            DEFAULT_VALUE
        );

        let thoroughness = get_current_value(&conn, DEFAULT_AGENT_NAME, DispositionTrait::Thoroughness);
        assert!(
            (thoroughness - DEFAULT_VALUE).abs() < f64::EPSILON,
            "initial thoroughness should be {}",
            DEFAULT_VALUE
        );
    }

    #[test]
    fn test_disposition_clamping() {
        let conn = open_db();

        // Store a disposition with value near 1.0
        let d = Disposition {
            id: format!("{}-caution", DEFAULT_AGENT_NAME),
            agent: DEFAULT_AGENT_NAME.to_string(),
            disposition_trait: DispositionTrait::Caution,
            domain: None,
            value: 0.98,
            trend: Trend::Rising,
            updated_at: manas::now_offset(0),
            evidence: vec![],
        };
        manas::store_disposition(&conn, &d).unwrap();

        // Simulate adding MAX_DELTA — should clamp to 1.0
        let current = get_current_value(&conn, DEFAULT_AGENT_NAME, DispositionTrait::Caution);
        let new_val = (current + MAX_DELTA).clamp(0.0, 1.0);
        assert!(new_val <= 1.0, "value must not exceed 1.0, got {}", new_val);
        assert!(
            (new_val - 1.0).abs() < f64::EPSILON,
            "0.98 + 0.05 should clamp to 1.0"
        );

        // Store a disposition with value near 0.0
        let d2 = Disposition {
            id: format!("{}-thoroughness", DEFAULT_AGENT_NAME),
            agent: DEFAULT_AGENT_NAME.to_string(),
            disposition_trait: DispositionTrait::Thoroughness,
            domain: None,
            value: 0.02,
            trend: Trend::Falling,
            updated_at: manas::now_offset(0),
            evidence: vec![],
        };
        manas::store_disposition(&conn, &d2).unwrap();

        // Simulate subtracting MAX_DELTA — should clamp to 0.0
        let current2 = get_current_value(&conn, DEFAULT_AGENT_NAME, DispositionTrait::Thoroughness);
        let new_val2 = (current2 - MAX_DELTA).clamp(0.0, 1.0);
        assert!(
            new_val2 >= 0.0,
            "value must not go below 0.0, got {}",
            new_val2
        );
        assert!(
            new_val2.abs() < f64::EPSILON,
            "0.02 - 0.05 should clamp to 0.0"
        );
    }

    #[test]
    fn test_compute_trend() {
        assert_eq!(compute_trend(0.5, 0.55), Trend::Rising);
        assert_eq!(compute_trend(0.5, 0.45), Trend::Falling);
        assert_eq!(compute_trend(0.5, 0.5), Trend::Stable);
        assert_eq!(compute_trend(0.5, 0.5001), Trend::Stable); // within threshold
    }

    #[test]
    fn test_parse_time_component() {
        assert_eq!(parse_time_component("2026-04-03 12:30:45"), 45045);
        assert_eq!(parse_time_component("2026-04-03 00:00:00"), 0);
        assert_eq!(parse_time_component("short"), 0);
    }

    #[test]
    fn test_estimate_duration_same_day() {
        let dur =
            estimate_duration_secs("2026-04-03 10:00:00", "2026-04-03 10:15:00");
        assert_eq!(dur, 900); // 15 minutes
    }

    #[test]
    fn test_max_delta_cap() {
        // Verify the delta calculation never exceeds MAX_DELTA
        let delta = (MAX_DELTA * 0.9).clamp(-MAX_DELTA, MAX_DELTA);
        assert!(delta.abs() <= MAX_DELTA);

        let delta2 = (MAX_DELTA * 1.5).clamp(-MAX_DELTA, MAX_DELTA);
        assert!((delta2 - MAX_DELTA).abs() < f64::EPSILON);
    }

    #[test]
    fn test_tick_for_agent_produces_traits() {
        let conn = open_db();
        // Insert a recent active session
        conn.execute(
            "INSERT INTO session (id, agent, project, cwd, status, started_at) VALUES (?1, ?2, ?3, ?4, 'active', ?5)",
            rusqlite::params!["test-session-1", "claude-code", "test-project", "/tmp", manas::now_offset(-300)],
        ).unwrap();

        // Run the tick
        tick_for_agent(&conn, "claude-code");

        // Verify traits were stored
        let caution: f64 = conn.query_row(
            "SELECT value FROM disposition WHERE id = 'claude-code-caution'",
            [],
            |row| row.get(0),
        ).expect("caution trait should exist after tick");
        assert!(caution >= 0.0 && caution <= 1.0, "caution should be in [0,1], got {}", caution);

        let thoroughness: f64 = conn.query_row(
            "SELECT value FROM disposition WHERE id = 'claude-code-thoroughness'",
            [],
            |row| row.get(0),
        ).expect("thoroughness trait should exist after tick");
        assert!(thoroughness >= 0.0 && thoroughness <= 1.0, "thoroughness should be in [0,1], got {}", thoroughness);
    }

    #[test]
    fn test_tick_for_agent_with_old_active_sessions() {
        let conn = open_db();
        // Insert an OLD active session (48 hours ago) — previously would have been missed
        conn.execute(
            "INSERT INTO session (id, agent, project, cwd, status, started_at) VALUES (?1, ?2, ?3, ?4, 'active', ?5)",
            rusqlite::params!["old-session", "claude-code", "test-project", "/tmp", manas::now_offset(-172800)],
        ).unwrap();

        // Run the tick — should find the old active session
        tick_for_agent(&conn, "claude-code");

        // Verify traits were stored (not skipped due to empty sessions)
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM disposition WHERE agent = 'claude-code'",
            [],
            |row| row.get(0),
        ).unwrap();
        assert!(count >= 2, "should have at least 2 traits (caution + thoroughness), got {}", count);
    }

    #[test]
    fn test_query_active_agents_finds_active_sessions() {
        let conn = open_db();
        // Insert an active session
        conn.execute(
            "INSERT INTO session (id, agent, project, cwd, status, started_at) VALUES (?1, ?2, ?3, ?4, 'active', ?5)",
            rusqlite::params!["sess-1", "claude-code", "proj", "/tmp", manas::now_offset(-7200)],
        ).unwrap();

        let agents = query_active_agents(&conn);
        assert!(agents.contains(&"claude-code".to_string()), "should find claude-code agent");
    }

    #[test]
    fn test_query_active_agents_defaults_when_empty() {
        let conn = open_db();
        // No sessions at all
        let agents = query_active_agents(&conn);
        assert_eq!(agents, vec![DEFAULT_AGENT_NAME.to_string()], "should fallback to default agent");
    }
}
