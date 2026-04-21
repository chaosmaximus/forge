//! Metrics table helpers. Owned by this module; do not modify ops.rs for SP1.
//!
//! The `metrics` table is the canonical store for counters read by
//! `forge-next stats`. See `ops::query_stats` for the read query.
//!
//! ## Schema adaptation
//!
//! The plan's draft INSERT used `(session_id, value, meta, timestamp)` columns
//! that do not exist on the real schema. The real `metrics` table has columns:
//! `id, metric_type, timestamp, model, tokens_in, tokens_out, latency_ms, cost_usd, status, details`.
//! `ops::query_stats` reads:
//!   - `SUM(tokens_in) / SUM(tokens_out) / SUM(cost_usd) / AVG(latency_ms)` for totals, and
//!   - `status != 'ok'` to count errors.
//!
//! So we translate the helper's `cost_cents` -> `cost_usd`, set `status`
//! to `"ok"` or `"error"` based on whether `error` is present, and pack
//! the session ID / per-row memories-created / stringified error into the
//! `details` JSON column for future debugging.

use rusqlite::{params, Connection};

/// Record one extraction event. Called from the writer actor in response
/// to `WriteCommand::RecordExtraction`. On success, `error` is None and the
/// row lands with `status = 'ok'`. On error, `error` carries the stringified
/// failure and the row lands with `status = 'error'` so that
/// `ops::query_stats` counts it toward the errors total.
///
/// Token counts and memory counts are stored on the dedicated columns where
/// the query expects them. Session ID / memories_created / error are packed
/// as JSON into the `details` column for offline debugging.
pub fn record_extraction(
    conn: &Connection,
    session_id: &str,
    memories_created: usize,
    tokens_in: u64,
    tokens_out: u64,
    cost_cents: u64,
    error: Option<&str>,
) -> rusqlite::Result<()> {
    // Sanitize the error string: cap length and strip control chars so
    // user-supplied input in extraction errors cannot poison the details
    // column with arbitrary bytes.
    let sanitized_error = error.map(sanitize_error);

    let details = serde_json::json!({
        "session_id":       session_id,
        "memories_created": memories_created,
        "error":            sanitized_error,
    })
    .to_string();

    let status = if error.is_some() { "error" } else { "ok" };
    // cost_cents is integer cents; query_stats sums `cost_usd` (REAL) in USD.
    let cost_usd = cost_cents as f64 / 100.0;

    conn.execute(
        "INSERT INTO metrics (id, metric_type, timestamp, model, tokens_in, tokens_out, latency_ms, cost_usd, status, details)
         VALUES (?1, 'extraction', datetime('now'), ?2, ?3, ?4, 0, ?5, ?6, ?7)",
        params![
            format!("metric-{}", ulid::Ulid::new()),
            "extractor",
            tokens_in as i64,
            tokens_out as i64,
            cost_usd,
            status,
            details,
        ],
    )?;
    Ok(())
}

/// Cap error strings at 2 KiB and strip ASCII control characters (except
/// newline/tab) so rogue bytes in an extraction error can't break JSON
/// parsing downstream or poison log output.
fn sanitize_error(e: &str) -> String {
    const MAX: usize = 2048;
    let mut out: String = e
        .chars()
        .filter(|c| !c.is_control() || *c == '\n' || *c == '\t')
        .collect();
    if out.len() > MAX {
        out.truncate(MAX);
        out.push_str("...[truncated]");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> Connection {
        crate::db::vec::init_sqlite_vec();
        let conn = Connection::open_in_memory().unwrap();
        crate::db::schema::create_schema(&conn).unwrap();
        conn
    }

    #[test]
    fn test_record_extraction_success_writes_row() {
        let conn = setup();
        record_extraction(&conn, "sess-1", 5, 1000, 500, 12, None).unwrap();

        let (count, status, tokens_in, tokens_out, cost_usd, details): (
            i64,
            String,
            i64,
            i64,
            f64,
            String,
        ) = conn
            .query_row(
                "SELECT COUNT(*), MAX(status), MAX(tokens_in), MAX(tokens_out), MAX(cost_usd), MAX(details)
                 FROM metrics WHERE metric_type = 'extraction'",
                [],
                |r| {
                    Ok((
                        r.get(0)?,
                        r.get(1)?,
                        r.get(2)?,
                        r.get(3)?,
                        r.get(4)?,
                        r.get(5)?,
                    ))
                },
            )
            .unwrap();

        assert_eq!(count, 1);
        assert_eq!(status, "ok");
        assert_eq!(tokens_in, 1000);
        assert_eq!(tokens_out, 500);
        assert!((cost_usd - 0.12).abs() < 1e-9, "cost_usd={cost_usd}");
        assert!(
            details.contains("sess-1"),
            "details should carry session_id"
        );
        assert!(
            details.contains("\"memories_created\":5"),
            "details should carry memories_created"
        );
    }

    #[test]
    fn test_record_extraction_error_writes_row_with_error_in_details() {
        let conn = setup();
        record_extraction(&conn, "sess-2", 0, 0, 0, 0, Some("connection refused")).unwrap();

        let (status, details): (String, String) = conn
            .query_row(
                "SELECT status, details FROM metrics WHERE metric_type = 'extraction'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();

        assert_eq!(status, "error", "error path should flip status to 'error'");
        assert!(
            details.contains("connection refused"),
            "details should carry error string; got: {details}"
        );

        // Verify this row is counted as an error by the query_stats shape.
        let errors: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM metrics WHERE metric_type = 'extraction' AND status != 'ok'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(errors, 1, "query_stats error count should see this row");
    }

    #[test]
    fn test_record_extraction_details_valid_json() {
        // Adversarial self-review: ensure `details` is valid JSON even when
        // the error string contains JSON-hostile characters (quotes, backslash).
        let conn = setup();
        record_extraction(
            &conn,
            "sess-3",
            0,
            0,
            0,
            0,
            Some("failed to parse \"foo\" at offset \\n"),
        )
        .unwrap();
        let details: String = conn
            .query_row(
                "SELECT details FROM metrics WHERE metric_type = 'extraction'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        // Must parse back — serde_json::json!() handles escaping.
        let parsed: serde_json::Value = serde_json::from_str(&details).unwrap();
        assert_eq!(parsed["session_id"], "sess-3");
        assert!(parsed["error"]
            .as_str()
            .unwrap()
            .contains("failed to parse"));
    }

    #[test]
    fn test_record_extraction_feeds_query_stats() {
        // Adversarial self-review: does `forge-next stats` light up?
        // Drive the real read path via `ops::query_stats` and confirm
        // extractions > 0 + errors counted correctly.
        let conn = setup();
        record_extraction(&conn, "s-a", 3, 1_000, 200, 5, None).unwrap();
        record_extraction(&conn, "s-b", 2, 500, 100, 2, None).unwrap();
        record_extraction(&conn, "s-c", 0, 0, 0, 0, Some("http 500")).unwrap();

        let stats = crate::db::ops::query_stats(&conn, 24).unwrap();
        assert_eq!(stats.extractions, 3, "counter must advance");
        assert_eq!(stats.extraction_errors, 1, "error row must count");
        assert_eq!(stats.tokens_in, 1_500);
        assert_eq!(stats.tokens_out, 300);
        // 5 cents + 2 cents = 0.07 USD
        assert!(
            (stats.total_cost_usd - 0.07).abs() < 1e-9,
            "cost_usd = {}",
            stats.total_cost_usd
        );
    }

    #[test]
    fn test_record_extraction_sanitizes_control_chars() {
        let conn = setup();
        // NUL and bell should be stripped; newline allowed.
        record_extraction(&conn, "sess-4", 0, 0, 0, 0, Some("bad\x00err\x07\nmore")).unwrap();
        let details: String = conn
            .query_row(
                "SELECT details FROM metrics WHERE metric_type = 'extraction'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&details).unwrap();
        let err_str = parsed["error"].as_str().unwrap();
        assert!(!err_str.contains('\x00'));
        assert!(!err_str.contains('\x07'));
        assert!(err_str.contains('\n'));
        assert!(err_str.contains("baderr"));
        assert!(err_str.contains("more"));
    }
}
