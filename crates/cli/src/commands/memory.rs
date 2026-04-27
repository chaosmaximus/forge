use crate::client;
use forge_core::protocol::{Request, Response, ResponseData};
use forge_core::types::MemoryType;

/// Parse a string into a MemoryType.
fn parse_memory_type(s: &str) -> Result<MemoryType, String> {
    match s.to_lowercase().as_str() {
        "decision" => Ok(MemoryType::Decision),
        "lesson" => Ok(MemoryType::Lesson),
        "pattern" => Ok(MemoryType::Pattern),
        "preference" => Ok(MemoryType::Preference),
        "protocol" => Ok(MemoryType::Protocol),
        _ => Err(format!(
            "unknown memory type: '{s}'. Expected: decision, lesson, pattern, preference, protocol"
        )),
    }
}

/// Search memories (hybrid BM25 + vector + graph).
#[allow(clippy::too_many_arguments)]
pub async fn recall(
    query: String,
    type_filter: Option<String>,
    project: Option<String>,
    limit: usize,
    layer: Option<String>,
    since: Option<String>,
    include_globals: bool,
) {
    let memory_type = match type_filter {
        Some(t) => match parse_memory_type(&t) {
            Ok(mt) => Some(mt),
            Err(e) => {
                eprintln!("error: {e}");
                std::process::exit(1);
            }
        },
        None => None,
    };

    let request = Request::Recall {
        query,
        memory_type,
        project,
        limit: Some(limit),
        layer,
        since,
        include_flipped: None,
        // Phase P3-3.11 W29: forward the CLI `--include-globals` flag.
        // None and Some(false) are wire-equivalent (the daemon defaults
        // to strict scope), so encode the flag faithfully so a future
        // daemon could distinguish "explicitly opted out" from "didn't
        // ask" if it wanted to.
        include_globals: Some(include_globals),
        query_embedding: None,
    };

    match client::send(&request).await {
        Ok(Response::Ok {
            data: ResponseData::Memories { results, count },
        }) => {
            if count == 0 {
                println!("No memories found.");
                return;
            }
            println!(
                "{count} memor{} found:\n",
                if count == 1 { "y" } else { "ies" }
            );
            for (i, r) in results.iter().enumerate() {
                println!(
                    "  [{}] {title} (score: {score:.3}, type: {mtype:?})",
                    i + 1,
                    title = r.memory.title,
                    score = r.score,
                    mtype = r.memory.memory_type,
                );
                println!("      {}", r.memory.content);
                if !r.memory.tags.is_empty() {
                    println!("      tags: {}", r.memory.tags.join(", "));
                }
                println!();
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

/// Store a memory.
#[allow(clippy::too_many_arguments)]
pub async fn remember(
    memory_type: String,
    title: String,
    content: String,
    confidence: Option<f64>,
    tags: Option<Vec<String>>,
    project: Option<String>,
    metadata: Option<serde_json::Value>,
    valence: Option<String>,
    intensity: Option<f64>,
) {
    let mt = match parse_memory_type(&memory_type) {
        Ok(mt) => mt,
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    };

    let request = Request::Remember {
        memory_type: mt,
        title,
        content,
        confidence,
        tags,
        project,
        metadata,
        valence,
        intensity,
    };

    match client::send(&request).await {
        Ok(Response::Ok {
            data: ResponseData::Stored { id },
        }) => {
            println!("Stored: {id}");
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

/// Soft-delete a memory by ID.
pub async fn forget(id: String) {
    let request = Request::Forget { id };

    match client::send(&request).await {
        Ok(Response::Ok {
            data: ResponseData::Forgotten { id },
        }) => {
            println!("Forgotten: {id}");
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

/// Mark an old memory as superseded by a newer one.
pub async fn supersede(old_id: String, new_id: String) {
    let request = Request::Supersede { old_id, new_id };

    match client::send(&request).await {
        Ok(Response::Ok {
            data: ResponseData::Superseded { old_id, new_id },
        }) => {
            println!("Superseded: {old_id} → {new_id}");
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
