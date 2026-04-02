//! `forge recall` — search memory from the local cache.
//!
//! Reads from cache.json (written by remember + MCP server sync).
//! No DB access needed — pure file read, <5ms.

use serde_json::json;
use std::fs;
use std::path::Path;

/// Search memory cache by keyword.
pub fn run(state_dir: &str, query: &str, mem_type: Option<&str>) {
    let cache_path = Path::new(state_dir).join("memory").join("cache.json");

    let cache: serde_json::Value = match fs::read_to_string(&cache_path) {
        Ok(s) => serde_json::from_str(&s).unwrap_or_else(|_| json!({"entries": []})),
        Err(_) => {
            println!("{}", json!({"results": [], "count": 0, "source": "cache_empty"}));
            return;
        }
    };

    let entries = match cache.get("entries").and_then(|e| e.as_array()) {
        Some(e) => e,
        None => {
            println!("{}", json!({"results": [], "count": 0, "source": "no_entries"}));
            return;
        }
    };

    let query_lower = query.to_lowercase();

    let results: Vec<&serde_json::Value> = entries
        .iter()
        .filter(|entry| {
            // Filter by type if specified
            if let Some(t) = mem_type {
                if entry.get("type").and_then(|v| v.as_str()) != Some(t) {
                    return false;
                }
            }

            // Only active entries
            if entry.get("status").and_then(|v| v.as_str()) != Some("active") {
                return false;
            }

            // Keyword search in title + content
            let title = entry.get("title").and_then(|v| v.as_str()).unwrap_or("");
            let content = entry.get("content").and_then(|v| v.as_str()).unwrap_or("");
            let rationale = entry.get("rationale").and_then(|v| v.as_str()).unwrap_or("");

            title.to_lowercase().contains(&query_lower)
                || content.to_lowercase().contains(&query_lower)
                || rationale.to_lowercase().contains(&query_lower)
        })
        .collect();

    println!("{}", json!({
        "results": results,
        "count": results.len(),
        "source": "local_cache",
        "query": query
    }));
}

/// List all memory entries (no search filter).
pub fn list(state_dir: &str, mem_type: Option<&str>) {
    let cache_path = Path::new(state_dir).join("memory").join("cache.json");

    let cache: serde_json::Value = match fs::read_to_string(&cache_path) {
        Ok(s) => serde_json::from_str(&s).unwrap_or_else(|_| json!({"entries": []})),
        Err(_) => {
            println!("{}", json!({"results": [], "count": 0}));
            return;
        }
    };

    let entries = match cache.get("entries").and_then(|e| e.as_array()) {
        Some(e) => e,
        None => {
            println!("{}", json!({"results": [], "count": 0}));
            return;
        }
    };

    let results: Vec<&serde_json::Value> = entries
        .iter()
        .filter(|entry| {
            if let Some(t) = mem_type {
                if entry.get("type").and_then(|v| v.as_str()) != Some(t) {
                    return false;
                }
            }
            entry.get("status").and_then(|v| v.as_str()) == Some("active")
        })
        .collect();

    println!("{}", json!({
        "results": results,
        "count": results.len(),
        "source": "local_cache"
    }));
}
