use rusqlite::Connection;

pub fn create_schema(conn: &Connection) -> rusqlite::Result<()> {
    // Create memory_vec virtual table (sqlite-vec must be loaded before this call)
    conn.execute_batch(
        "CREATE VIRTUAL TABLE IF NOT EXISTS memory_vec USING vec0(
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
