/// Integration tests for the LSP subsystem.
///
/// These tests verify the LspManager API surface, language server detection,
/// and symbol conversion pipeline. Tests that require a real language server
/// (rust-analyzer) are guarded by a `has_rust_analyzer()` check.

fn has_rust_analyzer() -> bool {
    std::process::Command::new("which")
        .arg("rust-analyzer")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

#[test]
fn test_lsp_detect_on_this_repo() {
    use forge_daemon::lsp::detect::detect_language_servers;
    // This repo has Cargo.toml at the workspace root
    let servers = detect_language_servers(env!("CARGO_MANIFEST_DIR"));
    if has_rust_analyzer() {
        assert!(
            servers.iter().any(|s| s.language == "rust"),
            "should detect rust-analyzer for this Cargo project"
        );
    }
}

#[test]
fn test_lsp_manager_creation() {
    // Verify LspManager compiles and the API surface works.
    // Actual LSP server spawning requires rust-analyzer.
    use forge_daemon::lsp::LspManager;
    let manager = LspManager::new("/tmp/nonexistent".to_string());
    assert_eq!(manager.project_dir(), "/tmp/nonexistent");
    // Manager created successfully with no clients
}

#[test]
fn test_build_call_edges_integration() {
    use forge_daemon::lsp::symbols::build_call_edges;
    use lsp_types::{Location, Position, Range, Uri};

    let def_file = "src/main.rs";
    let refs = vec![
        Location {
            uri: "file:///project/src/lib.rs".parse::<Uri>().unwrap(),
            range: Range {
                start: Position {
                    line: 10,
                    character: 5,
                },
                end: Position {
                    line: 10,
                    character: 15,
                },
            },
        },
        Location {
            uri: "file:///project/src/test.rs".parse::<Uri>().unwrap(),
            range: Range {
                start: Position {
                    line: 20,
                    character: 0,
                },
                end: Position {
                    line: 20,
                    character: 10,
                },
            },
        },
    ];

    let edges = build_call_edges("src/main.rs:main:0", def_file, &refs);
    assert_eq!(edges.len(), 2, "should produce edges for both cross-file references");
}

#[test]
fn test_build_call_edges_deduplication() {
    use forge_daemon::lsp::symbols::build_call_edges;
    use lsp_types::{Location, Position, Range, Uri};

    let def_file = "/src/lib.rs";
    // Two references from the same file should deduplicate to one edge
    let refs = vec![
        Location {
            uri: "file:///src/caller.rs".parse::<Uri>().unwrap(),
            range: Range {
                start: Position { line: 5, character: 0 },
                end: Position { line: 5, character: 10 },
            },
        },
        Location {
            uri: "file:///src/caller.rs".parse::<Uri>().unwrap(),
            range: Range {
                start: Position { line: 15, character: 0 },
                end: Position { line: 15, character: 10 },
            },
        },
    ];

    let edges = build_call_edges("/src/lib.rs:process:0", def_file, &refs);
    assert_eq!(edges.len(), 1, "duplicate same-file references should be deduplicated");
}

#[test]
fn test_convert_symbols_roundtrip() {
    use forge_daemon::lsp::symbols::convert_symbols;
    use lsp_types::{DocumentSymbol, Position, Range, SymbolKind};

    #[allow(deprecated)]
    let symbols = vec![DocumentSymbol {
        name: "my_function".to_string(),
        detail: Some("fn my_function(x: i32) -> bool".to_string()),
        kind: SymbolKind::FUNCTION,
        tags: None,
        deprecated: None,
        range: Range {
            start: Position { line: 0, character: 0 },
            end: Position { line: 10, character: 1 },
        },
        selection_range: Range {
            start: Position { line: 0, character: 3 },
            end: Position { line: 0, character: 14 },
        },
        children: None,
    }];

    let result = convert_symbols("src/example.rs", &symbols);
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].name, "my_function");
    assert_eq!(result[0].kind, "function");
    assert_eq!(result[0].file_path, "src/example.rs");
    assert_eq!(result[0].line_start, 0);
    assert_eq!(result[0].line_end, Some(10));
    assert_eq!(
        result[0].signature,
        Some("fn my_function(x: i32) -> bool".to_string())
    );
    assert_eq!(result[0].id, "src/example.rs:my_function:0");
}
