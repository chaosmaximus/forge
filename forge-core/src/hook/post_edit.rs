use crate::scan::rules::RULES;
use serde_json::json;
use std::path::Path;
use tree_sitter::Parser;

pub fn run(file_path: &str) {
    let path = Path::new(file_path);

    // Symlink check
    if path
        .symlink_metadata()
        .map(|m| m.file_type().is_symlink())
        .unwrap_or(false)
    {
        return;
    }

    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return,
    };

    let mut alerts: Vec<String> = Vec::new();
    let mut context_parts: Vec<String> = Vec::new();

    // --- Layer 1: Secret scan ---
    for line in content.lines() {
        for rule in RULES.iter() {
            if rule.regex.is_match(line) {
                alerts.push(format!("{} detected.", rule.description));
                break; // One alert per line is enough
            }
        }
    }

    // --- Layer 2: Syntax validation via tree-sitter ---
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    let syntax_errors = check_syntax(ext, &content);
    if !syntax_errors.is_empty() {
        alerts.push(format!(
            "SYNTAX ERROR in {}: {}",
            file_path,
            syntax_errors.join("; ")
        ));
    }

    // --- Layer 4: Cross-file breakage detection ---
    let language = match ext {
        "py" | "pyi" => "python",
        "ts" | "tsx" => "typescript",
        "js" | "jsx" | "mjs" => "javascript",
        _ => "",
    };

    if !language.is_empty() {
        let state_dir =
            std::env::var("CLAUDE_PLUGIN_DATA").unwrap_or_else(|_| ".forge".to_string());
        let breakages =
            crate::verify::cross_file::check_file(file_path, &content, language, &state_dir);
        for b in &breakages {
            alerts.push(format!(
                "BREAKAGE: {}() signature changed ({} -> {} params). Affects: {}",
                b.function,
                b.old_params,
                b.new_params,
                b.affected_files.join(", ")
            ));
        }
    }

    // --- Layer 6: Per-file decision injection ---
    {
        let state_dir =
            std::env::var("CLAUDE_PLUGIN_DATA").unwrap_or_else(|_| ".forge".to_string());
        let decisions = read_decisions_for_file(&state_dir, file_path);
        for d in &decisions {
            context_parts.push(d.clone());
        }
    }

    if !alerts.is_empty() || !context_parts.is_empty() {
        let mut parts = Vec::new();
        if !alerts.is_empty() {
            parts.push(format!(
                "ALERT in {}: {} Consider reviewing before continuing.",
                file_path,
                alerts.join(" ")
            ));
        }
        if !context_parts.is_empty() {
            parts.push(format!("Context: {}", context_parts.join("; ")));
        }
        let output = json!({
            "hookSpecificOutput": {
                "additionalContext": parts.join("\n")
            }
        });
        println!("{}", output);
    }
}

/// Read decisions from the memory cache that are relevant to the given file.
///
/// Filters `$STATE/memory/cache.json` entries of type "decision" whose title
/// or content contains the filename or parent directory name.
fn read_decisions_for_file(state_dir: &str, file_path: &str) -> Vec<String> {
    let cache_path = Path::new(state_dir).join("memory").join("cache.json");
    let content = match std::fs::read_to_string(&cache_path) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };

    let cache: serde_json::Value = match serde_json::from_str(&content) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };

    let entries = match cache.get("entries").and_then(|e| e.as_array()) {
        Some(arr) => arr,
        None => return Vec::new(),
    };

    // Extract filename and parent dir name for matching
    let path = Path::new(file_path);
    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("");
    let parent_name = path
        .parent()
        .and_then(|p| p.file_name())
        .and_then(|n| n.to_str())
        .unwrap_or("");

    let mut decisions = Vec::new();

    for entry in entries {
        // Only look at decisions
        let entry_type = entry
            .get("type")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if entry_type != "decision" {
            continue;
        }

        let title = entry
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let content_str = entry
            .get("content")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        let matches_file = (!file_name.is_empty()
            && (title.contains(file_name) || content_str.contains(file_name)))
            || (!parent_name.is_empty()
                && (title.contains(parent_name) || content_str.contains(parent_name)));

        if matches_file {
            decisions.push(format!("[Decision] {}", title));
        }
    }

    decisions
}

/// Use tree-sitter to check for syntax errors. Returns a list of error descriptions.
fn check_syntax(ext: &str, source: &str) -> Vec<String> {
    let ts_language = match ext {
        "py" | "pyi" | "pyw" => Some(tree_sitter_python::LANGUAGE),
        "ts" | "tsx" => Some(tree_sitter_typescript::LANGUAGE_TYPESCRIPT),
        "js" | "jsx" | "mjs" | "cjs" => Some(tree_sitter_javascript::LANGUAGE),
        _ => None,
    };

    let ts_language = match ts_language {
        Some(lang) => lang,
        None => return Vec::new(),
    };

    let mut parser = Parser::new();
    if parser.set_language(&ts_language.into()).is_err() {
        return Vec::new();
    }

    let tree = match parser.parse(source, None) {
        Some(t) => t,
        None => return Vec::new(),
    };

    let root = tree.root_node();
    if !root.has_error() {
        return Vec::new();
    }

    // Collect up to 5 error nodes with their positions
    let mut errors = Vec::new();
    collect_errors(&root, source, &mut errors, 5);
    errors
}

fn collect_errors(
    node: &tree_sitter::Node,
    source: &str,
    errors: &mut Vec<String>,
    max_errors: usize,
) {
    if errors.len() >= max_errors {
        return;
    }

    if node.is_error() || node.is_missing() {
        let row = node.start_position().row + 1;
        let col = node.start_position().column + 1;
        let snippet = &source[node.byte_range()];
        let short_snippet = if snippet.len() > 40 {
            format!("{}...", &snippet[..40])
        } else {
            snippet.to_string()
        };

        if node.is_missing() {
            errors.push(format!("line {}:{} missing expected syntax", row, col));
        } else {
            let clean: String = short_snippet.chars().filter(|c| !c.is_control()).collect();
            errors.push(format!("line {}:{} parse error near '{}'", row, col, clean));
        }
        return; // Don't recurse into error nodes
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.has_error() || child.is_error() || child.is_missing() {
            collect_errors(&child, source, errors, max_errors);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_python_syntax() {
        let errors = check_syntax("py", "def hello():\n    print('hi')\n");
        assert!(errors.is_empty());
    }

    #[test]
    fn test_invalid_python_syntax() {
        let errors = check_syntax("py", "def hello(\n    print 'hi'\n");
        assert!(!errors.is_empty());
    }

    #[test]
    fn test_valid_js_syntax() {
        let errors = check_syntax("js", "function hello() { return 42; }\n");
        assert!(errors.is_empty());
    }

    #[test]
    fn test_invalid_js_syntax() {
        let errors = check_syntax("js", "function hello( { return 42; }\n");
        assert!(!errors.is_empty());
    }

    #[test]
    fn test_valid_ts_syntax() {
        let errors = check_syntax("ts", "const x: number = 42;\n");
        assert!(errors.is_empty());
    }

    #[test]
    fn test_unknown_extension() {
        let errors = check_syntax("txt", "random content");
        assert!(errors.is_empty());
    }

    #[test]
    fn test_empty_source() {
        let errors = check_syntax("py", "");
        assert!(errors.is_empty());
    }

    #[test]
    fn test_max_errors_cap() {
        // Many syntax errors - should be capped at 5
        let bad_code = "def (\ndef (\ndef (\ndef (\ndef (\ndef (\ndef (\n";
        let errors = check_syntax("py", bad_code);
        assert!(errors.len() <= 5);
    }

    #[test]
    fn test_read_decisions_for_file_empty() {
        let dir = tempfile::tempdir().unwrap();
        let decisions = read_decisions_for_file(dir.path().to_str().unwrap(), "src/main.py");
        assert!(decisions.is_empty());
    }

    #[test]
    fn test_read_decisions_for_file_matching() {
        let dir = tempfile::tempdir().unwrap();
        let mem_dir = dir.path().join("memory");
        std::fs::create_dir_all(&mem_dir).unwrap();

        let cache = json!({
            "entries": [
                {
                    "type": "decision",
                    "title": "Use async in cli.py",
                    "content": "All CLI handlers should be async"
                },
                {
                    "type": "decision",
                    "title": "Database schema v2",
                    "content": "Switched to new schema"
                },
                {
                    "type": "pattern",
                    "title": "cli.py uses click",
                    "content": "click framework in cli.py"
                }
            ]
        });
        std::fs::write(
            mem_dir.join("cache.json"),
            serde_json::to_string(&cache).unwrap(),
        )
        .unwrap();

        let decisions =
            read_decisions_for_file(dir.path().to_str().unwrap(), "src/forge_graph/cli.py");
        // Should match "Use async in cli.py" (title contains "cli.py")
        // Should NOT match "Database schema v2" (no file match)
        // Should NOT match "cli.py uses click" (type is "pattern", not "decision")
        assert_eq!(decisions.len(), 1);
        assert!(decisions[0].contains("Use async in cli.py"));
    }

    #[test]
    fn test_read_decisions_for_file_parent_match() {
        let dir = tempfile::tempdir().unwrap();
        let mem_dir = dir.path().join("memory");
        std::fs::create_dir_all(&mem_dir).unwrap();

        let cache = json!({
            "entries": [
                {
                    "type": "decision",
                    "title": "forge_graph module architecture",
                    "content": "All tools import from app.py in forge_graph"
                }
            ]
        });
        std::fs::write(
            mem_dir.join("cache.json"),
            serde_json::to_string(&cache).unwrap(),
        )
        .unwrap();

        let decisions = read_decisions_for_file(
            dir.path().to_str().unwrap(),
            "src/forge_graph/server.py",
        );
        // Should match because parent dir "forge_graph" appears in both title and content
        assert_eq!(decisions.len(), 1);
    }
}
