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
    ")
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
}
