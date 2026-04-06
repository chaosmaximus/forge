use rusqlite::Connection;

pub fn create_schema(conn: &Connection) -> rusqlite::Result<()> {
    // Create memory_vec virtual table (sqlite-vec must be loaded before this call)
    conn.execute_batch(
        "CREATE VIRTUAL TABLE IF NOT EXISTS memory_vec USING vec0(
            id TEXT PRIMARY KEY,
            embedding float[768] distance_metric=cosine
        );"
    )?;

    // Create code_vec virtual table for code embeddings (sqlite-vec must be loaded before this call)
    conn.execute_batch(
        "CREATE VIRTUAL TABLE IF NOT EXISTS code_vec USING vec0(
            id TEXT PRIMARY KEY,
            embedding float[768] distance_metric=cosine
        );"
    )?;

    conn.execute_batch("
        CREATE TABLE IF NOT EXISTS memory (
            id TEXT PRIMARY KEY,
            memory_type TEXT NOT NULL,
            title TEXT NOT NULL,
            content TEXT NOT NULL,
            confidence REAL NOT NULL DEFAULT 0.9,
            status TEXT NOT NULL DEFAULT 'active',
            project TEXT,
            tags TEXT NOT NULL DEFAULT '[]',
            created_at TEXT NOT NULL,
            accessed_at TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_memory_type ON memory(memory_type);
        CREATE INDEX IF NOT EXISTS idx_memory_status ON memory(status);
        CREATE INDEX IF NOT EXISTS idx_memory_project ON memory(project);

        CREATE VIRTUAL TABLE IF NOT EXISTS memory_fts USING fts5(
            title, content, tags,
            content='memory', content_rowid='rowid'
        );

        CREATE TRIGGER IF NOT EXISTS memory_fts_insert AFTER INSERT ON memory BEGIN
            INSERT INTO memory_fts(rowid, title, content, tags) VALUES (new.rowid, new.title, new.content, new.tags);
        END;

        CREATE TRIGGER IF NOT EXISTS memory_fts_delete AFTER DELETE ON memory BEGIN
            INSERT INTO memory_fts(memory_fts, rowid, title, content, tags) VALUES ('delete', old.rowid, old.title, old.content, old.tags);
        END;

        CREATE TRIGGER IF NOT EXISTS memory_fts_update AFTER UPDATE ON memory BEGIN
            INSERT INTO memory_fts(memory_fts, rowid, title, content, tags) VALUES ('delete', old.rowid, old.title, old.content, old.tags);
            INSERT INTO memory_fts(rowid, title, content, tags) VALUES (new.rowid, new.title, new.content, new.tags);
        END;

        CREATE TABLE IF NOT EXISTS edge (
            id TEXT PRIMARY KEY,
            from_id TEXT NOT NULL,
            to_id TEXT NOT NULL,
            edge_type TEXT NOT NULL,
            properties TEXT NOT NULL DEFAULT '{}',
            created_at TEXT NOT NULL,
            valid_from TEXT NOT NULL,
            valid_until TEXT
        );
        CREATE INDEX IF NOT EXISTS idx_edge_from ON edge(from_id);
        CREATE INDEX IF NOT EXISTS idx_edge_to ON edge(to_id);
        CREATE INDEX IF NOT EXISTS idx_edge_type ON edge(edge_type);

        CREATE TABLE IF NOT EXISTS code_file (
            id TEXT PRIMARY KEY,
            path TEXT NOT NULL UNIQUE,
            language TEXT NOT NULL,
            project TEXT NOT NULL DEFAULT '',
            hash TEXT NOT NULL,
            indexed_at TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_code_file_path ON code_file(path);

        CREATE TABLE IF NOT EXISTS code_symbol (
            id TEXT PRIMARY KEY,
            name TEXT NOT NULL,
            kind TEXT NOT NULL,
            file_path TEXT NOT NULL,
            line_start INTEGER,
            line_end INTEGER,
            signature TEXT
        );
        CREATE INDEX IF NOT EXISTS idx_code_symbol_name ON code_symbol(name);
        CREATE INDEX IF NOT EXISTS idx_code_symbol_file ON code_symbol(file_path);

        CREATE TABLE IF NOT EXISTS session (
            id TEXT PRIMARY KEY,
            agent TEXT NOT NULL,
            project TEXT,
            cwd TEXT,
            started_at TEXT NOT NULL,
            ended_at TEXT,
            status TEXT NOT NULL DEFAULT 'active'
        );
        CREATE INDEX IF NOT EXISTS idx_session_agent ON session(agent);
        CREATE INDEX IF NOT EXISTS idx_session_status ON session(status);

        -- Manas Layer 0: Platform
        CREATE TABLE IF NOT EXISTS platform (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL,
            detected_at TEXT NOT NULL
        );

        -- Manas Layer 1: Tools
        CREATE TABLE IF NOT EXISTS tool (
            id TEXT PRIMARY KEY,
            name TEXT NOT NULL,
            kind TEXT NOT NULL,
            capabilities TEXT NOT NULL DEFAULT '[]',
            config TEXT,
            health TEXT NOT NULL DEFAULT 'unknown',
            last_used TEXT,
            use_count INTEGER NOT NULL DEFAULT 0,
            discovered_at TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_tool_kind ON tool(kind);
        CREATE INDEX IF NOT EXISTS idx_tool_health ON tool(health);

        -- Manas Layer 2: Skills
        CREATE TABLE IF NOT EXISTS skill (
            id TEXT PRIMARY KEY,
            name TEXT NOT NULL,
            domain TEXT NOT NULL,
            description TEXT NOT NULL,
            steps TEXT NOT NULL DEFAULT '[]',
            success_count INTEGER NOT NULL DEFAULT 0,
            fail_count INTEGER NOT NULL DEFAULT 0,
            last_used TEXT,
            source TEXT NOT NULL,
            version INTEGER NOT NULL DEFAULT 1,
            project TEXT
        );
        CREATE INDEX IF NOT EXISTS idx_skill_domain ON skill(domain);
        CREATE INDEX IF NOT EXISTS idx_skill_source ON skill(source);
        CREATE INDEX IF NOT EXISTS idx_skill_project ON skill(project);

        -- Manas Layer 3: Domain DNA
        CREATE TABLE IF NOT EXISTS domain_dna (
            id TEXT PRIMARY KEY,
            project TEXT NOT NULL,
            aspect TEXT NOT NULL,
            pattern TEXT NOT NULL,
            confidence REAL NOT NULL DEFAULT 0.5,
            evidence TEXT NOT NULL DEFAULT '[]',
            detected_at TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_domain_dna_project ON domain_dna(project);
        CREATE INDEX IF NOT EXISTS idx_domain_dna_aspect ON domain_dna(aspect);

        -- Manas Layer 4: Perception
        CREATE TABLE IF NOT EXISTS perception (
            id TEXT PRIMARY KEY,
            kind TEXT NOT NULL,
            data TEXT NOT NULL,
            severity TEXT NOT NULL DEFAULT 'info',
            project TEXT,
            created_at TEXT NOT NULL,
            expires_at TEXT,
            consumed INTEGER NOT NULL DEFAULT 0
        );
        CREATE INDEX IF NOT EXISTS idx_perception_kind ON perception(kind);
        CREATE INDEX IF NOT EXISTS idx_perception_consumed ON perception(consumed);
        CREATE INDEX IF NOT EXISTS idx_perception_project ON perception(project);

        -- Manas Layer 5: Declared Knowledge
        CREATE TABLE IF NOT EXISTS declared (
            id TEXT PRIMARY KEY,
            source TEXT NOT NULL,
            path TEXT,
            content TEXT NOT NULL,
            hash TEXT NOT NULL,
            project TEXT,
            ingested_at TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_declared_source ON declared(source);
        CREATE INDEX IF NOT EXISTS idx_declared_project ON declared(project);
        CREATE INDEX IF NOT EXISTS idx_declared_hash ON declared(hash);

        -- Manas Layer 6: Identity
        CREATE TABLE IF NOT EXISTS identity (
            id TEXT PRIMARY KEY,
            agent TEXT NOT NULL,
            facet TEXT NOT NULL,
            description TEXT NOT NULL,
            strength REAL NOT NULL DEFAULT 0.5,
            source TEXT NOT NULL,
            active INTEGER NOT NULL DEFAULT 1,
            created_at TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_identity_agent ON identity(agent);
        CREATE INDEX IF NOT EXISTS idx_identity_facet ON identity(facet);
        CREATE INDEX IF NOT EXISTS idx_identity_active ON identity(active);

        -- Manas Layer 7: Disposition
        CREATE TABLE IF NOT EXISTS disposition (
            id TEXT PRIMARY KEY,
            agent TEXT NOT NULL,
            trait_name TEXT NOT NULL,
            domain TEXT,
            value REAL NOT NULL,
            trend TEXT NOT NULL DEFAULT 'stable',
            updated_at TEXT NOT NULL,
            evidence TEXT NOT NULL DEFAULT '[]'
        );
        CREATE INDEX IF NOT EXISTS idx_disposition_agent ON disposition(agent);
        CREATE INDEX IF NOT EXISTS idx_disposition_trait ON disposition(trait_name);

        -- Observability: metrics for extraction, embedding, and other operations
        CREATE TABLE IF NOT EXISTS metrics (
            id TEXT PRIMARY KEY,
            metric_type TEXT NOT NULL,
            timestamp TEXT NOT NULL,
            model TEXT,
            tokens_in INTEGER DEFAULT 0,
            tokens_out INTEGER DEFAULT 0,
            latency_ms INTEGER DEFAULT 0,
            cost_usd REAL DEFAULT 0.0,
            status TEXT DEFAULT 'ok',
            details TEXT DEFAULT '{}'
        );
        CREATE INDEX IF NOT EXISTS idx_metrics_type_time ON metrics(metric_type, timestamp);

        -- Chitta: Proactive diagnostic cache
        CREATE TABLE IF NOT EXISTS diagnostic (
            id TEXT PRIMARY KEY,
            file_path TEXT NOT NULL,
            severity TEXT NOT NULL,
            message TEXT NOT NULL,
            source TEXT NOT NULL,
            line INTEGER,
            col INTEGER,
            created_at TEXT NOT NULL,
            expires_at TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_diagnostic_file ON diagnostic(file_path);
        CREATE INDEX IF NOT EXISTS idx_diagnostic_expires ON diagnostic(expires_at);

        -- Knowledge Intelligence: Entity tracking
        CREATE TABLE IF NOT EXISTS entity (
            id TEXT PRIMARY KEY,
            name TEXT NOT NULL,
            entity_type TEXT NOT NULL DEFAULT 'concept',
            description TEXT NOT NULL DEFAULT '',
            mention_count INTEGER NOT NULL DEFAULT 1,
            first_seen TEXT NOT NULL,
            last_seen TEXT NOT NULL,
            project TEXT
        );
        CREATE INDEX IF NOT EXISTS idx_entity_name ON entity(name);
        CREATE INDEX IF NOT EXISTS idx_entity_project ON entity(project);
        CREATE INDEX IF NOT EXISTS idx_entity_type ON entity(entity_type);
    ")?;

    // v2.0 Entity Model
    conn.execute_batch("
        CREATE TABLE IF NOT EXISTS organization (
            id TEXT PRIMARY KEY,
            name TEXT NOT NULL UNIQUE,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS forge_user (
            id TEXT PRIMARY KEY,
            name TEXT NOT NULL,
            email TEXT,
            organization_id TEXT NOT NULL DEFAULT 'default',
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_user_org ON forge_user(organization_id);

        CREATE TABLE IF NOT EXISTS team (
            id TEXT PRIMARY KEY,
            name TEXT NOT NULL,
            organization_id TEXT NOT NULL DEFAULT 'default',
            created_by TEXT NOT NULL DEFAULT 'system',
            status TEXT NOT NULL DEFAULT 'active',
            created_at TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_team_org ON team(organization_id);

        CREATE TABLE IF NOT EXISTS team_member (
            team_id TEXT NOT NULL,
            user_id TEXT NOT NULL,
            role TEXT NOT NULL DEFAULT 'member',
            joined_at TEXT NOT NULL,
            PRIMARY KEY (team_id, user_id)
        );

        CREATE TABLE IF NOT EXISTS reality (
            id TEXT PRIMARY KEY,
            name TEXT NOT NULL,
            reality_type TEXT NOT NULL DEFAULT 'code',
            detected_from TEXT,
            project_path TEXT,
            domain TEXT,
            organization_id TEXT NOT NULL DEFAULT 'default',
            owner_type TEXT NOT NULL DEFAULT 'user',
            owner_id TEXT NOT NULL DEFAULT 'local',
            engine_status TEXT NOT NULL DEFAULT 'idle',
            engine_pid INTEGER,
            created_at TEXT NOT NULL,
            last_active TEXT NOT NULL,
            metadata TEXT DEFAULT '{}'
        );
        CREATE INDEX IF NOT EXISTS idx_reality_org ON reality(organization_id);
        CREATE INDEX IF NOT EXISTS idx_reality_path ON reality(project_path);
        CREATE INDEX IF NOT EXISTS idx_reality_owner ON reality(owner_type, owner_id);

        -- Scoped configuration
        CREATE TABLE IF NOT EXISTS config_scope (
            id TEXT PRIMARY KEY,
            scope_type TEXT NOT NULL,
            scope_id TEXT NOT NULL,
            key TEXT NOT NULL,
            value TEXT NOT NULL,
            locked INTEGER NOT NULL DEFAULT 0,
            ceiling REAL,
            set_by TEXT NOT NULL DEFAULT 'system',
            set_at TEXT NOT NULL,
            UNIQUE(scope_type, scope_id, key)
        );
        CREATE INDEX IF NOT EXISTS idx_config_scope_lookup ON config_scope(scope_type, scope_id);

        -- Permission rules (RBAC)
        CREATE TABLE IF NOT EXISTS permission_rule (
            id TEXT PRIMARY KEY,
            scope_type TEXT NOT NULL,
            scope_id TEXT NOT NULL,
            role TEXT NOT NULL,
            action TEXT NOT NULL,
            resource_type TEXT NOT NULL,
            effect TEXT NOT NULL DEFAULT 'allow',
            created_at TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_perm_scope ON permission_rule(scope_type, scope_id, role);

        -- Audit log
        CREATE TABLE IF NOT EXISTS audit_log (
            id TEXT PRIMARY KEY,
            actor_type TEXT NOT NULL,
            actor_id TEXT NOT NULL,
            action TEXT NOT NULL,
            resource_type TEXT NOT NULL,
            resource_id TEXT NOT NULL,
            scope_path TEXT,
            details TEXT DEFAULT '{}',
            timestamp TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_audit_scope ON audit_log(scope_path, timestamp);
        CREATE INDEX IF NOT EXISTS idx_audit_actor ON audit_log(actor_id, timestamp);
    ")?;

    // A2A permission table: controlled inter-session messaging permissions
    conn.execute_batch("
        CREATE TABLE IF NOT EXISTS a2a_permission (
            id TEXT PRIMARY KEY,
            from_agent TEXT NOT NULL,
            to_agent TEXT NOT NULL,
            from_project TEXT,
            to_project TEXT,
            allowed INTEGER NOT NULL DEFAULT 1,
            created_by TEXT NOT NULL DEFAULT 'system',
            created_at TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_a2a_perm_from ON a2a_permission(from_agent);
        CREATE INDEX IF NOT EXISTS idx_a2a_perm_to ON a2a_permission(to_agent);
    ")?;

    // Bootstrap: transcript processing log for efficient skip/resume
    conn.execute_batch("
        CREATE TABLE IF NOT EXISTS transcript_log (
            path TEXT PRIMARY KEY,
            adapter TEXT NOT NULL,
            project TEXT,
            size_bytes INTEGER NOT NULL,
            offset_processed INTEGER NOT NULL DEFAULT 0,
            content_hash TEXT NOT NULL,
            processed_at TEXT NOT NULL,
            memories_extracted INTEGER NOT NULL DEFAULT 0
        );
    ")?;

    // A2A FISP: inter-session message queue
    conn.execute_batch("
        CREATE TABLE IF NOT EXISTS session_message (
            id TEXT PRIMARY KEY,
            from_session TEXT NOT NULL,
            to_session TEXT NOT NULL,
            kind TEXT NOT NULL,
            topic TEXT NOT NULL DEFAULT '',
            parts TEXT NOT NULL DEFAULT '[]',
            status TEXT NOT NULL DEFAULT 'pending',
            in_reply_to TEXT,
            project TEXT,
            timeout_secs INTEGER,
            created_at TEXT NOT NULL,
            delivered_at TEXT,
            expires_at TEXT
        );
        CREATE INDEX IF NOT EXISTS idx_msg_to ON session_message(to_session, status);
        CREATE INDEX IF NOT EXISTS idx_msg_from ON session_message(from_session);
        CREATE INDEX IF NOT EXISTS idx_msg_reply ON session_message(in_reply_to);
    ")?;

    // Add valence columns (safe to re-run — ignores if already exists)
    let _ = conn.execute("ALTER TABLE memory ADD COLUMN valence TEXT NOT NULL DEFAULT 'neutral'", []);
    let _ = conn.execute("ALTER TABLE memory ADD COLUMN intensity REAL NOT NULL DEFAULT 0.0", []);

    // Add HLC sync columns (safe to re-run — ignores if already exists)
    let _ = conn.execute("ALTER TABLE memory ADD COLUMN hlc_timestamp TEXT NOT NULL DEFAULT ''", []);
    let _ = conn.execute("ALTER TABLE memory ADD COLUMN node_id TEXT NOT NULL DEFAULT ''", []);

    // Add session provenance columns (safe to re-run — ignores if already exists)
    let _ = conn.execute("ALTER TABLE memory ADD COLUMN session_id TEXT NOT NULL DEFAULT ''", []);
    let _ = conn.execute("ALTER TABLE memory ADD COLUMN access_count INTEGER NOT NULL DEFAULT 0", []);

    // Add working set column to session table (safe to re-run)
    let _ = conn.execute("ALTER TABLE session ADD COLUMN working_set TEXT NOT NULL DEFAULT ''", []);

    // Add activation_level column for activation tracking (safe to re-run)
    let _ = conn.execute("ALTER TABLE memory ADD COLUMN activation_level REAL DEFAULT 0.0", []);

    // Skill Intelligence: behavioral skill columns (safe to re-run — ignores if already exists)
    let _ = conn.execute("ALTER TABLE skill ADD COLUMN skill_type TEXT NOT NULL DEFAULT 'procedural'", []);
    let _ = conn.execute("ALTER TABLE skill ADD COLUMN user_specific INTEGER NOT NULL DEFAULT 0", []);
    let _ = conn.execute("ALTER TABLE skill ADD COLUMN observed_count INTEGER NOT NULL DEFAULT 1", []);
    let _ = conn.execute("ALTER TABLE skill ADD COLUMN correlation_ids TEXT NOT NULL DEFAULT '[]'", []);

    // Cross-session awareness: track tool_use count per session (safe to re-run)
    let _ = conn.execute("ALTER TABLE session ADD COLUMN tool_use_count INTEGER NOT NULL DEFAULT 0", []);
    // Counterfactual + relational memory columns (safe to re-run — ignores if already exists)
    let _ = conn.execute("ALTER TABLE memory ADD COLUMN alternatives TEXT NOT NULL DEFAULT '[]'", []);
    let _ = conn.execute("ALTER TABLE memory ADD COLUMN participants TEXT NOT NULL DEFAULT '[]'", []);

    // A2A FISP: session capabilities and current task (safe to re-run — ignores if already exists)
    let _ = conn.execute("ALTER TABLE session ADD COLUMN capabilities TEXT NOT NULL DEFAULT '[]'", []);
    let _ = conn.execute("ALTER TABLE session ADD COLUMN current_task TEXT NOT NULL DEFAULT ''", []);

    // Memory Intelligence: quality score column (safe to re-run — ignores if already exists)
    let _ = conn.execute("ALTER TABLE memory ADD COLUMN quality_score REAL DEFAULT 0.5", []);

    // v2.0: Scoping columns on existing tables
    // Session hierarchy + scoping
    let _ = conn.execute("ALTER TABLE session ADD COLUMN user_id TEXT", []);
    let _ = conn.execute("ALTER TABLE session ADD COLUMN team_id TEXT", []);
    let _ = conn.execute("ALTER TABLE session ADD COLUMN organization_id TEXT DEFAULT 'default'", []);
    let _ = conn.execute("ALTER TABLE session ADD COLUMN reality_id TEXT", []);
    let _ = conn.execute("ALTER TABLE session ADD COLUMN parent_session_id TEXT", []);

    // Memory scoping + portability
    let _ = conn.execute("ALTER TABLE memory ADD COLUMN user_id TEXT", []);
    let _ = conn.execute("ALTER TABLE memory ADD COLUMN reality_id TEXT", []);
    let _ = conn.execute("ALTER TABLE memory ADD COLUMN portability TEXT DEFAULT 'unknown'", []);
    let _ = conn.execute("ALTER TABLE memory ADD COLUMN visibility TEXT DEFAULT 'inherited'", []);
    let _ = conn.execute("ALTER TABLE memory ADD COLUMN deleted_at TEXT", []);

    // Identity scoping (per-user, not per-agent-type)
    let _ = conn.execute("ALTER TABLE identity ADD COLUMN user_id TEXT", []);
    let _ = conn.execute("ALTER TABLE identity ADD COLUMN organization_id TEXT DEFAULT 'default'", []);

    // Entity scoping
    let _ = conn.execute("ALTER TABLE entity ADD COLUMN reality_id TEXT", []);
    let _ = conn.execute("ALTER TABLE entity ADD COLUMN user_id TEXT", []);

    // Edge scoping
    let _ = conn.execute("ALTER TABLE edge ADD COLUMN reality_id TEXT", []);

    // Code file scoping
    let _ = conn.execute("ALTER TABLE code_file ADD COLUMN reality_id TEXT", []);

    // v2.0: Composite indexes for scoped queries
    let _ = conn.execute("CREATE INDEX IF NOT EXISTS idx_memory_reality ON memory(reality_id, memory_type, status)", []);
    let _ = conn.execute("CREATE INDEX IF NOT EXISTS idx_memory_user ON memory(user_id, memory_type, status)", []);
    let _ = conn.execute("CREATE INDEX IF NOT EXISTS idx_memory_portability ON memory(portability)", []);
    let _ = conn.execute("CREATE INDEX IF NOT EXISTS idx_session_reality ON session(reality_id, status)", []);
    let _ = conn.execute("CREATE INDEX IF NOT EXISTS idx_session_user ON session(user_id, status)", []);
    let _ = conn.execute("CREATE INDEX IF NOT EXISTS idx_session_parent ON session(parent_session_id)", []);
    let _ = conn.execute("CREATE INDEX IF NOT EXISTS idx_identity_user ON identity(user_id, active)", []);
    let _ = conn.execute("CREATE INDEX IF NOT EXISTS idx_entity_reality ON entity(reality_id)", []);
    let _ = conn.execute("CREATE INDEX IF NOT EXISTS idx_edge_reality ON edge(reality_id, edge_type)", []);
    let _ = conn.execute("CREATE INDEX IF NOT EXISTS idx_code_file_reality ON code_file(reality_id)", []);

    // v2.0 fix: Missing indexes for cross-org query performance
    let _ = conn.execute("CREATE INDEX IF NOT EXISTS idx_session_org ON session(organization_id)", []);
    let _ = conn.execute("CREATE INDEX IF NOT EXISTS idx_session_team ON session(team_id)", []);
    let _ = conn.execute("CREATE INDEX IF NOT EXISTS idx_identity_org ON identity(organization_id)", []);
    let _ = conn.execute("CREATE INDEX IF NOT EXISTS idx_entity_user ON entity(user_id)", []);
    let _ = conn.execute("CREATE INDEX IF NOT EXISTS idx_team_member_user ON team_member(user_id)", []);

    // v2.0 fix: Unique constraint on reality(project_path) to prevent duplicate path rows.
    // Filtered: only applies to non-NULL project_path values.
    let _ = conn.execute("CREATE UNIQUE INDEX IF NOT EXISTS idx_reality_path_unique ON reality(project_path) WHERE project_path IS NOT NULL", []);

    // ── v2.1: Agent Teams ──

    // Agent template: reusable definition for agent roles
    conn.execute_batch("
        CREATE TABLE IF NOT EXISTS agent_template (
            id TEXT PRIMARY KEY,
            name TEXT NOT NULL,
            description TEXT NOT NULL DEFAULT '',
            agent_type TEXT NOT NULL,
            organization_id TEXT NOT NULL DEFAULT 'default',
            system_context TEXT NOT NULL DEFAULT '',
            identity_facets TEXT NOT NULL DEFAULT '[]',
            config_overrides TEXT NOT NULL DEFAULT '{}',
            knowledge_domains TEXT NOT NULL DEFAULT '[]',
            decision_style TEXT NOT NULL DEFAULT 'analytical',
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );
        CREATE UNIQUE INDEX IF NOT EXISTS idx_agent_template_name ON agent_template(name, organization_id);
        CREATE INDEX IF NOT EXISTS idx_agent_template_org ON agent_template(organization_id);
    ")?;

    // Meeting: structured multi-agent deliberation
    conn.execute_batch("
        CREATE TABLE IF NOT EXISTS meeting (
            id TEXT PRIMARY KEY,
            team_id TEXT NOT NULL,
            topic TEXT NOT NULL,
            context TEXT,
            status TEXT NOT NULL DEFAULT 'open',
            orchestrator_session_id TEXT NOT NULL,
            synthesis TEXT,
            decision TEXT,
            decision_memory_id TEXT,
            created_at TEXT NOT NULL,
            decided_at TEXT
        );
        CREATE INDEX IF NOT EXISTS idx_meeting_team ON meeting(team_id, status);
        CREATE INDEX IF NOT EXISTS idx_meeting_orchestrator ON meeting(orchestrator_session_id);
    ")?;

    // Meeting participant: tracks each agent's response in a meeting
    conn.execute_batch("
        CREATE TABLE IF NOT EXISTS meeting_participant (
            id TEXT PRIMARY KEY,
            meeting_id TEXT NOT NULL,
            session_id TEXT NOT NULL,
            template_name TEXT NOT NULL DEFAULT '',
            status TEXT NOT NULL DEFAULT 'pending',
            response TEXT,
            responded_at TEXT,
            confidence REAL
        );
        CREATE INDEX IF NOT EXISTS idx_mp_meeting ON meeting_participant(meeting_id, status);
        CREATE INDEX IF NOT EXISTS idx_mp_session ON meeting_participant(session_id);
    ")?;

    // Agent lifecycle: session gains template tracking, agent status, last activity
    let _ = conn.execute("ALTER TABLE session ADD COLUMN template_id TEXT", []);
    let _ = conn.execute("ALTER TABLE session ADD COLUMN agent_status TEXT DEFAULT 'idle'", []);
    let _ = conn.execute("ALTER TABLE session ADD COLUMN last_activity_at TEXT", []);

    // Session heartbeat: lightweight keep-alive separate from semantic last_activity_at
    let _ = conn.execute("ALTER TABLE session ADD COLUMN last_heartbeat_at TEXT", []);

    // Team enhancements: type, orchestrator, purpose
    let _ = conn.execute("ALTER TABLE team ADD COLUMN team_type TEXT DEFAULT 'human'", []);
    let _ = conn.execute("ALTER TABLE team ADD COLUMN orchestrator_session_id TEXT", []);
    let _ = conn.execute("ALTER TABLE team ADD COLUMN purpose TEXT", []);

    // Team member: support agent sessions (not just user_id)
    let _ = conn.execute("ALTER TABLE team_member ADD COLUMN session_id TEXT", []);

    // FISP: meeting_id for deterministic response matching
    let _ = conn.execute("ALTER TABLE session_message ADD COLUMN meeting_id TEXT", []);
    let _ = conn.execute("CREATE INDEX IF NOT EXISTS idx_msg_meeting ON session_message(meeting_id)", []);

    // Agent team indexes
    let _ = conn.execute("CREATE INDEX IF NOT EXISTS idx_session_template ON session(template_id)", []);
    let _ = conn.execute("CREATE INDEX IF NOT EXISTS idx_session_agent_status ON session(agent_status)", []);
    let _ = conn.execute("CREATE INDEX IF NOT EXISTS idx_session_heartbeat ON session(status, last_heartbeat_at)", []);

    // ── v2.2: Notification Engine ──

    conn.execute_batch("
        CREATE TABLE IF NOT EXISTS notification (
            id TEXT PRIMARY KEY,
            category TEXT NOT NULL,
            priority TEXT NOT NULL,
            title TEXT NOT NULL,
            content TEXT NOT NULL,
            source TEXT NOT NULL,
            source_id TEXT,
            target_type TEXT DEFAULT 'broadcast',
            target_id TEXT,
            status TEXT NOT NULL DEFAULT 'pending',
            action_type TEXT,
            action_payload TEXT,
            action_result TEXT,
            created_at TEXT NOT NULL,
            delivered_at TEXT,
            acknowledged_at TEXT,
            expires_at TEXT,
            organization_id TEXT DEFAULT 'default',
            reality_id TEXT,
            topic TEXT,
            metadata TEXT DEFAULT '{}'
        );
        CREATE INDEX IF NOT EXISTS idx_notif_status ON notification(status, priority);
        CREATE INDEX IF NOT EXISTS idx_notif_target ON notification(target_type, target_id, status);
        CREATE INDEX IF NOT EXISTS idx_notif_topic ON notification(topic, created_at);

        CREATE TABLE IF NOT EXISTS notification_tuning (
            topic TEXT NOT NULL,
            user_id TEXT NOT NULL DEFAULT 'local',
            dismiss_count INTEGER NOT NULL DEFAULT 0,
            ack_count INTEGER NOT NULL DEFAULT 0,
            last_adjusted_at TEXT,
            priority_override TEXT,
            PRIMARY KEY (topic, user_id)
        );
    ")?;

    // ── Enterprise: RBAC audit columns on audit_log ──
    // These extend the existing audit_log table (v2.0 entity model) with
    // columns needed for HTTP RBAC audit logging. Existing rows are unaffected.
    let _ = conn.execute("ALTER TABLE audit_log ADD COLUMN user_id TEXT NOT NULL DEFAULT 'local'", []);
    let _ = conn.execute("ALTER TABLE audit_log ADD COLUMN email TEXT NOT NULL DEFAULT ''", []);
    let _ = conn.execute("ALTER TABLE audit_log ADD COLUMN role TEXT NOT NULL DEFAULT ''", []);
    let _ = conn.execute("ALTER TABLE audit_log ADD COLUMN request_type TEXT NOT NULL DEFAULT ''", []);
    let _ = conn.execute("ALTER TABLE audit_log ADD COLUMN request_summary TEXT NOT NULL DEFAULT ''", []);
    let _ = conn.execute("ALTER TABLE audit_log ADD COLUMN source TEXT NOT NULL DEFAULT 'socket'", []);
    let _ = conn.execute("ALTER TABLE audit_log ADD COLUMN source_ip TEXT NOT NULL DEFAULT ''", []);
    let _ = conn.execute("ALTER TABLE audit_log ADD COLUMN response_status TEXT NOT NULL DEFAULT 'ok'", []);
    let _ = conn.execute("CREATE INDEX IF NOT EXISTS idx_audit_log_timestamp ON audit_log(timestamp)", []);
    let _ = conn.execute("CREATE INDEX IF NOT EXISTS idx_audit_log_user ON audit_log(user_id)", []);

    // Append-only enforcement: block UPDATE and DELETE on audit_log
    let _ = conn.execute_batch(
        "CREATE TRIGGER IF NOT EXISTS audit_log_no_update
         BEFORE UPDATE ON audit_log
         BEGIN
             SELECT RAISE(ABORT, 'audit_log is append-only: UPDATE not allowed');
         END;
         CREATE TRIGGER IF NOT EXISTS audit_log_no_delete
         BEFORE DELETE ON audit_log
         BEGIN
             SELECT RAISE(ABORT, 'audit_log is append-only: DELETE not allowed');
         END;"
    );

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_schema_creates_tables() {
        crate::db::vec::init_sqlite_vec();
        let conn = Connection::open_in_memory().unwrap();
        create_schema(&conn).unwrap();

        // Verify memory table exists by querying sqlite_master
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='memory'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1, "memory table should exist");

        // Also verify edge table
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='edge'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1, "edge table should exist");
    }

    #[test]
    fn test_schema_idempotent() {
        crate::db::vec::init_sqlite_vec();
        let conn = Connection::open_in_memory().unwrap();
        // Calling create_schema twice should not error
        create_schema(&conn).unwrap();
        create_schema(&conn).unwrap();
    }

    #[test]
    fn test_valence_columns_exist() {
        crate::db::vec::init_sqlite_vec();
        let conn = Connection::open_in_memory().unwrap();
        create_schema(&conn).unwrap();
        // Verify we can insert with valence
        conn.execute(
            "INSERT INTO memory (id, memory_type, title, content, confidence, status, tags, created_at, accessed_at, valence, intensity)
             VALUES ('v1', 'decision', 'test', 'test', 0.9, 'active', '[]', datetime('now'), datetime('now'), 'negative', 0.8)",
            [],
        ).unwrap();
        let valence: String = conn.query_row("SELECT valence FROM memory WHERE id = 'v1'", [], |r| r.get(0)).unwrap();
        assert_eq!(valence, "negative");
        let intensity: f64 = conn.query_row("SELECT intensity FROM memory WHERE id = 'v1'", [], |r| r.get(0)).unwrap();
        assert!((intensity - 0.8).abs() < f64::EPSILON);
    }

    #[test]
    fn test_hlc_columns_exist() {
        crate::db::vec::init_sqlite_vec();
        let conn = Connection::open_in_memory().unwrap();
        create_schema(&conn).unwrap();
        conn.execute(
            "INSERT INTO memory (id, memory_type, title, content, confidence, status, tags, created_at, accessed_at, valence, intensity, hlc_timestamp, node_id)
             VALUES ('h1', 'decision', 'test', 'test', 0.9, 'active', '[]', datetime('now'), datetime('now'), 'neutral', 0.0, '1712345678000-0-abc12345', 'abc12345')",
            [],
        ).unwrap();
        let hlc: String = conn.query_row("SELECT hlc_timestamp FROM memory WHERE id = 'h1'", [], |r| r.get(0)).unwrap();
        assert!(hlc.contains("abc12345"));
        let node: String = conn.query_row("SELECT node_id FROM memory WHERE id = 'h1'", [], |r| r.get(0)).unwrap();
        assert_eq!(node, "abc12345");
    }

    #[test]
    fn test_diagnostic_table_exists() {
        crate::db::vec::init_sqlite_vec();
        let conn = Connection::open_in_memory().unwrap();
        create_schema(&conn).unwrap();
        conn.execute(
            "INSERT INTO diagnostic (id, file_path, severity, message, source, line, created_at, expires_at)
             VALUES ('d1', 'src/main.rs', 'error', 'undefined variable x', 'pyright', 10, datetime('now'), datetime('now', '+5 minutes'))",
            [],
        ).unwrap();
        let msg: String = conn.query_row("SELECT message FROM diagnostic WHERE id = 'd1'", [], |r| r.get(0)).unwrap();
        assert_eq!(msg, "undefined variable x");
    }

    #[test]
    fn test_v2_entity_tables_exist() {
        crate::db::vec::init_sqlite_vec();
        let conn = Connection::open_in_memory().unwrap();
        create_schema(&conn).unwrap();

        let v2_tables = [
            "organization",
            "forge_user",
            "team",
            "team_member",
            "reality",
            "config_scope",
            "permission_rule",
            "audit_log",
        ];

        for table_name in &v2_tables {
            let count: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?1",
                    [table_name],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(count, 1, "v2 table '{}' should exist", table_name);
        }
    }

    #[test]
    fn test_v2_scoping_columns_exist() {
        crate::db::vec::init_sqlite_vec();
        let conn = Connection::open_in_memory().unwrap();
        create_schema(&conn).unwrap();

        // Verify session scoping columns
        conn.execute(
            "UPDATE session SET user_id = 'u1', team_id = 't1', organization_id = 'org1', reality_id = 'r1', parent_session_id = 'ps1' WHERE 0",
            [],
        ).unwrap();

        // Verify memory scoping columns
        conn.execute(
            "UPDATE memory SET user_id = 'u1', reality_id = 'r1', portability = 'universal', visibility = 'local', deleted_at = NULL WHERE 0",
            [],
        ).unwrap();

        // Verify identity scoping columns
        conn.execute(
            "UPDATE identity SET user_id = 'u1', organization_id = 'org1' WHERE 0",
            [],
        ).unwrap();

        // Verify entity scoping columns
        conn.execute(
            "UPDATE entity SET reality_id = 'r1', user_id = 'u1' WHERE 0",
            [],
        ).unwrap();

        // Verify edge scoping columns
        conn.execute(
            "UPDATE edge SET reality_id = 'r1' WHERE 0",
            [],
        ).unwrap();

        // Verify code_file scoping columns
        conn.execute(
            "UPDATE code_file SET reality_id = 'r1' WHERE 0",
            [],
        ).unwrap();
    }

    #[test]
    fn test_config_scope_unique_constraint() {
        crate::db::vec::init_sqlite_vec();
        let conn = Connection::open_in_memory().unwrap();
        create_schema(&conn).unwrap();

        // Insert a config entry
        conn.execute(
            "INSERT INTO config_scope (id, scope_type, scope_id, key, value, set_at) VALUES ('c1', 'organization', 'default', 'max_tokens', '4096', datetime('now'))",
            [],
        ).unwrap();

        // Duplicate (scope_type, scope_id, key) should fail
        let result = conn.execute(
            "INSERT INTO config_scope (id, scope_type, scope_id, key, value, set_at) VALUES ('c2', 'organization', 'default', 'max_tokens', '8192', datetime('now'))",
            [],
        );
        assert!(result.is_err(), "duplicate config scope entry should fail");
    }

    #[test]
    fn test_audit_log_table() {
        crate::db::vec::init_sqlite_vec();
        let conn = Connection::open_in_memory().unwrap();
        create_schema(&conn).unwrap();

        conn.execute(
            "INSERT INTO audit_log (id, actor_type, actor_id, action, resource_type, resource_id, scope_path, timestamp)
             VALUES ('a1', 'user', 'local', 'create', 'memory', 'm1', 'default/local', datetime('now'))",
            [],
        ).unwrap();

        let action: String = conn.query_row(
            "SELECT action FROM audit_log WHERE id = 'a1'",
            [],
            |r| r.get(0),
        ).unwrap();
        assert_eq!(action, "create");
    }

    #[test]
    fn test_audit_log_rbac_columns() {
        crate::db::vec::init_sqlite_vec();
        let conn = Connection::open_in_memory().unwrap();
        create_schema(&conn).unwrap();

        // Insert an RBAC audit record using the new columns
        conn.execute(
            "INSERT INTO audit_log (id, actor_type, actor_id, action, resource_type, resource_id, timestamp,
             user_id, email, role, request_type, request_summary, source, source_ip, response_status)
             VALUES ('rbac1', 'http', 'user-123', 'remember', 'api', '/api', datetime('now'),
             'user-123', 'user@test.com', 'member', 'Remember', 'title=test', 'http', '10.0.0.1', 'ok')",
            [],
        ).unwrap();

        let (user_id, email, role, req_type, source, source_ip, status): (String, String, String, String, String, String, String) = conn.query_row(
            "SELECT user_id, email, role, request_type, source, source_ip, response_status FROM audit_log WHERE id = 'rbac1'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?)),
        ).unwrap();
        assert_eq!(user_id, "user-123");
        assert_eq!(email, "user@test.com");
        assert_eq!(role, "member");
        assert_eq!(req_type, "Remember");
        assert_eq!(source, "http");
        assert_eq!(source_ip, "10.0.0.1");
        assert_eq!(status, "ok");
    }

    #[test]
    fn test_manas_tables_exist() {
        crate::db::vec::init_sqlite_vec();
        let conn = Connection::open_in_memory().unwrap();
        create_schema(&conn).unwrap();

        let manas_tables = [
            "platform",
            "tool",
            "skill",
            "domain_dna",
            "perception",
            "declared",
            "identity",
            "disposition",
        ];

        for table_name in &manas_tables {
            let count: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?1",
                    [table_name],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(count, 1, "manas table '{}' should exist", table_name);
        }
    }
}
