use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use forge_core::types::{
    PlatformEntry, Tool, ToolKind, ToolHealth,
    Skill, DomainDna, Perception, PerceptionKind, Severity,
    Declared, IdentityFacet, Disposition, DispositionTrait, Trend,
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
    }
}

fn perception_kind_from_str(s: &str) -> PerceptionKind {
    match s {
        "file_change" => PerceptionKind::FileChange,
        "error" => PerceptionKind::Error,
        "build_result" => PerceptionKind::BuildResult,
        "test_result" => PerceptionKind::TestResult,
        "user_feedback" => PerceptionKind::UserFeedback,
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

fn now_iso() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let days_since_epoch = secs / 86400;
    let time_of_day = secs % 86400;

    let mut year = 1970u64;
    let mut remaining_days = days_since_epoch;
    loop {
        let is_leap = year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400));
        let days_in_year = if is_leap { 366 } else { 365 };
        if remaining_days < days_in_year {
            break;
        }
        remaining_days -= days_in_year;
        year += 1;
    }

    let is_leap = year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400));
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

// ──────────────────────────────────────────────
// Layer 2: Skill ops
// ──────────────────────────────────────────────

/// Store or update a skill.
pub fn store_skill(conn: &Connection, skill: &Skill) -> rusqlite::Result<()> {
    let steps_json = serde_json::to_string(&skill.steps).unwrap_or_else(|_| "[]".to_string());
    conn.execute(
        "INSERT INTO skill (id, name, domain, description, steps, success_count, fail_count, last_used, source, version)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
         ON CONFLICT(id) DO UPDATE SET
            name = excluded.name,
            domain = excluded.domain,
            description = excluded.description,
            steps = excluded.steps,
            success_count = excluded.success_count,
            fail_count = excluded.fail_count,
            last_used = excluded.last_used,
            version = excluded.version",
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
        ],
    )?;
    Ok(())
}

/// List all skills, optionally filtered by domain.
pub fn list_skills(conn: &Connection, domain_filter: Option<&str>) -> rusqlite::Result<Vec<Skill>> {
    if let Some(d) = domain_filter {
        let mut stmt = conn.prepare(
            "SELECT id, name, domain, description, steps, success_count, fail_count, last_used, source, version
             FROM skill WHERE domain = ?1 ORDER BY name"
        )?;
        let rows = stmt.query_map(params![d], row_to_skill)?;
        rows.collect()
    } else {
        let mut stmt = conn.prepare(
            "SELECT id, name, domain, description, steps, success_count, fail_count, last_used, source, version
             FROM skill ORDER BY name"
        )?;
        let rows = stmt.query_map([], row_to_skill)?;
        rows.collect()
    }
}

fn row_to_skill(row: &rusqlite::Row) -> rusqlite::Result<Skill> {
    let steps_str: String = row.get(4)?;
    let steps: Vec<String> = serde_json::from_str(&steps_str).unwrap_or_default();
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
    })
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

/// Store a perception event.
pub fn store_perception(conn: &Connection, p: &Perception) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO perception (id, kind, data, severity, project, created_at, expires_at, consumed)
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
// Layer 6: Identity ops
// ──────────────────────────────────────────────

/// Store or update an identity facet.
pub fn store_identity(conn: &Connection, facet: &IdentityFacet) -> rusqlite::Result<()> {
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
    pub perceptions_unconsumed: usize,
    pub declared_entries: usize,
    pub identity_facets_active: usize,
    pub dispositions: usize,
}

/// Gather health counts across all 8 Manas layers.
pub fn manas_health(conn: &Connection) -> rusqlite::Result<ManasHealth> {
    let count_table = |table: &str, where_clause: &str| -> rusqlite::Result<usize> {
        let sql = format!("SELECT COUNT(*) FROM {}{}", table, where_clause);
        conn.query_row(&sql, [], |row| row.get::<_, i64>(0))
            .map(|n| n as usize)
    };

    Ok(ManasHealth {
        platform_entries: count_table("platform", "")?,
        tools: count_table("tool", "")?,
        skills: count_table("skill", "")?,
        domain_dna_entries: count_table("domain_dna", "")?,
        perceptions_unconsumed: count_table("perception", " WHERE consumed = 0")?,
        declared_entries: count_table("declared", "")?,
        identity_facets_active: count_table("identity", " WHERE active = 1")?,
        dispositions: count_table("disposition", "")?,
    })
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
}
