//! Recall quality benchmark — automated precision tests for recall scoring.
//!
//! Seeds 20 realistic Forge memories (decisions, lessons, patterns, protocols)
//! across different topics, runs 5 benchmark queries with known expected results,
//! and verifies:
//!   - Top results contain the expected memories
//!   - Score range shows meaningful discrimination (top/bottom ratio > 1.5)
//!   - Each query returns results
//!
//! Note: The recall engine uses FTS5 BM25 (literal keyword matching with OR).
//! Queries are designed so their terms have literal overlap with multiple seed
//! memories, ensuring the benchmark tests real BM25 ranking quality.

use forge_core::protocol::*;
use forge_core::types::MemoryType;
use forge_daemon::server::handler::{handle_request, DaemonState};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Create a fresh in-memory DaemonState (includes schema + defaults).
fn fresh_state() -> DaemonState {
    DaemonState::new(":memory:").expect("DaemonState::new(:memory:)")
}

/// Remember a memory through the handler and return its ID.
fn do_remember(
    state: &mut DaemonState,
    memory_type: MemoryType,
    title: &str,
    content: &str,
) -> String {
    let resp = handle_request(
        state,
        Request::Remember {
            memory_type,
            title: title.into(),
            content: content.into(),
            confidence: Some(0.9),
            tags: None,
            project: Some("forge".into()),
            metadata: None,
        },
    );
    match resp {
        Response::Ok {
            data: ResponseData::Stored { id },
        } => {
            assert!(!id.is_empty(), "stored ID must not be empty");
            id
        }
        other => panic!("expected Stored, got: {other:?}"),
    }
}

/// Recall memories through the handler with project filter.
fn do_recall(state: &mut DaemonState, query: &str, limit: Option<usize>) -> Vec<MemoryResult> {
    let resp = handle_request(
        state,
        Request::Recall {
            query: query.into(),
            memory_type: None,
            project: Some("forge".into()),
            limit,
            layer: None,
            since: None,
            include_flipped: None,
            include_globals: None,
            query_embedding: None,
        },
    );
    match resp {
        Response::Ok {
            data: ResponseData::Memories { results, .. },
        } => results,
        other => panic!("expected Memories, got: {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Seed data: 20 realistic Forge memories
// ---------------------------------------------------------------------------

struct SeedMemory {
    memory_type: MemoryType,
    title: &'static str,
    content: &'static str,
}

fn seed_memories() -> Vec<SeedMemory> {
    vec![
        SeedMemory {
            memory_type: MemoryType::Decision,
            title: "Use SQLite with WAL mode",
            content: "SQLite WAL provides concurrent reads with single writer. Chosen over PostgreSQL for embedded simplicity and single-file portability.",
        },
        SeedMemory {
            memory_type: MemoryType::Decision,
            title: "JWT auth with OIDC discovery",
            content: "RS256 JWT validation with automatic JWKS discovery from issuer URL. Supports Okta, Azure AD, any OIDC provider.",
        },
        SeedMemory {
            memory_type: MemoryType::Decision,
            title: "8-layer Manas memory architecture",
            content: "Bio-inspired 8 layers: platform, tools, skills, domain DNA, experience, perception, declared, latent. Each layer serves different cognitive function.",
        },
        SeedMemory {
            memory_type: MemoryType::Lesson,
            title: "Read-only connections cannot write in SQLite",
            content: "Socket handler SQLITE_OPEN_READ_ONLY silently fails on touch/boost. Must route writes through writer actor channel.",
        },
        SeedMemory {
            memory_type: MemoryType::Lesson,
            title: "RRF scoring needs original scores blended in",
            content: "Pure RRF with k=60 flattens all score differences. Blending 60% original BM25 + 40% RRF gives better discrimination.",
        },
        SeedMemory {
            memory_type: MemoryType::Decision,
            title: "Distroless Docker image",
            content: "gcr.io/distroless/cc-debian12:nonroot at 41.7MB vs 95MB debian:bookworm-slim. Alpine failed due to sqlite-vec musl compilation.",
        },
        SeedMemory {
            memory_type: MemoryType::Pattern,
            title: "Actor model for write serialization",
            content: "WriterActor owns the write connection. All writes go through mpsc channel. Read-only connections for socket handlers.",
        },
        SeedMemory {
            memory_type: MemoryType::Decision,
            title: "Rate limiter exempts localhost",
            content: "100 req/min/IP for remote, unlimited for 127.0.0.1/::1. Daemon's own web UI must not be rate-limited.",
        },
        SeedMemory {
            memory_type: MemoryType::Lesson,
            title: "Config writes must be surgical not full-serialize",
            content: "toml::Value table edits preserve unmodified keys. ForgeConfig deserialization injects defaults that overwrite user settings.",
        },
        SeedMemory {
            memory_type: MemoryType::Decision,
            title: "FISP inter-session messaging protocol",
            content: "Sessions communicate through daemon via messages. Supports broadcast, delegation, meetings, voting.",
        },
        SeedMemory {
            memory_type: MemoryType::Protocol,
            title: "TDD with adversarial review before merge",
            content: "Every feature: test first, implement, Codex adversarial review. Fix all HIGH findings before merge.",
        },
        SeedMemory {
            memory_type: MemoryType::Lesson,
            title: "Indexer project detection must use session CWD",
            content: "find_project_dir timestamp heuristic picks wrong project on multi-project machines. Active session CWD is reliable.",
        },
        SeedMemory {
            memory_type: MemoryType::Decision,
            title: "Regex fallback for TS/JS symbol extraction",
            content: "When typescript-language-server unavailable, extract symbols via regex patterns. Covers functions, classes, interfaces, imports.",
        },
        SeedMemory {
            memory_type: MemoryType::Pattern,
            title: "Healing two-tier system",
            content: "Aggressive fade for quality<0.1 at 3 days. Normal fade for quality<0.2 at 7 days. Accelerated decay 0.15/cycle for quality<0.3.",
        },
        SeedMemory {
            memory_type: MemoryType::Decision,
            title: "Team goal ancestry from Paperclip analysis",
            content: "Every team task traces to a project goal. Gives agents the 'why' behind their work. Competitive feature from Paperclip.",
        },
        SeedMemory {
            memory_type: MemoryType::Lesson,
            title: "blast-radius needs multi-format path resolution",
            content: "Edge table has bare paths, file: prefixed paths, absolute and relative. Must try all formats to find matches.",
        },
        SeedMemory {
            memory_type: MemoryType::Decision,
            title: "Budget enforcement per agent session",
            content: "Track cumulative cost per agent. Auto-detect when budget exceeded. Reject negative amounts to prevent bypass.",
        },
        SeedMemory {
            memory_type: MemoryType::Pattern,
            title: "Proactive context from 8 Manas layers",
            content: "Compile decisions, lessons, skills, identity, disposition, protocols, guardrails into XML context block for each session.",
        },
        SeedMemory {
            memory_type: MemoryType::Lesson,
            title: "FTS5 terms must be double-quoted for safety",
            content: "Even with alphanumeric sanitization, double-quoting prevents FTS5 special syntax. Defense in depth.",
        },
        SeedMemory {
            memory_type: MemoryType::Decision,
            title: "Surgical TOML config writes via toml::Value",
            content: "Read as toml::Value table, modify only the changed key, write back. Preserves all other user settings.",
        },
    ]
}

/// Seed all 20 memories into a fresh state and return (state, title->id map).
fn seed_state() -> (DaemonState, std::collections::HashMap<&'static str, String>) {
    let mut state = fresh_state();
    let seeds = seed_memories();
    let mut title_to_id: std::collections::HashMap<&'static str, String> =
        std::collections::HashMap::new();
    for seed in &seeds {
        let id = do_remember(
            &mut state,
            seed.memory_type.clone(),
            seed.title,
            seed.content,
        );
        title_to_id.insert(seed.title, id);
    }
    assert_eq!(title_to_id.len(), 20, "expected 20 seeded memories");
    (state, title_to_id)
}

// ---------------------------------------------------------------------------
// Benchmark queries with expected results
//
// Each query uses terms that literally appear (via FTS5 tokenization) in
// multiple seed memories, so BM25 returns enough results for ranking tests.
//
// FTS5 tokenizer splits on non-alphanumeric characters and is case-insensitive.
// Query terms are OR'd, so any matching term contributes results.
// ---------------------------------------------------------------------------

struct BenchmarkQuery {
    query: &'static str,
    /// Titles that should appear in top-5 results.
    expected_titles: Vec<&'static str>,
    /// Minimum number of expected matches in top-5.
    min_expected_in_top5: usize,
    /// Minimum total results expected (FTS5 literal match constraint).
    min_results: usize,
}

fn benchmark_queries() -> Vec<BenchmarkQuery> {
    vec![
        // Query 1: "SQLite" matches #0 (WAL), #3 (read-only), #5 (sqlite-vec in content).
        // "connection" matches #3, #6. "handler" matches #3, #6.
        // Expected: #0 and #3 should rank highest (most term overlap with "SQLite").
        BenchmarkQuery {
            query: "SQLite connection handler",
            expected_titles: vec![
                "Use SQLite with WAL mode",
                "Read-only connections cannot write in SQLite",
                "Actor model for write serialization",
            ],
            min_expected_in_top5: 2,
            min_results: 3,
        },
        // Query 2: "session" matches #9 (FISP), #11 (indexer), #16 (budget), #17 (context).
        // "daemon" matches #7 (rate limiter content), #9 (FISP content).
        // Expected: #9 (FISP) should rank highest (both "session" and "daemon").
        BenchmarkQuery {
            query: "session daemon protocol",
            expected_titles: vec![
                "FISP inter-session messaging protocol",
                "Budget enforcement per agent session",
                "Proactive context from 8 Manas layers",
            ],
            min_expected_in_top5: 2,
            min_results: 3,
        },
        // Query 3: "Docker" matches #5. "image" matches #5.
        // "Alpine" matches #5. "compilation" matches #5.
        // This query strongly targets one memory — tests precision.
        BenchmarkQuery {
            query: "Docker image Alpine",
            expected_titles: vec!["Distroless Docker image"],
            min_expected_in_top5: 1,
            min_results: 1,
        },
        // Query 4: "layer" matches #2 (8-layer), #17 (8 Manas layers).
        // "Manas" matches #2, #17. "cognitive" matches #2.
        // Expected: #2 and #17 should dominate.
        BenchmarkQuery {
            query: "Manas layer cognitive",
            expected_titles: vec![
                "8-layer Manas memory architecture",
                "Proactive context from 8 Manas layers",
            ],
            min_expected_in_top5: 2,
            min_results: 2,
        },
        // Query 5: "TOML" matches #8 (config writes content "toml"), #19 (title "TOML").
        // "config" matches #8 (title "Config"), #19 (content "config").
        // "settings" matches #8 (content "settings"), #19 (content "settings").
        // "Value" matches #8, #19 (both mention "toml::Value").
        // Expected: both TOML memories should rank top.
        BenchmarkQuery {
            query: "TOML config settings Value",
            expected_titles: vec![
                "Surgical TOML config writes via toml::Value",
                "Config writes must be surgical not full-serialize",
            ],
            min_expected_in_top5: 2,
            min_results: 2,
        },
    ]
}

// ===========================================================================
// Main benchmark test — runs all 5 queries in one seeded state
// ===========================================================================
#[test]
fn test_recall_quality_benchmark() {
    let (mut state, _title_to_id) = seed_state();

    let queries = benchmark_queries();
    for bq in &queries {
        let results = do_recall(&mut state, bq.query, Some(10));

        // Assertion 1: minimum results returned
        assert!(
            results.len() >= bq.min_results,
            "query '{}': expected at least {} results, got {}. \
             Titles returned: {:?}",
            bq.query,
            bq.min_results,
            results.len(),
            results
                .iter()
                .map(|r| r.memory.title.as_str())
                .collect::<Vec<_>>()
        );

        // Assertion 2: expected titles appear in top-5
        let top5_titles: Vec<&str> = results
            .iter()
            .take(5)
            .map(|r| r.memory.title.as_str())
            .collect();

        let matches_in_top5 = bq
            .expected_titles
            .iter()
            .filter(|expected| top5_titles.contains(expected))
            .count();

        assert!(
            matches_in_top5 >= bq.min_expected_in_top5,
            "query '{}': expected at least {} of {:?} in top-5, found {} matches. \
             Top-5 titles: {:?}",
            bq.query,
            bq.min_expected_in_top5,
            bq.expected_titles,
            matches_in_top5,
            top5_titles
        );

        // Assertion 3: score discrimination when we have enough results
        if results.len() >= 3 {
            let top_score = results[0].score;
            let bottom_score = results.last().unwrap().score;

            if bottom_score > 0.0 {
                let ratio = top_score / bottom_score;
                assert!(
                    ratio > 1.5,
                    "query '{}': score discrimination too weak. \
                     top={:.4}, bottom={:.4}, ratio={:.4}. \
                     Expected ratio > 1.5. All scores: {:?}",
                    bq.query,
                    top_score,
                    bottom_score,
                    ratio,
                    results.iter().map(|r| r.score).collect::<Vec<_>>()
                );
            }
        }
    }
}

// ===========================================================================
// Individual query tests for detailed diagnostics
// ===========================================================================

/// Query 1: SQLite-related memories should surface for database queries.
#[test]
fn test_recall_benchmark_sqlite_query() {
    let (mut state, _) = seed_state();

    let results = do_recall(&mut state, "SQLite connection handler", Some(10));
    assert!(
        results.len() >= 3,
        "SQLite query: expected >= 3 results, got {}. Titles: {:?}",
        results.len(),
        results
            .iter()
            .map(|r| r.memory.title.as_str())
            .collect::<Vec<_>>()
    );

    let top5: Vec<&str> = results
        .iter()
        .take(5)
        .map(|r| r.memory.title.as_str())
        .collect();
    let has_wal = top5.contains(&"Use SQLite with WAL mode");
    let has_readonly = top5.contains(&"Read-only connections cannot write in SQLite");
    assert!(
        has_wal || has_readonly,
        "SQLite query: expected WAL or read-only memory in top-5, got: {top5:?}"
    );
}

/// Query 2: Session/daemon queries should surface FISP protocol.
#[test]
fn test_recall_benchmark_session_query() {
    let (mut state, _) = seed_state();

    let results = do_recall(&mut state, "session daemon protocol", Some(10));
    assert!(
        results.len() >= 3,
        "session query: expected >= 3 results, got {}. Titles: {:?}",
        results.len(),
        results
            .iter()
            .map(|r| r.memory.title.as_str())
            .collect::<Vec<_>>()
    );

    let top5: Vec<&str> = results
        .iter()
        .take(5)
        .map(|r| r.memory.title.as_str())
        .collect();
    let has_fisp = top5.contains(&"FISP inter-session messaging protocol");
    assert!(
        has_fisp,
        "session query: expected FISP memory in top-5, got: {top5:?}"
    );
}

/// Query 3: Docker query should find the distroless image decision.
#[test]
fn test_recall_benchmark_docker_query() {
    let (mut state, _) = seed_state();

    let results = do_recall(&mut state, "Docker image Alpine", Some(10));
    assert!(
        !results.is_empty(),
        "Docker query: expected at least 1 result, got 0"
    );

    let top5: Vec<&str> = results
        .iter()
        .take(5)
        .map(|r| r.memory.title.as_str())
        .collect();
    let has_distroless = top5.contains(&"Distroless Docker image");
    assert!(
        has_distroless,
        "Docker query: expected Distroless memory in top-5, got: {top5:?}"
    );
}

/// Query 4: Manas/layer query should surface the memory architecture decisions.
#[test]
fn test_recall_benchmark_manas_query() {
    let (mut state, _) = seed_state();

    let results = do_recall(&mut state, "Manas layer cognitive", Some(10));
    assert!(
        results.len() >= 2,
        "Manas query: expected >= 2 results, got {}. Titles: {:?}",
        results.len(),
        results
            .iter()
            .map(|r| r.memory.title.as_str())
            .collect::<Vec<_>>()
    );

    let top5: Vec<&str> = results
        .iter()
        .take(5)
        .map(|r| r.memory.title.as_str())
        .collect();
    let has_architecture = top5.contains(&"8-layer Manas memory architecture");
    let has_proactive = top5.contains(&"Proactive context from 8 Manas layers");
    assert!(
        has_architecture || has_proactive,
        "Manas query: expected architecture or proactive memory in top-5, got: {top5:?}"
    );
}

/// Query 5: TOML config queries should surface both config-related memories.
#[test]
fn test_recall_benchmark_toml_query() {
    let (mut state, _) = seed_state();

    let results = do_recall(&mut state, "TOML config settings Value", Some(10));
    assert!(
        results.len() >= 2,
        "TOML query: expected >= 2 results, got {}. Titles: {:?}",
        results.len(),
        results
            .iter()
            .map(|r| r.memory.title.as_str())
            .collect::<Vec<_>>()
    );

    let top5: Vec<&str> = results
        .iter()
        .take(5)
        .map(|r| r.memory.title.as_str())
        .collect();
    let has_surgical = top5.contains(&"Surgical TOML config writes via toml::Value");
    let has_config = top5.contains(&"Config writes must be surgical not full-serialize");
    assert!(
        has_surgical || has_config,
        "TOML query: expected surgical or config memory in top-5, got: {top5:?}"
    );
}

// ===========================================================================
// Score discrimination test — verifies BM25 produces non-flat scores
// ===========================================================================
#[test]
fn test_recall_benchmark_score_discrimination() {
    let (mut state, _) = seed_state();

    // Use queries that are guaranteed to return 3+ results for ratio testing.
    // (TOML query only returns 2 results, so it is excluded here.)
    let queries = ["SQLite connection handler", "session daemon protocol"];

    for query in &queries {
        let results = do_recall(&mut state, query, Some(10));
        assert!(
            results.len() >= 3,
            "query '{}': expected at least 3 results, got {}. Titles: {:?}",
            query,
            results.len(),
            results
                .iter()
                .map(|r| r.memory.title.as_str())
                .collect::<Vec<_>>()
        );

        let scores: Vec<f64> = results.iter().map(|r| r.score).collect();
        let max_score = scores.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
        let min_score = scores.iter().cloned().fold(f64::INFINITY, f64::min);

        // Scores should not be flat — there must be meaningful discrimination.
        assert!(
            max_score > min_score,
            "query '{query}': all scores identical ({max_score:.4}), no discrimination"
        );

        // The score range ratio should exceed 1.5x.
        if min_score > 0.0 {
            let ratio = max_score / min_score;
            assert!(
                ratio > 1.5,
                "query '{query}': discrimination ratio {ratio:.4} (max={max_score:.4}, min={min_score:.4}) below 1.5 threshold. \
                 Scores: {scores:?}"
            );
        }
    }
}

// ===========================================================================
// All queries return non-empty results — basic recall health check
// ===========================================================================
#[test]
fn test_recall_benchmark_all_queries_return_results() {
    let (mut state, _) = seed_state();

    let queries = benchmark_queries();
    for bq in &queries {
        let results = do_recall(&mut state, bq.query, Some(10));
        assert!(
            !results.is_empty(),
            "query '{}' returned 0 results — FTS5 BM25 may be broken",
            bq.query
        );
        // Every result should have a positive score.
        for (i, r) in results.iter().enumerate() {
            assert!(
                r.score > 0.0,
                "query '{}': result[{}] ('{}') has non-positive score {:.6}",
                bq.query,
                i,
                r.memory.title,
                r.score
            );
        }
    }
}
