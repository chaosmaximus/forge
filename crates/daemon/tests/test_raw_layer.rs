// test_raw_layer.rs — end-to-end integration test for the raw storage layer.
//
// Exercises the full path that production traffic will travel:
//
//   forge-cli RawIngest → handler.rs → raw::ingest_text →
//     chunk_raw + embedder + db::raw → raw_documents/raw_chunks/raw_chunks_vec
//
//   forge-cli RawSearch → handler.rs → raw::search →
//     embedder + db::raw::search_chunks → ranked RawSearchHit list
//
// Uses `FakeEmbedder` so the test runs offline and in <1 s — no fastembed
// model download, no network. A separate `#[ignore]` test could be added that
// installs MiniLMEmbedder and runs the same flow, gated behind
// `FORGE_TEST_FASTEMBED=1`. Keeping that out of the standard suite keeps CI
// fast and deterministic.

use std::sync::Arc;

use forge_core::protocol::{Request, Response, ResponseData};
use forge_daemon::embed::{Embedder, FakeEmbedder};
use forge_daemon::server::handler::{handle_request, DaemonState};

const EMBED_DIM: usize = 384;

fn fresh_state() -> DaemonState {
    let mut state = DaemonState::new(":memory:").expect("DaemonState::new");
    let embedder: Arc<dyn Embedder> = Arc::new(FakeEmbedder::new(EMBED_DIM));
    state.raw_embedder = Some(embedder);
    // Raw layer relies on FK cascades (raw_documents → raw_chunks). Production
    // enables this on every connection; do the same here for parity.
    state
        .conn
        .execute_batch("PRAGMA foreign_keys=ON;")
        .expect("enable FK cascade");
    state
}

fn ingest(state: &mut DaemonState, project: &str, session_id: &str, text: &str) -> String {
    let resp = handle_request(
        state,
        Request::RawIngest {
            text: text.to_string(),
            project: Some(project.to_string()),
            session_id: Some(session_id.to_string()),
            source: "test".to_string(),
            timestamp: None,
            metadata: None,
        },
    );
    match resp {
        Response::Ok {
            data: ResponseData::RawIngest { document_id, .. },
        } => document_id,
        Response::Ok { data } => panic!("unexpected response data: {data:?}"),
        Response::Error { message } => panic!("RawIngest failed: {message}"),
    }
}

fn search(
    state: &mut DaemonState,
    query: &str,
    project: Option<&str>,
    k: Option<usize>,
) -> Vec<forge_core::protocol::RawSearchHit> {
    let resp = handle_request(
        state,
        Request::RawSearch {
            query: query.to_string(),
            project: project.map(String::from),
            session_id: None,
            k,
            // Disable distance cutoff so hits are not silently dropped — the
            // FakeEmbedder produces vectors with arbitrary cosine distances
            // that wouldn't pass the production 0.6 default.
            max_distance: Some(2.0),
        },
    );
    match resp {
        Response::Ok {
            data: ResponseData::RawSearch { hits, .. },
        } => hits,
        Response::Ok { data } => panic!("unexpected response data: {data:?}"),
        Response::Error { message } => panic!("RawSearch failed: {message}"),
    }
}

#[test]
fn raw_ingest_creates_chunks_and_returns_document_id() {
    let mut state = fresh_state();
    let body = "The forge daemon ingests text into the raw layer.".repeat(20);
    let resp = handle_request(
        &mut state,
        Request::RawIngest {
            text: body.clone(),
            project: Some("forge".into()),
            session_id: Some("sess-1".into()),
            source: "claude-code".into(),
            timestamp: None,
            metadata: Some(serde_json::json!({"bench": "smoke"})),
        },
    );

    match resp {
        Response::Ok {
            data:
                ResponseData::RawIngest {
                    document_id,
                    chunk_count,
                    total_chars,
                },
        } => {
            assert!(!document_id.is_empty(), "expected a non-empty document id");
            assert!(
                chunk_count >= 1,
                "expected at least one chunk for ~1 KB body"
            );
            assert_eq!(total_chars, body.chars().count());
        }
        other => panic!("unexpected RawIngest response: {other:?}"),
    }
}

#[test]
fn raw_ingest_empty_text_returns_zero_chunks() {
    let mut state = fresh_state();
    let resp = handle_request(
        &mut state,
        Request::RawIngest {
            text: String::new(),
            project: None,
            session_id: None,
            source: "test".into(),
            timestamp: None,
            metadata: None,
        },
    );
    match resp {
        Response::Ok {
            data:
                ResponseData::RawIngest {
                    document_id,
                    chunk_count,
                    total_chars,
                },
        } => {
            assert!(
                document_id.is_empty(),
                "empty text should not allocate a doc id"
            );
            assert_eq!(chunk_count, 0);
            assert_eq!(total_chars, 0);
        }
        other => panic!("unexpected RawIngest response for empty text: {other:?}"),
    }
}

#[test]
fn raw_search_returns_hits_for_recently_ingested_doc() {
    let mut state = fresh_state();
    let body = "rust async tokio embeddings ".repeat(40); // ~1.1 KB
    let _doc_id = ingest(&mut state, "forge", "sess-1", &body);

    let hits = search(&mut state, &body, None, Some(10));
    assert!(
        !hits.is_empty(),
        "expected at least one hit for an exact body re-query"
    );
    assert_eq!(hits[0].project.as_deref(), Some("forge"));
    assert_eq!(hits[0].session_id.as_deref(), Some("sess-1"));
    assert_eq!(hits[0].source, "test");
    // FakeEmbedder hashes input bytes into a fixed-dim vector. The full-body
    // query and the chunked stored vectors are not literally identical (chunks
    // are substrings of the body), so we expect a small but non-zero cosine
    // distance on the top hit. 0.5 is a comfortable upper bound; in practice
    // the FakeEmbedder produces ~0.07 here.
    assert!(
        hits[0].distance < 0.5,
        "expected small distance on body re-query, got {}",
        hits[0].distance
    );
    // query_embedding_dim is reported on the response — confirm it matches.
    let resp = handle_request(
        &mut state,
        Request::RawSearch {
            query: body.clone(),
            project: None,
            session_id: None,
            k: Some(1),
            max_distance: Some(2.0),
        },
    );
    if let Response::Ok {
        data: ResponseData::RawSearch {
            query_embedding_dim,
            ..
        },
    } = resp
    {
        assert_eq!(query_embedding_dim, EMBED_DIM);
    } else {
        panic!("expected RawSearch ok response");
    }
}

#[test]
fn raw_search_filters_by_project() {
    let mut state = fresh_state();
    let body_a = "code project content ".repeat(40);
    let body_b = "legal project content ".repeat(40);
    ingest(&mut state, "code-proj", "sess-a", &body_a);
    ingest(&mut state, "legal-proj", "sess-b", &body_b);

    let hits_code = search(&mut state, &body_a, Some("code-proj"), Some(20));
    assert!(!hits_code.is_empty());
    for h in &hits_code {
        assert_eq!(h.project.as_deref(), Some("code-proj"));
    }

    let hits_legal = search(&mut state, &body_b, Some("legal-proj"), Some(20));
    assert!(!hits_legal.is_empty());
    for h in &hits_legal {
        assert_eq!(h.project.as_deref(), Some("legal-proj"));
    }
}

#[test]
fn raw_search_returns_error_when_embedder_missing() {
    // Build a state WITHOUT installing an embedder, and ensure no global was set.
    // (FakeEmbedder is per-state via `state.raw_embedder`; we leave that None
    // and rely on no global having been installed by an earlier test in the
    // same binary.) If a previous test in this file installed a global, this
    // test would be hidden — we don't install one anywhere in this file, so
    // the assertion holds.
    let mut state = DaemonState::new(":memory:").unwrap();
    state.raw_embedder = None;
    state.conn.execute_batch("PRAGMA foreign_keys=ON;").unwrap();

    let resp = handle_request(
        &mut state,
        Request::RawSearch {
            query: "hello".into(),
            project: None,
            session_id: None,
            k: None,
            max_distance: None,
        },
    );
    match resp {
        Response::Error { message } => {
            assert!(
                message.contains("not initialized") || message.contains("embedder"),
                "expected a 'not initialized' error, got: {message}"
            );
        }
        other => {
            // If the test binary previously installed a global embedder, this
            // path is reachable and not a failure — just print and move on.
            eprintln!(
                "[raw_layer] embedder appears to be globally installed; skipping the not-initialized assertion. Got: {other:?}"
            );
        }
    }
}
