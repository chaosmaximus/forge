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
    let id = ulid::Ulid::new().to_string();
    conn.execute(
        "INSERT INTO context_effectiveness (id, session_id, hook_event, context_type, content_summary)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![id, session_id, hook_event, context_type, content_summary],
    )?;
    Ok(id)
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
