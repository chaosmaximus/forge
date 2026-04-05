use forge_core::types::team::AgentTemplate;
use rusqlite::{params, Connection};

// ── Agent Template CRUD ──

pub fn create_agent_template(conn: &Connection, t: &AgentTemplate) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO agent_template (id, name, description, agent_type, organization_id,
         system_context, identity_facets, config_overrides, knowledge_domains, decision_style,
         created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
        params![
            t.id, t.name, t.description, t.agent_type, t.organization_id,
            t.system_context, t.identity_facets, t.config_overrides,
            t.knowledge_domains, t.decision_style, t.created_at, t.updated_at,
        ],
    )?;
    Ok(())
}

pub fn get_agent_template(conn: &Connection, id: &str) -> rusqlite::Result<Option<AgentTemplate>> {
    let mut stmt = conn.prepare(
        "SELECT id, name, description, agent_type, organization_id,
         system_context, identity_facets, config_overrides, knowledge_domains,
         decision_style, created_at, updated_at
         FROM agent_template WHERE id = ?1"
    )?;
    let result = stmt.query_row(params![id], row_to_template).ok();
    Ok(result)
}

pub fn get_agent_template_by_name(
    conn: &Connection,
    name: &str,
    org_id: &str,
) -> rusqlite::Result<Option<AgentTemplate>> {
    let mut stmt = conn.prepare(
        "SELECT id, name, description, agent_type, organization_id,
         system_context, identity_facets, config_overrides, knowledge_domains,
         decision_style, created_at, updated_at
         FROM agent_template WHERE name = ?1 AND organization_id = ?2"
    )?;
    let result = stmt.query_row(params![name, org_id], row_to_template).ok();
    Ok(result)
}

pub fn list_agent_templates(
    conn: &Connection,
    org_id: Option<&str>,
    limit: usize,
) -> rusqlite::Result<Vec<AgentTemplate>> {
    let (sql, param_values): (&str, Vec<Box<dyn rusqlite::types::ToSql>>) = match org_id {
        Some(oid) => (
            "SELECT id, name, description, agent_type, organization_id,
             system_context, identity_facets, config_overrides, knowledge_domains,
             decision_style, created_at, updated_at
             FROM agent_template WHERE organization_id = ?1
             ORDER BY name LIMIT ?2",
            vec![Box::new(oid.to_string()), Box::new(limit as i64)],
        ),
        None => (
            "SELECT id, name, description, agent_type, organization_id,
             system_context, identity_facets, config_overrides, knowledge_domains,
             decision_style, created_at, updated_at
             FROM agent_template ORDER BY name LIMIT ?1",
            vec![Box::new(limit as i64)],
        ),
    };
    let mut stmt = conn.prepare(sql)?;
    let params_ref: Vec<&dyn rusqlite::types::ToSql> =
        param_values.iter().map(|p| p.as_ref()).collect();
    let rows = stmt.query_map(params_ref.as_slice(), row_to_template)?;
    rows.collect()
}

pub fn delete_agent_template(conn: &Connection, id: &str) -> rusqlite::Result<bool> {
    let count = conn.execute("DELETE FROM agent_template WHERE id = ?1", params![id])?;
    Ok(count > 0)
}

pub fn update_agent_template(
    conn: &Connection,
    id: &str,
    name: Option<&str>,
    description: Option<&str>,
    system_context: Option<&str>,
    identity_facets: Option<&str>,
    config_overrides: Option<&str>,
    knowledge_domains: Option<&str>,
    decision_style: Option<&str>,
) -> rusqlite::Result<bool> {
    let mut sets = Vec::new();
    let mut values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
    let mut idx = 1;

    macro_rules! add_field {
        ($opt:expr, $col:expr) => {
            if let Some(v) = $opt {
                sets.push(format!("{} = ?{}", $col, idx));
                values.push(Box::new(v.to_string()));
                idx += 1;
            }
        };
    }

    add_field!(name, "name");
    add_field!(description, "description");
    add_field!(system_context, "system_context");
    add_field!(identity_facets, "identity_facets");
    add_field!(config_overrides, "config_overrides");
    add_field!(knowledge_domains, "knowledge_domains");
    add_field!(decision_style, "decision_style");

    if sets.is_empty() {
        return Ok(false);
    }

    sets.push(format!("updated_at = ?{}", idx));
    values.push(Box::new(forge_core::time::now_iso()));
    idx += 1;

    let sql = format!(
        "UPDATE agent_template SET {} WHERE id = ?{}",
        sets.join(", "),
        idx
    );
    values.push(Box::new(id.to_string()));

    let params_ref: Vec<&dyn rusqlite::types::ToSql> =
        values.iter().map(|p| p.as_ref()).collect();
    let count = conn.execute(&sql, params_ref.as_slice())?;
    Ok(count > 0)
}

fn row_to_template(row: &rusqlite::Row) -> rusqlite::Result<AgentTemplate> {
    Ok(AgentTemplate {
        id: row.get(0)?,
        name: row.get(1)?,
        description: row.get(2)?,
        agent_type: row.get(3)?,
        organization_id: row.get(4)?,
        system_context: row.get(5)?,
        identity_facets: row.get(6)?,
        config_overrides: row.get(7)?,
        knowledge_domains: row.get(8)?,
        decision_style: row.get(9)?,
        created_at: row.get(10)?,
        updated_at: row.get(11)?,
    })
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

    fn make_template(name: &str) -> AgentTemplate {
        let now = forge_core::time::now_iso();
        AgentTemplate {
            id: ulid::Ulid::new().to_string(),
            name: name.into(),
            description: format!("{name} agent"),
            agent_type: "claude-code".into(),
            organization_id: "default".into(),
            system_context: format!("You are the {name}"),
            identity_facets: r#"[{"facet":"role","description":"test"}]"#.into(),
            config_overrides: r#"{"context.budget_chars":5000}"#.into(),
            knowledge_domains: r#"["architecture","scalability"]"#.into(),
            decision_style: "analytical".into(),
            created_at: now.clone(),
            updated_at: now,
        }
    }

    #[test]
    fn test_create_and_get_template() {
        let conn = setup();
        let t = make_template("CTO");
        create_agent_template(&conn, &t).unwrap();

        let fetched = get_agent_template(&conn, &t.id).unwrap().unwrap();
        assert_eq!(fetched.name, "CTO");
        assert_eq!(fetched.agent_type, "claude-code");
        assert_eq!(fetched.decision_style, "analytical");
    }

    #[test]
    fn test_get_by_name() {
        let conn = setup();
        let t = make_template("CMO");
        create_agent_template(&conn, &t).unwrap();

        let fetched = get_agent_template_by_name(&conn, "CMO", "default").unwrap().unwrap();
        assert_eq!(fetched.id, t.id);
    }

    #[test]
    fn test_list_templates() {
        let conn = setup();
        create_agent_template(&conn, &make_template("CTO")).unwrap();
        create_agent_template(&conn, &make_template("CMO")).unwrap();
        create_agent_template(&conn, &make_template("CFO")).unwrap();

        let all = list_agent_templates(&conn, None, 100).unwrap();
        assert_eq!(all.len(), 3);
        // Should be alphabetical
        assert_eq!(all[0].name, "CFO");
        assert_eq!(all[1].name, "CMO");
        assert_eq!(all[2].name, "CTO");
    }

    #[test]
    fn test_list_by_org() {
        let conn = setup();
        let mut t1 = make_template("CTO");
        t1.organization_id = "org-a".into();
        let mut t2 = make_template("CMO");
        t2.organization_id = "org-b".into();
        create_agent_template(&conn, &t1).unwrap();
        create_agent_template(&conn, &t2).unwrap();

        let org_a = list_agent_templates(&conn, Some("org-a"), 100).unwrap();
        assert_eq!(org_a.len(), 1);
        assert_eq!(org_a[0].name, "CTO");
    }

    #[test]
    fn test_delete_template() {
        let conn = setup();
        let t = make_template("CTO");
        create_agent_template(&conn, &t).unwrap();

        let deleted = delete_agent_template(&conn, &t.id).unwrap();
        assert!(deleted);

        let fetched = get_agent_template(&conn, &t.id).unwrap();
        assert!(fetched.is_none());
    }

    #[test]
    fn test_delete_nonexistent() {
        let conn = setup();
        let deleted = delete_agent_template(&conn, "nonexistent").unwrap();
        assert!(!deleted);
    }

    #[test]
    fn test_update_template() {
        let conn = setup();
        let t = make_template("CTO");
        create_agent_template(&conn, &t).unwrap();

        let updated = update_agent_template(
            &conn, &t.id,
            Some("Chief Tech Officer"),
            Some("Updated description"),
            None, None, None, None,
            Some("conservative"),
        ).unwrap();
        assert!(updated);

        let fetched = get_agent_template(&conn, &t.id).unwrap().unwrap();
        assert_eq!(fetched.name, "Chief Tech Officer");
        assert_eq!(fetched.description, "Updated description");
        assert_eq!(fetched.decision_style, "conservative");
        // Unchanged fields preserved
        assert_eq!(fetched.agent_type, "claude-code");
    }

    #[test]
    fn test_duplicate_name_same_org_rejected() {
        let conn = setup();
        let t1 = make_template("CTO");
        let mut t2 = make_template("CTO");
        t2.id = ulid::Ulid::new().to_string();
        create_agent_template(&conn, &t1).unwrap();
        let err = create_agent_template(&conn, &t2);
        assert!(err.is_err(), "Duplicate name+org should fail");
    }

    #[test]
    fn test_schema_idempotent() {
        let conn = setup();
        // Second call should not error
        create_schema(&conn).unwrap();
        // Tables still work
        let t = make_template("CTO");
        create_agent_template(&conn, &t).unwrap();
        let fetched = get_agent_template(&conn, &t.id).unwrap().unwrap();
        assert_eq!(fetched.name, "CTO");
    }
}
