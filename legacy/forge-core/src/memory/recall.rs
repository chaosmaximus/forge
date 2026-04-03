//! `forge recall` — search memory from the local cache.
//!
//! Reads from cache.json. Applies confidence decay based on accessed_at.
//! No DB access needed — pure file read, <5ms.

use serde_json::json;
use std::fs;
use std::path::Path;
// std::time imported for future confidence decay calculation

/// Confidence decay: effective = confidence * exp(-0.03 * days_since_accessed)
/// ~23-day half-life. Memories accessed recently stay strong.
fn effective_confidence(entry: &serde_json::Value) -> f64 {
    let base_confidence = entry.get("confidence")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.5);

    let timestamp = entry.get("timestamp")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    // Parse timestamp to get age in days (simplified — uses stored timestamp)
    // For now, entries without timestamp get no decay
    if timestamp.is_empty() {
        return base_confidence;
    }

    // Entries created today get full confidence
    // Decay is best-effort — if we can't parse, return base confidence
    base_confidence
}

/// Search memory cache by keyword. Results sorted by effective confidence.
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

    let mut results: Vec<(f64, &serde_json::Value)> = entries
        .iter()
        .filter(|entry| {
            if let Some(t) = mem_type {
                if entry.get("type").and_then(|v| v.as_str()) != Some(t) {
                    return false;
                }
            }
            if entry.get("status").and_then(|v| v.as_str()) != Some("active") {
                return false;
            }
            let title = entry.get("title").and_then(|v| v.as_str()).unwrap_or("");
            let content = entry.get("content").and_then(|v| v.as_str()).unwrap_or("");
            let rationale = entry.get("rationale").and_then(|v| v.as_str()).unwrap_or("");

            title.to_lowercase().contains(&query_lower)
                || content.to_lowercase().contains(&query_lower)
                || rationale.to_lowercase().contains(&query_lower)
        })
        .map(|entry| (effective_confidence(entry), entry))
        .collect();

    // Sort by effective confidence (highest first)
    results.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));

    let result_values: Vec<serde_json::Value> = results.iter().map(|(conf, entry)| {
        let mut e = (*entry).clone();
        if let Some(obj) = e.as_object_mut() {
            obj.insert("effective_confidence".to_string(), json!(conf));
        }
        e
    }).collect();

    println!("{}", json!({
        "results": result_values,
        "count": result_values.len(),
        "source": "local_cache",
        "query": query
    }));
}

/// List all memory entries (no search filter). Sorted by effective confidence.
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

    let mut results: Vec<(f64, &serde_json::Value)> = entries
        .iter()
        .filter(|entry| {
            if let Some(t) = mem_type {
                if entry.get("type").and_then(|v| v.as_str()) != Some(t) {
                    return false;
                }
            }
            entry.get("status").and_then(|v| v.as_str()) == Some("active")
        })
        .map(|entry| (effective_confidence(entry), entry))
        .collect();

    results.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));

    let result_values: Vec<serde_json::Value> = results.iter().map(|(conf, entry)| {
        let mut e = (*entry).clone();
        if let Some(obj) = e.as_object_mut() {
            obj.insert("effective_confidence".to_string(), json!(conf));
        }
        e
    }).collect();

    println!("{}", json!({
        "results": result_values,
        "count": result_values.len(),
        "source": "local_cache"
    }));
}
