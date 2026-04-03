use forge_core::types::CodeSymbol;
use lsp_types::{DocumentSymbol, SymbolKind};

/// Convert LSP DocumentSymbols to Forge CodeSymbol records.
///
/// Recursively flattens nested symbols (e.g., methods inside a class)
/// into a flat list suitable for database storage.
pub fn convert_symbols(file_path: &str, symbols: &[DocumentSymbol]) -> Vec<CodeSymbol> {
    let mut result = Vec::new();
    flatten_symbols(file_path, symbols, &mut result);
    result
}

fn flatten_symbols(file_path: &str, symbols: &[DocumentSymbol], out: &mut Vec<CodeSymbol>) {
    for sym in symbols {
        let kind = match sym.kind {
            SymbolKind::FUNCTION | SymbolKind::METHOD => "function",
            SymbolKind::CLASS | SymbolKind::STRUCT => "class",
            SymbolKind::INTERFACE | SymbolKind::TYPE_PARAMETER => "interface",
            SymbolKind::MODULE | SymbolKind::NAMESPACE => "module",
            SymbolKind::ENUM => "enum",
            SymbolKind::CONSTANT | SymbolKind::VARIABLE => "variable",
            SymbolKind::FIELD | SymbolKind::PROPERTY => "field",
            SymbolKind::CONSTRUCTOR => "function",
            _ => "other",
        };

        out.push(CodeSymbol {
            id: format!("{}:{}:{}", file_path, sym.name, sym.range.start.line),
            name: sym.name.clone(),
            kind: kind.to_string(),
            file_path: file_path.to_string(),
            line_start: sym.range.start.line as usize,
            line_end: Some(sym.range.end.line as usize),
            signature: sym.detail.clone(),
        });

        // Recurse into children (e.g., methods inside a class).
        if let Some(children) = &sym.children {
            flatten_symbols(file_path, children, out);
        }
    }
}

/// Build "calls" edges from LSP reference locations.
///
/// For a symbol defined in `definition_file` with `symbol_id`, creates edges
/// from each referencing file to the symbol. Self-file references are excluded
/// and duplicate edges (same from_id) are deduplicated.
pub fn build_call_edges(
    symbol_id: &str,
    definition_file: &str,
    references: &[lsp_types::Location],
) -> Vec<(String, String)> {
    let mut edges = Vec::new();
    let mut seen = std::collections::HashSet::new();

    for loc in references {
        let ref_path = loc.uri.path().as_str();
        // Skip self-references (same file as the definition)
        if ref_path == definition_file {
            continue;
        }
        let from_id = format!("file:{}", ref_path);
        if seen.insert(from_id.clone()) {
            edges.push((from_id, symbol_id.to_string()));
        }
    }
    edges
}

#[cfg(test)]
mod tests {
    use super::*;
    use lsp_types::{Position, Range};

    /// Helper to create a DocumentSymbol for testing.
    fn make_symbol(
        name: &str,
        kind: SymbolKind,
        start_line: u32,
        end_line: u32,
        detail: Option<&str>,
        children: Option<Vec<DocumentSymbol>>,
    ) -> DocumentSymbol {
        #[allow(deprecated)]
        DocumentSymbol {
            name: name.to_string(),
            detail: detail.map(|s| s.to_string()),
            kind,
            tags: None,
            deprecated: None,
            range: Range {
                start: Position { line: start_line, character: 0 },
                end: Position { line: end_line, character: 0 },
            },
            selection_range: Range {
                start: Position { line: start_line, character: 0 },
                end: Position { line: start_line, character: name.len() as u32 },
            },
            children,
        }
    }

    #[test]
    fn test_convert_simple_symbols() {
        let symbols = vec![
            make_symbol("main", SymbolKind::FUNCTION, 0, 10, Some("fn main()"), None),
            make_symbol("helper", SymbolKind::FUNCTION, 12, 20, None, None),
        ];

        let result = convert_symbols("src/main.rs", &symbols);

        assert_eq!(result.len(), 2);
        assert_eq!(result[0].name, "main");
        assert_eq!(result[0].kind, "function");
        assert_eq!(result[0].line_start, 0);
        assert_eq!(result[0].line_end, Some(10));
        assert_eq!(result[0].signature, Some("fn main()".into()));
        assert_eq!(result[0].file_path, "src/main.rs");

        assert_eq!(result[1].name, "helper");
        assert_eq!(result[1].kind, "function");
        assert_eq!(result[1].signature, None);
    }

    #[test]
    fn test_convert_nested_symbols() {
        let methods = vec![
            make_symbol("new", SymbolKind::METHOD, 2, 5, Some("fn new() -> Self"), None),
            make_symbol("run", SymbolKind::METHOD, 7, 15, Some("fn run(&self)"), None),
        ];
        let class = make_symbol("MyClass", SymbolKind::CLASS, 0, 16, None, Some(methods));

        let result = convert_symbols("src/lib.rs", &[class]);

        assert_eq!(result.len(), 3, "class + 2 methods");
        assert_eq!(result[0].name, "MyClass");
        assert_eq!(result[0].kind, "class");
        assert_eq!(result[1].name, "new");
        assert_eq!(result[1].kind, "function");
        assert_eq!(result[2].name, "run");
        assert_eq!(result[2].kind, "function");
    }

    #[test]
    fn test_symbol_id_format() {
        let symbols = vec![
            make_symbol("process", SymbolKind::FUNCTION, 42, 60, None, None),
        ];

        let result = convert_symbols("src/worker.rs", &symbols);

        assert_eq!(result[0].id, "src/worker.rs:process:42");
    }

    #[test]
    fn test_all_symbol_kinds() {
        let symbols = vec![
            make_symbol("MyStruct", SymbolKind::STRUCT, 0, 5, None, None),
            make_symbol("MyInterface", SymbolKind::INTERFACE, 6, 10, None, None),
            make_symbol("my_mod", SymbolKind::MODULE, 11, 20, None, None),
            make_symbol("Color", SymbolKind::ENUM, 21, 30, None, None),
            make_symbol("MAX_SIZE", SymbolKind::CONSTANT, 31, 31, None, None),
            make_symbol("count", SymbolKind::VARIABLE, 32, 32, None, None),
            make_symbol("name", SymbolKind::FIELD, 33, 33, None, None),
        ];

        let result = convert_symbols("test.rs", &symbols);

        assert_eq!(result[0].kind, "class");      // STRUCT -> "class"
        assert_eq!(result[1].kind, "interface");
        assert_eq!(result[2].kind, "module");
        assert_eq!(result[3].kind, "enum");
        assert_eq!(result[4].kind, "variable");    // CONSTANT -> "variable"
        assert_eq!(result[5].kind, "variable");
        assert_eq!(result[6].kind, "field");
    }

    #[test]
    fn test_empty_symbols() {
        let result = convert_symbols("empty.rs", &[]);
        assert!(result.is_empty());
    }

    /// Helper to create an lsp_types::Location for testing build_call_edges.
    fn make_location(path: &str, line: u32) -> lsp_types::Location {
        let uri: lsp_types::Uri = format!("file://{}", path)
            .parse()
            .expect("valid URI");
        lsp_types::Location {
            uri,
            range: Range {
                start: Position { line, character: 0 },
                end: Position { line, character: 10 },
            },
        }
    }

    #[test]
    fn test_build_call_edges() {
        // 3 references from 2 different files → 2 edges (deduped)
        let refs = vec![
            make_location("/src/caller_a.rs", 10),
            make_location("/src/caller_b.rs", 20),
            make_location("/src/caller_a.rs", 30), // duplicate file
        ];

        let edges = super::build_call_edges(
            "/src/lib.rs:process:5",
            "/src/lib.rs",
            &refs,
        );

        assert_eq!(edges.len(), 2, "should deduplicate same-file refs");
        assert_eq!(edges[0], ("file:/src/caller_a.rs".to_string(), "/src/lib.rs:process:5".to_string()));
        assert_eq!(edges[1], ("file:/src/caller_b.rs".to_string(), "/src/lib.rs:process:5".to_string()));
    }

    #[test]
    fn test_build_call_edges_skips_self_file() {
        // All references from the same file as the definition → 0 edges
        let refs = vec![
            make_location("/src/lib.rs", 10),
            make_location("/src/lib.rs", 25),
        ];

        let edges = super::build_call_edges(
            "/src/lib.rs:process:5",
            "/src/lib.rs",
            &refs,
        );

        assert!(edges.is_empty(), "self-file references should be excluded");
    }
}
