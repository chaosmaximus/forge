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
            data: ResponseData::BlastRadius { decisions, callers, importers, files_affected },
        }) => {
            println!("Blast radius for {file}:");
            println!("  Decisions:         {}", decisions.len());
            for d in &decisions {
                println!("    - {} (confidence: {:.2}) [{}]", d.title, d.confidence, d.id);
            }
            println!("  Callers:           {callers}");
            println!("  Importers:         {}", importers.len());
            for imp in &importers {
                println!("    - {imp}");
            }
            println!("  Co-affected files: {}", files_affected.len());
            for f in &files_affected {
                println!("    - {f}");
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
pub async fn register_session(id: String, agent: String, project: Option<String>, cwd: Option<String>) {
    match client::send(&Request::RegisterSession { id: id.clone(), agent, project, cwd }).await {
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

fn chrono_now() -> String {
    forge_core::time::timestamp_now()
}
