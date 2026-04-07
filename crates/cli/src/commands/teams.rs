use crate::client;
use forge_core::protocol::{Request, Response, ResponseData};

// ── Agent Templates ──

#[allow(clippy::too_many_arguments)]
pub async fn create_agent_template(
    name: String,
    description: String,
    agent_type: String,
    system_context: Option<String>,
    identity_facets: Option<String>,
    config_overrides: Option<String>,
    knowledge_domains: Option<String>,
    decision_style: Option<String>,
) {
    let req = Request::CreateAgentTemplate {
        name: name.clone(),
        description,
        agent_type,
        organization_id: None,
        system_context,
        identity_facets,
        config_overrides,
        knowledge_domains,
        decision_style,
    };
    match client::send(&req).await {
        Ok(Response::Ok { data: ResponseData::AgentTemplateCreated { id, name } }) => {
            println!("Template created: {} ({})", &id[..13.min(id.len())], name);
        }
        Ok(Response::Error { message }) => {
            eprintln!("error: {message}");
            std::process::exit(1);
        }
        Ok(_) => {
            eprintln!("unexpected response");
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    }
}

pub async fn list_agent_templates(org: Option<String>) {
    let req = Request::ListAgentTemplates {
        organization_id: org,
        limit: None,
    };
    match client::send(&req).await {
        Ok(Response::Ok { data: ResponseData::AgentTemplateList { templates, count } }) => {
            if count == 0 {
                println!("No agent templates.");
                return;
            }
            println!("{count} template(s):");
            for t in &templates {
                println!("  {}: {} ({}) [{}]", t.name, t.description, t.agent_type, t.decision_style);
            }
        }
        Ok(Response::Error { message }) => {
            eprintln!("error: {message}");
            std::process::exit(1);
        }
        Ok(_) => {
            eprintln!("unexpected response");
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    }
}

pub async fn get_agent_template(id: Option<String>, name: Option<String>) {
    if id.is_none() && name.is_none() {
        eprintln!("error: must provide --id or --name");
        std::process::exit(1);
    }
    let req = Request::GetAgentTemplate { id, name };
    match client::send(&req).await {
        Ok(Response::Ok { data: ResponseData::AgentTemplateData { template } }) => {
            println!("Template: {}", template.name);
            println!("  ID:          {}", template.id);
            println!("  Description: {}", template.description);
            println!("  Agent type:  {}", template.agent_type);
            println!("  Org:         {}", template.organization_id);
            println!("  Decision:    {}", template.decision_style);
            if !template.system_context.is_empty() {
                let preview = if template.system_context.len() > 80 {
                    format!("{}...", &template.system_context[..80])
                } else {
                    template.system_context.clone()
                };
                println!("  Context:     {}", preview);
            }
            println!("  Created:     {}", template.created_at);
            println!("  Updated:     {}", template.updated_at);
        }
        Ok(Response::Error { message }) => {
            eprintln!("error: {message}");
            std::process::exit(1);
        }
        Ok(_) => {
            eprintln!("unexpected response");
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    }
}

pub async fn delete_agent_template(id: String) {
    let req = Request::DeleteAgentTemplate { id: id.clone() };
    match client::send(&req).await {
        Ok(Response::Ok { data: ResponseData::AgentTemplateDeleted { id, found } }) => {
            if found {
                println!("Template deleted: {}", &id[..13.min(id.len())]);
            } else {
                println!("Template not found: {}", &id[..13.min(id.len())]);
            }
        }
        Ok(Response::Error { message }) => {
            eprintln!("error: {message}");
            std::process::exit(1);
        }
        Ok(_) => {
            eprintln!("unexpected response");
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    }
}

// ── Agents ──

pub async fn spawn_agent(
    template: String,
    session_id: String,
    project: Option<String>,
    team: Option<String>,
) {
    let req = Request::SpawnAgent {
        template_name: template.clone(),
        session_id: session_id.clone(),
        project,
        team: team.clone(),
    };
    match client::send(&req).await {
        Ok(Response::Ok { data: ResponseData::AgentSpawned { session_id, template_name, team } }) => {
            let team_str = match &team {
                Some(t) => format!(", team={t}"),
                None => String::new(),
            };
            println!("Agent spawned: {} ({}{team_str})", session_id, template_name);
        }
        Ok(Response::Error { message }) => {
            eprintln!("error: {message}");
            std::process::exit(1);
        }
        Ok(_) => {
            eprintln!("unexpected response");
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    }
}

pub async fn list_agents(team: Option<String>) {
    let req = Request::ListAgents { team, limit: None };
    match client::send(&req).await {
        Ok(Response::Ok { data: ResponseData::AgentList { agents, count } }) => {
            if count == 0 {
                println!("No active agents.");
                return;
            }
            println!("{count} agent(s):");
            for agent in &agents {
                let session_id = agent.get("session_id").and_then(|v| v.as_str()).unwrap_or("?");
                let template = agent.get("template_name").and_then(|v| v.as_str()).unwrap_or("?");
                let agent_type = agent.get("agent_type").and_then(|v| v.as_str()).unwrap_or("?");
                let status = agent.get("status").and_then(|v| v.as_str()).unwrap_or("idle");
                let team = agent.get("team").and_then(|v| v.as_str()).unwrap_or("");
                let team_str = if team.is_empty() { String::new() } else { format!(" team={team}") };
                println!("  {session_id}: {template} ({agent_type}) [{status}]{team_str}");
            }
        }
        Ok(Response::Error { message }) => {
            eprintln!("error: {message}");
            std::process::exit(1);
        }
        Ok(_) => {
            eprintln!("unexpected response");
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    }
}

pub async fn update_agent_status(session_id: String, status: String, task: Option<String>) {
    let req = Request::UpdateAgentStatus {
        session_id: session_id.clone(),
        status: status.clone(),
        current_task: task,
    };
    match client::send(&req).await {
        Ok(Response::Ok { data: ResponseData::AgentStatusUpdated { session_id, status } }) => {
            println!("Agent status updated: {session_id} -> {status}");
        }
        Ok(Response::Error { message }) => {
            eprintln!("error: {message}");
            std::process::exit(1);
        }
        Ok(_) => {
            eprintln!("unexpected response");
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    }
}

pub async fn retire_agent(session_id: String) {
    let req = Request::RetireAgent { session_id: session_id.clone() };
    match client::send(&req).await {
        Ok(Response::Ok { data: ResponseData::AgentRetired { session_id } }) => {
            println!("Agent retired: {session_id}");
        }
        Ok(Response::Error { message }) => {
            eprintln!("error: {message}");
            std::process::exit(1);
        }
        Ok(_) => {
            eprintln!("unexpected response");
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    }
}

// ── Teams ──

pub async fn create_team(name: String, team_type: Option<String>, purpose: Option<String>, parent: Option<String>) {
    // TODO: pass `parent` as parent_team_id once Request::CreateTeam adds the field
    let _ = &parent;
    let req = Request::CreateTeam {
        name: name.clone(),
        team_type,
        purpose,
        organization_id: None,
    };
    match client::send(&req).await {
        Ok(Response::Ok { data: ResponseData::TeamCreated { id, name } }) => {
            println!("Team created: {} ({})", &id[..13.min(id.len())], name);
        }
        Ok(Response::Error { message }) => {
            eprintln!("error: {message}");
            std::process::exit(1);
        }
        Ok(_) => {
            eprintln!("unexpected response");
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    }
}

pub async fn list_team_members(team_name: String) {
    let req = Request::ListTeamMembers { team_name: team_name.clone() };
    match client::send(&req).await {
        Ok(Response::Ok { data: ResponseData::TeamMemberList { members, count } }) => {
            if count == 0 {
                println!("No members in team '{team_name}'.");
                return;
            }
            println!("{count} member(s) in team '{team_name}':");
            for m in &members {
                let session_id = m.get("session_id").and_then(|v| v.as_str()).unwrap_or("?");
                let template = m.get("template_name").and_then(|v| v.as_str()).unwrap_or("?");
                let status = m.get("status").and_then(|v| v.as_str()).unwrap_or("idle");
                println!("  {session_id}: {template} [{status}]");
            }
        }
        Ok(Response::Error { message }) => {
            eprintln!("error: {message}");
            std::process::exit(1);
        }
        Ok(_) => {
            eprintln!("unexpected response");
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    }
}

pub async fn set_team_orchestrator(team_name: String, session_id: String) {
    let req = Request::SetTeamOrchestrator {
        team_name: team_name.clone(),
        session_id: session_id.clone(),
    };
    match client::send(&req).await {
        Ok(Response::Ok { data: ResponseData::TeamOrchestratorSet { team_name, session_id } }) => {
            println!("Orchestrator set: {session_id} for team '{team_name}'");
        }
        Ok(Response::Error { message }) => {
            eprintln!("error: {message}");
            std::process::exit(1);
        }
        Ok(_) => {
            eprintln!("unexpected response");
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    }
}

pub async fn team_status(team_name: String, team_id: Option<String>) {
    let req = Request::TeamStatus { team_name: team_name.clone(), team_id };
    match client::send(&req).await {
        Ok(Response::Ok { data: ResponseData::TeamStatusData { team } }) => {
            // team is a JSON value — print human-readable summary
            let name = team.get("name").and_then(|v| v.as_str()).unwrap_or(&team_name);
            let team_type = team.get("team_type").and_then(|v| v.as_str()).unwrap_or("?");
            let purpose = team.get("purpose").and_then(|v| v.as_str()).unwrap_or("");
            let member_count = team.get("member_count").and_then(|v| v.as_u64()).unwrap_or(0);
            let orchestrator = team.get("orchestrator").and_then(|v| v.as_str()).unwrap_or("none");
            println!("Team: {name}");
            println!("  Type:         {team_type}");
            if !purpose.is_empty() {
                println!("  Purpose:      {purpose}");
            }
            println!("  Members:      {member_count}");
            println!("  Orchestrator: {orchestrator}");
        }
        Ok(Response::Error { message }) => {
            eprintln!("error: {message}");
            std::process::exit(1);
        }
        Ok(_) => {
            eprintln!("unexpected response");
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    }
}

pub async fn run_team(
    name: String,
    templates: Option<Vec<String>>,
    from_file: Option<String>,
    topology: Option<String>,
) {
    let (team_name, template_names, topo) = if let Some(path) = from_file {
        // Read and parse JSON config file
        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("error: cannot read file '{}': {}", path, e);
                std::process::exit(1);
            }
        };
        let config: serde_json::Value = match serde_json::from_str(&content) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("error: invalid JSON in '{}': {}", path, e);
                std::process::exit(1);
            }
        };
        let file_team_name = config
            .get("team_name")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        // Validate all template_names are strings — reject non-string entries
        let raw_templates = config.get("template_names").and_then(|v| v.as_array());
        let file_templates: Vec<String> = match raw_templates {
            Some(arr) => {
                let mut names = Vec::with_capacity(arr.len());
                for (i, v) in arr.iter().enumerate() {
                    match v.as_str() {
                        Some(s) => names.push(s.to_string()),
                        None => {
                            eprintln!("error: template_names[{}] is not a string in '{}'", i, path);
                            std::process::exit(1);
                        }
                    }
                }
                names
            }
            None => Vec::new(),
        };
        if file_templates.is_empty() {
            eprintln!("error: 'template_names' array is missing or empty in '{}'", path);
            std::process::exit(1);
        }
        let file_topology = config
            .get("topology")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        // CLI --name takes precedence over JSON team_name
        let resolved_name = if name.trim().is_empty() {
            match file_team_name {
                Some(n) => n,
                None => {
                    eprintln!("error: --name not provided and 'team_name' missing in '{}'", path);
                    std::process::exit(1);
                }
            }
        } else {
            name
        };
        // CLI --topology takes precedence over JSON topology
        let resolved_topology = topology.or(file_topology);
        (resolved_name, file_templates, resolved_topology)
    } else if let Some(tmpls) = templates {
        if tmpls.is_empty() {
            eprintln!("error: --templates must not be empty");
            std::process::exit(1);
        }
        (name, tmpls, topology)
    } else {
        eprintln!("error: must provide --templates or --from-file");
        std::process::exit(1);
    };

    // Reject whitespace-only team names
    let team_name = team_name.trim().to_string();
    if team_name.is_empty() {
        eprintln!("error: team name must not be empty or whitespace-only");
        std::process::exit(1);
    }

    let req = Request::RunTeam {
        team_name: team_name.clone(),
        template_names,
        topology: topo,
        goal: None,
    };
    match client::send(&req).await {
        Ok(Response::Ok { data: ResponseData::RunTeamResult { team_name, agents_spawned, session_ids } }) => {
            println!("Team '{}' started: {} agent(s) spawned", team_name, agents_spawned);
            for sid in &session_ids {
                println!("  {}", sid);
            }
        }
        Ok(Response::Error { message }) => {
            eprintln!("error: {message}");
            std::process::exit(1);
        }
        Ok(_) => {
            eprintln!("unexpected response");
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    }
}

pub async fn stop_team(name: String) {
    let name = name.trim().to_string();
    if name.is_empty() {
        eprintln!("error: team name must not be empty or whitespace-only");
        std::process::exit(1);
    }
    let req = Request::StopTeam { team_name: name.clone() };
    match client::send(&req).await {
        Ok(Response::Ok { data: ResponseData::TeamStopped { team_name, agents_retired } }) => {
            println!("Team '{}' stopped: {} agent(s) retired", team_name, agents_retired);
        }
        Ok(Response::Error { message }) => {
            eprintln!("error: {message}");
            std::process::exit(1);
        }
        Ok(_) => {
            eprintln!("unexpected response");
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    }
}

// ── Meetings ──

pub async fn create_meeting(
    team_id: String,
    topic: String,
    context: Option<String>,
    orchestrator: String,
    participants: Vec<String>,
) {
    let participant_count = participants.len();
    let req = Request::CreateMeeting {
        team_id,
        topic,
        context,
        orchestrator_session_id: orchestrator,
        participant_session_ids: participants,
        goal: None,
    };
    match client::send(&req).await {
        Ok(Response::Ok { data: ResponseData::MeetingCreated { meeting_id, participant_count: count } }) => {
            let display_count = if count > 0 { count } else { participant_count };
            println!("Meeting created: {} ({} participants)", &meeting_id[..13.min(meeting_id.len())], display_count);
        }
        Ok(Response::Error { message }) => {
            eprintln!("error: {message}");
            std::process::exit(1);
        }
        Ok(_) => {
            eprintln!("unexpected response");
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    }
}

pub async fn meeting_status(meeting_id: String) {
    let req = Request::MeetingStatus { meeting_id };
    match client::send(&req).await {
        Ok(Response::Ok { data: ResponseData::MeetingStatusData { meeting, participants } }) => {
            let topic = meeting.get("topic").and_then(|v| v.as_str()).unwrap_or("?");
            let status = meeting.get("status").and_then(|v| v.as_str()).unwrap_or("?");
            let mid = meeting.get("id").and_then(|v| v.as_str()).unwrap_or("?");
            println!("Meeting: {} [{}]", &mid[..13.min(mid.len())], status);
            println!("  Topic: {topic}");
            if let Some(ctx) = meeting.get("context").and_then(|v| v.as_str()) {
                if !ctx.is_empty() {
                    let preview = if ctx.len() > 80 { format!("{}...", &ctx[..80]) } else { ctx.to_string() };
                    println!("  Context: {preview}");
                }
            }
            println!("  Participants ({}):", participants.len());
            for p in &participants {
                let sid = p.get("session_id").and_then(|v| v.as_str()).unwrap_or("?");
                let tpl = p.get("template_name").and_then(|v| v.as_str()).unwrap_or("?");
                let pstatus = p.get("status").and_then(|v| v.as_str()).unwrap_or("pending");
                println!("    {sid}: {tpl} [{pstatus}]");
            }
        }
        Ok(Response::Error { message }) => {
            eprintln!("error: {message}");
            std::process::exit(1);
        }
        Ok(_) => {
            eprintln!("unexpected response");
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    }
}

pub async fn meeting_responses(meeting_id: String) {
    let req = Request::MeetingResponses { meeting_id };
    match client::send(&req).await {
        Ok(Response::Ok { data: ResponseData::MeetingResponseList { responses, count } }) => {
            if count == 0 {
                println!("No responses yet.");
                return;
            }
            println!("{count} response(s):");
            for r in &responses {
                let sid = r.get("session_id").and_then(|v| v.as_str()).unwrap_or("?");
                let tpl = r.get("template_name").and_then(|v| v.as_str()).unwrap_or("?");
                let status = r.get("status").and_then(|v| v.as_str()).unwrap_or("?");
                let response_text = r.get("response").and_then(|v| v.as_str()).unwrap_or("");
                let confidence = r.get("confidence").and_then(|v| v.as_f64());
                let conf_str = match confidence {
                    Some(c) => format!(" (confidence: {c:.2})"),
                    None => String::new(),
                };
                println!("  {sid} ({tpl}) [{status}]{conf_str}:");
                if !response_text.is_empty() {
                    // Indent response text
                    for line in response_text.lines() {
                        println!("    {line}");
                    }
                }
            }
        }
        Ok(Response::Error { message }) => {
            eprintln!("error: {message}");
            std::process::exit(1);
        }
        Ok(_) => {
            eprintln!("unexpected response");
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    }
}

pub async fn meeting_synthesize(meeting_id: String, synthesis: String) {
    let req = Request::MeetingSynthesize { meeting_id: meeting_id.clone(), synthesis };
    match client::send(&req).await {
        Ok(Response::Ok { data: ResponseData::MeetingSynthesized { meeting_id } }) => {
            println!("Synthesis stored for meeting: {}", &meeting_id[..13.min(meeting_id.len())]);
        }
        Ok(Response::Error { message }) => {
            eprintln!("error: {message}");
            std::process::exit(1);
        }
        Ok(_) => {
            eprintln!("unexpected response");
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    }
}

pub async fn meeting_decide(meeting_id: String, decision: String) {
    let req = Request::MeetingDecide { meeting_id: meeting_id.clone(), decision };
    match client::send(&req).await {
        Ok(Response::Ok { data: ResponseData::MeetingDecided { meeting_id, decision_memory_id } }) => {
            println!("Decision recorded for meeting: {}", &meeting_id[..13.min(meeting_id.len())]);
            println!("  Decision memory: {}", &decision_memory_id[..13.min(decision_memory_id.len())]);
        }
        Ok(Response::Error { message }) => {
            eprintln!("error: {message}");
            std::process::exit(1);
        }
        Ok(_) => {
            eprintln!("unexpected response");
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    }
}

pub async fn list_meetings(team_id: Option<String>, status: Option<String>) {
    let req = Request::ListMeetings { team_id, status, limit: None };
    match client::send(&req).await {
        Ok(Response::Ok { data: ResponseData::MeetingList { meetings, count } }) => {
            if count == 0 {
                println!("No meetings.");
                return;
            }
            println!("{count} meeting(s):");
            for m in &meetings {
                let mid = m.get("id").and_then(|v| v.as_str()).unwrap_or("?");
                let topic = m.get("topic").and_then(|v| v.as_str()).unwrap_or("?");
                let status = m.get("status").and_then(|v| v.as_str()).unwrap_or("?");
                let team = m.get("team_id").and_then(|v| v.as_str()).unwrap_or("?");
                println!("  {} [{}] team={}: {}", &mid[..13.min(mid.len())], status, team, topic);
            }
        }
        Ok(Response::Error { message }) => {
            eprintln!("error: {message}");
            std::process::exit(1);
        }
        Ok(_) => {
            eprintln!("unexpected response");
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    }
}

pub async fn meeting_transcript(meeting_id: String) {
    let req = Request::MeetingTranscript { meeting_id };
    match client::send(&req).await {
        Ok(Response::Ok { data: ResponseData::MeetingTranscriptData { transcript } }) => {
            let topic = transcript.get("topic").and_then(|v| v.as_str()).unwrap_or("?");
            let status = transcript.get("status").and_then(|v| v.as_str()).unwrap_or("?");
            let mid = transcript.get("id").and_then(|v| v.as_str()).unwrap_or("?");

            println!("=== Meeting Transcript ===");
            println!("ID:     {}", &mid[..13.min(mid.len())]);
            println!("Topic:  {topic}");
            println!("Status: {status}");

            if let Some(ctx) = transcript.get("context").and_then(|v| v.as_str()) {
                if !ctx.is_empty() {
                    println!("\n--- Context ---");
                    println!("{ctx}");
                }
            }

            if let Some(participants) = transcript.get("participants").and_then(|v| v.as_array()) {
                println!("\n--- Responses ---");
                for p in participants {
                    let sid = p.get("session_id").and_then(|v| v.as_str()).unwrap_or("?");
                    let tpl = p.get("template_name").and_then(|v| v.as_str()).unwrap_or("?");
                    let pstatus = p.get("status").and_then(|v| v.as_str()).unwrap_or("pending");
                    let response = p.get("response").and_then(|v| v.as_str()).unwrap_or("");
                    println!("\n[{tpl}] ({sid}) [{pstatus}]:");
                    if !response.is_empty() {
                        for line in response.lines() {
                            println!("  {line}");
                        }
                    } else {
                        println!("  (no response)");
                    }
                }
            }

            if let Some(synthesis) = transcript.get("synthesis").and_then(|v| v.as_str()) {
                if !synthesis.is_empty() {
                    println!("\n--- Synthesis ---");
                    println!("{synthesis}");
                }
            }

            if let Some(decision) = transcript.get("decision").and_then(|v| v.as_str()) {
                if !decision.is_empty() {
                    println!("\n--- Decision ---");
                    println!("{decision}");
                }
            }
        }
        Ok(Response::Error { message }) => {
            eprintln!("error: {message}");
            std::process::exit(1);
        }
        Ok(_) => {
            eprintln!("unexpected response");
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    }
}

// ── Notifications ──

pub async fn list_notifications(status: Option<String>, category: Option<String>, limit: usize) {
    let req = Request::ListNotifications {
        status,
        category,
        limit: Some(limit),
    };
    match client::send(&req).await {
        Ok(Response::Ok { data: ResponseData::NotificationList { notifications, count } }) => {
            if count == 0 {
                println!("No notifications.");
                return;
            }
            println!("{count} notification(s):");
            for n in &notifications {
                let id = n["id"].as_str().unwrap_or("?");
                let short_id = &id[..13.min(id.len())];
                let priority = n["priority"].as_str().unwrap_or("?");
                let cat = n["category"].as_str().unwrap_or("?");
                let title = n["title"].as_str().unwrap_or("?");
                let status_val = n["status"].as_str().unwrap_or("?");
                println!("  [{priority}] {short_id} ({cat}/{status_val}) {title}");
            }
        }
        Ok(Response::Error { message }) => {
            eprintln!("error: {message}");
            std::process::exit(1);
        }
        Ok(_) => {
            eprintln!("unexpected response");
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    }
}

pub async fn ack_notification(id: String) {
    let req = Request::AckNotification { id: id.clone() };
    match client::send(&req).await {
        Ok(Response::Ok { data: ResponseData::NotificationAcked { id } }) => {
            println!("Acknowledged: {}", &id[..13.min(id.len())]);
        }
        Ok(Response::Error { message }) => {
            eprintln!("error: {message}");
            std::process::exit(1);
        }
        Ok(_) => {
            eprintln!("unexpected response");
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    }
}

pub async fn dismiss_notification(id: String) {
    let req = Request::DismissNotification { id: id.clone() };
    match client::send(&req).await {
        Ok(Response::Ok { data: ResponseData::NotificationDismissed { id } }) => {
            println!("Dismissed: {}", &id[..13.min(id.len())]);
        }
        Ok(Response::Error { message }) => {
            eprintln!("error: {message}");
            std::process::exit(1);
        }
        Ok(_) => {
            eprintln!("unexpected response");
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    }
}

pub async fn act_on_notification(id: String, approved: bool) {
    let req = Request::ActOnNotification { id: id.clone(), approved };
    match client::send(&req).await {
        Ok(Response::Ok { data: ResponseData::NotificationActed { id, result } }) => {
            let action = if approved { "Approved" } else { "Rejected" };
            let short_id = &id[..13.min(id.len())];
            match result {
                Some(r) => println!("{action}: {short_id} (result: {r})"),
                None => println!("{action}: {short_id}"),
            }
        }
        Ok(Response::Error { message }) => {
            eprintln!("error: {message}");
            std::process::exit(1);
        }
        Ok(_) => {
            eprintln!("unexpected response");
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    }
}

// ── Organization Hierarchy ──

pub async fn team_tree(org: Option<String>) {
    let req = Request::TeamTree { organization_id: org };
    match client::send(&req).await {
        Ok(Response::Ok { data: ResponseData::TeamTreeData { tree } }) => {
            if tree.is_empty() {
                println!("No teams.");
                return;
            }
            fn print_tree(nodes: &[serde_json::Value], indent: usize) {
                for n in nodes {
                    let prefix = "  ".repeat(indent);
                    let name = n["name"].as_str().unwrap_or("?");
                    let purpose = n["purpose"].as_str().unwrap_or("");
                    let marker = "\u{251c}\u{2500}";
                    println!(
                        "{}{} {} {}",
                        prefix,
                        marker,
                        name,
                        if purpose.is_empty() {
                            String::new()
                        } else {
                            format!("\u{2014} {}", purpose)
                        }
                    );
                    if let Some(children) = n["children"].as_array() {
                        print_tree(children, indent + 1);
                    }
                }
            }
            print_tree(&tree, 0);
        }
        Ok(Response::Error { message }) => {
            eprintln!("error: {message}");
            std::process::exit(1);
        }
        Ok(_) => {
            eprintln!("unexpected response");
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    }
}

pub async fn team_send(
    team: String,
    kind: String,
    topic: String,
    text: String,
    from: Option<String>,
    recursive: bool,
) {
    let parts = vec![forge_core::protocol::MessagePart {
        kind: "text".to_string(),
        text: Some(text),
        path: None,
        data: None,
        memory_id: None,
    }];
    let req = Request::TeamSend {
        team_name: team,
        kind,
        topic,
        parts,
        from_session: from,
        recursive,
    };
    match client::send(&req).await {
        Ok(Response::Ok { data: ResponseData::TeamSent { messages_sent } }) => {
            println!("Sent to {} session(s)", messages_sent);
        }
        Ok(Response::Error { message }) => {
            eprintln!("error: {message}");
            std::process::exit(1);
        }
        Ok(_) => {
            eprintln!("unexpected response");
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    }
}
