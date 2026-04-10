//! Ingest Claude Code's MEMORY.md files into the Forge daemon.
//! Scans ~/.claude/projects/*/memory/ for MEMORY.md and linked .md files.
//! Parses frontmatter (name, description, type) and imports as memories.

use crate::db::ops;
use forge_core::types::{Memory, MemoryType};
use rusqlite::Connection;
use std::path::Path;

/// Scan all Claude project memory directories and import memories.
pub fn ingest_claude_memories(conn: &Connection) -> Result<(usize, usize), String> {
    let home = std::env::var("HOME").unwrap_or_default();
    let projects_dir = format!("{}/.claude/projects", home);

    let mut imported = 0usize;
    let mut skipped = 0usize;

    let entries = std::fs::read_dir(&projects_dir).map_err(|e| e.to_string())?;
    for entry in entries.flatten() {
        let memory_dir = entry.path().join("memory");
        if !memory_dir.is_dir() {
            continue;
        }

        // Read all .md files in the memory directory (except MEMORY.md index)
        if let Ok(files) = std::fs::read_dir(&memory_dir) {
            for file in files.flatten() {
                let path = file.path();
                if path.extension().and_then(|e| e.to_str()) != Some("md") {
                    continue;
                }
                if path.file_name().and_then(|n| n.to_str()) == Some("MEMORY.md") {
                    continue;
                }

                match parse_claude_memory_file(&path) {
                    Ok(Some(memory)) => match ops::remember(conn, &memory) {
                        Ok(()) => imported += 1,
                        Err(_) => skipped += 1,
                    },
                    Ok(None) => skipped += 1, // no frontmatter
                    Err(_) => skipped += 1,
                }
            }
        }
    }

    Ok((imported, skipped))
}

/// Parse a Claude memory .md file with YAML frontmatter.
pub fn parse_claude_memory_file(path: &Path) -> Result<Option<Memory>, String> {
    let content = std::fs::read_to_string(path).map_err(|e| e.to_string())?;

    // Parse frontmatter between --- markers
    if !content.starts_with("---") {
        return Ok(None);
    }
    let end = content[3..].find("---").map(|i| i + 3);
    let Some(end) = end else {
        return Ok(None);
    };

    // ISSUE-25: verify char boundaries before slicing
    if !content.is_char_boundary(end) || !content.is_char_boundary(end + 3) {
        return Ok(None);
    }
    let frontmatter = &content[3..end];
    let body = content[end + 3..].trim();

    // Extract fields from YAML-like frontmatter
    let mut name = String::new();
    let mut description = String::new();
    let mut mem_type = String::new();

    for line in frontmatter.lines() {
        let line = line.trim();
        if let Some(val) = line.strip_prefix("name:") {
            name = val.trim().to_string();
        } else if let Some(val) = line.strip_prefix("description:") {
            description = val.trim().to_string();
        } else if let Some(val) = line.strip_prefix("type:") {
            mem_type = val.trim().to_string();
        }
    }

    if name.is_empty() {
        return Ok(None);
    }

    // Map Claude memory types to Forge types
    let memory_type = match mem_type.as_str() {
        "user" => MemoryType::Preference,  // user info -> preference
        "feedback" => MemoryType::Lesson,  // feedback -> lesson
        "project" => MemoryType::Decision, // project info -> decision
        "reference" => MemoryType::Pattern, // reference -> pattern
        _ => MemoryType::Lesson,           // default
    };

    let full_content = if body.is_empty() {
        description.clone()
    } else {
        format!("{}\n\n{}", description, body)
    };

    // Tag with source project
    let project = path
        .parent()
        .and_then(|p| p.parent())
        .and_then(|p| p.file_name())
        .and_then(|n| n.to_str())
        .unwrap_or("unknown")
        .to_string();

    let memory = Memory::new(memory_type, name, full_content)
        .with_confidence(0.95) // Claude's own memories are high-confidence
        .with_tags(vec![
            "claude-memory".to_string(),
            format!("project:{}", project),
        ])
        .with_project(project);

    Ok(Some(memory))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn test_parse_claude_memory_file() {
        let content = r#"---
name: User autonomy preference
description: DurgaSaiK wants AI to guide clearly but never decide autonomously
type: feedback
---

"Proactive" means clear communication and guidance, NOT autonomous action.
"#;
        let mut tmp = NamedTempFile::with_suffix(".md").unwrap();
        write!(tmp, "{}", content).unwrap();

        let memory = parse_claude_memory_file(tmp.path()).unwrap().unwrap();
        assert_eq!(memory.title, "User autonomy preference");
        assert_eq!(memory.memory_type, MemoryType::Lesson); // feedback -> lesson
        assert!(memory.content.contains("Proactive"));
        assert!((memory.confidence - 0.95).abs() < f64::EPSILON);
        assert!(memory.tags.contains(&"claude-memory".to_string()));
    }

    #[test]
    fn test_parse_no_frontmatter() {
        let content = "# Just a regular markdown file\n\nNo frontmatter here.\n";
        let mut tmp = NamedTempFile::with_suffix(".md").unwrap();
        write!(tmp, "{}", content).unwrap();

        let result = parse_claude_memory_file(tmp.path()).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_parse_empty_name() {
        let content = "---\nname:\ndescription: Some desc\ntype: user\n---\nBody text.\n";
        let mut tmp = NamedTempFile::with_suffix(".md").unwrap();
        write!(tmp, "{}", content).unwrap();

        let result = parse_claude_memory_file(tmp.path()).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_parse_project_type() {
        let content = "---\nname: Forge plugin project\ndescription: v0.1.5 published\ntype: project\n---\n";
        let mut tmp = NamedTempFile::with_suffix(".md").unwrap();
        write!(tmp, "{}", content).unwrap();

        let memory = parse_claude_memory_file(tmp.path()).unwrap().unwrap();
        assert_eq!(memory.memory_type, MemoryType::Decision); // project -> decision
    }

    #[test]
    fn test_parse_user_type() {
        let content = "---\nname: User pref\ndescription: Prefers Rust\ntype: user\n---\n";
        let mut tmp = NamedTempFile::with_suffix(".md").unwrap();
        write!(tmp, "{}", content).unwrap();

        let memory = parse_claude_memory_file(tmp.path()).unwrap().unwrap();
        assert_eq!(memory.memory_type, MemoryType::Preference); // user -> preference
    }

    #[test]
    fn test_parse_reference_type() {
        let content = "---\nname: API reference\ndescription: REST API docs\ntype: reference\n---\n";
        let mut tmp = NamedTempFile::with_suffix(".md").unwrap();
        write!(tmp, "{}", content).unwrap();

        let memory = parse_claude_memory_file(tmp.path()).unwrap().unwrap();
        assert_eq!(memory.memory_type, MemoryType::Pattern); // reference -> pattern
    }
}
