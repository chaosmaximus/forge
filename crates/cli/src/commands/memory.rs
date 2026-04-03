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
        _ => Err(format!(
            "unknown memory type: '{s}'. Expected: decision, lesson, pattern, preference"
        )),
    }
}

/// Search memories (hybrid BM25 + vector + graph).
pub async fn recall(query: String, type_filter: Option<String>, limit: usize) {
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
        limit: Some(limit),
    };

    match client::send(&request).await {
        Ok(Response::Ok {
            data: ResponseData::Memories { results, count },
        }) => {
            if count == 0 {
                println!("No memories found.");
                return;
            }
            println!("{count} memor{} found:\n", if count == 1 { "y" } else { "ies" });
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
pub async fn remember(
    memory_type: String,
    title: String,
    content: String,
    confidence: Option<f64>,
    tags: Option<Vec<String>>,
    project: Option<String>,
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
