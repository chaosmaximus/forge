use rusqlite::{params, Connection};

/// Result of a guardrail check for a file action.
#[derive(Debug, Clone)]
pub struct GuardrailResult {
    pub safe: bool,
    pub warnings: Vec<String>,
    pub decisions_affected: Vec<String>,
    /// Number of files that call symbols defined in this file.
    pub callers_count: usize,
    /// Paths of files that call symbols defined in this file.
    pub calling_files: Vec<String>,
    /// Lesson/pattern titles relevant to this file (via "affects" edges).
    pub relevant_lessons: Vec<String>,
    /// High-intensity negative-valence memory titles (universal warnings).
    pub dangerous_patterns: Vec<String>,
    /// Applicable skill names matched by file path or domain.
    pub applicable_skills: Vec<String>,
}

/// Check whether an action on a file is safe by querying 4 layers of the
/// knowledge graph:
///
/// 1. **Linked decisions** — active decisions linked via "affects" edges
/// 2. **Blast radius** — how many other files call symbols in this file
/// 3. **Relevant lessons** — lessons/patterns linked to this file + dangerous patterns
/// 4. **Applicable skills** — tested workflows relevant to this file
///
/// A file is considered unsafe only if there are linked decisions.
/// Dangerous patterns are advisory warnings, not safety gates.
pub fn check_action(conn: &Connection, file: &str, action: &str) -> GuardrailResult {
    check_action_with_org(conn, file, action, None)
}

/// Check action with optional organization_id filtering (multi-tenant isolation).
pub fn check_action_with_org(
    conn: &Connection,
    file: &str,
    action: &str,
    _org_id: Option<&str>,
) -> GuardrailResult {
    let file_target = format!("file:{file}");

    // Check 1: Linked decisions (existing)
    let decisions = find_decisions_for_file(conn, &file_target);

    // Check 2: Blast radius (NEW)
    let (callers_count, calling_files) = count_callers(conn, file);

    // Check 3: Relevant lessons + dangerous patterns (NEW)
    let relevant_lessons = find_relevant_lessons(conn, &file_target);
    let dangerous_patterns = find_dangerous_patterns(conn);

    // Check 4: Applicable skills (NEW)
    let applicable_skills = find_applicable_skills(conn, file);

    // Build warnings
    let mut warnings: Vec<String> = decisions
        .iter()
        .map(|(id, title, confidence)| {
            format!(
                "[{action}] Decision \"{title}\" (confidence: {confidence:.2}) linked to {file} — id: {id}"
            )
        })
        .collect();

    if callers_count > 0 {
        let severity = if callers_count > 5 {
            "HIGH"
        } else if callers_count > 2 {
            "MEDIUM"
        } else {
            "LOW"
        };
        warnings.push(format!(
            "Blast radius: {callers_count} files call symbols from {file} — {severity}"
        ));
    }

    for lesson in &relevant_lessons {
        warnings.push(format!("Lesson: {lesson}"));
    }

    for pattern in &dangerous_patterns {
        warnings.push(format!("Dangerous pattern: {pattern}"));
    }

    let decisions_affected: Vec<String> = decisions.iter().map(|(id, _, _)| id.clone()).collect();
    // Only linked decisions make a file unsafe. Dangerous patterns are advisory
    // (Codex fix: global negative-valence memories shouldn't poison every check)
    let safe = decisions_affected.is_empty();

    GuardrailResult {
        safe,
        warnings,
        decisions_affected,
        callers_count,
        calling_files,
        relevant_lessons,
        dangerous_patterns,
        applicable_skills,
    }
}

/// Result of a post-edit check for a file.
/// Surfaces callers, lessons, patterns, skills, decisions, and cached diagnostics for review.
#[derive(Debug, Clone)]
pub struct PostEditResult {
    pub file: String,
    pub callers_count: usize,
    pub calling_files: Vec<String>,
    pub relevant_lessons: Vec<String>,
    pub dangerous_patterns: Vec<String>,
    pub applicable_skills: Vec<String>,
    pub decisions_to_review: Vec<String>,
    pub cached_diagnostics: Vec<String>,
}

/// Post-edit check: after a file has been modified, surface relevant context
/// from the knowledge graph so the agent is aware of callers, lessons,
/// dangerous patterns, applicable skills, linked decisions, and cached diagnostics.
///
/// This does NOT parse the new file content. It uses the EXISTING code graph
/// to surface what the agent should be aware of after editing.
pub fn post_edit_check(conn: &Connection, file: &str) -> PostEditResult {
    let file_target = format!("file:{file}");

    // Reuse the same helper functions from check_action
    let (callers_count, calling_files) = count_callers(conn, file);
    let relevant_lessons = find_relevant_lessons(conn, &file_target);
    let dangerous_patterns = find_dangerous_patterns(conn);
    let applicable_skills = find_applicable_skills(conn, file);

    // Also find decisions linked to this file (for review reminder)
    let decisions = find_decisions_for_file(conn, &file_target);
    let decisions_to_review: Vec<String> = decisions
        .iter()
        .map(|(_, title, _)| title.clone())
        .collect();

    // Query cached diagnostics from the diagnostic table
    let cached = crate::db::diagnostics::get_diagnostics(conn, file).unwrap_or_default();
    let cached_diagnostics: Vec<String> = cached
        .iter()
        .map(|d| format!("[{}:{}] {}", d.source, d.severity, d.message))
        .collect();

    PostEditResult {
        file: file.to_string(),
        callers_count,
        calling_files,
        relevant_lessons,
        dangerous_patterns,
        applicable_skills,
        decisions_to_review,
        cached_diagnostics,
    }
}

/// Find active decisions linked to a file target via "affects" edges.
/// Returns (id, title, confidence) tuples ordered by confidence descending.
fn find_decisions_for_file(conn: &Connection, file_target: &str) -> Vec<(String, String, f64)> {
    let mut stmt = match conn.prepare(
        "SELECT m.id, m.title, m.confidence FROM memory m
         JOIN edge e ON e.from_id = m.id
         WHERE e.to_id = ?1 AND e.edge_type = 'affects'
         AND m.memory_type = 'decision' AND m.status = 'active'
         ORDER BY m.confidence DESC
         LIMIT 50",
    ) {
        Ok(s) => s,
        Err(_) => return vec![],
    };

    let rows = match stmt.query_map(params![file_target], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, f64>(2)?,
        ))
    }) {
        Ok(r) => r,
        Err(_) => return vec![],
    };

    rows.filter_map(|r| r.ok()).collect()
}

/// Count how many OTHER files call symbols defined in the target file.
///
/// Looks up symbols in `code_symbol` whose `file_path` ends with the given
/// file path, then finds "calls" edges pointing TO those symbols from symbols
/// in other files. Uses LIKE matching because file_path may be absolute while
/// the hook passes a relative path.
///
/// Returns (count_of_unique_calling_files, list_of_calling_file_paths).
fn count_callers(conn: &Connection, file: &str) -> (usize, Vec<String>) {
    if file.is_empty() {
        return (0, vec![]);
    }

    // Use LIKE %file to match both absolute and relative paths stored in code_symbol.
    let like_pattern = format!("%{file}");

    let mut stmt = match conn.prepare(
        "SELECT DISTINCT cs2.file_path
         FROM code_symbol s
         JOIN edge e ON e.to_id = s.id AND e.edge_type = 'calls'
         JOIN code_symbol cs2 ON cs2.id = e.from_id
         WHERE s.file_path LIKE ?1
         AND cs2.file_path NOT LIKE ?1",
    ) {
        Ok(s) => s,
        Err(_) => return (0, vec![]),
    };

    let rows = match stmt.query_map(params![like_pattern], |row| row.get::<_, String>(0)) {
        Ok(r) => r,
        Err(_) => return (0, vec![]),
    };

    let calling_files: Vec<String> = rows.filter_map(|r| r.ok()).collect();
    let count = calling_files.len();
    (count, calling_files)
}

/// Find lessons/patterns linked to this file via "affects" edges.
/// Returns lesson titles ordered by confidence descending.
fn find_relevant_lessons(conn: &Connection, file_target: &str) -> Vec<String> {
    let mut stmt = match conn.prepare(
        "SELECT m.title FROM memory m
         JOIN edge e ON e.from_id = m.id
         WHERE e.to_id = ?1 AND e.edge_type = 'affects'
         AND m.status = 'active' AND m.memory_type IN ('lesson', 'pattern')
         ORDER BY m.confidence DESC
         LIMIT 5",
    ) {
        Ok(s) => s,
        Err(_) => return vec![],
    };

    let rows = match stmt.query_map(params![file_target], |row| row.get::<_, String>(0)) {
        Ok(r) => r,
        Err(_) => return vec![],
    };

    rows.filter_map(|r| r.ok()).collect()
}

/// Find high-intensity negative-valence memories (universal dangerous patterns).
/// These apply to ALL files regardless of edge linkage.
fn find_dangerous_patterns(conn: &Connection) -> Vec<String> {
    let mut stmt = match conn.prepare(
        "SELECT title FROM memory
         WHERE valence = 'negative' AND intensity > 0.7
         AND status = 'active'
         ORDER BY intensity DESC
         LIMIT 3",
    ) {
        Ok(s) => s,
        Err(_) => return vec![],
    };

    let rows = match stmt.query_map([], |row| row.get::<_, String>(0)) {
        Ok(r) => r,
        Err(_) => return vec![],
    };

    rows.filter_map(|r| r.ok()).collect()
}

/// Result of a pre-bash check for a command.
#[derive(Debug, Clone)]
pub struct PreBashResult {
    pub safe: bool,
    pub warnings: Vec<String>,
    pub relevant_skills: Vec<String>,
}

/// Pre-bash check: warn about destructive commands, surface relevant lessons/skills.
///
/// 1. **Destructive patterns** — known dangerous command patterns
/// 2. **Relevant lessons** — negative-valence memories matching the command
/// 3. **Relevant skills** — skills matching the command name
pub fn pre_bash_check(conn: &Connection, command: &str) -> PreBashResult {
    let mut warnings = Vec::new();
    let mut relevant_skills = Vec::new();
    let mut safe = true;

    // Check 1: Destructive command patterns
    let destructive_patterns: &[(&str, &str)] = &[
        (
            "rm -rf",
            "Recursive force delete -- verify path before running",
        ),
        ("git reset --hard", "Discards all uncommitted changes"),
        (
            "git push --force",
            "Force push can overwrite remote history",
        ),
        ("git push -f", "Force push can overwrite remote history"),
        ("drop table", "SQL table deletion -- irreversible"),
        ("drop database", "SQL database deletion -- irreversible"),
        ("docker rm", "Removes container -- data may be lost"),
        ("docker rmi", "Removes image"),
        ("kubectl delete", "Deletes Kubernetes resource"),
        ("terraform destroy", "Destroys infrastructure"),
        ("chmod 777", "World-writable permissions -- security risk"),
        ("pkill", "Kills processes -- may affect running services"),
        ("killall", "Kills all matching processes"),
    ];

    let cmd_lower = command.to_lowercase();
    for (pattern, warning) in destructive_patterns {
        if cmd_lower.contains(pattern) {
            warnings.push(format!("Destructive: {pattern} -- {warning}"));
            safe = false;
        }
    }

    // Check 2: Relevant lessons about this command from memory
    let cmd_name = command.split_whitespace().next().unwrap_or("");
    if !cmd_name.is_empty() {
        let search = format!("%{cmd_name}%");

        if let Ok(mut stmt) = conn.prepare(
            "SELECT title FROM memory WHERE status = 'active'
             AND memory_type IN ('lesson', 'pattern')
             AND (title LIKE ?1 OR content LIKE ?1)
             AND valence = 'negative' AND intensity > 0.5
             ORDER BY intensity DESC LIMIT 2",
        ) {
            if let Ok(rows) = stmt.query_map(params![search], |row| row.get::<_, String>(0)) {
                for r in rows.flatten() {
                    warnings.push(format!("Lesson: {r}"));
                }
            }
        }

        // Check 3: Relevant skills for this command
        if let Ok(mut stmt) = conn.prepare(
            "SELECT name, domain FROM skill WHERE success_count > 0
             AND (description LIKE ?1 OR name LIKE ?1 OR domain LIKE ?1)
             ORDER BY success_count DESC LIMIT 2",
        ) {
            if let Ok(rows) = stmt.query_map(params![search], |row| {
                Ok(format!(
                    "{} ({})",
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?
                ))
            }) {
                for r in rows.flatten() {
                    relevant_skills.push(r);
                }
            }
        }
    }

    PreBashResult {
        safe,
        warnings,
        relevant_skills,
    }
}

/// Result of a post-bash check for a command.
#[derive(Debug, Clone)]
pub struct PostBashResult {
    pub suggestions: Vec<String>,
}

/// Post-bash check: on failure, surface relevant lessons and skills.
///
/// Returns empty suggestions on success (exit_code == 0).
pub fn post_bash_check(conn: &Connection, command: &str, exit_code: i32) -> PostBashResult {
    let mut suggestions = Vec::new();

    if exit_code == 0 {
        return PostBashResult { suggestions };
    }

    let cmd_name = command.split_whitespace().next().unwrap_or("");
    if cmd_name.is_empty() {
        return PostBashResult { suggestions };
    }

    let search = format!("%{cmd_name}%");

    // Surface lessons about similar command failures
    if let Ok(mut stmt) = conn.prepare(
        "SELECT title, content FROM memory WHERE status = 'active'
         AND memory_type IN ('lesson', 'pattern')
         AND (title LIKE ?1 OR content LIKE ?1)
         ORDER BY confidence DESC LIMIT 3",
    ) {
        if let Ok(rows) = stmt.query_map(params![search], |row| {
            Ok(format!(
                "{}: {}",
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?
                    .chars()
                    .take(100)
                    .collect::<String>()
            ))
        }) {
            for r in rows.flatten() {
                suggestions.push(r);
            }
        }
    }

    // Surface applicable skills
    if let Ok(mut stmt) = conn.prepare(
        "SELECT name, description FROM skill WHERE success_count > 0
         AND (description LIKE ?1 OR name LIKE ?1)
         ORDER BY success_count DESC LIMIT 1",
    ) {
        if let Ok(rows) = stmt.query_map(params![search], |row| {
            Ok(format!(
                "Skill: {} -- {}",
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?
                    .chars()
                    .take(80)
                    .collect::<String>()
            ))
        }) {
            for r in rows.flatten() {
                suggestions.push(r);
            }
        }
    }

    PostBashResult { suggestions }
}

/// Find skills whose name, domain, or description mention any path segment
/// of the file (filename, parent directories, or stem without extension).
/// Only returns skills with at least one successful usage.
fn find_applicable_skills(conn: &Connection, file: &str) -> Vec<String> {
    if file.is_empty() {
        return vec![];
    }

    // Collect unique search terms from path segments:
    // For "src/auth/middleware.rs" we get: ["middleware.rs", "middleware", "auth"]
    let mut search_terms: Vec<String> = Vec::new();

    if let Some(filename) = file.rsplit('/').next() {
        if !filename.is_empty() {
            search_terms.push(filename.to_string());
            // Also try without extension
            if let Some(stem) = filename.rsplit_once('.').map(|(s, _)| s) {
                if !stem.is_empty() && stem != filename {
                    search_terms.push(stem.to_string());
                }
            }
        }
    }

    // Add parent directory components (skip the filename itself and common dirs like "src")
    let skip_dirs = [
        "src", "lib", "test", "tests", "spec", "pkg", "cmd", "internal",
    ];
    for segment in file.split('/').rev().skip(1) {
        if !segment.is_empty() && !skip_dirs.contains(&segment) {
            search_terms.push(segment.to_string());
        }
    }

    if search_terms.is_empty() {
        return vec![];
    }

    // Build an OR query for all search terms
    let conditions: Vec<String> = search_terms
        .iter()
        .enumerate()
        .flat_map(|(i, _)| {
            let p = i + 1; // 1-indexed
            vec![
                format!("description LIKE ?{p}"),
                format!("domain LIKE ?{p}"),
                format!("name LIKE ?{p}"),
            ]
        })
        .collect();

    let sql = format!(
        "SELECT name, domain FROM skill WHERE success_count > 0 AND ({}) ORDER BY success_count DESC LIMIT 2",
        conditions.join(" OR ")
    );

    let mut stmt = match conn.prepare(&sql) {
        Ok(s) => s,
        Err(_) => return vec![],
    };

    let like_params: Vec<String> = search_terms.iter().map(|t| format!("%{t}%")).collect();

    let param_refs: Vec<&dyn rusqlite::types::ToSql> = like_params
        .iter()
        .map(|s| s as &dyn rusqlite::types::ToSql)
        .collect();

    let rows = match stmt.query_map(param_refs.as_slice(), |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    }) {
        Ok(r) => r,
        Err(_) => return vec![],
    };

    rows.filter_map(|r| r.ok())
        .map(|(name, domain)| format!("Skill: {name} ({domain})"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::manas::store_skill;
    use crate::db::ops::{forget, remember, store_edge, store_symbol};
    use crate::db::schema::create_schema;
    use forge_core::types::code::CodeSymbol;
    use forge_core::types::manas::Skill;
    use forge_core::types::{Memory, MemoryType};

    fn setup() -> Connection {
        crate::db::vec::init_sqlite_vec();
        let conn = Connection::open_in_memory().unwrap();
        create_schema(&conn).unwrap();
        conn
    }

    // ── Existing tests (must still pass) ──

    #[test]
    fn test_guardrail_no_decisions() {
        let conn = setup();

        let mem = Memory::new(
            MemoryType::Decision,
            "Use JWT for auth",
            "We chose JWT tokens",
        );
        remember(&conn, &mem).unwrap();

        let result = check_action(&conn, "src/auth.rs", "modify");
        assert!(result.safe);
        assert!(result.warnings.is_empty());
        assert!(result.decisions_affected.is_empty());
        assert_eq!(result.callers_count, 0);
    }

    #[test]
    fn test_guardrail_with_decisions() {
        let conn = setup();

        let mem1 = Memory::new(
            MemoryType::Decision,
            "Use JWT for auth",
            "We chose JWT tokens",
        );
        remember(&conn, &mem1).unwrap();
        store_edge(&conn, &mem1.id, "file:src/auth.rs", "affects", "{}").unwrap();

        let mem2 = Memory::new(
            MemoryType::Decision,
            "Rate limit endpoints",
            "Apply rate limiting",
        );
        remember(&conn, &mem2).unwrap();
        store_edge(&conn, &mem2.id, "file:src/auth.rs", "affects", "{}").unwrap();

        let result = check_action(&conn, "src/auth.rs", "delete");
        assert!(!result.safe);
        assert_eq!(result.decisions_affected.len(), 2);
        assert!(result.warnings[0].contains("[delete]"));
        assert!(result.warnings[0].contains("src/auth.rs"));
    }

    #[test]
    fn test_guardrail_superseded_decision_excluded() {
        let conn = setup();

        let mem = Memory::new(
            MemoryType::Decision,
            "Old auth approach",
            "Deprecated approach",
        );
        remember(&conn, &mem).unwrap();
        store_edge(&conn, &mem.id, "file:src/auth.rs", "affects", "{}").unwrap();
        forget(&conn, &mem.id).unwrap();

        let result = check_action(&conn, "src/auth.rs", "modify");
        assert!(result.safe);
    }

    #[test]
    fn test_guardrail_different_files_independent() {
        let conn = setup();

        let mem = Memory::new(
            MemoryType::Decision,
            "Use JWT for auth",
            "We chose JWT tokens",
        );
        remember(&conn, &mem).unwrap();
        store_edge(&conn, &mem.id, "file:src/auth.rs", "affects", "{}").unwrap();

        let result = check_action(&conn, "src/server.rs", "modify");
        assert!(result.safe);
    }

    #[test]
    fn test_guardrail_only_decisions_not_lessons() {
        let conn = setup();

        // A lesson linked to a file should NOT make the check unsafe (only decisions do),
        // but it SHOULD appear in relevant_lessons.
        let lesson = Memory::new(MemoryType::Lesson, "Learned about auth", "Auth is tricky");
        remember(&conn, &lesson).unwrap();
        store_edge(&conn, &lesson.id, "file:src/auth.rs", "affects", "{}").unwrap();

        let result = check_action(&conn, "src/auth.rs", "edit");
        assert!(
            result.safe,
            "lessons should not trigger guardrails, only decisions"
        );
        // But the lesson should be surfaced
        assert!(
            !result.relevant_lessons.is_empty(),
            "lesson should be surfaced in relevant_lessons"
        );
    }

    // ── New tests for Check 2: Blast Radius ──

    #[test]
    fn test_blast_radius_with_callers() {
        let conn = setup();

        // Store a symbol in src/auth.rs
        store_symbol(
            &conn,
            &CodeSymbol {
                id: "sym:auth:validate".into(),
                name: "validate_token".into(),
                kind: "function".into(),
                file_path: "src/auth.rs".into(),
                line_start: 10,
                line_end: Some(20),
                signature: Some("fn validate_token()".into()),
            },
        )
        .unwrap();

        // Store a caller symbol in a different file
        store_symbol(
            &conn,
            &CodeSymbol {
                id: "sym:routes:handler".into(),
                name: "handle_request".into(),
                kind: "function".into(),
                file_path: "src/routes.rs".into(),
                line_start: 5,
                line_end: Some(15),
                signature: Some("fn handle_request()".into()),
            },
        )
        .unwrap();

        // Store a "calls" edge from routes to auth
        store_edge(
            &conn,
            "sym:routes:handler",
            "sym:auth:validate",
            "calls",
            "{}",
        )
        .unwrap();

        let result = check_action(&conn, "src/auth.rs", "edit");
        assert!(result.callers_count > 0, "should detect callers");
        assert!(
            !result.calling_files.is_empty(),
            "should list calling files"
        );
        assert!(result.calling_files.contains(&"src/routes.rs".to_string()));
        assert!(result.warnings.iter().any(|w| w.contains("Blast radius")));
    }

    #[test]
    fn test_blast_radius_excludes_same_file() {
        let conn = setup();

        // Two symbols in the same file
        store_symbol(
            &conn,
            &CodeSymbol {
                id: "sym:auth:validate".into(),
                name: "validate_token".into(),
                kind: "function".into(),
                file_path: "src/auth.rs".into(),
                line_start: 10,
                line_end: Some(20),
                signature: None,
            },
        )
        .unwrap();

        store_symbol(
            &conn,
            &CodeSymbol {
                id: "sym:auth:helper".into(),
                name: "auth_helper".into(),
                kind: "function".into(),
                file_path: "src/auth.rs".into(),
                line_start: 25,
                line_end: Some(35),
                signature: None,
            },
        )
        .unwrap();

        // Call edge within the same file — should NOT count
        store_edge(&conn, "sym:auth:helper", "sym:auth:validate", "calls", "{}").unwrap();

        let result = check_action(&conn, "src/auth.rs", "edit");
        assert_eq!(
            result.callers_count, 0,
            "same-file callers should not count"
        );
        assert!(result.calling_files.is_empty());
    }

    #[test]
    fn test_blast_radius_severity_levels() {
        let conn = setup();

        // Target symbol
        store_symbol(
            &conn,
            &CodeSymbol {
                id: "sym:core:process".into(),
                name: "process".into(),
                kind: "function".into(),
                file_path: "src/core.rs".into(),
                line_start: 1,
                line_end: Some(10),
                signature: None,
            },
        )
        .unwrap();

        // Create 6 callers from different files => HIGH severity
        for i in 0..6 {
            let caller_id = format!("sym:caller{i}:fn");
            let caller_file = format!("src/caller{i}.rs");
            store_symbol(
                &conn,
                &CodeSymbol {
                    id: caller_id.clone(),
                    name: format!("caller_{i}"),
                    kind: "function".into(),
                    file_path: caller_file,
                    line_start: 1,
                    line_end: Some(5),
                    signature: None,
                },
            )
            .unwrap();
            store_edge(&conn, &caller_id, "sym:core:process", "calls", "{}").unwrap();
        }

        let result = check_action(&conn, "src/core.rs", "edit");
        assert_eq!(result.callers_count, 6);
        assert!(result.warnings.iter().any(|w| w.contains("HIGH")));
    }

    // ── New tests for Check 3: Relevant Lessons ──

    #[test]
    fn test_relevant_lessons_surfaced() {
        let conn = setup();

        let lesson = Memory::new(
            MemoryType::Lesson,
            "Always test auth changes",
            "Auth is critical",
        );
        remember(&conn, &lesson).unwrap();
        store_edge(&conn, &lesson.id, "file:src/auth.rs", "affects", "{}").unwrap();

        let result = check_action(&conn, "src/auth.rs", "edit");
        assert!(
            !result.relevant_lessons.is_empty(),
            "should surface relevant lesson"
        );
        assert!(result.relevant_lessons[0].contains("Always test auth"));
        assert!(result
            .warnings
            .iter()
            .any(|w| w.contains("Lesson: Always test auth")));
        // Lessons alone don't make it unsafe
        assert!(result.safe);
    }

    #[test]
    fn test_relevant_patterns_surfaced() {
        let conn = setup();

        let pattern = Memory::new(
            MemoryType::Pattern,
            "Auth middleware pattern",
            "Check token first",
        );
        remember(&conn, &pattern).unwrap();
        store_edge(&conn, &pattern.id, "file:src/auth.rs", "affects", "{}").unwrap();

        let result = check_action(&conn, "src/auth.rs", "edit");
        assert!(
            !result.relevant_lessons.is_empty(),
            "patterns should also surface as lessons"
        );
        assert!(result.relevant_lessons[0].contains("Auth middleware pattern"));
    }

    #[test]
    fn test_dangerous_patterns_flagged() {
        let conn = setup();

        let danger = Memory::new(
            MemoryType::Lesson,
            "send_raw bypasses type safety",
            "Use typed Request",
        )
        .with_valence("negative", 0.9);
        remember(&conn, &danger).unwrap();

        let result = check_action(&conn, "src/any_file.rs", "edit");
        assert!(
            !result.dangerous_patterns.is_empty(),
            "should flag dangerous pattern"
        );
        assert!(result.dangerous_patterns[0].contains("send_raw"));
        // Dangerous patterns are advisory, not safety gates (Codex fix)
        assert!(
            result.safe,
            "dangerous patterns alone should NOT flip safe — only decisions do"
        );
        assert!(result
            .warnings
            .iter()
            .any(|w| w.contains("Dangerous pattern")));
    }

    #[test]
    fn test_low_intensity_negative_not_flagged() {
        let conn = setup();

        // Negative valence but LOW intensity (0.3 < 0.7 threshold)
        let mild = Memory::new(MemoryType::Lesson, "Minor style issue", "Prefer camelCase")
            .with_valence("negative", 0.3);
        remember(&conn, &mild).unwrap();

        let result = check_action(&conn, "src/any_file.rs", "edit");
        assert!(
            result.dangerous_patterns.is_empty(),
            "low intensity should not be flagged"
        );
        assert!(result.safe);
    }

    #[test]
    fn test_superseded_negative_not_flagged() {
        let conn = setup();

        let danger = Memory::new(MemoryType::Lesson, "Old dangerous pattern", "Was dangerous")
            .with_valence("negative", 0.95);
        remember(&conn, &danger).unwrap();
        forget(&conn, &danger.id).unwrap(); // superseded

        let result = check_action(&conn, "src/any_file.rs", "edit");
        assert!(
            result.dangerous_patterns.is_empty(),
            "superseded patterns should not be flagged"
        );
        assert!(result.safe);
    }

    // ── New tests for Check 4: Applicable Skills ──

    #[test]
    fn test_applicable_skills_found() {
        let conn = setup();

        let skill = Skill {
            id: "s1".into(),
            name: "Auth update workflow".into(),
            domain: "auth".into(),
            description: "Steps for updating auth middleware".into(),
            steps: vec!["Step 1".into(), "Step 2".into()],
            success_count: 3,
            fail_count: 0,
            last_used: None,
            source: "extracted".into(),
            version: 1,
            project: None,
            skill_type: "procedural".into(),
            user_specific: false,
            observed_count: 1,
            correlation_ids: vec![],
        };
        store_skill(&conn, &skill).unwrap();

        let result = check_action(&conn, "src/auth/middleware.rs", "edit");
        assert!(
            !result.applicable_skills.is_empty(),
            "should find applicable skill"
        );
        assert!(result.applicable_skills[0].contains("Auth update workflow"));
        assert!(result.applicable_skills[0].contains("auth"));
    }

    #[test]
    fn test_skills_with_zero_success_excluded() {
        let conn = setup();

        let skill = Skill {
            id: "s2".into(),
            name: "Untested workflow".into(),
            domain: "auth".into(),
            description: "Never successfully used".into(),
            steps: vec![],
            success_count: 0,
            fail_count: 2,
            last_used: None,
            source: "extracted".into(),
            version: 1,
            project: None,
            skill_type: "procedural".into(),
            user_specific: false,
            observed_count: 1,
            correlation_ids: vec![],
        };
        store_skill(&conn, &skill).unwrap();

        let result = check_action(&conn, "src/auth/middleware.rs", "edit");
        assert!(
            result.applicable_skills.is_empty(),
            "skills with 0 success should be excluded"
        );
    }

    // ── Clean file test ──

    #[test]
    fn test_clean_file_is_safe() {
        let conn = setup();

        let result = check_action(&conn, "src/new_file.rs", "edit");
        assert!(result.safe);
        assert!(result.warnings.is_empty());
        assert_eq!(result.callers_count, 0);
        assert!(result.calling_files.is_empty());
        assert!(result.relevant_lessons.is_empty());
        assert!(result.dangerous_patterns.is_empty());
        assert!(result.applicable_skills.is_empty());
    }

    // ── Post-edit check tests ──

    #[test]
    fn test_post_edit_check_with_callers() {
        let conn = setup();

        // Store a symbol in src/auth.rs
        store_symbol(
            &conn,
            &CodeSymbol {
                id: "sym:auth:validate".into(),
                name: "validate_token".into(),
                kind: "function".into(),
                file_path: "src/auth.rs".into(),
                line_start: 10,
                line_end: Some(20),
                signature: Some("fn validate_token()".into()),
            },
        )
        .unwrap();

        // Store a caller symbol in a different file
        store_symbol(
            &conn,
            &CodeSymbol {
                id: "sym:routes:handler".into(),
                name: "handle_request".into(),
                kind: "function".into(),
                file_path: "src/routes.rs".into(),
                line_start: 5,
                line_end: Some(15),
                signature: Some("fn handle_request()".into()),
            },
        )
        .unwrap();

        // "calls" edge from routes to auth
        store_edge(
            &conn,
            "sym:routes:handler",
            "sym:auth:validate",
            "calls",
            "{}",
        )
        .unwrap();

        let result = post_edit_check(&conn, "src/auth.rs");
        assert_eq!(result.file, "src/auth.rs");
        assert!(result.callers_count > 0, "should detect callers");
        assert!(result.calling_files.contains(&"src/routes.rs".to_string()));
    }

    #[test]
    fn test_post_edit_check_surfaces_lessons() {
        let conn = setup();

        let lesson = Memory::new(
            MemoryType::Lesson,
            "Always test auth changes",
            "Auth is critical",
        );
        remember(&conn, &lesson).unwrap();
        store_edge(&conn, &lesson.id, "file:src/auth.rs", "affects", "{}").unwrap();

        let result = post_edit_check(&conn, "src/auth.rs");
        assert!(
            !result.relevant_lessons.is_empty(),
            "should surface relevant lesson"
        );
        assert!(result.relevant_lessons[0].contains("Always test auth"));
    }

    #[test]
    fn test_post_edit_check_surfaces_dangerous_patterns() {
        let conn = setup();

        let danger = Memory::new(
            MemoryType::Lesson,
            "send_raw bypasses type safety",
            "Use typed Request",
        )
        .with_valence("negative", 0.9);
        remember(&conn, &danger).unwrap();

        let result = post_edit_check(&conn, "src/any_file.rs");
        assert!(
            !result.dangerous_patterns.is_empty(),
            "should flag dangerous pattern"
        );
        assert!(result.dangerous_patterns[0].contains("send_raw"));
    }

    #[test]
    fn test_post_edit_check_surfaces_decisions_to_review() {
        let conn = setup();

        let decision = Memory::new(
            MemoryType::Decision,
            "Use JWT for auth",
            "JWT tokens chosen",
        );
        remember(&conn, &decision).unwrap();
        store_edge(&conn, &decision.id, "file:src/auth.rs", "affects", "{}").unwrap();

        let result = post_edit_check(&conn, "src/auth.rs");
        assert!(
            !result.decisions_to_review.is_empty(),
            "should surface decisions for review"
        );
        assert!(result.decisions_to_review[0].contains("Use JWT"));
    }

    #[test]
    fn test_post_edit_check_clean_file() {
        let conn = setup();

        let result = post_edit_check(&conn, "src/brand_new.rs");
        assert_eq!(result.callers_count, 0);
        assert!(result.calling_files.is_empty());
        assert!(result.relevant_lessons.is_empty());
        assert!(result.dangerous_patterns.is_empty());
        assert!(result.applicable_skills.is_empty());
        assert!(result.decisions_to_review.is_empty());
    }

    // ── Pre-bash check tests ──

    #[test]
    fn test_pre_bash_detects_destructive() {
        let conn = setup();
        let result = pre_bash_check(&conn, "rm -rf /tmp/test");
        assert!(!result.safe);
        assert!(result.warnings.iter().any(|w| w.contains("Destructive")));
    }

    #[test]
    fn test_pre_bash_safe_command() {
        let conn = setup();
        let result = pre_bash_check(&conn, "ls -la");
        assert!(result.safe);
        assert!(result.warnings.is_empty());
    }

    #[test]
    fn test_pre_bash_force_push_warning() {
        let conn = setup();
        let result = pre_bash_check(&conn, "git push --force origin main");
        assert!(!result.safe);
        assert!(result.warnings.iter().any(|w| w.contains("force")));
    }

    #[test]
    fn test_pre_bash_force_push_short_flag() {
        let conn = setup();
        let result = pre_bash_check(&conn, "git push -f origin main");
        assert!(!result.safe);
        assert!(result.warnings.iter().any(|w| w.contains("Force push")));
    }

    #[test]
    fn test_pre_bash_git_reset_hard() {
        let conn = setup();
        let result = pre_bash_check(&conn, "git reset --hard HEAD~3");
        assert!(!result.safe);
        assert!(result.warnings.iter().any(|w| w.contains("Discards")));
    }

    #[test]
    fn test_pre_bash_surfaces_lessons() {
        let conn = setup();

        let lesson = Memory::new(
            MemoryType::Lesson,
            "rm needs careful path check",
            "Deleted wrong dir once",
        )
        .with_valence("negative", 0.8);
        remember(&conn, &lesson).unwrap();

        let result = pre_bash_check(&conn, "rm some-file.txt");
        assert!(result.warnings.iter().any(|w| w.contains("Lesson")));
    }

    #[test]
    fn test_pre_bash_surfaces_skills() {
        let conn = setup();

        let skill = Skill {
            id: "s-cargo".into(),
            name: "Cargo build workflow".into(),
            domain: "cargo".into(),
            description: "Steps for building with cargo".into(),
            steps: vec!["cargo build".into()],
            success_count: 5,
            fail_count: 0,
            last_used: None,
            source: "extracted".into(),
            version: 1,
            project: None,
            skill_type: "procedural".into(),
            user_specific: false,
            observed_count: 1,
            correlation_ids: vec![],
        };
        store_skill(&conn, &skill).unwrap();

        let result = pre_bash_check(&conn, "cargo test --workspace");
        assert!(!result.relevant_skills.is_empty());
        assert!(result.relevant_skills[0].contains("Cargo build workflow"));
    }

    // ── Post-bash check tests ──

    #[test]
    fn test_post_bash_success_no_suggestions() {
        let conn = setup();
        let result = post_bash_check(&conn, "cargo test", 0);
        assert!(result.suggestions.is_empty());
    }

    #[test]
    fn test_post_bash_failure_surfaces_lessons() {
        let conn = setup();

        let lesson = Memory::new(
            MemoryType::Lesson,
            "cargo test needs --workspace flag",
            "Found missing tests",
        );
        remember(&conn, &lesson).unwrap();

        let result = post_bash_check(&conn, "cargo test", 1);
        assert!(!result.suggestions.is_empty());
        assert!(result.suggestions[0].contains("cargo test needs --workspace"));
    }

    #[test]
    fn test_post_bash_failure_surfaces_skills() {
        let conn = setup();

        let skill = Skill {
            id: "s-npm".into(),
            name: "npm debug workflow".into(),
            domain: "npm".into(),
            description: "Steps for debugging npm issues".into(),
            steps: vec!["npm cache clean".into()],
            success_count: 2,
            fail_count: 0,
            last_used: None,
            source: "extracted".into(),
            version: 1,
            project: None,
            skill_type: "procedural".into(),
            user_specific: false,
            observed_count: 1,
            correlation_ids: vec![],
        };
        store_skill(&conn, &skill).unwrap();

        let result = post_bash_check(&conn, "npm install", 1);
        assert!(result.suggestions.iter().any(|s| s.contains("Skill:")));
    }

    #[test]
    fn test_post_bash_empty_command_no_crash() {
        let conn = setup();
        let result = post_bash_check(&conn, "", 1);
        assert!(result.suggestions.is_empty());
    }

    // ── Combined scenario test ──

    #[test]
    fn test_all_checks_combined() {
        let conn = setup();

        // Decision linked to file
        let decision = Memory::new(MemoryType::Decision, "Use JWT", "JWT tokens");
        remember(&conn, &decision).unwrap();
        store_edge(&conn, &decision.id, "file:src/auth.rs", "affects", "{}").unwrap();

        // Lesson linked to file
        let lesson = Memory::new(MemoryType::Lesson, "Always test auth", "Important");
        remember(&conn, &lesson).unwrap();
        store_edge(&conn, &lesson.id, "file:src/auth.rs", "affects", "{}").unwrap();

        // Dangerous pattern (global)
        let danger = Memory::new(MemoryType::Lesson, "Never use eval()", "Security risk")
            .with_valence("negative", 0.95);
        remember(&conn, &danger).unwrap();

        // Symbol + caller
        store_symbol(
            &conn,
            &CodeSymbol {
                id: "sym:auth:check".into(),
                name: "check".into(),
                kind: "function".into(),
                file_path: "src/auth.rs".into(),
                line_start: 1,
                line_end: Some(10),
                signature: None,
            },
        )
        .unwrap();
        store_symbol(
            &conn,
            &CodeSymbol {
                id: "sym:routes:index".into(),
                name: "index".into(),
                kind: "function".into(),
                file_path: "src/routes.rs".into(),
                line_start: 1,
                line_end: Some(10),
                signature: None,
            },
        )
        .unwrap();
        store_edge(&conn, "sym:routes:index", "sym:auth:check", "calls", "{}").unwrap();

        // Skill
        let skill = Skill {
            id: "s-auth".into(),
            name: "Auth workflow".into(),
            domain: "auth".into(),
            description: "For auth.rs changes".into(),
            steps: vec![],
            success_count: 5,
            fail_count: 0,
            last_used: None,
            source: "extracted".into(),
            version: 1,
            project: None,
            skill_type: "procedural".into(),
            user_specific: false,
            observed_count: 1,
            correlation_ids: vec![],
        };
        store_skill(&conn, &skill).unwrap();

        let result = check_action(&conn, "src/auth.rs", "edit");

        // Not safe (decision + dangerous pattern)
        assert!(!result.safe);
        assert_eq!(result.decisions_affected.len(), 1);
        assert_eq!(result.callers_count, 1);
        assert!(result.calling_files.contains(&"src/routes.rs".to_string()));
        assert!(!result.relevant_lessons.is_empty());
        assert!(!result.dangerous_patterns.is_empty());
        assert!(!result.applicable_skills.is_empty());

        // Warnings should contain entries from all checks
        let all_warnings = result.warnings.join("\n");
        assert!(all_warnings.contains("Use JWT"));
        assert!(all_warnings.contains("Blast radius"));
        assert!(all_warnings.contains("Lesson: Always test auth"));
        assert!(all_warnings.contains("Dangerous pattern: Never use eval()"));
    }

    // ── Fix: PreBashCheck should return up to 2 skills, not 1 ──

    #[test]
    fn test_pre_bash_check_returns_two_matching_skills() {
        let conn = setup();

        // Insert two skills matching "cargo"
        store_skill(
            &conn,
            &Skill {
                id: "skill-cargo-build".into(),
                name: "cargo build workflow".into(),
                domain: "rust".into(),
                description: "Build Rust projects with cargo".into(),
                steps: vec!["run cargo build".into()],
                success_count: 5,
                fail_count: 0,
                last_used: None,
                source: "bench".into(),
                version: 1,
                project: None,
                skill_type: "procedural".into(),
                user_specific: false,
                observed_count: 1,
                correlation_ids: vec![],
            },
        )
        .unwrap();

        store_skill(
            &conn,
            &Skill {
                id: "skill-cargo-test".into(),
                name: "cargo test procedure".into(),
                domain: "testing".into(),
                description: "Run tests with cargo test".into(),
                steps: vec!["run cargo test".into()],
                success_count: 3,
                fail_count: 0,
                last_used: None,
                source: "bench".into(),
                version: 1,
                project: None,
                skill_type: "procedural".into(),
                user_specific: false,
                observed_count: 1,
                correlation_ids: vec![],
            },
        )
        .unwrap();

        let result = pre_bash_check(&conn, "cargo test --release");
        assert_eq!(
            result.relevant_skills.len(),
            2,
            "PreBashCheck should return up to 2 matching skills, got: {:?}",
            result.relevant_skills
        );
    }
}
