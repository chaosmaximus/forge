use rusqlite::Connection;

/// Seed default agent templates if none exist.
/// Called during create_schema — idempotent (skips if templates already present).
fn seed_default_templates(conn: &Connection) -> rusqlite::Result<()> {
    let now = forge_core::time::now_iso();

    // Base agent templates (3 adapters)
    let base_templates = [
        (
            "claude-code",
            "Claude Code Agent",
            "General-purpose coding agent using Claude Code CLI",
            "claude-code",
            r#"["software-engineering","debugging","code-review","testing"]"#,
            "analytical",
        ),
        (
            "codex-cli",
            "Codex CLI Agent",
            "OpenAI Codex agent for adversarial review and second opinions",
            "codex",
            r#"["code-review","security-analysis","adversarial-testing"]"#,
            "critical",
        ),
        (
            "gemini-cli",
            "Gemini CLI Agent",
            "Google Gemini agent for research and alternative perspectives",
            "gemini",
            r#"["research","exploration","documentation"]"#,
            "exploratory",
        ),
    ];

    // Role-specific templates for team orchestration (referenced by team templates)
    let role_templates = [
        (
            "tech-lead",
            "Tech Lead",
            "Technical leadership: architecture decisions, code review, mentoring",
            "claude-code",
            r#"["architecture","code-review","mentoring","technical-design"]"#,
            "analytical",
        ),
        (
            "frontend-dev",
            "Frontend Developer",
            "Frontend implementation: UI components, state management, UX",
            "claude-code",
            r#"["frontend","ui","ux","react","css"]"#,
            "creative",
        ),
        (
            "backend-dev",
            "Backend Developer",
            "Backend implementation: APIs, databases, services, infrastructure",
            "claude-code",
            r#"["backend","api","database","infrastructure"]"#,
            "analytical",
        ),
        (
            "qa",
            "QA Engineer",
            "Quality assurance: test planning, automation, regression, edge cases",
            "claude-code",
            r#"["testing","qa","automation","edge-cases"]"#,
            "critical",
        ),
        (
            "devops",
            "DevOps Engineer",
            "Infrastructure: CI/CD, deployment, monitoring, scaling",
            "claude-code",
            r#"["devops","ci-cd","docker","kubernetes","monitoring"]"#,
            "analytical",
        ),
        (
            "security-lead",
            "Security Lead",
            "Security: threat modeling, vulnerability assessment, hardening",
            "claude-code",
            r#"["security","threat-modeling","penetration-testing"]"#,
            "critical",
        ),
        (
            "product-manager",
            "Product Manager",
            "Product strategy: requirements, prioritization, user stories",
            "claude-code",
            r#"["product","requirements","user-stories","prioritization"]"#,
            "strategic",
        ),
        (
            "content-writer",
            "Content Writer",
            "Content creation: blog posts, documentation, marketing copy",
            "claude-code",
            r#"["content","writing","marketing","seo"]"#,
            "creative",
        ),
        (
            "data-scientist",
            "Data Scientist",
            "Data analysis: statistics, ML, visualization, insights",
            "claude-code",
            r#"["data-science","statistics","ml","visualization"]"#,
            "analytical",
        ),
        (
            "ux-researcher",
            "UX Researcher",
            "User research: interviews, usability testing, personas",
            "claude-code",
            r#"["ux-research","usability","personas","user-interviews"]"#,
            "empathetic",
        ),
        (
            "ceo",
            "CEO",
            "Executive leadership: vision, strategy, fundraising, culture",
            "claude-code",
            r#"["strategy","leadership","fundraising","culture"]"#,
            "strategic",
        ),
        (
            "cto",
            "CTO",
            "Technical leadership: architecture, tech stack, engineering culture",
            "claude-code",
            r#"["architecture","tech-strategy","engineering-culture"]"#,
            "analytical",
        ),
        (
            "cfo",
            "CFO",
            "Financial leadership: budgets, forecasting, unit economics",
            "claude-code",
            r#"["finance","budgeting","forecasting","unit-economics"]"#,
            "analytical",
        ),
        (
            "cmo",
            "CMO",
            "Marketing leadership: brand, growth, channels, positioning",
            "claude-code",
            r#"["marketing","brand","growth","positioning"]"#,
            "creative",
        ),
        (
            "cpo",
            "CPO",
            "Product leadership: roadmap, user research, feature prioritization",
            "claude-code",
            r#"["product-strategy","roadmap","user-research"]"#,
            "strategic",
        ),
    ];

    // Idempotent: INSERT OR IGNORE so existing templates are not duplicated
    for (name, desc, system_ctx, agent_type, domains, style) in
        base_templates.iter().chain(role_templates.iter())
    {
        let id = ulid::Ulid::new().to_string();
        conn.execute(
            "INSERT OR IGNORE INTO agent_template (id, name, description, agent_type, system_context, knowledge_domains, decision_style, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8)",
            rusqlite::params![id, name, desc, agent_type, system_ctx, domains, style, now],
        )?;
    }

    Ok(())
}

pub fn create_schema(conn: &Connection) -> rusqlite::Result<()> {
    // P3-4 ZR-C3: rename legacy `reality` table → `project`. Idempotent;
    // runs once per legacy DB. Per
    // `feedback_sqlite_no_reverse_silent_migration_failure.md` we surface
    // errors with `?` instead of `let _ = conn.execute(...)` so a
    // botched ALTER cannot silently no-op.
    //
    // ZR-C3 fw1 (HIGH-1): handle the four-quadrant state matrix
    // explicitly so a mid-state DB (both `reality` AND `project`
    // present from a partial rename / aborted upgrade / downgrade-
    // then-upgrade) is never silently treated as already-migrated.
    {
        let reality_exists: i64 = conn.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='reality'",
            [],
            |r| r.get(0),
        )?;
        let project_exists: i64 = conn.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='project'",
            [],
            |r| r.get(0),
        )?;
        match (reality_exists == 1, project_exists == 1) {
            (true, false) => {
                // Clean legacy DB. ALTER TABLE … RENAME TO carries the
                // row data and re-binds existing indexes to the new
                // table name automatically. We DROP the legacy index
                // names below so subsequent CREATE INDEX statements
                // install the canonical names cleanly.
                conn.execute("ALTER TABLE reality RENAME TO project", [])?;
            }
            (true, true) => {
                // Mid-state: both tables exist. We refuse to clobber
                // `project` with `reality`'s data and refuse to silently
                // drop `reality`'s rows. If `reality` is empty (legacy
                // residue from a successful prior rename followed by
                // an inconsistent CREATE TABLE IF NOT EXISTS reality
                // earlier in this fn — pre-fw1 schema), drop it; if
                // it's non-empty, surface a hard error so the operator
                // resolves the ambiguity manually.
                let reality_rows: i64 = conn.query_row(
                    "SELECT COUNT(*) FROM reality",
                    [],
                    |r| r.get(0),
                )?;
                if reality_rows == 0 {
                    conn.execute("DROP TABLE reality", [])?;
                } else {
                    return Err(rusqlite::Error::SqliteFailure(
                        rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_ERROR),
                        Some(format!(
                            "P3-4 ZR-C3 mid-state DB: both `reality` ({reality_rows} \
                             rows) and `project` tables exist. The migration cannot \
                             safely merge them. Manually resolve: if `reality` is the \
                             intended source of truth, DROP TABLE project before \
                             restarting; otherwise DROP TABLE reality. See \
                             docs/operations/v0.6.0-pre-iteration-deferrals.md."
                        )),
                    ));
                }
            }
            // (false, true) — fresh-DB / post-migration steady state.
            // (false, false) — fresh-DB pre-CREATE TABLE; no migration needed.
            _ => {}
        }
    }

    // Drop legacy idx_reality_* names so the new idx_project_* CREATE
    // statements below can install the canonical names without a dupe.
    // DROP INDEX IF EXISTS is a no-op when the index isn't present.
    conn.execute("DROP INDEX IF EXISTS idx_reality_org", [])?;
    conn.execute("DROP INDEX IF EXISTS idx_reality_path", [])?;
    conn.execute("DROP INDEX IF EXISTS idx_reality_owner", [])?;
    conn.execute("DROP INDEX IF EXISTS idx_reality_path_unique", [])?;

    // Create memory_vec virtual table (sqlite-vec must be loaded before this call)
    conn.execute_batch(
        "CREATE VIRTUAL TABLE IF NOT EXISTS memory_vec USING vec0(
            id TEXT PRIMARY KEY,
            embedding float[768] distance_metric=cosine
        );",
    )?;

    // Create code_vec virtual table for code embeddings (sqlite-vec must be loaded before this call)
    conn.execute_batch(
        "CREATE VIRTUAL TABLE IF NOT EXISTS code_vec USING vec0(
            id TEXT PRIMARY KEY,
            embedding float[768] distance_metric=cosine
        );",
    )?;

    // Raw layer: verbatim chunk storage for benchmark parity with published retrieval systems.
    // 384-dim matches all-MiniLM-L6-v2 (fastembed-rs default) — do NOT merge with the
    // 768-dim memory_vec/code_vec tables above. Raw ingest is LLM-free and fires in parallel
    // with the extraction pipeline; both paths are independent.
    conn.execute_batch(
        "CREATE VIRTUAL TABLE IF NOT EXISTS raw_chunks_vec USING vec0(
            id TEXT PRIMARY KEY,
            embedding float[384] distance_metric=cosine
        );",
    )?;

    conn.execute_batch("
        CREATE TABLE IF NOT EXISTS raw_documents (
            id TEXT PRIMARY KEY,
            project TEXT,
            session_id TEXT,
            source TEXT NOT NULL,
            text TEXT NOT NULL,
            timestamp TEXT NOT NULL,
            metadata_json TEXT NOT NULL DEFAULT '{}'
        );
        CREATE INDEX IF NOT EXISTS idx_raw_documents_project ON raw_documents(project);
        CREATE INDEX IF NOT EXISTS idx_raw_documents_session ON raw_documents(session_id);
        CREATE INDEX IF NOT EXISTS idx_raw_documents_timestamp ON raw_documents(timestamp);
        CREATE INDEX IF NOT EXISTS idx_raw_documents_source ON raw_documents(source);

        -- raw_chunks MUST stay rowid-backed (no WITHOUT ROWID). The FTS5
        -- contentless table `raw_chunks_fts` below joins on `raw_chunks.rowid`
        -- via its triggers; removing rowid would silently break the BM25
        -- search path used by `db::raw::search_chunks_bm25`.
        CREATE TABLE IF NOT EXISTS raw_chunks (
            id TEXT PRIMARY KEY,
            document_id TEXT NOT NULL REFERENCES raw_documents(id) ON DELETE CASCADE,
            chunk_index INTEGER NOT NULL,
            text TEXT NOT NULL,
            metadata_json TEXT NOT NULL DEFAULT '{}',
            UNIQUE(document_id, chunk_index)
        );
        CREATE INDEX IF NOT EXISTS idx_raw_chunks_document ON raw_chunks(document_id);

        CREATE VIRTUAL TABLE IF NOT EXISTS raw_chunks_fts USING fts5(
            text,
            content='raw_chunks', content_rowid='rowid'
        );

        CREATE TRIGGER IF NOT EXISTS raw_chunks_fts_insert AFTER INSERT ON raw_chunks BEGIN
            INSERT INTO raw_chunks_fts(rowid, text) VALUES (new.rowid, new.text);
        END;

        CREATE TRIGGER IF NOT EXISTS raw_chunks_fts_delete AFTER DELETE ON raw_chunks BEGIN
            INSERT INTO raw_chunks_fts(raw_chunks_fts, rowid, text) VALUES ('delete', old.rowid, old.text);
        END;

        CREATE TRIGGER IF NOT EXISTS raw_chunks_fts_update AFTER UPDATE ON raw_chunks BEGIN
            INSERT INTO raw_chunks_fts(raw_chunks_fts, rowid, text) VALUES ('delete', old.rowid, old.text);
            INSERT INTO raw_chunks_fts(rowid, text) VALUES (new.rowid, new.text);
        END;
    ")?;

    // KPI observability tables — track retrieval events, hourly snapshots, benchmark runs,
    // and UAT user-story pass/fail. See docs/benchmarks/plan.md §11-13.
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS kpi_events (
            id TEXT PRIMARY KEY,
            timestamp INTEGER NOT NULL,
            event_type TEXT NOT NULL,
            project TEXT,
            latency_ms INTEGER,
            result_count INTEGER,
            success INTEGER NOT NULL,
            metadata_json TEXT NOT NULL DEFAULT '{}'
        );
        CREATE INDEX IF NOT EXISTS idx_kpi_events_timestamp ON kpi_events(timestamp);
        -- Pre-release audit E-9: composite (event_type, timestamp) index
        -- for the kpi_reaper retention pass (`DELETE … WHERE event_type = ?
        -- AND timestamp < ? LIMIT ?`). The pre-fix single-column
        -- `idx_kpi_events_type` forced the planner to pick one of the two
        -- conditions, then filter the rest in-row — at high event_type
        -- cardinality (e.g. `phase_completed` dominates the table) this
        -- devolves to a near-full-scan inside each batch and amplifies WAL
        -- contention with the W22 indexer. The composite serves the leftmost-
        -- prefix `WHERE event_type = ?` case identically to the legacy
        -- single-column index, AND serves the (event_type, timestamp) filter
        -- with a single seek — so the legacy index is now redundant. We
        -- keep the legacy index (CREATE IF NOT EXISTS is idempotent on
        -- existing DBs) so a downgrade is non-destructive; the composite is
        -- additive.
        CREATE INDEX IF NOT EXISTS idx_kpi_events_type ON kpi_events(event_type);
        CREATE INDEX IF NOT EXISTS idx_kpi_events_type_timestamp
            ON kpi_events(event_type, timestamp);
        -- Phase 2A-4d.2 T3: expression index on metadata_json.$.phase_name so
        -- /inspect's GROUP BY phase queries don't require a full JSON scan.
        -- SQLite >= 3.9.0 supports expression indexes; JSON1 is compiled in.
        CREATE INDEX IF NOT EXISTS idx_kpi_events_phase
            ON kpi_events(json_extract(metadata_json, '$.phase_name'));

        CREATE TABLE IF NOT EXISTS kpi_snapshots (
            id TEXT PRIMARY KEY,
            taken_at INTEGER NOT NULL,
            kpi_name TEXT NOT NULL,
            value REAL NOT NULL,
            window TEXT NOT NULL,
            project TEXT,
            metadata_json TEXT NOT NULL DEFAULT '{}'
        );
        CREATE INDEX IF NOT EXISTS idx_kpi_snapshots_taken_at ON kpi_snapshots(taken_at);
        CREATE INDEX IF NOT EXISTS idx_kpi_snapshots_name ON kpi_snapshots(kpi_name);

        CREATE TABLE IF NOT EXISTS kpi_benchmarks (
            id TEXT PRIMARY KEY,
            run_at INTEGER NOT NULL,
            benchmark TEXT NOT NULL,
            mode TEXT NOT NULL,
            metric TEXT NOT NULL,
            category TEXT,
            value REAL NOT NULL,
            n_questions INTEGER NOT NULL,
            full_run INTEGER NOT NULL,
            commit_sha TEXT,
            hardware TEXT,
            metadata_json TEXT NOT NULL DEFAULT '{}'
        );
        CREATE INDEX IF NOT EXISTS idx_kpi_benchmarks_run_at ON kpi_benchmarks(run_at);
        CREATE INDEX IF NOT EXISTS idx_kpi_benchmarks_bm ON kpi_benchmarks(benchmark, mode, metric);

        CREATE TABLE IF NOT EXISTS uat_stories (
            id TEXT PRIMARY KEY,
            name TEXT NOT NULL,
            description TEXT NOT NULL,
            benchmark TEXT NOT NULL,
            metric_name TEXT NOT NULL,
            metric_threshold REAL NOT NULL,
            last_run_at INTEGER,
            last_value REAL,
            last_passed INTEGER
        );
    ",
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
            accessed_at TEXT NOT NULL,
            organization_id TEXT NOT NULL DEFAULT 'default'
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
        -- NOTE: UNIQUE index created AFTER dedup migration (see end of create_schema)

        CREATE TABLE IF NOT EXISTS code_file (
            id TEXT PRIMARY KEY,
            path TEXT NOT NULL UNIQUE,
            language TEXT NOT NULL,
            project TEXT NOT NULL DEFAULT '_global_',
            hash TEXT NOT NULL,
            indexed_at TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_code_file_path ON code_file(path);
        -- P3-4 W1.25 (W1.3 LOW-6): composite (project, path) index so
        -- a project-filtered scan also covers the JOIN's
        -- code_symbol.file_path = code_file.path lookup. Pre-W1.25 the
        -- single-col idx_code_file_project required a second seek
        -- through idx_code_file_path or a sequential scan when the
        -- planner picked the symbol-side scan. Path-only queries still
        -- use idx_code_file_path (above) since composite indexes
        -- can't satisfy non-leftmost-prefix lookups.
        CREATE INDEX IF NOT EXISTS idx_code_file_project_path ON code_file(project, path);

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

        -- session.status (lifecycle of the connection itself):
        --   active  — recently active (heartbeat within heartbeat_idle_secs)
        --   idle    — quiet (heartbeat older than heartbeat_idle_secs but within
        --             heartbeat_timeout_secs); the next heartbeat atomically
        --             revives to active. See workers/reaper.rs Phase 0.
        --   ended   — reaped after no heartbeat for heartbeat_timeout_secs
        --
        -- A separate column agent_status (added by the agent-template migration
        -- below) tracks the agent current WORK state. The canonical values
        -- mirror the AgentStatus enum in crates/core/src/types/team.rs:
        --   idle / thinking / responding / in_meeting / error / retired
        -- The column is stored as a freeform TEXT (not constrained at the SQL
        -- level), so older rows or external writers may carry legacy values
        -- (e.g. busy / active / working) — readers should treat unknowns as
        -- equivalent to idle.
        -- The shared word idle across the two columns is intentional but
        -- distinct: session.status=idle means no-heartbeat-lately; while
        -- agent_status=idle means agent-is-between-turns. A session can be
        -- session.status=active AND agent_status=idle (alive, awaiting work)
        -- or session.status=idle AND agent_status=responding (heartbeat lapsed
        -- mid-task — operator should investigate).
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
        -- P3-3.11 W30: 'project' column added (NOT NULL, DEFAULT '_global_')
        -- so identity facets are scoped to (agent, project) instead of
        -- (agent) alone. Closes F16 cross-project identity pollution. The
        -- ALTER TABLE migration for existing DBs lives near the memory
        -- migrations below. Mirror of the W29 memory.project sentinel.
        CREATE TABLE IF NOT EXISTS identity (
            id TEXT PRIMARY KEY,
            agent TEXT NOT NULL,
            facet TEXT NOT NULL,
            description TEXT NOT NULL,
            strength REAL NOT NULL DEFAULT 0.5,
            source TEXT NOT NULL,
            active INTEGER NOT NULL DEFAULT 1,
            created_at TEXT NOT NULL,
            project TEXT NOT NULL DEFAULT '_global_'
        );
        CREATE INDEX IF NOT EXISTS idx_identity_agent ON identity(agent);
        CREATE INDEX IF NOT EXISTS idx_identity_facet ON identity(facet);
        CREATE INDEX IF NOT EXISTS idx_identity_active ON identity(active);
        -- W30 project indexes are created in the migration section below,
        -- after the ALTER TABLE that adds the project column on existing
        -- DBs (CREATE INDEX cannot reference a column that does not yet
        -- exist on a pre-W30 schema).

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
    conn.execute_batch(
        "
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
        -- NOTE: UNIQUE index created AFTER dedup migration (see end of create_schema)

        CREATE TABLE IF NOT EXISTS team_member (
            team_id TEXT NOT NULL,
            user_id TEXT NOT NULL,
            role TEXT NOT NULL DEFAULT 'member',
            joined_at TEXT NOT NULL,
            PRIMARY KEY (team_id, user_id)
        );

        CREATE TABLE IF NOT EXISTS project (
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
        CREATE INDEX IF NOT EXISTS idx_project_org ON project(organization_id);
        CREATE INDEX IF NOT EXISTS idx_project_path ON project(project_path);
        CREATE INDEX IF NOT EXISTS idx_project_owner ON project(owner_type, owner_id);

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
    ",
    )?;

    // A2A permission table: controlled inter-session messaging permissions
    conn.execute_batch(
        "
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
    ",
    )?;

    // Bootstrap: transcript processing log for efficient skip/resume
    conn.execute_batch(
        "
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
    ",
    )?;

    // A2A FISP: inter-session message queue
    conn.execute_batch(
        "
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
    ",
    )?;

    // Add valence columns (safe to re-run — ignores if already exists)
    let _ = conn.execute(
        "ALTER TABLE memory ADD COLUMN valence TEXT NOT NULL DEFAULT 'neutral'",
        [],
    );
    let _ = conn.execute(
        "ALTER TABLE memory ADD COLUMN intensity REAL NOT NULL DEFAULT 0.0",
        [],
    );

    // Add HLC sync columns (safe to re-run — ignores if already exists)
    let _ = conn.execute(
        "ALTER TABLE memory ADD COLUMN hlc_timestamp TEXT NOT NULL DEFAULT ''",
        [],
    );
    let _ = conn.execute(
        "ALTER TABLE memory ADD COLUMN node_id TEXT NOT NULL DEFAULT ''",
        [],
    );

    // Add session provenance columns (safe to re-run — ignores if already exists)
    let _ = conn.execute(
        "ALTER TABLE memory ADD COLUMN session_id TEXT NOT NULL DEFAULT ''",
        [],
    );
    let _ = conn.execute(
        "ALTER TABLE memory ADD COLUMN access_count INTEGER NOT NULL DEFAULT 0",
        [],
    );

    // Add working set column to session table (safe to re-run)
    let _ = conn.execute(
        "ALTER TABLE session ADD COLUMN working_set TEXT NOT NULL DEFAULT ''",
        [],
    );

    // Add activation_level column for activation tracking (safe to re-run)
    let _ = conn.execute(
        "ALTER TABLE memory ADD COLUMN activation_level REAL DEFAULT 0.0",
        [],
    );

    // Skill Intelligence: behavioral skill columns (safe to re-run — ignores if already exists)
    let _ = conn.execute(
        "ALTER TABLE skill ADD COLUMN skill_type TEXT NOT NULL DEFAULT 'procedural'",
        [],
    );
    let _ = conn.execute(
        "ALTER TABLE skill ADD COLUMN user_specific INTEGER NOT NULL DEFAULT 0",
        [],
    );
    let _ = conn.execute(
        "ALTER TABLE skill ADD COLUMN observed_count INTEGER NOT NULL DEFAULT 1",
        [],
    );
    let _ = conn.execute(
        "ALTER TABLE skill ADD COLUMN correlation_ids TEXT NOT NULL DEFAULT '[]'",
        [],
    );
    // Phase 2A-4c2: behavioral skill inference columns (safe to re-run — ignores if already exists)
    let _ = conn.execute(
        "ALTER TABLE skill ADD COLUMN agent TEXT NOT NULL DEFAULT 'claude-code'",
        [],
    );
    let _ = conn.execute(
        "ALTER TABLE skill ADD COLUMN fingerprint TEXT NOT NULL DEFAULT ''",
        [],
    );
    let _ = conn.execute(
        "ALTER TABLE skill ADD COLUMN inferred_from TEXT NOT NULL DEFAULT '[]'",
        [],
    );
    let _ = conn.execute("ALTER TABLE skill ADD COLUMN inferred_at TEXT", []);
    // Partial unique index on (agent, project, fingerprint) — gated on non-empty
    // fingerprint so existing rows with default '' do not collide. Project is
    // included so the same behavior pattern in different projects produces
    // distinct rows (T10 review Codex-H2). Safe to re-run (IF NOT EXISTS).
    //
    // Drop the pre-Codex-H2 index name if present so re-running schema init
    // against an older DB migrates cleanly.
    let _ = conn.execute_batch(
        "DROP INDEX IF EXISTS idx_skill_agent_fingerprint;
         CREATE UNIQUE INDEX IF NOT EXISTS idx_skill_agent_project_fingerprint
            ON skill(agent, project, fingerprint)
            WHERE fingerprint != '';",
    );

    // Cross-session awareness: track tool_use count per session (safe to re-run)
    let _ = conn.execute(
        "ALTER TABLE session ADD COLUMN tool_use_count INTEGER NOT NULL DEFAULT 0",
        [],
    );
    // Counterfactual + relational memory columns (safe to re-run — ignores if already exists)
    let _ = conn.execute(
        "ALTER TABLE memory ADD COLUMN alternatives TEXT NOT NULL DEFAULT '[]'",
        [],
    );
    let _ = conn.execute(
        "ALTER TABLE memory ADD COLUMN participants TEXT NOT NULL DEFAULT '[]'",
        [],
    );

    // A2A FISP: session capabilities and current task (safe to re-run — ignores if already exists)
    let _ = conn.execute(
        "ALTER TABLE session ADD COLUMN capabilities TEXT NOT NULL DEFAULT '[]'",
        [],
    );
    let _ = conn.execute(
        "ALTER TABLE session ADD COLUMN current_task TEXT NOT NULL DEFAULT ''",
        [],
    );

    // Memory Intelligence: quality score column (safe to re-run — ignores if already exists)
    let _ = conn.execute(
        "ALTER TABLE memory ADD COLUMN quality_score REAL DEFAULT 0.5",
        [],
    );

    // v2.0: Scoping columns on existing tables
    // Session hierarchy + scoping
    let _ = conn.execute("ALTER TABLE session ADD COLUMN user_id TEXT", []);
    let _ = conn.execute("ALTER TABLE session ADD COLUMN team_id TEXT", []);
    let _ = conn.execute(
        "ALTER TABLE session ADD COLUMN organization_id TEXT DEFAULT 'default'",
        [],
    );
    let _ = conn.execute("ALTER TABLE session ADD COLUMN reality_id TEXT", []);
    let _ = conn.execute("ALTER TABLE session ADD COLUMN parent_session_id TEXT", []);

    // Memory scoping + portability
    let _ = conn.execute("ALTER TABLE memory ADD COLUMN user_id TEXT", []);
    let _ = conn.execute("ALTER TABLE memory ADD COLUMN reality_id TEXT", []);
    let _ = conn.execute(
        "ALTER TABLE memory ADD COLUMN portability TEXT DEFAULT 'unknown'",
        [],
    );
    let _ = conn.execute(
        "ALTER TABLE memory ADD COLUMN visibility TEXT DEFAULT 'inherited'",
        [],
    );
    let _ = conn.execute("ALTER TABLE memory ADD COLUMN deleted_at TEXT", []);

    // Multi-tenant isolation: organization_id on memory (safe to re-run)
    let _ = conn.execute(
        "ALTER TABLE memory ADD COLUMN organization_id TEXT NOT NULL DEFAULT 'default'",
        [],
    );
    let _ = conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_memory_org ON memory(organization_id)",
        [],
    );
    // Backfill: derive org_id from the session that created each memory.
    // Pre-release audit E-15: was previously `let _ = conn.execute(...)`
    // which silently no-oped if the migration ordering ever regressed
    // and `session.organization_id` didn't exist yet — collapsing
    // multi-org isolation. `?` propagation surfaces the regression at
    // startup. The UPDATE itself is safe-by-WHERE on legacy DBs.
    conn.execute(
        "UPDATE memory SET organization_id = COALESCE(
            (SELECT s.organization_id FROM session s WHERE s.id = memory.session_id AND s.organization_id IS NOT NULL LIMIT 1),
            'default'
        ) WHERE organization_id = 'default' AND session_id != ''",
        [],
    )?;

    // Identity scoping (per-user, not per-agent-type)
    let _ = conn.execute("ALTER TABLE identity ADD COLUMN user_id TEXT", []);
    let _ = conn.execute(
        "ALTER TABLE identity ADD COLUMN organization_id TEXT DEFAULT 'default'",
        [],
    );

    // P3-3.11 W30: identity.project (per-(agent, project) scoping). Mirror of
    // the W29 memory.project sentinel migration. SQLite's ADD COLUMN with
    // NOT NULL DEFAULT 'literal' backfills existing rows to '_global_'
    // automatically, so no separate UPDATE is required for the column-add
    // path. The defensive UPDATE below (near the W29 backfill block)
    // re-normalises any NULL/empty values that may exist on a database
    // produced by a hypothetical pre-release build that ADDed the column
    // without the DEFAULT clause.
    let _ = conn.execute(
        "ALTER TABLE identity ADD COLUMN project TEXT NOT NULL DEFAULT '_global_'",
        [],
    );
    // Project indexes — created here (post-ALTER) rather than in the CREATE
    // TABLE batch above so they can reference the project column on a
    // pre-W30 database where the column did not exist at table-creation
    // time.
    let _ = conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_identity_project ON identity(project)",
        [],
    );
    let _ = conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_identity_agent_project \
         ON identity(agent, project, active)",
        [],
    );

    // Entity scoping
    let _ = conn.execute("ALTER TABLE entity ADD COLUMN reality_id TEXT", []);
    let _ = conn.execute("ALTER TABLE entity ADD COLUMN user_id TEXT", []);

    // Edge scoping
    let _ = conn.execute("ALTER TABLE edge ADD COLUMN reality_id TEXT", []);

    // Code file scoping
    let _ = conn.execute("ALTER TABLE code_file ADD COLUMN reality_id TEXT", []);

    // P3-4 W1.2 c1 (I-7) — code-graph per-project scoping (sentinel + basename migration).
    //
    // Pre-W1.2 daemons stored the full project directory PATH in
    // `code_file.project` (e.g. `/mnt/colab/.../forge/forge`), but
    // `memory.project` (W29) and `identity.project` (W30) store the
    // human-readable NAME (`forge`). That made `--project forge` filters
    // useless on find-symbol/code-search/blast-radius — symbols from
    // every indexed project leaked through. Repaired here in three passes:
    //
    // 1. Strip trailing slashes from rooted paths (W1.3 fw2 review MED-3
    //    fix). A path like `/foo/` would otherwise produce a basename
    //    of '' under the next pass (the trailing slash makes the "last
    //    separator" look like position 0), and the sentinel UPDATE
    //    would silently demote the real project NAME `foo` to
    //    `_global_`. RTRIM removes any trailing `/` chars before the
    //    basename pass runs.
    // 2. Backfill the basename-equivalent for any path-tagged legacy
    //    row. SQLite has no built-in `REVERSE()`, so we use the standard
    //    SQLite basename idiom: `REPLACE(p, RTRIM(p, REPLACE(p, '/', '')), '')`
    //    yields the suffix following the last `/` — see W1.3 fw2 review
    //    note (the original c1 SUBSTR/INSTR/REVERSE form silently no-op'd
    //    against `REVERSE()` not existing in SQLite, so on every legacy
    //    DB the basename rewrite never fired; the live `forge|188`
    //    verification only looked clean because the DB had been wiped
    //    fresh, never tested against a pre-W1.2 row).
    // 3. Defensive sentinel fallback for any row that's NULL / empty /
    //    root-only after the basename pass.
    //
    // Idempotent — re-running on already-migrated rows is a no-op
    // because basename(forge) = forge, RTRIM('forge', '/') = 'forge',
    // and the WHERE LIKE '/%' excludes already-basename rows from the
    // basename pass.
    let _ = conn.execute(
        "UPDATE code_file SET project = RTRIM(project, '/') WHERE project LIKE '%/'",
        [],
    );
    let _ = conn.execute(
        "UPDATE code_file
         SET project = REPLACE(project, RTRIM(project, REPLACE(project, '/', '')), '')
         WHERE project LIKE '/%'",
        [],
    );
    let _ = conn.execute(
        "UPDATE code_file SET project = '_global_' WHERE project IS NULL OR project = ''",
        [],
    );
    // P3-4 W1.25 (W1.3 LOW-6): drop the single-column legacy index in
    // favour of the composite (project, path) index. The composite
    // serves both `WHERE project = ?` queries (leftmost-column scan)
    // and the JOIN'd `WHERE project = ? AND code_symbol.file_path =
    // code_file.path` lookup with a single seek. Path-only queries
    // still use idx_code_file_path; nothing regresses by replacing.
    // Idempotent — DROP is no-op if the index has already been
    // dropped on a prior daemon start.
    let _ = conn.execute("DROP INDEX IF EXISTS idx_code_file_project", []);
    let _ = conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_code_file_project_path ON code_file(project, path)",
        [],
    );

    // P3-4 W1.3 fw2 (review HIGH-3) — companion DELETE for pre-W1.2
    // foreign-project pollution.
    //
    // Pre-W1.2, `find_project_dir`'s decode-fallback could land at a
    // shallow filesystem root (`/mnt`, `/home`, `/usr`, etc.) when the
    // Claude transcript dirname's dash↔slash decode round-trip failed
    // (live-verified — 10,005 jupyterlab files leaked from a different
    // user's $HOME via `/mnt`). The W1.2 c3 depth guard prevents future
    // leaks; this companion DELETE cleans up DBs that already accrued
    // the pollution before upgrading. The c1 SUBSTR backfill renames
    // those rows from `project='/mnt'` to `project='mnt'`, so the
    // detection predicate is doubly-anchored: `project=basename(root)`
    // AND `path LIKE '/<root>/%'`. A legitimate project at
    // `/repo/foo/mnt` named `mnt` is therefore NOT touched (its `path`
    // wouldn't start with `/mnt/`).
    //
    // Cascade — there is no FK cascade configured. Order: edges that
    // reference symbols of polluted files → those symbols → the files
    // themselves.
    //
    // Idempotent — second run finds no matching rows, deletes 0.
    const POLLUTION_PREDICATE: &str = "((project = 'mnt' AND path LIKE '/mnt/%') \
        OR (project = 'home' AND path LIKE '/home/%') \
        OR (project = 'usr' AND path LIKE '/usr/%') \
        OR (project = 'var' AND path LIKE '/var/%') \
        OR (project = 'tmp' AND path LIKE '/tmp/%') \
        OR (project = 'etc' AND path LIKE '/etc/%') \
        OR (project = 'opt' AND path LIKE '/opt/%') \
        OR (project = 'srv' AND path LIKE '/srv/%') \
        OR (project = 'proc' AND path LIKE '/proc/%') \
        OR (project = 'sys' AND path LIKE '/sys/%') \
        OR (project = 'boot' AND path LIKE '/boot/%') \
        OR (project = 'dev' AND path LIKE '/dev/%') \
        OR (project = 'run' AND path LIKE '/run/%'))";

    let polluted_paths_subquery = format!("SELECT path FROM code_file WHERE {POLLUTION_PREDICATE}");

    // Pre-release audit E-16: previously three sequential
    // `let _ = conn.execute(...)` calls. If pass 2 (code_symbol DELETE)
    // silently failed (e.g. POLLUTION_PREDICATE refactor breaks the
    // subquery), pass 3 still ran and deleted code_file rows, leaving
    // orphan symbols. `?` propagation + the fact that the three
    // statements are commutative (each DELETE is idempotent and
    // bounded by POLLUTION_PREDICATE) makes a partial-failure surface
    // operator-visible without a transaction wrapper.
    conn.execute(
        &format!(
            "DELETE FROM edge WHERE from_id IN (SELECT id FROM code_symbol WHERE file_path IN ({polluted_paths_subquery})) \
                                OR to_id IN (SELECT id FROM code_symbol WHERE file_path IN ({polluted_paths_subquery}))"
        ),
        [],
    )?;
    conn.execute(
        &format!("DELETE FROM code_symbol WHERE file_path IN ({polluted_paths_subquery})"),
        [],
    )?;
    conn.execute(
        &format!("DELETE FROM code_file WHERE {POLLUTION_PREDICATE}"),
        [],
    )?;

    // P3-4 pre-release Phase 10G (audit E-17): backfill NULL line_start
    // to a sentinel of 1. Pre-fix, `db::ops::list_symbols` silently
    // coerced NULL → 0 via `unwrap_or(0)`, hiding the data-quality
    // issue and exposing the illegal value `0` (lines are 1-indexed)
    // to LSP / blast-radius consumers. After this backfill,
    // list_symbols still defends with `unwrap_or(1)` but the row
    // values are now coherent. Idempotent — re-running on a DB with
    // no NULLs is a safe no-op. `?` propagation per
    // `feedback_sqlite_no_reverse_silent_migration_failure.md`.
    conn.execute(
        "UPDATE code_symbol SET line_start = 1 WHERE line_start IS NULL",
        [],
    )?;

    // v2.0: Composite indexes for scoped queries
    let _ = conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_memory_reality ON memory(reality_id, memory_type, status)",
        [],
    );
    let _ = conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_memory_user ON memory(user_id, memory_type, status)",
        [],
    );
    let _ = conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_memory_portability ON memory(portability)",
        [],
    );
    let _ = conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_session_reality ON session(reality_id, status)",
        [],
    );
    let _ = conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_session_user ON session(user_id, status)",
        [],
    );
    let _ = conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_session_parent ON session(parent_session_id)",
        [],
    );
    let _ = conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_identity_user ON identity(user_id, active)",
        [],
    );
    let _ = conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_entity_reality ON entity(reality_id)",
        [],
    );
    let _ = conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_edge_reality ON edge(reality_id, edge_type)",
        [],
    );
    let _ = conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_code_file_reality ON code_file(reality_id)",
        [],
    );

    // v2.0 fix: Missing indexes for cross-org query performance
    let _ = conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_session_org ON session(organization_id)",
        [],
    );
    let _ = conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_session_team ON session(team_id)",
        [],
    );
    let _ = conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_identity_org ON identity(organization_id)",
        [],
    );
    let _ = conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_entity_user ON entity(user_id)",
        [],
    );
    let _ = conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_team_member_user ON team_member(user_id)",
        [],
    );

    // v2.0 fix: Unique constraint on project(project_path) to prevent duplicate path rows.
    // Filtered: only applies to non-NULL project_path values.
    // ZR-C3 fw1 (LOW-1): surface errors with `?` instead of `let _ =`
    // per `feedback_sqlite_no_reverse_silent_migration_failure.md` so
    // a future SQLite-version regression in partial-index syntax
    // can't silently no-op the unique constraint.
    conn.execute("CREATE UNIQUE INDEX IF NOT EXISTS idx_project_path_unique ON project(project_path) WHERE project_path IS NOT NULL", [])?;

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
    conn.execute_batch(
        "
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
    ",
    )?;

    // Meeting participant: tracks each agent's response in a meeting
    conn.execute_batch(
        "
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
    ",
    )?;

    // FISP Consensus: voting columns on meeting table (idempotent ALTERs)
    let _ = conn.execute("ALTER TABLE meeting ADD COLUMN voting_options TEXT", []);
    let _ = conn.execute(
        "ALTER TABLE meeting ADD COLUMN threshold TEXT DEFAULT 'majority'",
        [],
    );
    let _ = conn.execute("ALTER TABLE meeting ADD COLUMN outcome TEXT", []);

    // FISP Consensus: meeting_vote table for structured voting
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS meeting_vote (
            meeting_id TEXT NOT NULL,
            session_id TEXT NOT NULL,
            choice TEXT NOT NULL,
            voted_at TEXT NOT NULL DEFAULT (datetime('now')),
            PRIMARY KEY (meeting_id, session_id)
        );
    ",
    )?;

    // Agent lifecycle: session gains template tracking, agent status, last activity.
    //
    // NOTE on the column naming: `session.agent_status` is the agent's WORK
    // state — canonical values mirror `AgentStatus` in
    // crates/core/src/types/team.rs ('idle' | 'thinking' | 'responding' |
    // 'in_meeting' | 'error' | 'retired'), but the SQL column is freeform
    // TEXT, so readers may also encounter legacy values like 'busy' /
    // 'active' / 'working' from older rows or external writers.
    //
    // This is distinct from `session.status` (lifecycle: 'active' | 'idle' |
    // 'ended' — see CREATE TABLE above for the full state machine). Both
    // columns can hold the value 'idle' on the same row with different
    // meanings: session.status='idle' means heartbeat-lapsed;
    // agent_status='idle' means awaiting work.
    let _ = conn.execute("ALTER TABLE session ADD COLUMN template_id TEXT", []);
    let _ = conn.execute(
        "ALTER TABLE session ADD COLUMN agent_status TEXT DEFAULT 'idle'",
        [],
    );
    let _ = conn.execute("ALTER TABLE session ADD COLUMN last_activity_at TEXT", []);

    // Session heartbeat: lightweight keep-alive separate from semantic last_activity_at
    let _ = conn.execute("ALTER TABLE session ADD COLUMN last_heartbeat_at TEXT", []);

    // Team enhancements: type, orchestrator, purpose
    let _ = conn.execute(
        "ALTER TABLE team ADD COLUMN team_type TEXT DEFAULT 'human'",
        [],
    );
    let _ = conn.execute(
        "ALTER TABLE team ADD COLUMN orchestrator_session_id TEXT",
        [],
    );
    let _ = conn.execute("ALTER TABLE team ADD COLUMN purpose TEXT", []);

    // Team member: support agent sessions (not just user_id)
    let _ = conn.execute("ALTER TABLE team_member ADD COLUMN session_id TEXT", []);

    // FISP: meeting_id for deterministic response matching
    let _ = conn.execute("ALTER TABLE session_message ADD COLUMN meeting_id TEXT", []);
    let _ = conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_msg_meeting ON session_message(meeting_id)",
        [],
    );

    // Agent team indexes
    let _ = conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_session_template ON session(template_id)",
        [],
    );
    let _ = conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_session_agent_status ON session(agent_status)",
        [],
    );
    let _ = conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_session_heartbeat ON session(status, last_heartbeat_at)",
        [],
    );
    // Pre-release audit E-12: working-set inheritance query in
    // sessions.rs:274-281 joins agent + project + status, ordered by
    // started_at desc. Pre-fix the planner picked one single-column
    // index then filtered the rest, devolving to an O(N) scan on
    // multi-week session histories. Composite covers the 3-column
    // WHERE plus the ORDER BY direction.
    let _ = conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_session_agent_project_status \
            ON session(agent, project, status, started_at DESC)",
        [],
    );

    // ── v2.5: Organization Hierarchy ──
    let _ = conn.execute("ALTER TABLE team ADD COLUMN parent_team_id TEXT", []);
    let _ = conn.execute("ALTER TABLE team ADD COLUMN description TEXT", []);
    let _ = conn.execute("ALTER TABLE session ADD COLUMN role TEXT", []);
    let _ = conn.execute("ALTER TABLE organization ADD COLUMN description TEXT", []);
    let _ = conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_team_parent ON team(parent_team_id)",
        [],
    );

    // Team topology: star, mesh, chain (default: mesh)
    let _ = conn.execute(
        "ALTER TABLE team ADD COLUMN topology TEXT DEFAULT 'mesh'",
        [],
    );

    // ── v2.2: Notification Engine ──

    conn.execute_batch(
        "
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
    ",
    )?;

    // ── Enterprise: RBAC audit columns on audit_log ──
    // These extend the existing audit_log table (v2.0 entity model) with
    // columns needed for HTTP RBAC audit logging. Existing rows are unaffected.
    let _ = conn.execute(
        "ALTER TABLE audit_log ADD COLUMN user_id TEXT NOT NULL DEFAULT 'local'",
        [],
    );
    let _ = conn.execute(
        "ALTER TABLE audit_log ADD COLUMN email TEXT NOT NULL DEFAULT ''",
        [],
    );
    let _ = conn.execute(
        "ALTER TABLE audit_log ADD COLUMN role TEXT NOT NULL DEFAULT ''",
        [],
    );
    let _ = conn.execute(
        "ALTER TABLE audit_log ADD COLUMN request_type TEXT NOT NULL DEFAULT ''",
        [],
    );
    let _ = conn.execute(
        "ALTER TABLE audit_log ADD COLUMN request_summary TEXT NOT NULL DEFAULT ''",
        [],
    );
    let _ = conn.execute(
        "ALTER TABLE audit_log ADD COLUMN source TEXT NOT NULL DEFAULT 'socket'",
        [],
    );
    let _ = conn.execute(
        "ALTER TABLE audit_log ADD COLUMN source_ip TEXT NOT NULL DEFAULT ''",
        [],
    );
    let _ = conn.execute(
        "ALTER TABLE audit_log ADD COLUMN response_status TEXT NOT NULL DEFAULT 'ok'",
        [],
    );
    let _ = conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_audit_log_timestamp ON audit_log(timestamp)",
        [],
    );
    let _ = conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_audit_log_user ON audit_log(user_id)",
        [],
    );

    // ── Proactive Context (Prajna) ──

    conn.execute_batch("
        CREATE TABLE IF NOT EXISTS context_effectiveness (
            id TEXT PRIMARY KEY,
            session_id TEXT NOT NULL,
            hook_event TEXT NOT NULL,
            context_type TEXT NOT NULL,
            content_summary TEXT NOT NULL,
            injected_at TEXT NOT NULL DEFAULT (datetime('now')),
            acknowledged INTEGER NOT NULL DEFAULT 0,
            outcome TEXT,
            chars_injected INTEGER NOT NULL DEFAULT 0
        );
        CREATE INDEX IF NOT EXISTS idx_ce_session ON context_effectiveness(session_id);
        CREATE INDEX IF NOT EXISTS idx_ce_hook_type ON context_effectiveness(hook_event, context_type);
    ")?;

    // Migration: add chars_injected to existing context_effectiveness tables
    let _ = conn.execute(
        "ALTER TABLE context_effectiveness ADD COLUMN chars_injected INTEGER NOT NULL DEFAULT 0",
        [],
    );

    // Append-only enforcement: block UPDATE and DELETE on audit_log.
    //
    // Pre-release audit E-6: pre-fix used `let _ = conn.execute_batch(...)`,
    // which per `feedback_sqlite_no_reverse_silent_migration_failure.md`
    // swallows the error if the CREATE TRIGGER ever fails (trigger-name
    // collision after a future rename, SQLite-version regression on the
    // BEFORE UPDATE/DELETE syntax, etc.). The daemon would start with
    // an audit_log table that is silently mutable — tampering protection
    // would vanish without operator detection. Surface the error with `?`.
    conn.execute_batch(
        "CREATE TRIGGER IF NOT EXISTS audit_log_no_update
         BEFORE UPDATE ON audit_log
         BEGIN
             SELECT RAISE(ABORT, 'audit_log is append-only: UPDATE not allowed');
         END;
         CREATE TRIGGER IF NOT EXISTS audit_log_no_delete
         BEFORE DELETE ON audit_log
         BEGIN
             SELECT RAISE(ABORT, 'audit_log is append-only: DELETE not allowed');
         END;",
    )?;

    // ── v2.6: Memory Supersede + Structured Metadata ──
    let _ = conn.execute("ALTER TABLE memory ADD COLUMN superseded_by TEXT", []);
    let _ = conn.execute("ALTER TABLE memory ADD COLUMN metadata TEXT", []);

    // ── Phase 2A-4a: Valence Flipping ──
    // Phase 2A-4a: valence_flipped_at marks preferences that have been superseded
    // via Request::FlipPreference (as opposed to plain Supersede). Used by
    // CompileContext's <preferences-flipped> section and the ListFlipped endpoint.
    let _ = conn.execute("ALTER TABLE memory ADD COLUMN valence_flipped_at TEXT", []);
    let _ = conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_memory_valence_flipped_at
             ON memory(valence_flipped_at)
             WHERE valence_flipped_at IS NOT NULL",
        [],
    );

    // ── Phase 2A-4b: Recency-weighted Preference Decay ───────────────────────
    // Adds `reaffirmed_at` for user/agent-controlled freshness anchor.
    // Used by `recency_factor` (recall.rs ranker) and `decay_memories` (fader).
    // NULL means the preference has never been reaffirmed; falls back to created_at.
    let _ = conn.execute("ALTER TABLE memory ADD COLUMN reaffirmed_at TEXT", []);
    // No partial index — recall doesn't filter on reaffirmed_at; only ORDER BY
    // COALESCE(reaffirmed_at, created_at) which can't use a single-column index.

    // ── v2.7: Memory Self-Healing ──
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS healing_log (
            id TEXT PRIMARY KEY,
            action TEXT NOT NULL,
            old_memory_id TEXT NOT NULL,
            new_memory_id TEXT,
            similarity_score REAL,
            overlap_score REAL,
            reason TEXT NOT NULL,
            created_at TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_healing_log_action ON healing_log(action);
        CREATE INDEX IF NOT EXISTS idx_healing_log_created ON healing_log(created_at);
    ",
    )?;

    // ── Skills Registry ──
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS skill_registry (
            id TEXT PRIMARY KEY,
            name TEXT NOT NULL,
            description TEXT NOT NULL DEFAULT '',
            category TEXT NOT NULL DEFAULT 'general',
            file_path TEXT NOT NULL,
            installed_for_project TEXT,
            indexed_at TEXT NOT NULL DEFAULT (datetime('now')),
            UNIQUE(name, category)
        );
        CREATE INDEX IF NOT EXISTS idx_skill_registry_category ON skill_registry(category);
    ",
    )?;

    // FTS5 virtual table for skill search
    // Use IF NOT EXISTS to be idempotent; FTS5 tables cannot use CREATE TABLE IF NOT EXISTS
    // directly with content= sync, so we check existence first.
    //
    // Pre-release audit E-19: previously `unwrap_or(false)` masked
    // real probe errors (e.g. WAL busy-lock at startup) as "doesn't
    // exist", which then failed louder when the CREATE collided with
    // an actual existing table. `?` lets the real error surface so
    // operators can see the lock contention or schema corruption
    // directly.
    let fts_exists: bool = conn.query_row(
        "SELECT COUNT(*) > 0 FROM sqlite_master WHERE type='table' AND name='skill_registry_fts'",
        [],
        |row| row.get(0),
    )?;
    if !fts_exists {
        conn.execute_batch(
            "CREATE VIRTUAL TABLE skill_registry_fts USING fts5(name, description, content=skill_registry, content_rowid=rowid);"
        )?;

        // Triggers to keep FTS in sync with the skill_registry table
        conn.execute_batch("
            CREATE TRIGGER IF NOT EXISTS skill_registry_fts_insert AFTER INSERT ON skill_registry BEGIN
                INSERT INTO skill_registry_fts(rowid, name, description) VALUES (new.rowid, new.name, new.description);
            END;

            CREATE TRIGGER IF NOT EXISTS skill_registry_fts_delete AFTER DELETE ON skill_registry BEGIN
                INSERT INTO skill_registry_fts(skill_registry_fts, rowid, name, description) VALUES ('delete', old.rowid, old.name, old.description);
            END;

            CREATE TRIGGER IF NOT EXISTS skill_registry_fts_update AFTER UPDATE ON skill_registry BEGIN
                INSERT INTO skill_registry_fts(skill_registry_fts, rowid, name, description) VALUES ('delete', old.rowid, old.name, old.description);
                INSERT INTO skill_registry_fts(rowid, name, description) VALUES (new.rowid, new.name, new.description);
            END;
        ")?;
    }

    // ── Smart Model Router: routing stats ──
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS routing_stats (
            tier TEXT NOT NULL,
            provider TEXT NOT NULL,
            success INTEGER NOT NULL DEFAULT 1,
            tokens_saved INTEGER NOT NULL DEFAULT 0,
            created_at TEXT NOT NULL DEFAULT (datetime('now')),
            quality_score REAL
        );
        CREATE INDEX IF NOT EXISTS idx_routing_stats_tier ON routing_stats(tier);
        CREATE INDEX IF NOT EXISTS idx_routing_stats_created ON routing_stats(created_at);
    ",
    )?;

    // Quality tracking for smart router quality guard
    let _ = conn.execute(
        "ALTER TABLE routing_stats ADD COLUMN quality_score REAL",
        [],
    );

    // ── v2.8: Paperclip-inspired features ──

    // Goal ancestry: traces team/meeting work to a project mission
    let _ = conn.execute("ALTER TABLE team ADD COLUMN goal TEXT", []);
    let _ = conn.execute("ALTER TABLE meeting ADD COLUMN goal TEXT", []);

    // Per-agent budget enforcement
    let _ = conn.execute(
        "ALTER TABLE agent_template ADD COLUMN budget_limit REAL",
        [],
    );
    let _ = conn.execute(
        "ALTER TABLE session ADD COLUMN budget_spent REAL DEFAULT 0",
        [],
    );

    // Seed default agent templates (idempotent)
    if let Err(e) = seed_default_templates(conn) {
        eprintln!("[schema] warning: failed to seed default agent templates: {e}");
    }

    // Seed pre-built team templates (idempotent)
    if let Err(e) = crate::teams::seed_team_templates(conn) {
        eprintln!("[schema] warning: failed to seed team templates: {e}");
    }

    // Seed default agent templates (idempotent) — required for web app spawn_agent
    if let Err(e) = crate::teams::seed_agent_templates(conn) {
        eprintln!("[schema] warning: failed to seed agent templates: {e}");
    }

    // ── Migration: remove FK constraints from edge table ──
    // Legacy databases have FOREIGN KEY (from_id/to_id) REFERENCES memory(id)
    // on the edge table, but import/call/affects edges use non-memory IDs (file: prefixed).
    // These FKs block all edge creation silently. Recreate without FKs if present.
    let has_fk: bool = conn
        .query_row(
            "SELECT COUNT(*) > 0 FROM pragma_foreign_key_list('edge')",
            [],
            |row| row.get(0),
        )
        .unwrap_or(false);
    if has_fk {
        eprintln!("[schema] migrating edge table: removing legacy FK constraints");
        // Pre-release audit E-13: previously `let _ = conn.execute_batch(...)`
        // which silently swallowed migration failures. Per
        // feedback_sqlite_no_reverse_silent_migration_failure.md the trap is
        // that any error in CREATE TABLE / INSERT / DROP / ALTER /
        // CREATE INDEX leaves edge_migrated orphaned with the live `edge`
        // table either deleted or stuck mid-rename. Use `?` propagation so
        // operators see the failure at startup rather than discovering it
        // weeks later via missing edges.
        conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS edge_migrated (
                id TEXT PRIMARY KEY,
                from_id TEXT NOT NULL,
                to_id TEXT NOT NULL,
                edge_type TEXT NOT NULL,
                properties TEXT NOT NULL DEFAULT '{}',
                created_at TEXT NOT NULL,
                valid_from TEXT NOT NULL,
                valid_until TEXT,
                reality_id TEXT
            );
            INSERT OR IGNORE INTO edge_migrated SELECT * FROM edge;
            DROP TABLE edge;
            ALTER TABLE edge_migrated RENAME TO edge;
            CREATE INDEX IF NOT EXISTS idx_edge_from ON edge(from_id);
            CREATE INDEX IF NOT EXISTS idx_edge_to ON edge(to_id);
            CREATE INDEX IF NOT EXISTS idx_edge_type ON edge(edge_type);
        ",
        )?;
    }

    // ── Migration: dedup edges THEN create UNIQUE index (ISS-D6) ──
    // MUST dedup before index creation — CREATE UNIQUE INDEX fails on duplicate data.
    // M1 fix: only dedup if UNIQUE index doesn't already exist (avoids full-table scan on every startup).
    let edge_idx_exists: bool = conn
        .query_row(
            "SELECT COUNT(*) > 0 FROM sqlite_master WHERE type='index' AND name='idx_edge_unique'",
            [],
            |r| r.get(0),
        )
        .unwrap_or(false);
    if !edge_idx_exists {
        let edge_deduped: usize = conn
            .execute(
                "DELETE FROM edge WHERE rowid NOT IN (
                SELECT MIN(rowid) FROM edge GROUP BY from_id, to_id, edge_type
            )",
                [],
            )
            .unwrap_or_else(|e| {
                eprintln!("[schema] edge dedup failed: {e}");
                0
            });
        if edge_deduped > 0 {
            eprintln!("[schema] deduplicated {edge_deduped} duplicate edge rows");
        }
    }
    conn.execute_batch(
        "CREATE UNIQUE INDEX IF NOT EXISTS idx_edge_unique ON edge(from_id, to_id, edge_type);",
    )?;

    // ── Migration: dedup teams THEN create UNIQUE index (ISS-D6) ──
    let team_idx_exists: bool = conn.query_row(
        "SELECT COUNT(*) > 0 FROM sqlite_master WHERE type='index' AND name='idx_team_name_org'",
        [], |r| r.get(0),
    ).unwrap_or(false);
    if !team_idx_exists {
        let team_deduped: usize = conn
            .execute(
                "DELETE FROM team WHERE id NOT IN (
                SELECT MIN(id) FROM team GROUP BY name, organization_id
            )",
                [],
            )
            .unwrap_or_else(|e| {
                eprintln!("[schema] team dedup failed: {e}");
                0
            });
        if team_deduped > 0 {
            eprintln!("[schema] deduplicated {team_deduped} duplicate team rows");
        }
    }
    conn.execute_batch(
        "CREATE UNIQUE INDEX IF NOT EXISTS idx_team_name_org ON team(name, organization_id);",
    )?;

    // ── Phase 2A-4c1: Tool-Use Recording ──
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS session_tool_call (
            id                    TEXT PRIMARY KEY,
            session_id            TEXT NOT NULL,
            agent                 TEXT NOT NULL,
            tool_name             TEXT NOT NULL,
            tool_args             TEXT NOT NULL,
            tool_result_summary   TEXT NOT NULL,
            success               INTEGER NOT NULL,
            user_correction_flag  INTEGER NOT NULL DEFAULT 0,
            organization_id       TEXT NOT NULL DEFAULT 'default',
            created_at            TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_session_tool_session
            ON session_tool_call (session_id, created_at);
        CREATE INDEX IF NOT EXISTS idx_session_tool_name_agent
            ON session_tool_call (agent, tool_name);
        CREATE INDEX IF NOT EXISTS idx_session_tool_org_session_created
            ON session_tool_call (organization_id, session_id, created_at DESC);
    ",
    )?;

    // ── Phase 2A-4d.2.1 #4 (W7): kpi_events.run_id for the HUD 24h rollup ──
    //
    // The HUD's 24h rollup (`events.rs::build_hud_state`) does
    // `COUNT(DISTINCT json_extract(metadata_json, '$.run_id'))` against
    // 24 hours of `phase_completed` rows. With only the existing
    // `idx_kpi_events_timestamp`, the planner can range-scan
    // timestamp-matching rows but must still parse JSON for every row
    // to compute the DISTINCT — bounded today by the kpi_events
    // retention reaper (so the table never grows unbounded), but slow
    // enough to matter once `kpi_events_retention_days` is set high
    // (>14d) on a high-throughput daemon.
    //
    // Fix: promote `run_id` to a real TEXT column with its own index,
    // backfill from existing rows once, populate via writers going
    // forward. The HUD query then becomes
    // `COUNT(DISTINCT run_id)` against the indexed column.
    let _ = conn.execute("ALTER TABLE kpi_events ADD COLUMN run_id TEXT", []);
    // Backfill: once-per-DB UPDATE that pulls run_id from metadata_json
    // for any row with a NULL column. Idempotent (a second run sees no
    // NULL rows). Bounded by retention; on a fresh DB this is a no-op.
    let _ = conn.execute(
        "UPDATE kpi_events
         SET run_id = json_extract(metadata_json, '$.run_id')
         WHERE run_id IS NULL
           AND json_extract(metadata_json, '$.run_id') IS NOT NULL",
        [],
    );
    let _ = conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_kpi_events_run_id_timestamp \
         ON kpi_events(run_id, timestamp)",
        [],
    );

    // ── Phase P3-3.11 W29: project sentinel '_global_' ──
    //
    // Historic bug: extractor and write paths could leave memory.project as
    // NULL or empty. Several INSERT sites omit the project column entirely
    // (e.g. crates/daemon/src/db/vec.rs:317-322 and
    // crates/daemon/src/teams.rs:1144), producing rows with NULL project.
    // These NULL/empty-project memories were admitted into every
    // project-scoped recall query via the historic soft-scope clause
    // `m.project IS NULL OR m.project = ''`, causing the F15/F17
    // cross-project content leak observed in the P3-3.8 dogfood.
    //
    // Fix (this migration): backfill all existing NULL/empty `project` rows
    // to the explicit '_global_' sentinel. Future writes are gated by the
    // application-layer `project_or_global()` helper (see W29 commit 2 in
    // crates/daemon/src/db/ops.rs) — every memory-INSERT call site routes
    // its `project` parameter through that helper, which substitutes the
    // sentinel for `None` / `Some("")`. A schema-level AFTER INSERT trigger
    // was considered for defence in depth but is incompatible with the
    // `memory_fts` external-content FTS5 index: the trigger's nested UPDATE
    // perturbs FTS5's invariant on the just-inserted rowid and corrupts the
    // index with `database disk image is malformed (11)`. Application-layer
    // enforcement is sufficient because every memory write goes through
    // Rust code in this crate — there is no out-of-band SQL writer.
    //
    // Recall semantics (see crates/daemon/src/db/ops.rs::recall_bm25_project_org_flipped):
    //   - `Request::Recall { project: Some("forge"), include_globals: false }`
    //     is STRICT — only `m.project = 'forge'` rows match.
    //   - `Request::Recall { project: Some("forge"), include_globals: true }`
    //     matches `m.project IN ('forge', '_global_')`.
    //   - `Request::Recall { project: None, ... }` is unscoped (returns all).
    //
    // Idempotent: re-running on an already-migrated DB is a no-op (no rows
    // satisfy the backfill WHERE clause).
    //
    // FTS5 sync defence: the backfill UPDATE fires the `memory_fts_update`
    // trigger (defined at the top of this fn), which issues FTS5's 'delete'
    // command for every updated rowid. On a database where some rows in
    // `memory` are not mirrored in `memory_fts` (e.g., historical FTS sync
    // drift, or legacy rows that pre-date the FTS triggers), the 'delete'
    // corrupts the FTS index with `database disk image is malformed (11)`.
    // We pre-rebuild memory_fts only when the backfill is actually going to
    // run, so the rebuild cost is paid at most once per database (on the
    // upgrade that introduces this migration) and is a no-op on every
    // subsequent daemon startup once the table converges.
    let needs_backfill: bool = conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM memory WHERE project IS NULL OR project = '')",
            [],
            |r| r.get::<_, i64>(0),
        )
        .map(|n| n == 1)
        .unwrap_or(false);
    if needs_backfill {
        let _ = conn.execute("INSERT INTO memory_fts(memory_fts) VALUES('rebuild')", []);
        let _ = conn.execute(
            "UPDATE memory SET project = '_global_' \
             WHERE project IS NULL OR project = ''",
            [],
        );
    }

    // P3-3.11 W30: identity.project sentinel backfill (defensive).
    //
    // The ALTER TABLE above adds the column with NOT NULL DEFAULT '_global_',
    // so SQLite backfills existing rows automatically. This UPDATE is a
    // defence in depth for the rare case where a hypothetical prerelease
    // build added the column without the DEFAULT clause, or where a row was
    // explicitly set to '' by an out-of-band SQL writer. No FTS5 dependency
    // here — the identity table has no shadow virtual table.
    //
    // Identity scoping semantics (see crates/daemon/src/db/manas.rs):
    //   - `list_identity_for_project(agent, "forge", include_globals=false)`
    //     is STRICT — only `project = 'forge'` rows match.
    //   - `list_identity_for_project(agent, "forge", include_globals=true)`
    //     matches `project IN ('forge', '_global_')`.
    //   - `list_identity(agent)` returns all rows for the agent (legacy
    //     unscoped path, still used by sync export).
    let _ = conn.execute(
        "UPDATE identity SET project = '_global_' \
         WHERE project IS NULL OR project = ''",
        [],
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
    fn p3_4_w1_25_composite_project_path_index_present_and_planner_uses_it() {
        // W1.3 LOW-6 contract: code_file must carry a composite
        // (project, path) index so a project-filtered scan also
        // satisfies the JOIN to code_symbol via path. We pin both:
        //   1. the index exists in sqlite_master with the exact name
        //      and column list, AND
        //   2. EXPLAIN QUERY PLAN for a project-filtered query reports
        //      that the planner uses idx_code_file_project_path
        //      (instead of falling back to idx_code_file_path or a
        //      table scan).
        crate::db::vec::init_sqlite_vec();
        let conn = Connection::open_in_memory().unwrap();
        create_schema(&conn).unwrap();

        // 1. Index exists with the right shape.
        let sql_def: String = conn
            .query_row(
                "SELECT sql FROM sqlite_master WHERE type='index' AND name='idx_code_file_project_path'",
                [],
                |row| row.get(0),
            )
            .expect("idx_code_file_project_path must exist after create_schema");
        assert!(
            sql_def.contains("(project, path)"),
            "composite index must be on (project, path); got: {sql_def}"
        );

        // 2. Planner picks the composite index for a project-filtered
        //    query. EXPLAIN QUERY PLAN's "detail" column names the
        //    chosen index. Use a row-yielding query (we can't use
        //    `query_row` here because EXPLAIN may produce multiple
        //    rows for joins; just collect them and look for the
        //    index name).
        let mut stmt = conn
            .prepare("EXPLAIN QUERY PLAN SELECT path FROM code_file WHERE project = 'forge'")
            .expect("prepare EXPLAIN");
        let plan: Vec<String> = stmt
            .query_map([], |row| row.get::<_, String>(3))
            .expect("query plan rows")
            .flatten()
            .collect();
        let plan_text = plan.join(" | ");
        assert!(
            plan_text.contains("idx_code_file_project_path"),
            "planner must use idx_code_file_project_path for `WHERE project = ?`; got plan: {plan_text}"
        );
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
        let valence: String = conn
            .query_row("SELECT valence FROM memory WHERE id = 'v1'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(valence, "negative");
        let intensity: f64 = conn
            .query_row("SELECT intensity FROM memory WHERE id = 'v1'", [], |r| {
                r.get(0)
            })
            .unwrap();
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
        let hlc: String = conn
            .query_row(
                "SELECT hlc_timestamp FROM memory WHERE id = 'h1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(hlc.contains("abc12345"));
        let node: String = conn
            .query_row("SELECT node_id FROM memory WHERE id = 'h1'", [], |r| {
                r.get(0)
            })
            .unwrap();
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
        let msg: String = conn
            .query_row("SELECT message FROM diagnostic WHERE id = 'd1'", [], |r| {
                r.get(0)
            })
            .unwrap();
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
            "project",
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
            assert_eq!(count, 1, "v2 table '{table_name}' should exist");
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
        )
        .unwrap();

        // Verify entity scoping columns
        conn.execute(
            "UPDATE entity SET reality_id = 'r1', user_id = 'u1' WHERE 0",
            [],
        )
        .unwrap();

        // Verify edge scoping columns
        conn.execute("UPDATE edge SET reality_id = 'r1' WHERE 0", [])
            .unwrap();

        // Verify code_file scoping columns
        conn.execute("UPDATE code_file SET reality_id = 'r1' WHERE 0", [])
            .unwrap();
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

        let action: String = conn
            .query_row("SELECT action FROM audit_log WHERE id = 'a1'", [], |r| {
                r.get(0)
            })
            .unwrap();
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
            assert_eq!(count, 1, "manas table '{table_name}' should exist");
        }
    }

    #[test]
    fn test_healing_log_table_exists() {
        crate::db::vec::init_sqlite_vec();
        let conn = Connection::open_in_memory().unwrap();
        create_schema(&conn).unwrap();
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='healing_log'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1, "healing_log table should exist");
    }

    #[test]
    fn test_default_agent_templates_seeded() {
        crate::db::vec::init_sqlite_vec();
        let conn = Connection::open_in_memory().unwrap();
        create_schema(&conn).unwrap();

        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM agent_template", [], |row| row.get(0))
            .unwrap();
        assert_eq!(
            count, 18,
            "should seed 18 agent templates (3 base + 15 role)"
        );

        // Verify specific templates exist by name
        let claude: String = conn
            .query_row(
                "SELECT name FROM agent_template WHERE name = 'claude-code'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(claude, "claude-code");

        let codex: String = conn
            .query_row(
                "SELECT name FROM agent_template WHERE name = 'codex-cli'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(codex, "codex-cli");
    }

    #[test]
    fn test_agent_template_seed_idempotent() {
        crate::db::vec::init_sqlite_vec();
        let conn = Connection::open_in_memory().unwrap();
        create_schema(&conn).unwrap();

        // Call seed again — should not duplicate
        seed_default_templates(&conn).unwrap();

        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM agent_template", [], |row| row.get(0))
            .unwrap();
        assert_eq!(
            count, 18,
            "should still be 18 after double-seed (INSERT OR IGNORE)"
        );
    }

    #[test]
    fn test_schema_survives_duplicate_edges() {
        // ISS-D6: create_schema must succeed on a DB with pre-existing duplicate edges.
        // Simulates an existing user's DB that accumulated duplicates before the UNIQUE index.
        crate::db::vec::init_sqlite_vec();
        let conn = Connection::open_in_memory().unwrap();

        // First pass: create schema (fresh DB, no duplicates — succeeds)
        create_schema(&conn).unwrap();

        // Drop the unique index to simulate a pre-Session-13 DB
        conn.execute("DROP INDEX IF EXISTS idx_edge_unique", [])
            .unwrap();

        // Insert duplicate edges (same from_id, to_id, edge_type)
        let now = forge_core::time::now_iso();
        for i in 0..3 {
            conn.execute(
                "INSERT INTO edge (id, from_id, to_id, edge_type, properties, created_at, valid_from)
                 VALUES (?1, 'file:src/main.rs', 'sym:main', 'calls', '{}', ?2, ?2)",
                rusqlite::params![format!("dup-edge-{i}"), &now],
            ).unwrap();
        }

        // Verify duplicates exist
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM edge WHERE from_id = 'file:src/main.rs' AND to_id = 'sym:main'",
            [], |r| r.get(0),
        ).unwrap();
        assert_eq!(count, 3, "should have 3 duplicate edges");

        // Second pass: create_schema must NOT crash — dedup runs before unique index
        create_schema(&conn).unwrap();

        // After dedup, only 1 should remain
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM edge WHERE from_id = 'file:src/main.rs' AND to_id = 'sym:main'",
            [], |r| r.get(0),
        ).unwrap();
        assert_eq!(
            count, 1,
            "dedup should keep only 1 edge per (from_id, to_id, edge_type)"
        );

        // Unique index should exist
        let has_idx: bool = conn.query_row(
            "SELECT COUNT(*) > 0 FROM sqlite_master WHERE type='index' AND name='idx_edge_unique'",
            [], |r| r.get(0),
        ).unwrap();
        assert!(has_idx, "unique index should exist after migration");
    }

    #[test]
    fn test_schema_survives_duplicate_teams() {
        // ISS-D6: create_schema must succeed on a DB with pre-existing duplicate teams.
        crate::db::vec::init_sqlite_vec();
        let conn = Connection::open_in_memory().unwrap();
        create_schema(&conn).unwrap();

        // Drop unique index to simulate pre-Session-13 DB
        conn.execute("DROP INDEX IF EXISTS idx_team_name_org", [])
            .unwrap();

        // Insert duplicate teams
        let now = forge_core::time::now_iso();
        for i in 0..3 {
            conn.execute(
                "INSERT INTO team (id, name, organization_id, created_by, status, created_at)
                 VALUES (?1, 'uat-team', 'default', 'system', 'active', ?2)",
                rusqlite::params![format!("dup-team-{i}"), &now],
            )
            .unwrap();
        }

        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM team WHERE name = 'uat-team'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 3, "should have 3 duplicate teams");

        // Re-run create_schema — must not crash
        create_schema(&conn).unwrap();

        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM team WHERE name = 'uat-team'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 1, "dedup should keep only 1 team per (name, org)");
    }

    #[test]
    fn test_memory_schema_has_valence_flipped_at_column() {
        crate::db::vec::init_sqlite_vec();
        let conn = Connection::open_in_memory().unwrap();
        create_schema(&conn).unwrap();

        let cols: Vec<String> = conn
            .prepare("PRAGMA table_info(memory)")
            .unwrap()
            .query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();

        assert!(
            cols.contains(&"valence_flipped_at".to_string()),
            "memory table missing valence_flipped_at column; columns: {cols:?}"
        );
    }

    #[test]
    fn test_memory_schema_has_valence_flipped_at_partial_index() {
        crate::db::vec::init_sqlite_vec();
        let conn = Connection::open_in_memory().unwrap();
        create_schema(&conn).unwrap();

        let indexes: Vec<String> = conn
            .prepare("SELECT name FROM sqlite_master WHERE type='index' AND tbl_name='memory'")
            .unwrap()
            .query_map([], |row| row.get::<_, String>(0))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();

        assert!(
            indexes.contains(&"idx_memory_valence_flipped_at".to_string()),
            "memory table missing idx_memory_valence_flipped_at; indexes: {indexes:?}"
        );
    }

    #[test]
    fn test_valence_flipped_at_rollback_recipe_works() {
        crate::db::vec::init_sqlite_vec();
        let conn = Connection::open_in_memory().unwrap();
        create_schema(&conn).unwrap();

        // Insert a row with valence_flipped_at set
        conn.execute(
            "INSERT INTO memory (id, memory_type, title, content, confidence, status, project, tags, created_at, accessed_at, valence, intensity, valence_flipped_at, superseded_by)
             VALUES ('01F', 'preference', 't', 'c', 0.9, 'superseded', NULL, '[]', '2026-04-17 00:00:00', '2026-04-17 00:00:00', 'positive', 0.5, '2026-04-17 14:00:00', '01N')",
            [],
        ).unwrap();

        // Execute the documented rollback recipe.
        conn.execute("DROP INDEX IF EXISTS idx_memory_valence_flipped_at", [])
            .unwrap();
        conn.execute("ALTER TABLE memory DROP COLUMN valence_flipped_at", [])
            .unwrap();

        // Verify remaining queries still work (column-less SELECT)
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM memory WHERE id = '01F'", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(count, 1);

        // Verify the column is gone
        let cols: Vec<String> = conn
            .prepare("PRAGMA table_info(memory)")
            .unwrap()
            .query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();
        assert!(
            !cols.contains(&"valence_flipped_at".to_string()),
            "valence_flipped_at column should be gone after rollback; columns: {cols:?}"
        );

        // Verify the index is also gone
        let indexes: Vec<String> = conn
            .prepare("SELECT name FROM sqlite_master WHERE type='index' AND tbl_name='memory'")
            .unwrap()
            .query_map([], |row| row.get::<_, String>(0))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();
        assert!(
            !indexes.contains(&"idx_memory_valence_flipped_at".to_string()),
            "idx_memory_valence_flipped_at should be gone after rollback; indexes: {indexes:?}"
        );
    }

    #[test]
    fn forge_db_schema_creates_reaffirmed_at_column() {
        use rusqlite::Connection;
        crate::db::vec::init_sqlite_vec();
        let conn = Connection::open_in_memory().unwrap();
        create_schema(&conn).unwrap();

        let cols: Vec<String> = conn
            .prepare("PRAGMA table_info(memory)")
            .unwrap()
            .query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .filter_map(|r| r.ok())
            .collect();

        assert!(
            cols.iter().any(|c| c == "reaffirmed_at"),
            "memory table missing reaffirmed_at column; got: {:?}",
            cols
        );
    }

    #[test]
    fn forge_db_schema_migrates_existing_memory_table() {
        use rusqlite::Connection;
        crate::db::vec::init_sqlite_vec();
        let conn = Connection::open_in_memory().unwrap();

        // Simulate pre-2A-4b memory table (without reaffirmed_at column).
        // Includes columns required by create_schema index/trigger DDL (project, tags,
        // confidence, organization_id) so that the IF NOT EXISTS index creation succeeds.
        // All columns that were added via ALTER TABLE after the base schema are omitted
        // (e.g. valence_flipped_at, superseded_by, metadata) to prove the migration path.
        conn.execute_batch(
            "CREATE TABLE memory (
                id TEXT PRIMARY KEY,
                memory_type TEXT NOT NULL,
                title TEXT NOT NULL,
                content TEXT NOT NULL,
                confidence REAL NOT NULL DEFAULT 0.8,
                status TEXT NOT NULL DEFAULT 'active',
                project TEXT,
                tags TEXT NOT NULL DEFAULT '[]',
                created_at TEXT NOT NULL,
                accessed_at TEXT NOT NULL,
                organization_id TEXT NOT NULL DEFAULT 'default',
                valence TEXT NOT NULL DEFAULT 'neutral',
                intensity REAL NOT NULL DEFAULT 0.5,
                alternatives TEXT NOT NULL DEFAULT '[]',
                participants TEXT NOT NULL DEFAULT '[]'
            );",
        )
        .unwrap();

        // Insert a pre-existing row
        conn.execute(
            "INSERT INTO memory (id, memory_type, title, content, created_at, accessed_at)
             VALUES ('legacy-1', 'preference', 'old-pref', 'content', '2026-01-01 00:00:00', '2026-01-01 00:00:00')",
            [],
        )
        .unwrap();

        // Run create_schema to apply the 2A-4b ALTER TABLE
        create_schema(&conn).unwrap();

        // Assert: column was added
        let cols: Vec<String> = conn
            .prepare("PRAGMA table_info(memory)")
            .unwrap()
            .query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .filter_map(|r| r.ok())
            .collect();
        assert!(
            cols.iter().any(|c| c == "reaffirmed_at"),
            "column should exist after migration; got: {:?}",
            cols
        );

        // Assert: existing row preserved with NULL reaffirmed_at
        let row_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM memory WHERE id = 'legacy-1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(row_count, 1, "existing row should be preserved");

        let reaffirmed: Option<String> = conn
            .query_row(
                "SELECT reaffirmed_at FROM memory WHERE id = 'legacy-1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            reaffirmed, None,
            "existing row should have NULL reaffirmed_at after migration"
        );
    }

    #[test]
    fn test_reaffirmed_at_rollback_recipe_works() {
        // T15: rollback recipe for the reaffirmed_at column added in T1.
        // Verifies: forward migration adds column; INSERT with reaffirmed_at;
        // rollback (ALTER TABLE DROP COLUMN) removes column cleanly;
        // other column data intact; queries not referencing the column still work.
        crate::db::vec::init_sqlite_vec();
        let conn = Connection::open_in_memory().unwrap();
        create_schema(&conn).unwrap();

        // 1. Forward migration: verify column exists after create_schema.
        let cols: Vec<String> = conn
            .prepare("PRAGMA table_info(memory)")
            .unwrap()
            .query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();
        assert!(
            cols.contains(&"reaffirmed_at".to_string()),
            "reaffirmed_at should exist after create_schema; columns: {cols:?}"
        );

        // 2. INSERT a row using the reaffirmed_at column.
        conn.execute(
            "INSERT INTO memory (id, memory_type, title, content, confidence, status, project, tags, created_at, accessed_at, valence, intensity, reaffirmed_at)
             VALUES ('t15-rollback-01', 'preference', 'pref-title', 'pref-content', 0.85, 'active', NULL, '[]', '2026-04-18 00:00:00', '2026-04-18 00:00:00', 'positive', 0.7, '2026-04-18 12:00:00')",
            [],
        ).unwrap();

        // Confirm readback.
        let reaffirmed: Option<String> = conn
            .query_row(
                "SELECT reaffirmed_at FROM memory WHERE id = 't15-rollback-01'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            reaffirmed,
            Some("2026-04-18 12:00:00".to_string()),
            "reaffirmed_at should hold the written value before rollback"
        );

        // 3. Execute rollback recipe: DROP COLUMN (SQLite 3.35+ / rusqlite bundled 3.46+).
        conn.execute("ALTER TABLE memory DROP COLUMN reaffirmed_at", [])
            .unwrap();

        // 4. Verify column is gone.
        let cols_after: Vec<String> = conn
            .prepare("PRAGMA table_info(memory)")
            .unwrap()
            .query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();
        assert!(
            !cols_after.contains(&"reaffirmed_at".to_string()),
            "reaffirmed_at should be gone after rollback; columns: {cols_after:?}"
        );

        // 5. Other column data intact.
        let (title, conf): (String, f64) = conn
            .query_row(
                "SELECT title, confidence FROM memory WHERE id = 't15-rollback-01'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(title, "pref-title");
        assert!(
            (conf - 0.85).abs() < 1e-6,
            "confidence should be 0.85; got {conf}"
        );

        // 6. Queries not referencing reaffirmed_at still work.
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM memory WHERE status = 'active'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn session_tool_call_table_and_three_indexes_exist_after_migration() {
        crate::db::vec::init_sqlite_vec();
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        create_schema(&conn).unwrap();

        // Table present
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='session_tool_call'",
                [], |row| row.get(0),
            ).unwrap();
        assert_eq!(count, 1, "session_tool_call table should exist");

        // Three indexes present
        let mut stmt = conn
            .prepare(
                "SELECT name FROM sqlite_master WHERE type='index' AND tbl_name='session_tool_call'
             ORDER BY name",
            )
            .unwrap();
        let names: Vec<String> = stmt
            .query_map([], |row| row.get::<_, String>(0))
            .unwrap()
            .filter_map(|r| r.ok())
            .collect();
        assert!(
            names.contains(&"idx_session_tool_name_agent".to_string()),
            "missing idx_session_tool_name_agent; got {:?}",
            names
        );
        assert!(
            names.contains(&"idx_session_tool_org_session_created".to_string()),
            "missing idx_session_tool_org_session_created; got {:?}",
            names
        );
        assert!(
            names.contains(&"idx_session_tool_session".to_string()),
            "missing idx_session_tool_session; got {:?}",
            names
        );
    }

    // ── Phase 2A-4c1 T11: documented rollback-recipe validation ──────────────

    #[test]
    fn test_session_tool_call_rollback_recipe_works_on_populated_db() {
        crate::db::vec::init_sqlite_vec();
        let conn = Connection::open_in_memory().unwrap();
        create_schema(&conn).unwrap();

        conn.execute(
            "INSERT INTO session (id, agent, started_at, status, organization_id)
             VALUES ('S', 'a', '2026-04-19 10:00:00', 'active', 'default')",
            [],
        )
        .unwrap();

        for i in 0..5 {
            conn.execute(
                &format!(
                    "INSERT INTO session_tool_call VALUES
                        ('ID{i}', 'S', 'a', 'T', '{{}}', 'ok', 1, 0, 'default',
                         '2026-04-19 12:00:00')"
                ),
                [],
            )
            .unwrap();
        }

        // Pre-assertion: the forward migration must leave all 3 indexes present
        // before the rollback runs. Without this, a regression that removed the
        // `CREATE INDEX IF NOT EXISTS` lines from `create_schema` would let the
        // `DROP INDEX IF EXISTS` below silently no-op and the post-rollback
        // `idx_count == 0` assertion would pass vacuously.
        let idx_count_before: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master
                 WHERE type='index' AND name IN (
                     'idx_session_tool_session',
                     'idx_session_tool_name_agent',
                     'idx_session_tool_org_session_created'
                 )",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            idx_count_before, 3,
            "all 3 indexes must exist before rollback executes — forward migration regression"
        );

        conn.execute_batch(
            "
            DROP INDEX IF EXISTS idx_session_tool_org_session_created;
            DROP INDEX IF EXISTS idx_session_tool_name_agent;
            DROP INDEX IF EXISTS idx_session_tool_session;
            DROP TABLE IF EXISTS session_tool_call;
            ",
        )
        .unwrap();

        let row_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master
                 WHERE type='table' AND name='session_tool_call'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(row_count, 0, "session_tool_call table should be dropped");

        let idx_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master
                 WHERE type='index' AND name IN (
                     'idx_session_tool_session',
                     'idx_session_tool_name_agent',
                     'idx_session_tool_org_session_created'
                 )",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(idx_count, 0, "all 3 indexes should be dropped");
    }

    // ── Phase 2A-4c2 T1: skill Phase-23 columns + partial unique index ───────

    #[test]
    fn test_skill_has_phase23_columns_and_partial_unique_index() {
        crate::db::vec::init_sqlite_vec();
        let conn = Connection::open_in_memory().unwrap();
        create_schema(&conn).unwrap();

        // All 4 new columns present with correct types + NOT NULL flags.
        let columns: Vec<(String, String, i32)> = conn
            .prepare("PRAGMA table_info(skill)")
            .unwrap()
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(1)?, // name
                    row.get::<_, String>(2)?, // type
                    row.get::<_, i32>(3)?,    // notnull
                ))
            })
            .unwrap()
            .filter_map(|r| r.ok())
            .collect();

        let col_map: std::collections::HashMap<&str, (&str, i32)> = columns
            .iter()
            .map(|(n, t, nn)| (n.as_str(), (t.as_str(), *nn)))
            .collect();

        assert_eq!(
            col_map.get("agent"),
            Some(&("TEXT", 1)),
            "agent column must be TEXT NOT NULL"
        );
        assert_eq!(
            col_map.get("fingerprint"),
            Some(&("TEXT", 1)),
            "fingerprint column must be TEXT NOT NULL"
        );
        assert_eq!(
            col_map.get("inferred_from"),
            Some(&("TEXT", 1)),
            "inferred_from column must be TEXT NOT NULL"
        );
        assert_eq!(
            col_map.get("inferred_at"),
            Some(&("TEXT", 0)),
            "inferred_at column must be TEXT NULL"
        );

        // Partial unique index present, gated on fingerprint != '',
        // and scoped per project so cross-project patterns don't collide
        // (T10 review Codex-H2).
        let idx_sql: String = conn
            .query_row(
                "SELECT sql FROM sqlite_master
                 WHERE type='index' AND name='idx_skill_agent_project_fingerprint'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(
            idx_sql.contains("UNIQUE")
                && idx_sql.contains("agent")
                && idx_sql.contains("project")
                && idx_sql.contains("fingerprint"),
            "expected partial unique index on (agent, project, fingerprint); got: {idx_sql}"
        );
        assert!(
            idx_sql.to_lowercase().contains("where") && idx_sql.contains("fingerprint"),
            "expected WHERE fingerprint != '' partial predicate; got: {idx_sql}"
        );
        // Pre-Codex-H2 index name must not coexist.
        let legacy_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master
                 WHERE type='index' AND name='idx_skill_agent_fingerprint'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            legacy_count, 0,
            "legacy idx_skill_agent_fingerprint must be dropped by the migration"
        );
    }

    // ── Phase 2A-4c2 T9: Phase 23 schema rollback recipe ─────────────────────

    #[test]
    fn test_skill_phase23_columns_and_index_rollback_recipe_works_on_populated_db() {
        crate::db::vec::init_sqlite_vec();
        let conn = Connection::open_in_memory().unwrap();
        create_schema(&conn).unwrap();

        // Seed a Phase 23 skill row.
        conn.execute(
            "INSERT INTO skill
             (id, name, domain, description, steps, source, agent, fingerprint,
              inferred_from, inferred_at, success_count)
             VALUES ('s1', 'Inferred: Read+Edit+Bash [deadbeef]', 'file-ops', '', '[]',
                     'inferred', 'claude-code', 'deadbeefcafe1234',
                     '[\"SA\",\"SB\",\"SC\"]', '2026-04-23T10:00:00Z', 0)",
            [],
        )
        .unwrap();

        // Pre-assertion: the partial unique index must exist before rollback.
        // Without this, a regression that silently removed the index creation
        // would let the rollback's DROP IF EXISTS no-op and the post-assertion
        // pass vacuously (per 2A-4c1 H1 precedent).
        let idx_count_before: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master
                 WHERE type='index' AND name='idx_skill_agent_project_fingerprint'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            idx_count_before, 1,
            "partial unique index must exist before rollback — forward migration regression"
        );

        // Rollback recipe (documented in spec §6 / this test's commit message).
        // SQLite 3.35+ supports ALTER TABLE ... DROP COLUMN directly.
        // Drop both the current and the pre-Codex-H2 index names so the
        // recipe is correct regardless of which schema the DB was migrated
        // from.
        conn.execute_batch(
            "
            DROP INDEX IF EXISTS idx_skill_agent_project_fingerprint;
            DROP INDEX IF EXISTS idx_skill_agent_fingerprint;
            ALTER TABLE skill DROP COLUMN inferred_at;
            ALTER TABLE skill DROP COLUMN inferred_from;
            ALTER TABLE skill DROP COLUMN fingerprint;
            ALTER TABLE skill DROP COLUMN agent;
            ",
        )
        .unwrap();

        // Post-assertions.
        let idx_after: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master
                 WHERE type='index'
                   AND name IN ('idx_skill_agent_project_fingerprint',
                                'idx_skill_agent_fingerprint')",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(idx_after, 0, "partial unique indexes should be dropped");

        // None of the 4 Phase 23 columns exist in PRAGMA table_info any more.
        let cols: Vec<String> = conn
            .prepare("PRAGMA table_info(skill)")
            .unwrap()
            .query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .filter_map(|r| r.ok())
            .collect();
        for phase23_col in ["agent", "fingerprint", "inferred_from", "inferred_at"] {
            assert!(
                !cols.contains(&phase23_col.to_string()),
                "column {phase23_col} must be absent after rollback"
            );
        }

        // Legacy skill columns still present (rollback didn't damage pre-existing schema).
        for legacy_col in ["id", "name", "domain", "description", "success_count"] {
            assert!(
                cols.contains(&legacy_col.to_string()),
                "legacy column {legacy_col} must still exist"
            );
        }
    }

    #[test]
    fn p3_3_11_w29_project_sentinel_backfill() {
        // Phase P3-3.11 W29: project='_global_' sentinel migration —
        // backfill leg.
        //
        // Verifies the data-side migration: pre-existing rows with `project
        // IS NULL` or `project = ''` are rewritten to '_global_' the first
        // time `create_schema` runs against the DB. The forward-going
        // enforcement (every memory-INSERT call site routes its `project`
        // parameter through `db::ops::project_or_global`) is covered in
        // commit 2 of the W29 series. This test deliberately stays on the
        // schema layer: pre-W29 row shape, run create_schema, assert
        // backfill, assert idempotence.
        crate::db::vec::init_sqlite_vec();
        let conn = Connection::open_in_memory().unwrap();

        // Build the pre-W29 memory table shape (project nullable, no
        // sentinel) and seed three rows representative of the F15/F17 leak
        // surface: one true global (NULL), one defensive-empty, one
        // properly tagged.
        conn.execute_batch(
            "CREATE TABLE memory (
                id TEXT PRIMARY KEY,
                memory_type TEXT NOT NULL,
                title TEXT NOT NULL,
                content TEXT NOT NULL,
                confidence REAL NOT NULL DEFAULT 0.8,
                status TEXT NOT NULL DEFAULT 'active',
                project TEXT,
                tags TEXT NOT NULL DEFAULT '[]',
                created_at TEXT NOT NULL,
                accessed_at TEXT NOT NULL,
                organization_id TEXT NOT NULL DEFAULT 'default'
            );",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO memory (id, memory_type, title, content, project, created_at, accessed_at)
             VALUES
                ('legacy-null',  'lesson',  't', 'c', NULL,    '2026-01-01', '2026-01-01'),
                ('legacy-empty', 'lesson',  't', 'c', '',      '2026-01-01', '2026-01-01'),
                ('legacy-forge', 'decision','t', 'c', 'forge', '2026-01-01', '2026-01-01')",
            [],
        )
        .unwrap();

        // Apply the migration.
        create_schema(&conn).unwrap();

        // Backfill: NULL and empty rows now read '_global_'; tagged row is
        // unchanged.
        for (id, expected) in [
            ("legacy-null", "_global_"),
            ("legacy-empty", "_global_"),
            ("legacy-forge", "forge"),
        ] {
            let actual: String = conn
                .query_row(
                    "SELECT project FROM memory WHERE id = ?1",
                    rusqlite::params![id],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(
                actual, expected,
                "backfill must produce expected project for id={id}"
            );
        }

        // Sanity: no row in the table has NULL or empty project after
        // migration.
        let bad_rows: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM memory WHERE project IS NULL OR project = ''",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            bad_rows, 0,
            "after migration no row may have NULL or empty project"
        );

        // Idempotence: re-running create_schema must be a no-op (the
        // needs_backfill predicate skips the rebuild + UPDATE because no
        // rows match the WHERE clause any more).
        create_schema(&conn).unwrap();
        let bad_rows_after_rerun: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM memory WHERE project IS NULL OR project = ''",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            bad_rows_after_rerun, 0,
            "re-running migration must be a no-op"
        );
    }

    #[test]
    fn p3_3_11_w30_identity_project_sentinel_backfill() {
        // Phase P3-3.11 W30: identity.project='_global_' sentinel migration —
        // backfill leg. Mirror of the W29 memory.project sentinel test.
        //
        // Verifies the data-side migration: pre-W30 identity rows (no
        // 'project' column at all) are migrated via ALTER TABLE ADD COLUMN
        // ... NOT NULL DEFAULT '_global_', which SQLite uses to backfill
        // every existing row. The defensive UPDATE inside create_schema
        // covers the rare case where the column was added without DEFAULT
        // (or rows were explicitly set to '' by an out-of-band writer).
        // The forward-going enforcement (DAO helper at every store_identity
        // call site) is covered in W30 commit 2.
        crate::db::vec::init_sqlite_vec();
        let conn = Connection::open_in_memory().unwrap();

        // Build the pre-W30 identity table shape (no project column) and
        // seed three rows — analogues of the F16 leak surface (one true
        // global, one project-tagged extracted, one CLI/declared facet).
        // After ALTER TABLE in create_schema all three should read
        // '_global_' because the pre-existing rows had no project value at
        // all; the migration cannot recover the original project (no source
        // data on the row); future writes will tag explicitly via the DAO
        // helper.
        conn.execute_batch(
            "CREATE TABLE identity (
                id TEXT PRIMARY KEY,
                agent TEXT NOT NULL,
                facet TEXT NOT NULL,
                description TEXT NOT NULL,
                strength REAL NOT NULL DEFAULT 0.5,
                source TEXT NOT NULL,
                active INTEGER NOT NULL DEFAULT 1,
                created_at TEXT NOT NULL
            );",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO identity (id, agent, facet, description, strength, source, active, created_at)
             VALUES
                ('legacy-1', 'claude-code', 'expertise', 'Hive Finance credit risk pipeline', 0.9, 'extracted', 1, '2026-01-01'),
                ('legacy-2', 'claude-code', 'role',      'Architect/Lead for Forge project', 0.9, 'extracted', 1, '2026-01-01'),
                ('legacy-3', 'claude-code', 'values',    'Pragmatic problem solver',         0.9, 'cli',       1, '2026-01-01')",
            [],
        )
        .unwrap();

        // Apply the migration.
        create_schema(&conn).unwrap();

        // Verify the column was added.
        let cols: Vec<String> = conn
            .prepare("PRAGMA table_info(identity)")
            .unwrap()
            .query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .filter_map(Result::ok)
            .collect();
        assert!(
            cols.contains(&"project".to_string()),
            "ALTER TABLE must add 'project' column; got cols={cols:?}"
        );

        // Backfill: existing rows now read '_global_' (the migration cannot
        // recover the original project tagging — that data was never
        // captured pre-W30).
        for id in ["legacy-1", "legacy-2", "legacy-3"] {
            let actual: String = conn
                .query_row(
                    "SELECT project FROM identity WHERE id = ?1",
                    rusqlite::params![id],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(
                actual, "_global_",
                "backfill must produce '_global_' for pre-W30 row id={id}"
            );
        }

        // Sanity: no row in the table has NULL or empty project after
        // migration.
        let bad_rows: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM identity WHERE project IS NULL OR project = ''",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            bad_rows, 0,
            "after migration no identity row may have NULL or empty project"
        );

        // Idempotence: re-running create_schema must be a no-op (the
        // ADD COLUMN errors out silently because the column already exists,
        // and the defensive UPDATE matches zero rows).
        create_schema(&conn).unwrap();
        let bad_rows_after_rerun: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM identity WHERE project IS NULL OR project = ''",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            bad_rows_after_rerun, 0,
            "re-running migration must be a no-op"
        );

        // Defensive UPDATE leg: a row written with an explicit empty string
        // (bypassing the DEFAULT) gets normalised to '_global_' on next
        // create_schema run. This guards against future write sites that
        // forget the DAO helper.
        conn.execute("UPDATE identity SET project = '' WHERE id = 'legacy-1'", [])
            .unwrap();
        create_schema(&conn).unwrap();
        let project_after_heal: String = conn
            .query_row(
                "SELECT project FROM identity WHERE id = 'legacy-1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            project_after_heal, "_global_",
            "defensive UPDATE must heal explicit empty-string project values"
        );
    }

    #[test]
    fn p3_4_w1_2_code_file_project_basename_backfill() {
        // P3-4 W1.2 c1 + W1.3 fw2: code_file.project basename migration —
        // backfill leg. Mirror of the W29/W30 sentinel tests, extended for
        // the SUBSTR/REVERSE/INSTR PATH→basename rewrite and the
        // trailing-slash hygiene pass added in W1.3 fw2.
        //
        // Verifies the data-side migration: pre-W1.2 rows tagged with the
        // full project PATH (`/repo/forge`, `/repo/forge/`, `/`) are
        // rewritten to basename (`forge`, `forge`, sentinel). Already-
        // basename rows (`forge`) are unchanged. NULL / empty rows fall
        // through to `_global_`. Idempotent across reruns.
        crate::db::vec::init_sqlite_vec();
        let conn = Connection::open_in_memory().unwrap();

        // Build the pre-W1.2 code_file table shape and seed rows
        // representative of the PATH-tagging surface plus the corner
        // cases reviewer MED-3 flagged (trailing slash, root, already-
        // basename, NULL, empty).
        conn.execute_batch(
            "CREATE TABLE code_file (
                id TEXT PRIMARY KEY,
                path TEXT NOT NULL UNIQUE,
                language TEXT NOT NULL,
                project TEXT,
                hash TEXT NOT NULL,
                indexed_at TEXT NOT NULL
            );",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO code_file (id, path, language, project, hash, indexed_at) VALUES \
             ('rooted',         '/x/a.rs',  'rust',   '/repo/forge',  'h', '2026-01-01'), \
             ('rooted-deep',    '/x/b.rs',  'rust',   '/mnt/colab-disk/DurgaSaiK/forge', 'h', '2026-01-01'), \
             ('trailing-slash', '/x/c.rs',  'rust',   '/repo/forge/', 'h', '2026-01-01'), \
             ('root-only',      '/x/d.rs',  'rust',   '/',            'h', '2026-01-01'), \
             ('already-name',   '/x/e.rs',  'rust',   'forge',        'h', '2026-01-01'), \
             ('null-project',   '/x/f.rs',  'rust',   NULL,           'h', '2026-01-01'), \
             ('empty-project',  '/x/g.rs',  'rust',   '',             'h', '2026-01-01')",
            [],
        )
        .unwrap();

        // Apply the migration.
        create_schema(&conn).unwrap();

        // Backfill must produce expected basename / sentinel for each row.
        for (id, expected) in [
            ("rooted", "forge"),
            ("rooted-deep", "forge"),
            ("trailing-slash", "forge"),
            ("root-only", "_global_"),
            ("already-name", "forge"),
            ("null-project", "_global_"),
            ("empty-project", "_global_"),
        ] {
            let actual: String = conn
                .query_row(
                    "SELECT project FROM code_file WHERE id = ?1",
                    rusqlite::params![id],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(
                actual, expected,
                "basename backfill must produce expected project for id={id}"
            );
        }

        // Sanity: no row carries NULL / empty / a leading-slash path post-migration.
        let bad_rows: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM code_file WHERE project IS NULL OR project = '' OR project LIKE '/%'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            bad_rows, 0,
            "after migration no code_file row may have NULL/empty/PATH-shaped project"
        );

        // Idempotence: re-running create_schema is a no-op for already-
        // migrated rows (RTRIM('forge', '/')='forge', SUBSTR rooted-only,
        // sentinel WHERE matches nothing).
        create_schema(&conn).unwrap();
        let bad_rows_after_rerun: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM code_file WHERE project IS NULL OR project = '' OR project LIKE '/%'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            bad_rows_after_rerun, 0,
            "re-running migration must be a no-op"
        );
        let still_forge: String = conn
            .query_row(
                "SELECT project FROM code_file WHERE id = 'rooted'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            still_forge, "forge",
            "idempotent rerun must NOT re-mangle already-basename rows"
        );
    }

    #[test]
    fn p3_4_w1_3_code_file_foreign_pollution_cleanup() {
        // P3-4 W1.3 fw2 (review HIGH-3): companion DELETE for pre-W1.2
        // foreign-project pollution. Pre-W1.2 the indexer's `find_project_dir`
        // could fall back to `/mnt`/`/home`/etc. and walk every subtree,
        // pulling foreign files into the code graph. After c1's basename
        // backfill those rows carry `project='mnt'` / `'home'` / etc.,
        // alongside any legitimate project rows. The companion DELETE
        // is doubly-anchored: project=basename(root) AND path begins
        // with /<root>/ — so it removes pollution but spares a real
        // project that just happens to be named 'mnt'.
        crate::db::vec::init_sqlite_vec();
        let conn = Connection::open_in_memory().unwrap();

        // Pre-W1.2 row shape — the migration rewrites `project` from
        // PATH to basename, then the cleanup arm runs.
        conn.execute_batch(
            "CREATE TABLE code_file (
                id TEXT PRIMARY KEY,
                path TEXT NOT NULL UNIQUE,
                language TEXT NOT NULL,
                project TEXT,
                hash TEXT NOT NULL,
                indexed_at TEXT NOT NULL
            );
            CREATE TABLE code_symbol (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                kind TEXT NOT NULL,
                file_path TEXT NOT NULL,
                line_start INTEGER,
                line_end INTEGER,
                signature TEXT
            );
            CREATE TABLE edge (
                id TEXT PRIMARY KEY,
                from_id TEXT NOT NULL,
                to_id TEXT NOT NULL,
                edge_type TEXT NOT NULL,
                properties TEXT NOT NULL DEFAULT '{}',
                created_at TEXT NOT NULL,
                valid_from TEXT NOT NULL,
                valid_until TEXT
            );",
        )
        .unwrap();

        // Pollution: tagged with PATH (which c1 rewrites to 'mnt'/'home'/etc.),
        // path under that root.
        conn.execute(
            "INSERT INTO code_file (id, path, language, project, hash, indexed_at) VALUES \
             ('pollute-mnt',  '/mnt/foreign/user/.cache/jupyter/file.py', 'python', '/mnt',  'h', '2026-01-01'), \
             ('pollute-home', '/home/other-user/repo/file.py',            'python', '/home', 'h', '2026-01-01'), \
             ('pollute-tmp',  '/tmp/scratch/file.py',                     'python', '/tmp',  'h', '2026-01-01')",
            [],
        )
        .unwrap();
        // Legitimate project: real rows under a plausible repo path.
        conn.execute(
            "INSERT INTO code_file (id, path, language, project, hash, indexed_at) VALUES \
             ('legit-forge', '/repo/forge/main.rs', 'rust', '/repo/forge', 'h', '2026-01-01')",
            [],
        )
        .unwrap();
        // Adversarial: a real project that happens to be named 'mnt' but
        // whose PATH does NOT start with /mnt/. Must NOT be cleaned up.
        conn.execute(
            "INSERT INTO code_file (id, path, language, project, hash, indexed_at) VALUES \
             ('legit-mnt-named', '/repo/mnt-tool/lib.rs', 'rust', '/repo/mnt-tool', 'h', '2026-01-01')",
            [],
        )
        .unwrap();

        // Symbols of every file (including pollution) — so we can verify
        // cascading DELETE through code_symbol → edge.
        conn.execute(
            "INSERT INTO code_symbol (id, name, kind, file_path, line_start, line_end, signature) VALUES \
             ('sym-pollute-mnt',  'foreign_fn',     'function', '/mnt/foreign/user/.cache/jupyter/file.py', 1, 1, ''), \
             ('sym-pollute-home', 'other_fn',       'function', '/home/other-user/repo/file.py',            1, 1, ''), \
             ('sym-pollute-tmp',  'scratch_fn',     'function', '/tmp/scratch/file.py',                     1, 1, ''), \
             ('sym-legit-forge',  'main',           'function', '/repo/forge/main.rs',                      1, 1, ''), \
             ('sym-legit-mnt',    'mnt_named_fn',   'function', '/repo/mnt-tool/lib.rs',                    1, 1, '')",
            [],
        )
        .unwrap();
        // Edge connecting two pollution symbols (should be cleaned), one
        // edge connecting two legit symbols (should survive).
        conn.execute(
            "INSERT INTO edge (id, from_id, to_id, edge_type, properties, created_at, valid_from) VALUES \
             ('e-pollute', 'sym-pollute-mnt',  'sym-pollute-home', 'imports', '{}', '2026-01-01', '2026-01-01'), \
             ('e-cross',   'sym-legit-forge',  'sym-pollute-tmp',  'imports', '{}', '2026-01-01', '2026-01-01'), \
             ('e-legit',   'sym-legit-forge',  'sym-legit-mnt',    'imports', '{}', '2026-01-01', '2026-01-01')",
            [],
        )
        .unwrap();

        // Apply the migration.
        create_schema(&conn).unwrap();

        // Pollution rows are gone from code_file.
        for id in ["pollute-mnt", "pollute-home", "pollute-tmp"] {
            let count: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM code_file WHERE id = ?1",
                    rusqlite::params![id],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(count, 0, "pollution row {id} must be deleted");
        }
        // Legit rows survive — both the typical case and the adversarial
        // 'mnt'-named-but-not-/mnt-rooted case.
        for id in ["legit-forge", "legit-mnt-named"] {
            let count: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM code_file WHERE id = ?1",
                    rusqlite::params![id],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(count, 1, "legit row {id} must survive cleanup");
        }
        // The 'mnt'-named legit row's project should be the basename of
        // its path post-migration: 'mnt-tool'.
        let legit_mnt_project: String = conn
            .query_row(
                "SELECT project FROM code_file WHERE id = 'legit-mnt-named'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(legit_mnt_project, "mnt-tool");

        // Symbols of polluted files cascade-deleted; symbols of legit
        // files survive.
        for sym_id in ["sym-pollute-mnt", "sym-pollute-home", "sym-pollute-tmp"] {
            let count: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM code_symbol WHERE id = ?1",
                    rusqlite::params![sym_id],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(count, 0, "polluted symbol {sym_id} must be cascaded out");
        }
        for sym_id in ["sym-legit-forge", "sym-legit-mnt"] {
            let count: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM code_symbol WHERE id = ?1",
                    rusqlite::params![sym_id],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(count, 1, "legit symbol {sym_id} must survive");
        }

        // Edges with EITHER endpoint in pollution are cascade-deleted;
        // the legit-only edge survives.
        let count_pollute: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM edge WHERE id IN ('e-pollute','e-cross')",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            count_pollute, 0,
            "polluted-endpoint edges must be cascaded out"
        );
        let count_legit: i64 = conn
            .query_row("SELECT COUNT(*) FROM edge WHERE id = 'e-legit'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(count_legit, 1, "legit-only edge must survive");

        // Idempotence: second create_schema is a no-op (no rows match the
        // pollution predicate after the first run).
        create_schema(&conn).unwrap();
        let total_files: i64 = conn
            .query_row("SELECT COUNT(*) FROM code_file", [], |r| r.get(0))
            .unwrap();
        assert_eq!(total_files, 2, "second pass deletes nothing further");
    }

    /// P3-4 ZR-C3 migration regression: legacy DB shaped with the
    /// `reality` table (no `project`) survives the schema migration —
    /// the row is preserved under the new table name and the
    /// canonical idx_project_* indexes exist.
    ///
    /// Per `feedback_sqlite_no_reverse_silent_migration_failure.md`,
    /// the migration must NOT silently no-op: this test seeds a real
    /// pre-migration row, runs `create_schema`, and asserts the row
    /// (a) is gone from `reality` and (b) lives in `project` with the
    /// same id. Idempotence is verified by re-running create_schema.
    #[test]
    fn zr_c3_legacy_reality_row_survives_rename_to_project() {
        crate::db::vec::init_sqlite_vec();
        let conn = Connection::open_in_memory().unwrap();

        // Seed the legacy schema directly (NOT via create_schema —
        // that would put us in the new shape). Mirrors the legacy
        // `reality` table + ALL FOUR canonical indexes from prior
        // schema versions so the post-migration DROP-cleanup
        // assertions actually exercise migration code (ZR-C3 fw1
        // MED-2 — without this, three of four assertions are
        // trivially satisfied because the fixture never created
        // the indexes in the first place).
        conn.execute_batch(
            "CREATE TABLE reality (
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
            CREATE INDEX idx_reality_org ON reality(organization_id);
            CREATE INDEX idx_reality_path ON reality(project_path);
            CREATE INDEX idx_reality_owner ON reality(owner_type, owner_id);
            CREATE UNIQUE INDEX idx_reality_path_unique ON reality(project_path) WHERE project_path IS NOT NULL;",
        )
        .unwrap();

        let row_id = "zr-c3-legacy-row";
        conn.execute(
            "INSERT INTO reality (id, name, reality_type, project_path,
                organization_id, owner_type, owner_id, engine_status,
                created_at, last_active)
             VALUES (?1, ?2, 'code', ?3, 'default', 'user', 'local',
                'idle', '2026-04-27T00:00:00Z', '2026-04-27T00:00:00Z')",
            rusqlite::params![row_id, "legacy-name", "/mnt/legacy/path"],
        )
        .unwrap();

        // Run the migration.
        create_schema(&conn).unwrap();

        // Legacy table is gone; canonical table exists.
        let reality_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='reality'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            reality_count, 0,
            "legacy `reality` table must NOT exist post-migration"
        );

        let project_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='project'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            project_count, 1,
            "canonical `project` table must exist post-migration"
        );

        // Row data preserved under the new name.
        let (got_id, got_name, got_path): (String, String, Option<String>) = conn
            .query_row(
                "SELECT id, name, project_path FROM project WHERE id = ?1",
                [row_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(got_id, row_id, "row id preserved through rename");
        assert_eq!(got_name, "legacy-name", "row name preserved");
        assert_eq!(
            got_path.as_deref(),
            Some("/mnt/legacy/path"),
            "project_path column preserved"
        );

        // Canonical indexes exist; legacy index names dropped.
        for legacy in &[
            "idx_reality_org",
            "idx_reality_path",
            "idx_reality_owner",
            "idx_reality_path_unique",
        ] {
            let n: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_master WHERE type='index' AND name=?1",
                    [legacy],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(n, 0, "legacy index `{legacy}` must be dropped");
        }
        for canonical in &[
            "idx_project_org",
            "idx_project_path",
            "idx_project_owner",
            "idx_project_path_unique",
        ] {
            let n: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_master WHERE type='index' AND name=?1",
                    [canonical],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(n, 1, "canonical index `{canonical}` must exist");
        }

        // Idempotence: re-running create_schema is a no-op.
        create_schema(&conn).unwrap();
        let after_id: String = conn
            .query_row(
                "SELECT id FROM project WHERE id = ?1",
                [row_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(after_id, row_id, "row survives idempotent re-run");
    }

    /// P3-4 ZR-C3 fresh-DB regression: create_schema on a brand-new
    /// connection lands the `project` table directly (no legacy
    /// `reality` table is left behind), and the canonical indexes
    /// are present. Mirrors the legacy-row test for the
    /// no-pre-existing-data path.
    #[test]
    fn zr_c3_fresh_db_creates_project_table_no_legacy_residue() {
        crate::db::vec::init_sqlite_vec();
        let conn = Connection::open_in_memory().unwrap();
        create_schema(&conn).unwrap();

        let project_exists: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='project'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(project_exists, 1, "fresh DB must have `project` table");

        let reality_exists: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='reality'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            reality_exists, 0,
            "fresh DB must NOT have legacy `reality` table"
        );
    }

    /// P3-4 ZR-C3 fw1 (HIGH-1): mid-state DB where BOTH `reality`
    /// (empty, legacy residue) AND `project` (canonical) tables
    /// exist. The migration must drop the empty legacy table —
    /// silent skip would leave the orphan in place forever.
    #[test]
    fn zr_c3_fw1_mid_state_with_empty_legacy_drops_reality() {
        crate::db::vec::init_sqlite_vec();
        let conn = Connection::open_in_memory().unwrap();

        // Seed both tables. `reality` is empty (legacy residue);
        // `project` is the canonical table with a row.
        conn.execute_batch(
            "CREATE TABLE reality (
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
            CREATE TABLE project (
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
            );",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO project (id, name, reality_type, organization_id,
                owner_type, owner_id, engine_status, created_at, last_active)
             VALUES ('canonical-row', 'canonical', 'code', 'default',
                'user', 'local', 'idle', '2026-04-27T00:00:00Z',
                '2026-04-27T00:00:00Z')",
            [],
        )
        .unwrap();

        // Migration must succeed.
        create_schema(&conn).unwrap();

        // `reality` is gone; canonical row in `project` survives.
        let reality_exists: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='reality'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            reality_exists, 0,
            "empty legacy `reality` table must be dropped"
        );

        let canonical: String = conn
            .query_row(
                "SELECT name FROM project WHERE id = 'canonical-row'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(canonical, "canonical", "canonical row preserved");
    }

    /// P3-4 ZR-C3 fw1 (HIGH-1): mid-state DB where BOTH tables exist
    /// AND `reality` is non-empty. The migration must error out
    /// rather than silently picking one — the operator has to
    /// resolve the data conflict manually.
    #[test]
    fn zr_c3_fw1_mid_state_with_nonempty_legacy_errors_out() {
        crate::db::vec::init_sqlite_vec();
        let conn = Connection::open_in_memory().unwrap();

        // Seed both tables; both have rows (worst-case mid-state).
        conn.execute_batch(
            "CREATE TABLE reality (
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
            CREATE TABLE project (
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
            );",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO reality (id, name, reality_type, organization_id,
                owner_type, owner_id, engine_status, created_at, last_active)
             VALUES ('legacy-row', 'legacy', 'code', 'default',
                'user', 'local', 'idle', '2026-04-27T00:00:00Z',
                '2026-04-27T00:00:00Z')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO project (id, name, reality_type, organization_id,
                owner_type, owner_id, engine_status, created_at, last_active)
             VALUES ('canonical-row', 'canonical', 'code', 'default',
                'user', 'local', 'idle', '2026-04-27T00:00:00Z',
                '2026-04-27T00:00:00Z')",
            [],
        )
        .unwrap();

        // Migration must error out — silent merging would risk data loss.
        let result = create_schema(&conn);
        assert!(
            result.is_err(),
            "non-empty mid-state must produce an error, not silent skip"
        );
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("mid-state DB"),
            "error must mention mid-state context; got: {err}"
        );
        assert!(
            err.contains("`reality` (1 rows)"),
            "error must report row count of legacy table; got: {err}"
        );

        // Both tables and rows are still intact (untouched).
        let reality_rows: i64 = conn
            .query_row("SELECT COUNT(*) FROM reality", [], |r| r.get(0))
            .unwrap();
        let project_rows: i64 = conn
            .query_row("SELECT COUNT(*) FROM project", [], |r| r.get(0))
            .unwrap();
        assert_eq!(reality_rows, 1, "legacy data preserved on error");
        assert_eq!(project_rows, 1, "canonical data preserved on error");
    }
}
