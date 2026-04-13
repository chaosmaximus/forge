use rusqlite::Connection;

/// A notification record from the notification table.
pub struct Notification {
    pub id: String,
    pub category: String,
    pub priority: String,
    pub title: String,
    pub content: String,
    pub source: String,
    pub source_id: Option<String>,
    pub target_type: String,
    pub target_id: Option<String>,
    pub status: String,
    pub action_type: Option<String>,
    pub action_payload: Option<String>,
    pub action_result: Option<String>,
    pub topic: Option<String>,
    pub created_at: String,
    pub metadata: String,
}

/// Builder for creating notifications without too many function args.
pub struct NotificationBuilder {
    pub category: String,
    pub priority: String,
    pub title: String,
    pub content: String,
    pub source: String,
    pub source_id: Option<String>,
    pub target_type: String,
    pub target_id: Option<String>,
    pub topic: Option<String>,
    pub action_type: Option<String>,
    pub action_payload: Option<String>,
    pub organization_id: String,
    pub reality_id: Option<String>,
    pub expires_at: Option<String>,
}

impl NotificationBuilder {
    pub fn new(category: &str, priority: &str, title: &str, content: &str, source: &str) -> Self {
        Self {
            category: category.to_string(),
            priority: priority.to_string(),
            title: title.to_string(),
            content: content.to_string(),
            source: source.to_string(),
            source_id: None,
            target_type: "broadcast".to_string(),
            target_id: None,
            topic: None,
            action_type: None,
            action_payload: None,
            organization_id: "default".to_string(),
            reality_id: None,
            expires_at: None,
        }
    }

    pub fn source_id(mut self, id: &str) -> Self {
        self.source_id = Some(id.to_string());
        self
    }

    pub fn target_session(mut self, id: &str) -> Self {
        self.target_type = "session".to_string();
        self.target_id = Some(id.to_string());
        self
    }

    pub fn target_team(mut self, id: &str) -> Self {
        self.target_type = "team".to_string();
        self.target_id = Some(id.to_string());
        self
    }

    pub fn topic(mut self, t: &str) -> Self {
        self.topic = Some(t.to_string());
        self
    }

    pub fn action(mut self, action_type: &str, payload: &str) -> Self {
        self.action_type = Some(action_type.to_string());
        self.action_payload = Some(payload.to_string());
        self
    }

    pub fn expires_at(mut self, ts: &str) -> Self {
        self.expires_at = Some(ts.to_string());
        self
    }

    /// Insert the notification into the database and return its ID.
    pub fn build(self, conn: &Connection) -> rusqlite::Result<String> {
        create_notification(conn, self)
    }
}

/// Insert a notification into the database and return its ID.
fn create_notification(conn: &Connection, nb: NotificationBuilder) -> rusqlite::Result<String> {
    let id = ulid::Ulid::new().to_string();
    let now = forge_core::time::now_iso();
    conn.execute(
        "INSERT INTO notification (id, category, priority, title, content, source, source_id,
         target_type, target_id, status, action_type, action_payload, created_at, expires_at,
         organization_id, reality_id, topic, metadata)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 'pending', ?10, ?11, ?12, ?13, ?14, ?15, ?16, '{}')",
        rusqlite::params![
            id, nb.category, nb.priority, nb.title, nb.content, nb.source, nb.source_id,
            nb.target_type, nb.target_id, nb.action_type, nb.action_payload, now, nb.expires_at,
            nb.organization_id, nb.reality_id, nb.topic,
        ],
    )?;
    Ok(id)
}

/// List notifications with optional filters.
pub fn list_notifications(
    conn: &Connection,
    status: Option<&str>,
    category: Option<&str>,
    _priority_min: Option<&str>,
    target_id: Option<&str>,
    limit: usize,
) -> rusqlite::Result<Vec<Notification>> {
    let mut sql = String::from(
        "SELECT id, category, priority, title, content, source, source_id,
         target_type, target_id, status, action_type, action_payload, action_result,
         topic, created_at, metadata
         FROM notification WHERE 1=1",
    );
    let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

    if let Some(s) = status {
        params.push(Box::new(s.to_string()));
        sql.push_str(&format!(" AND status = ?{}", params.len()));
    }
    if let Some(c) = category {
        params.push(Box::new(c.to_string()));
        sql.push_str(&format!(" AND category = ?{}", params.len()));
    }
    if let Some(t) = target_id {
        params.push(Box::new(t.to_string()));
        sql.push_str(&format!(" AND target_id = ?{}", params.len()));
    }

    params.push(Box::new(limit as i64));
    sql.push_str(&format!(
        " ORDER BY created_at DESC LIMIT ?{}",
        params.len()
    ));

    let param_refs: Vec<&dyn rusqlite::types::ToSql> = params.iter().map(|p| p.as_ref()).collect();
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(param_refs.as_slice(), |row| {
        Ok(Notification {
            id: row.get(0)?,
            category: row.get(1)?,
            priority: row.get(2)?,
            title: row.get(3)?,
            content: row.get(4)?,
            source: row.get(5)?,
            source_id: row.get(6)?,
            target_type: row.get(7)?,
            target_id: row.get(8)?,
            status: row.get(9)?,
            action_type: row.get(10)?,
            action_payload: row.get(11)?,
            action_result: row.get(12)?,
            topic: row.get(13)?,
            created_at: row.get(14)?,
            metadata: row.get(15)?,
        })
    })?;

    let mut results = Vec::new();
    for row in rows {
        results.push(row?);
    }
    Ok(results)
}

/// Acknowledge a notification (set status='acknowledged', acknowledged_at=now).
/// Returns true if the notification was found and updated.
pub fn ack_notification(conn: &Connection, id: &str) -> rusqlite::Result<bool> {
    let now = forge_core::time::now_iso();
    let updated = conn.execute(
        "UPDATE notification SET status = 'acknowledged', acknowledged_at = ?1 WHERE id = ?2 AND status = 'pending'",
        rusqlite::params![now, id],
    )?;
    Ok(updated > 0)
}

/// Dismiss a notification (set status='dismissed') and increment dismiss_count in notification_tuning.
/// Returns true if the notification was found and updated.
pub fn dismiss_notification(conn: &Connection, id: &str) -> rusqlite::Result<bool> {
    // Look up the notification topic first
    let topic: Option<String> = conn
        .query_row(
            "SELECT topic FROM notification WHERE id = ?1",
            rusqlite::params![id],
            |row| row.get(0),
        )
        .ok();

    let updated = conn.execute(
        "UPDATE notification SET status = 'dismissed' WHERE id = ?1 AND status = 'pending'",
        rusqlite::params![id],
    )?;

    if updated > 0 {
        if let Some(Some(topic_val)) = Some(topic) {
            let now = forge_core::time::now_iso();
            // Upsert into notification_tuning
            conn.execute(
                "INSERT INTO notification_tuning (topic, user_id, dismiss_count, ack_count, last_adjusted_at)
                 VALUES (?1, 'local', 1, 0, ?2)
                 ON CONFLICT(topic, user_id) DO UPDATE SET
                     dismiss_count = dismiss_count + 1,
                     last_adjusted_at = ?2",
                rusqlite::params![topic_val, now],
            )?;

            // Adaptive: if dismiss_count >= 3, set priority_override to 'low'
            conn.execute(
                "UPDATE notification_tuning SET priority_override = 'low'
                 WHERE topic = ?1 AND user_id = 'local' AND dismiss_count >= 3",
                rusqlite::params![topic_val],
            )?;
        }
    }

    Ok(updated > 0)
}

/// Act on a confirmation notification. If approved, store the action_result.
/// Returns the action_result if approved, None if rejected.
pub fn act_on_notification(
    conn: &Connection,
    id: &str,
    approved: bool,
) -> rusqlite::Result<Option<String>> {
    let result_str = if approved { "approved" } else { "rejected" };
    let status_str = if approved { "acted" } else { "dismissed" };

    let updated = conn.execute(
        "UPDATE notification SET status = ?1, action_result = ?2
         WHERE id = ?3 AND status = 'pending'",
        rusqlite::params![status_str, result_str, id],
    )?;

    if updated == 0 {
        // Either not found or already acted upon
        return Ok(None);
    }

    if approved {
        Ok(Some(result_str.to_string()))
    } else {
        Ok(None)
    }
}

/// Count pending notifications, optionally for a specific target.
pub fn count_pending(conn: &Connection, target_id: Option<&str>) -> rusqlite::Result<usize> {
    let count: i64 = match target_id {
        Some(tid) => conn.query_row(
            "SELECT COUNT(*) FROM notification WHERE status = 'pending' AND target_id = ?1",
            rusqlite::params![tid],
            |row| row.get(0),
        )?,
        None => conn.query_row(
            "SELECT COUNT(*) FROM notification WHERE status = 'pending'",
            [],
            |row| row.get(0),
        )?,
    };
    Ok(count as usize)
}

/// Expire notifications whose expires_at is in the past.
/// Returns the number of notifications expired.
pub fn expire_old(conn: &Connection) -> rusqlite::Result<usize> {
    let now = forge_core::time::now_iso();
    let expired = conn.execute(
        "UPDATE notification SET status = 'expired' WHERE status = 'pending' AND expires_at IS NOT NULL AND expires_at < ?1",
        rusqlite::params![now],
    )?;
    Ok(expired)
}

/// Check if a notification with the given topic was created within cooldown_secs.
/// Returns true if throttled (should NOT send a new notification).
pub fn check_throttle(
    conn: &Connection,
    topic: &str,
    _user_id: &str,
    cooldown_secs: i64,
) -> rusqlite::Result<bool> {
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM notification
         WHERE topic = ?1 AND created_at > datetime('now', '-' || ?2 || ' seconds')",
        rusqlite::params![topic, cooldown_secs],
        |row| row.get(0),
    )?;
    Ok(count > 0)
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
    fn test_create_notification() {
        let conn = setup();
        let id =
            NotificationBuilder::new("alert", "high", "Build failed", "CI pipeline failed", "ci")
                .source_id("run-123")
                .topic("build_failure")
                .build(&conn)
                .unwrap();

        assert!(!id.is_empty());

        // Verify it's in the DB
        let title: String = conn
            .query_row("SELECT title FROM notification WHERE id = ?1", [&id], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(title, "Build failed");

        let status: String = conn
            .query_row(
                "SELECT status FROM notification WHERE id = ?1",
                [&id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(status, "pending");
    }

    #[test]
    fn test_list_pending() {
        let conn = setup();
        for i in 0..3 {
            NotificationBuilder::new("alert", "high", &format!("Alert {i}"), "body", "system")
                .build(&conn)
                .unwrap();
        }

        let results = list_notifications(&conn, Some("pending"), None, None, None, 10).unwrap();
        assert_eq!(results.len(), 3);
    }

    #[test]
    fn test_list_by_category() {
        let conn = setup();
        NotificationBuilder::new("alert", "high", "Alert 1", "body", "system")
            .build(&conn)
            .unwrap();
        NotificationBuilder::new("insight", "low", "Insight 1", "body", "system")
            .build(&conn)
            .unwrap();

        let alerts = list_notifications(&conn, None, Some("alert"), None, None, 10).unwrap();
        assert_eq!(alerts.len(), 1);
        assert_eq!(alerts[0].category, "alert");

        let insights = list_notifications(&conn, None, Some("insight"), None, None, 10).unwrap();
        assert_eq!(insights.len(), 1);
        assert_eq!(insights[0].category, "insight");
    }

    #[test]
    fn test_ack_notification() {
        let conn = setup();
        let id = NotificationBuilder::new("alert", "high", "Test", "body", "system")
            .build(&conn)
            .unwrap();

        let acked = ack_notification(&conn, &id).unwrap();
        assert!(acked);

        let status: String = conn
            .query_row(
                "SELECT status FROM notification WHERE id = ?1",
                [&id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(status, "acknowledged");

        let ack_at: Option<String> = conn
            .query_row(
                "SELECT acknowledged_at FROM notification WHERE id = ?1",
                [&id],
                |r| r.get(0),
            )
            .unwrap();
        assert!(ack_at.is_some());
    }

    #[test]
    fn test_dismiss_notification() {
        let conn = setup();
        let id = NotificationBuilder::new("alert", "high", "Test", "body", "system")
            .topic("test_topic")
            .build(&conn)
            .unwrap();

        let dismissed = dismiss_notification(&conn, &id).unwrap();
        assert!(dismissed);

        let status: String = conn
            .query_row(
                "SELECT status FROM notification WHERE id = ?1",
                [&id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(status, "dismissed");

        // Check tuning table was updated
        let dismiss_count: i64 = conn
            .query_row(
                "SELECT dismiss_count FROM notification_tuning WHERE topic = 'test_topic'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(dismiss_count, 1);
    }

    #[test]
    fn test_dismiss_adaptive_throttle() {
        let conn = setup();
        // Dismiss 3 notifications with the same topic
        for _ in 0..3 {
            let id = NotificationBuilder::new("alert", "high", "Noisy", "body", "system")
                .topic("noisy_topic")
                .build(&conn)
                .unwrap();
            dismiss_notification(&conn, &id).unwrap();
        }

        // After 3 dismissals, priority_override should be 'low'
        let override_val: Option<String> = conn
            .query_row(
                "SELECT priority_override FROM notification_tuning WHERE topic = 'noisy_topic'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(override_val, Some("low".to_string()));
    }

    #[test]
    fn test_act_confirmation_approve() {
        let conn = setup();
        let id = NotificationBuilder::new(
            "confirmation",
            "high",
            "Deploy?",
            "Approve deploy",
            "deploy",
        )
        .action("deploy", r#"{"target":"production"}"#)
        .build(&conn)
        .unwrap();

        let result = act_on_notification(&conn, &id, true).unwrap();
        assert_eq!(result, Some("approved".to_string()));

        let status: String = conn
            .query_row(
                "SELECT status FROM notification WHERE id = ?1",
                [&id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(status, "acted");

        let action_result: Option<String> = conn
            .query_row(
                "SELECT action_result FROM notification WHERE id = ?1",
                [&id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(action_result, Some("approved".to_string()));
    }

    #[test]
    fn test_act_confirmation_reject() {
        let conn = setup();
        let id = NotificationBuilder::new(
            "confirmation",
            "high",
            "Deploy?",
            "Approve deploy",
            "deploy",
        )
        .action("deploy", r#"{"target":"production"}"#)
        .build(&conn)
        .unwrap();

        let result = act_on_notification(&conn, &id, false).unwrap();
        assert!(result.is_none());

        let status: String = conn
            .query_row(
                "SELECT status FROM notification WHERE id = ?1",
                [&id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(status, "dismissed");
    }

    #[test]
    fn test_throttle_check() {
        let conn = setup();
        NotificationBuilder::new("alert", "high", "Test", "body", "system")
            .topic("throttle_topic")
            .build(&conn)
            .unwrap();

        // Should be throttled (created within 60 seconds)
        let throttled = check_throttle(&conn, "throttle_topic", "local", 60).unwrap();
        assert!(throttled);
    }

    #[test]
    fn test_throttle_expired() {
        let conn = setup();
        // Insert a notification with an old created_at (2 hours ago)
        let id = ulid::Ulid::new().to_string();
        conn.execute(
            "INSERT INTO notification (id, category, priority, title, content, source,
             target_type, status, topic, created_at, metadata)
             VALUES (?1, 'alert', 'high', 'Old', 'body', 'system', 'broadcast', 'pending',
             'old_topic', datetime('now', '-7200 seconds'), '{}')",
            rusqlite::params![id],
        )
        .unwrap();

        // Should NOT be throttled (older than 60 seconds)
        let throttled = check_throttle(&conn, "old_topic", "local", 60).unwrap();
        assert!(!throttled);
    }

    #[test]
    fn test_expire_old() {
        let conn = setup();
        // Insert a notification with expires_at in the past
        let id = ulid::Ulid::new().to_string();
        conn.execute(
            "INSERT INTO notification (id, category, priority, title, content, source,
             target_type, status, expires_at, created_at, metadata)
             VALUES (?1, 'alert', 'low', 'Expired', 'body', 'system', 'broadcast', 'pending',
             datetime('now', '-3600 seconds'), datetime('now', '-7200 seconds'), '{}')",
            rusqlite::params![id],
        )
        .unwrap();

        let expired = expire_old(&conn).unwrap();
        assert_eq!(expired, 1);

        let status: String = conn
            .query_row(
                "SELECT status FROM notification WHERE id = ?1",
                [&id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(status, "expired");
    }

    #[test]
    fn test_count_pending() {
        let conn = setup();
        // Create 3 pending
        for i in 0..3 {
            NotificationBuilder::new("alert", "high", &format!("N{i}"), "body", "system")
                .build(&conn)
                .unwrap();
        }
        // Ack one
        let all = list_notifications(&conn, Some("pending"), None, None, None, 10).unwrap();
        ack_notification(&conn, &all[0].id).unwrap();

        let count = count_pending(&conn, None).unwrap();
        assert_eq!(count, 2);
    }

    #[test]
    fn test_schema_idempotent() {
        let conn = setup();
        // Calling create_schema again should not error
        create_schema(&conn).unwrap();

        // We can still create notifications
        let id = NotificationBuilder::new("alert", "high", "Test", "body", "system")
            .build(&conn)
            .unwrap();
        assert!(!id.is_empty());
    }

    #[test]
    fn test_builder_with_action() {
        let conn = setup();
        let id = NotificationBuilder::new(
            "confirmation",
            "medium",
            "Approve protocol?",
            "New pattern detected",
            "consolidator",
        )
        .action("approve_protocol", r#"{"memory_id":"mem-123"}"#)
        .topic("protocol_suggestion")
        .build(&conn)
        .unwrap();

        let (action_type, action_payload): (Option<String>, Option<String>) = conn
            .query_row(
                "SELECT action_type, action_payload FROM notification WHERE id = ?1",
                [&id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();

        assert_eq!(action_type, Some("approve_protocol".to_string()));
        assert_eq!(
            action_payload,
            Some(r#"{"memory_id":"mem-123"}"#.to_string())
        );

        let category: String = conn
            .query_row(
                "SELECT category FROM notification WHERE id = ?1",
                [&id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(category, "confirmation");
    }

    #[test]
    fn test_builder_with_target() {
        let conn = setup();
        let id =
            NotificationBuilder::new("alert", "high", "Session alert", "Your build failed", "ci")
                .target_session("session-abc")
                .build(&conn)
                .unwrap();

        let (target_type, target_id): (String, Option<String>) = conn
            .query_row(
                "SELECT target_type, target_id FROM notification WHERE id = ?1",
                [&id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();

        assert_eq!(target_type, "session");
        assert_eq!(target_id, Some("session-abc".to_string()));

        // Also test team targeting
        let id2 = NotificationBuilder::new("alert", "high", "Team alert", "Deploy ready", "deploy")
            .target_team("team-xyz")
            .build(&conn)
            .unwrap();

        let (target_type2, target_id2): (String, Option<String>) = conn
            .query_row(
                "SELECT target_type, target_id FROM notification WHERE id = ?1",
                [&id2],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();

        assert_eq!(target_type2, "team");
        assert_eq!(target_id2, Some("team-xyz".to_string()));
    }
}
