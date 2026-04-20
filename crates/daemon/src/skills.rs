use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::path::Path;

/// A skill entry parsed from SKILL.md frontmatter or read from the registry.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SkillEntry {
    pub id: String,
    pub name: String,
    pub description: String,
    pub category: String,
    pub file_path: String,
    pub installed_for_project: Option<String>,
    pub indexed_at: String,
}

/// Parse SKILL.md frontmatter (YAML between --- delimiters).
/// Returns Some(SkillEntry) if the file has valid frontmatter with at least a `name`.
pub fn parse_skill_frontmatter(path: &Path) -> Option<SkillEntry> {
    let content = std::fs::read_to_string(path).ok()?;
    let trimmed = content.trim_start();

    // Must start with "---"
    if !trimmed.starts_with("---") {
        return None;
    }

    // Find the closing "---"
    let after_first = &trimmed[3..];
    let end_idx = after_first.find("\n---")?;
    let frontmatter = &after_first[..end_idx];

    let mut name: Option<String> = None;
    let mut description = String::new();
    let mut category = String::from("general");

    for line in frontmatter.lines() {
        let line = line.trim();
        if let Some(val) = line.strip_prefix("name:") {
            name = Some(val.trim().trim_matches('"').trim_matches('\'').to_string());
        } else if let Some(val) = line.strip_prefix("description:") {
            description = val.trim().trim_matches('"').trim_matches('\'').to_string();
        } else if let Some(val) = line.strip_prefix("category:") {
            category = val.trim().trim_matches('"').trim_matches('\'').to_string();
        }
    }

    let name = name?;
    if name.is_empty() {
        return None;
    }

    Some(SkillEntry {
        id: ulid::Ulid::new().to_string(),
        name,
        description,
        category,
        file_path: path.to_string_lossy().to_string(),
        installed_for_project: None,
        indexed_at: String::new(), // filled by DB
    })
}

/// Index all SKILL.md files from a directory tree.
/// Inserts or replaces entries in the skill_registry table.
/// Returns the number of skills indexed.
pub fn index_skills_directory(conn: &Connection, dir: &Path) -> Result<usize, String> {
    if !dir.exists() || !dir.is_dir() {
        return Err(format!(
            "skills directory does not exist: {}",
            dir.display()
        ));
    }

    let mut count = 0;
    index_recursive(conn, dir, &mut count)?;
    Ok(count)
}

fn index_recursive(conn: &Connection, dir: &Path, count: &mut usize) -> Result<(), String> {
    let entries = std::fs::read_dir(dir)
        .map_err(|e| format!("failed to read directory {}: {e}", dir.display()))?;

    for entry in entries {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        let path = entry.path();

        // Symlink defense: skip symlinks to prevent directory traversal attacks
        if path
            .symlink_metadata()
            .is_ok_and(|m| m.file_type().is_symlink())
        {
            continue;
        }

        if path.is_dir() {
            index_recursive(conn, &path, count)?;
        } else if path.file_name().is_some_and(|n| n == "SKILL.md") {
            if let Some(skill) = parse_skill_frontmatter(&path) {
                upsert_skill(conn, &skill)?;
                *count += 1;
            }
        }
    }
    Ok(())
}

fn upsert_skill(conn: &Connection, skill: &SkillEntry) -> Result<(), String> {
    conn.execute(
        "INSERT INTO skill_registry (id, name, description, category, file_path, indexed_at)
         VALUES (?1, ?2, ?3, ?4, ?5, datetime('now'))
         ON CONFLICT(name, category) DO UPDATE SET
           description = excluded.description,
           file_path = excluded.file_path,
           indexed_at = datetime('now')",
        rusqlite::params![
            skill.id,
            skill.name,
            skill.description,
            skill.category,
            skill.file_path,
        ],
    )
    .map_err(|e| format!("failed to upsert skill '{}': {e}", skill.name))?;
    Ok(())
}

/// List skills with optional category filter and FTS5 search.
pub fn list_skills(
    conn: &Connection,
    category: Option<&str>,
    search: Option<&str>,
    limit: usize,
) -> Result<Vec<SkillEntry>, String> {
    let limit = if limit == 0 { 100 } else { limit };

    match (category, search) {
        (None, None) => {
            let mut stmt = conn
                .prepare(
                    "SELECT id, name, description, category, file_path, installed_for_project, indexed_at
                     FROM skill_registry ORDER BY name LIMIT ?1",
                )
                .map_err(|e| format!("prepare list_skills: {e}"))?;
            let rows = stmt
                .query_map(rusqlite::params![limit], row_to_entry)
                .map_err(|e| format!("query list_skills: {e}"))?;
            collect_rows(rows)
        }
        (Some(cat), None) => {
            let mut stmt = conn
                .prepare(
                    "SELECT id, name, description, category, file_path, installed_for_project, indexed_at
                     FROM skill_registry WHERE category = ?1 ORDER BY name LIMIT ?2",
                )
                .map_err(|e| format!("prepare list_skills category: {e}"))?;
            let rows = stmt
                .query_map(rusqlite::params![cat, limit], row_to_entry)
                .map_err(|e| format!("query list_skills category: {e}"))?;
            collect_rows(rows)
        }
        (cat_opt, Some(query)) => {
            // FTS5 search; sanitize to prevent FTS5 operator injection
            let sanitized: String = query
                .chars()
                .filter(|c| c.is_alphanumeric() || *c == ' ' || *c == '_')
                .collect();
            if sanitized.trim().is_empty() {
                return Ok(Vec::new());
            }
            let fts_query = format!("{sanitized}*");
            if let Some(cat) = cat_opt {
                let mut stmt = conn
                    .prepare(
                        "SELECT sr.id, sr.name, sr.description, sr.category, sr.file_path, sr.installed_for_project, sr.indexed_at
                         FROM skill_registry sr
                         JOIN skill_registry_fts fts ON sr.rowid = fts.rowid
                         WHERE skill_registry_fts MATCH ?1 AND sr.category = ?2
                         ORDER BY rank LIMIT ?3",
                    )
                    .map_err(|e| format!("prepare list_skills fts+cat: {e}"))?;
                let rows = stmt
                    .query_map(rusqlite::params![fts_query, cat, limit], row_to_entry)
                    .map_err(|e| format!("query list_skills fts+cat: {e}"))?;
                collect_rows(rows)
            } else {
                let mut stmt = conn
                    .prepare(
                        "SELECT sr.id, sr.name, sr.description, sr.category, sr.file_path, sr.installed_for_project, sr.indexed_at
                         FROM skill_registry sr
                         JOIN skill_registry_fts fts ON sr.rowid = fts.rowid
                         WHERE skill_registry_fts MATCH ?1
                         ORDER BY rank LIMIT ?2",
                    )
                    .map_err(|e| format!("prepare list_skills fts: {e}"))?;
                let rows = stmt
                    .query_map(rusqlite::params![fts_query, limit], row_to_entry)
                    .map_err(|e| format!("query list_skills fts: {e}"))?;
                collect_rows(rows)
            }
        }
    }
}

/// Install a skill for a project (marks it as active).
pub fn install_skill(conn: &Connection, skill_name: &str, project: &str) -> Result<(), String> {
    // Check skill exists
    let exists: bool = conn
        .query_row(
            "SELECT COUNT(*) > 0 FROM skill_registry WHERE name = ?1",
            rusqlite::params![skill_name],
            |row| row.get(0),
        )
        .unwrap_or(false);

    if !exists {
        return Err(format!("skill '{skill_name}' not found in registry"));
    }

    conn.execute(
        "UPDATE skill_registry SET installed_for_project = ?2 WHERE name = ?1",
        rusqlite::params![skill_name, project],
    )
    .map_err(|e| format!("install_skill: {e}"))?;
    Ok(())
}

/// Uninstall a skill (clears the installed_for_project field).
pub fn uninstall_skill(conn: &Connection, skill_name: &str, project: &str) -> Result<(), String> {
    let updated = conn
        .execute(
            "UPDATE skill_registry SET installed_for_project = NULL WHERE name = ?1 AND installed_for_project = ?2",
            rusqlite::params![skill_name, project],
        )
        .map_err(|e| format!("uninstall_skill: {e}"))?;

    if updated == 0 {
        return Err(format!(
            "skill '{skill_name}' not found or not installed for project '{project}'"
        ));
    }
    Ok(())
}

/// Get full skill details by name.
/// If `workspace_root` is provided, absolute file_path values are made relative to it.
pub fn skill_info(
    conn: &Connection,
    name: &str,
    workspace_root: Option<&str>,
) -> Result<Option<SkillEntry>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id, name, description, category, file_path, installed_for_project, indexed_at
             FROM skill_registry WHERE name = ?1",
        )
        .map_err(|e| format!("prepare skill_info: {e}"))?;

    let mut rows = stmt
        .query_map(rusqlite::params![name], row_to_entry)
        .map_err(|e| format!("query skill_info: {e}"))?;

    match rows.next() {
        Some(Ok(mut entry)) => {
            // Strip workspace root from absolute file_path to prevent leaking server paths
            if let Some(root) = workspace_root {
                let root_prefix = if root.ends_with('/') {
                    root.to_string()
                } else {
                    format!("{root}/")
                };
                if entry.file_path.starts_with(&root_prefix) {
                    entry.file_path = entry.file_path[root_prefix.len()..].to_string();
                }
            }
            Ok(Some(entry))
        }
        Some(Err(e)) => Err(format!("skill_info row: {e}")),
        None => Ok(None),
    }
}

/// Refresh the registry: delete all entries and re-index from disk.
pub fn refresh_skills(conn: &Connection, dir: &Path) -> Result<usize, String> {
    // Delete all existing entries (triggers will clean FTS)
    conn.execute("DELETE FROM skill_registry", [])
        .map_err(|e| format!("clear skill_registry: {e}"))?;

    index_skills_directory(conn, dir)
}

/// Auto-populate the skill registry on daemon boot.
///
/// Fix #55: the populator already worked, but nothing invoked it at startup,
/// so `forge-next skills-list` returned `count: 0` on a fresh daemon. This is
/// the boot-time wrapper wired in from `main.rs`.
///
/// Critically this does **NOT** delegate to [`refresh_skills`]: that function
/// begins with `DELETE FROM skill_registry`, which would wipe per-project
/// `installed_for_project` state on every daemon restart (rows are reinserted
/// with that column NULL). Instead this path calls [`index_skills_directory`]
/// directly, which is pure idempotent upsert — the `ON CONFLICT` clause in
/// `upsert_skill` leaves `installed_for_project` untouched. Trade-off: a
/// skill deleted from disk is not evicted at boot; callers can still run
/// `RefreshSkillsIndex` for the clear-and-rebuild semantics.
///
/// Semantics (spec §5.2):
/// * If `skills_dir` is not an existing directory, returns `Ok(0)` — the
///   daemon must still boot; callers log a note and move on. Checking
///   `is_dir()` (not just `exists()`) means a stray file at that path
///   short-circuits before any write.
/// * Otherwise upserts every `SKILL.md` found under the directory tree.
///   Errors propagate so the caller can log them; already-indexed rows
///   survive partial failures.
pub fn auto_populate_on_start(conn: &Connection, skills_dir: &Path) -> Result<usize, String> {
    if !skills_dir.is_dir() {
        return Ok(0);
    }
    index_skills_directory(conn, skills_dir)
}

/// Resolve the directory used by daemon boot to seed the skill registry.
///
/// Cascade (spec §3.1):
/// 1. `FORGE_SKILLS_DIR` env var — explicit override. Empty string treated
///    as unset (mirrors `forge_core::forge_dir`'s handling of `FORGE_DIR`).
/// 2. `config_skills_dir` — the `skills_directory` field from `config.toml`,
///    passed through as `Some(&str)`. Empty string treated as unset.
/// 3. `{forge_home}/skills` — user-global install location, if it exists.
/// 4. `{cwd}/skills` — project-local fallback.
///
/// The returned path is not validated; `auto_populate_on_start` short-circuits
/// when the directory is missing.
pub fn resolve_skills_dir(
    forge_home: &Path,
    config_skills_dir: Option<&str>,
    cwd: &Path,
) -> std::path::PathBuf {
    if let Ok(env_dir) = std::env::var("FORGE_SKILLS_DIR") {
        if !env_dir.is_empty() {
            return std::path::PathBuf::from(env_dir);
        }
    }
    if let Some(cfg) = config_skills_dir {
        if !cfg.is_empty() {
            return std::path::PathBuf::from(cfg);
        }
    }
    let home_skills = forge_home.join("skills");
    if home_skills.is_dir() {
        return home_skills;
    }
    cwd.join("skills")
}

// ── Internal helpers ──

fn row_to_entry(row: &rusqlite::Row<'_>) -> rusqlite::Result<SkillEntry> {
    Ok(SkillEntry {
        id: row.get(0)?,
        name: row.get(1)?,
        description: row.get(2)?,
        category: row.get(3)?,
        file_path: row.get(4)?,
        installed_for_project: row.get(5)?,
        indexed_at: row.get(6)?,
    })
}

fn collect_rows(
    rows: rusqlite::MappedRows<'_, impl FnMut(&rusqlite::Row<'_>) -> rusqlite::Result<SkillEntry>>,
) -> Result<Vec<SkillEntry>, String> {
    let mut results = Vec::new();
    for row in rows {
        results.push(row.map_err(|e| format!("row error: {e}"))?);
    }
    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::schema::create_schema;
    use std::io::Write;
    use tempfile::TempDir;

    fn setup_db() -> Connection {
        crate::db::vec::init_sqlite_vec();
        let conn = Connection::open_in_memory().unwrap();
        create_schema(&conn).unwrap();
        conn
    }

    fn create_skill_md(dir: &Path, name: &str, desc: &str, category: Option<&str>) {
        let skill_dir = dir.join(name);
        std::fs::create_dir_all(&skill_dir).unwrap();
        let mut f = std::fs::File::create(skill_dir.join("SKILL.md")).unwrap();
        if let Some(cat) = category {
            writeln!(
                f,
                "---\nname: {name}\ndescription: \"{desc}\"\ncategory: {cat}\n---\n\n# {name}\n\nContent here."
            )
            .unwrap();
        } else {
            writeln!(
                f,
                "---\nname: {name}\ndescription: \"{desc}\"\n---\n\n# {name}\n\nContent here."
            )
            .unwrap();
        }
    }

    #[test]
    fn test_parse_skill_frontmatter() {
        let tmp = TempDir::new().unwrap();
        let skill_path = tmp.path().join("SKILL.md");
        std::fs::write(
            &skill_path,
            "---\nname: test-skill\ndescription: \"A test skill\"\ncategory: engineering\n---\n\n# Test\n",
        )
        .unwrap();

        let entry = parse_skill_frontmatter(&skill_path);
        assert!(entry.is_some());
        let entry = entry.unwrap();
        assert_eq!(entry.name, "test-skill");
        assert_eq!(entry.description, "A test skill");
        assert_eq!(entry.category, "engineering");
    }

    #[test]
    fn test_parse_skill_frontmatter_no_category() {
        let tmp = TempDir::new().unwrap();
        let skill_path = tmp.path().join("SKILL.md");
        std::fs::write(
            &skill_path,
            "---\nname: my-skill\ndescription: \"desc\"\n---\n\n# My Skill\n",
        )
        .unwrap();

        let entry = parse_skill_frontmatter(&skill_path).unwrap();
        assert_eq!(entry.name, "my-skill");
        assert_eq!(entry.category, "general"); // default
    }

    #[test]
    fn test_parse_skill_frontmatter_no_name() {
        let tmp = TempDir::new().unwrap();
        let skill_path = tmp.path().join("SKILL.md");
        std::fs::write(
            &skill_path,
            "---\ndescription: \"no name\"\n---\n\n# No Name\n",
        )
        .unwrap();

        assert!(parse_skill_frontmatter(&skill_path).is_none());
    }

    #[test]
    fn test_parse_skill_frontmatter_no_frontmatter() {
        let tmp = TempDir::new().unwrap();
        let skill_path = tmp.path().join("SKILL.md");
        std::fs::write(&skill_path, "# No frontmatter\n\nJust content.").unwrap();

        assert!(parse_skill_frontmatter(&skill_path).is_none());
    }

    #[test]
    fn test_index_skills_directory() {
        let conn = setup_db();
        let tmp = TempDir::new().unwrap();

        create_skill_md(
            tmp.path(),
            "skill-alpha",
            "Alpha skill",
            Some("engineering"),
        );
        create_skill_md(tmp.path(), "skill-beta", "Beta skill", Some("marketing"));
        create_skill_md(tmp.path(), "skill-gamma", "Gamma skill", None);

        let count = index_skills_directory(&conn, tmp.path()).unwrap();
        assert_eq!(count, 3);

        // Verify they are in the DB
        let skills = list_skills(&conn, None, None, 100).unwrap();
        assert_eq!(skills.len(), 3);
    }

    #[test]
    fn test_list_skills_by_category() {
        let conn = setup_db();
        let tmp = TempDir::new().unwrap();

        create_skill_md(tmp.path(), "eng-1", "Eng skill 1", Some("engineering"));
        create_skill_md(tmp.path(), "eng-2", "Eng skill 2", Some("engineering"));
        create_skill_md(tmp.path(), "mkt-1", "Marketing skill", Some("marketing"));

        index_skills_directory(&conn, tmp.path()).unwrap();

        let eng_skills = list_skills(&conn, Some("engineering"), None, 100).unwrap();
        assert_eq!(eng_skills.len(), 2);

        let mkt_skills = list_skills(&conn, Some("marketing"), None, 100).unwrap();
        assert_eq!(mkt_skills.len(), 1);
        assert_eq!(mkt_skills[0].name, "mkt-1");

        let none_skills = list_skills(&conn, Some("nonexistent"), None, 100).unwrap();
        assert!(none_skills.is_empty());
    }

    #[test]
    fn test_list_skills_fts_search() {
        let conn = setup_db();
        let tmp = TempDir::new().unwrap();

        create_skill_md(
            tmp.path(),
            "seo-optimizer",
            "Optimize SEO rankings and metadata",
            Some("marketing"),
        );
        create_skill_md(
            tmp.path(),
            "code-review",
            "Automated code review and feedback",
            Some("engineering"),
        );
        create_skill_md(
            tmp.path(),
            "seo-audit",
            "Audit SEO performance",
            Some("marketing"),
        );

        index_skills_directory(&conn, tmp.path()).unwrap();

        let seo_results = list_skills(&conn, None, Some("seo"), 100).unwrap();
        assert_eq!(seo_results.len(), 2);

        // Search with category filter
        let seo_mkt = list_skills(&conn, Some("marketing"), Some("seo"), 100).unwrap();
        assert_eq!(seo_mkt.len(), 2);

        // Search that should match description
        let review_results = list_skills(&conn, None, Some("code"), 100).unwrap();
        assert!(!review_results.is_empty());
    }

    #[test]
    fn test_install_uninstall_skill() {
        let conn = setup_db();
        let tmp = TempDir::new().unwrap();

        create_skill_md(
            tmp.path(),
            "test-skill",
            "A test skill",
            Some("engineering"),
        );
        index_skills_directory(&conn, tmp.path()).unwrap();

        // Install
        install_skill(&conn, "test-skill", "my-project").unwrap();
        let info = skill_info(&conn, "test-skill", None).unwrap().unwrap();
        assert_eq!(info.installed_for_project.as_deref(), Some("my-project"));

        // Uninstall
        uninstall_skill(&conn, "test-skill", "my-project").unwrap();
        let info = skill_info(&conn, "test-skill", None).unwrap().unwrap();
        assert!(info.installed_for_project.is_none());
    }

    #[test]
    fn test_install_nonexistent_skill() {
        let conn = setup_db();
        let result = install_skill(&conn, "nonexistent", "my-project");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not found"));
    }

    #[test]
    fn test_skill_info() {
        let conn = setup_db();
        let tmp = TempDir::new().unwrap();

        create_skill_md(
            tmp.path(),
            "info-skill",
            "Detailed skill",
            Some("engineering"),
        );
        index_skills_directory(&conn, tmp.path()).unwrap();

        let info = skill_info(&conn, "info-skill", None).unwrap();
        assert!(info.is_some());
        let info = info.unwrap();
        assert_eq!(info.name, "info-skill");
        assert_eq!(info.description, "Detailed skill");
        assert_eq!(info.category, "engineering");

        // Non-existent skill
        let none = skill_info(&conn, "nonexistent", None).unwrap();
        assert!(none.is_none());
    }

    #[test]
    fn test_skill_info_strips_workspace_root() {
        let conn = setup_db();
        let tmp = TempDir::new().unwrap();

        create_skill_md(tmp.path(), "path-skill", "Path test", Some("engineering"));
        index_skills_directory(&conn, tmp.path()).unwrap();

        // Without workspace_root, file_path is absolute
        let info = skill_info(&conn, "path-skill", None).unwrap().unwrap();
        assert!(
            info.file_path.starts_with('/'),
            "file_path should be absolute without workspace_root: {}",
            info.file_path
        );

        // With workspace_root, file_path is stripped to relative
        let ws_root = tmp.path().to_string_lossy().to_string();
        let info = skill_info(&conn, "path-skill", Some(&ws_root))
            .unwrap()
            .unwrap();
        assert!(
            !info.file_path.starts_with('/'),
            "file_path should be relative with workspace_root: {}",
            info.file_path
        );
        assert!(
            info.file_path.starts_with("path-skill/"),
            "file_path should start with skill dir name: {}",
            info.file_path
        );
    }

    #[test]
    fn test_refresh_skills() {
        let conn = setup_db();
        let tmp = TempDir::new().unwrap();

        create_skill_md(tmp.path(), "skill-a", "Skill A", Some("engineering"));
        index_skills_directory(&conn, tmp.path()).unwrap();
        assert_eq!(list_skills(&conn, None, None, 100).unwrap().len(), 1);

        // Add another skill file
        create_skill_md(tmp.path(), "skill-b", "Skill B", Some("marketing"));

        // Refresh should pick up both
        let count = refresh_skills(&conn, tmp.path()).unwrap();
        assert_eq!(count, 2);
        assert_eq!(list_skills(&conn, None, None, 100).unwrap().len(), 2);
    }

    #[test]
    fn test_index_nonexistent_directory() {
        let conn = setup_db();
        let result = index_skills_directory(&conn, Path::new("/tmp/nonexistent_skills_dir_xyz"));
        assert!(result.is_err());
    }

    // ── Fix #55 regression: auto_populate_on_start ──
    //
    // Daemon startup must populate the skill_registry so `forge-next skills-list`
    // returns a non-zero count on a fresh daemon. Two properties matter:
    //   1. Happy path: a valid directory indexes every fixture and list_skills
    //      returns them.
    //   2. Missing directory: returns Ok(0) rather than erroring, so boot
    //      proceeds with an empty registry (spec §5.2).

    #[test]
    fn test_auto_populate_on_start_indexes_fixtures() {
        let conn = setup_db();
        let tmp = TempDir::new().unwrap();

        create_skill_md(tmp.path(), "auto-a", "auto skill A", Some("test"));
        create_skill_md(tmp.path(), "auto-b", "auto skill B", Some("test"));

        let n = auto_populate_on_start(&conn, tmp.path()).unwrap();
        assert_eq!(n, 2, "auto_populate_on_start returns count indexed");

        let listed = list_skills(&conn, None, None, 100).unwrap();
        assert_eq!(listed.len(), 2, "list_skills should return both fixtures");
        let names: Vec<String> = listed.iter().map(|s| s.name.clone()).collect();
        assert!(names.contains(&"auto-a".to_string()));
        assert!(names.contains(&"auto-b".to_string()));
    }

    #[test]
    fn test_auto_populate_on_start_missing_dir_returns_zero() {
        let conn = setup_db();
        // Explicitly nonexistent path — must NOT error, must return Ok(0).
        let missing = Path::new("/tmp/forge_skills_dir_does_not_exist_xyz_55");
        assert!(!missing.exists(), "test precondition: path must not exist");

        let n = auto_populate_on_start(&conn, missing).unwrap();
        assert_eq!(n, 0, "missing skills dir must return Ok(0), not an error");

        let listed = list_skills(&conn, None, None, 100).unwrap();
        assert!(
            listed.is_empty(),
            "registry stays empty when skills dir is missing"
        );
    }

    #[test]
    fn test_auto_populate_on_start_path_is_file_returns_zero() {
        // A stray file at the configured path must NOT trigger a DB wipe or
        // crash — it must return Ok(0) just like a missing path.
        let conn = setup_db();
        let tmp = TempDir::new().unwrap();
        let bogus = tmp.path().join("skills_not_a_dir");
        std::fs::write(&bogus, "oops").unwrap();

        let n = auto_populate_on_start(&conn, &bogus).unwrap();
        assert_eq!(n, 0);
    }

    #[test]
    fn test_auto_populate_on_start_preserves_installed_for_project() {
        // Regression: daemon restart must not wipe per-project install state.
        // Seed a skill, install it for a project, then run auto_populate_on_start
        // again (simulating reboot). The `installed_for_project` column must
        // survive the re-index because we use upsert-on-conflict, not DELETE.
        let conn = setup_db();
        let tmp = TempDir::new().unwrap();

        create_skill_md(
            tmp.path(),
            "sticky-skill",
            "should survive restart",
            Some("engineering"),
        );

        // First boot indexes the skill, then a client installs it.
        let n1 = auto_populate_on_start(&conn, tmp.path()).unwrap();
        assert_eq!(n1, 1);
        install_skill(&conn, "sticky-skill", "my-project").unwrap();
        let before = skill_info(&conn, "sticky-skill", None).unwrap().unwrap();
        assert_eq!(before.installed_for_project.as_deref(), Some("my-project"));

        // Second boot (simulated daemon restart) must preserve the install.
        let n2 = auto_populate_on_start(&conn, tmp.path()).unwrap();
        assert_eq!(n2, 1, "second boot re-indexes the same skill");

        let after = skill_info(&conn, "sticky-skill", None).unwrap().unwrap();
        assert_eq!(
            after.installed_for_project.as_deref(),
            Some("my-project"),
            "installed_for_project MUST survive daemon restart (#55 regression)"
        );
    }

    // Serialized because the test mutates the process-global
    // `FORGE_SKILLS_DIR` env var. Any other test in this binary that reads or
    // writes the same var must also carry `#[serial_test::serial]` so the two
    // don't race — `cargo test` runs unit tests in parallel by default.
    #[test]
    #[serial_test::serial]
    fn test_resolve_skills_dir_cascade() {
        let tmp = TempDir::new().unwrap();
        let forge_home = tmp.path().join("home");
        let cwd = tmp.path().join("cwd");
        std::fs::create_dir_all(&forge_home).unwrap();
        std::fs::create_dir_all(&cwd).unwrap();

        // SAFETY: `#[serial_test::serial]` holds a process-wide test lock, so
        // no other serialized test mutates env while this block runs. No
        // non-serial test in this crate reads or writes `FORGE_SKILLS_DIR`
        // (grep to verify before adding one).
        unsafe { std::env::remove_var("FORGE_SKILLS_DIR") };

        // Rung 4: cwd/skills when nothing else is configured and no forge_home/skills exists.
        let resolved = resolve_skills_dir(&forge_home, None, &cwd);
        assert_eq!(resolved, cwd.join("skills"));

        // Rung 3: forge_home/skills wins once it exists on disk.
        std::fs::create_dir_all(forge_home.join("skills")).unwrap();
        let resolved = resolve_skills_dir(&forge_home, None, &cwd);
        assert_eq!(resolved, forge_home.join("skills"));

        // Rung 2: config value overrides forge_home/skills even if the home
        // directory has skills; empty string is treated as unset.
        let resolved = resolve_skills_dir(&forge_home, Some(""), &cwd);
        assert_eq!(resolved, forge_home.join("skills"));
        let resolved = resolve_skills_dir(&forge_home, Some("/from/config"), &cwd);
        assert_eq!(resolved, Path::new("/from/config"));

        // Rung 1: env var beats everything.
        // SAFETY: see above.
        unsafe { std::env::set_var("FORGE_SKILLS_DIR", "/from/env") };
        let resolved = resolve_skills_dir(&forge_home, Some("/from/config"), &cwd);
        assert_eq!(resolved, Path::new("/from/env"));
        unsafe { std::env::remove_var("FORGE_SKILLS_DIR") };
    }
}
