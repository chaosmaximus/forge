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

    // --- Secret scan ---
    for line in content.lines() {
        for rule in RULES.iter() {
            if rule.regex.is_match(line) {
                alerts.push(format!("{} detected.", rule.description));
                break; // One alert per line is enough
            }
        }
    }

    // --- Syntax validation via tree-sitter ---
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    let syntax_errors = check_syntax(ext, &content);
    if !syntax_errors.is_empty() {
        alerts.push(format!(
            "SYNTAX ERROR in {}: {}",
            file_path,
            syntax_errors.join("; ")
        ));
    }

    if !alerts.is_empty() {
        let alert_str = alerts.join(" ");
        let output = json!({
            "hookSpecificOutput": {
                "additionalContext": format!(
                    "ALERT in {}: {} Consider reviewing before continuing.",
                    file_path, alert_str
                )
            }
        });
        println!("{}", output);
    }
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
            errors.push(format!(
                "line {}:{} missing expected syntax",
                row, col
            ));
        } else {
            let clean: String = short_snippet.chars().filter(|c| !c.is_control()).collect();
            errors.push(format!(
                "line {}:{} parse error near '{}'",
                row, col, clean
            ));
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
}
