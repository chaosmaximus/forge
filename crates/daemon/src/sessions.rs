use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub agent: String,
    pub project: Option<String>,
    pub cwd: Option<String>,
    pub started_at: String,
    pub ended_at: Option<String>,
    pub status: String,
}

/// Register a new agent session. Uses INSERT OR REPLACE so re-registering
/// the same ID updates the existing record.
pub fn register_session(
    conn: &Connection,
    id: &str,
    agent: &str,
    project: Option<&str>,
    cwd: Option<&str>,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT OR REPLACE INTO session (id, agent, project, cwd, started_at, status)
         VALUES (?1, ?2, ?3, ?4, datetime('now'), 'active')",
        params![id, agent, project, cwd],
    )?;
    Ok(())
}

/// Mark a session as ended. Returns true if the session existed.
pub fn end_session(conn: &Connection, id: &str) -> rusqlite::Result<bool> {
    let updated = conn.execute(
        "UPDATE session SET status = 'ended', ended_at = datetime('now') WHERE id = ?1 AND status = 'active'",
        params![id],
    )?;
    Ok(updated > 0)
}

/// List sessions. If active_only is true, only return active sessions.
pub fn list_sessions(conn: &Connection, active_only: bool) -> rusqlite::Result<Vec<Session>> {
    let sql = if active_only {
        "SELECT id, agent, project, cwd, started_at, ended_at, status FROM session WHERE status = 'active' ORDER BY started_at DESC"
    } else {
        "SELECT id, agent, project, cwd, started_at, ended_at, status FROM session ORDER BY started_at DESC"
    };
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt.query_map([], |row| {
        Ok(Session {
            id: row.get(0)?,
            agent: row.get(1)?,
            project: row.get(2)?,
            cwd: row.get(3)?,
            started_at: row.get(4)?,
            ended_at: row.get(5)?,
            status: row.get(6)?,
        })
    })?;
    rows.collect()
}

/// Get a single session by ID.
pub fn get_session(conn: &Connection, id: &str) -> rusqlite::Result<Option<Session>> {
    let mut stmt = conn.prepare(
        "SELECT id, agent, project, cwd, started_at, ended_at, status FROM session WHERE id = ?1",
    )?;
    let mut rows = stmt.query(params![id])?;
    if let Some(row) = rows.next()? {
        Ok(Some(Session {
            id: row.get(0)?,
            agent: row.get(1)?,
            project: row.get(2)?,
            cwd: row.get(3)?,
            started_at: row.get(4)?,
            ended_at: row.get(5)?,
            status: row.get(6)?,
        }))
    } else {
        Ok(None)
    }
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
    fn test_register_and_list() {
        let conn = setup();
        register_session(&conn, "s1", "claude-code", Some("forge"), Some("/project")).unwrap();
        register_session(&conn, "s2", "cline", None, None).unwrap();

        let active = list_sessions(&conn, true).unwrap();
        assert_eq!(active.len(), 2);
        let agents: Vec<&str> = active.iter().map(|s| s.agent.as_str()).collect();
        assert!(agents.contains(&"claude-code"));
        assert!(agents.contains(&"cline"));
    }

    #[test]
    fn test_end_session() {
        let conn = setup();
        register_session(&conn, "s1", "claude-code", None, None).unwrap();

        assert!(end_session(&conn, "s1").unwrap());
        assert!(!end_session(&conn, "s1").unwrap()); // already ended

        let active = list_sessions(&conn, true).unwrap();
        assert!(active.is_empty());

        let all = list_sessions(&conn, false).unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].status, "ended");
        assert!(all[0].ended_at.is_some());
    }

    #[test]
    fn test_register_duplicate_updates() {
        let conn = setup();
        register_session(&conn, "s1", "claude-code", Some("proj1"), None).unwrap();
        register_session(&conn, "s1", "claude-code", Some("proj2"), None).unwrap();

        let all = list_sessions(&conn, false).unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].project.as_deref(), Some("proj2"));
    }

    #[test]
    fn test_get_session() {
        let conn = setup();
        register_session(&conn, "s1", "claude-code", Some("forge"), Some("/cwd")).unwrap();

        let s = get_session(&conn, "s1").unwrap().unwrap();
        assert_eq!(s.agent, "claude-code");
        assert_eq!(s.project.as_deref(), Some("forge"));
        assert_eq!(s.cwd.as_deref(), Some("/cwd"));

        assert!(get_session(&conn, "nonexistent").unwrap().is_none());
    }
}
