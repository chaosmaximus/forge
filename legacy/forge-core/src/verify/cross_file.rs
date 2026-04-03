//! Cross-file breakage detection via signature diff + import graph.
//!
//! After an edit, this module detects when a function signature change or removal
//! could break other files that import the changed function.

use crate::index::imports::Import;
use crate::index::signatures::{
    diff_signatures, extract_signatures, read_cache, write_cache, FunctionSig,
};
use serde::Serialize;
use std::path::Path;
use tree_sitter::Parser;

#[derive(Serialize, Debug, Clone)]
pub struct Breakage {
    pub file: String,
    pub function: String,
    pub change_type: String, // "signature_changed", "function_removed"
    pub old_params: usize,
    pub new_params: usize,
    pub affected_files: Vec<String>,
    pub severity: String, // "error"
}

/// Read the import cache from `$STATE/index/imports.json`.
/// Returns an empty vec if the file does not exist or is invalid.
fn read_import_cache(state_dir: &str) -> Vec<Import> {
    let path = Path::new(state_dir).join("index").join("imports.json");
    std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

/// Find files that import a given function name from the edited file.
///
/// This checks imports where:
/// - The target module path plausibly resolves to `file_path`
/// - The imported names include the function name, or it's a wildcard/bare import
fn find_importers(
    imports: &[Import],
    file_path: &str,
    function_name: &str,
) -> Vec<String> {
    let file_stem = Path::new(file_path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("");

    let mut importers = Vec::new();

    for imp in imports {
        // Skip self-imports
        if imp.source_file == file_path {
            continue;
        }

        // Check if the target module plausibly resolves to our file
        let target = &imp.target_module;
        let module_matches = target == file_stem
            || target.ends_with(&format!(".{}", file_stem))
            || target.ends_with(&format!("/{}", file_stem))
            || target.contains(file_stem);

        if !module_matches {
            continue;
        }

        // Check if the function is explicitly imported, or it's a wildcard/bare import
        let imports_function = imp.names.is_empty() // bare import (e.g. `import module`)
            || imp.names.iter().any(|n| n == function_name || n == "*");

        if imports_function {
            importers.push(imp.source_file.clone());
        }
    }

    importers.sort();
    importers.dedup();
    importers
}

/// Check a file for cross-file breakage after an edit.
///
/// Returns breakage alerts for any changed/removed function signatures
/// that are imported by other files.
pub fn check_file(
    file_path: &str,
    content: &str,
    language: &str,
    state_dir: &str,
) -> Vec<Breakage> {
    // 1. Parse the file with tree-sitter -> extract current signatures
    let new_sigs = match parse_and_extract(content, language) {
        Some(sigs) => sigs,
        None => return Vec::new(),
    };

    // 2. Read old signatures from cache
    let cache = read_cache(state_dir);
    let old_sigs = cache.get(file_path).cloned().unwrap_or_default();

    // If no cache existed, store the current state and return (no breakage on first index)
    if old_sigs.is_empty() && !cache.contains_key(file_path) {
        let mut new_cache = cache;
        new_cache.insert(file_path.to_string(), new_sigs);
        write_cache(state_dir, &new_cache);
        return Vec::new();
    }

    // 3. Diff against cached signatures
    let (_added, removed, changed) = diff_signatures(&old_sigs, &new_sigs);

    // If nothing changed or removed, no breakage
    if removed.is_empty() && changed.is_empty() {
        // Still update cache
        let mut new_cache = cache;
        new_cache.insert(file_path.to_string(), new_sigs);
        write_cache(state_dir, &new_cache);
        return Vec::new();
    }

    // 4. Read import cache and find affected files
    let imports = read_import_cache(state_dir);
    let mut breakages = Vec::new();

    for sig in &removed {
        let affected = find_importers(&imports, file_path, &sig.name);
        if !affected.is_empty() {
            breakages.push(Breakage {
                file: file_path.to_string(),
                function: sig.name.clone(),
                change_type: "function_removed".to_string(),
                old_params: sig.param_count,
                new_params: 0,
                affected_files: affected,
                severity: "error".to_string(),
            });
        }
    }

    for (old_sig, new_sig) in &changed {
        let affected = find_importers(&imports, file_path, &new_sig.name);
        if !affected.is_empty() {
            breakages.push(Breakage {
                file: file_path.to_string(),
                function: new_sig.name.clone(),
                change_type: "signature_changed".to_string(),
                old_params: old_sig.param_count,
                new_params: new_sig.param_count,
                affected_files: affected,
                severity: "error".to_string(),
            });
        }
    }

    // 6. Update the signature cache with new signatures
    let mut new_cache = cache;
    new_cache.insert(file_path.to_string(), new_sigs);
    write_cache(state_dir, &new_cache);

    breakages
}

/// Parse source with tree-sitter and extract function signatures.
fn parse_and_extract(content: &str, language: &str) -> Option<Vec<FunctionSig>> {
    let ts_language = match language {
        "python" => tree_sitter_python::LANGUAGE.into(),
        "javascript" => tree_sitter_javascript::LANGUAGE.into(),
        "typescript" => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        _ => return None,
    };

    let mut parser = Parser::new();
    parser.set_language(&ts_language).ok()?;
    let tree = parser.parse(content, None)?;
    Some(extract_signatures(&tree.root_node(), content, language))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::signatures::{write_cache, SignatureCache};

    /// Helper to set up a temp state dir with a signature cache and import cache.
    fn setup_state(
        old_sigs: Vec<FunctionSig>,
        imports: Vec<Import>,
    ) -> (tempfile::TempDir, String) {
        let dir = tempfile::tempdir().unwrap();
        let state_dir = dir.path().to_str().unwrap().to_string();

        // Write signature cache
        let mut cache = SignatureCache::new();
        cache.insert("utils.py".to_string(), old_sigs);
        write_cache(&state_dir, &cache);

        // Write import cache
        let index_dir = dir.path().join("index");
        std::fs::create_dir_all(&index_dir).unwrap();
        let import_json = serde_json::to_string(&imports).unwrap();
        std::fs::write(index_dir.join("imports.json"), import_json).unwrap();

        (dir, state_dir)
    }

    #[test]
    fn test_signature_changed_breakage() {
        let old_sigs = vec![FunctionSig {
            name: "process".into(),
            params: vec!["data".into()],
            param_count: 1,
            line: 1,
        }];
        let imports = vec![Import {
            source_file: "main.py".into(),
            target_module: "utils".into(),
            names: vec!["process".into()],
            line: 1,
        }];
        let (_dir, state_dir) = setup_state(old_sigs, imports);

        let new_content = "def process(data, mode):\n    pass\n";
        let breakages = check_file("utils.py", new_content, "python", &state_dir);

        assert_eq!(breakages.len(), 1);
        assert_eq!(breakages[0].function, "process");
        assert_eq!(breakages[0].change_type, "signature_changed");
        assert_eq!(breakages[0].old_params, 1);
        assert_eq!(breakages[0].new_params, 2);
        assert_eq!(breakages[0].affected_files, vec!["main.py"]);
        assert_eq!(breakages[0].severity, "error");
    }

    #[test]
    fn test_function_removed_breakage() {
        let old_sigs = vec![FunctionSig {
            name: "helper".into(),
            params: vec!["x".into()],
            param_count: 1,
            line: 5,
        }];
        let imports = vec![Import {
            source_file: "app.py".into(),
            target_module: "utils".into(),
            names: vec!["helper".into()],
            line: 2,
        }];
        let (_dir, state_dir) = setup_state(old_sigs, imports);

        // New content has no functions at all
        let new_content = "# empty module\nVAR = 42\n";
        let breakages = check_file("utils.py", new_content, "python", &state_dir);

        assert_eq!(breakages.len(), 1);
        assert_eq!(breakages[0].function, "helper");
        assert_eq!(breakages[0].change_type, "function_removed");
        assert_eq!(breakages[0].old_params, 1);
        assert_eq!(breakages[0].new_params, 0);
        assert_eq!(breakages[0].affected_files, vec!["app.py"]);
    }

    #[test]
    fn test_internal_function_changed_no_breakage() {
        // _internal is not imported by anyone
        let old_sigs = vec![FunctionSig {
            name: "_internal".into(),
            params: vec!["x".into()],
            param_count: 1,
            line: 1,
        }];
        let imports = vec![Import {
            source_file: "main.py".into(),
            target_module: "utils".into(),
            names: vec!["process".into()], // imports process, not _internal
            line: 1,
        }];
        let (_dir, state_dir) = setup_state(old_sigs, imports);

        let new_content = "def _internal(x, y, z):\n    pass\n";
        let breakages = check_file("utils.py", new_content, "python", &state_dir);

        assert!(breakages.is_empty(), "Internal function change should not produce breakage");
    }

    #[test]
    fn test_no_cache_exists_no_breakage() {
        let dir = tempfile::tempdir().unwrap();
        let state_dir = dir.path().to_str().unwrap();

        let content = "def foo(x, y):\n    pass\n";
        let breakages = check_file("brand_new.py", content, "python", state_dir);

        assert!(breakages.is_empty(), "First-time index should not report breakage");

        // Verify cache was created
        let cache = read_cache(state_dir);
        assert!(cache.contains_key("brand_new.py"));
    }

    #[test]
    fn test_empty_file_no_breakage() {
        let old_sigs = vec![FunctionSig {
            name: "foo".into(),
            params: vec!["x".into()],
            param_count: 1,
            line: 1,
        }];
        // No one imports foo
        let imports: Vec<Import> = vec![];
        let (_dir, state_dir) = setup_state(old_sigs, imports);

        let breakages = check_file("utils.py", "", "python", &state_dir);

        // foo was removed but not imported by anyone => no breakage
        assert!(breakages.is_empty(), "Removed function with no importers should not produce breakage");
    }

    #[test]
    fn test_multiple_affected_files() {
        let old_sigs = vec![FunctionSig {
            name: "shared".into(),
            params: vec!["a".into()],
            param_count: 1,
            line: 1,
        }];
        let imports = vec![
            Import {
                source_file: "a.py".into(),
                target_module: "utils".into(),
                names: vec!["shared".into()],
                line: 1,
            },
            Import {
                source_file: "b.py".into(),
                target_module: "utils".into(),
                names: vec!["shared".into()],
                line: 1,
            },
            Import {
                source_file: "c.py".into(),
                target_module: "utils".into(),
                names: vec!["other".into()], // does NOT import 'shared'
                line: 1,
            },
        ];
        let (_dir, state_dir) = setup_state(old_sigs, imports);

        let new_content = "def shared(a, b, c):\n    pass\n";
        let breakages = check_file("utils.py", new_content, "python", &state_dir);

        assert_eq!(breakages.len(), 1);
        assert_eq!(breakages[0].affected_files, vec!["a.py", "b.py"]);
    }

    #[test]
    fn test_wildcard_import_affected() {
        let old_sigs = vec![FunctionSig {
            name: "do_stuff".into(),
            params: vec!["x".into()],
            param_count: 1,
            line: 1,
        }];
        let imports = vec![Import {
            source_file: "consumer.py".into(),
            target_module: "utils".into(),
            names: vec!["*".into()], // wildcard import
            line: 1,
        }];
        let (_dir, state_dir) = setup_state(old_sigs, imports);

        let new_content = "def do_stuff(x, y):\n    pass\n";
        let breakages = check_file("utils.py", new_content, "python", &state_dir);

        assert_eq!(breakages.len(), 1);
        assert_eq!(breakages[0].affected_files, vec!["consumer.py"]);
    }

    #[test]
    fn test_read_import_cache_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        let imports = read_import_cache(dir.path().to_str().unwrap());
        assert!(imports.is_empty());
    }

    #[test]
    fn test_cache_updated_after_check() {
        let old_sigs = vec![FunctionSig {
            name: "foo".into(),
            params: vec!["x".into()],
            param_count: 1,
            line: 1,
        }];
        let imports: Vec<Import> = vec![];
        let (_dir, state_dir) = setup_state(old_sigs, imports);

        let new_content = "def foo(x, y):\n    pass\n";
        let _ = check_file("utils.py", new_content, "python", &state_dir);

        // Verify cache was updated
        let cache = read_cache(&state_dir);
        let sigs = cache.get("utils.py").unwrap();
        assert_eq!(sigs[0].param_count, 2);
    }
}
