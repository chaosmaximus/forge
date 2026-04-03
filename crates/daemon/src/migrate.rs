// migrate.rs — Import v1 cache.json into SQLite
//
// Reads the v1 JSON cache format and converts active entries into v2 Memory records.

use crate::db::ops;
use forge_core::types::{Memory, MemoryType};
use rusqlite::Connection;

#[derive(Debug, serde::Deserialize)]
struct V1CacheEntry {
    #[serde(rename = "type")]
    entry_type: Option<String>,
    title: Option<String>,
    content: Option<String>,
    confidence: Option<f64>,
    status: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
struct V1Cache {
    entries: Vec<V1CacheEntry>,
}

/// Import v1 cache.json into SQLite. Returns (imported, skipped).
pub fn import_v1_cache(conn: &Connection, cache_path: &str) -> Result<(usize, usize), String> {
    let content = std::fs::read_to_string(cache_path)
        .map_err(|e| format!("cannot read {}: {}", cache_path, e))?;
    let cache: V1Cache =
        serde_json::from_str(&content).map_err(|e| format!("cannot parse: {}", e))?;

    let mut imported = 0;
    let mut skipped = 0;

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

        let memory = Memory::new(memory_type, title, entry.content.clone().unwrap_or_default())
            .with_confidence(entry.confidence.unwrap_or(0.5));

        match ops::remember(conn, &memory) {
            Ok(()) => imported += 1,
            Err(e) => {
                eprintln!("[migrate] error: {}", e);
                skipped += 1;
            }
        }
    }
    Ok((imported, skipped))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::schema::create_schema;
    use std::io::Write;

    fn open_db() -> Connection {
        crate::db::vec::init_sqlite_vec();
        let conn = Connection::open_in_memory().unwrap();
        create_schema(&conn).unwrap();
        conn
    }

    #[test]
    fn test_import_v1_cache() {
        let conn = open_db();

        let cache_json = r#"{
            "entries": [
                {
                    "type": "decision",
                    "title": "Use JWT",
                    "content": "For auth",
                    "confidence": 0.9,
                    "status": "active"
                },
                {
                    "type": "lesson",
                    "title": "TDD first",
                    "content": "Always write tests",
                    "confidence": 0.8,
                    "status": "active"
                },
                {
                    "type": "decision",
                    "title": "Old decision",
                    "content": "Superseded",
                    "confidence": 0.5,
                    "status": "superseded"
                }
            ]
        }"#;

        let mut tmpfile = tempfile::NamedTempFile::new().unwrap();
        tmpfile.write_all(cache_json.as_bytes()).unwrap();
        tmpfile.flush().unwrap();

        let (imported, skipped) =
            import_v1_cache(&conn, tmpfile.path().to_str().unwrap()).unwrap();

        assert_eq!(imported, 2);
        assert_eq!(skipped, 1);
    }

    #[test]
    fn test_import_missing_file() {
        let conn = open_db();
        let result = import_v1_cache(&conn, "/nonexistent/cache.json");
        assert!(result.is_err());
    }

    #[test]
    fn test_import_empty_cache() {
        let conn = open_db();

        let cache_json = r#"{"entries": []}"#;

        let mut tmpfile = tempfile::NamedTempFile::new().unwrap();
        tmpfile.write_all(cache_json.as_bytes()).unwrap();
        tmpfile.flush().unwrap();

        let (imported, skipped) =
            import_v1_cache(&conn, tmpfile.path().to_str().unwrap()).unwrap();

        assert_eq!(imported, 0);
        assert_eq!(skipped, 0);
    }
}
