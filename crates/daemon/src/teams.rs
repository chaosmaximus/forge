use forge_core::types::team::AgentTemplate;
use rusqlite::{params, Connection};
use serde_json::Value;

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

/// Fields to update on an agent template. All None = no-op.
pub struct TemplateUpdate<'a> {
    pub name: Option<&'a str>,
    pub description: Option<&'a str>,
    pub system_context: Option<&'a str>,
    pub identity_facets: Option<&'a str>,
    pub config_overrides: Option<&'a str>,
    pub knowledge_domains: Option<&'a str>,
    pub decision_style: Option<&'a str>,
}

pub fn update_agent_template(
    conn: &Connection,
    id: &str,
    update: &TemplateUpdate<'_>,
) -> rusqlite::Result<bool> {
    let name = update.name;
    let description = update.description;
    let system_context = update.system_context;
    let identity_facets = update.identity_facets;
    let config_overrides = update.config_overrides;
    let knowledge_domains = update.knowledge_domains;
    let decision_style = update.decision_style;
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

// ── Agent Lifecycle ──

/// Spawn an agent from a template — creates session, sets identity, joins team.
pub fn spawn_agent(
    conn: &Connection,
    template_name: &str,
    session_id: &str,
    project: Option<&str>,
    team: Option<&str>,
) -> rusqlite::Result<()> {
    // Look up template by name (org_id="default")
    let template = get_agent_template_by_name(conn, template_name, "default")?
        .ok_or_else(|| {
            rusqlite::Error::QueryReturnedNoRows
        })?;

    let now = forge_core::time::now_iso();

    // Register the session
    crate::sessions::register_session(
        conn,
        session_id,
        &template.agent_type,
        project,
        None, // cwd
        None, // capabilities
        None, // current_task
    )?;

    // Set template_id, agent_status, last_activity_at on the session
    conn.execute(
        "UPDATE session SET template_id = ?1, agent_status = 'idle', last_activity_at = ?2 WHERE id = ?3",
        params![template.id, now, session_id],
    )?;

    // Parse identity_facets JSON array and store each facet
    if let Ok(facets) = serde_json::from_str::<Vec<serde_json::Value>>(&template.identity_facets) {
        for facet_val in facets {
            let facet_name = facet_val.get("facet")
                .and_then(|v| v.as_str())
                .unwrap_or("role");
            let description = facet_val.get("description")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let strength = facet_val.get("strength")
                .and_then(|v| v.as_f64())
                .unwrap_or(0.8);

            let identity = forge_core::types::manas::IdentityFacet {
                id: ulid::Ulid::new().to_string(),
                agent: template.agent_type.clone(),
                facet: facet_name.to_string(),
                description: description.to_string(),
                strength,
                source: format!("template:{}", template.name),
                active: true,
                created_at: now.clone(),
                user_id: None,
            };
            crate::db::manas::store_identity(conn, &identity)?;
        }
    }

    // If team specified: add session to team
    if let Some(team_name) = team {
        // Look up team by name
        let team_id: Option<String> = conn.query_row(
            "SELECT id FROM team WHERE name = ?1",
            params![team_name],
            |row| row.get(0),
        ).ok();

        if let Some(tid) = team_id {
            conn.execute(
                "INSERT OR IGNORE INTO team_member (team_id, user_id, role, joined_at, session_id)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![tid, session_id, template.name, now, session_id],
            )?;
        }
    }

    Ok(())
}

/// List active agents (sessions with template_id set).
pub fn list_agents(
    conn: &Connection,
    team: Option<&str>,
    limit: usize,
) -> rusqlite::Result<Vec<Value>> {
    let mut agents = Vec::new();

    match team {
        Some(team_name) => {
            let mut stmt = conn.prepare(
                "SELECT s.id, s.agent, s.template_id, s.agent_status, s.current_task, s.last_activity_at, s.project
                 FROM session s
                 JOIN team_member tm ON tm.session_id = s.id
                 JOIN team t ON t.id = tm.team_id
                 WHERE s.template_id IS NOT NULL AND s.status = 'active'
                   AND t.name = ?1
                 ORDER BY s.last_activity_at DESC
                 LIMIT ?2"
            )?;
            let rows = stmt.query_map(params![team_name, limit as i64], |row| {
                Ok(serde_json::json!({
                    "session_id": row.get::<_, String>(0)?,
                    "agent": row.get::<_, String>(1)?,
                    "template_id": row.get::<_, Option<String>>(2)?,
                    "agent_status": row.get::<_, Option<String>>(3)?,
                    "current_task": row.get::<_, Option<String>>(4)?,
                    "last_activity_at": row.get::<_, Option<String>>(5)?,
                    "project": row.get::<_, Option<String>>(6)?,
                }))
            })?;
            for row in rows {
                agents.push(row?);
            }
        }
        None => {
            let mut stmt = conn.prepare(
                "SELECT id, agent, template_id, agent_status, current_task, last_activity_at, project
                 FROM session
                 WHERE template_id IS NOT NULL AND status = 'active'
                 ORDER BY last_activity_at DESC
                 LIMIT ?1"
            )?;
            let rows = stmt.query_map(params![limit as i64], |row| {
                Ok(serde_json::json!({
                    "session_id": row.get::<_, String>(0)?,
                    "agent": row.get::<_, String>(1)?,
                    "template_id": row.get::<_, Option<String>>(2)?,
                    "agent_status": row.get::<_, Option<String>>(3)?,
                    "current_task": row.get::<_, Option<String>>(4)?,
                    "last_activity_at": row.get::<_, Option<String>>(5)?,
                    "project": row.get::<_, Option<String>>(6)?,
                }))
            })?;
            for row in rows {
                agents.push(row?);
            }
        }
    }

    Ok(agents)
}

/// Manually update an agent's status.
pub fn update_agent_status(
    conn: &Connection,
    session_id: &str,
    status: &str,
    task: Option<&str>,
) -> rusqlite::Result<bool> {
    let now = forge_core::time::now_iso();
    let count = conn.execute(
        "UPDATE session SET agent_status = ?1, current_task = ?2, last_activity_at = ?3 WHERE id = ?4",
        params![status, task.unwrap_or(""), now, session_id],
    )?;
    Ok(count > 0)
}

/// Retire an agent (soft delete — preserves memories).
pub fn retire_agent(conn: &Connection, session_id: &str) -> rusqlite::Result<bool> {
    let now = forge_core::time::now_iso();
    let count = conn.execute(
        "UPDATE session SET agent_status = 'retired', ended_at = ?1, status = 'ended' WHERE id = ?2",
        params![now, session_id],
    )?;
    Ok(count > 0)
}

// ── Team Functions ──

/// Create a team with type (human/agent/mixed).
pub fn create_team(
    conn: &Connection,
    name: &str,
    team_type: Option<&str>,
    purpose: Option<&str>,
    org_id: Option<&str>,
    parent_team_id: Option<&str>,
) -> rusqlite::Result<String> {
    let id = ulid::Ulid::new().to_string();
    let now = forge_core::time::now_iso();
    let org = org_id.unwrap_or("default");
    let tt = team_type.unwrap_or("human");

    conn.execute(
        "INSERT INTO team (id, name, organization_id, parent_team_id, created_by, status, created_at, team_type, purpose)
         VALUES (?1, ?2, ?3, ?4, 'system', 'active', ?5, ?6, ?7)",
        params![id, name, org, parent_team_id, now, tt, purpose],
    )?;

    Ok(id)
}

/// List members of a team (including agent sessions).
pub fn list_team_members(
    conn: &Connection,
    team_name: &str,
) -> rusqlite::Result<Vec<Value>> {
    let mut members = Vec::new();
    let mut stmt = conn.prepare(
        "SELECT tm.user_id, tm.role, tm.joined_at, tm.session_id,
                s.agent, s.agent_status, s.current_task, s.template_id
         FROM team_member tm
         JOIN team t ON t.id = tm.team_id
         LEFT JOIN session s ON s.id = tm.session_id
         WHERE t.name = ?1
         ORDER BY tm.joined_at"
    )?;
    let rows = stmt.query_map(params![team_name], |row| {
        Ok(serde_json::json!({
            "user_id": row.get::<_, String>(0)?,
            "role": row.get::<_, String>(1)?,
            "joined_at": row.get::<_, String>(2)?,
            "session_id": row.get::<_, Option<String>>(3)?,
            "agent": row.get::<_, Option<String>>(4)?,
            "agent_status": row.get::<_, Option<String>>(5)?,
            "current_task": row.get::<_, Option<String>>(6)?,
            "template_id": row.get::<_, Option<String>>(7)?,
        }))
    })?;
    for row in rows {
        members.push(row?);
    }
    Ok(members)
}

/// Set the orchestrator session for a team.
pub fn set_team_orchestrator(
    conn: &Connection,
    team_name: &str,
    session_id: &str,
) -> rusqlite::Result<bool> {
    let count = conn.execute(
        "UPDATE team SET orchestrator_session_id = ?1 WHERE name = ?2",
        params![session_id, team_name],
    )?;
    Ok(count > 0)
}

/// Get full team status (members, active agents, meeting count).
pub fn team_status(
    conn: &Connection,
    team_name: &str,
) -> rusqlite::Result<Value> {
    // Get team info
    struct TeamRow { id: String, name: String, status: String, team_type: Option<String>, purpose: Option<String> }
    let team_row: Option<TeamRow> = conn.query_row(
        "SELECT id, name, status, team_type, purpose FROM team WHERE name = ?1",
        params![team_name],
        |row| Ok(TeamRow {
            id: row.get(0)?, name: row.get(1)?, status: row.get(2)?,
            team_type: row.get(3)?, purpose: row.get(4)?,
        }),
    ).ok();

    let tr = match team_row {
        Some(r) => r,
        None => return Ok(serde_json::json!({"error": "team not found"})),
    };

    let team_id = tr.id;
    let name = tr.name;
    let status = tr.status;
    let team_type = tr.team_type;
    let purpose = tr.purpose;

    // Count members
    let member_count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM team_member WHERE team_id = ?1",
        params![team_id],
        |row| row.get(0),
    ).unwrap_or(0);

    // Count active agents in team
    let active_agents: i64 = conn.query_row(
        "SELECT COUNT(*) FROM team_member tm
         JOIN session s ON s.id = tm.session_id
         WHERE tm.team_id = ?1 AND s.status = 'active' AND s.template_id IS NOT NULL",
        params![team_id],
        |row| row.get(0),
    ).unwrap_or(0);

    // Count meetings for this team (if meeting table exists)
    let meeting_count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM meeting WHERE team_id = ?1",
        params![team_id],
        |row| row.get(0),
    ).unwrap_or(0);

    // Get orchestrator
    let orchestrator: Option<String> = conn.query_row(
        "SELECT orchestrator_session_id FROM team WHERE id = ?1",
        params![team_id],
        |row| row.get(0),
    ).unwrap_or(None);

    Ok(serde_json::json!({
        "id": team_id,
        "name": name,
        "status": status,
        "team_type": team_type,
        "purpose": purpose,
        "member_count": member_count,
        "active_agents": active_agents,
        "meeting_count": meeting_count,
        "orchestrator_session_id": orchestrator,
    }))
}

// ── Meeting Protocol ──

/// Create a meeting — sends FISP messages to all participants.
/// Returns (meeting_id, participant_count).
pub fn create_meeting(
    conn: &Connection,
    team_id: &str,
    topic: &str,
    context: Option<&str>,
    orchestrator_session_id: &str,
    participant_session_ids: &[String],
) -> rusqlite::Result<(String, usize)> {
    let meeting_id = ulid::Ulid::new().to_string();
    let now = forge_core::time::now_iso();

    conn.execute(
        "INSERT INTO meeting (id, team_id, topic, context, status, orchestrator_session_id, created_at)
         VALUES (?1, ?2, ?3, ?4, 'collecting', ?5, ?6)",
        params![meeting_id, team_id, topic, context, orchestrator_session_id, now],
    )?;

    for session_id in participant_session_ids {
        // Look up template_name from session.template_id -> agent_template.name
        let template_name: String = conn.query_row(
            "SELECT COALESCE(at.name, s.id)
             FROM session s
             LEFT JOIN agent_template at ON at.id = s.template_id
             WHERE s.id = ?1",
            params![session_id],
            |row| row.get(0),
        ).unwrap_or_else(|_| session_id.clone());

        let participant_id = ulid::Ulid::new().to_string();
        conn.execute(
            "INSERT INTO meeting_participant (id, meeting_id, session_id, template_name, status)
             VALUES (?1, ?2, ?3, ?4, 'pending')",
            params![participant_id, meeting_id, session_id, template_name],
        )?;

        // Send FISP message to participant
        let msg_body = serde_json::json!({
            "meeting_id": meeting_id,
            "topic": topic,
            "context": context,
        });
        crate::sessions::send_message(
            conn,
            orchestrator_session_id,
            session_id,
            "request",
            "meeting",
            &msg_body.to_string(),
            None,
            None,
            Some(&meeting_id),
        )?;
    }

    Ok((meeting_id, participant_session_ids.len()))
}

/// Get meeting status + participant response statuses.
pub fn get_meeting_status(
    conn: &Connection,
    meeting_id: &str,
) -> rusqlite::Result<(Value, Vec<Value>)> {
    let meeting = conn.query_row(
        "SELECT id, team_id, topic, context, status, orchestrator_session_id, synthesis, decision, decision_memory_id, created_at, decided_at
         FROM meeting WHERE id = ?1",
        params![meeting_id],
        |row| {
            Ok(serde_json::json!({
                "id": row.get::<_, String>(0)?,
                "team_id": row.get::<_, String>(1)?,
                "topic": row.get::<_, String>(2)?,
                "context": row.get::<_, Option<String>>(3)?,
                "status": row.get::<_, String>(4)?,
                "orchestrator_session_id": row.get::<_, String>(5)?,
                "synthesis": row.get::<_, Option<String>>(6)?,
                "decision": row.get::<_, Option<String>>(7)?,
                "decision_memory_id": row.get::<_, Option<String>>(8)?,
                "created_at": row.get::<_, String>(9)?,
                "decided_at": row.get::<_, Option<String>>(10)?,
            }))
        },
    )?;

    let mut stmt = conn.prepare(
        "SELECT id, meeting_id, session_id, template_name, status, response, responded_at, confidence
         FROM meeting_participant WHERE meeting_id = ?1"
    )?;
    let rows = stmt.query_map(params![meeting_id], |row| {
        Ok(serde_json::json!({
            "id": row.get::<_, String>(0)?,
            "meeting_id": row.get::<_, String>(1)?,
            "session_id": row.get::<_, String>(2)?,
            "template_name": row.get::<_, String>(3)?,
            "status": row.get::<_, String>(4)?,
            "response": row.get::<_, Option<String>>(5)?,
            "responded_at": row.get::<_, Option<String>>(6)?,
            "confidence": row.get::<_, Option<f64>>(7)?,
        }))
    })?;

    let mut participants = Vec::new();
    for row in rows {
        participants.push(row?);
    }

    Ok((meeting, participants))
}

/// Get all participant responses for a meeting (only those who responded).
pub fn get_meeting_responses(
    conn: &Connection,
    meeting_id: &str,
) -> rusqlite::Result<Vec<Value>> {
    let mut stmt = conn.prepare(
        "SELECT id, session_id, template_name, response, responded_at, confidence
         FROM meeting_participant
         WHERE meeting_id = ?1 AND status = 'responded'"
    )?;
    let rows = stmt.query_map(params![meeting_id], |row| {
        Ok(serde_json::json!({
            "id": row.get::<_, String>(0)?,
            "session_id": row.get::<_, String>(1)?,
            "template_name": row.get::<_, String>(2)?,
            "response": row.get::<_, Option<String>>(3)?,
            "responded_at": row.get::<_, Option<String>>(4)?,
            "confidence": row.get::<_, Option<f64>>(5)?,
        }))
    })?;

    let mut responses = Vec::new();
    for row in rows {
        responses.push(row?);
    }
    Ok(responses)
}

/// Record a participant's response to a meeting.
/// Returns true if all participants have now responded.
pub fn record_meeting_response(
    conn: &Connection,
    meeting_id: &str,
    session_id: &str,
    response: &str,
    confidence: Option<f64>,
) -> rusqlite::Result<bool> {
    let now = forge_core::time::now_iso();
    let conf = confidence.unwrap_or(0.8);

    let updated = conn.execute(
        "UPDATE meeting_participant SET status = 'responded', response = ?1, responded_at = ?2, confidence = ?3
         WHERE meeting_id = ?4 AND session_id = ?5 AND status = 'pending'",
        params![response, now, conf, meeting_id, session_id],
    )?;

    if updated == 0 {
        return Ok(false);
    }

    // Check if all participants have responded
    let pending: i64 = conn.query_row(
        "SELECT COUNT(*) FROM meeting_participant WHERE meeting_id = ?1 AND status = 'pending'",
        params![meeting_id],
        |row| row.get(0),
    )?;

    Ok(pending == 0)
}

/// Store orchestrator synthesis for a meeting.
pub fn synthesize_meeting(
    conn: &Connection,
    meeting_id: &str,
    synthesis: &str,
) -> rusqlite::Result<bool> {
    let count = conn.execute(
        "UPDATE meeting SET synthesis = ?1, status = 'synthesizing'
         WHERE id = ?2 AND status IN ('collecting', 'timed_out')",
        params![synthesis, meeting_id],
    )?;
    Ok(count > 0)
}

/// Record decision, store as memory, close meeting.
/// Returns (updated, decision_memory_id).
pub fn decide_meeting(
    conn: &Connection,
    meeting_id: &str,
    decision: &str,
) -> rusqlite::Result<(bool, String)> {
    let now = forge_core::time::now_iso();

    // Get the meeting topic for the memory title
    let topic: String = conn.query_row(
        "SELECT topic FROM meeting WHERE id = ?1",
        params![meeting_id],
        |row| row.get(0),
    )?;

    // Store decision as memory
    let memory_id = ulid::Ulid::new().to_string();
    conn.execute(
        "INSERT INTO memory (id, memory_type, title, content, confidence, status, created_at, accessed_at)
         VALUES (?1, 'decision', ?2, ?3, 0.9, 'active', ?4, ?5)",
        params![memory_id, topic, decision, now, now],
    )?;

    // Update meeting
    let count = conn.execute(
        "UPDATE meeting SET decision = ?1, decision_memory_id = ?2, status = 'decided', decided_at = ?3
         WHERE id = ?4",
        params![decision, memory_id, now, meeting_id],
    )?;

    Ok((count > 0, memory_id))
}

/// List meetings, optionally filtered by team_id and status.
/// Get active meetings where this session is a participant.
/// Returns meetings with status open, collecting, or synthesizing.
pub fn get_active_meetings_for_session(
    conn: &Connection,
    session_id: &str,
) -> rusqlite::Result<Vec<Value>> {
    let sql = "SELECT m.id, m.topic, m.status,
        (SELECT COUNT(*) FROM meeting_participant WHERE meeting_id = m.id AND response IS NOT NULL) as responded,
        (SELECT COUNT(*) FROM meeting_participant WHERE meeting_id = m.id) as total
        FROM meeting m
        JOIN meeting_participant mp ON mp.meeting_id = m.id
        WHERE mp.session_id = ?1 AND m.status IN ('open', 'collecting', 'synthesizing')
        ORDER BY m.created_at DESC LIMIT 5";
    let mut stmt = conn.prepare(sql)?;
    let rows: Vec<Value> = stmt.query_map(params![session_id], |row| {
        Ok(serde_json::json!({
            "id": row.get::<_, String>(0)?,
            "topic": row.get::<_, String>(1)?,
            "status": row.get::<_, String>(2)?,
            "responded": row.get::<_, i64>(3)?,
            "total": row.get::<_, i64>(4)?,
        }))
    })?.collect::<rusqlite::Result<Vec<Value>>>()?;
    Ok(rows)
}

pub fn list_meetings(
    conn: &Connection,
    team_id: Option<&str>,
    status: Option<&str>,
    limit: usize,
) -> rusqlite::Result<Vec<Value>> {
    let mut sql = String::from(
        "SELECT m.id, m.team_id, m.topic, m.status, m.created_at, m.decided_at,
                (SELECT COUNT(*) FROM meeting_participant mp WHERE mp.meeting_id = m.id AND mp.status = 'responded') as responded,
                (SELECT COUNT(*) FROM meeting_participant mp2 WHERE mp2.meeting_id = m.id) as total
         FROM meeting m WHERE 1=1"
    );
    let mut values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
    let mut idx = 1;

    if let Some(tid) = team_id {
        sql.push_str(&format!(" AND m.team_id = ?{idx}"));
        values.push(Box::new(tid.to_string()));
        idx += 1;
    }
    if let Some(st) = status {
        sql.push_str(&format!(" AND m.status = ?{idx}"));
        values.push(Box::new(st.to_string()));
        idx += 1;
    }

    sql.push_str(&format!(" ORDER BY m.created_at DESC LIMIT ?{idx}"));
    values.push(Box::new(limit as i64));

    let mut stmt = conn.prepare(&sql)?;
    let params_ref: Vec<&dyn rusqlite::types::ToSql> =
        values.iter().map(|p| p.as_ref()).collect();
    let rows = stmt.query_map(params_ref.as_slice(), |row| {
        Ok(serde_json::json!({
            "id": row.get::<_, String>(0)?,
            "team_id": row.get::<_, String>(1)?,
            "topic": row.get::<_, String>(2)?,
            "status": row.get::<_, String>(3)?,
            "created_at": row.get::<_, String>(4)?,
            "decided_at": row.get::<_, Option<String>>(5)?,
            "responded": row.get::<_, i64>(6)?,
            "total": row.get::<_, i64>(7)?,
        }))
    })?;

    let mut meetings = Vec::new();
    for row in rows {
        meetings.push(row?);
    }
    Ok(meetings)
}

/// Get full meeting transcript including all responses and FISP messages.
pub fn get_meeting_transcript(
    conn: &Connection,
    meeting_id: &str,
) -> rusqlite::Result<Value> {
    // Get meeting details
    let meeting = conn.query_row(
        "SELECT id, team_id, topic, context, status, orchestrator_session_id, synthesis, decision, decision_memory_id, created_at, decided_at
         FROM meeting WHERE id = ?1",
        params![meeting_id],
        |row| {
            Ok(serde_json::json!({
                "id": row.get::<_, String>(0)?,
                "team_id": row.get::<_, String>(1)?,
                "topic": row.get::<_, String>(2)?,
                "context": row.get::<_, Option<String>>(3)?,
                "status": row.get::<_, String>(4)?,
                "orchestrator_session_id": row.get::<_, String>(5)?,
                "synthesis": row.get::<_, Option<String>>(6)?,
                "decision": row.get::<_, Option<String>>(7)?,
                "decision_memory_id": row.get::<_, Option<String>>(8)?,
                "created_at": row.get::<_, String>(9)?,
                "decided_at": row.get::<_, Option<String>>(10)?,
            }))
        },
    )?;

    // Get all participants with responses
    let mut stmt = conn.prepare(
        "SELECT session_id, template_name, status, response, responded_at, confidence
         FROM meeting_participant WHERE meeting_id = ?1
         ORDER BY responded_at"
    )?;
    let rows = stmt.query_map(params![meeting_id], |row| {
        Ok(serde_json::json!({
            "session_id": row.get::<_, String>(0)?,
            "template_name": row.get::<_, String>(1)?,
            "status": row.get::<_, String>(2)?,
            "response": row.get::<_, Option<String>>(3)?,
            "responded_at": row.get::<_, Option<String>>(4)?,
            "confidence": row.get::<_, Option<f64>>(5)?,
        }))
    })?;
    let mut participants = Vec::new();
    for row in rows {
        participants.push(row?);
    }

    // Get FISP messages for this meeting
    let mut msg_stmt = conn.prepare(
        "SELECT id, from_session, to_session, kind, topic, parts, status, created_at
         FROM session_message WHERE meeting_id = ?1
         ORDER BY created_at"
    )?;
    let msg_rows = msg_stmt.query_map(params![meeting_id], |row| {
        Ok(serde_json::json!({
            "id": row.get::<_, String>(0)?,
            "from_session": row.get::<_, String>(1)?,
            "to_session": row.get::<_, String>(2)?,
            "kind": row.get::<_, String>(3)?,
            "topic": row.get::<_, String>(4)?,
            "parts": row.get::<_, String>(5)?,
            "status": row.get::<_, String>(6)?,
            "created_at": row.get::<_, String>(7)?,
        }))
    })?;
    let mut messages = Vec::new();
    for row in msg_rows {
        messages.push(row?);
    }

    Ok(serde_json::json!({
        "meeting": meeting,
        "participants": participants,
        "messages": messages,
    }))
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
        // 3 seeded defaults (claude-code, codex-cli, gemini-cli) + 3 manual (CTO, CMO, CFO) = 6
        assert_eq!(all.len(), 6);
        // Verify our 3 manual templates are present
        let names: Vec<&str> = all.iter().map(|t| t.name.as_str()).collect();
        assert!(names.contains(&"CFO"));
        assert!(names.contains(&"CMO"));
        assert!(names.contains(&"CTO"));
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
            &TemplateUpdate {
                name: Some("Chief Tech Officer"),
                description: Some("Updated description"),
                system_context: None, identity_facets: None,
                config_overrides: None, knowledge_domains: None,
                decision_style: Some("conservative"),
            },
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

    // ── Agent Lifecycle Tests ──

    #[test]
    fn test_spawn_agent_from_template() {
        let conn = setup();
        let t = make_template("CTO");
        create_agent_template(&conn, &t).unwrap();

        spawn_agent(&conn, "CTO", "s-cto-1", Some("forge"), None).unwrap();

        // Verify session exists with template_id set
        let template_id: Option<String> = conn.query_row(
            "SELECT template_id FROM session WHERE id = 's-cto-1'",
            [],
            |row| row.get(0),
        ).unwrap();
        assert_eq!(template_id, Some(t.id));

        // Verify agent_status is 'idle'
        let status: String = conn.query_row(
            "SELECT agent_status FROM session WHERE id = 's-cto-1'",
            [],
            |row| row.get(0),
        ).unwrap();
        assert_eq!(status, "idle");
    }

    #[test]
    fn test_spawn_agent_sets_identity() {
        let conn = setup();
        let t = make_template("CTO");
        create_agent_template(&conn, &t).unwrap();

        spawn_agent(&conn, "CTO", "s-cto-2", None, None).unwrap();

        // The template has identity_facets: [{"facet":"role","description":"test"}]
        // Verify at least one identity facet was stored
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM identity WHERE source LIKE 'template:%'",
            [],
            |row| row.get(0),
        ).unwrap();
        assert!(count >= 1, "expected at least 1 identity facet, got {count}");
    }

    #[test]
    fn test_spawn_invalid_template() {
        let conn = setup();
        let result = spawn_agent(&conn, "NonExistent", "s-bad", None, None);
        assert!(result.is_err(), "spawning from nonexistent template should fail");
    }

    #[test]
    fn test_list_agents() {
        let conn = setup();
        let t1 = make_template("CTO");
        let t2 = make_template("CMO");
        create_agent_template(&conn, &t1).unwrap();
        create_agent_template(&conn, &t2).unwrap();

        spawn_agent(&conn, "CTO", "s-cto-3", Some("forge"), None).unwrap();
        spawn_agent(&conn, "CMO", "s-cmo-3", Some("forge"), None).unwrap();

        let agents = list_agents(&conn, None, 50).unwrap();
        assert_eq!(agents.len(), 2, "expected 2 agents");
    }

    #[test]
    fn test_list_agents_by_team() {
        let conn = setup();
        let t = make_template("CTO");
        create_agent_template(&conn, &t).unwrap();

        // Create a team
        let team_id = create_team(&conn, "leadership", Some("agent"), None, None, None).unwrap();
        assert!(!team_id.is_empty());

        // Spawn agent into team
        spawn_agent(&conn, "CTO", "s-cto-team", Some("forge"), Some("leadership")).unwrap();

        // Spawn another agent NOT in team
        let t2 = make_template("CFO");
        create_agent_template(&conn, &t2).unwrap();
        spawn_agent(&conn, "CFO", "s-cfo-noteam", Some("forge"), None).unwrap();

        // List by team should only return the one in leadership
        let agents = list_agents(&conn, Some("leadership"), 50).unwrap();
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0]["session_id"], "s-cto-team");

        // List all should return both
        let all = list_agents(&conn, None, 50).unwrap();
        assert_eq!(all.len(), 2);
    }

    #[test]
    fn test_update_agent_status() {
        let conn = setup();
        let t = make_template("CTO");
        create_agent_template(&conn, &t).unwrap();
        spawn_agent(&conn, "CTO", "s-cto-4", None, None).unwrap();

        let updated = update_agent_status(&conn, "s-cto-4", "thinking", Some("reviewing architecture")).unwrap();
        assert!(updated);

        let status: String = conn.query_row(
            "SELECT agent_status FROM session WHERE id = 's-cto-4'",
            [],
            |row| row.get(0),
        ).unwrap();
        assert_eq!(status, "thinking");

        let task: String = conn.query_row(
            "SELECT current_task FROM session WHERE id = 's-cto-4'",
            [],
            |row| row.get(0),
        ).unwrap();
        assert_eq!(task, "reviewing architecture");
    }

    #[test]
    fn test_retire_agent() {
        let conn = setup();
        let t = make_template("CTO");
        create_agent_template(&conn, &t).unwrap();
        spawn_agent(&conn, "CTO", "s-cto-5", None, None).unwrap();

        let retired = retire_agent(&conn, "s-cto-5").unwrap();
        assert!(retired);

        let status: String = conn.query_row(
            "SELECT agent_status FROM session WHERE id = 's-cto-5'",
            [],
            |row| row.get(0),
        ).unwrap();
        assert_eq!(status, "retired");

        let session_status: String = conn.query_row(
            "SELECT status FROM session WHERE id = 's-cto-5'",
            [],
            |row| row.get(0),
        ).unwrap();
        assert_eq!(session_status, "ended");

        // Retired agents should not appear in list_agents (status != 'active')
        let agents = list_agents(&conn, None, 50).unwrap();
        assert_eq!(agents.len(), 0);
    }

    // ── Team Tests ──

    #[test]
    fn test_create_team_agent_type() {
        let conn = setup();
        let id = create_team(&conn, "ai-team", Some("agent"), Some("AI research"), None, None).unwrap();
        assert!(!id.is_empty());

        let tt: String = conn.query_row(
            "SELECT team_type FROM team WHERE id = ?1",
            params![id],
            |row| row.get(0),
        ).unwrap();
        assert_eq!(tt, "agent");

        let purpose: Option<String> = conn.query_row(
            "SELECT purpose FROM team WHERE id = ?1",
            params![id],
            |row| row.get(0),
        ).unwrap();
        assert_eq!(purpose, Some("AI research".to_string()));
    }

    #[test]
    fn test_set_orchestrator() {
        let conn = setup();
        create_team(&conn, "orch-team", Some("agent"), None, None, None).unwrap();

        // Create a session to be orchestrator
        let t = make_template("CTO");
        create_agent_template(&conn, &t).unwrap();
        spawn_agent(&conn, "CTO", "s-orch", None, None).unwrap();

        let set = set_team_orchestrator(&conn, "orch-team", "s-orch").unwrap();
        assert!(set);

        let orch: Option<String> = conn.query_row(
            "SELECT orchestrator_session_id FROM team WHERE name = 'orch-team'",
            [],
            |row| row.get(0),
        ).unwrap();
        assert_eq!(orch, Some("s-orch".to_string()));
    }

    #[test]
    fn test_team_status() {
        let conn = setup();
        create_team(&conn, "status-team", Some("mixed"), Some("testing"), None, None).unwrap();

        let t = make_template("CTO");
        create_agent_template(&conn, &t).unwrap();
        spawn_agent(&conn, "CTO", "s-status-1", None, Some("status-team")).unwrap();

        let status = team_status(&conn, "status-team").unwrap();
        assert_eq!(status["name"], "status-team");
        assert_eq!(status["team_type"], "mixed");
        assert_eq!(status["purpose"], "testing");
        assert_eq!(status["member_count"], 1);
        assert_eq!(status["active_agents"], 1);
    }

    // ── Meeting Protocol Tests ──

    fn setup_meeting_env(conn: &Connection) -> (String, String, String, String) {
        // Create templates
        let t1 = make_template("CTO");
        let t2 = make_template("CMO");
        create_agent_template(conn, &t1).unwrap();
        create_agent_template(conn, &t2).unwrap();

        // Create team
        let team_id = create_team(conn, "leadership", Some("agent"), None, None, None).unwrap();

        // Spawn agents (creates sessions)
        spawn_agent(conn, "CTO", "s-cto-m", Some("forge"), Some("leadership")).unwrap();
        spawn_agent(conn, "CMO", "s-cmo-m", Some("forge"), Some("leadership")).unwrap();

        // Create orchestrator session
        crate::sessions::register_session(conn, "s-orch-m", "orchestrator", Some("forge"), None, None, None).unwrap();

        (team_id, "s-orch-m".into(), "s-cto-m".into(), "s-cmo-m".into())
    }

    #[test]
    fn test_create_meeting() {
        let conn = setup();
        let (team_id, orch, cto, cmo) = setup_meeting_env(&conn);

        let (meeting_id, count) = create_meeting(
            &conn, &team_id, "Architecture review", Some("Q2 planning"),
            &orch, &[cto, cmo],
        ).unwrap();

        assert!(!meeting_id.is_empty());
        assert_eq!(count, 2);

        // Verify participants in pending status
        let pending: i64 = conn.query_row(
            "SELECT COUNT(*) FROM meeting_participant WHERE meeting_id = ?1 AND status = 'pending'",
            params![meeting_id],
            |row| row.get(0),
        ).unwrap();
        assert_eq!(pending, 2);

        // Verify meeting status is 'collecting'
        let status: String = conn.query_row(
            "SELECT status FROM meeting WHERE id = ?1",
            params![meeting_id],
            |row| row.get(0),
        ).unwrap();
        assert_eq!(status, "collecting");
    }

    #[test]
    fn test_meeting_status() {
        let conn = setup();
        let (team_id, orch, cto, cmo) = setup_meeting_env(&conn);

        let (meeting_id, _) = create_meeting(
            &conn, &team_id, "Status check", None, &orch, &[cto, cmo],
        ).unwrap();

        let (meeting, participants) = get_meeting_status(&conn, &meeting_id).unwrap();
        assert_eq!(meeting["topic"], "Status check");
        assert_eq!(meeting["status"], "collecting");
        assert_eq!(participants.len(), 2);
        assert_eq!(participants[0]["status"], "pending");
        assert_eq!(participants[1]["status"], "pending");
    }

    #[test]
    fn test_record_response() {
        let conn = setup();
        let (team_id, orch, cto, _cmo) = setup_meeting_env(&conn);

        let (meeting_id, _) = create_meeting(
            &conn, &team_id, "Response test", None, &orch, &[cto.clone()],
        ).unwrap();

        // Record response
        let all_responded = record_meeting_response(
            &conn, &meeting_id, &cto, "I think we should use Rust", Some(0.95),
        ).unwrap();

        // Only one participant, so all responded
        assert!(all_responded);

        // Verify participant status changed
        let status: String = conn.query_row(
            "SELECT status FROM meeting_participant WHERE meeting_id = ?1 AND session_id = ?2",
            params![meeting_id, cto],
            |row| row.get(0),
        ).unwrap();
        assert_eq!(status, "responded");
    }

    #[test]
    fn test_all_responded() {
        let conn = setup();
        let (team_id, orch, cto, cmo) = setup_meeting_env(&conn);

        let (meeting_id, _) = create_meeting(
            &conn, &team_id, "All respond test", None, &orch, &[cto.clone(), cmo.clone()],
        ).unwrap();

        // First response — not all responded yet
        let all = record_meeting_response(&conn, &meeting_id, &cto, "Yes", Some(0.9)).unwrap();
        assert!(!all);

        // Second response — all responded
        let all = record_meeting_response(&conn, &meeting_id, &cmo, "Agreed", Some(0.85)).unwrap();
        assert!(all);
    }

    #[test]
    fn test_partial_response() {
        let conn = setup();
        let (team_id, orch, cto, cmo) = setup_meeting_env(&conn);

        let (meeting_id, _) = create_meeting(
            &conn, &team_id, "Partial test", None, &orch, &[cto.clone(), cmo],
        ).unwrap();

        // Only CTO responds
        let all = record_meeting_response(&conn, &meeting_id, &cto, "Agree", None).unwrap();
        assert!(!all, "not all responded yet");

        // Only 1 response should be returned
        let responses = get_meeting_responses(&conn, &meeting_id).unwrap();
        assert_eq!(responses.len(), 1);
    }

    #[test]
    fn test_synthesize_meeting() {
        let conn = setup();
        let (team_id, orch, cto, _cmo) = setup_meeting_env(&conn);

        let (meeting_id, _) = create_meeting(
            &conn, &team_id, "Synthesis test", None, &orch, &[cto],
        ).unwrap();

        let updated = synthesize_meeting(&conn, &meeting_id, "Everyone agrees on Rust").unwrap();
        assert!(updated);

        // Verify status changed
        let status: String = conn.query_row(
            "SELECT status FROM meeting WHERE id = ?1",
            params![meeting_id],
            |row| row.get(0),
        ).unwrap();
        assert_eq!(status, "synthesizing");

        // Verify synthesis stored
        let synthesis: String = conn.query_row(
            "SELECT synthesis FROM meeting WHERE id = ?1",
            params![meeting_id],
            |row| row.get(0),
        ).unwrap();
        assert_eq!(synthesis, "Everyone agrees on Rust");
    }

    #[test]
    fn test_decide_meeting() {
        let conn = setup();
        let (team_id, orch, cto, _cmo) = setup_meeting_env(&conn);

        let (meeting_id, _) = create_meeting(
            &conn, &team_id, "Decision test", None, &orch, &[cto],
        ).unwrap();

        let (updated, memory_id) = decide_meeting(&conn, &meeting_id, "We will use Rust").unwrap();
        assert!(updated);
        assert!(!memory_id.is_empty());

        // Verify status changed
        let status: String = conn.query_row(
            "SELECT status FROM meeting WHERE id = ?1",
            params![meeting_id],
            |row| row.get(0),
        ).unwrap();
        assert_eq!(status, "decided");

        // Verify decision_memory_id is set
        let stored_mid: String = conn.query_row(
            "SELECT decision_memory_id FROM meeting WHERE id = ?1",
            params![meeting_id],
            |row| row.get(0),
        ).unwrap();
        assert_eq!(stored_mid, memory_id);
    }

    #[test]
    fn test_decide_creates_memory() {
        let conn = setup();
        let (team_id, orch, cto, _cmo) = setup_meeting_env(&conn);

        let (meeting_id, _) = create_meeting(
            &conn, &team_id, "Memory check", None, &orch, &[cto],
        ).unwrap();

        let (_, memory_id) = decide_meeting(&conn, &meeting_id, "Rust is the way").unwrap();

        // Verify memory record exists
        let title: String = conn.query_row(
            "SELECT title FROM memory WHERE id = ?1",
            params![memory_id],
            |row| row.get(0),
        ).unwrap();
        assert_eq!(title, "Memory check");

        let content: String = conn.query_row(
            "SELECT content FROM memory WHERE id = ?1",
            params![memory_id],
            |row| row.get(0),
        ).unwrap();
        assert_eq!(content, "Rust is the way");

        let mem_type: String = conn.query_row(
            "SELECT memory_type FROM memory WHERE id = ?1",
            params![memory_id],
            |row| row.get(0),
        ).unwrap();
        assert_eq!(mem_type, "decision");
    }

    #[test]
    fn test_list_meetings() {
        let conn = setup();
        let (team_id, orch, cto, cmo) = setup_meeting_env(&conn);

        create_meeting(&conn, &team_id, "Meeting 1", None, &orch, &[cto.clone()]).unwrap();
        create_meeting(&conn, &team_id, "Meeting 2", None, &orch, &[cmo.clone()]).unwrap();

        // List all meetings for team
        let meetings = list_meetings(&conn, Some(&team_id), None, 50).unwrap();
        assert_eq!(meetings.len(), 2);

        // List by status
        let collecting = list_meetings(&conn, Some(&team_id), Some("collecting"), 50).unwrap();
        assert_eq!(collecting.len(), 2);

        let decided = list_meetings(&conn, Some(&team_id), Some("decided"), 50).unwrap();
        assert_eq!(decided.len(), 0);
    }

    #[test]
    fn test_meeting_transcript() {
        let conn = setup();
        let (team_id, orch, cto, cmo) = setup_meeting_env(&conn);

        let (meeting_id, _) = create_meeting(
            &conn, &team_id, "Transcript test", Some("Full context"), &orch, &[cto.clone(), cmo.clone()],
        ).unwrap();

        // Record responses
        record_meeting_response(&conn, &meeting_id, &cto, "CTO says yes", Some(0.9)).unwrap();
        record_meeting_response(&conn, &meeting_id, &cmo, "CMO agrees", Some(0.85)).unwrap();

        // Synthesize and decide
        synthesize_meeting(&conn, &meeting_id, "Unanimous agreement").unwrap();
        decide_meeting(&conn, &meeting_id, "Approved for Q2").unwrap();

        // Get transcript
        let transcript = get_meeting_transcript(&conn, &meeting_id).unwrap();
        assert_eq!(transcript["meeting"]["topic"], "Transcript test");
        assert_eq!(transcript["meeting"]["context"], "Full context");
        assert_eq!(transcript["meeting"]["synthesis"], "Unanimous agreement");
        assert_eq!(transcript["meeting"]["decision"], "Approved for Q2");
        assert_eq!(transcript["participants"].as_array().unwrap().len(), 2);
        // FISP messages should be present (one per participant from create_meeting)
        assert!(transcript["messages"].as_array().unwrap().len() >= 2);
    }

    #[test]
    fn test_state_machine_guard() {
        let conn = setup();
        let (team_id, orch, cto, _cmo) = setup_meeting_env(&conn);

        let (meeting_id, _) = create_meeting(
            &conn, &team_id, "Guard test", None, &orch, &[cto],
        ).unwrap();

        // Decide (changes status to 'decided')
        decide_meeting(&conn, &meeting_id, "Done").unwrap();

        // Cannot synthesize a decided meeting (status must be 'collecting' or 'timed_out')
        let updated = synthesize_meeting(&conn, &meeting_id, "Late synthesis").unwrap();
        assert!(!updated, "should not be able to synthesize a decided meeting");
    }

    #[test]
    fn test_cross_team_meeting() {
        let conn = setup();
        // Create two teams
        let team1_id = create_team(&conn, "team-alpha", Some("agent"), None, None, None).unwrap();
        let _team2_id = create_team(&conn, "team-beta", Some("agent"), None, None, None).unwrap();

        // Create templates and spawn agents into different teams
        let t1 = make_template("CTO");
        let t2 = make_template("CMO");
        create_agent_template(&conn, &t1).unwrap();
        create_agent_template(&conn, &t2).unwrap();

        spawn_agent(&conn, "CTO", "s-alpha-cto", Some("forge"), Some("team-alpha")).unwrap();
        spawn_agent(&conn, "CMO", "s-beta-cmo", Some("forge"), Some("team-beta")).unwrap();

        // Create orchestrator
        crate::sessions::register_session(&conn, "s-cross-orch", "orchestrator", Some("forge"), None, None, None).unwrap();

        // Create meeting under team1 but with participant from team2
        let (meeting_id, count) = create_meeting(
            &conn, &team1_id, "Cross-team sync", None,
            "s-cross-orch", &["s-alpha-cto".into(), "s-beta-cmo".into()],
        ).unwrap();

        assert_eq!(count, 2);

        // Both can respond
        let _ = record_meeting_response(&conn, &meeting_id, "s-alpha-cto", "Alpha input", Some(0.9)).unwrap();
        let all = record_meeting_response(&conn, &meeting_id, "s-beta-cmo", "Beta input", Some(0.85)).unwrap();
        assert!(all, "both participants responded");

        // Responses from both teams
        let responses = get_meeting_responses(&conn, &meeting_id).unwrap();
        assert_eq!(responses.len(), 2);
    }
}
