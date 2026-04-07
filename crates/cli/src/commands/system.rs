use crate::client;
use forge_core::protocol::{Request, Response, ResponseData};
#[allow(unused_imports)]
use forge_core::types::MemoryType;

/// Print daemon health diagnostics (doctor).
pub async fn doctor() {
    match client::send(&Request::Doctor).await {
        Ok(Response::Ok {
            data:
                ResponseData::Doctor {
                    daemon_up,
                    memory_count,
                    embedding_count,
                    file_count,
                    symbol_count,
                    edge_count,
                    workers,
                    uptime_secs,
                    ..
                },
        }) => {
            println!("Forge Doctor");
            println!(
                "  Daemon:    {} (uptime: {}s)",
                if daemon_up { "UP" } else { "DOWN" },
                uptime_secs
            );
            println!("  Memories:  {}", memory_count);
            println!("  Embeddings:{}", embedding_count);
            println!("  Files:     {}", file_count);
            println!("  Symbols:   {}", symbol_count);
            println!("  Edges:     {}", edge_count);
            println!("  Workers:   {}", workers.join(", "));
        }
        Ok(Response::Error { message }) => eprintln!("error: {}", message),
        Ok(_) => eprintln!("unexpected response"),
        Err(e) => eprintln!("error: {}", e),
    }
}

/// Print system health grouped by project.
pub async fn health_by_project() {
    let request = Request::HealthByProject;

    match client::send(&request).await {
        Ok(Response::Ok {
            data: ResponseData::HealthByProject { projects },
        }) => {
            println!("Health by Project:");
            if projects.is_empty() {
                println!("  (no memories stored)");
            } else {
                let mut sorted: Vec<_> = projects.iter().collect();
                sorted.sort_by_key(|(k, _)| (*k).clone());
                for (project, data) in sorted {
                    let total = data.decisions + data.lessons + data.patterns + data.preferences;
                    println!("  {}:", project);
                    println!("    decisions: {}, lessons: {}, patterns: {}, preferences: {}, total: {}",
                        data.decisions, data.lessons, data.patterns, data.preferences, total);
                }
            }
        }
        Ok(Response::Error { message }) => {
            eprintln!("error: {message}");
            std::process::exit(1);
        }
        Ok(other) => {
            eprintln!("unexpected response: {other:?}");
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    }
}

/// Print system health (memory counts by type + edges).
pub async fn health() {
    let request = Request::Health;

    match client::send(&request).await {
        Ok(Response::Ok {
            data:
                ResponseData::Health {
                    decisions,
                    lessons,
                    patterns,
                    preferences,
                    edges,
                },
        }) => {
            let total = decisions + lessons + patterns + preferences;
            println!("Health:");
            println!("  decisions:   {decisions}");
            println!("  lessons:     {lessons}");
            println!("  patterns:    {patterns}");
            println!("  preferences: {preferences}");
            println!("  total:       {total}");
            println!("  edges:       {edges}");
        }
        Ok(Response::Error { message }) => {
            eprintln!("error: {message}");
            std::process::exit(1);
        }
        Ok(other) => {
            eprintln!("unexpected response: {other:?}");
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    }
}

#[derive(serde::Deserialize)]
struct V1CacheEntry {
    #[serde(rename = "type")]
    entry_type: Option<String>,
    title: Option<String>,
    content: Option<String>,
    confidence: Option<f64>,
    status: Option<String>,
}

#[derive(serde::Deserialize)]
struct V1Cache {
    entries: Vec<V1CacheEntry>,
}

/// Import v1 cache.json by reading the file and sending Remember requests to the daemon.
pub async fn migrate(state_dir: String) {
    let cache_path = std::path::Path::new(&state_dir).join("cache.json");
    let cache_str = cache_path.to_string_lossy().to_string();

    let content = match std::fs::read_to_string(&cache_path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: cannot read {}: {}", cache_str, e);
            std::process::exit(1);
        }
    };

    let cache: V1Cache = match serde_json::from_str(&content) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: cannot parse {}: {}", cache_str, e);
            std::process::exit(1);
        }
    };

    let mut imported = 0usize;
    let mut skipped = 0usize;

    for entry in &cache.entries {
        let title = match &entry.title {
            Some(t) if !t.trim().is_empty() => t.clone(),
            _ => {
                skipped += 1;
                continue;
            }
        };
        let memory_type = match entry.entry_type.as_deref() {
            Some("decision") => MemoryType::Decision,
            Some("pattern") => MemoryType::Pattern,
            Some("lesson") => MemoryType::Lesson,
            Some("preference") => MemoryType::Preference,
            _ => {
                skipped += 1;
                continue;
            }
        };
        if entry.status.as_deref() != Some("active") {
            skipped += 1;
            continue;
        }

        let req = Request::Remember {
            memory_type,
            title,
            content: entry.content.clone().unwrap_or_default(),
            confidence: entry.confidence,
            tags: None,
            project: None,
            metadata: None,
        };

        match client::send(&req).await {
            Ok(Response::Ok { .. }) => imported += 1,
            Ok(Response::Error { message }) => {
                eprintln!("  skip: {}", message);
                skipped += 1;
            }
            Err(e) => {
                eprintln!("  skip: {}", e);
                skipped += 1;
            }
        }
    }

    println!("Migration complete: {} imported, {} skipped", imported, skipped);
}

/// Export all data as JSON.
pub async fn export(format: &str) {
    let req = Request::Export { format: Some(format.to_string()), since: None };
    match client::send(&req).await {
        Ok(Response::Ok { data: ResponseData::Export { memories, files, symbols, edges } }) => {
            let output = serde_json::json!({
                "memories": memories,
                "files": files,
                "symbols": symbols,
                "edges": edges,
                "exported_at": chrono_now(),
                "count": {
                    "memories": memories.len(),
                    "files": files.len(),
                    "symbols": symbols.len(),
                    "edges": edges.len(),
                }
            });
            println!("{}", serde_json::to_string_pretty(&output).unwrap_or_default());
        }
        Ok(Response::Error { message }) => eprintln!("error: {}", message),
        Ok(_) => eprintln!("unexpected response"),
        Err(e) => eprintln!("error: {}", e),
    }
}

/// Import data from JSON (stdin or file).
pub async fn import(file: Option<String>) {
    let data = match file {
        Some(path) => match std::fs::read_to_string(&path) {
            Ok(content) => content,
            Err(e) => {
                eprintln!("error: cannot read {}: {}", path, e);
                std::process::exit(1);
            }
        },
        None => {
            use std::io::Read;
            let mut buf = String::new();
            if let Err(e) = std::io::stdin().read_to_string(&mut buf) {
                eprintln!("error: cannot read stdin: {}", e);
                std::process::exit(1);
            }
            buf
        }
    };

    let req = Request::Import { data };
    match client::send(&req).await {
        Ok(Response::Ok { data: ResponseData::Import { memories_imported, files_imported, symbols_imported, skipped } }) => {
            println!("Import complete:");
            println!("  memories: {}", memories_imported);
            println!("  files:    {}", files_imported);
            println!("  symbols:  {}", symbols_imported);
            println!("  skipped:  {}", skipped);
        }
        Ok(Response::Error { message }) => eprintln!("error: {}", message),
        Ok(_) => eprintln!("unexpected response"),
        Err(e) => eprintln!("error: {}", e),
    }
}

/// Ingest Claude Code's MEMORY.md files into Forge.
pub async fn ingest_claude() {
    match client::send(&Request::IngestClaude).await {
        Ok(Response::Ok {
            data: ResponseData::IngestClaude { imported, skipped },
        }) => {
            println!("Claude memory ingestion complete:");
            println!("  imported: {}", imported);
            println!("  skipped:  {}", skipped);
        }
        Ok(Response::Error { message }) => eprintln!("error: {}", message),
        Ok(_) => eprintln!("unexpected response"),
        Err(e) => eprintln!("error: {}", e),
    }
}

/// Backfill: re-process a transcript file from scratch.
pub async fn backfill(path: String) {
    // Verify file exists before sending to daemon
    if !std::path::Path::new(&path).exists() {
        eprintln!("error: file not found: {}", path);
        std::process::exit(1);
    }

    let req = Request::Backfill { path };
    match client::send(&req).await {
        Ok(Response::Ok {
            data: ResponseData::Backfill { chunks_processed, memories_stored },
        }) => {
            println!("Backfill complete:");
            println!("  chunks processed: {}", chunks_processed);
            println!("  memories stored:  {}", memories_stored);
        }
        Ok(Response::Error { message }) => eprintln!("error: {}", message),
        Ok(_) => eprintln!("unexpected response"),
        Err(e) => eprintln!("error: {}", e),
    }
}

/// Pre-execution guardrail check on a file.
pub async fn check(file: String, action: String) {
    match client::send(&Request::GuardrailsCheck { file: file.clone(), action }).await {
        Ok(Response::Ok {
            data: ResponseData::GuardrailsCheck {
                safe, warnings, decisions_affected, callers_count,
                calling_files, relevant_lessons, dangerous_patterns, applicable_skills,
            },
        }) => {
            if safe {
                println!("Safe to proceed — no decisions linked to {file}");
            } else {
                println!("{} decision(s) linked to {file}:", decisions_affected.len());
                for w in &warnings {
                    println!("  {w}");
                }
            }
            if callers_count > 0 {
                println!("  Blast radius: {callers_count} file(s) call symbols in this file");
                for cf in &calling_files {
                    println!("    - {cf}");
                }
            }
            for lesson in &relevant_lessons {
                println!("  Lesson: {lesson}");
            }
            for pattern in &dangerous_patterns {
                println!("  Dangerous: {pattern}");
            }
            for skill in &applicable_skills {
                println!("  {skill}");
            }
        }
        Ok(Response::Error { message }) => {
            eprintln!("error: {message}");
            std::process::exit(1);
        }
        Ok(_) => eprintln!("unexpected response"),
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    }
}

/// Post-edit check — surface callers, lessons, and warnings after a file edit.
pub async fn post_edit_check(file: String) {
    match client::send(&Request::PostEditCheck { file: file.clone() }).await {
        Ok(Response::Ok {
            data: ResponseData::PostEditChecked {
                file: _,
                callers_count,
                calling_files,
                relevant_lessons,
                dangerous_patterns,
                applicable_skills,
                decisions_to_review,
                cached_diagnostics,
            },
        }) => {
            for diag in &cached_diagnostics {
                println!("{diag}");
            }
            if callers_count > 0 {
                println!("callers: {} file(s) call symbols in {file}", callers_count);
                for cf in &calling_files {
                    println!("  - {cf}");
                }
            }
            for lesson in &relevant_lessons {
                println!("Lesson: {lesson}");
            }
            for pattern in &dangerous_patterns {
                println!("Dangerous: {pattern}");
            }
            for skill in &applicable_skills {
                println!("Skill: {skill}");
            }
            for decision in &decisions_to_review {
                println!("Decision to review: {decision}");
            }
        }
        Ok(Response::Error { message }) => {
            eprintln!("error: {message}");
            std::process::exit(1);
        }
        Ok(_) => eprintln!("unexpected response"),
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    }
}

/// Blast radius analysis on a file.
pub async fn blast_radius(file: String) {
    match client::send(&Request::BlastRadius { file: file.clone() }).await {
        Ok(Response::Ok {
            data: ResponseData::BlastRadius {
                decisions, callers, importers, files_affected,
                cluster_name, cluster_files, calling_files, warnings,
            },
        }) => {
            for w in &warnings {
                println!("  ⚠ {w}");
            }
            println!("Blast radius for {file}:");
            println!("  Decisions:         {}", decisions.len());
            for d in &decisions {
                println!("    - {} (confidence: {:.2}) [{}]", d.title, d.confidence, d.id);
            }
            println!("  Callers:           {callers}");
            if !calling_files.is_empty() {
                for cf in &calling_files {
                    println!("    - {cf}");
                }
            }
            println!("  Importers:         {}", importers.len());
            for imp in &importers {
                println!("    - {imp}");
            }
            println!("  Co-affected files: {}", files_affected.len());
            for f in &files_affected {
                println!("    - {f}");
            }
            if let Some(ref cluster) = cluster_name {
                println!("  Cluster:           {cluster}");
                println!("  Cluster files:     {}", cluster_files.len());
                for cf in &cluster_files {
                    println!("    - {cf}");
                }
            }
        }
        Ok(Response::Error { message }) => {
            eprintln!("error: {message}");
            std::process::exit(1);
        }
        Ok(_) => eprintln!("unexpected response"),
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    }
}

/// Pre-bash check -- warn about destructive commands, surface relevant skills/lessons.
pub async fn pre_bash_check(command: String) {
    match client::send(&Request::PreBashCheck { command: command.clone() }).await {
        Ok(Response::Ok {
            data: ResponseData::PreBashChecked {
                safe, warnings, relevant_skills,
            },
        }) => {
            for w in &warnings {
                println!("{w}");
            }
            for s in &relevant_skills {
                println!("Skill: {s}");
            }
            if safe && warnings.is_empty() && relevant_skills.is_empty() {
                // Silent on safe -- context budget rule
            }
        }
        Ok(Response::Error { message }) => {
            eprintln!("error: {message}");
            std::process::exit(1);
        }
        Ok(_) => eprintln!("unexpected response"),
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    }
}

/// Post-bash check -- surface lessons and skills after command failure.
pub async fn post_bash_check(command: String, exit_code: i32) {
    match client::send(&Request::PostBashCheck { command: command.clone(), exit_code }).await {
        Ok(Response::Ok {
            data: ResponseData::PostBashChecked { suggestions },
        }) => {
            for s in &suggestions {
                println!("{s}");
            }
        }
        Ok(Response::Error { message }) => {
            eprintln!("error: {message}");
            std::process::exit(1);
        }
        Ok(_) => eprintln!("unexpected response"),
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    }
}

/// List active (or all) agent sessions.
pub async fn sessions(active_only: bool) {
    match client::send(&Request::Sessions { active_only: Some(active_only) }).await {
        Ok(Response::Ok {
            data: ResponseData::Sessions { sessions, count },
        }) => {
            if sessions.is_empty() {
                println!("No {} sessions.", if active_only { "active" } else { "" });
            } else {
                println!("{count} session(s):");
                for s in &sessions {
                    let project = s.project.as_deref().unwrap_or("(none)");
                    let status_str = if s.status == "active" { "ACTIVE" } else { "ended" };
                    println!("  [{}] {} — {} (project: {}, since: {})", status_str, s.id, s.agent, project, s.started_at);
                }
            }
        }
        Ok(Response::Error { message }) => eprintln!("error: {message}"),
        Ok(_) => eprintln!("unexpected response"),
        Err(e) => eprintln!("error: {e}"),
    }
}

/// Show available language servers for the current project.
pub async fn lsp_status() {
    match client::send(&Request::LspStatus).await {
        Ok(Response::Ok {
            data: ResponseData::LspStatus { servers },
        }) => {
            if servers.is_empty() {
                println!("No language servers detected for the current project.");
                println!("Tip: Set FORGE_PROJECT to your project directory, or ensure language server binaries are on PATH.");
            } else {
                println!("Language Servers:");
                for s in &servers {
                    let status = if s.available { "available" } else { "not found" };
                    println!("  {} — {} ({})", s.language, s.command, status);
                }
            }
        }
        Ok(Response::Error { message }) => {
            eprintln!("error: {message}");
            std::process::exit(1);
        }
        Ok(_) => eprintln!("unexpected response"),
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    }
}

/// Register an active agent session.
pub async fn register_session(id: String, agent: String, project: Option<String>, cwd: Option<String>, role: Option<String>) {
    // TODO: pass `role` to Request::RegisterSession once the protocol adds the field
    let _ = &role;
    match client::send(&Request::RegisterSession { id: id.clone(), agent, project, cwd, capabilities: None, current_task: None }).await {
        Ok(Response::Ok { data: ResponseData::SessionRegistered { .. } }) => {
            println!("Session registered: {id}");
        }
        Ok(Response::Error { message }) => eprintln!("error: {message}"),
        Ok(_) => eprintln!("unexpected response"),
        Err(e) => eprintln!("error: {e}"),
    }
}

/// End an active agent session.
pub async fn end_session(id: String) {
    match client::send(&Request::EndSession { id: id.clone() }).await {
        Ok(Response::Ok { data: ResponseData::SessionEnded { found, .. } }) => {
            if found {
                println!("Session ended: {id}");
            } else {
                println!("Session not found or already ended: {id}");
            }
        }
        Ok(Response::Error { message }) => eprintln!("error: {message}"),
        Ok(_) => eprintln!("unexpected response"),
        Err(e) => eprintln!("error: {e}"),
    }
}

pub async fn cleanup_sessions(prefix: Option<String>, older_than_secs: Option<u64>, prune_ended: bool) {
    match client::send(&Request::CleanupSessions { prefix: prefix.clone(), older_than_secs, prune_ended }).await {
        Ok(Response::Ok { data: ResponseData::SessionsCleaned { ended } }) => {
            println!("Cleaned up {ended} session(s){}", match &prefix {
                Some(p) => format!(" (prefix: {p})"),
                None => " (all)".to_string(),
            });
        }
        Ok(Response::Error { message }) => eprintln!("error: {message}"),
        Ok(_) => eprintln!("unexpected response"),
        Err(e) => eprintln!("error: {e}"),
    }
}

// ── A2A Inter-Session Messaging ──

pub async fn send_message(to: String, kind: String, topic: String, text: String, project: Option<String>, timeout: Option<u64>) {
    let parts = vec![forge_core::protocol::MessagePart {
        kind: "text".to_string(),
        text: Some(text),
        path: None,
        data: None,
        memory_id: None,
    }];
    let req = Request::SessionSend { to, kind, topic, parts, project, timeout_secs: timeout, meeting_id: None };
    match client::send(&req).await {
        Ok(Response::Ok { data: ResponseData::MessageSent { id, status } }) => {
            println!("Message sent: {id} (status: {status})");
        }
        Ok(Response::Error { message }) => eprintln!("error: {message}"),
        Ok(_) => eprintln!("unexpected response"),
        Err(e) => eprintln!("error: {e}"),
    }
}

pub async fn list_messages(session: String, status: Option<String>, limit: Option<usize>, full: bool) {
    let req = Request::SessionMessages { session_id: session, status, limit };
    match client::send(&req).await {
        Ok(Response::Ok { data: ResponseData::SessionMessageList { messages, count } }) => {
            if count == 0 {
                println!("No messages.");
                return;
            }
            println!("{count} message(s):\n");
            for m in &messages {
                let parts_text: String = m.parts.iter()
                    .filter_map(|p| p.text.as_deref())
                    .collect::<Vec<_>>()
                    .join(" ");
                if full {
                    println!("--- [{status}] {id} from {from} --- topic: {topic} ---",
                        status = m.status, id = &m.id,
                        from = m.from_session, topic = m.topic);
                    println!("{parts_text}");
                    println!();
                } else {
                    let preview = if parts_text.len() > 80 { &parts_text[..80] } else { &parts_text };
                    println!("  [{status}] {id} from {from} — {topic}: {preview}",
                        status = m.status, id = &m.id[..8.min(m.id.len())],
                        from = m.from_session, topic = m.topic, preview = preview);
                }
            }
        }
        Ok(Response::Error { message }) => eprintln!("error: {message}"),
        Ok(_) => eprintln!("unexpected response"),
        Err(e) => eprintln!("error: {e}"),
    }
}

/// Read a single FISP message by ID.
pub async fn message_read(id: String) {
    // No single-message-by-ID endpoint exists, so fetch a batch and filter client-side.
    let req = Request::SessionMessages { session_id: String::new(), status: None, limit: Some(100) };
    match client::send(&req).await {
        Ok(Response::Ok { data: ResponseData::SessionMessageList { messages, .. } }) => {
            match messages.iter().find(|m| m.id == id) {
                Some(m) => {
                    println!("Message: {}", m.id);
                    println!("From:    {}", m.from_session);
                    println!("To:      {}", m.to_session);
                    println!("Kind:    {}", m.kind);
                    println!("Topic:   {}", m.topic);
                    println!("Status:  {}", m.status);
                    if let Some(ref reply) = m.in_reply_to {
                        println!("Reply-to:{}", reply);
                    }
                    if let Some(ref proj) = m.project {
                        println!("Project: {}", proj);
                    }
                    println!("Created: {}", m.created_at);
                    if let Some(ref delivered) = m.delivered_at {
                        println!("Delivered:{}", delivered);
                    }
                    println!("---");
                    let full_text: String = m.parts.iter()
                        .filter_map(|p| p.text.as_deref())
                        .collect::<Vec<_>>()
                        .join("\n");
                    println!("{full_text}");
                }
                None => {
                    eprintln!("message not found: {id}");
                    std::process::exit(1);
                }
            }
        }
        Ok(Response::Error { message }) => eprintln!("error: {message}"),
        Ok(_) => eprintln!("unexpected response"),
        Err(e) => eprintln!("error: {e}"),
    }
}

pub async fn ack_messages(ids: Vec<String>) {
    if ids.is_empty() {
        eprintln!("error: no message IDs provided");
        return;
    }
    let req = Request::SessionAck { message_ids: ids, session_id: None };
    match client::send(&req).await {
        Ok(Response::Ok { data: ResponseData::MessagesAcked { count } }) => {
            println!("Acknowledged {count} message(s).");
        }
        Ok(Response::Error { message }) => eprintln!("error: {message}"),
        Ok(_) => eprintln!("unexpected response"),
        Err(e) => eprintln!("error: {e}"),
    }
}

// ── A2A Permission Management ──

pub async fn grant_permission(from: String, to: String, from_project: Option<String>, to_project: Option<String>) {
    let req = Request::GrantPermission { from_agent: from.clone(), to_agent: to.clone(), from_project: from_project.clone(), to_project: to_project.clone() };
    match client::send(&req).await {
        Ok(Response::Ok { data: ResponseData::PermissionGranted { id } }) => {
            println!("Permission granted: {id} ({from} → {to}{})", match (&from_project, &to_project) {
                (Some(fp), Some(tp)) => format!(", {fp} → {tp}"),
                _ => String::new(),
            });
        }
        Ok(Response::Error { message }) => eprintln!("error: {message}"),
        Ok(_) => eprintln!("unexpected response"),
        Err(e) => eprintln!("error: {e}"),
    }
}

pub async fn revoke_permission(id: String) {
    let req = Request::RevokePermission { id: id.clone() };
    match client::send(&req).await {
        Ok(Response::Ok { data: ResponseData::PermissionRevoked { id: _, found } }) => {
            if found { println!("Permission revoked: {id}"); }
            else { println!("Permission not found: {id}"); }
        }
        Ok(Response::Error { message }) => eprintln!("error: {message}"),
        Ok(_) => eprintln!("unexpected response"),
        Err(e) => eprintln!("error: {e}"),
    }
}

pub async fn list_permissions() {
    let req = Request::ListPermissions;
    match client::send(&req).await {
        Ok(Response::Ok { data: ResponseData::PermissionList { permissions, count } }) => {
            if count == 0 {
                println!("No A2A permissions configured (trust mode: open by default).");
                return;
            }
            println!("{count} permission(s):\n");
            for p in &permissions {
                let proj = match (&p.from_project, &p.to_project) {
                    (Some(fp), Some(tp)) => format!(" ({fp} → {tp})"),
                    (Some(fp), None) => format!(" (from {fp})"),
                    (None, Some(tp)) => format!(" (to {tp})"),
                    _ => String::new(),
                };
                println!("  [{}] {} → {}{} ({})",
                    if p.allowed { "ALLOW" } else { "DENY" },
                    p.from_agent, p.to_agent, proj, &p.id[..8.min(p.id.len())]);
            }
        }
        Ok(Response::Error { message }) => eprintln!("error: {message}"),
        Ok(_) => eprintln!("unexpected response"),
        Err(e) => eprintln!("error: {e}"),
    }
}

// ── Knowledge Intelligence ──

pub async fn list_entities(project: Option<String>, limit: usize) {
    let req = Request::ListEntities { project, limit: Some(limit) };
    match client::send(&req).await {
        Ok(Response::Ok { data: ResponseData::EntityList { entities, count } }) => {
            if count == 0 {
                println!("No entities detected yet. Run forge-next consolidate to detect entities from memory titles.");
                return;
            }
            println!("{count} entity(ies):\n");
            for e in &entities {
                println!("  {} ({}) — {} mentions, first seen {}",
                    e.name, e.entity_type, e.mention_count,
                    &e.first_seen[..10.min(e.first_seen.len())]);
            }
        }
        Ok(Response::Error { message }) => eprintln!("error: {message}"),
        Ok(_) => eprintln!("unexpected response"),
        Err(e) => eprintln!("error: {e}"),
    }
}

pub async fn context_trace(agent: String, project: Option<String>) {
    let req = Request::CompileContextTrace { agent: Some(agent), project };
    match client::send(&req).await {
        Ok(Response::Ok { data: ResponseData::ContextTrace {
            considered, included, excluded, budget_total, budget_used, ..
        } }) => {
            println!("Context Compilation Trace");
            println!("Budget: {budget_used}/{budget_total} chars\n");
            println!("INCLUDED ({}):", included.len());
            for t in &included {
                println!("  [{}] {} (conf={:.2}, activation={:.2}) — {}",
                    t.memory_type, &t.title[..60.min(t.title.len())],
                    t.confidence, t.activation_level, t.reason);
            }
            if !excluded.is_empty() {
                println!("\nEXCLUDED ({}):", excluded.len());
                for t in &excluded {
                    println!("  [{}] {} — {}",
                        t.memory_type, &t.title[..60.min(t.title.len())], t.reason);
                }
            }
            println!("\nTotal considered: {}", considered.len());
        }
        Ok(Response::Error { message }) => eprintln!("error: {message}"),
        Ok(_) => eprintln!("unexpected response"),
        Err(e) => eprintln!("error: {e}"),
    }
}

/// Run proactive checks on a file or show all active diagnostics.
pub async fn verify(file: Option<String>) {
    let req = Request::Verify { file: file.clone() };
    match client::send(&req).await {
        Ok(Response::Ok {
            data: ResponseData::VerifyResult {
                files_checked, errors, warnings, diagnostics,
            },
        }) => {
            let target = file.as_deref().unwrap_or("all files");
            println!("Verify: {target}");
            println!("  Files checked: {files_checked}");
            println!("  Errors:        {errors}");
            println!("  Warnings:      {warnings}");
            if !diagnostics.is_empty() {
                println!();
                for d in &diagnostics {
                    let line_str = d.line.map(|l| format!(":{l}")).unwrap_or_default();
                    println!("  [{severity}] {file_path}{line_str}: {message} ({source})",
                        severity = d.severity,
                        file_path = d.file_path,
                        message = d.message,
                        source = d.source,
                    );
                }
            }
            if errors > 0 {
                std::process::exit(1);
            }
        }
        Ok(Response::Error { message }) => {
            eprintln!("error: {message}");
            std::process::exit(1);
        }
        Ok(_) => eprintln!("unexpected response"),
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    }
}

/// Show cached diagnostics for a file.
pub async fn diagnostics(file: String) {
    let req = Request::GetDiagnostics { file: file.clone() };
    match client::send(&req).await {
        Ok(Response::Ok {
            data: ResponseData::DiagnosticList { diagnostics, count },
        }) => {
            if diagnostics.is_empty() {
                println!("No diagnostics for {file}");
            } else {
                println!("{count} diagnostic(s) for {file}:");
                for d in &diagnostics {
                    let line_str = d.line.map(|l| format!(":{l}")).unwrap_or_default();
                    println!("  [{severity}] {file_path}{line_str}: {message} ({source})",
                        severity = d.severity,
                        file_path = d.file_path,
                        message = d.message,
                        source = d.source,
                    );
                }
            }
        }
        Ok(Response::Error { message }) => {
            eprintln!("error: {message}");
            std::process::exit(1);
        }
        Ok(_) => eprintln!("unexpected response"),
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    }
}

/// Bootstrap: scan and process all existing transcript files.
pub async fn bootstrap(project: Option<String>) {
    let req = Request::Bootstrap { project };
    match client::send(&req).await {
        Ok(Response::Ok {
            data: ResponseData::BootstrapComplete {
                files_processed,
                files_skipped,
                memories_extracted,
                errors,
            },
        }) => {
            println!("Bootstrap complete:");
            println!("  Files processed:  {}", files_processed);
            println!("  Files skipped:    {}", files_skipped);
            println!("  Memories created: {}", memories_extracted);
            if errors > 0 {
                println!("  Errors:           {}", errors);
            }
        }
        Ok(Response::Error { message }) => {
            eprintln!("error: {}", message);
            std::process::exit(1);
        }
        Ok(_) => eprintln!("unexpected response"),
        Err(e) => {
            eprintln!("error: {}", e);
            std::process::exit(1);
        }
    }
}

/// Backfill project field on memories with NULL/empty project.
pub async fn backfill_project() {
    let req = Request::BackfillProject;
    match client::send(&req).await {
        Ok(Response::Ok {
            data: ResponseData::BackfillProjectResult { updated, skipped },
        }) => {
            if updated == 0 && skipped == 0 {
                println!("All memories already have a project set.");
            } else if updated == 0 {
                println!("No memories could be backfilled. {} still have no project.", skipped);
            } else {
                println!("Backfilled project on {} memories.", updated);
                if skipped > 0 {
                    println!("  {} memories still have no project (no session/transcript match).", skipped);
                }
            }
        }
        Ok(Response::Error { message }) => {
            eprintln!("error: {}", message);
            std::process::exit(1);
        }
        Ok(_) => eprintln!("unexpected response"),
        Err(e) => {
            eprintln!("error: {}", e);
            std::process::exit(1);
        }
    }
}

/// Force-run all consolidation phases (dedup, decay, promotion, etc.)
pub async fn consolidate() {
    match client::send(&Request::ForceConsolidate).await {
        Ok(Response::Ok {
            data:
                ResponseData::ConsolidationComplete {
                    exact_dedup,
                    semantic_dedup,
                    linked,
                    faded,
                    promoted,
                    reconsolidated,
                    embedding_merged,
                    strengthened,
                    contradictions,
                    ..
                },
        }) => {
            println!("Consolidation complete:");
            println!("  Exact dedup:     {}", exact_dedup);
            println!("  Semantic dedup:  {}", semantic_dedup);
            println!("  Linked:          {}", linked);
            println!("  Faded:           {}", faded);
            println!("  Promoted:        {}", promoted);
            println!("  Reconsolidated:  {}", reconsolidated);
            println!("  Embedding merge: {}", embedding_merged);
            println!("  Strengthened:    {}", strengthened);
            println!("  Contradictions:  {}", contradictions);
        }
        Ok(Response::Error { message }) => {
            eprintln!("error: {}", message);
            std::process::exit(1);
        }
        Ok(_) => eprintln!("unexpected response"),
        Err(e) => {
            eprintln!("error: {}", e);
            std::process::exit(1);
        }
    }
}

/// Trigger extraction on pending transcripts.
pub async fn extract(force: bool) {
    if !force {
        println!("Hint: use --force to trigger extraction immediately.");
        return;
    }
    match client::send(&Request::ForceExtract).await {
        Ok(Response::Ok {
            data: ResponseData::ExtractionTriggered { files_queued },
        }) => {
            println!("Extraction triggered: {} transcript file(s) need processing", files_queued);
        }
        Ok(Response::Error { message }) => {
            eprintln!("error: {message}");
            std::process::exit(1);
        }
        Ok(_) => eprintln!("unexpected response"),
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    }
}

/// Display current daemon configuration.
pub async fn config_show() {
    match client::send(&Request::GetConfig).await {
        Ok(Response::Ok {
            data:
                ResponseData::ConfigData {
                    backend,
                    ollama_model,
                    ollama_endpoint,
                    claude_cli_model,
                    claude_api_model,
                    claude_api_key_set,
                    openai_model,
                    openai_endpoint,
                    openai_key_set,
                    gemini_model,
                    gemini_key_set,
                    embedding_model,
                },
        }) => {
            println!("Forge Configuration:");
            println!("  Active backend:              {backend}");
            println!();
            println!("  Ollama (local):");
            println!("    model:     {ollama_model}");
            println!("    endpoint:  {ollama_endpoint}");
            println!();
            println!("  Claude CLI:");
            println!("    model:     {claude_cli_model}");
            println!();
            println!("  Claude API (Anthropic):");
            println!("    model:     {claude_api_model}");
            println!("    API key:   {}", if claude_api_key_set { "****set" } else { "not set" });
            println!();
            println!("  OpenAI:");
            println!("    model:     {openai_model}");
            println!("    endpoint:  {openai_endpoint}");
            println!("    API key:   {}", if openai_key_set { "****set" } else { "not set" });
            println!();
            println!("  Gemini (Google):");
            println!("    model:     {gemini_model}");
            println!("    API key:   {}", if gemini_key_set { "****set" } else { "not set" });
            println!();
            println!("  Embedding:   {embedding_model}");
        }
        Ok(Response::Error { message }) => {
            eprintln!("error: {message}");
            std::process::exit(1);
        }
        Ok(_) => eprintln!("unexpected response"),
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    }
}

/// Update a config value.
pub async fn config_set(key: String, value: String) {
    let req = Request::SetConfig {
        key: key.clone(),
        value: value.clone(),
    };
    match client::send(&req).await {
        Ok(Response::Ok {
            data: ResponseData::ConfigUpdated { key, value },
        }) => {
            println!("Config updated: {key} = {value}");
        }
        Ok(Response::Error { message }) => {
            eprintln!("error: {message}");
            std::process::exit(1);
        }
        Ok(_) => eprintln!("unexpected response"),
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    }
}

/// Set a scoped configuration value at a specific scope level.
pub async fn config_set_scoped(scope: String, scope_id: String, key: String, value: String, locked: bool, ceiling: Option<f64>) {
    let req = Request::SetScopedConfig {
        scope_type: scope.clone(),
        scope_id: scope_id.clone(),
        key: key.clone(),
        value: value.clone(),
        locked,
        ceiling,
    };
    match client::send(&req).await {
        Ok(Response::Ok {
            data: ResponseData::ScopedConfigSet { scope_type, scope_id, key },
        }) => {
            println!("Scoped config set: {key} at {scope_type}/{scope_id}");
            if locked {
                println!("  Locked: yes (lower scopes cannot override)");
            }
            if let Some(c) = ceiling {
                println!("  Ceiling: {c}");
            }
        }
        Ok(Response::Error { message }) => {
            eprintln!("error: {message}");
            std::process::exit(1);
        }
        Ok(_) => eprintln!("unexpected response"),
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    }
}

/// Get the effective (resolved) config for a session context.
pub async fn config_get_effective(
    session: Option<String>,
    agent: Option<String>,
    reality: Option<String>,
    user: Option<String>,
    team: Option<String>,
    organization: Option<String>,
) {
    let req = Request::GetEffectiveConfig {
        session_id: session,
        agent,
        reality_id: reality,
        user_id: user,
        team_id: team,
        organization_id: organization,
    };
    match client::send(&req).await {
        Ok(Response::Ok {
            data: ResponseData::EffectiveConfig { config },
        }) => {
            if config.is_empty() {
                println!("No scoped configuration values set.");
                return;
            }
            println!("Effective configuration ({} key(s)):\n", config.len());
            let mut keys: Vec<_> = config.keys().collect();
            keys.sort();
            for key in keys {
                let resolved = &config[key];
                println!("  {key} = {}", resolved.value);
                println!("    from: {}/{} (locked: {})",
                    resolved.source_scope_type, resolved.source_scope_id, resolved.locked);
            }
        }
        Ok(Response::Error { message }) => {
            eprintln!("error: {message}");
            std::process::exit(1);
        }
        Ok(_) => eprintln!("unexpected response"),
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    }
}

/// List all scoped config entries for a scope.
pub async fn config_list_scoped(scope: String, scope_id: String) {
    let req = Request::ListScopedConfig {
        scope_type: scope.clone(),
        scope_id: scope_id.clone(),
    };
    match client::send(&req).await {
        Ok(Response::Ok {
            data: ResponseData::ScopedConfigList { entries },
        }) => {
            if entries.is_empty() {
                println!("No config entries for {scope}/{scope_id}.");
                return;
            }
            println!("{} config entry(ies) for {scope}/{scope_id}:\n", entries.len());
            for e in &entries {
                let locked_str = if e.locked { " [LOCKED]" } else { "" };
                let ceiling_str = e.ceiling.map(|c| format!(" (ceiling: {c})")).unwrap_or_default();
                println!("  {} = {}{}{}", e.key, e.value, locked_str, ceiling_str);
            }
        }
        Ok(Response::Error { message }) => {
            eprintln!("error: {message}");
            std::process::exit(1);
        }
        Ok(_) => eprintln!("unexpected response"),
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    }
}

/// Delete a scoped config entry.
pub async fn config_delete_scoped(scope: String, scope_id: String, key: String) {
    let req = Request::DeleteScopedConfig {
        scope_type: scope.clone(),
        scope_id: scope_id.clone(),
        key: key.clone(),
    };
    match client::send(&req).await {
        Ok(Response::Ok {
            data: ResponseData::ScopedConfigDeleted { deleted },
        }) => {
            if deleted {
                println!("Deleted: {key} from {scope}/{scope_id}");
            } else {
                println!("Not found: {key} in {scope}/{scope_id}");
            }
        }
        Ok(Response::Error { message }) => {
            eprintln!("error: {message}");
            std::process::exit(1);
        }
        Ok(_) => eprintln!("unexpected response"),
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    }
}

/// Force-trigger the code indexer and show current index counts.
pub async fn force_index() {
    match client::send(&Request::ForceIndex).await {
        Ok(Response::Ok {
            data: ResponseData::IndexComplete { files_indexed, symbols_indexed },
        }) => {
            println!("Index status:");
            println!("  Files indexed:   {files_indexed}");
            println!("  Symbols indexed: {symbols_indexed}");
        }
        Ok(Response::Error { message }) => {
            eprintln!("error: {message}");
            std::process::exit(1);
        }
        Ok(_) => eprintln!("unexpected response"),
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    }
}

/// Show extraction metrics, token usage, and cost tracking.
pub async fn stats(hours: u64) {
    let req = Request::GetStats { hours: Some(hours) };
    match client::send(&req).await {
        Ok(Response::Ok {
            data: ResponseData::Stats {
                period_hours,
                extractions,
                extraction_errors,
                tokens_in,
                tokens_out,
                total_cost_usd,
                avg_latency_ms,
                memories_created,
            },
        }) => {
            println!("Forge Stats (last {}h):", period_hours);
            println!("  Extractions:      {} ({} errors)", extractions, extraction_errors);
            println!("  Tokens:           {} in / {} out", tokens_in, tokens_out);
            println!("  Cost:             ${:.4}", total_cost_usd);
            println!("  Avg latency:      {}ms", avg_latency_ms);
            println!("  Memories created: {}", memories_created);
        }
        Ok(Response::Error { message }) => {
            eprintln!("error: {message}");
            std::process::exit(1);
        }
        Ok(_) => eprintln!("unexpected response"),
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    }
}

/// Manage daemon as a system service (install/start/stop/status/uninstall).
pub async fn service(action: crate::ServiceAction) {
    use crate::ServiceAction;
    match action {
        ServiceAction::Install => service_install(),
        ServiceAction::Start => service_start(),
        ServiceAction::Stop => service_stop(),
        ServiceAction::Status => service_status(),
        ServiceAction::Uninstall => service_uninstall(),
    }
}

fn service_install() {
    let home = std::env::var("HOME").unwrap_or_default();

    #[cfg(target_os = "linux")]
    {
        let service_dir = format!("{}/.config/systemd/user", home);
        std::fs::create_dir_all(&service_dir).ok();

        let daemon_path = format!("{}/.local/bin/forge-daemon", home);
        let service_content = format!(
            r#"[Unit]
Description=Forge Daemon — Agent OS Memory System
After=network.target

[Service]
Type=simple
ExecStart={daemon_path}
Restart=on-failure
RestartSec=5
StandardOutput=journal
StandardError=journal
Environment=HOME={home}
Environment=FORGE_PROJECT_DIR={home}

[Install]
WantedBy=default.target
"#
        );

        let service_path = format!("{}/forge-daemon.service", service_dir);
        match std::fs::write(&service_path, service_content) {
            Ok(()) => {
                println!("Service file installed: {}", service_path);
                // Enable and start
                let _ = std::process::Command::new("systemctl")
                    .args(["--user", "daemon-reload"])
                    .status();
                let _ = std::process::Command::new("systemctl")
                    .args(["--user", "enable", "forge-daemon"])
                    .status();
                println!("Service enabled. Start with: forge-next service start");
                println!("View logs with: journalctl --user -u forge-daemon -f");
            }
            Err(e) => {
                eprintln!("error: failed to install service: {e}");
                std::process::exit(1);
            }
        }
    }

    #[cfg(target_os = "macos")]
    {
        let daemon_path = "/usr/local/bin/forge-daemon";
        let plist_content = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>com.forge.daemon</string>
    <key>ProgramArguments</key>
    <array>
        <string>{daemon_path}</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <dict>
        <key>SuccessfulExit</key>
        <false/>
    </dict>
    <key>StandardOutPath</key>
    <string>/tmp/forge-daemon.out.log</string>
    <key>StandardErrorPath</key>
    <string>/tmp/forge-daemon.err.log</string>
    <key>EnvironmentVariables</key>
    <dict>
        <key>HOME</key>
        <string>{home}</string>
    </dict>
    <key>ThrottleInterval</key>
    <integer>5</integer>
</dict>
</plist>"#
        );

        let plist_path = format!("{}/Library/LaunchAgents/com.forge.daemon.plist", home);
        match std::fs::write(&plist_path, plist_content) {
            Ok(()) => {
                println!("LaunchAgent installed: {}", plist_path);
                println!("Start with: forge-next service start");
                println!("View logs: tail -f /tmp/forge-daemon.err.log");
            }
            Err(e) => {
                eprintln!("error: failed to install service: {e}");
                std::process::exit(1);
            }
        }
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        eprintln!("Service install not supported on this platform.");
        eprintln!("Run the daemon manually: forge-daemon &");
    }
}

fn service_start() {
    #[cfg(target_os = "linux")]
    {
        let status = std::process::Command::new("systemctl")
            .args(["--user", "start", "forge-daemon"])
            .status();
        match status {
            Ok(s) if s.success() => println!("Forge daemon started."),
            Ok(s) => {
                eprintln!("systemctl start failed (exit {}). Is the service installed?", s);
                eprintln!("Run: forge-next service install");
            }
            Err(e) => eprintln!("error: {e}"),
        }
    }

    #[cfg(target_os = "macos")]
    {
        let home = std::env::var("HOME").unwrap_or_default();
        let plist = format!("{}/Library/LaunchAgents/com.forge.daemon.plist", home);
        let status = std::process::Command::new("launchctl")
            .args(["load", &plist])
            .status();
        match status {
            Ok(s) if s.success() => println!("Forge daemon started."),
            Ok(s) => eprintln!("launchctl load failed (exit {})", s),
            Err(e) => eprintln!("error: {e}"),
        }
    }
}

fn service_stop() {
    #[cfg(target_os = "linux")]
    {
        let status = std::process::Command::new("systemctl")
            .args(["--user", "stop", "forge-daemon"])
            .status();
        match status {
            Ok(s) if s.success() => println!("Forge daemon stopped."),
            _ => eprintln!("Failed to stop. Try: pkill -f forge-daemon"),
        }
    }

    #[cfg(target_os = "macos")]
    {
        let home = std::env::var("HOME").unwrap_or_default();
        let plist = format!("{}/Library/LaunchAgents/com.forge.daemon.plist", home);
        let _ = std::process::Command::new("launchctl")
            .args(["unload", &plist])
            .status();
        println!("Forge daemon stopped.");
    }
}

fn service_status() {
    #[cfg(target_os = "linux")]
    {
        let _ = std::process::Command::new("systemctl")
            .args(["--user", "status", "forge-daemon", "--no-pager"])
            .status();
    }

    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("launchctl")
            .args(["list", "com.forge.daemon"])
            .status();
    }

    // Also check if the daemon socket exists
    let socket = forge_core::default_socket_path();
    if std::path::Path::new(&socket).exists() {
        println!("\nDaemon socket exists at: {}", socket);
        println!("Run 'forge-next health' to verify it's responding.");
    } else {
        println!("\nDaemon socket NOT found. Service may not be running.");
    }
}

fn service_uninstall() {
    service_stop();

    #[cfg(target_os = "linux")]
    {
        let home = std::env::var("HOME").unwrap_or_default();
        let service_path = format!("{}/.config/systemd/user/forge-daemon.service", home);
        let _ = std::process::Command::new("systemctl")
            .args(["--user", "disable", "forge-daemon"])
            .status();
        let _ = std::fs::remove_file(&service_path);
        let _ = std::process::Command::new("systemctl")
            .args(["--user", "daemon-reload"])
            .status();
        println!("Service uninstalled.");
    }

    #[cfg(target_os = "macos")]
    {
        let home = std::env::var("HOME").unwrap_or_default();
        let plist = format!("{}/Library/LaunchAgents/com.forge.daemon.plist", home);
        let _ = std::fs::remove_file(&plist);
        println!("LaunchAgent uninstalled.");
    }
}

/// Detect the reality (project type) for a path.
pub async fn detect_reality(path: Option<String>) {
    let path = path.unwrap_or_else(|| {
        std::env::current_dir()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|_| ".".to_string())
    });
    let req = Request::DetectReality { path: path.clone() };
    match client::send(&req).await {
        Ok(Response::Ok {
            data:
                ResponseData::RealityDetected {
                    reality_id,
                    name,
                    reality_type,
                    domain,
                    detected_from,
                    confidence,
                    is_new,
                    ..
                },
        }) => {
            let status = if is_new { "NEW" } else { "existing" };
            println!("Reality detected ({status}):");
            println!("  ID:            {reality_id}");
            println!("  Name:          {name}");
            println!("  Type:          {reality_type}");
            println!("  Domain:        {domain}");
            println!("  Detected from: {detected_from}");
            println!("  Confidence:    {confidence:.2}");
        }
        Ok(Response::Error { message }) => {
            eprintln!("error: {message}");
            std::process::exit(1);
        }
        Ok(_) => eprintln!("unexpected response"),
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    }
}

/// List all known realities (projects).
pub async fn list_realities(organization: Option<String>) {
    let req = Request::ListRealities { organization_id: organization };
    match client::send(&req).await {
        Ok(Response::Ok { data: ResponseData::RealitiesList { realities } }) => {
            if realities.is_empty() {
                println!("No realities registered.");
                println!("Tip: Use 'forge-next detect-reality --path <dir>' to detect and register a project.");
                return;
            }
            println!("{} reality(ies):\n", realities.len());
            for r in &realities {
                let domain = r.domain.as_deref().unwrap_or("unknown");
                let path = r.project_path.as_deref().unwrap_or("(no path)");
                println!("  {} ({}) — {} [{}]", r.name, r.reality_type, domain, r.engine_status);
                println!("    ID:   {}", r.id);
                println!("    Path: {}", path);
                println!("    Last: {}", r.last_active);
            }
        }
        Ok(Response::Error { message }) => {
            eprintln!("error: {message}");
            std::process::exit(1);
        }
        Ok(_) => eprintln!("unexpected response"),
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    }
}

/// Code search: find symbols by name pattern.
pub async fn code_search(query: String, kind: Option<String>, limit: usize) {
    let req = Request::CodeSearch {
        query: query.clone(),
        kind,
        limit: Some(limit),
    };
    match client::send(&req).await {
        Ok(Response::Ok {
            data: ResponseData::CodeSearchResult { hits },
        }) => {
            if hits.is_empty() {
                println!("No symbols found matching '{query}'.");
                return;
            }
            println!("{} result(s) for '{query}':\n", hits.len());
            for hit in &hits {
                let name = hit.get("name").and_then(|v| v.as_str()).unwrap_or("?");
                let kind = hit.get("kind").and_then(|v| v.as_str()).unwrap_or("?");
                let file_path = hit.get("file_path").and_then(|v| v.as_str()).unwrap_or("?");
                let line_start = hit
                    .get("line_start")
                    .and_then(|v| v.as_i64())
                    .map(|l| format!(":{l}"))
                    .unwrap_or_default();
                println!("  {name} ({kind}) — {file_path}{line_start}");
            }
        }
        Ok(Response::Error { message }) => {
            eprintln!("error: {message}");
            std::process::exit(1);
        }
        Ok(_) => eprintln!("unexpected response"),
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    }
}

/// Send a heartbeat to keep a session alive.
pub async fn session_heartbeat(session_id: String) {
    let req = Request::SessionHeartbeat { session_id: session_id.clone() };
    match client::send(&req).await {
        Ok(Response::Ok { data: ResponseData::Heartbeat { status, .. } }) => {
            println!("{}", status);
        }
        Ok(Response::Error { message }) => {
            eprintln!("error: {}", message);
            std::process::exit(1);
        }
        Ok(other) => {
            eprintln!("unexpected response: {:?}", other);
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!("error: {}", e);
            std::process::exit(1);
        }
    }
}

/// Subscribe to real-time daemon events (streams NDJSON to stdout).
pub async fn subscribe(
    events: Option<Vec<String>>,
    session_id: Option<String>,
    team_id: Option<String>,
) {
    if let Err(e) = crate::transport::subscribe_stream(events, session_id, team_id).await {
        eprintln!("subscribe error: {}", e);
        std::process::exit(1);
    }
}

// ── Proactive Context (Prajna) ──

/// Per-turn context delta check (used by UserPromptSubmit hook).
pub async fn context_refresh(session_id: String, since: Option<String>) {
    let req = Request::ContextRefresh { session_id, since };
    match client::send(&req).await {
        Ok(Response::Ok {
            data:
                ResponseData::ContextDelta {
                    notifications,
                    warnings,
                    anti_patterns,
                    messages_pending,
                },
        }) => {
            for n in &notifications {
                println!("notification: {}", n);
            }
            for w in &warnings {
                println!("warning: {}", w);
            }
            for a in &anti_patterns {
                println!("anti-pattern: {}", a);
            }
            if messages_pending > 0 {
                println!("messages-pending: {}", messages_pending);
            }
        }
        Ok(Response::Error { message }) => {
            eprintln!("error: {}", message);
            std::process::exit(1);
        }
        Ok(_) => {}
        Err(e) => {
            eprintln!("error: {}", e);
            std::process::exit(1);
        }
    }
}

/// Check for premature completion signals (used by Stop hook).
pub async fn completion_check(session_id: String, claimed_done: bool) {
    let req = Request::CompletionCheck {
        session_id,
        claimed_done,
    };
    match client::send(&req).await {
        Ok(Response::Ok {
            data:
                ResponseData::CompletionCheckResult {
                    has_completion_signal,
                    relevant_lessons,
                    severity,
                },
        }) => {
            if has_completion_signal && !relevant_lessons.is_empty() {
                println!("severity: {}", severity);
                for l in &relevant_lessons {
                    println!("lesson: {}", l);
                }
            }
        }
        Ok(Response::Error { message }) => {
            eprintln!("error: {}", message);
            std::process::exit(1);
        }
        Ok(_) => {}
        Err(e) => {
            eprintln!("error: {}", e);
            std::process::exit(1);
        }
    }
}

/// Verify task completion criteria (used by TaskCompleted hook).
pub async fn task_completion_check(
    session_id: String,
    subject: String,
    description: Option<String>,
) {
    let req = Request::TaskCompletionCheck {
        session_id,
        task_subject: subject,
        task_description: description,
    };
    match client::send(&req).await {
        Ok(Response::Ok {
            data: ResponseData::TaskCompletionCheckResult { warnings, checklists },
        }) => {
            for w in &warnings {
                println!("warning: {}", w);
            }
            for c in &checklists {
                println!("checklist: {}", c);
            }
        }
        Ok(Response::Error { message }) => {
            eprintln!("error: {}", message);
            std::process::exit(1);
        }
        Ok(_) => {}
        Err(e) => {
            eprintln!("error: {}", e);
            std::process::exit(1);
        }
    }
}

pub async fn context_stats(session_id: Option<String>) {
    let req = Request::ContextStats { session_id: session_id.clone() };
    match client::send(&req).await {
        Ok(Response::Ok { data: ResponseData::ContextStatsResult {
            total_injections, total_chars, estimated_tokens, acknowledged, effectiveness_rate, per_hook,
        } }) => {
            println!("Context Injection Stats{}", session_id.as_deref().map(|s| format!(" (session: {})", s)).unwrap_or_default());
            println!("─────────────────────────");
            println!("  Injections:      {}", total_injections);
            println!("  Total chars:     {}", total_chars);
            println!("  Est. tokens:     {}", estimated_tokens);
            println!("  Acknowledged:    {}", acknowledged);
            println!("  Effectiveness:   {:.1}%", effectiveness_rate * 100.0);
            if !per_hook.is_empty() {
                println!("  ─────────────────────────");
                println!("  Per-hook breakdown:");
                for (hook, count, chars) in &per_hook {
                    println!("    {:<20} {:>3} inj, {:>6} chars (~{} tokens)", hook, count, chars, chars / 4);
                }
            }
        }
        Ok(Response::Error { message }) => { eprintln!("error: {}", message); std::process::exit(1); }
        Ok(_) => {}
        Err(e) => { eprintln!("error: {}", e); std::process::exit(1); }
    }
}

// ── Organization Hierarchy ──

pub async fn org_create(name: String, description: Option<String>) {
    let req = Request::CreateOrganization { name, description };
    match client::send(&req).await {
        Ok(Response::Ok { data: ResponseData::OrganizationCreated { id } }) => {
            println!("Organization created: {}", id);
        }
        Ok(Response::Error { message }) => {
            eprintln!("error: {}", message);
            std::process::exit(1);
        }
        Ok(_) => {
            eprintln!("unexpected response");
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!("error: {}", e);
            std::process::exit(1);
        }
    }
}

pub async fn org_list() {
    let req = Request::ListOrganizations;
    match client::send(&req).await {
        Ok(Response::Ok { data: ResponseData::OrganizationList { organizations } }) => {
            println!("{} organization(s):", organizations.len());
            for o in &organizations {
                println!(
                    "  {} — {} {}",
                    o["id"].as_str().unwrap_or("?"),
                    o["name"].as_str().unwrap_or("?"),
                    o["description"]
                        .as_str()
                        .map(|d| format!("({})", d))
                        .unwrap_or_default()
                );
            }
        }
        Ok(Response::Error { message }) => {
            eprintln!("error: {}", message);
            std::process::exit(1);
        }
        Ok(_) => {
            eprintln!("unexpected response");
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!("error: {}", e);
            std::process::exit(1);
        }
    }
}

pub async fn org_from_template(template: String, name: String) {
    let req = Request::CreateOrgFromTemplate {
        template_name: template,
        org_name: name,
    };
    match client::send(&req).await {
        Ok(Response::Ok {
            data: ResponseData::OrgFromTemplateCreated { org_id, teams_created },
        }) => {
            println!(
                "Organization created: {} ({} teams from template)",
                org_id, teams_created
            );
        }
        Ok(Response::Error { message }) => {
            eprintln!("error: {}", message);
            std::process::exit(1);
        }
        Ok(_) => {
            eprintln!("unexpected response");
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!("error: {}", e);
            std::process::exit(1);
        }
    }
}

pub async fn healing_status() {
    match client::send(&Request::HealingStatus).await {
        Ok(Response::Ok {
            data:
                ResponseData::HealingStatusResult {
                    total_healed,
                    auto_superseded,
                    auto_faded,
                    last_cycle_at,
                    stale_candidates,
                },
        }) => {
            println!("Memory Healing Status");
            println!("─────────────────────");
            println!("  Total healed:      {total_healed}");
            println!("  Auto-superseded:   {auto_superseded}");
            println!("  Auto-faded:        {auto_faded}");
            println!(
                "  Last cycle:        {}",
                last_cycle_at.unwrap_or_else(|| "never".into())
            );
            println!("  Stale candidates:  {stale_candidates}");
        }
        Ok(Response::Error { message }) => {
            eprintln!("error: {message}");
            std::process::exit(1);
        }
        Ok(_) => eprintln!("unexpected response"),
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    }
}

pub async fn healing_run() {
    match client::send(&Request::HealingRun).await {
        Ok(Response::Ok {
            data:
                ResponseData::HealingRunResult {
                    topic_superseded,
                    session_faded,
                    quality_adjusted,
                },
        }) => {
            println!("Healing cycle complete:");
            println!("  Topic superseded:  {topic_superseded}");
            println!("  Session faded:     {session_faded}");
            println!("  Quality adjusted:  {quality_adjusted}");
        }
        Ok(Response::Error { message }) => {
            eprintln!("error: {message}");
            std::process::exit(1);
        }
        Ok(_) => eprintln!("unexpected response"),
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    }
}

pub async fn healing_log(limit: usize, action: Option<String>) {
    match client::send(&Request::HealingLog {
        limit: Some(limit),
        action,
    })
    .await
    {
        Ok(Response::Ok {
            data: ResponseData::HealingLogResult { entries, count },
        }) => {
            if count == 0 {
                println!("No healing log entries.");
                return;
            }
            println!("{count} healing log entries:\n");
            for e in &entries {
                println!(
                    "  [{}] {} — {}",
                    e["action"].as_str().unwrap_or("?"),
                    e["reason"].as_str().unwrap_or("?"),
                    e["created_at"].as_str().unwrap_or("?")
                );
                if let Some(old) = e["old_memory_id"].as_str() {
                    print!("    old: {old}");
                    if let Some(new) = e["new_memory_id"].as_str() {
                        print!(" → new: {new}");
                    }
                    println!();
                }
            }
        }
        Ok(Response::Error { message }) => {
            eprintln!("error: {message}");
            std::process::exit(1);
        }
        Ok(_) => eprintln!("unexpected response"),
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    }
}

pub async fn org_init(name: String, template: Option<String>) {
    let req = Request::WorkspaceInit { org_name: name, template };
    match client::send(&req).await {
        Ok(Response::Ok { data: ResponseData::WorkspaceInitialized { path, teams_created } }) => {
            println!("Workspace initialized at: {}", path);
            println!("Teams created: {}", teams_created);
        }
        Ok(Response::Error { message }) => {
            eprintln!("error: {}", message);
            std::process::exit(1);
        }
        Ok(_) => {
            eprintln!("unexpected response");
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!("error: {}", e);
            std::process::exit(1);
        }
    }
}

pub async fn workspace_status() {
    let req = Request::WorkspaceStatus;
    match client::send(&req).await {
        Ok(Response::Ok { data: ResponseData::WorkspaceStatusData { mode, org, root, teams } }) => {
            println!("Workspace Mode: {}", mode);
            if !org.is_empty() {
                println!("Organization:   {}", org);
            }
            if !root.is_empty() {
                println!("Root:           {}", root);
            }
            if teams.is_empty() {
                println!("Teams:          (none)");
            } else {
                println!("Teams ({}):", teams.len());
                for t in &teams {
                    println!("  - {}", t);
                }
            }
        }
        Ok(Response::Error { message }) => {
            eprintln!("error: {}", message);
            std::process::exit(1);
        }
        Ok(_) => {
            eprintln!("unexpected response");
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!("error: {}", e);
            std::process::exit(1);
        }
    }
}

fn chrono_now() -> String {
    forge_core::time::timestamp_now()
}
