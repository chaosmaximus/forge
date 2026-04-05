use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use forge_core::types::{
    PlatformEntry, Tool, ToolKind, ToolHealth,
    Skill, DomainDna, Perception, PerceptionKind, Severity,
    Declared, IdentityFacet, Disposition, DispositionTrait, Trend,
    Entity,
};

// ──────────────────────────────────────────────
// Helper: string conversions for enums
// ──────────────────────────────────────────────

fn tool_kind_str(k: &ToolKind) -> &'static str {
    match k {
        ToolKind::Cli => "cli",
        ToolKind::Mcp => "mcp",
        ToolKind::Builtin => "builtin",
        ToolKind::Plugin => "plugin",
    }
}

fn tool_kind_from_str(s: &str) -> ToolKind {
    match s {
        "cli" => ToolKind::Cli,
        "mcp" => ToolKind::Mcp,
        "builtin" => ToolKind::Builtin,
        "plugin" => ToolKind::Plugin,
        other => {
            eprintln!("[manas] unknown tool kind '{}', defaulting to Cli", other);
            ToolKind::Cli
        }
    }
}

fn tool_health_str(h: &ToolHealth) -> &'static str {
    match h {
        ToolHealth::Healthy => "healthy",
        ToolHealth::Degraded => "degraded",
        ToolHealth::Unavailable => "unavailable",
        ToolHealth::Unknown => "unknown",
    }
}

fn tool_health_from_str(s: &str) -> ToolHealth {
    match s {
        "healthy" => ToolHealth::Healthy,
        "degraded" => ToolHealth::Degraded,
        "unavailable" => ToolHealth::Unavailable,
        other => {
            eprintln!("[manas] unknown tool health '{}', defaulting to Unknown", other);
            ToolHealth::Unknown
        }
    }
}

fn perception_kind_str(k: &PerceptionKind) -> &'static str {
    match k {
        PerceptionKind::FileChange => "file_change",
        PerceptionKind::Error => "error",
        PerceptionKind::BuildResult => "build_result",
        PerceptionKind::TestResult => "test_result",
        PerceptionKind::UserFeedback => "user_feedback",
        PerceptionKind::MissingTool => "missing_tool",
        PerceptionKind::CrossSessionDecision => "cross_session_decision",
        PerceptionKind::ActionSummary => "action_summary",
        PerceptionKind::KnowledgeGap => "knowledge_gap",
    }
}

fn perception_kind_from_str(s: &str) -> PerceptionKind {
    match s {
        "file_change" => PerceptionKind::FileChange,
        "error" => PerceptionKind::Error,
        "build_result" => PerceptionKind::BuildResult,
        "test_result" => PerceptionKind::TestResult,
        "user_feedback" => PerceptionKind::UserFeedback,
        "missing_tool" => PerceptionKind::MissingTool,
        "cross_session_decision" => PerceptionKind::CrossSessionDecision,
        "action_summary" => PerceptionKind::ActionSummary,
        "knowledge_gap" => PerceptionKind::KnowledgeGap,
        other => {
            eprintln!("[manas] unknown perception kind '{}', defaulting to Error", other);
            PerceptionKind::Error
        }
    }
}

fn severity_str(s: &Severity) -> &'static str {
    match s {
        Severity::Debug => "debug",
        Severity::Info => "info",
        Severity::Warning => "warning",
        Severity::Error => "error",
        Severity::Critical => "critical",
    }
}

fn severity_from_str(s: &str) -> Severity {
    match s {
        "debug" => Severity::Debug,
        "info" => Severity::Info,
        "warning" => Severity::Warning,
        "error" => Severity::Error,
        "critical" => Severity::Critical,
        other => {
            eprintln!("[manas] unknown severity '{}', defaulting to Info", other);
            Severity::Info
        }
    }
}

fn disposition_trait_str(t: &DispositionTrait) -> &'static str {
    match t {
        DispositionTrait::Caution => "caution",
        DispositionTrait::Thoroughness => "thoroughness",
        DispositionTrait::Autonomy => "autonomy",
        DispositionTrait::Verbosity => "verbosity",
        DispositionTrait::Creativity => "creativity",
    }
}

fn disposition_trait_from_str(s: &str) -> DispositionTrait {
    match s {
        "caution" => DispositionTrait::Caution,
        "thoroughness" => DispositionTrait::Thoroughness,
        "autonomy" => DispositionTrait::Autonomy,
        "verbosity" => DispositionTrait::Verbosity,
        "creativity" => DispositionTrait::Creativity,
        other => {
            eprintln!("[manas] unknown disposition trait '{}', defaulting to Caution", other);
            DispositionTrait::Caution
        }
    }
}

fn trend_str(t: &Trend) -> &'static str {
    match t {
        Trend::Rising => "rising",
        Trend::Stable => "stable",
        Trend::Falling => "falling",
    }
}

fn trend_from_str(s: &str) -> Trend {
    match s {
        "rising" => Trend::Rising,
        "stable" => Trend::Stable,
        "falling" => Trend::Falling,
        other => {
            eprintln!("[manas] unknown trend '{}', defaulting to Stable", other);
            Trend::Stable
        }
    }
}

// Use shared timestamp from forge_core::time
fn now_iso() -> String {
    forge_core::time::now_iso()
}

// ──────────────────────────────────────────────
// Layer 0: Platform ops
// ──────────────────────────────────────────────

/// Store or update a platform key-value pair.
pub fn store_platform(conn: &Connection, entry: &PlatformEntry) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO platform (key, value, detected_at) VALUES (?1, ?2, ?3)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value, detected_at = excluded.detected_at",
        params![entry.key, entry.value, entry.detected_at],
    )?;
    Ok(())
}

/// List all platform entries.
pub fn list_platform(conn: &Connection) -> rusqlite::Result<Vec<PlatformEntry>> {
    let mut stmt = conn.prepare("SELECT key, value, detected_at FROM platform ORDER BY key")?;
    let rows = stmt.query_map([], |row| {
        Ok(PlatformEntry {
            key: row.get(0)?,
            value: row.get(1)?,
            detected_at: row.get(2)?,
        })
    })?;
    rows.collect()
}

/// Get a single platform entry by key.
pub fn get_platform(conn: &Connection, key: &str) -> rusqlite::Result<Option<PlatformEntry>> {
    conn.query_row(
        "SELECT key, value, detected_at FROM platform WHERE key = ?1",
        params![key],
        |row| {
            Ok(PlatformEntry {
                key: row.get(0)?,
                value: row.get(1)?,
                detected_at: row.get(2)?,
            })
        },
    )
    .optional()
}

/// Auto-detect platform info and store it.
pub fn detect_and_store_platform(conn: &Connection) -> rusqlite::Result<usize> {
    let now = now_iso();
    let mut count = 0;

    // OS
    let os = std::env::consts::OS;
    store_platform(conn, &PlatformEntry {
        key: "os".into(),
        value: os.into(),
        detected_at: now.clone(),
    })?;
    count += 1;

    // Architecture
    let arch = std::env::consts::ARCH;
    store_platform(conn, &PlatformEntry {
        key: "arch".into(),
        value: arch.into(),
        detected_at: now.clone(),
    })?;
    count += 1;

    // Shell
    if let Ok(shell) = std::env::var("SHELL") {
        store_platform(conn, &PlatformEntry {
            key: "shell".into(),
            value: shell,
            detected_at: now.clone(),
        })?;
        count += 1;
    }

    // Home directory
    if let Ok(home) = std::env::var("HOME") {
        store_platform(conn, &PlatformEntry {
            key: "home".into(),
            value: home,
            detected_at: now.clone(),
        })?;
        count += 1;
    }

    // Hostname
    if let Ok(hostname) = std::fs::read_to_string("/etc/hostname") {
        store_platform(conn, &PlatformEntry {
            key: "hostname".into(),
            value: hostname.trim().to_string(),
            detected_at: now,
        })?;
        count += 1;
    }

    Ok(count)
}

// ──────────────────────────────────────────────
// Layer 1: Tool ops
// ──────────────────────────────────────────────

/// Store or update a tool.
pub fn store_tool(conn: &Connection, tool: &Tool) -> rusqlite::Result<()> {
    let caps_json = serde_json::to_string(&tool.capabilities).unwrap_or_else(|_| "[]".to_string());
    conn.execute(
        "INSERT INTO tool (id, name, kind, capabilities, config, health, last_used, use_count, discovered_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
         ON CONFLICT(id) DO UPDATE SET
            name = excluded.name,
            kind = excluded.kind,
            capabilities = excluded.capabilities,
            config = excluded.config,
            health = excluded.health,
            last_used = excluded.last_used,
            use_count = excluded.use_count",
        params![
            tool.id,
            tool.name,
            tool_kind_str(&tool.kind),
            caps_json,
            tool.config,
            tool_health_str(&tool.health),
            tool.last_used,
            tool.use_count as i64,
            tool.discovered_at,
        ],
    )?;
    Ok(())
}

/// List all tools, optionally filtered by kind.
pub fn list_tools(conn: &Connection, kind_filter: Option<&ToolKind>) -> rusqlite::Result<Vec<Tool>> {
    if let Some(k) = kind_filter {
        let mut stmt = conn.prepare(
            "SELECT id, name, kind, capabilities, config, health, last_used, use_count, discovered_at
             FROM tool WHERE kind = ?1 ORDER BY name"
        )?;
        let rows = stmt.query_map(params![tool_kind_str(k)], row_to_tool)?;
        rows.collect()
    } else {
        let mut stmt = conn.prepare(
            "SELECT id, name, kind, capabilities, config, health, last_used, use_count, discovered_at
             FROM tool ORDER BY name"
        )?;
        let rows = stmt.query_map([], row_to_tool)?;
        rows.collect()
    }
}

/// Record a tool usage (increment use_count, update last_used).
pub fn record_tool_use(conn: &Connection, tool_id: &str) -> rusqlite::Result<bool> {
    let now = now_iso();
    let rows = conn.execute(
        "UPDATE tool SET use_count = use_count + 1, last_used = ?1 WHERE id = ?2",
        params![now, tool_id],
    )?;
    Ok(rows > 0)
}

fn row_to_tool(row: &rusqlite::Row) -> rusqlite::Result<Tool> {
    let caps_str: String = row.get(3)?;
    let capabilities: Vec<String> = serde_json::from_str(&caps_str).unwrap_or_default();
    Ok(Tool {
        id: row.get(0)?,
        name: row.get(1)?,
        kind: tool_kind_from_str(&row.get::<_, String>(2)?),
        capabilities,
        config: row.get(4)?,
        health: tool_health_from_str(&row.get::<_, String>(5)?),
        last_used: row.get(6)?,
        use_count: row.get::<_, i64>(7)? as u64,
        discovered_at: row.get(8)?,
    })
}

/// Check if a command exists on PATH (same pattern as lsp/detect.rs).
fn command_exists(cmd: &str) -> bool {
    std::process::Command::new("which")
        .arg(cmd)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Additional developer tools beyond the core known set.
/// Format: (binary_name, category, capabilities)
const EXTRA_TOOL_PATTERNS: &[(&str, &str, &[&str])] = &[
    // Languages/Runtimes
    ("python",   "runtime",          &["python-runtime"]),
    ("ruby",     "runtime",          &["ruby-runtime"]),
    ("go",       "runtime",          &["go-build", "go-test", "go-modules"]),
    ("java",     "runtime",          &["java-runtime"]),
    ("javac",    "compiler",         &["java-compiler"]),
    ("swift",    "compiler",         &["swift-compiler", "swift-build"]),
    ("zig",      "compiler",         &["zig-build", "zig-test"]),
    ("flutter",  "sdk",             &["flutter-build", "flutter-test", "dart-analyze"]),
    ("dart",     "runtime",          &["dart-runtime", "dart-analyze"]),
    ("deno",     "runtime",          &["deno-runtime", "deno-test"]),
    ("bun",      "runtime",          &["bun-runtime", "bun-test"]),

    // Package managers
    ("yarn",     "package-manager",  &["yarn-packages", "yarn-scripts"]),
    ("pnpm",     "package-manager",  &["pnpm-packages", "pnpm-scripts"]),
    ("pip3",     "package-manager",  &["python-packages"]),
    ("pipenv",   "package-manager",  &["python-venv"]),
    ("poetry",   "package-manager",  &["python-build", "python-packages"]),
    ("bundler",  "package-manager",  &["ruby-packages"]),
    ("gem",      "package-manager",  &["ruby-gems"]),
    ("gradle",   "build",           &["gradle-build", "gradle-test"]),
    ("mvn",      "build",           &["maven-build", "maven-test"]),
    ("cmake",    "build",           &["cmake-build", "cmake-config"]),
    ("bazel",    "build",           &["bazel-build", "bazel-test"]),

    // Test frameworks
    ("pytest",   "test",            &["python-test", "pytest-fixtures"]),
    ("jest",     "test",            &["js-test", "snapshot-test"]),
    ("vitest",   "test",            &["js-test", "vite-test"]),
    ("mocha",    "test",            &["js-test"]),
    ("rspec",    "test",            &["ruby-test"]),

    // Linters/Formatters
    ("eslint",           "lint",    &["js-lint"]),
    ("prettier",         "format",  &["js-format"]),
    ("ruff",             "lint",    &["python-lint", "python-format"]),
    ("flake8",           "lint",    &["python-lint"]),
    ("black",            "format",  &["python-format"]),
    ("rubocop",          "lint",    &["ruby-lint"]),
    ("golangci-lint",    "lint",    &["go-lint"]),
    ("clippy-driver",    "lint",    &["rust-lint"]),
    ("rustfmt",          "format",  &["rust-format"]),
    ("hadolint",         "lint",    &["dockerfile-lint"]),
    ("shellcheck",       "lint",    &["shell-lint"]),
    ("tflint",           "lint",    &["terraform-lint"]),

    // DevOps
    ("helm",      "deploy",         &["kubernetes-helm", "chart-management"]),
    ("ansible",   "deploy",         &["infrastructure-automation"]),
    ("vagrant",   "vm",             &["virtual-machines"]),
    ("podman",    "container",       &["containers"]),
    ("kind",      "kubernetes",      &["local-kubernetes"]),
    ("minikube",  "kubernetes",      &["local-kubernetes"]),

    // Cloud CLIs
    ("az",        "cloud",           &["azure"]),
    ("doctl",     "cloud",           &["digitalocean"]),
    ("flyctl",    "cloud",           &["fly-io"]),
    ("vercel",    "deploy",          &["vercel-deploy"]),
    ("netlify",   "deploy",          &["netlify-deploy"]),
    ("railway",   "deploy",          &["railway-deploy"]),

    // Editors/Tools
    ("code",      "editor",          &["vscode"]),
    ("nvim",      "editor",          &["neovim"]),
    ("vim",       "editor",          &["vim"]),
    ("emacs",     "editor",          &["emacs"]),
    ("screen",    "terminal",        &["terminal-multiplexer"]),

    // Search/Files
    ("fzf",       "search",          &["fuzzy-search"]),
    ("bat",       "viewer",          &["file-viewer"]),
    ("eza",       "search",          &["ls-replacement"]),
    ("delta",     "diff",            &["diff-viewer"]),
    ("tokei",     "analysis",        &["code-stats"]),
    ("hyperfine", "benchmark",       &["benchmarking"]),
];

/// Scan PATH for common developer tools and store in the tool table.
/// Phase 1: Known tools with rich, curated capabilities.
/// Phase 2: Extended discovery from EXTRA_TOOL_PATTERNS with category metadata.
/// Returns the number of tools found.
pub fn detect_and_store_tools(conn: &Connection) -> rusqlite::Result<usize> {
    // Phase 1: Known tools — curated with rich capabilities
    let known_tools: &[(&str, &[&str])] = &[
        ("git",       &["version-control", "diff", "merge", "branch"]),
        ("cargo",     &["rust-build", "rust-test", "rust-publish"]),
        ("rustc",     &["rust-compiler"]),
        ("npm",       &["node-packages", "node-scripts"]),
        ("node",      &["javascript-runtime", "node-scripts"]),
        ("python3",   &["python-runtime", "python-scripts"]),
        ("pip",       &["python-packages"]),
        ("docker",    &["containers", "images", "compose"]),
        ("kubectl",   &["kubernetes", "deployments", "pods"]),
        ("gh",        &["github-api", "issues", "pull-requests"]),
        ("make",      &["build-automation", "makefiles"]),
        ("curl",      &["http-client", "api-calls"]),
        ("jq",        &["json-processing"]),
        ("rg",        &["fast-search", "ripgrep"]),
        ("fd",        &["fast-find"]),
        ("terraform", &["infrastructure", "iac"]),
        ("gcloud",    &["gcp", "cloud"]),
        ("aws",       &["aws", "cloud"]),
        ("ssh",       &["remote-access", "ssh"]),
        ("scp",       &["file-transfer", "ssh"]),
        ("rsync",     &["file-sync"]),
        ("tmux",      &["terminal-multiplexer"]),
    ];

    let mut found = 0;
    let now = now_iso();

    for (name, caps) in known_tools {
        if command_exists(name) {
            let tool = Tool {
                id: format!("cli:{}", name),
                name: name.to_string(),
                kind: ToolKind::Cli,
                capabilities: caps.iter().map(|s| s.to_string()).collect(),
                config: None,
                health: ToolHealth::Healthy,
                last_used: None,
                use_count: 0,
                discovered_at: now.clone(),
            };
            store_tool(conn, &tool)?;
            found += 1;
        }
    }

    // Phase 2: Extended discovery — additional developer tools with category
    for (name, category, caps) in EXTRA_TOOL_PATTERNS {
        if command_exists(name) {
            let tool = Tool {
                id: format!("cli:{}", name),
                name: name.to_string(),
                kind: ToolKind::Cli,
                capabilities: caps.iter().map(|s| s.to_string()).collect(),
                config: Some(
                    serde_json::json!({"category": category}).to_string(),
                ),
                health: ToolHealth::Healthy,
                last_used: None,
                use_count: 0,
                discovered_at: now.clone(),
            };
            store_tool(conn, &tool)?;
            found += 1;
        }
    }

    eprintln!("[tools] discovered {} tools on PATH", found);
    Ok(found)
}

/// Get a set of available tool names for skill validation.
pub fn available_tool_names(conn: &Connection) -> rusqlite::Result<std::collections::HashSet<String>> {
    let tools = list_tools(conn, None)?;
    Ok(tools.into_iter().map(|t| t.name).collect())
}

// ──────────────────────────────────────────────
// Layer 2: Skill ops
// ──────────────────────────────────────────────

/// Store or update a skill.
pub fn store_skill(conn: &Connection, skill: &Skill) -> rusqlite::Result<()> {
    let steps_json = serde_json::to_string(&skill.steps).unwrap_or_else(|_| "[]".to_string());
    let correlation_ids_json = serde_json::to_string(&skill.correlation_ids).unwrap_or_else(|_| "[]".to_string());
    conn.execute(
        "INSERT INTO skill (id, name, domain, description, steps, success_count, fail_count, last_used, source, version, project, skill_type, user_specific, observed_count, correlation_ids)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)
         ON CONFLICT(id) DO UPDATE SET
            name = excluded.name,
            domain = excluded.domain,
            description = excluded.description,
            steps = excluded.steps,
            success_count = excluded.success_count,
            fail_count = excluded.fail_count,
            last_used = excluded.last_used,
            version = excluded.version,
            skill_type = excluded.skill_type,
            user_specific = excluded.user_specific,
            observed_count = excluded.observed_count,
            correlation_ids = excluded.correlation_ids",
        params![
            skill.id,
            skill.name,
            skill.domain,
            skill.description,
            steps_json,
            skill.success_count as i64,
            skill.fail_count as i64,
            skill.last_used,
            skill.source,
            skill.version as i64,
            skill.project,
            skill.skill_type,
            skill.user_specific as i32,
            skill.observed_count as i32,
            correlation_ids_json,
        ],
    )?;
    Ok(())
}

/// Store a skill, or if a similar one exists, increment its observed_count.
/// Used for behavioral skills where observing the same pattern again means
/// higher confidence rather than a duplicate entry.
pub fn store_or_observe_skill(conn: &Connection, skill: &Skill) -> rusqlite::Result<()> {
    // Check for existing skill with the same name (exact match)
    let existing: Option<(String, u32)> = conn.query_row(
        "SELECT id, observed_count FROM skill WHERE name = ?1 AND skill_type = 'behavioral'",
        params![skill.name],
        |row| Ok((row.get(0)?, row.get::<_, i32>(1).unwrap_or(1) as u32)),
    ).ok();

    if let Some((existing_id, count)) = existing {
        // Increment observation count
        conn.execute(
            "UPDATE skill SET observed_count = ?1, last_used = ?2 WHERE id = ?3",
            params![count as i32 + 1, now_offset(0), existing_id],
        )?;
        eprintln!("[skill-intelligence] observed pattern '{}' again (count: {})", skill.name, count + 1);
        return Ok(());
    }

    // Also check for fuzzy match: skills with overlapping first 3 words in name
    let words: Vec<&str> = skill.name.split_whitespace().take(3).collect();
    if words.len() >= 2 {
        let fuzzy_pattern = format!("%{}%", words.join("%"));
        let fuzzy_match: Option<(String, u32)> = conn.query_row(
            "SELECT id, observed_count FROM skill WHERE name LIKE ?1 AND skill_type = 'behavioral'",
            params![fuzzy_pattern],
            |row| Ok((row.get(0)?, row.get::<_, i32>(1).unwrap_or(1) as u32)),
        ).ok();

        if let Some((existing_id, count)) = fuzzy_match {
            conn.execute(
                "UPDATE skill SET observed_count = ?1, last_used = ?2 WHERE id = ?3",
                params![count as i32 + 1, now_offset(0), existing_id],
            )?;
            eprintln!("[skill-intelligence] observed similar pattern '{}' again (count: {})", skill.name, count + 1);
            return Ok(());
        }
    }

    // New skill — store it
    store_skill(conn, skill)
}

/// Find and create correlations between a behavioral skill and related memories.
/// Returns the number of correlation edges created.
pub fn correlate_skill(conn: &Connection, skill_id: &str, skill_title: &str, skill_tags: &[String]) -> rusqlite::Result<usize> {
    let mut correlated = 0;

    // Find identity facets with similar description
    let words: Vec<&str> = skill_title.split_whitespace().take(3).collect();
    if words.len() >= 2 {
        let identity_pattern = format!("%{}%", words.join("%"));
        let mut stmt = conn.prepare(
            "SELECT id FROM identity WHERE active = 1 AND description LIKE ?1"
        )?;
        let identity_matches: Vec<String> = stmt.query_map(
            params![identity_pattern],
            |row| row.get(0),
        )?.filter_map(|r| r.ok()).collect();

        for id in &identity_matches {
            if let Err(e) = crate::db::ops::store_edge(conn, skill_id, id, "correlates_with", "{}") {
                eprintln!("[skill-intelligence] correlation edge error: {e}");
            } else {
                correlated += 1;
            }
        }
    }

    // Find decisions with matching tags
    for tag in skill_tags {
        if tag == "behavioral" { continue; }
        let mut stmt = conn.prepare(
            "SELECT id FROM memory WHERE memory_type = 'decision' AND status = 'active' AND tags LIKE ?1"
        )?;
        let decision_matches: Vec<String> = stmt.query_map(
            params![format!("%{}%", tag)],
            |row| row.get(0),
        )?.filter_map(|r| r.ok()).collect();

        for id in &decision_matches {
            if let Err(e) = crate::db::ops::store_edge(conn, skill_id, id, "correlates_with", "{}") {
                eprintln!("[skill-intelligence] correlation edge error: {e}");
            } else {
                correlated += 1;
            }
        }
    }

    // Update correlation_ids on the skill itself
    if correlated > 0 {
        let mut edge_ids: Vec<String> = Vec::new();
        let mut stmt = conn.prepare(
            "SELECT to_id FROM edge WHERE from_id = ?1 AND edge_type = 'correlates_with'"
        )?;
        let rows = stmt.query_map(params![skill_id], |row| row.get(0))?;
        for id in rows.flatten() {
            edge_ids.push(id);
        }
        let ids_json = serde_json::to_string(&edge_ids).unwrap_or_else(|_| "[]".to_string());
        conn.execute(
            "UPDATE skill SET correlation_ids = ?1 WHERE id = ?2",
            params![ids_json, skill_id],
        )?;
        eprintln!("[skill-intelligence] created {} correlations for '{}'", correlated, skill_title);
    }

    Ok(correlated)
}

/// List all skills, optionally filtered by domain.
pub fn list_skills(conn: &Connection, domain_filter: Option<&str>) -> rusqlite::Result<Vec<Skill>> {
    if let Some(d) = domain_filter {
        let mut stmt = conn.prepare(
            "SELECT id, name, domain, description, steps, success_count, fail_count, last_used, source, version, project, skill_type, user_specific, observed_count, correlation_ids
             FROM skill WHERE domain = ?1 ORDER BY name"
        )?;
        let rows = stmt.query_map(params![d], row_to_skill)?;
        rows.collect()
    } else {
        let mut stmt = conn.prepare(
            "SELECT id, name, domain, description, steps, success_count, fail_count, last_used, source, version, project, skill_type, user_specific, observed_count, correlation_ids
             FROM skill ORDER BY name"
        )?;
        let rows = stmt.query_map([], row_to_skill)?;
        rows.collect()
    }
}

fn row_to_skill(row: &rusqlite::Row) -> rusqlite::Result<Skill> {
    let steps_str: String = row.get(4)?;
    let steps: Vec<String> = serde_json::from_str(&steps_str).unwrap_or_default();
    let correlation_ids_str: String = row.get::<_, String>(14).unwrap_or_else(|_| "[]".to_string());
    let correlation_ids: Vec<String> = serde_json::from_str(&correlation_ids_str).unwrap_or_default();
    Ok(Skill {
        id: row.get(0)?,
        name: row.get(1)?,
        domain: row.get(2)?,
        description: row.get(3)?,
        steps,
        success_count: row.get::<_, i64>(5)? as u64,
        fail_count: row.get::<_, i64>(6)? as u64,
        last_used: row.get(7)?,
        source: row.get(8)?,
        version: row.get::<_, i64>(9)? as u64,
        project: row.get(10).ok().unwrap_or(None),
        skill_type: row.get::<_, String>(11).unwrap_or_else(|_| "procedural".to_string()),
        user_specific: row.get::<_, i32>(12).unwrap_or(0) != 0,
        observed_count: row.get::<_, i32>(13).unwrap_or(1) as u32,
        correlation_ids,
    })
}

/// Search skills by name, description, or domain keyword (LIKE search).
/// Respects project scoping: returns skills for the given project + global skills (project IS NULL).
pub fn search_skills(conn: &Connection, query: &str, project: Option<&str>) -> rusqlite::Result<Vec<Skill>> {
    let search = format!("%{}%", query);
    if let Some(proj) = project {
        let mut stmt = conn.prepare(
            "SELECT id, name, domain, description, steps, success_count, fail_count, last_used, source, version, project, skill_type, user_specific, observed_count, correlation_ids
             FROM skill WHERE (name LIKE ?1 OR description LIKE ?1 OR domain LIKE ?1)
             AND (project = ?2 OR project IS NULL OR project = '')
             ORDER BY success_count DESC LIMIT 5"
        )?;
        let rows = stmt.query_map(params![search, proj], row_to_skill)?;
        rows.collect()
    } else {
        let mut stmt = conn.prepare(
            "SELECT id, name, domain, description, steps, success_count, fail_count, last_used, source, version, project, skill_type, user_specific, observed_count, correlation_ids
             FROM skill WHERE name LIKE ?1 OR description LIKE ?1 OR domain LIKE ?1
             ORDER BY success_count DESC LIMIT 5"
        )?;
        let rows = stmt.query_map(params![search], row_to_skill)?;
        rows.collect()
    }
}

/// Record a skill execution result (success or failure).
///
/// The format! for the field name is safe because `field` is a hardcoded
/// string literal ("success_count" or "fail_count"), not user input.
pub fn record_skill_result(conn: &Connection, skill_id: &str, success: bool) -> rusqlite::Result<bool> {
    // Safe: `field` is a compile-time literal, not user input
    let field = if success { "success_count" } else { "fail_count" };
    let updated = conn.execute(
        &format!("UPDATE skill SET {} = {} + 1, last_used = datetime('now') WHERE id = ?1", field, field),
        params![skill_id],
    )?;
    Ok(updated > 0)
}

/// Prune low-quality skills (no steps, short descriptions, status-like names).
/// Only removes skills with zero success_count to avoid deleting proven workflows.
/// Behavioral skills are exempt from the "no steps" check since they don't have steps.
pub fn prune_junk_skills(conn: &Connection) -> rusqlite::Result<usize> {
    conn.execute(
        "DELETE FROM skill WHERE
         skill_type != 'behavioral'
         AND (steps = '[]' OR steps = '' OR LENGTH(description) < 50)
         AND success_count = 0",
        [],
    )
}

// ──────────────────────────────────────────────
// Layer 3: Domain DNA ops
// ──────────────────────────────────────────────

/// Store or update a domain DNA entry.
pub fn store_domain_dna(conn: &Connection, dna: &DomainDna) -> rusqlite::Result<()> {
    let evidence_json = serde_json::to_string(&dna.evidence).unwrap_or_else(|_| "[]".to_string());
    conn.execute(
        "INSERT INTO domain_dna (id, project, aspect, pattern, confidence, evidence, detected_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT(id) DO UPDATE SET
            project = excluded.project,
            aspect = excluded.aspect,
            pattern = excluded.pattern,
            confidence = excluded.confidence,
            evidence = excluded.evidence,
            detected_at = excluded.detected_at",
        params![
            dna.id,
            dna.project,
            dna.aspect,
            dna.pattern,
            dna.confidence,
            evidence_json,
            dna.detected_at,
        ],
    )?;
    Ok(())
}

/// List domain DNA entries, optionally filtered by project.
pub fn list_domain_dna(conn: &Connection, project_filter: Option<&str>) -> rusqlite::Result<Vec<DomainDna>> {
    if let Some(p) = project_filter {
        let mut stmt = conn.prepare(
            "SELECT id, project, aspect, pattern, confidence, evidence, detected_at
             FROM domain_dna WHERE project = ?1 ORDER BY aspect"
        )?;
        let rows = stmt.query_map(params![p], row_to_domain_dna)?;
        rows.collect()
    } else {
        let mut stmt = conn.prepare(
            "SELECT id, project, aspect, pattern, confidence, evidence, detected_at
             FROM domain_dna ORDER BY aspect"
        )?;
        let rows = stmt.query_map([], row_to_domain_dna)?;
        rows.collect()
    }
}

fn row_to_domain_dna(row: &rusqlite::Row) -> rusqlite::Result<DomainDna> {
    let evidence_str: String = row.get(5)?;
    let evidence: Vec<String> = serde_json::from_str(&evidence_str).unwrap_or_default();
    Ok(DomainDna {
        id: row.get(0)?,
        project: row.get(1)?,
        aspect: row.get(2)?,
        pattern: row.get(3)?,
        confidence: row.get(4)?,
        evidence,
        detected_at: row.get(6)?,
    })
}

// ──────────────────────────────────────────────
// Layer 4: Perception ops
// ──────────────────────────────────────────────

/// Store a perception event. Uses INSERT OR REPLACE so repeated calls with the same ID
/// update the perception rather than failing.
pub fn store_perception(conn: &Connection, p: &Perception) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT OR REPLACE INTO perception (id, kind, data, severity, project, created_at, expires_at, consumed)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            p.id,
            perception_kind_str(&p.kind),
            p.data,
            severity_str(&p.severity),
            p.project,
            p.created_at,
            p.expires_at,
            p.consumed as i32,
        ],
    )?;
    Ok(())
}

/// List unconsumed perceptions, optionally filtered by kind.
pub fn list_unconsumed_perceptions(
    conn: &Connection,
    kind_filter: Option<&PerceptionKind>,
) -> rusqlite::Result<Vec<Perception>> {
    if let Some(k) = kind_filter {
        let mut stmt = conn.prepare(
            "SELECT id, kind, data, severity, project, created_at, expires_at, consumed
             FROM perception WHERE consumed = 0 AND kind = ?1 ORDER BY created_at DESC"
        )?;
        let rows = stmt.query_map(params![perception_kind_str(k)], row_to_perception)?;
        rows.collect()
    } else {
        let mut stmt = conn.prepare(
            "SELECT id, kind, data, severity, project, created_at, expires_at, consumed
             FROM perception WHERE consumed = 0 ORDER BY created_at DESC"
        )?;
        let rows = stmt.query_map([], row_to_perception)?;
        rows.collect()
    }
}

/// Delete perceptions whose expires_at has passed.
pub fn expire_perceptions(conn: &Connection) -> rusqlite::Result<usize> {
    let now = now_iso();
    let rows = conn.execute(
        "DELETE FROM perception WHERE expires_at IS NOT NULL AND expires_at < ?1",
        params![now],
    )?;
    Ok(rows)
}

/// Generate a timestamp string offset by `delta_secs` from now.
/// Positive values are in the future, negative in the past.
pub fn now_offset(delta_secs: i64) -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
        + delta_secs;
    epoch_to_iso(secs as u64)
}

fn epoch_to_iso(secs: u64) -> String {
    let days_since_epoch = secs / 86400;
    let time_of_day = secs % 86400;

    let mut year = 1970u64;
    let mut remaining_days = days_since_epoch;
    loop {
        let is_leap =
            year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400));
        let days_in_year = if is_leap { 366 } else { 365 };
        if remaining_days < days_in_year {
            break;
        }
        remaining_days -= days_in_year;
        year += 1;
    }

    let is_leap =
        year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400));
    let month_days: [u64; 12] = if is_leap {
        [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    } else {
        [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    };

    let mut month = 1u64;
    for &days in &month_days {
        if remaining_days < days {
            break;
        }
        remaining_days -= days;
        month += 1;
    }
    let day = remaining_days + 1;

    let hours = time_of_day / 3600;
    let minutes = (time_of_day % 3600) / 60;
    let seconds = time_of_day % 60;

    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
        year, month, day, hours, minutes, seconds
    )
}

/// Mark a perception as consumed.
pub fn consume_perception(conn: &Connection, id: &str) -> rusqlite::Result<bool> {
    let rows = conn.execute(
        "UPDATE perception SET consumed = 1 WHERE id = ?1 AND consumed = 0",
        params![id],
    )?;
    Ok(rows > 0)
}

fn row_to_perception(row: &rusqlite::Row) -> rusqlite::Result<Perception> {
    Ok(Perception {
        id: row.get(0)?,
        kind: perception_kind_from_str(&row.get::<_, String>(1)?),
        data: row.get(2)?,
        severity: severity_from_str(&row.get::<_, String>(3)?),
        project: row.get(4)?,
        created_at: row.get(5)?,
        expires_at: row.get(6)?,
        consumed: row.get::<_, i32>(7)? != 0,
    })
}

// ──────────────────────────────────────────────
// Layer 5: Declared Knowledge ops
// ──────────────────────────────────────────────

/// Store a declared knowledge entry.
pub fn store_declared(conn: &Connection, d: &Declared) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO declared (id, source, path, content, hash, project, ingested_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT(id) DO UPDATE SET
            source = excluded.source,
            path = excluded.path,
            content = excluded.content,
            hash = excluded.hash,
            project = excluded.project,
            ingested_at = excluded.ingested_at",
        params![d.id, d.source, d.path, d.content, d.hash, d.project, d.ingested_at],
    )?;
    Ok(())
}

/// List declared entries, optionally filtered by project.
pub fn list_declared(conn: &Connection, project_filter: Option<&str>) -> rusqlite::Result<Vec<Declared>> {
    if let Some(p) = project_filter {
        let mut stmt = conn.prepare(
            "SELECT id, source, path, content, hash, project, ingested_at
             FROM declared WHERE project = ?1 ORDER BY ingested_at DESC"
        )?;
        let rows = stmt.query_map(params![p], row_to_declared)?;
        rows.collect()
    } else {
        let mut stmt = conn.prepare(
            "SELECT id, source, path, content, hash, project, ingested_at
             FROM declared ORDER BY ingested_at DESC"
        )?;
        let rows = stmt.query_map([], row_to_declared)?;
        rows.collect()
    }
}

/// Get a declared entry by hash (for dedup).
pub fn get_declared_by_hash(conn: &Connection, hash: &str) -> rusqlite::Result<Option<Declared>> {
    conn.query_row(
        "SELECT id, source, path, content, hash, project, ingested_at
         FROM declared WHERE hash = ?1",
        params![hash],
        row_to_declared,
    )
    .optional()
}

/// Search declared knowledge entries by content or source using LIKE.
pub fn search_declared(conn: &Connection, query: &str, project: Option<&str>) -> rusqlite::Result<Vec<Declared>> {
    let search = format!("%{}%", query);
    match project {
        Some(p) => {
            let mut stmt = conn.prepare(
                "SELECT id, source, path, content, hash, project, ingested_at FROM declared
                 WHERE (content LIKE ?1 OR source LIKE ?1) AND (project = ?2 OR project IS NULL)
                 ORDER BY ingested_at DESC LIMIT 5",
            )?;
            let rows = stmt.query_map(params![search, p], row_to_declared)?;
            rows.collect()
        }
        None => {
            let mut stmt = conn.prepare(
                "SELECT id, source, path, content, hash, project, ingested_at FROM declared
                 WHERE content LIKE ?1 OR source LIKE ?1
                 ORDER BY ingested_at DESC LIMIT 5",
            )?;
            let rows = stmt.query_map(params![search], row_to_declared)?;
            rows.collect()
        }
    }
}

/// Ingest a file (e.g. CLAUDE.md) as declared knowledge with content-hash dedup.
pub fn ingest_declared_file(conn: &Connection, path: &str, source: &str, project: Option<&str>) -> rusqlite::Result<bool> {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return Ok(false),
    };

    // Hash the content for change detection
    let mut hasher = DefaultHasher::new();
    content.hash(&mut hasher);
    let hash = format!("{:x}", hasher.finish());

    // Check if already ingested with same hash
    if get_declared_by_hash(conn, &hash)?.is_some() {
        return Ok(false); // Already up to date
    }

    let id = format!("declared-{}", ulid::Ulid::new());
    let declared = Declared {
        id,
        source: source.to_string(),
        path: Some(path.to_string()),
        content,
        hash,
        project: project.map(|s| s.to_string()),
        ingested_at: now_iso(),
    };
    store_declared(conn, &declared)?;
    Ok(true)
}

fn row_to_declared(row: &rusqlite::Row) -> rusqlite::Result<Declared> {
    Ok(Declared {
        id: row.get(0)?,
        source: row.get(1)?,
        path: row.get(2)?,
        content: row.get(3)?,
        hash: row.get(4)?,
        project: row.get(5)?,
        ingested_at: row.get(6)?,
    })
}

// ──────────────────────────────────────────────
// Project ingestion: Declared Knowledge + Domain DNA
// ──────────────────────────────────────────────

/// Ingest project documentation files into Declared Knowledge (Layer 5).
/// Scans for CLAUDE.md, README.md, AGENTS.md, .cursorrules, GEMINI.md in the project root.
/// Uses content hash for idempotency — skips unchanged files.
pub fn ingest_project_declared(conn: &Connection, project_dir: &str) -> rusqlite::Result<(usize, usize)> {
    let doc_files = ["CLAUDE.md", "README.md", "AGENTS.md", ".cursorrules", "GEMINI.md"];
    let mut ingested = 0usize;
    let mut skipped = 0usize;

    let project_name = std::path::Path::new(project_dir)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown")
        .to_string();

    for filename in &doc_files {
        let path = format!("{}/{}", project_dir, filename);
        if !std::path::Path::new(&path).exists() {
            continue;
        }

        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("[declared] ERROR reading {}: {}", path, e);
                continue;
            }
        };
        if content.trim().is_empty() {
            continue;
        }

        // Content hash for idempotency (same approach as ingest_declared_file)
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let mut hasher = DefaultHasher::new();
        content.hash(&mut hasher);
        let hash = format!("{:x}", hasher.finish());

        // Check if already ingested with same hash
        if get_declared_by_hash(conn, &hash)?.is_some() {
            skipped += 1;
            continue;
        }

        // Determine source type
        let source = match *filename {
            "CLAUDE.md" => "claude_md",
            "README.md" => "readme",
            "AGENTS.md" => "agents_md",
            ".cursorrules" => "cursor_rules",
            "GEMINI.md" => "gemini_md",
            _ => "other",
        };

        // Truncate content to 10KB for storage (declared knowledge is high-level)
        let stored_content = if content.len() > 10_000 {
            // Find a safe UTF-8 boundary near 10KB
            let mut end = 10_000;
            while !content.is_char_boundary(end) && end > 0 {
                end -= 1;
            }
            content[..end].to_string()
        } else {
            content.clone()
        };

        let id = format!("declared-{}", ulid::Ulid::new());
        let declared = Declared {
            id,
            source: source.to_string(),
            path: Some(path.clone()),
            content: stored_content,
            hash,
            project: Some(project_name.clone()),
            ingested_at: now_iso(),
        };
        store_declared(conn, &declared)?;
        ingested += 1;
        eprintln!("[declared] ingested {} ({} bytes)", filename, content.len());
    }

    Ok((ingested, skipped))
}

/// Detect project type and conventions from marker files, storing into Domain DNA (Layer 3).
/// Idempotent — uses deterministic IDs so re-runs update in place.
pub fn detect_domain_dna(conn: &Connection, project_dir: &str) -> rusqlite::Result<usize> {
    let project_name = std::path::Path::new(project_dir)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown")
        .to_string();

    let markers: &[(&str, &str, &[&str])] = &[
        ("Cargo.toml", "rust", &["cargo test", "cargo clippy", "cargo build"]),
        ("package.json", "javascript/typescript", &["npm test", "npm run lint", "npm run build"]),
        ("pubspec.yaml", "flutter/dart", &["flutter test", "dart analyze", "flutter build"]),
        ("pyproject.toml", "python", &["pytest", "ruff check", "python -m build"]),
        ("setup.py", "python", &["pytest", "flake8", "python setup.py build"]),
        ("go.mod", "go", &["go test ./...", "golangci-lint run", "go build"]),
        ("build.gradle", "java/kotlin", &["gradle test", "gradle check", "gradle build"]),
        ("Gemfile", "ruby", &["rspec", "rubocop", "bundle exec rake build"]),
        ("CMakeLists.txt", "c/cpp", &["ctest", "clang-tidy", "cmake --build"]),
        ("Makefile", "make", &["make test", "make lint", "make"]),
        ("Dockerfile", "container", &["docker build", "hadolint", "docker compose"]),
    ];

    let mut detected = 0usize;
    let now = now_iso();

    for (file, lang, commands) in markers {
        let path = format!("{}/{}", project_dir, file);
        if !std::path::Path::new(&path).exists() {
            continue;
        }

        // Store language/framework detection — unique per marker file
        // so multiple languages in one repo coexist (e.g., Cargo.toml + Dockerfile)
        let dna = DomainDna {
            id: format!("dna-{}-lang-{}", project_name, lang),
            project: project_name.clone(),
            aspect: "language".to_string(),
            pattern: lang.to_string(),
            confidence: 0.95,
            evidence: vec![file.to_string()],
            detected_at: now.clone(),
        };
        store_domain_dna(conn, &dna)?;

        // Store test/lint/build commands — unique per language
        for (i, cmd) in commands.iter().enumerate() {
            let aspect = match i {
                0 => "test_command",
                1 => "lint_command",
                2 => "build_command",
                _ => continue,
            };
            let dna = DomainDna {
                id: format!("dna-{}-{}-{}", project_name, lang, aspect),
                project: project_name.clone(),
                aspect: aspect.to_string(),
                pattern: cmd.to_string(),
                confidence: 0.9,
                evidence: vec![file.to_string()],
                detected_at: now.clone(),
            };
            store_domain_dna(conn, &dna)?;
        }

        detected += 1;
        eprintln!("[domain-dna] detected {} project (from {})", lang, file);
    }

    Ok(detected)
}

// ──────────────────────────────────────────────
// Layer 6: Identity ops
// ──────────────────────────────────────────────

/// Store or update an identity facet.
///
/// Deduplicates by (agent, description): if an active facet with the same
/// agent and description already exists, the one with higher strength wins
/// and the insert is skipped. This prevents the extractor from creating
/// hundreds of near-identical identity facets over time.
pub fn store_identity(conn: &Connection, facet: &IdentityFacet) -> rusqlite::Result<()> {
    // Dedup check: merge with existing facet that has the same description for this agent
    let existing: Option<(String, f64)> = conn
        .query_row(
            "SELECT id, strength FROM identity WHERE agent = ?1 AND description = ?2 AND active = 1",
            params![facet.agent, facet.description],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .ok();

    if let Some((existing_id, existing_strength)) = existing {
        // Merge: keep higher strength, skip the insert
        if facet.strength > existing_strength {
            conn.execute(
                "UPDATE identity SET strength = ?1 WHERE id = ?2",
                params![facet.strength, existing_id],
            )?;
        }
        return Ok(());
    }

    // No duplicate — insert normally
    conn.execute(
        "INSERT INTO identity (id, agent, facet, description, strength, source, active, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
         ON CONFLICT(id) DO UPDATE SET
            agent = excluded.agent,
            facet = excluded.facet,
            description = excluded.description,
            strength = excluded.strength,
            source = excluded.source,
            active = excluded.active",
        params![
            facet.id,
            facet.agent,
            facet.facet,
            facet.description,
            facet.strength,
            facet.source,
            facet.active as i32,
            facet.created_at,
        ],
    )?;
    Ok(())
}

/// List identity facets for an agent, optionally only active ones.
pub fn list_identity(
    conn: &Connection,
    agent: &str,
    active_only: bool,
) -> rusqlite::Result<Vec<IdentityFacet>> {
    let sql = if active_only {
        "SELECT id, agent, facet, description, strength, source, active, created_at
         FROM identity WHERE agent = ?1 AND active = 1 ORDER BY strength DESC"
    } else {
        "SELECT id, agent, facet, description, strength, source, active, created_at
         FROM identity WHERE agent = ?1 ORDER BY strength DESC"
    };

    let mut stmt = conn.prepare(sql)?;
    let rows = stmt.query_map(params![agent], row_to_identity)?;
    rows.collect()
}

/// List identity facets for a user (across all agents), falling back to agent-only if user_id is None.
///
/// When user_id is provided, returns facets that either belong to that user or have no user_id
/// (shared/system facets), filtered by agent. When user_id is None, delegates to `list_identity`.
pub fn list_identity_for_user(
    conn: &Connection,
    user_id: Option<&str>,
    agent: &str,
    only_active: bool,
) -> rusqlite::Result<Vec<IdentityFacet>> {
    match user_id {
        Some(uid) => {
            let sql = if only_active {
                "SELECT id, agent, facet, description, strength, source, active, created_at
                 FROM identity
                 WHERE (user_id = ?1 OR user_id IS NULL) AND agent = ?2
                 AND active = 1
                 ORDER BY strength DESC"
            } else {
                "SELECT id, agent, facet, description, strength, source, active, created_at
                 FROM identity
                 WHERE (user_id = ?1 OR user_id IS NULL) AND agent = ?2
                 ORDER BY strength DESC"
            };
            let mut stmt = conn.prepare(sql)?;
            let rows = stmt.query_map(params![uid, agent], row_to_identity)?;
            rows.collect()
        }
        None => list_identity(conn, agent, only_active),
    }
}

/// Deactivate an identity facet.
pub fn deactivate_identity(conn: &Connection, id: &str) -> rusqlite::Result<bool> {
    let rows = conn.execute(
        "UPDATE identity SET active = 0 WHERE id = ?1 AND active = 1",
        params![id],
    )?;
    Ok(rows > 0)
}

fn row_to_identity(row: &rusqlite::Row) -> rusqlite::Result<IdentityFacet> {
    Ok(IdentityFacet {
        id: row.get(0)?,
        agent: row.get(1)?,
        facet: row.get(2)?,
        description: row.get(3)?,
        strength: row.get(4)?,
        source: row.get(5)?,
        active: row.get::<_, i32>(6)? != 0,
        created_at: row.get(7)?,
    })
}

// ──────────────────────────────────────────────
// Layer 7: Disposition ops
// ──────────────────────────────────────────────

/// Store or update a disposition.
pub fn store_disposition(conn: &Connection, d: &Disposition) -> rusqlite::Result<()> {
    let evidence_json = serde_json::to_string(&d.evidence).unwrap_or_else(|_| "[]".to_string());
    conn.execute(
        "INSERT INTO disposition (id, agent, trait_name, domain, value, trend, updated_at, evidence)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
         ON CONFLICT(id) DO UPDATE SET
            agent = excluded.agent,
            trait_name = excluded.trait_name,
            domain = excluded.domain,
            value = excluded.value,
            trend = excluded.trend,
            updated_at = excluded.updated_at,
            evidence = excluded.evidence",
        params![
            d.id,
            d.agent,
            disposition_trait_str(&d.disposition_trait),
            d.domain,
            d.value,
            trend_str(&d.trend),
            d.updated_at,
            evidence_json,
        ],
    )?;
    Ok(())
}

/// List dispositions for an agent.
pub fn list_dispositions(conn: &Connection, agent: &str) -> rusqlite::Result<Vec<Disposition>> {
    let mut stmt = conn.prepare(
        "SELECT id, agent, trait_name, domain, value, trend, updated_at, evidence
         FROM disposition WHERE agent = ?1 ORDER BY trait_name"
    )?;
    let rows = stmt.query_map(params![agent], row_to_disposition)?;
    rows.collect()
}

fn row_to_disposition(row: &rusqlite::Row) -> rusqlite::Result<Disposition> {
    let evidence_str: String = row.get(7)?;
    let evidence: Vec<String> = serde_json::from_str(&evidence_str).unwrap_or_default();
    Ok(Disposition {
        id: row.get(0)?,
        agent: row.get(1)?,
        disposition_trait: disposition_trait_from_str(&row.get::<_, String>(2)?),
        domain: row.get(3)?,
        value: row.get(4)?,
        trend: trend_from_str(&row.get::<_, String>(5)?),
        updated_at: row.get(6)?,
        evidence,
    })
}

// ──────────────────────────────────────────────
// Manas Health
// ──────────────────────────────────────────────

/// Health report across all Manas layers.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ManasHealth {
    pub platform_entries: usize,
    pub tools: usize,
    pub skills: usize,
    pub domain_dna_entries: usize,
    pub experience_count: usize,
    pub perceptions_unconsumed: usize,
    pub declared_entries: usize,
    pub embedding_count: usize,
    pub identity_facets_active: usize,
    pub dispositions: usize,
    pub trait_names: Vec<String>,
}

/// Gather health counts across all 8 Manas layers.
pub fn manas_health(conn: &Connection) -> rusqlite::Result<ManasHealth> {
    // SAFETY: table and where_clause are always hardcoded literals from the callers below.
    // No user input flows into this SQL. Using format! for DRY convenience only.
    let count_table = |table: &str, where_clause: &str| -> rusqlite::Result<usize> {
        let sql = format!("SELECT COUNT(*) FROM {}{}", table, where_clause);
        conn.query_row(&sql, [], |row| row.get::<_, i64>(0))
            .map(|n| n as usize)
    };

    // Trait names for disposition display
    let trait_names: Vec<String> = conn.prepare(
        "SELECT DISTINCT trait_name FROM disposition"
    ).and_then(|mut stmt| {
        stmt.query_map([], |row| row.get(0))?.collect()
    }).unwrap_or_default();

    Ok(ManasHealth {
        platform_entries: count_table("platform", "")?,
        tools: count_table("tool", "")?,
        skills: count_table("skill", "")?,
        domain_dna_entries: count_table("domain_dna", "")?,
        experience_count: count_table("memory", " WHERE status = 'active'")?,
        perceptions_unconsumed: count_table("perception", " WHERE consumed = 0")?,
        declared_entries: count_table("declared", "")?,
        embedding_count: crate::db::vec::count_embeddings(conn).unwrap_or(0),
        identity_facets_active: count_table("identity", " WHERE active = 1")?,
        dispositions: count_table("disposition", "")?,
        trait_names,
    })
}

/// Returns true if the project has zero active memories in the memory table.
/// Used to detect brand-new projects that need onboarding guidance.
pub fn is_new_project(conn: &Connection, project: &str) -> rusqlite::Result<bool> {
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM memory WHERE project = ?1 AND status = 'active'",
        params![project],
        |row| row.get(0),
    )?;
    Ok(count == 0)
}

// ──────────────────────────────────────────────
// Knowledge Intelligence: Entity operations
// ──────────────────────────────────────────────

/// Store or update an entity. Upserts by name+project:
/// if an entity with the same name and project exists, updates
/// mention_count, last_seen, and description (if non-empty).
pub fn store_entity(conn: &Connection, entity: &Entity) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO entity (id, name, entity_type, description, mention_count, first_seen, last_seen, project)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
         ON CONFLICT(id) DO UPDATE SET
            mention_count = excluded.mention_count,
            last_seen = excluded.last_seen,
            description = CASE WHEN excluded.description != '' THEN excluded.description ELSE entity.description END",
        params![
            entity.id,
            entity.name,
            entity.entity_type,
            entity.description,
            entity.mention_count as i64,
            entity.first_seen,
            entity.last_seen,
            entity.project,
        ],
    )?;
    Ok(())
}

/// List entities, optionally filtered by project. Ordered by mention_count descending.
pub fn list_entities(
    conn: &Connection,
    project: Option<&str>,
    limit: usize,
) -> rusqlite::Result<Vec<Entity>> {
    if let Some(proj) = project {
        let mut stmt = conn.prepare(
            "SELECT id, name, entity_type, description, mention_count, first_seen, last_seen, project
             FROM entity WHERE project = ?1 ORDER BY mention_count DESC LIMIT ?2"
        )?;
        let rows = stmt.query_map(params![proj, limit as i64], row_to_entity)?;
        rows.collect()
    } else {
        let mut stmt = conn.prepare(
            "SELECT id, name, entity_type, description, mention_count, first_seen, last_seen, project
             FROM entity ORDER BY mention_count DESC LIMIT ?1"
        )?;
        let rows = stmt.query_map(params![limit as i64], row_to_entity)?;
        rows.collect()
    }
}

/// Upsert an entity by name+project: if it exists, increment mention_count and
/// update last_seen. If it doesn't exist, create it with mention_count=1.
pub fn increment_entity_mention(
    conn: &Connection,
    name: &str,
    project: Option<&str>,
) -> rusqlite::Result<()> {
    let now = now_iso();
    // Try to find existing entity with same name+project
    let existing_id: Option<String> = if let Some(proj) = project {
        conn.query_row(
            "SELECT id FROM entity WHERE name = ?1 AND project = ?2",
            params![name, proj],
            |row| row.get(0),
        ).optional()?
    } else {
        conn.query_row(
            "SELECT id FROM entity WHERE name = ?1 AND project IS NULL",
            params![name],
            |row| row.get(0),
        ).optional()?
    };

    if let Some(id) = existing_id {
        conn.execute(
            "UPDATE entity SET mention_count = mention_count + 1, last_seen = ?1 WHERE id = ?2",
            params![now, id],
        )?;
    } else {
        let id = format!("ent-{}", ulid::Ulid::new());
        conn.execute(
            "INSERT INTO entity (id, name, entity_type, description, mention_count, first_seen, last_seen, project)
             VALUES (?1, ?2, 'concept', '', 1, ?3, ?3, ?4)",
            params![id, name, now, project],
        )?;
    }
    Ok(())
}

fn row_to_entity(row: &rusqlite::Row) -> rusqlite::Result<Entity> {
    Ok(Entity {
        id: row.get(0)?,
        name: row.get(1)?,
        entity_type: row.get(2)?,
        description: row.get(3)?,
        mention_count: row.get::<_, i64>(4)? as u64,
        first_seen: row.get(5)?,
        last_seen: row.get(6)?,
        project: row.get(7)?,
    })
}

/// Detect entities from memory titles by word frequency analysis.
/// Words (lowercased, stripped of punctuation) appearing in 3+ distinct
/// active memory titles are upserted as entities.
/// Returns the number of entities created or updated.
pub fn detect_entities(conn: &Connection) -> rusqlite::Result<usize> {
    // Collect all active memory titles
    let mut stmt = conn.prepare(
        "SELECT id, title, project FROM memory WHERE status = 'active'"
    )?;

    struct MemRow {
        id: String,
        title: String,
        project: Option<String>,
    }

    let rows: Vec<MemRow> = stmt.query_map([], |row| {
        Ok(MemRow {
            id: row.get(0)?,
            title: row.get(1)?,
            project: row.get(2)?,
        })
    })?.filter_map(|r| r.ok()).collect();

    // Count word frequency across memory titles (per project)
    // Key: (lowercase_word, project), Value: (count, Vec<memory_id>)
    let mut word_freq: std::collections::HashMap<(String, Option<String>), (usize, Vec<String>)> =
        std::collections::HashMap::new();

    // Stop words to exclude from entity detection
    let stop_words: std::collections::HashSet<&str> = [
        "the", "a", "an", "is", "are", "was", "were", "be", "been", "being",
        "have", "has", "had", "do", "does", "did", "will", "would", "could",
        "should", "may", "might", "shall", "can", "need", "must",
        "in", "on", "at", "to", "for", "with", "by", "from", "of", "about",
        "into", "through", "during", "before", "after", "above", "below",
        "and", "or", "but", "not", "no", "nor", "so", "yet",
        "it", "its", "this", "that", "these", "those", "my", "your", "his",
        "her", "our", "their", "what", "which", "who", "whom", "how", "when",
        "where", "why", "all", "each", "every", "both", "few", "more", "most",
        "other", "some", "any", "such", "only", "own", "same", "than", "too",
        "very", "just", "also", "then", "now", "here", "there", "up", "out",
        "if", "as", "use", "used", "using", "new", "add", "set", "get",
    ].iter().copied().collect();

    for mem in &rows {
        // Normalize: lowercase, replace non-alphanumeric with space, split
        let normalized: String = mem.title.to_lowercase()
            .chars()
            .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { ' ' })
            .collect();

        // Track which words we've already counted for this memory (dedup within one title)
        let mut seen_in_this_title: std::collections::HashSet<String> = std::collections::HashSet::new();

        for word in normalized.split_whitespace() {
            // Skip short words, stop words, and pure numbers
            if word.len() < 3 || stop_words.contains(word) || word.parse::<f64>().is_ok() {
                continue;
            }

            if seen_in_this_title.insert(word.to_string()) {
                let key = (word.to_string(), mem.project.clone());
                let entry = word_freq.entry(key).or_insert((0, Vec::new()));
                entry.0 += 1;
                entry.1.push(mem.id.clone());
            }
        }
    }

    // Upsert entities for words appearing in 3+ distinct memories
    let mut count = 0;
    let now = now_iso();
    for ((word, project), (freq, memory_ids)) in &word_freq {
        if *freq < 3 {
            continue;
        }

        // Upsert the entity
        increment_entity_mention_with_count(conn, word, project.as_deref(), *freq, &now)?;

        // Create edges linking this entity to the memories (best-effort)
        let entity_id = if let Some(proj) = project {
            conn.query_row(
                "SELECT id FROM entity WHERE name = ?1 AND project = ?2",
                params![word, proj],
                |row| row.get::<_, String>(0),
            ).optional()?
        } else {
            conn.query_row(
                "SELECT id FROM entity WHERE name = ?1 AND project IS NULL",
                params![word],
                |row| row.get::<_, String>(0),
            ).optional()?
        };

        if let Some(eid) = entity_id {
            for mid in memory_ids {
                // Insert edge if not already exists (best-effort, ignore conflicts)
                let edge_id = format!("edge-{}", ulid::Ulid::new());
                let _ = conn.execute(
                    "INSERT OR IGNORE INTO edge (id, from_id, to_id, edge_type, created_at, valid_from)
                     VALUES (?1, ?2, ?3, 'mentions', ?4, ?4)",
                    params![edge_id, eid, mid, now],
                );
            }
        }

        count += 1;
    }

    Ok(count)
}

/// Internal helper: upsert entity with a specific mention count (used by detect_entities).
fn increment_entity_mention_with_count(
    conn: &Connection,
    name: &str,
    project: Option<&str>,
    count: usize,
    now: &str,
) -> rusqlite::Result<()> {
    let existing_id: Option<String> = if let Some(proj) = project {
        conn.query_row(
            "SELECT id FROM entity WHERE name = ?1 AND project = ?2",
            params![name, proj],
            |row| row.get(0),
        ).optional()?
    } else {
        conn.query_row(
            "SELECT id FROM entity WHERE name = ?1 AND project IS NULL",
            params![name],
            |row| row.get(0),
        ).optional()?
    };

    if let Some(id) = existing_id {
        conn.execute(
            "UPDATE entity SET mention_count = ?1, last_seen = ?2 WHERE id = ?3",
            params![count as i64, now, id],
        )?;
    } else {
        let id = format!("ent-{}", ulid::Ulid::new());
        conn.execute(
            "INSERT INTO entity (id, name, entity_type, description, mention_count, first_seen, last_seen, project)
             VALUES (?1, ?2, 'concept', '', ?3, ?4, ?4, ?5)",
            params![id, name, count as i64, now, project],
        )?;
    }
    Ok(())
}

/// Find knowledge gaps: words that appear in 3+ memory titles but have no entity.
/// Returns the list of gap words (lowercase).
pub fn detect_knowledge_gaps(conn: &Connection, project: Option<&str>) -> rusqlite::Result<Vec<String>> {
    // Stop words to exclude
    let stop_words: std::collections::HashSet<&str> = [
        "the", "a", "an", "is", "are", "was", "were", "be", "been", "being",
        "have", "has", "had", "do", "does", "did", "will", "would", "could",
        "should", "may", "might", "shall", "can", "need", "must",
        "in", "on", "at", "to", "for", "with", "by", "from", "of", "about",
        "into", "through", "during", "before", "after", "above", "below",
        "and", "or", "but", "not", "no", "nor", "so", "yet",
        "it", "its", "this", "that", "these", "those", "my", "your", "his",
        "her", "our", "their", "what", "which", "who", "whom", "how", "when",
        "where", "why", "all", "each", "every", "both", "few", "more", "most",
        "other", "some", "any", "such", "only", "own", "same", "than", "too",
        "very", "just", "also", "then", "now", "here", "there", "up", "out",
        "if", "as", "use", "used", "using", "new", "add", "set", "get",
    ].iter().copied().collect();

    // 1. Get all active memory titles for the project
    let rows: Vec<String> = if let Some(proj) = project {
        let mut stmt = conn.prepare(
            "SELECT title FROM memory WHERE status = 'active' AND (project = ?1 OR project IS NULL OR project = '') LIMIT 1000"
        )?;
        let mapped = stmt.query_map(params![proj], |row| row.get::<_, String>(0))?;
        mapped.filter_map(|r| r.ok()).collect()
    } else {
        let mut stmt = conn.prepare(
            "SELECT title FROM memory WHERE status = 'active' LIMIT 1000"
        )?;
        let mapped = stmt.query_map([], |row| row.get::<_, String>(0))?;
        mapped.filter_map(|r| r.ok()).collect()
    };

    // 2. Tokenize into words (split whitespace, lowercase, skip stop words)
    let mut word_freq: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for title in &rows {
        let normalized: String = title.to_lowercase()
            .chars()
            .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { ' ' })
            .collect();

        // Dedup within one title
        let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
        for word in normalized.split_whitespace() {
            if word.len() < 3 || stop_words.contains(word) || word.parse::<f64>().is_ok() {
                continue;
            }
            if seen.insert(word.to_string()) {
                *word_freq.entry(word.to_string()).or_insert(0) += 1;
            }
        }
    }

    // 3. For words appearing 3+ times, check if entity exists
    let mut gaps = Vec::new();
    for (word, count) in &word_freq {
        if *count < 3 {
            continue;
        }

        // Check if entity exists for this word
        let entity_exists: bool = if let Some(proj) = project {
            match conn.query_row(
                "SELECT COUNT(*) > 0 FROM entity WHERE name = ?1 AND (project = ?2 OR project IS NULL)",
                params![word, proj],
                |row| row.get(0),
            ) {
                Ok(v) => v,
                Err(e) => {
                    eprintln!("[knowledge_gaps] entity check error for '{}': {e}", word);
                    continue; // skip this word on error, don't create false gap
                }
            }
        } else {
            match conn.query_row(
                "SELECT COUNT(*) > 0 FROM entity WHERE name = ?1",
                params![word],
                |row| row.get(0),
            ) {
                Ok(v) => v,
                Err(e) => {
                    eprintln!("[knowledge_gaps] entity check error for '{}': {e}", word);
                    continue;
                }
            }
        };

        if !entity_exists {
            gaps.push(word.clone());
        }
    }

    gaps.sort(); // deterministic ordering
    Ok(gaps)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::schema::create_schema;

    fn open_db() -> Connection {
        crate::db::vec::init_sqlite_vec();
        let conn = Connection::open_in_memory().unwrap();
        create_schema(&conn).unwrap();
        conn
    }

    #[test]
    fn test_platform_crud() {
        let conn = open_db();

        let entry = PlatformEntry {
            key: "os".into(),
            value: "linux".into(),
            detected_at: "2026-04-03 12:00:00".into(),
        };
        store_platform(&conn, &entry).unwrap();

        // Read back
        let got = get_platform(&conn, "os").unwrap().expect("should exist");
        assert_eq!(got.key, "os");
        assert_eq!(got.value, "linux");

        // Update via upsert
        let updated = PlatformEntry {
            key: "os".into(),
            value: "darwin".into(),
            detected_at: "2026-04-03 13:00:00".into(),
        };
        store_platform(&conn, &updated).unwrap();
        let got = get_platform(&conn, "os").unwrap().expect("should exist");
        assert_eq!(got.value, "darwin");

        // List all
        let all = list_platform(&conn).unwrap();
        assert_eq!(all.len(), 1);

        // Non-existent key
        let missing = get_platform(&conn, "nonexistent").unwrap();
        assert!(missing.is_none());
    }

    #[test]
    fn test_tool_crud() {
        let conn = open_db();

        let tool = Tool {
            id: "t1".into(),
            name: "cargo".into(),
            kind: ToolKind::Cli,
            capabilities: vec!["build".into(), "test".into()],
            config: None,
            health: ToolHealth::Healthy,
            last_used: None,
            use_count: 0,
            discovered_at: "2026-04-03 12:00:00".into(),
        };
        store_tool(&conn, &tool).unwrap();

        // List all
        let all = list_tools(&conn, None).unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].name, "cargo");
        assert_eq!(all[0].capabilities, vec!["build", "test"]);

        // List by kind
        let cli_tools = list_tools(&conn, Some(&ToolKind::Cli)).unwrap();
        assert_eq!(cli_tools.len(), 1);
        let mcp_tools = list_tools(&conn, Some(&ToolKind::Mcp)).unwrap();
        assert_eq!(mcp_tools.len(), 0);

        // Record usage
        let used = record_tool_use(&conn, "t1").unwrap();
        assert!(used);
        let tools = list_tools(&conn, None).unwrap();
        assert_eq!(tools[0].use_count, 1);
        assert!(tools[0].last_used.is_some());

        // Record usage for non-existent tool
        let used = record_tool_use(&conn, "nonexistent").unwrap();
        assert!(!used);
    }

    #[test]
    fn test_skill_crud() {
        let conn = open_db();

        let skill1 = Skill {
            id: "s1".into(),
            name: "TDD".into(),
            domain: "testing".into(),
            description: "Test-driven development".into(),
            steps: vec!["write test".into(), "make pass".into()],
            success_count: 5,
            fail_count: 1,
            last_used: None,
            source: "learned".into(),
            version: 1,
            project: None,
        skill_type: "procedural".to_string(),
        user_specific: false,
        observed_count: 1,
        correlation_ids: vec![],
        };
        let skill2 = Skill {
            id: "s2".into(),
            name: "Code Review".into(),
            domain: "quality".into(),
            description: "Peer code review".into(),
            steps: vec!["read diff".into(), "comment".into()],
            success_count: 3,
            fail_count: 0,
            last_used: None,
            source: "declared".into(),
            version: 1,
            project: None,
        skill_type: "procedural".to_string(),
        user_specific: false,
        observed_count: 1,
        correlation_ids: vec![],
        };
        store_skill(&conn, &skill1).unwrap();
        store_skill(&conn, &skill2).unwrap();

        // List all
        let all = list_skills(&conn, None).unwrap();
        assert_eq!(all.len(), 2);

        // List by domain
        let testing = list_skills(&conn, Some("testing")).unwrap();
        assert_eq!(testing.len(), 1);
        assert_eq!(testing[0].name, "TDD");
        assert_eq!(testing[0].steps, vec!["write test", "make pass"]);

        let quality = list_skills(&conn, Some("quality")).unwrap();
        assert_eq!(quality.len(), 1);
        assert_eq!(quality[0].name, "Code Review");

        let empty = list_skills(&conn, Some("nonexistent")).unwrap();
        assert!(empty.is_empty());
    }

    #[test]
    fn test_search_skills() {
        let conn = open_db();
        let skill = Skill {
            id: "s1".into(),
            name: "Deploy Rust".into(),
            domain: "devops".into(),
            description: "cargo build then scp".into(),
            steps: vec!["cargo build".into()],
            success_count: 3,
            fail_count: 0,
            last_used: None,
            source: "extracted".into(),
            version: 1,
            project: None,
        skill_type: "procedural".to_string(),
        user_specific: false,
        observed_count: 1,
        correlation_ids: vec![],
        };
        store_skill(&conn, &skill).unwrap();

        let found = search_skills(&conn, "deploy", None).unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].name, "Deploy Rust");

        // Search by domain
        let by_domain = search_skills(&conn, "devops", None).unwrap();
        assert_eq!(by_domain.len(), 1);

        // Search by description keyword
        let by_desc = search_skills(&conn, "scp", None).unwrap();
        assert_eq!(by_desc.len(), 1);

        let not_found = search_skills(&conn, "nonexistent", None).unwrap();
        assert!(not_found.is_empty());
    }

    #[test]
    fn test_record_skill_result() {
        let conn = open_db();
        let skill = Skill {
            id: "s1".into(),
            name: "Test".into(),
            domain: "qa".into(),
            description: "Run tests".into(),
            steps: vec![],
            success_count: 0,
            fail_count: 0,
            last_used: None,
            source: "extracted".into(),
            version: 1,
            project: None,
        skill_type: "procedural".to_string(),
        user_specific: false,
        observed_count: 1,
        correlation_ids: vec![],
        };
        store_skill(&conn, &skill).unwrap();

        record_skill_result(&conn, "s1", true).unwrap();
        record_skill_result(&conn, "s1", true).unwrap();
        record_skill_result(&conn, "s1", false).unwrap();

        let skills = list_skills(&conn, None).unwrap();
        assert_eq!(skills[0].success_count, 2);
        assert_eq!(skills[0].fail_count, 1);
        assert!(skills[0].last_used.is_some());

        // Non-existent skill should return false
        let updated = record_skill_result(&conn, "nonexistent", true).unwrap();
        assert!(!updated);
    }

    #[test]
    fn test_prune_junk_skills() {
        let conn = open_db();

        // Store a junk skill (no steps, short description)
        let junk = Skill {
            id: "junk".into(),
            name: "All Tasks Complete".into(),
            domain: "tasks".into(),
            description: "Done".into(),
            steps: vec![],
            success_count: 0,
            fail_count: 0,
            last_used: None,
            source: "extracted".into(),
            version: 1,
            project: None,
        skill_type: "procedural".to_string(),
        user_specific: false,
        observed_count: 1,
        correlation_ids: vec![],
        };
        store_skill(&conn, &junk).unwrap();

        // Store a good skill (has steps, long description)
        let good = Skill {
            id: "good".into(),
            name: "Deploy Rust".into(),
            domain: "devops".into(),
            description: "1) cargo build --release 2) scp binary to server 3) systemctl restart the-service".into(),
            steps: vec!["cargo build".into(), "scp".into(), "restart".into()],
            success_count: 3,
            fail_count: 0,
            last_used: None,
            source: "extracted".into(),
            version: 1,
            project: None,
        skill_type: "procedural".to_string(),
        user_specific: false,
        observed_count: 1,
        correlation_ids: vec![],
        };
        store_skill(&conn, &good).unwrap();

        // Prune should remove only the junk skill
        let pruned = prune_junk_skills(&conn).unwrap();
        assert_eq!(pruned, 1);

        let remaining = list_skills(&conn, None).unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].name, "Deploy Rust");
    }

    #[test]
    fn test_prune_preserves_proven_skills() {
        let conn = open_db();

        // Even a short-description skill should be kept if it has success_count > 0
        let proven = Skill {
            id: "proven".into(),
            name: "Quick Fix".into(),
            domain: "dev".into(),
            description: "Short".into(),
            steps: vec![],
            success_count: 1,
            fail_count: 0,
            last_used: None,
            source: "extracted".into(),
            version: 1,
            project: None,
        skill_type: "procedural".to_string(),
        user_specific: false,
        observed_count: 1,
        correlation_ids: vec![],
        };
        store_skill(&conn, &proven).unwrap();

        let pruned = prune_junk_skills(&conn).unwrap();
        assert_eq!(pruned, 0);

        let remaining = list_skills(&conn, None).unwrap();
        assert_eq!(remaining.len(), 1);
    }

    #[test]
    fn test_domain_dna_crud() {
        let conn = open_db();

        let dna = DomainDna {
            id: "d1".into(),
            project: "forge".into(),
            aspect: "naming".into(),
            pattern: "snake_case for functions".into(),
            confidence: 0.9,
            evidence: vec!["src/main.rs".into(), "src/lib.rs".into()],
            detected_at: "2026-04-03 12:00:00".into(),
        };
        store_domain_dna(&conn, &dna).unwrap();

        // List all
        let all = list_domain_dna(&conn, None).unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].pattern, "snake_case for functions");
        assert_eq!(all[0].evidence, vec!["src/main.rs", "src/lib.rs"]);

        // List by project
        let forge = list_domain_dna(&conn, Some("forge")).unwrap();
        assert_eq!(forge.len(), 1);
        let other = list_domain_dna(&conn, Some("other")).unwrap();
        assert!(other.is_empty());

        // Upsert
        let updated = DomainDna {
            id: "d1".into(),
            project: "forge".into(),
            aspect: "naming".into(),
            pattern: "snake_case everywhere".into(),
            confidence: 0.95,
            evidence: vec!["src/main.rs".into(), "src/lib.rs".into(), "src/db.rs".into()],
            detected_at: "2026-04-03 13:00:00".into(),
        };
        store_domain_dna(&conn, &updated).unwrap();
        let all = list_domain_dna(&conn, None).unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].pattern, "snake_case everywhere");
    }

    #[test]
    fn test_perception_lifecycle() {
        let conn = open_db();

        let p1 = Perception {
            id: "p1".into(),
            kind: PerceptionKind::Error,
            data: "compilation failed".into(),
            severity: Severity::Error,
            project: Some("forge".into()),
            created_at: "2026-04-03 12:00:00".into(),
            expires_at: None,
            consumed: false,
        };
        let p2 = Perception {
            id: "p2".into(),
            kind: PerceptionKind::TestResult,
            data: "all tests pass".into(),
            severity: Severity::Info,
            project: Some("forge".into()),
            created_at: "2026-04-03 12:01:00".into(),
            expires_at: None,
            consumed: false,
        };
        store_perception(&conn, &p1).unwrap();
        store_perception(&conn, &p2).unwrap();

        // List unconsumed
        let unconsumed = list_unconsumed_perceptions(&conn, None).unwrap();
        assert_eq!(unconsumed.len(), 2);

        // Filter by kind
        let errors = list_unconsumed_perceptions(&conn, Some(&PerceptionKind::Error)).unwrap();
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].data, "compilation failed");

        // Consume one
        let consumed = consume_perception(&conn, "p1").unwrap();
        assert!(consumed);

        // Now only one unconsumed
        let unconsumed = list_unconsumed_perceptions(&conn, None).unwrap();
        assert_eq!(unconsumed.len(), 1);
        assert_eq!(unconsumed[0].id, "p2");

        // Double-consume returns false
        let consumed = consume_perception(&conn, "p1").unwrap();
        assert!(!consumed);
    }

    #[test]
    fn test_declared_crud() {
        let conn = open_db();

        let d = Declared {
            id: "dk1".into(),
            source: "CLAUDE.md".into(),
            path: Some("/project/CLAUDE.md".into()),
            content: "Use snake_case".into(),
            hash: "abc123".into(),
            project: Some("forge".into()),
            ingested_at: "2026-04-03 12:00:00".into(),
        };
        store_declared(&conn, &d).unwrap();

        // List all
        let all = list_declared(&conn, None).unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].source, "CLAUDE.md");

        // List by project
        let forge = list_declared(&conn, Some("forge")).unwrap();
        assert_eq!(forge.len(), 1);
        let other = list_declared(&conn, Some("other")).unwrap();
        assert!(other.is_empty());

        // Get by hash
        let found = get_declared_by_hash(&conn, "abc123").unwrap().expect("should find by hash");
        assert_eq!(found.id, "dk1");
        assert_eq!(found.content, "Use snake_case");

        // Get by hash - not found
        let missing = get_declared_by_hash(&conn, "nonexistent").unwrap();
        assert!(missing.is_none());
    }

    #[test]
    fn test_identity_crud() {
        let conn = open_db();

        let facet = IdentityFacet {
            id: "if1".into(),
            agent: "forge".into(),
            facet: "role".into(),
            description: "memory system for AI agents".into(),
            strength: 0.9,
            source: "declared".into(),
            active: true,
            created_at: "2026-04-03 12:00:00".into(),
        };
        store_identity(&conn, &facet).unwrap();

        // List active only
        let active = list_identity(&conn, "forge", true).unwrap();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].description, "memory system for AI agents");

        // Deactivate
        let deactivated = deactivate_identity(&conn, "if1").unwrap();
        assert!(deactivated);

        // Active list should be empty
        let active = list_identity(&conn, "forge", true).unwrap();
        assert!(active.is_empty());

        // All list should show it
        let all = list_identity(&conn, "forge", false).unwrap();
        assert_eq!(all.len(), 1);
        assert!(!all[0].active);

        // Double-deactivate returns false
        let deactivated = deactivate_identity(&conn, "if1").unwrap();
        assert!(!deactivated);
    }

    #[test]
    fn test_disposition_crud() {
        let conn = open_db();

        let d = Disposition {
            id: "dp1".into(),
            agent: "forge".into(),
            disposition_trait: DispositionTrait::Caution,
            domain: Some("security".into()),
            value: 0.7,
            trend: Trend::Rising,
            updated_at: "2026-04-03 12:00:00".into(),
            evidence: vec!["always runs clippy".into()],
        };
        store_disposition(&conn, &d).unwrap();

        let list = list_dispositions(&conn, "forge").unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].disposition_trait, DispositionTrait::Caution);
        assert_eq!(list[0].domain, Some("security".into()));
        assert!((list[0].value - 0.7).abs() < f64::EPSILON);
        assert_eq!(list[0].trend, Trend::Rising);
        assert_eq!(list[0].evidence, vec!["always runs clippy"]);

        // Update via upsert
        let updated = Disposition {
            id: "dp1".into(),
            agent: "forge".into(),
            disposition_trait: DispositionTrait::Caution,
            domain: Some("security".into()),
            value: 0.8,
            trend: Trend::Stable,
            updated_at: "2026-04-03 13:00:00".into(),
            evidence: vec!["always runs clippy".into(), "uses parameterized queries".into()],
        };
        store_disposition(&conn, &updated).unwrap();
        let list = list_dispositions(&conn, "forge").unwrap();
        assert_eq!(list.len(), 1);
        assert!((list[0].value - 0.8).abs() < f64::EPSILON);
        assert_eq!(list[0].trend, Trend::Stable);

        // Different agent shows empty
        let other = list_dispositions(&conn, "other-agent").unwrap();
        assert!(other.is_empty());
    }

    #[test]
    fn test_manas_health() {
        let conn = open_db();

        // Empty initially
        let h = manas_health(&conn).unwrap();
        assert_eq!(h.platform_entries, 0);
        assert_eq!(h.tools, 0);
        assert_eq!(h.skills, 0);
        assert_eq!(h.domain_dna_entries, 0);
        assert_eq!(h.perceptions_unconsumed, 0);
        assert_eq!(h.declared_entries, 0);
        assert_eq!(h.identity_facets_active, 0);
        assert_eq!(h.dispositions, 0);

        // Add some data
        store_platform(&conn, &PlatformEntry {
            key: "os".into(),
            value: "linux".into(),
            detected_at: "2026-04-03 12:00:00".into(),
        }).unwrap();

        store_tool(&conn, &Tool {
            id: "t1".into(),
            name: "cargo".into(),
            kind: ToolKind::Cli,
            capabilities: vec![],
            config: None,
            health: ToolHealth::Healthy,
            last_used: None,
            use_count: 0,
            discovered_at: "2026-04-03 12:00:00".into(),
        }).unwrap();

        store_perception(&conn, &Perception {
            id: "p1".into(),
            kind: PerceptionKind::Error,
            data: "err".into(),
            severity: Severity::Error,
            project: None,
            created_at: "2026-04-03 12:00:00".into(),
            expires_at: None,
            consumed: false,
        }).unwrap();

        store_identity(&conn, &IdentityFacet {
            id: "if1".into(),
            agent: "forge".into(),
            facet: "role".into(),
            description: "memory".into(),
            strength: 0.9,
            source: "declared".into(),
            active: true,
            created_at: "2026-04-03 12:00:00".into(),
        }).unwrap();

        let h = manas_health(&conn).unwrap();
        assert_eq!(h.platform_entries, 1);
        assert_eq!(h.tools, 1);
        assert_eq!(h.perceptions_unconsumed, 1);
        assert_eq!(h.identity_facets_active, 1);

        // Consume the perception — unconsumed count should drop
        consume_perception(&conn, "p1").unwrap();
        let h = manas_health(&conn).unwrap();
        assert_eq!(h.perceptions_unconsumed, 0);
    }

    #[test]
    fn test_platform_auto_detection() {
        let conn = open_db();

        let count = detect_and_store_platform(&conn).unwrap();
        // At minimum, os and arch are always detected
        assert!(count >= 2);

        let entries = list_platform(&conn).unwrap();
        assert!(entries.len() >= 2);

        // os should be one of the entries
        let os_entry = entries.iter().find(|e| e.key == "os");
        assert!(os_entry.is_some(), "os entry should exist");
        assert!(!os_entry.unwrap().value.is_empty());

        // arch should be one of the entries
        let arch_entry = entries.iter().find(|e| e.key == "arch");
        assert!(arch_entry.is_some(), "arch entry should exist");
        assert!(!arch_entry.unwrap().value.is_empty());
    }

    #[test]
    fn test_search_declared() {
        let conn = open_db();

        // Store some declared knowledge
        let d1 = Declared {
            id: "dk1".into(),
            source: "CLAUDE.md".into(),
            path: Some("/project/CLAUDE.md".into()),
            content: "Always use snake_case for Rust functions".into(),
            hash: "hash1".into(),
            project: Some("forge".into()),
            ingested_at: "2026-04-03 12:00:00".into(),
        };
        let d2 = Declared {
            id: "dk2".into(),
            source: "CONVENTIONS.md".into(),
            path: Some("/project/CONVENTIONS.md".into()),
            content: "Use parameterized SQL queries for security".into(),
            hash: "hash2".into(),
            project: Some("forge".into()),
            ingested_at: "2026-04-03 12:01:00".into(),
        };
        store_declared(&conn, &d1).unwrap();
        store_declared(&conn, &d2).unwrap();

        // Search by content keyword
        let results = search_declared(&conn, "snake_case", None).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, "dk1");

        // Search by source keyword
        let results = search_declared(&conn, "CONVENTIONS", None).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, "dk2");

        // Search with project filter
        let results = search_declared(&conn, "SQL", Some("forge")).unwrap();
        assert_eq!(results.len(), 1);

        // Search with wrong project — should return nothing (project != "other" and not NULL)
        let results = search_declared(&conn, "snake_case", Some("other")).unwrap();
        assert!(results.is_empty());

        // Search for non-existent content
        let results = search_declared(&conn, "nonexistent_gibberish", None).unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn test_ingest_declared_file() {
        let conn = open_db();

        // Write a temp file
        let dir = std::env::temp_dir().join("forge_test_ingest");
        let _ = std::fs::create_dir_all(&dir);
        let file_path = dir.join("test_claude.md");
        std::fs::write(&file_path, "# Forge Rules\nAlways run tests before committing.").unwrap();

        let path_str = file_path.to_str().unwrap();
        let ingested = ingest_declared_file(&conn, path_str, "CLAUDE.md", Some("forge")).unwrap();
        assert!(ingested, "first ingest should return true");

        // Verify stored
        let all = list_declared(&conn, Some("forge")).unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].source, "CLAUDE.md");
        assert!(all[0].content.contains("Always run tests"));

        // Cleanup
        let _ = std::fs::remove_file(&file_path);
        let _ = std::fs::remove_dir(&dir);
    }

    #[test]
    fn test_ingest_declared_idempotent() {
        let conn = open_db();

        // Write a temp file
        let dir = std::env::temp_dir().join("forge_test_ingest_idem");
        let _ = std::fs::create_dir_all(&dir);
        let file_path = dir.join("test_idem.md");
        std::fs::write(&file_path, "Idempotent content check").unwrap();

        let path_str = file_path.to_str().unwrap();

        // First ingest
        let first = ingest_declared_file(&conn, path_str, "TEST.md", None).unwrap();
        assert!(first, "first ingest should succeed");

        // Second ingest with same content — should be idempotent
        let second = ingest_declared_file(&conn, path_str, "TEST.md", None).unwrap();
        assert!(!second, "second ingest of same content should return false");

        // Verify only one entry
        let all = list_declared(&conn, None).unwrap();
        assert_eq!(all.len(), 1);

        // Cleanup
        let _ = std::fs::remove_file(&file_path);
        let _ = std::fs::remove_dir(&dir);
    }

    #[test]
    fn test_detect_and_store_tools() {
        let conn = open_db();
        let found = detect_and_store_tools(&conn).unwrap();
        // Should find at least 1 tool on any dev machine (git, bash, etc.)
        assert!(found > 0, "should detect at least one tool");

        let tools = list_tools(&conn, None).unwrap();
        assert!(!tools.is_empty());
        // All tools should have ToolKind::Cli and ToolHealth::Healthy
        for t in &tools {
            assert_eq!(t.kind, ToolKind::Cli);
            assert_eq!(t.health, ToolHealth::Healthy);
            assert!(!t.capabilities.is_empty());
        }
    }

    #[test]
    fn test_available_tool_names() {
        let conn = open_db();

        // Initially empty
        let names = available_tool_names(&conn).unwrap();
        assert!(names.is_empty());

        // Store a tool
        store_tool(&conn, &Tool {
            id: "cli:git".into(),
            name: "git".into(),
            kind: ToolKind::Cli,
            capabilities: vec!["version-control".into()],
            config: None,
            health: ToolHealth::Healthy,
            last_used: None,
            use_count: 0,
            discovered_at: "2026-04-03 12:00:00".into(),
        }).unwrap();

        let names = available_tool_names(&conn).unwrap();
        assert_eq!(names.len(), 1);
        assert!(names.contains("git"));
        assert!(!names.contains("kubectl"));
    }

    #[test]
    fn test_detect_and_store_tools_idempotent() {
        let conn = open_db();
        let found1 = detect_and_store_tools(&conn).unwrap();
        let found2 = detect_and_store_tools(&conn).unwrap();
        // Running twice should find the same number (upsert, no duplicates)
        assert_eq!(found1, found2);

        let tools = list_tools(&conn, None).unwrap();
        assert_eq!(tools.len(), found1);
    }

    #[test]
    fn test_ingest_project_declared() {
        let conn = open_db();

        // Create temp project dir with CLAUDE.md
        let dir = std::env::temp_dir().join("forge_test_ingest_project");
        let _ = std::fs::create_dir_all(&dir);
        std::fs::write(dir.join("CLAUDE.md"), "# Project Rules\nAlways run tests.").unwrap();
        std::fs::write(dir.join("README.md"), "# My Project\nA test project.").unwrap();

        let dir_str = dir.to_str().unwrap();
        let (ingested, skipped) = ingest_project_declared(&conn, dir_str).unwrap();
        assert_eq!(ingested, 2, "should ingest CLAUDE.md and README.md");
        assert_eq!(skipped, 0, "nothing to skip on first run");

        // Verify stored
        let project_name = dir.file_name().unwrap().to_str().unwrap();
        let all = list_declared(&conn, Some(project_name)).unwrap();
        assert_eq!(all.len(), 2);

        // One should be claude_md source, one readme
        let sources: Vec<&str> = all.iter().map(|d| d.source.as_str()).collect();
        assert!(sources.contains(&"claude_md"), "should have claude_md source");
        assert!(sources.contains(&"readme"), "should have readme source");

        // Cleanup
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_ingest_project_declared_idempotent() {
        let conn = open_db();

        let dir = std::env::temp_dir().join("forge_test_ingest_idemp");
        let _ = std::fs::create_dir_all(&dir);
        std::fs::write(dir.join("CLAUDE.md"), "Idempotent test content").unwrap();

        let dir_str = dir.to_str().unwrap();

        // First ingest
        let (ingested1, skipped1) = ingest_project_declared(&conn, dir_str).unwrap();
        assert_eq!(ingested1, 1);
        assert_eq!(skipped1, 0);

        // Second ingest with same content — should skip
        let (ingested2, skipped2) = ingest_project_declared(&conn, dir_str).unwrap();
        assert_eq!(ingested2, 0, "second run should ingest nothing");
        assert_eq!(skipped2, 1, "second run should skip 1");

        // Only one entry in DB
        let project_name = dir.file_name().unwrap().to_str().unwrap();
        let all = list_declared(&conn, Some(project_name)).unwrap();
        assert_eq!(all.len(), 1);

        // Cleanup
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_detect_domain_dna_rust() {
        let conn = open_db();

        let dir = std::env::temp_dir().join("forge_test_dna_rust");
        let _ = std::fs::create_dir_all(&dir);
        std::fs::write(dir.join("Cargo.toml"), "[package]\nname = \"test\"\n").unwrap();

        let dir_str = dir.to_str().unwrap();
        let detected = detect_domain_dna(&conn, dir_str).unwrap();
        assert_eq!(detected, 1, "should detect 1 project type marker");

        let project_name = dir.file_name().unwrap().to_str().unwrap();
        let dna_list = list_domain_dna(&conn, Some(project_name)).unwrap();
        assert!(!dna_list.is_empty(), "should have domain DNA entries");

        // Should have language, test_command, lint_command, build_command
        let aspects: Vec<&str> = dna_list.iter().map(|d| d.aspect.as_str()).collect();
        assert!(aspects.contains(&"language"), "should detect language");
        assert!(aspects.contains(&"test_command"), "should detect test command");
        assert!(aspects.contains(&"lint_command"), "should detect lint command");
        assert!(aspects.contains(&"build_command"), "should detect build command");

        // Verify language is rust
        let lang = dna_list.iter().find(|d| d.aspect == "language").unwrap();
        assert_eq!(lang.pattern, "rust");
        assert!((lang.confidence - 0.95).abs() < f64::EPSILON);

        // Verify test command
        let test = dna_list.iter().find(|d| d.aspect == "test_command").unwrap();
        assert_eq!(test.pattern, "cargo test");

        // Cleanup
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_detect_domain_dna_multiple() {
        let conn = open_db();

        let dir = std::env::temp_dir().join("forge_test_dna_multi");
        let _ = std::fs::create_dir_all(&dir);
        std::fs::write(dir.join("Cargo.toml"), "[package]\nname = \"test\"\n").unwrap();
        std::fs::write(dir.join("Dockerfile"), "FROM rust:latest\n").unwrap();

        let dir_str = dir.to_str().unwrap();
        let detected = detect_domain_dna(&conn, dir_str).unwrap();
        assert_eq!(detected, 2, "should detect both Cargo.toml and Dockerfile markers");

        // The language should be set by the last matching marker (Dockerfile overrides Cargo.toml
        // since both write to the same dna-{project}-language ID). But since markers are iterated
        // in order and Cargo.toml comes first, Dockerfile comes second and wins.
        // Actually this is by design — the last one wins via ON CONFLICT DO UPDATE.
        let project_name = dir.file_name().unwrap().to_str().unwrap();
        let dna_list = list_domain_dna(&conn, Some(project_name)).unwrap();

        // We should have entries for language, test_command, lint_command, build_command
        // (4 entries, with the latest values from Dockerfile)
        assert!(dna_list.len() >= 4, "should have at least 4 DNA entries, got {}", dna_list.len());

        // Cleanup
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_identity_dedup_same_description() {
        let conn = open_db();

        let f1 = IdentityFacet {
            id: "f1".into(),
            agent: "claude-code".into(),
            facet: "values".into(),
            description: "Values thoroughness".into(),
            strength: 0.8,
            source: "extracted".into(),
            active: true,
            created_at: "2026-04-04 12:00:00".into(),
        };
        store_identity(&conn, &f1).unwrap();

        // Insert a second facet with same agent + description but different id and higher strength
        let f2 = IdentityFacet {
            id: "f2".into(),
            agent: "claude-code".into(),
            facet: "values".into(),
            description: "Values thoroughness".into(),
            strength: 0.9,
            source: "extracted".into(),
            active: true,
            created_at: "2026-04-04 12:00:01".into(),
        };
        store_identity(&conn, &f2).unwrap();

        let all = list_identity(&conn, "claude-code", true).unwrap();
        assert_eq!(all.len(), 1, "duplicate facets should be merged, got {}", all.len());
        assert!(
            (all[0].strength - 0.9).abs() < 0.01,
            "should keep higher strength (0.9), got {}",
            all[0].strength
        );
    }

    #[test]
    fn test_identity_dedup_lower_strength_ignored() {
        let conn = open_db();

        let f1 = IdentityFacet {
            id: "f1".into(),
            agent: "claude-code".into(),
            facet: "values".into(),
            description: "Values thoroughness".into(),
            strength: 0.9,
            source: "extracted".into(),
            active: true,
            created_at: "2026-04-04 12:00:00".into(),
        };
        store_identity(&conn, &f1).unwrap();

        // Insert a duplicate with lower strength — should be silently merged (no update)
        let f2 = IdentityFacet {
            id: "f2".into(),
            agent: "claude-code".into(),
            facet: "values".into(),
            description: "Values thoroughness".into(),
            strength: 0.5,
            source: "extracted".into(),
            active: true,
            created_at: "2026-04-04 12:00:01".into(),
        };
        store_identity(&conn, &f2).unwrap();

        let all = list_identity(&conn, "claude-code", true).unwrap();
        assert_eq!(all.len(), 1, "duplicate facets should be merged");
        assert!(
            (all[0].strength - 0.9).abs() < 0.01,
            "should keep original higher strength (0.9), got {}",
            all[0].strength
        );
    }

    #[test]
    fn test_identity_dedup_different_agents_not_merged() {
        let conn = open_db();

        let f1 = IdentityFacet {
            id: "f1".into(),
            agent: "claude-code".into(),
            facet: "values".into(),
            description: "Values thoroughness".into(),
            strength: 0.8,
            source: "extracted".into(),
            active: true,
            created_at: "2026-04-04 12:00:00".into(),
        };
        store_identity(&conn, &f1).unwrap();

        // Same description but different agent — should NOT be merged
        let f2 = IdentityFacet {
            id: "f2".into(),
            agent: "codex".into(),
            facet: "values".into(),
            description: "Values thoroughness".into(),
            strength: 0.9,
            source: "extracted".into(),
            active: true,
            created_at: "2026-04-04 12:00:01".into(),
        };
        store_identity(&conn, &f2).unwrap();

        let claude = list_identity(&conn, "claude-code", true).unwrap();
        let codex = list_identity(&conn, "codex", true).unwrap();
        assert_eq!(claude.len(), 1, "claude-code should have 1 facet");
        assert_eq!(codex.len(), 1, "codex should have 1 facet");
    }
}
