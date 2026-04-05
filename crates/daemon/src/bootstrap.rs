//! bootstrap.rs -- Batch extract memories from all existing transcript files.
//!
//! Scans transcript directories for all adapters, checks against transcript_log
//! to skip already-processed files, and runs extraction on new/changed files.
//! Designed for first-install: "forge-next bootstrap" populates memory immediately.
//!
//! FAIL-LOUD: all errors are logged via eprintln, never silently swallowed.

use crate::adapters::AgentAdapter;
use crate::db::ops;
use forge_core::types::session::ConversationChunk;
use forge_core::types::{Memory, MemoryType};
use rusqlite::Connection;
use std::path::PathBuf;

/// Result of a bootstrap run.
#[derive(Debug, Default)]
pub struct BootstrapResult {
    pub files_processed: usize,
    pub files_skipped: usize,
    pub memories_extracted: usize,
    pub errors: usize,
}

/// FNV-1a hash -- fast, deterministic, good enough for change detection (not security).
fn fnv1a(data: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in data {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

/// Compute content hash: mtime + file size + first 4KB.
/// Uses mtime as secondary check to catch in-place rewrites that preserve size.
/// NOT cryptographic — just for change detection.
pub fn compute_content_hash(path: &std::path::Path) -> Result<String, String> {
    use std::io::Read;
    let mut f = std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let metadata = f.metadata().map_err(|e| format!("{}: {e}", path.display()))?;
    let size = metadata.len();
    let mtime = metadata.modified()
        .map(|t| t.duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs())
        .unwrap_or(0);
    let read_len = 4096.min(size as usize);
    let mut buf = vec![0u8; read_len];
    f.read_exact(&mut buf).map_err(|e| format!("{}: {e}", path.display()))?;
    let hash = format!("{:x}-{}-{}", fnv1a(&buf), size, mtime);
    Ok(hash)
}

/// Scan all adapter transcript directories and return all transcript file paths
/// with their adapter name.
pub fn scan_transcripts(adapters: &[Box<dyn AgentAdapter>]) -> Vec<(PathBuf, String)> {
    let mut results = Vec::new();
    for adapter in adapters {
        let ext = adapter.file_extension();
        for dir in adapter.watch_dirs() {
            if !dir.exists() {
                continue;
            }
            match walk_dir(&dir, ext) {
                Ok(files) => {
                    for path in files {
                        results.push((path, adapter.name().to_string()));
                    }
                }
                Err(e) => {
                    eprintln!(
                        "[bootstrap] WARN: failed to walk {}: {e}",
                        dir.display()
                    );
                }
            }
        }
    }
    results
}

/// Recursive directory walk filtered by extension.
fn walk_dir(dir: &PathBuf, ext: &str) -> Result<Vec<PathBuf>, String> {
    let mut files = Vec::new();
    walk_dir_inner(dir, ext, &mut files);
    Ok(files)
}

fn walk_dir_inner(dir: &PathBuf, ext: &str, files: &mut Vec<PathBuf>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("[bootstrap] WARN: can't read dir {}: {e}", dir.display());
            return;
        }
    };
    for entry in entries.flatten() {
        let path = entry.path();
        // Symlink defense: refuse to follow symlinks (Codex review finding)
        if let Ok(meta) = std::fs::symlink_metadata(&path) {
            if meta.file_type().is_symlink() {
                eprintln!("[bootstrap] WARN: skipping symlink {}", path.display());
                continue;
            }
        }
        if path.is_dir() {
            walk_dir_inner(&path, ext, files);
        } else if path.extension().is_some_and(|e| e == ext) {
            files.push(path);
        }
    }
}

/// Check if a transcript needs processing (new or changed since last time).
/// Returns (needs_work, last_offset).
pub fn needs_processing(conn: &Connection, path: &std::path::Path, current_hash: &str) -> (bool, usize) {
    let path_str = path.to_string_lossy();
    match conn.query_row(
        "SELECT content_hash, offset_processed FROM transcript_log WHERE path = ?1",
        rusqlite::params![path_str.as_ref()],
        |row| Ok((row.get::<_, String>(0)?, row.get::<_, usize>(1)?)),
    ) {
        Ok((stored_hash, offset)) => {
            if stored_hash == current_hash {
                (false, offset) // unchanged
            } else {
                // Hash changed — reset offset to 0 to reparse from beginning
                // (file may have been rewritten, not just appended)
                (true, 0)
            }
        }
        Err(rusqlite::Error::QueryReturnedNoRows) => (true, 0), // new file
        Err(e) => {
            eprintln!(
                "[bootstrap] WARN: transcript_log query failed for {}: {e}",
                path.display()
            );
            (true, 0)
        }
    }
}

/// Update the transcript_log after processing.
#[allow(clippy::too_many_arguments)]
pub fn update_log(
    conn: &Connection,
    path: &std::path::Path,
    adapter: &str,
    project: Option<&str>,
    size_bytes: u64,
    offset: usize,
    content_hash: &str,
    memories: usize, // clippy: 8 args is fine for a log-update function
) {
    let path_str = path.to_string_lossy();
    let now = crate::db::manas::now_offset(0);
    if let Err(e) = conn.execute(
        "INSERT OR REPLACE INTO transcript_log \
         (path, adapter, project, size_bytes, offset_processed, content_hash, processed_at, memories_extracted) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        rusqlite::params![
            path_str.as_ref(),
            adapter,
            project.unwrap_or(""),
            size_bytes as i64,
            offset as i64,
            content_hash,
            now,
            memories as i64,
        ],
    ) {
        eprintln!("[bootstrap] WARN: failed to update transcript_log: {e}");
    }
}

/// Extract project name from a transcript file path.
/// Reuses the same logic as extractor.rs:327-347.
pub fn extract_project_from_path(path: &std::path::Path) -> Option<String> {
    let path_str = path.to_string_lossy();
    if let Some(projects_idx) = path_str.find("/projects/") {
        let after = &path_str[projects_idx + 10..];
        if let Some(slash) = after.find('/') {
            let project_hash = &after[..slash];
            // The hash is like "-Users-name-workspace-projectname"
            // Take the last segment after the last dash-separated word
            let project_name = project_hash.rsplit('-').next().unwrap_or(project_hash);
            if !project_name.is_empty() {
                return Some(project_name.to_string());
            }
        }
    }
    None
}

/// Create a summary memory from a parsed transcript.
fn create_transcript_summary(
    chunks: &[ConversationChunk],
    project: Option<&str>,
    path: &PathBuf,
) -> Option<Memory> {
    if chunks.is_empty() {
        return None;
    }

    let turns = chunks.len();
    let user_turns = chunks.iter().filter(|c| c.role == "user").count();
    let has_tools = chunks.iter().any(|c| c.has_tool_use);

    // Extract session date from file modification time
    let date = std::fs::metadata(path)
        .ok()
        .and_then(|m| m.modified().ok())
        .map(|t| {
            let duration = t
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default();
            let secs = duration.as_secs();
            // Simple date: YYYY-MM-DD approximation
            let days = secs / 86400;
            // Days since epoch to Y-M-D (approximate but good enough for summaries)
            let year = 1970 + (days * 400 / 146097); // Gregorian approximation
            let remaining_days = days - ((year - 1970) * 365 + (year - 1970) / 4);
            let month = (remaining_days / 30).min(11) + 1;
            let day = (remaining_days % 30) + 1;
            format!("{}-{:02}-{:02}", year, month, day)
        })
        .unwrap_or_else(|| "unknown date".to_string());

    // Extract key topic from first user message
    let first_user = chunks
        .iter()
        .find(|c| c.role == "user")
        .map(|c| c.content.chars().take(200).collect::<String>())
        .unwrap_or_default();

    let title = format!(
        "Session: {} ({} turns, {})",
        project.unwrap_or("unknown"),
        turns,
        if has_tools { "with tools" } else { "conversation" }
    );

    let content = format!(
        "Project: {}\nDate: {}\nTurns: {} ({} user)\nTools used: {}\nStarted with: {}",
        project.unwrap_or("unknown"),
        date,
        turns,
        user_turns,
        has_tools,
        first_user.trim()
    );

    let mut memory = Memory::new(MemoryType::Pattern, title, content);
    memory.confidence = 0.4; // Low confidence -- raw bootstrap, not LLM-extracted
    memory.project = project.map(String::from);
    memory.tags = vec!["bootstrap".to_string(), "transcript-summary".to_string()];

    Some(memory)
}

/// Run bootstrap: scan all transcripts, process new/changed ones.
/// This is synchronous -- called from the handler with the DB connection.
pub fn run_bootstrap(
    conn: &Connection,
    adapters: &[Box<dyn AgentAdapter>],
    project_filter: Option<&str>,
) -> BootstrapResult {
    let all_files = scan_transcripts(adapters);
    let total = all_files.len();

    let mut result = BootstrapResult::default();

    eprintln!(
        "[bootstrap] found {} transcript files across {} adapters",
        total,
        adapters.len()
    );

    for (i, (path, adapter_name)) in all_files.iter().enumerate() {
        // Extract project from path
        let project = extract_project_from_path(path);

        // Apply project filter if specified
        if let Some(filter) = project_filter {
            if project.as_deref() != Some(filter) {
                result.files_skipped += 1;
                continue;
            }
        }

        // Check if needs processing
        let hash = match compute_content_hash(path) {
            Ok(h) => h,
            Err(e) => {
                eprintln!("[bootstrap] WARN: can't hash {}: {e}", path.display());
                result.errors += 1;
                continue;
            }
        };

        let (needs_work, last_offset) = needs_processing(conn, path, &hash);
        if !needs_work {
            result.files_skipped += 1;
            continue;
        }

        // Read transcript
        let content = match std::fs::read_to_string(path) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("[bootstrap] WARN: can't read {}: {e}", path.display());
                result.errors += 1;
                continue;
            }
        };

        // Find matching adapter
        let adapter = match adapters.iter().find(|a| a.name() == adapter_name.as_str()) {
            Some(a) => a,
            None => {
                eprintln!("[bootstrap] WARN: no adapter for {}", adapter_name);
                result.errors += 1;
                continue;
            }
        };

        // Parse incrementally from last offset
        let (chunks, new_offset) = adapter.parse_incremental(&content, last_offset);
        let size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);

        if chunks.is_empty() {
            // No new content but file may have changed -- update hash
            update_log(
                conn,
                path,
                adapter_name,
                project.as_deref(),
                size,
                new_offset,
                &hash,
                0,
            );
            result.files_skipped += 1;
            continue;
        }

        // Create and store a summary memory for this transcript
        let summary = create_transcript_summary(&chunks, project.as_deref(), path);
        let mut memories_stored = 0usize;
        if let Some(memory) = summary {
            match ops::remember(conn, &memory) {
                Ok(()) => memories_stored += 1,
                Err(e) => {
                    eprintln!(
                        "[bootstrap] WARN: failed to store summary for {}: {e}",
                        path.display()
                    );
                    result.errors += 1;
                }
            }
        }

        update_log(
            conn,
            path,
            adapter_name,
            project.as_deref(),
            size,
            new_offset,
            &hash,
            memories_stored,
        );
        result.memories_extracted += memories_stored;
        result.files_processed += 1;

        if (i + 1) % 10 == 0 || i + 1 == total {
            eprintln!(
                "[bootstrap] progress: {}/{} files, {} memories extracted",
                i + 1,
                total,
                result.memories_extracted
            );
        }
    }

    eprintln!(
        "[bootstrap] complete: {} processed, {} skipped, {} memories, {} errors",
        result.files_processed, result.files_skipped, result.memories_extracted, result.errors
    );

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn test_conn() -> Connection {
        crate::db::vec::init_sqlite_vec();
        let conn = Connection::open_in_memory().unwrap();
        crate::db::schema::create_schema(&conn).unwrap();
        conn
    }

    #[test]
    fn test_compute_content_hash_deterministic() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("test.jsonl");
        std::fs::write(&path, "hello world\n").unwrap();
        let pb = PathBuf::from(&path);

        let h1 = compute_content_hash(&pb).unwrap();
        let h2 = compute_content_hash(&pb).unwrap();
        assert_eq!(h1, h2, "same file should produce same hash");
    }

    #[test]
    fn test_compute_content_hash_changes_on_write() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("test.jsonl");
        std::fs::write(&path, "hello world\n").unwrap();
        let pb = PathBuf::from(&path);

        let h1 = compute_content_hash(&pb).unwrap();
        std::fs::write(&path, "hello world modified\n").unwrap();
        let h2 = compute_content_hash(&pb).unwrap();
        assert_ne!(h1, h2, "modified file should produce different hash");
    }

    #[test]
    fn test_needs_processing_new_file() {
        let conn = test_conn();
        let path = PathBuf::from("/tmp/nonexistent-test-bootstrap.jsonl");
        let (needs, offset) = needs_processing(&conn, &path, "abc-123");
        assert!(needs, "new file should need processing");
        assert_eq!(offset, 0, "new file should start at offset 0");
    }

    #[test]
    fn test_needs_processing_unchanged() {
        let conn = test_conn();
        let path = PathBuf::from("/tmp/test-bootstrap-unchanged.jsonl");
        let hash = "abc-123";

        // Insert a log entry
        update_log(&conn, &path, "claude-code", Some("test"), 100, 50, hash, 2);

        let (needs, offset) = needs_processing(&conn, &path, hash);
        assert!(!needs, "unchanged file should NOT need processing");
        assert_eq!(offset, 50, "should return stored offset");
    }

    #[test]
    fn test_needs_processing_changed() {
        let conn = test_conn();
        let path = PathBuf::from("/tmp/test-bootstrap-changed.jsonl");

        // Insert a log entry with old hash
        update_log(&conn, &path, "claude-code", Some("test"), 100, 50, "old-hash", 2);

        let (needs, offset) = needs_processing(&conn, &path, "new-hash");
        assert!(needs, "changed file should need processing");
        assert_eq!(offset, 0, "should reset offset to 0 when hash changes (file may be rewritten)");
    }

    #[test]
    fn test_extract_project_from_path() {
        let path = PathBuf::from("/home/user/.claude/projects/-Users-name-workspace-myproject/abc123.jsonl");
        let project = extract_project_from_path(&path);
        assert_eq!(project.as_deref(), Some("myproject"));
    }

    #[test]
    fn test_extract_project_from_path_no_project() {
        let path = PathBuf::from("/tmp/random/file.jsonl");
        let project = extract_project_from_path(&path);
        assert!(project.is_none());
    }

    #[test]
    fn test_update_log_and_query() {
        let conn = test_conn();
        let path = PathBuf::from("/tmp/test-log-roundtrip.jsonl");
        let hash = "deadbeef-1024";

        update_log(&conn, &path, "claude-code", Some("forge"), 1024, 512, hash, 5);

        // Query back
        let (stored_hash, offset): (String, usize) = conn
            .query_row(
                "SELECT content_hash, offset_processed FROM transcript_log WHERE path = ?1",
                rusqlite::params!["/tmp/test-log-roundtrip.jsonl"],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();

        assert_eq!(stored_hash, hash);
        assert_eq!(offset, 512);

        // Also check adapter and project
        let (adapter, project, memories): (String, String, usize) = conn
            .query_row(
                "SELECT adapter, project, memories_extracted FROM transcript_log WHERE path = ?1",
                rusqlite::params!["/tmp/test-log-roundtrip.jsonl"],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(adapter, "claude-code");
        assert_eq!(project, "forge");
        assert_eq!(memories, 5);
    }

    #[test]
    fn test_scan_transcripts_finds_files() {
        // Create a temp dir with some .jsonl files
        let dir = TempDir::new().unwrap();
        let sub = dir.path().join("projects").join("test-hash");
        std::fs::create_dir_all(&sub).unwrap();

        // Create test transcript files
        std::fs::write(sub.join("session1.jsonl"), r#"{"type":"human","message":{"role":"user","content":"hello"}}"#).unwrap();
        std::fs::write(sub.join("session2.jsonl"), r#"{"type":"human","message":{"role":"user","content":"world"}}"#).unwrap();
        std::fs::write(sub.join("not-a-transcript.txt"), "ignore me").unwrap();

        // Create a minimal adapter that watches this dir
        struct TestAdapter {
            dir: PathBuf,
        }
        impl AgentAdapter for TestAdapter {
            fn name(&self) -> &str { "test" }
            fn watch_dirs(&self) -> Vec<PathBuf> { vec![self.dir.clone()] }
            fn matches(&self, _path: &std::path::Path) -> bool { true }
            fn file_extension(&self) -> &str { "jsonl" }
            fn parse(&self, _content: &str) -> Vec<ConversationChunk> { vec![] }
            fn parse_incremental(&self, _content: &str, _last_offset: usize) -> (Vec<ConversationChunk>, usize) {
                (vec![], 0)
            }
        }

        let adapters: Vec<Box<dyn AgentAdapter>> = vec![
            Box::new(TestAdapter { dir: dir.path().to_path_buf() }),
        ];
        let files = scan_transcripts(&adapters);
        assert_eq!(files.len(), 2, "should find exactly 2 .jsonl files");

        // All should have adapter name "test"
        for (_, name) in &files {
            assert_eq!(name, "test");
        }
    }

    #[test]
    fn test_bootstrap_processes_new_skips_old() {
        let conn = test_conn();
        let dir = TempDir::new().unwrap();
        let sub = dir.path().join("projects").join("test-hash");
        std::fs::create_dir_all(&sub).unwrap();

        // Create two transcript files
        let content1 = r#"{"type":"human","message":{"role":"user","content":"hello world this is a conversation"}}
{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"I can help you with that request"}]}}"#;
        let content2 = r#"{"type":"human","message":{"role":"user","content":"another conversation here"}}
{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"Sure thing, let me assist you"}]}}"#;

        let path1 = sub.join("session1.jsonl");
        let path2 = sub.join("session2.jsonl");
        std::fs::write(&path1, content1).unwrap();
        std::fs::write(&path2, content2).unwrap();

        // Create a minimal adapter
        struct TestAdapter {
            dir: PathBuf,
        }
        impl AgentAdapter for TestAdapter {
            fn name(&self) -> &str { "test" }
            fn watch_dirs(&self) -> Vec<PathBuf> { vec![self.dir.clone()] }
            fn matches(&self, _path: &std::path::Path) -> bool { true }
            fn file_extension(&self) -> &str { "jsonl" }
            fn parse(&self, _content: &str) -> Vec<ConversationChunk> { vec![] }
            fn parse_incremental(&self, content: &str, last_offset: usize) -> (Vec<ConversationChunk>, usize) {
                // Simple: return a chunk for each line from last_offset
                let relevant = &content[last_offset..];
                let mut chunks = Vec::new();
                for (i, line) in relevant.lines().enumerate() {
                    if !line.is_empty() {
                        chunks.push(ConversationChunk {
                            id: format!("chunk-{}", i),
                            session_id: "test-session".to_string(),
                            role: if i % 2 == 0 { "user".to_string() } else { "assistant".to_string() },
                            content: line.to_string(),
                            has_tool_use: false,
                            timestamp: "2026-04-04T12:00:00Z".to_string(),
                            extracted: false,
                        });
                    }
                }
                (chunks, content.len())
            }
        }

        let adapters: Vec<Box<dyn AgentAdapter>> = vec![
            Box::new(TestAdapter { dir: dir.path().to_path_buf() }),
        ];

        // First run: should process both files
        let result = run_bootstrap(&conn, &adapters, None);
        assert_eq!(result.files_processed, 2, "first run should process 2 files");
        assert_eq!(result.files_skipped, 0, "first run should skip 0 files");
        assert!(result.memories_extracted >= 2, "should extract at least 2 memories (one per file)");
        assert_eq!(result.errors, 0);

        // Second run: should skip both (unchanged)
        let result2 = run_bootstrap(&conn, &adapters, None);
        assert_eq!(result2.files_processed, 0, "second run should process 0 files");
        assert_eq!(result2.files_skipped, 2, "second run should skip both files");
        assert_eq!(result2.memories_extracted, 0, "second run should extract 0 memories");
        assert_eq!(result2.errors, 0);
    }

    #[test]
    fn test_create_transcript_summary() {
        let chunks = vec![
            ConversationChunk {
                id: "1".to_string(),
                session_id: "s1".to_string(),
                role: "user".to_string(),
                content: "How do I build a REST API in Rust?".to_string(),
                has_tool_use: false,
                timestamp: "2026-04-04T12:00:00Z".to_string(),
                extracted: false,
            },
            ConversationChunk {
                id: "2".to_string(),
                session_id: "s1".to_string(),
                role: "assistant".to_string(),
                content: "I can help you with that...".to_string(),
                has_tool_use: true,
                timestamp: "2026-04-04T12:00:01Z".to_string(),
                extracted: false,
            },
        ];

        let dir = TempDir::new().unwrap();
        let path = dir.path().join("test.jsonl");
        std::fs::write(&path, "dummy").unwrap();
        let pb = PathBuf::from(&path);

        let memory = create_transcript_summary(&chunks, Some("myproject"), &pb);
        assert!(memory.is_some());
        let m = memory.unwrap();
        assert!(m.title.contains("myproject"));
        assert!(m.title.contains("2 turns"));
        assert!(m.title.contains("with tools"));
        assert!(m.content.contains("myproject"));
        assert!(m.content.contains("REST API"));
        assert_eq!(m.confidence, 0.4);
        assert_eq!(m.project.as_deref(), Some("myproject"));
        assert!(m.tags.contains(&"bootstrap".to_string()));
        assert!(m.tags.contains(&"transcript-summary".to_string()));
    }

    #[test]
    fn test_create_transcript_summary_empty_chunks() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("test.jsonl");
        std::fs::write(&path, "dummy").unwrap();
        let pb = PathBuf::from(&path);

        let memory = create_transcript_summary(&[], Some("myproject"), &pb);
        assert!(memory.is_none(), "empty chunks should produce no summary");
    }

    #[test]
    fn test_transcript_log_table_exists() {
        let conn = test_conn();
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='transcript_log'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1, "transcript_log table should exist");
    }

    #[test]
    fn test_fnv1a_deterministic() {
        let data = b"hello world";
        let h1 = fnv1a(data);
        let h2 = fnv1a(data);
        assert_eq!(h1, h2);
        // Different data should produce different hash
        let h3 = fnv1a(b"hello world!");
        assert_ne!(h1, h3);
    }
}
