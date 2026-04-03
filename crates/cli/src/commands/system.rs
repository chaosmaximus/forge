use crate::client;
use forge_v2_core::protocol::{Request, Response, ResponseData};
use forge_v2_core::types::MemoryType;

/// Print daemon health diagnostics (doctor).
pub async fn doctor() {
    match client::send(&Request::Doctor).await {
        Ok(Response::Ok {
            data:
                ResponseData::Doctor {
                    daemon_up,
                    memory_count,
                    file_count,
                    symbol_count,
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
            println!("  Files:     {}", file_count);
            println!("  Symbols:   {}", symbol_count);
            println!("  Workers:   {}", workers.join(", "));
        }
        Ok(Response::Error { message }) => eprintln!("error: {}", message),
        Ok(_) => eprintln!("unexpected response"),
        Err(e) => eprintln!("error: {}", e),
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
