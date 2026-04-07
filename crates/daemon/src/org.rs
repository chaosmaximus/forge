use rusqlite::{params, Connection};
use serde_json::{json, Value};

/// Max lengths for input validation (F-006)
const MAX_NAME_LEN: usize = 256;
const MAX_DESCRIPTION_LEN: usize = 4096;
/// Max recursion depth for team tree CTE (F-004)
const MAX_TREE_DEPTH: usize = 20;

fn validate_name(name: &str) -> rusqlite::Result<()> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(rusqlite::Error::InvalidParameterName("name cannot be empty".into()));
    }
    if trimmed.len() > MAX_NAME_LEN {
        return Err(rusqlite::Error::InvalidParameterName(
            format!("name exceeds {} chars", MAX_NAME_LEN),
        ));
    }
    Ok(())
}

fn validate_description(desc: Option<&str>) -> rusqlite::Result<()> {
    if let Some(d) = desc {
        if d.len() > MAX_DESCRIPTION_LEN {
            return Err(rusqlite::Error::InvalidParameterName(
                format!("description exceeds {} chars", MAX_DESCRIPTION_LEN),
            ));
        }
    }
    Ok(())
}

/// Create a new organization. Returns the ULID.
pub fn create_organization(
    conn: &Connection,
    name: &str,
    description: Option<&str>,
) -> rusqlite::Result<String> {
    validate_name(name)?;
    validate_description(description)?;
    let id = ulid::Ulid::new().to_string();
    let now = forge_core::time::now_iso();
    conn.execute(
        "INSERT INTO organization (id, name, created_at, updated_at, description)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![id, name, now, now, description],
    )?;
    Ok(id)
}

/// List all organizations as JSON values.
pub fn list_organizations(conn: &Connection) -> rusqlite::Result<Vec<Value>> {
    let mut stmt = conn.prepare(
        "SELECT id, name, description, created_at, updated_at FROM organization ORDER BY name",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok(json!({
            "id": row.get::<_, String>(0)?,
            "name": row.get::<_, String>(1)?,
            "description": row.get::<_, Option<String>>(2)?,
            "created_at": row.get::<_, String>(3)?,
            "updated_at": row.get::<_, String>(4)?,
        }))
    })?;
    rows.collect()
}

/// Resolve an org identifier (name or ID) to the actual org ID.
pub fn resolve_org_id(conn: &Connection, org_ref: &str) -> rusqlite::Result<String> {
    // Try as ID first
    let exists: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM organization WHERE id = ?1)",
        params![org_ref],
        |row| row.get(0),
    )?;
    if exists { return Ok(org_ref.to_string()); }
    // Try as name
    conn.query_row(
        "SELECT id FROM organization WHERE name = ?1",
        params![org_ref],
        |row| row.get(0),
    )
}

/// Build a nested team tree for an organization.
/// Root nodes have parent_team_id IS NULL. Children are nested recursively.
/// `org_ref` can be an org ID or org name — resolved automatically.
pub fn team_tree(conn: &Connection, org_ref: &str) -> rusqlite::Result<Vec<Value>> {
    let org_id = resolve_org_id(conn, org_ref)?;
    let mut stmt = conn.prepare(
        "SELECT id, name, parent_team_id, description, team_type, purpose, status, created_at
         FROM team WHERE organization_id = ?1 ORDER BY name",
    )?;

    struct FlatTeam {
        id: String,
        name: String,
        parent_team_id: Option<String>,
        description: Option<String>,
        team_type: Option<String>,
        purpose: Option<String>,
        status: String,
        created_at: String,
    }

    let teams: Vec<FlatTeam> = stmt
        .query_map(params![org_id], |row| {
            Ok(FlatTeam {
                id: row.get(0)?,
                name: row.get(1)?,
                parent_team_id: row.get(2)?,
                description: row.get(3)?,
                team_type: row.get(4)?,
                purpose: row.get(5)?,
                status: row.get(6)?,
                created_at: row.get(7)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    fn build_children(parent_id: Option<&str>, teams: &[FlatTeam]) -> Vec<Value> {
        teams
            .iter()
            .filter(|t| t.parent_team_id.as_deref() == parent_id)
            .map(|t| {
                let children = build_children(Some(&t.id), teams);
                json!({
                    "id": t.id,
                    "name": t.name,
                    "description": t.description,
                    "team_type": t.team_type,
                    "purpose": t.purpose,
                    "status": t.status,
                    "created_at": t.created_at,
                    "children": children,
                })
            })
            .collect()
    }

    Ok(build_children(None, &teams))
}

/// Create an organization from a named template.
/// Idempotent: if the org already exists, reuses it and skips duplicate teams.
/// Returns (org_id, team_count).
pub fn create_org_from_template(
    conn: &Connection,
    template_name: &str,
    org_name: &str,
) -> rusqlite::Result<(String, usize)> {
    // Define template structures: (name, children)
    struct TeamDef {
        name: &'static str,
        children: &'static [&'static str],
    }

    let templates: &[TeamDef] = match template_name {
        "startup" => &[
            TeamDef { name: "engineering", children: &["backend", "frontend", "qa"] },
            TeamDef { name: "business", children: &["c-suite", "finance", "operations"] },
            TeamDef { name: "marketing", children: &["content", "community", "growth"] },
        ],
        "devteam" => &[
            TeamDef { name: "engineering", children: &["backend", "frontend", "devops", "qa"] },
        ],
        "agency" => &[
            TeamDef { name: "creative", children: &["design", "copywriting"] },
            TeamDef { name: "production", children: &["video"] },
            TeamDef { name: "accounts", children: &[] },
        ],
        _ => {
            return Err(rusqlite::Error::InvalidParameterName(format!(
                "unknown template: {template_name}"
            )));
        }
    };

    // Idempotent: reuse existing org or create a new one
    let org_id: String = match conn.query_row(
        "SELECT id FROM organization WHERE name = ?1",
        params![org_name],
        |row| row.get(0),
    ) {
        Ok(existing_id) => existing_id,
        Err(rusqlite::Error::QueryReturnedNoRows) => create_organization(conn, org_name, None)?,
        Err(e) => return Err(e),
    };

    let now = forge_core::time::now_iso();
    let mut count = 0usize;

    for def in templates {
        // Check if this parent team already exists for this org
        let existing_parent: Option<String> = conn.query_row(
            "SELECT id FROM team WHERE name = ?1 AND organization_id = ?2",
            params![def.name, org_id],
            |row| row.get(0),
        ).ok();

        let parent_id = match existing_parent {
            Some(id) => id, // reuse existing team
            None => {
                let pid = ulid::Ulid::new().to_string();
                conn.execute(
                    "INSERT INTO team (id, name, organization_id, created_by, status, created_at)
                     VALUES (?1, ?2, ?3, 'system', 'active', ?4)",
                    params![pid, def.name, org_id, now],
                )?;
                count += 1;
                pid
            }
        };

        for child_name in def.children {
            // Check if this child team already exists for this org
            let exists: bool = conn.query_row(
                "SELECT COUNT(*) > 0 FROM team WHERE name = ?1 AND organization_id = ?2 AND parent_team_id = ?3",
                params![*child_name, org_id, parent_id],
                |row| row.get(0),
            ).unwrap_or(false);

            if !exists {
                let child_id = ulid::Ulid::new().to_string();
                conn.execute(
                    "INSERT INTO team (id, name, organization_id, parent_team_id, created_by, status, created_at)
                     VALUES (?1, ?2, ?3, ?4, 'system', 'active', ?5)",
                    params![child_id, *child_name, org_id, parent_id, now],
                )?;
                count += 1;
            }
        }
    }

    Ok((org_id, count))
}

/// Get active session IDs for a team.
/// If `recursive` is true, includes sessions from all descendant teams.
/// Org-scoped: if multiple teams share a name across orgs, only the first match is used.
/// F-004: CTE uses UNION (not UNION ALL) + depth limit to prevent cycles.
pub fn team_session_ids(
    conn: &Connection,
    team_name: &str,
    recursive: bool,
) -> rusqlite::Result<Vec<String>> {
    if recursive {
        // F-004: depth-bounded CTE with UNION (dedup prevents infinite cycles)
        let mut stmt = conn.prepare(
            &format!("WITH RECURSIVE team_tree(id, depth) AS (
                SELECT id, 0 FROM team WHERE name = ?1
                UNION
                SELECT t.id, tt.depth + 1 FROM team t JOIN team_tree tt ON t.parent_team_id = tt.id
                WHERE tt.depth < {}
            )
            SELECT s.id FROM session s
            WHERE s.team_id IN (SELECT id FROM team_tree)
              AND s.status = 'active'", MAX_TREE_DEPTH),
        )?;
        let rows = stmt.query_map(params![team_name], |row| row.get(0))?;
        rows.collect()
    } else {
        let mut stmt = conn.prepare(
            "SELECT s.id FROM session s
             JOIN team t ON s.team_id = t.id
             WHERE t.name = ?1 AND s.status = 'active'",
        )?;
        let rows = stmt.query_map(params![team_name], |row| row.get(0))?;
        rows.collect()
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
    fn test_org_crud() {
        let conn = setup();

        // Default org may or may not exist; create a new one
        let id = create_organization(&conn, "Acme Corp", Some("Test org")).unwrap();
        assert!(!id.is_empty());

        let orgs = list_organizations(&conn).unwrap();
        // Find our org in the list
        let acme = orgs.iter().find(|o| o["name"] == "Acme Corp").unwrap();
        assert_eq!(acme["id"], id);
        assert_eq!(acme["description"], "Test org");
    }

    #[test]
    fn test_team_tree_nested() {
        let conn = setup();
        let org_id = create_organization(&conn, "TreeOrg", None).unwrap();
        let now = forge_core::time::now_iso();

        // Parent team
        let parent_id = ulid::Ulid::new().to_string();
        conn.execute(
            "INSERT INTO team (id, name, organization_id, created_by, status, created_at)
             VALUES (?1, 'engineering', ?2, 'system', 'active', ?3)",
            params![parent_id, org_id, now],
        )
        .unwrap();

        // Child team
        let child_id = ulid::Ulid::new().to_string();
        conn.execute(
            "INSERT INTO team (id, name, organization_id, parent_team_id, created_by, status, created_at)
             VALUES (?1, 'backend', ?2, ?3, 'system', 'active', ?4)",
            params![child_id, org_id, parent_id, now],
        )
        .unwrap();

        let tree = team_tree(&conn, &org_id).unwrap();
        assert_eq!(tree.len(), 1, "one root node");
        assert_eq!(tree[0]["name"], "engineering");

        let children = tree[0]["children"].as_array().unwrap();
        assert_eq!(children.len(), 1);
        assert_eq!(children[0]["name"], "backend");
    }

    #[test]
    fn test_org_template_startup() {
        let conn = setup();
        let (org_id, count) = create_org_from_template(&conn, "startup", "StartupCo").unwrap();
        assert_eq!(count, 12);

        let tree = team_tree(&conn, &org_id).unwrap();
        // 3 root nodes: business, engineering, marketing (alphabetical)
        assert_eq!(tree.len(), 3);
        let root_names: Vec<&str> = tree.iter().map(|t| t["name"].as_str().unwrap()).collect();
        assert!(root_names.contains(&"engineering"));
        assert!(root_names.contains(&"business"));
        assert!(root_names.contains(&"marketing"));
    }

    #[test]
    fn test_org_template_unknown() {
        let conn = setup();
        let result = create_org_from_template(&conn, "nonexistent", "BadOrg");
        assert!(result.is_err());
        let err_msg = format!("{}", result.unwrap_err());
        assert!(
            err_msg.contains("nonexistent"),
            "error should mention the unknown template name"
        );
    }

    #[test]
    fn test_team_session_ids_recursive() {
        let conn = setup();
        let org_id = create_organization(&conn, "SessionOrg", None).unwrap();
        let now = forge_core::time::now_iso();

        // Parent team
        let parent_id = ulid::Ulid::new().to_string();
        conn.execute(
            "INSERT INTO team (id, name, organization_id, created_by, status, created_at)
             VALUES (?1, 'eng', ?2, 'system', 'active', ?3)",
            params![parent_id, org_id, now],
        )
        .unwrap();

        // Child team
        let child_id = ulid::Ulid::new().to_string();
        conn.execute(
            "INSERT INTO team (id, name, organization_id, parent_team_id, created_by, status, created_at)
             VALUES (?1, 'backend', ?2, ?3, 'system', 'active', ?4)",
            params![child_id, org_id, parent_id, now],
        )
        .unwrap();

        // Register sessions and assign to teams
        crate::sessions::register_session(&conn, "s1", "claude-code", Some("forge"), None, None, None).unwrap();
        conn.execute("UPDATE session SET team_id = ?1 WHERE id = 's1'", params![parent_id]).unwrap();

        crate::sessions::register_session(&conn, "s2", "claude-code", Some("forge"), None, None, None).unwrap();
        conn.execute("UPDATE session SET team_id = ?1 WHERE id = 's2'", params![child_id]).unwrap();

        crate::sessions::register_session(&conn, "s3", "claude-code", Some("forge"), None, None, None).unwrap();
        conn.execute("UPDATE session SET team_id = ?1 WHERE id = 's3'", params![child_id]).unwrap();

        // Non-recursive: only parent team sessions
        let non_rec = team_session_ids(&conn, "eng", false).unwrap();
        assert_eq!(non_rec.len(), 1);
        assert!(non_rec.contains(&"s1".to_string()));

        // Recursive: parent + child team sessions
        let rec = team_session_ids(&conn, "eng", true).unwrap();
        assert_eq!(rec.len(), 3);
        assert!(rec.contains(&"s1".to_string()));
        assert!(rec.contains(&"s2".to_string()));
        assert!(rec.contains(&"s3".to_string()));
    }
}
