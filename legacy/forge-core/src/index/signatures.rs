use serde::{Serialize, Deserialize};
use std::collections::HashMap;
use std::path::Path;

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct FunctionSig {
    pub name: String,
    pub params: Vec<String>,
    pub param_count: usize,
    pub line: usize,
}

/// Type alias for the on-disk signature cache: file path -> signatures.
pub type SignatureCache = HashMap<String, Vec<FunctionSig>>;

/// Extract function/method signatures from a tree-sitter parse tree.
///
/// Supports Python (`def`), JavaScript (`function`, arrow, `const f = ...`),
/// and TypeScript (same as JS plus type annotations).
pub fn extract_signatures(
    root: &tree_sitter::Node,
    source: &str,
    language: &str,
) -> Vec<FunctionSig> {
    let mut sigs = Vec::new();
    match language {
        "python" => walk_python(root, source, &mut sigs),
        "javascript" | "typescript" => walk_js(root, source, &mut sigs),
        _ => {}
    }
    sigs
}

// ---------------------------------------------------------------------------
// Python
// ---------------------------------------------------------------------------

fn walk_python(node: &tree_sitter::Node, source: &str, sigs: &mut Vec<FunctionSig>) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "function_definition" {
            if let Some(sig) = extract_python_fn(&child, source) {
                sigs.push(sig);
            }
        }
        // Recurse into class bodies, nested scopes, etc.
        walk_python(&child, source, sigs);
    }
}

fn extract_python_fn(node: &tree_sitter::Node, source: &str) -> Option<FunctionSig> {
    let name_node = node.child_by_field_name("name")?;
    let name = source[name_node.byte_range()].to_string();
    let line = node.start_position().row + 1;

    let params_node = node.child_by_field_name("parameters")?;
    let params = extract_python_params(&params_node, source);
    let param_count = params.len();

    Some(FunctionSig { name, params, param_count, line })
}

/// Extract parameter names from a Python `parameters` node.
///
/// Handles: `identifier`, `typed_parameter`, `default_parameter`,
/// `typed_default_parameter`, `list_splat_pattern`, `dictionary_splat_pattern`.
/// Skips `(` and `)` punctuation and `,` separators.
fn extract_python_params(node: &tree_sitter::Node, source: &str) -> Vec<String> {
    let mut params = Vec::new();
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "identifier" => {
                params.push(source[child.byte_range()].to_string());
            }
            "typed_parameter" | "default_parameter" | "typed_default_parameter" => {
                // The parameter name is the first identifier child (or the
                // `name` field if available).
                if let Some(name) = python_param_name(&child, source) {
                    params.push(name);
                }
            }
            "list_splat_pattern" | "dictionary_splat_pattern" => {
                // *args, **kwargs — name is the identifier child
                let mut ic = child.walk();
                for c in child.children(&mut ic) {
                    if c.kind() == "identifier" {
                        params.push(source[c.byte_range()].to_string());
                        break;
                    }
                }
            }
            _ => {}
        }
    }
    params
}

/// Get the name of a Python parameter from typed_parameter, default_parameter, etc.
fn python_param_name(node: &tree_sitter::Node, source: &str) -> Option<String> {
    // Try `name` field first (typed_parameter has it)
    if let Some(n) = node.child_by_field_name("name") {
        return Some(source[n.byte_range()].to_string());
    }
    // Fallback: first identifier child
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "identifier" {
            return Some(source[child.byte_range()].to_string());
        }
        // default_parameter wraps typed_parameter
        if child.kind() == "typed_parameter" {
            return python_param_name(&child, source);
        }
    }
    None
}

// ---------------------------------------------------------------------------
// JavaScript / TypeScript
// ---------------------------------------------------------------------------

fn walk_js(node: &tree_sitter::Node, source: &str, sigs: &mut Vec<FunctionSig>) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "function_declaration" => {
                if let Some(sig) = extract_js_fn_decl(&child, source) {
                    sigs.push(sig);
                }
            }
            "lexical_declaration" | "variable_declaration" => {
                // `const foo = (x) => ...` or `const foo = function(x) { ... }`
                extract_js_var_fn(&child, source, sigs);
            }
            "export_statement" => {
                // `export function ...` or `export const ...`
                walk_js(&child, source, sigs);
            }
            _ => {}
        }
        // Always recurse for nested functions (inside class bodies, etc.)
        walk_js(&child, source, sigs);
    }
}

fn extract_js_fn_decl(node: &tree_sitter::Node, source: &str) -> Option<FunctionSig> {
    let name_node = node.child_by_field_name("name")?;
    let name = source[name_node.byte_range()].to_string();
    let line = node.start_position().row + 1;

    let params_node = node.child_by_field_name("parameters")?;
    let params = extract_js_params(&params_node, source);
    let param_count = params.len();

    Some(FunctionSig { name, params, param_count, line })
}

fn extract_js_var_fn(
    node: &tree_sitter::Node,
    source: &str,
    sigs: &mut Vec<FunctionSig>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "variable_declarator" {
            let name = match child.child_by_field_name("name") {
                Some(n) if n.kind() == "identifier" => source[n.byte_range()].to_string(),
                _ => continue,
            };
            let value = match child.child_by_field_name("value") {
                Some(v) => v,
                None => continue,
            };
            match value.kind() {
                "arrow_function" | "function_expression" | "function" => {
                    let params_node = match value.child_by_field_name("parameters") {
                        Some(p) => p,
                        None => {
                            // Arrow function with single param: `const f = x => ...`
                            // The parameter is the `parameter` field or first identifier
                            if let Some(p) = value.child_by_field_name("parameter") {
                                let pname = source[p.byte_range()].to_string();
                                sigs.push(FunctionSig {
                                    name,
                                    params: vec![pname.clone()],
                                    param_count: 1,
                                    line: child.start_position().row + 1,
                                });
                            }
                            continue;
                        }
                    };
                    let params = extract_js_params(&params_node, source);
                    let param_count = params.len();
                    sigs.push(FunctionSig {
                        name,
                        params,
                        param_count,
                        line: child.start_position().row + 1,
                    });
                }
                _ => {}
            }
        }
    }
}

/// Extract parameter names from JS/TS `formal_parameters`.
///
/// Handles: `identifier`, `required_parameter` (TS), `optional_parameter` (TS),
/// `assignment_pattern` (default values), `rest_pattern`.
fn extract_js_params(node: &tree_sitter::Node, source: &str) -> Vec<String> {
    let mut params = Vec::new();
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "identifier" => {
                params.push(source[child.byte_range()].to_string());
            }
            "required_parameter" | "optional_parameter" => {
                // TS: `required_parameter` has a `pattern` field (identifier)
                if let Some(pat) = child.child_by_field_name("pattern") {
                    if pat.kind() == "identifier" {
                        params.push(source[pat.byte_range()].to_string());
                    }
                } else {
                    // Fallback: first identifier
                    let mut ic = child.walk();
                    for c in child.children(&mut ic) {
                        if c.kind() == "identifier" {
                            params.push(source[c.byte_range()].to_string());
                            break;
                        }
                    }
                }
            }
            "assignment_pattern" => {
                // Default param: `x = 5`
                if let Some(left) = child.child_by_field_name("left") {
                    if left.kind() == "identifier" {
                        params.push(source[left.byte_range()].to_string());
                    }
                }
            }
            "rest_pattern" => {
                // `...args`
                let mut ic = child.walk();
                for c in child.children(&mut ic) {
                    if c.kind() == "identifier" {
                        params.push(source[c.byte_range()].to_string());
                        break;
                    }
                }
            }
            _ => {}
        }
    }
    params
}

// ---------------------------------------------------------------------------
// Cache
// ---------------------------------------------------------------------------

const CACHE_FILENAME: &str = "signatures.json";
const CACHE_SUBDIR: &str = "index";

fn cache_path(state_dir: &str) -> std::path::PathBuf {
    Path::new(state_dir).join(CACHE_SUBDIR).join(CACHE_FILENAME)
}

/// Read the signature cache from `$STATE_DIR/index/signatures.json`.
/// Returns an empty map if the file does not exist or is invalid.
pub fn read_cache(state_dir: &str) -> SignatureCache {
    let path = cache_path(state_dir);
    match std::fs::read_to_string(&path) {
        Ok(contents) => serde_json::from_str(&contents).unwrap_or_default(),
        Err(_) => HashMap::new(),
    }
}

/// Write the signature cache atomically (write to tmp, then rename).
pub fn write_cache(state_dir: &str, cache: &SignatureCache) {
    let path = cache_path(state_dir);
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let tmp = path.with_extension("json.tmp");
    if let Ok(json) = serde_json::to_string_pretty(cache) {
        if std::fs::write(&tmp, json).is_ok() {
            let _ = std::fs::rename(&tmp, &path);
        }
    }
}

/// Diff two signature lists (from the same file).
///
/// Returns `(added, removed, changed)` where:
/// - `added` = in `new` but not `old` (by name)
/// - `removed` = in `old` but not `new` (by name)
/// - `changed` = same name, different `param_count`
///
/// Each tuple in `changed` is `(old_sig, new_sig)`.
pub fn diff_signatures(
    old: &[FunctionSig],
    new: &[FunctionSig],
) -> (Vec<FunctionSig>, Vec<FunctionSig>, Vec<(FunctionSig, FunctionSig)>) {
    let old_map: HashMap<&str, &FunctionSig> =
        old.iter().map(|s| (s.name.as_str(), s)).collect();
    let new_map: HashMap<&str, &FunctionSig> =
        new.iter().map(|s| (s.name.as_str(), s)).collect();

    let added: Vec<FunctionSig> = new.iter()
        .filter(|s| !old_map.contains_key(s.name.as_str()))
        .cloned()
        .collect();

    let removed: Vec<FunctionSig> = old.iter()
        .filter(|s| !new_map.contains_key(s.name.as_str()))
        .cloned()
        .collect();

    let changed: Vec<(FunctionSig, FunctionSig)> = new.iter()
        .filter_map(|new_sig| {
            old_map.get(new_sig.name.as_str()).and_then(|old_sig| {
                if old_sig.param_count != new_sig.param_count {
                    Some(((*old_sig).clone(), new_sig.clone()))
                } else {
                    None
                }
            })
        })
        .collect();

    (added, removed, changed)
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
        parser.set_language(&tree_sitter_python::LANGUAGE.into()).unwrap();
        let tree = parser.parse(source, None).unwrap();
        (tree, source.to_string())
    }

    fn parse_js(source: &str) -> (tree_sitter::Tree, String) {
        let mut parser = Parser::new();
        parser.set_language(&tree_sitter_javascript::LANGUAGE.into()).unwrap();
        let tree = parser.parse(source, None).unwrap();
        (tree, source.to_string())
    }

    fn parse_ts(source: &str) -> (tree_sitter::Tree, String) {
        let mut parser = Parser::new();
        parser.set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()).unwrap();
        let tree = parser.parse(source, None).unwrap();
        (tree, source.to_string())
    }

    // ---- Python ----

    #[test]
    fn test_python_simple_params() {
        let (tree, src) = parse_python("def foo(self, x, y):\n    pass\n");
        let sigs = extract_signatures(&tree.root_node(), &src, "python");
        assert_eq!(sigs.len(), 1);
        assert_eq!(sigs[0].name, "foo");
        assert_eq!(sigs[0].params, vec!["self", "x", "y"]);
        assert_eq!(sigs[0].param_count, 3);
        assert_eq!(sigs[0].line, 1);
    }

    #[test]
    fn test_python_typed_params() {
        let (tree, src) = parse_python("def bar(a: int, b: str = \"\"):\n    pass\n");
        let sigs = extract_signatures(&tree.root_node(), &src, "python");
        assert_eq!(sigs.len(), 1);
        assert_eq!(sigs[0].name, "bar");
        assert_eq!(sigs[0].params, vec!["a", "b"]);
        assert_eq!(sigs[0].param_count, 2);
    }

    #[test]
    fn test_python_default_param() {
        let (tree, src) = parse_python("def baz(x, y=None):\n    pass\n");
        let sigs = extract_signatures(&tree.root_node(), &src, "python");
        assert_eq!(sigs.len(), 1);
        assert_eq!(sigs[0].params, vec!["x", "y"]);
    }

    #[test]
    fn test_python_no_params() {
        let (tree, src) = parse_python("def noop():\n    pass\n");
        let sigs = extract_signatures(&tree.root_node(), &src, "python");
        assert_eq!(sigs.len(), 1);
        assert_eq!(sigs[0].params, Vec::<String>::new());
        assert_eq!(sigs[0].param_count, 0);
    }

    // ---- JavaScript ----

    #[test]
    fn test_js_function_declaration() {
        let (tree, src) = parse_js("function foo(x, y) {}\n");
        let sigs = extract_signatures(&tree.root_node(), &src, "javascript");
        assert_eq!(sigs.len(), 1);
        assert_eq!(sigs[0].name, "foo");
        assert_eq!(sigs[0].params, vec!["x", "y"]);
        assert_eq!(sigs[0].param_count, 2);
    }

    #[test]
    fn test_js_arrow_function() {
        let (tree, src) = parse_js("const foo = (x, y) => {};\n");
        let sigs = extract_signatures(&tree.root_node(), &src, "javascript");
        assert_eq!(sigs.len(), 1);
        assert_eq!(sigs[0].name, "foo");
        assert_eq!(sigs[0].params, vec!["x", "y"]);
    }

    #[test]
    fn test_js_default_params() {
        let (tree, src) = parse_js("function greet(name, greeting = 'hi') {}\n");
        let sigs = extract_signatures(&tree.root_node(), &src, "javascript");
        assert_eq!(sigs.len(), 1);
        assert_eq!(sigs[0].params, vec!["name", "greeting"]);
    }

    // ---- TypeScript ----

    #[test]
    fn test_ts_typed_params() {
        let (tree, src) = parse_ts("function add(a: number, b: number): number { return a + b; }\n");
        let sigs = extract_signatures(&tree.root_node(), &src, "typescript");
        assert_eq!(sigs.len(), 1);
        assert_eq!(sigs[0].name, "add");
        assert_eq!(sigs[0].params, vec!["a", "b"]);
        assert_eq!(sigs[0].param_count, 2);
    }

    #[test]
    fn test_ts_arrow_typed() {
        let (tree, src) = parse_ts("const foo = (x: string, y: number) => {};\n");
        let sigs = extract_signatures(&tree.root_node(), &src, "typescript");
        assert_eq!(sigs.len(), 1);
        assert_eq!(sigs[0].name, "foo");
        assert_eq!(sigs[0].params, vec!["x", "y"]);
        assert_eq!(sigs[0].param_count, 2);
    }

    // ---- Cache ----

    #[test]
    fn test_cache_read_write() {
        let dir = tempfile::tempdir().unwrap();
        let state_dir = dir.path().to_str().unwrap();

        let mut cache = SignatureCache::new();
        cache.insert("foo.py".into(), vec![
            FunctionSig {
                name: "hello".into(),
                params: vec!["x".into()],
                param_count: 1,
                line: 1,
            },
        ]);

        write_cache(state_dir, &cache);
        let loaded = read_cache(state_dir);
        assert_eq!(loaded, cache);
    }

    #[test]
    fn test_cache_read_missing() {
        let dir = tempfile::tempdir().unwrap();
        let state_dir = dir.path().to_str().unwrap();
        let loaded = read_cache(state_dir);
        assert!(loaded.is_empty());
    }

    // ---- Diff ----

    #[test]
    fn test_diff_added() {
        let old: Vec<FunctionSig> = vec![];
        let new = vec![FunctionSig {
            name: "foo".into(), params: vec!["x".into()], param_count: 1, line: 1,
        }];
        let (added, removed, changed) = diff_signatures(&old, &new);
        assert_eq!(added.len(), 1);
        assert_eq!(added[0].name, "foo");
        assert!(removed.is_empty());
        assert!(changed.is_empty());
    }

    #[test]
    fn test_diff_removed() {
        let old = vec![FunctionSig {
            name: "bar".into(), params: vec![], param_count: 0, line: 5,
        }];
        let new: Vec<FunctionSig> = vec![];
        let (added, removed, changed) = diff_signatures(&old, &new);
        assert!(added.is_empty());
        assert_eq!(removed.len(), 1);
        assert_eq!(removed[0].name, "bar");
        assert!(changed.is_empty());
    }

    #[test]
    fn test_diff_changed() {
        let old = vec![FunctionSig {
            name: "baz".into(), params: vec!["a".into()], param_count: 1, line: 1,
        }];
        let new = vec![FunctionSig {
            name: "baz".into(), params: vec!["a".into(), "b".into()], param_count: 2, line: 1,
        }];
        let (added, removed, changed) = diff_signatures(&old, &new);
        assert!(added.is_empty());
        assert!(removed.is_empty());
        assert_eq!(changed.len(), 1);
        assert_eq!(changed[0].0.param_count, 1);
        assert_eq!(changed[0].1.param_count, 2);
    }

    #[test]
    fn test_diff_unchanged() {
        let old = vec![FunctionSig {
            name: "same".into(), params: vec!["x".into()], param_count: 1, line: 1,
        }];
        let new = vec![FunctionSig {
            name: "same".into(), params: vec!["x".into()], param_count: 1, line: 3,
        }];
        let (added, removed, changed) = diff_signatures(&old, &new);
        assert!(added.is_empty());
        assert!(removed.is_empty());
        assert!(changed.is_empty());
    }
}
