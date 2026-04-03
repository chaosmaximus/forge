use forge_daemon::chunk::parse_transcript;
use forge_daemon::config::ForgeConfig;
use forge_daemon::extraction::prompt::{parse_extraction_output, EXTRACTION_SYSTEM_PROMPT};
use forge_daemon::db::ops;
use forge_daemon::server::handler::DaemonState;
use forge_core::types::{Memory, MemoryType};

#[test]
fn test_full_extraction_pipeline_simulated() {
    // 1. Parse a realistic transcript
    let transcript = [
        r#"{"type":"user","message":{"role":"user","content":"Let's use PostgreSQL for the database. It has better JSON support than MySQL."},"uuid":"u1","timestamp":"2026-04-02T12:00:00Z","sessionId":"s1"}"#,
        r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"Good choice. I'll set up PostgreSQL with the sqlx crate for type-safe queries."}]},"uuid":"a1","timestamp":"2026-04-02T12:00:05Z","sessionId":"s1"}"#,
        r#"{"type":"user","message":{"role":"user","content":"Yes, and let's use migrations with sqlx migrate."},"uuid":"u2","timestamp":"2026-04-02T12:01:00Z","sessionId":"s1"}"#,
    ].join("\n");

    let chunks = parse_transcript(&transcript);
    assert_eq!(chunks.len(), 3, "should parse 3 chunks from 3 lines");

    // 2. Simulate extraction output (what Haiku/Ollama would return)
    let simulated_output = r#"[
        {"type":"decision","title":"Use PostgreSQL for database","content":"PostgreSQL chosen over MySQL for better JSON support. Using sqlx crate for type-safe queries.","confidence":0.95,"tags":["database","postgresql","sqlx"],"affects":["Cargo.toml","src/db/"]},
        {"type":"decision","title":"Use sqlx migrate for schema migrations","content":"Database migrations managed via sqlx migrate CLI tool.","confidence":0.85,"tags":["database","migrations"],"affects":["migrations/"]}
    ]"#;

    let extracted = parse_extraction_output(simulated_output);
    assert_eq!(extracted.len(), 2);
    assert_eq!(extracted[0].title, "Use PostgreSQL for database");

    // 3. Store extracted memories
    let state = DaemonState::new(":memory:").unwrap();
    for em in &extracted {
        let memory_type = match em.memory_type.as_str() {
            "decision" => MemoryType::Decision,
            "lesson" => MemoryType::Lesson,
            "pattern" => MemoryType::Pattern,
            "preference" => MemoryType::Preference,
            _ => continue,
        };
        let memory = Memory::new(memory_type, em.title.clone(), em.content.clone())
            .with_confidence(em.confidence)
            .with_tags(em.tags.clone());
        ops::remember(&state.conn, &memory).unwrap();
    }

    // 4. Verify memories are recallable
    let results = ops::recall_bm25(&state.conn, "PostgreSQL database", 10).unwrap();
    assert!(!results.is_empty(), "should find PostgreSQL decision");
    assert!(results.iter().any(|r| r.title.contains("PostgreSQL")));

    // 5. Verify health counts
    let health = ops::health(&state.conn).unwrap();
    assert_eq!(health.decisions, 2, "should have 2 decisions");
}

#[test]
fn test_extraction_prompt_is_valid() {
    assert!(!EXTRACTION_SYSTEM_PROMPT.is_empty());
    assert!(EXTRACTION_SYSTEM_PROMPT.contains("decision"));
    assert!(EXTRACTION_SYSTEM_PROMPT.contains("JSON array"));
    assert!(EXTRACTION_SYSTEM_PROMPT.contains("confidence"));
}

#[test]
fn test_config_defaults_are_sane() {
    let config = ForgeConfig::default();
    assert_eq!(config.extraction.backend, "auto");
    assert_eq!(config.extraction.claude.model, "haiku");
    assert_eq!(config.embedding.dimensions, 768);
    assert!(!config.extraction.ollama.endpoint.is_empty());
}
