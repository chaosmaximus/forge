use serde::{Serialize, Deserialize};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Import {
    pub source_file: String,
    pub target_module: String,
    pub names: Vec<String>,
    pub line: usize,
}

/// Extract import statements from a tree-sitter parse tree.
///
/// Supports Python (`import`, `from ... import`), JavaScript and TypeScript
/// (`import`, `require`).  The caller is responsible for creating the parser
/// and passing the root node of the already-parsed tree.
pub fn extract_imports(
    root: &tree_sitter::Node,
    source: &str,
    file_path: &str,
    language: &str,
) -> Vec<Import> {
    let mut imports = Vec::new();
    match language {
        "python" => extract_python_imports(root, source, file_path, &mut imports),
        "javascript" | "typescript" => extract_js_imports(root, source, file_path, &mut imports),
        _ => {}
    }
    imports
}

// ---------------------------------------------------------------------------
// Python
// ---------------------------------------------------------------------------

fn extract_python_imports(
    node: &tree_sitter::Node,
    source: &str,
    file_path: &str,
    imports: &mut Vec<Import>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "import_statement" => {
                // `import foo.bar` or `import foo.bar, baz.qux`
                let line = child.start_position().row + 1;
                let mut inner = child.walk();
                for c in child.children(&mut inner) {
                    if c.kind() == "dotted_name" {
                        let target = &source[c.byte_range()];
                        imports.push(Import {
                            source_file: file_path.to_string(),
                            target_module: target.to_string(),
                            names: Vec::new(),
                            line,
                        });
                    }
                }
            }
            "import_from_statement" => {
                // `from foo.bar import baz, qux` or `from . import models`
                let line = child.start_position().row + 1;
                let (target_module, names) = extract_python_from_parts(&child, source);
                imports.push(Import {
                    source_file: file_path.to_string(),
                    target_module,
                    names,
                    line,
                });
            }
            _ => {
                extract_python_imports(&child, source, file_path, imports);
            }
        }
    }
}

/// Parse an `import_from_statement` node.
///
/// The tree-sitter Python grammar lays out `import_from_statement` as:
///   from <dotted_name|relative_import> import <dotted_name>...
///   from <dotted_name|relative_import> import ( <dotted_name>, ... )
///
/// The module is the first `dotted_name` or `relative_import` child.
/// Imported names are `dotted_name` (or `aliased_import`) children that
/// appear *after* the `import` keyword node.
fn extract_python_from_parts(
    node: &tree_sitter::Node,
    source: &str,
) -> (String, Vec<String>) {
    let mut module = String::new();
    let mut names = Vec::new();
    let mut past_import_keyword = false;
    let mut found_module = false;

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "from" => {}
            "import" => {
                past_import_keyword = true;
            }
            "relative_import" if !found_module => {
                module = source[child.byte_range()].to_string();
                found_module = true;
            }
            "dotted_name" if !past_import_keyword && !found_module => {
                module = source[child.byte_range()].to_string();
                found_module = true;
            }
            "dotted_name" if past_import_keyword => {
                names.push(source[child.byte_range()].to_string());
            }
            "aliased_import" if past_import_keyword => {
                // `from x import foo as bar` — use original name
                let mut ac = child.walk();
                let result = child.children(&mut ac)
                    .find(|c| c.kind() == "dotted_name" || c.kind() == "identifier")
                    .map(|n| source[n.byte_range()].to_string())
                    .unwrap_or_default();
                if !result.is_empty() {
                    names.push(result);
                }
            }
            "wildcard_import" if past_import_keyword => {
                names.push("*".to_string());
            }
            _ => {}
        }
    }
    (module, names)
}

// ---------------------------------------------------------------------------
// JavaScript / TypeScript
// ---------------------------------------------------------------------------

fn extract_js_imports(
    node: &tree_sitter::Node,
    source: &str,
    file_path: &str,
    imports: &mut Vec<Import>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "import_statement" => {
                if let Some(imp) = parse_js_import_statement(&child, source, file_path) {
                    imports.push(imp);
                }
            }
            "expression_statement" | "lexical_declaration" | "variable_declaration" => {
                // Look for `require(...)` patterns
                extract_require_calls(&child, source, file_path, imports);
                extract_js_imports(&child, source, file_path, imports);
            }
            _ => {
                extract_js_imports(&child, source, file_path, imports);
            }
        }
    }
}

fn parse_js_import_statement(
    node: &tree_sitter::Node,
    source: &str,
    file_path: &str,
) -> Option<Import> {
    let line = node.start_position().row + 1;

    // Find the source string (the `from '...'` part)
    let target_module = find_js_import_source(node, source)?;

    // Collect imported names
    let mut names = Vec::new();
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "import_clause" => {
                collect_js_import_names(&child, source, &mut names);
            }
            // Some tree-sitter grammars put specifiers directly under import_statement
            "named_imports" => {
                collect_named_imports(&child, source, &mut names);
            }
            "identifier" => {
                // default import: `import foo from '...'`
                let name = &source[child.byte_range()];
                if name != "import" && name != "from" {
                    names.push(name.to_string());
                }
            }
            "namespace_import" => {
                names.push("*".to_string());
            }
            _ => {}
        }
    }

    Some(Import {
        source_file: file_path.to_string(),
        target_module,
        names,
        line,
    })
}

fn find_js_import_source(node: &tree_sitter::Node, source: &str) -> Option<String> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "string" {
            let raw = &source[child.byte_range()];
            // Strip quotes
            return Some(strip_quotes(raw));
        }
        // Recurse into import_clause etc. — but the source string is always
        // a direct child of the import_statement in the TS/JS grammars.
    }
    None
}

fn collect_js_import_names(
    node: &tree_sitter::Node,
    source: &str,
    names: &mut Vec<String>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "identifier" => {
                // default import
                names.push(source[child.byte_range()].to_string());
            }
            "named_imports" => {
                collect_named_imports(&child, source, names);
            }
            "namespace_import" => {
                names.push("*".to_string());
            }
            _ => {
                collect_js_import_names(&child, source, names);
            }
        }
    }
}

fn collect_named_imports(
    node: &tree_sitter::Node,
    source: &str,
    names: &mut Vec<String>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "import_specifier" {
            // The first child named "name" or the first identifier
            if let Some(name_node) = child.child_by_field_name("name") {
                names.push(source[name_node.byte_range()].to_string());
            } else {
                // Fallback: first identifier child
                let mut ic = child.walk();
                for c in child.children(&mut ic) {
                    if c.kind() == "identifier" {
                        names.push(source[c.byte_range()].to_string());
                        break;
                    }
                }
            }
        }
    }
}

fn extract_require_calls(
    node: &tree_sitter::Node,
    source: &str,
    file_path: &str,
    imports: &mut Vec<Import>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "call_expression" {
            if let Some(func) = child.child_by_field_name("function") {
                if func.kind() == "identifier" && &source[func.byte_range()] == "require" {
                    if let Some(args) = child.child_by_field_name("arguments") {
                        let mut ac = args.walk();
                        for arg in args.children(&mut ac) {
                            if arg.kind() == "string" {
                                let raw = &source[arg.byte_range()];
                                imports.push(Import {
                                    source_file: file_path.to_string(),
                                    target_module: strip_quotes(raw),
                                    names: Vec::new(),
                                    line: child.start_position().row + 1,
                                });
                                break;
                            }
                        }
                    }
                }
            }
        }
        // Recurse to find nested require() calls (e.g. inside variable_declarator)
        extract_require_calls(&child, source, file_path, imports);
    }
}

fn strip_quotes(s: &str) -> String {
    let s = s.trim();
    if (s.starts_with('"') && s.ends_with('"'))
        || (s.starts_with('\'') && s.ends_with('\''))
        || (s.starts_with('`') && s.ends_with('`'))
    {
        s[1..s.len() - 1].to_string()
    } else {
        s.to_string()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use tree_sitter::Parser;

    fn parse_python(source: &str) -> (tree_sitter::Tree, String) {
        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_python::LANGUAGE.into())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        (tree, source.to_string())
    }

    fn parse_js(source: &str) -> (tree_sitter::Tree, String) {
        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_javascript::LANGUAGE.into())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        (tree, source.to_string())
    }

    fn parse_ts(source: &str) -> (tree_sitter::Tree, String) {
        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        (tree, source.to_string())
    }

    #[test]
    fn test_python_import() {
        let (tree, src) = parse_python("import foo.bar\n");
        let imports = extract_imports(&tree.root_node(), &src, "test.py", "python");
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].target_module, "foo.bar");
        assert!(imports[0].names.is_empty());
        assert_eq!(imports[0].line, 1);
    }

    #[test]
    fn test_python_from_import() {
        let (tree, src) = parse_python("from foo.bar import baz, qux\n");
        let imports = extract_imports(&tree.root_node(), &src, "test.py", "python");
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].target_module, "foo.bar");
        assert_eq!(imports[0].names, vec!["baz", "qux"]);
    }

    #[test]
    fn test_python_relative_import() {
        let (tree, src) = parse_python("from . import models\n");
        let imports = extract_imports(&tree.root_node(), &src, "test.py", "python");
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].target_module, ".");
        assert_eq!(imports[0].names, vec!["models"]);
    }

    #[test]
    fn test_python_relative_dotted_import() {
        let (tree, src) = parse_python("from .auth import tokens\n");
        let imports = extract_imports(&tree.root_node(), &src, "test.py", "python");
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].target_module, ".auth");
        assert_eq!(imports[0].names, vec!["tokens"]);
    }

    #[test]
    fn test_js_import_destructured() {
        let (tree, src) = parse_js("import { foo, bar } from './module';\n");
        let imports = extract_imports(&tree.root_node(), &src, "test.js", "javascript");
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].target_module, "./module");
        assert_eq!(imports[0].names, vec!["foo", "bar"]);
    }

    #[test]
    fn test_js_import_default() {
        let (tree, src) = parse_js("import foo from './module';\n");
        let imports = extract_imports(&tree.root_node(), &src, "test.js", "javascript");
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].target_module, "./module");
        assert_eq!(imports[0].names, vec!["foo"]);
    }

    #[test]
    fn test_js_require() {
        let (tree, src) = parse_js("const foo = require('./module');\n");
        let imports = extract_imports(&tree.root_node(), &src, "test.js", "javascript");
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].target_module, "./module");
        assert!(imports[0].names.is_empty());
    }

    #[test]
    fn test_ts_import() {
        let (tree, src) = parse_ts("import { Foo } from './types';\n");
        let imports = extract_imports(&tree.root_node(), &src, "test.ts", "typescript");
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].target_module, "./types");
        assert_eq!(imports[0].names, vec!["Foo"]);
    }

    #[test]
    fn test_js_namespace_import() {
        let (tree, src) = parse_js("import * as foo from './module';\n");
        let imports = extract_imports(&tree.root_node(), &src, "test.js", "javascript");
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].target_module, "./module");
        assert_eq!(imports[0].names, vec!["*"]);
    }

    #[test]
    fn test_empty_file() {
        let (tree, src) = parse_python("");
        let imports = extract_imports(&tree.root_node(), &src, "empty.py", "python");
        assert!(imports.is_empty());
    }

    #[test]
    fn test_multiple_python_imports() {
        let code = "import os\nimport sys\nfrom pathlib import Path\n";
        let (tree, src) = parse_python(code);
        let imports = extract_imports(&tree.root_node(), &src, "test.py", "python");
        assert_eq!(imports.len(), 3);
        assert_eq!(imports[0].target_module, "os");
        assert_eq!(imports[1].target_module, "sys");
        assert_eq!(imports[2].target_module, "pathlib");
        assert_eq!(imports[2].names, vec!["Path"]);
    }
}
