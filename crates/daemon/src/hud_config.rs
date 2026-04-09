//! HUD Configuration — cascading config merge + K8s context + validation
//!
//! Config cascade: organization → team → project → user (most specific wins).
//! Uses the existing `config_scope` table with `hud.*` key prefix.
//! K8s context is read directly from `~/.kube/config` (no kubectl subprocess).

use rusqlite::{params, Connection};

/// Known HUD sections — validated on set.
pub const KNOWN_SECTIONS: &[&str] = &[
    "memory", "health", "agents", "k8s", "git", "security", "tasks",
];

/// Valid density values.
pub const VALID_DENSITIES: &[&str] = &["compact", "normal", "verbose"];

/// Valid theme values.
pub const VALID_THEMES: &[&str] = &["dark", "light", "high-contrast"];

/// Default HUD config (applied when no scoped config exists).
pub const DEFAULT_SECTIONS: &str = r#"["memory","health","agents","k8s","git","security","tasks"]"#;
pub const DEFAULT_DENSITY: &str = "normal";
pub const DEFAULT_THEME: &str = "dark";
pub const DEFAULT_REFRESH_SECS: &str = "10";
pub const DEFAULT_SHOW_NOTIFICATIONS: &str = "all";

/// Merged HUD config entry with provenance.
#[derive(Debug, Clone)]
pub struct MergedEntry {
    pub key: String,
    pub value: String,
    pub scope_type: String,
    pub scope_id: String,
    pub locked: bool,
}

/// Scope priority: higher number = more specific = wins.
fn scope_priority(scope_type: &str) -> u8 {
    match scope_type {
        "organization" => 1,
        "team" => 2,
        "project" => 3,
        "user" => 4,
        _ => 0,
    }
}

/// Get merged HUD config from config_scope table.
/// Cascade: org → team → project → user. Most specific wins unless locked at higher scope.
pub fn get_merged_hud_config(
    conn: &Connection,
    org_id: Option<&str>,
    team_id: Option<&str>,
    project: Option<&str>,
    user_id: Option<&str>,
) -> rusqlite::Result<Vec<MergedEntry>> {
    // Collect all hud.* entries across all relevant scopes
    let mut all_entries: Vec<(String, String, String, String, bool)> = Vec::new();

    let scopes: Vec<(&str, &str)> = [
        org_id.map(|id| ("organization", id)),
        team_id.map(|id| ("team", id)),
        project.map(|id| ("project", id)),
        user_id.map(|id| ("user", id)),
    ]
    .into_iter()
    .flatten()
    .collect();

    for (stype, sid) in &scopes {
        let mut stmt = conn.prepare(
            "SELECT key, value, locked FROM config_scope
             WHERE scope_type = ?1 AND scope_id = ?2 AND key LIKE 'hud.%'"
        )?;
        let rows = stmt.query_map(params![stype, sid], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, bool>(2)?,
            ))
        })?;
        for row in rows.flatten() {
            all_entries.push((row.0, row.1, stype.to_string(), sid.to_string(), row.2));
        }
    }

    // Merge: per key, most-specific scope wins UNLESS a higher scope has locked=true
    let mut merged: std::collections::HashMap<String, MergedEntry> = std::collections::HashMap::new();
    // Track locked keys — once locked at a scope, lower scopes can't override
    let mut locked_keys: std::collections::HashSet<String> = std::collections::HashSet::new();

    // Sort by scope priority (lowest first so higher overrides)
    let mut sorted = all_entries.clone();
    sorted.sort_by_key(|(_, _, stype, _, _)| scope_priority(stype));

    for (key, value, stype, sid, locked) in sorted {
        if locked_keys.contains(&key) {
            // Key is locked at a higher scope — skip this override
            continue;
        }
        if locked {
            locked_keys.insert(key.clone());
        }
        merged.insert(key.clone(), MergedEntry {
            key,
            value,
            scope_type: stype,
            scope_id: sid,
            locked,
        });
    }

    // Fill in defaults for missing keys
    let defaults = [
        ("hud.sections", DEFAULT_SECTIONS),
        ("hud.density", DEFAULT_DENSITY),
        ("hud.theme", DEFAULT_THEME),
        ("hud.refresh_secs", DEFAULT_REFRESH_SECS),
        ("hud.show_notifications", DEFAULT_SHOW_NOTIFICATIONS),
    ];
    for (key, default_val) in defaults {
        if !merged.contains_key(key) {
            merged.insert(key.to_string(), MergedEntry {
                key: key.to_string(),
                value: default_val.to_string(),
                scope_type: "default".to_string(),
                scope_id: "system".to_string(),
                locked: false,
            });
        }
    }

    let mut result: Vec<MergedEntry> = merged.into_values().collect();
    result.sort_by(|a, b| a.key.cmp(&b.key));
    Ok(result)
}

/// Validate a HUD config key/value pair.
/// Returns Ok(()) if valid, Err(message) if invalid.
pub fn validate_hud_config(key: &str, value: &str) -> Result<(), String> {
    if !key.starts_with("hud.") {
        return Err(format!("HUD config keys must start with 'hud.', got '{key}'"));
    }

    match key {
        "hud.sections" => {
            let sections: Vec<String> = serde_json::from_str(value)
                .map_err(|e| format!("hud.sections must be JSON array: {e}"))?;
            for s in &sections {
                if !KNOWN_SECTIONS.contains(&s.as_str()) {
                    return Err(format!("unknown section '{s}', known: {KNOWN_SECTIONS:?}"));
                }
            }
        }
        "hud.density" => {
            if !VALID_DENSITIES.contains(&value) {
                return Err(format!("hud.density must be one of {VALID_DENSITIES:?}, got '{value}'"));
            }
        }
        "hud.theme" => {
            if !VALID_THEMES.contains(&value) {
                return Err(format!("hud.theme must be one of {VALID_THEMES:?}, got '{value}'"));
            }
        }
        "hud.refresh_secs" => {
            let secs: u64 = value.parse()
                .map_err(|_| format!("hud.refresh_secs must be a number, got '{value}'"))?;
            if !(5..=60).contains(&secs) {
                return Err(format!("hud.refresh_secs must be 5-60, got {secs}"));
            }
        }
        "hud.show_notifications" => {
            if !["all", "high-only", "none"].contains(&value) {
                return Err(format!("hud.show_notifications must be all|high-only|none, got '{value}'"));
            }
        }
        _ => {
            // Unknown hud.* keys are allowed (extensibility)
        }
    }

    Ok(())
}

/// Check if a key is locked at a higher scope.
/// Returns Some(scope_type, scope_id) if locked, None if free.
pub fn check_lock(
    conn: &Connection,
    key: &str,
    target_scope: &str,
    org_id: Option<&str>,
    team_id: Option<&str>,
    project: Option<&str>,
) -> rusqlite::Result<Option<(String, String)>> {
    let target_priority = scope_priority(target_scope);

    let scopes: Vec<(&str, &str)> = [
        org_id.map(|id| ("organization", id)),
        team_id.map(|id| ("team", id)),
        project.map(|id| ("project", id)),
    ]
    .into_iter()
    .flatten()
    .collect();

    for (stype, sid) in &scopes {
        if scope_priority(stype) >= target_priority {
            continue; // Only check HIGHER scopes
        }
        let locked: bool = conn.query_row(
            "SELECT locked FROM config_scope WHERE scope_type = ?1 AND scope_id = ?2 AND key = ?3",
            params![stype, sid, key],
            |row| row.get(0),
        ).unwrap_or(false);

        if locked {
            return Ok(Some((stype.to_string(), sid.to_string())));
        }
    }
    Ok(None)
}

/// Export HUD config entries for a scope as TOML string.
pub fn export_as_toml(
    conn: &Connection,
    scope_type: &str,
    scope_id: &str,
) -> rusqlite::Result<String> {
    let mut stmt = conn.prepare(
        "SELECT key, value FROM config_scope
         WHERE scope_type = ?1 AND scope_id = ?2 AND key LIKE 'hud.%'
         ORDER BY key"
    )?;
    let rows: Vec<(String, String)> = stmt.query_map(params![scope_type, scope_id], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?.flatten().collect();

    if rows.is_empty() {
        return Ok("# No HUD configuration set for this scope\n".to_string());
    }

    let mut toml = String::from("# Forge HUD Configuration\n# Commit this as .forge/hud.toml\n\n[hud]\n");
    for (key, value) in &rows {
        let short_key = key.strip_prefix("hud.").unwrap_or(key);
        // Detect if value is JSON array/object or plain string
        if value.starts_with('[') || value.starts_with('{') || value.starts_with('"') {
            toml.push_str(&format!("{short_key} = {value}\n"));
        } else if let Ok(n) = value.parse::<u64>() {
            toml.push_str(&format!("{short_key} = {n}\n"));
        } else {
            toml.push_str(&format!("{short_key} = \"{value}\"\n"));
        }
    }
    Ok(toml)
}

/// Read current Kubernetes context from ~/.kube/config.
/// Returns (context_name, namespace) or None if unavailable.
/// No kubectl subprocess — direct YAML parse.
pub fn read_k8s_context() -> Option<(String, Option<String>)> {
    let home = std::env::var("HOME").ok()?;
    let kubeconfig = std::env::var("KUBECONFIG")
        .unwrap_or_else(|_| format!("{home}/.kube/config"));

    let content = std::fs::read_to_string(&kubeconfig).ok()?;

    // Simple YAML parsing for current-context (avoid adding yaml dep)
    let mut current_context: Option<String> = None;
    let mut in_context_block = false;
    let mut target_context: Option<String> = None;
    let mut namespace: Option<String> = None;

    for line in content.lines() {
        let trimmed = line.trim();

        // Find current-context field
        if trimmed.starts_with("current-context:") {
            current_context = trimmed
                .strip_prefix("current-context:")
                .map(|s| s.trim().trim_matches('"').trim_matches('\'').to_string());
        }
    }

    let ctx_name = current_context?;

    // Find the namespace for this context
    // Search for the context's namespace in the contexts list
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("- name:") && trimmed.contains(&ctx_name) {
            in_context_block = true;
            target_context = Some(ctx_name.clone());
        } else if in_context_block && trimmed.starts_with("namespace:") {
            namespace = trimmed
                .strip_prefix("namespace:")
                .map(|s| s.trim().trim_matches('"').trim_matches('\'').to_string())
                .filter(|s| !s.is_empty());
            break;
        } else if in_context_block && trimmed.starts_with("- name:") {
            // Next context block — stop searching
            break;
        }
    }

    target_context.or(Some(ctx_name.clone())).map(|ctx| (ctx, namespace))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{schema::create_schema, vec};

    fn open_db() -> Connection {
        vec::init_sqlite_vec();
        let conn = Connection::open_in_memory().unwrap();
        create_schema(&conn).unwrap();
        conn
    }

    fn set_config(conn: &Connection, scope_type: &str, scope_id: &str, key: &str, value: &str, locked: bool) {
        let id = ulid::Ulid::new().to_string();
        let now = forge_core::time::now_iso();
        conn.execute(
            "INSERT OR REPLACE INTO config_scope (id, scope_type, scope_id, key, value, locked, set_by, set_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'test', ?7)",
            params![id, scope_type, scope_id, key, value, locked, now],
        ).unwrap();
    }

    #[test]
    fn test_merge_cascade_most_specific_wins() {
        let conn = open_db();

        set_config(&conn, "organization", "default", "hud.density", "compact", false);
        set_config(&conn, "user", "durga", "hud.density", "verbose", false);

        let merged = get_merged_hud_config(&conn, Some("default"), None, None, Some("durga")).unwrap();
        let density = merged.iter().find(|e| e.key == "hud.density").unwrap();
        assert_eq!(density.value, "verbose", "user scope should override org");
        assert_eq!(density.scope_type, "user");
    }

    #[test]
    fn test_merge_locked_key_blocks_override() {
        let conn = open_db();

        set_config(&conn, "team", "backend", "hud.sections", r#"["memory","health"]"#, true);
        set_config(&conn, "user", "durga", "hud.sections", r#"["memory"]"#, false);

        let merged = get_merged_hud_config(&conn, None, Some("backend"), None, Some("durga")).unwrap();
        let sections = merged.iter().find(|e| e.key == "hud.sections").unwrap();
        assert_eq!(sections.value, r#"["memory","health"]"#, "locked team config should NOT be overridden by user");
        assert_eq!(sections.scope_type, "team");
    }

    #[test]
    fn test_merge_defaults_for_missing_keys() {
        let conn = open_db();

        let merged = get_merged_hud_config(&conn, None, None, None, None).unwrap();
        let density = merged.iter().find(|e| e.key == "hud.density").unwrap();
        assert_eq!(density.value, "normal");
        assert_eq!(density.scope_type, "default");
    }

    #[test]
    fn test_validate_sections() {
        assert!(validate_hud_config("hud.sections", r#"["memory","health","k8s"]"#).is_ok());
        assert!(validate_hud_config("hud.sections", r#"["invalid"]"#).is_err());
        assert!(validate_hud_config("hud.sections", "not json").is_err());
    }

    #[test]
    fn test_validate_density() {
        assert!(validate_hud_config("hud.density", "compact").is_ok());
        assert!(validate_hud_config("hud.density", "normal").is_ok());
        assert!(validate_hud_config("hud.density", "invalid").is_err());
    }

    #[test]
    fn test_validate_theme() {
        assert!(validate_hud_config("hud.theme", "dark").is_ok());
        assert!(validate_hud_config("hud.theme", "light").is_ok());
        assert!(validate_hud_config("hud.theme", "high-contrast").is_ok());
        assert!(validate_hud_config("hud.theme", "blue").is_err());
    }

    #[test]
    fn test_validate_refresh_secs() {
        assert!(validate_hud_config("hud.refresh_secs", "5").is_ok());
        assert!(validate_hud_config("hud.refresh_secs", "60").is_ok());
        assert!(validate_hud_config("hud.refresh_secs", "3").is_err());
        assert!(validate_hud_config("hud.refresh_secs", "100").is_err());
        assert!(validate_hud_config("hud.refresh_secs", "abc").is_err());
    }

    #[test]
    fn test_validate_key_prefix() {
        assert!(validate_hud_config("not_hud", "value").is_err());
    }

    #[test]
    fn test_export_toml() {
        let conn = open_db();
        set_config(&conn, "project", "forge", "hud.density", "compact", false);
        set_config(&conn, "project", "forge", "hud.sections", r#"["memory","health"]"#, false);

        let toml = export_as_toml(&conn, "project", "forge").unwrap();
        assert!(toml.contains("[hud]"));
        assert!(toml.contains("density = \"compact\""));
        assert!(toml.contains("sections = [\"memory\",\"health\"]"));
    }

    #[test]
    fn test_export_empty() {
        let conn = open_db();
        let toml = export_as_toml(&conn, "project", "nonexistent").unwrap();
        assert!(toml.contains("No HUD configuration"));
    }

    #[test]
    fn test_check_lock_blocks_user() {
        let conn = open_db();
        set_config(&conn, "team", "backend", "hud.sections", r#"["memory"]"#, true);

        let lock = check_lock(&conn, "hud.sections", "user", None, Some("backend"), None).unwrap();
        assert!(lock.is_some(), "should detect locked key at team scope");
        let (stype, _) = lock.unwrap();
        assert_eq!(stype, "team");
    }

    #[test]
    fn test_check_lock_allows_when_not_locked() {
        let conn = open_db();
        set_config(&conn, "team", "backend", "hud.sections", r#"["memory"]"#, false);

        let lock = check_lock(&conn, "hud.sections", "user", None, Some("backend"), None).unwrap();
        assert!(lock.is_none(), "unlocked key should allow override");
    }

    #[test]
    fn test_k8s_context_no_kubeconfig() {
        // When KUBECONFIG points to nonexistent file, should return None
        std::env::set_var("KUBECONFIG", "/tmp/nonexistent-kubeconfig-xyz");
        let result = read_k8s_context();
        // Reset
        std::env::remove_var("KUBECONFIG");
        assert!(result.is_none(), "should return None for missing kubeconfig");
    }

    #[test]
    fn test_k8s_context_reads_real_kubeconfig() {
        // If ~/.kube/config exists, should return the current context
        let home = std::env::var("HOME").unwrap_or_default();
        let kubeconfig = format!("{home}/.kube/config");
        if std::path::Path::new(&kubeconfig).exists() {
            // Reset KUBECONFIG to use default path
            std::env::remove_var("KUBECONFIG");
            let result = read_k8s_context();
            assert!(result.is_some(), "should read context from real kubeconfig");
            let (ctx, _ns) = result.unwrap();
            assert!(!ctx.is_empty(), "context name should not be empty");
        }
        // If no kubeconfig, this test is a no-op (skipped)
    }
}
