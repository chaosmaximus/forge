use rusqlite::{Connection, OptionalExtension, params};

/// Record that a context injection was delivered to a session.
/// Returns the ULID of the new row.
pub fn record_injection(
    conn: &Connection,
    session_id: &str,
    hook_event: &str,
    context_type: &str,
    content_summary: &str,
) -> rusqlite::Result<String> {
    record_injection_with_size(conn, session_id, hook_event, context_type, content_summary, 0)
}

/// Record a context injection with its character count for observability.
pub fn record_injection_with_size(
    conn: &Connection,
    session_id: &str,
    hook_event: &str,
    context_type: &str,
    content_summary: &str,
    chars_injected: usize,
) -> rusqlite::Result<String> {
    let id = ulid::Ulid::new().to_string();
    conn.execute(
        "INSERT INTO context_effectiveness (id, session_id, hook_event, context_type, content_summary, chars_injected)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![id, session_id, hook_event, context_type, content_summary, chars_injected as i64],
    )?;
    Ok(id)
}

/// Get injection stats for a session: total injections, total chars, per-hook breakdown.
pub fn session_injection_stats(conn: &Connection, session_id: &str) -> rusqlite::Result<InjectionStats> {
    let total_chars: i64 = conn.query_row(
        "SELECT COALESCE(SUM(chars_injected), 0) FROM context_effectiveness WHERE session_id = ?1",
        params![session_id],
        |row| row.get(0),
    )?;
    let total_injections: i64 = conn.query_row(
        "SELECT COUNT(*) FROM context_effectiveness WHERE session_id = ?1",
        params![session_id],
        |row| row.get(0),
    )?;
    let acknowledged: i64 = conn.query_row(
        "SELECT COUNT(*) FROM context_effectiveness WHERE session_id = ?1 AND acknowledged = 1",
        params![session_id],
        |row| row.get(0),
    )?;

    let mut stmt = conn.prepare(
        "SELECT hook_event, COUNT(*), COALESCE(SUM(chars_injected), 0)
         FROM context_effectiveness WHERE session_id = ?1
         GROUP BY hook_event ORDER BY SUM(chars_injected) DESC"
    )?;
    let per_hook: Vec<HookStats> = stmt.query_map(params![session_id], |row| {
        Ok(HookStats {
            hook_event: row.get(0)?,
            injections: row.get::<_, i64>(1)? as usize,
            chars: row.get::<_, i64>(2)? as usize,
        })
    })?.filter_map(|r| r.ok()).collect();

    Ok(InjectionStats {
        session_id: session_id.to_string(),
        total_injections: total_injections as usize,
        total_chars: total_chars as usize,
        estimated_tokens: total_chars as usize / 4,
        acknowledged: acknowledged as usize,
        effectiveness_rate: if total_injections > 0 { acknowledged as f64 / total_injections as f64 } else { 0.0 },
        per_hook,
    })
}

/// Get global injection stats across all sessions.
pub fn global_injection_stats(conn: &Connection) -> rusqlite::Result<GlobalStats> {
    let total_chars: i64 = conn.query_row(
        "SELECT COALESCE(SUM(chars_injected), 0) FROM context_effectiveness",
        [],
        |row| row.get(0),
    )?;
    let total_injections: i64 = conn.query_row(
        "SELECT COUNT(*) FROM context_effectiveness",
        [],
        |row| row.get(0),
    )?;
    let total_sessions: i64 = conn.query_row(
        "SELECT COUNT(DISTINCT session_id) FROM context_effectiveness",
        [],
        |row| row.get(0),
    )?;
    let acknowledged: i64 = conn.query_row(
        "SELECT COUNT(*) FROM context_effectiveness WHERE acknowledged = 1",
        [],
        |row| row.get(0),
    )?;

    Ok(GlobalStats {
        total_sessions: total_sessions as usize,
        total_injections: total_injections as usize,
        total_chars: total_chars as usize,
        estimated_tokens: total_chars as usize / 4,
        acknowledged: acknowledged as usize,
        effectiveness_rate: if total_injections > 0 { acknowledged as f64 / total_injections as f64 } else { 0.0 },
        avg_chars_per_session: if total_sessions > 0 { total_chars as usize / total_sessions as usize } else { 0 },
    })
}

#[derive(Debug)]
pub struct HookStats {
    pub hook_event: String,
    pub injections: usize,
    pub chars: usize,
}

#[derive(Debug)]
pub struct InjectionStats {
    pub session_id: String,
    pub total_injections: usize,
    pub total_chars: usize,
    pub estimated_tokens: usize,
    pub acknowledged: usize,
    pub effectiveness_rate: f64,
    pub per_hook: Vec<HookStats>,
}

#[derive(Debug)]
pub struct GlobalStats {
    pub total_sessions: usize,
    pub total_injections: usize,
    pub total_chars: usize,
    pub estimated_tokens: usize,
    pub acknowledged: usize,
    pub effectiveness_rate: f64,
    pub avg_chars_per_session: usize,
}

/// Mark a previously-recorded injection as acknowledged by the agent.
/// Returns true if a row was updated, false if the id was not found.
pub fn mark_acknowledged(conn: &Connection, id: &str) -> rusqlite::Result<bool> {
    let rows = conn.execute(
        "UPDATE context_effectiveness SET acknowledged = 1 WHERE id = ?1",
        params![id],
    )?;
    Ok(rows > 0)
}

/// Compute the acknowledgement rate for a given (hook_event, context_type) pair,
/// optionally scoped to a project (matched via session_id prefix or join).
///
/// Returns `Some(rate)` when there are at least 10 samples, `None` otherwise.
pub fn effectiveness_rate(
    conn: &Connection,
    hook_event: &str,
    context_type: &str,
    _project: Option<&str>,
) -> rusqlite::Result<Option<f64>> {
    // For now, project filtering is not yet wired (requires a join on the
    // session table). The parameter is accepted to keep the API stable.
    let row: Option<(i64, i64)> = conn
        .query_row(
            "SELECT COUNT(*), SUM(acknowledged)
             FROM context_effectiveness
             WHERE hook_event = ?1 AND context_type = ?2",
            params![hook_event, context_type],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;

    match row {
        Some((total, acked)) if total >= 10 => Ok(Some(acked as f64 / total as f64)),
        _ => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    fn setup() -> Connection {
        crate::db::vec::init_sqlite_vec();
        let conn = Connection::open_in_memory().unwrap();
        crate::db::schema::create_schema(&conn).unwrap();
        conn
    }

    #[test]
    fn test_record_and_acknowledge() {
        let conn = setup();

        let id = record_injection(
            &conn,
            "sess-1",
            "session-start",
            "decision",
            "Use Postgres for persistence",
        )
        .unwrap();

        // Verify the row exists and is not yet acknowledged
        let (ack, summary): (i64, String) = conn
            .query_row(
                "SELECT acknowledged, content_summary FROM context_effectiveness WHERE id = ?1",
                params![id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(ack, 0);
        assert_eq!(summary, "Use Postgres for persistence");

        // Acknowledge it
        let updated = mark_acknowledged(&conn, &id).unwrap();
        assert!(updated);

        // Verify acknowledged = 1
        let ack: i64 = conn
            .query_row(
                "SELECT acknowledged FROM context_effectiveness WHERE id = ?1",
                params![id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(ack, 1);

        // Acknowledging a non-existent id returns false
        let updated = mark_acknowledged(&conn, "nonexistent").unwrap();
        assert!(!updated);
    }

    #[test]
    fn test_effectiveness_rate_with_samples() {
        let conn = setup();

        // Insert 12 injections, acknowledge 9 of them => rate = 0.75
        let mut ids = Vec::new();
        for i in 0..12 {
            let id = record_injection(
                &conn,
                &format!("sess-{}", i),
                "post-edit",
                "pattern",
                &format!("pattern hint {}", i),
            )
            .unwrap();
            ids.push(id);
        }

        // Acknowledge the first 9
        for id in &ids[..9] {
            mark_acknowledged(&conn, id).unwrap();
        }

        let rate = effectiveness_rate(&conn, "post-edit", "pattern", None)
            .unwrap()
            .expect("should have rate with 12 samples");
        assert!((rate - 0.75).abs() < 1e-9, "expected 0.75, got {}", rate);
    }

    #[test]
    fn test_effectiveness_rate_insufficient() {
        let conn = setup();

        // Insert only 5 injections — below the 10-sample threshold
        for i in 0..5 {
            record_injection(
                &conn,
                &format!("sess-{}", i),
                "session-start",
                "memory",
                &format!("memory chunk {}", i),
            )
            .unwrap();
        }

        let rate = effectiveness_rate(&conn, "session-start", "memory", None).unwrap();
        assert!(rate.is_none(), "expected None with only 5 samples");
    }
}
