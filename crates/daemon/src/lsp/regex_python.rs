// lsp/regex_python.rs — Regex-based symbol extraction for Python
//
// Fallback when pyright-langserver is not installed.
// Extracts functions, classes, methods, and import edges using regex patterns.

use forge_core::types::CodeSymbol;
use regex::Regex;
use std::sync::LazyLock;

// ─── Compiled regex patterns ─────────────────────────────────────────────────

/// Top-level function: `def foo_bar(...):`
static RE_FUNCTION: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^def\s+(\w+)\s*\(").unwrap()
});

/// Async function: `async def foo_bar(...):`
static RE_ASYNC_FUNCTION: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^async\s+def\s+(\w+)\s*\(").unwrap()
});

/// Class: `class FooBar(Base):`
static RE_CLASS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^class\s+(\w+)").unwrap()
});

/// Method (indented def): `    def method(self, ...):`
static RE_METHOD: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^\s+(?:async\s+)?def\s+(\w+)\s*\(").unwrap()
});

/// Import: `from module import name`
static RE_FROM_IMPORT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^from\s+([\w.]+)\s+import\s+").unwrap()
});

/// Import: `import module`
static RE_IMPORT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^import\s+([\w.]+)").unwrap()
});

/// Global variable/constant: `FOO_BAR = ...` (all caps)
static RE_CONSTANT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^([A-Z][A-Z0-9_]{2,})\s*=").unwrap()
});

// ─── Symbol extraction ───────────────────────────────────────────────────────

/// Extract symbols from Python source code using regex patterns.
pub fn extract_symbols_python(file_path: &str, content: &str) -> Vec<CodeSymbol> {
    let mut symbols = Vec::new();
    let mut current_class: Option<String> = None;
    let mut class_indent = 0usize;

    for (line_idx, line) in content.lines().enumerate() {
        let line_num = line_idx + 1;
        let trimmed = line.trim_start();
        let indent = line.len() - trimmed.len();

        // Track class scope
        if indent <= class_indent
            && current_class.is_some()
            && !trimmed.is_empty()
            && !trimmed.starts_with("def ")
            && !trimmed.starts_with("async def ")
            && !trimmed.starts_with('#')
            && !trimmed.starts_with('@')
        {
            current_class = None;
        }

        // Class
        if let Some(cap) = RE_CLASS.captures(trimmed) {
            let name = cap[1].to_string();
            current_class = Some(name.clone());
            class_indent = indent;
            symbols.push(CodeSymbol {
                id: format!("{}:{}:{}", file_path, name, line_num),
                name,
                kind: "class".into(),
                file_path: file_path.to_string(),
                line_start: line_num,
                line_end: None,
                signature: Some(trimmed.trim_end_matches(':').to_string()),
            });
            continue;
        }

        // Method (indented def inside a class)
        if indent > 0 && current_class.is_some() {
            if let Some(cap) = RE_METHOD.captures(line) {
                let name = cap[1].to_string();
                let parent = current_class.clone();
                symbols.push(CodeSymbol {
                    id: format!("{}:{}:{}", file_path, name, line_num),
                    name,
                    kind: "function".into(),
                    file_path: file_path.to_string(),
                    line_start: line_num,
                    line_end: None,
                    signature: parent,
                });
                continue;
            }
        }

        // Top-level function
        if let Some(cap) = RE_FUNCTION.captures(trimmed) {
            let name = cap[1].to_string();
            symbols.push(CodeSymbol {
                id: format!("{}:{}:{}", file_path, name, line_num),
                name,
                kind: "function".into(),
                file_path: file_path.to_string(),
                line_start: line_num,
                line_end: None,
                signature: Some(trimmed.trim_end_matches(':').to_string()),
            });
            continue;
        }

        // Async function
        if let Some(cap) = RE_ASYNC_FUNCTION.captures(trimmed) {
            let name = cap[1].to_string();
            symbols.push(CodeSymbol {
                id: format!("{}:{}:{}", file_path, name, line_num),
                name,
                kind: "function".into(),
                file_path: file_path.to_string(),
                line_start: line_num,
                line_end: None,
                signature: Some(trimmed.trim_end_matches(':').to_string()),
            });
            continue;
        }

        // Constant
        if let Some(cap) = RE_CONSTANT.captures(trimmed) {
            let name = cap[1].to_string();
            symbols.push(CodeSymbol {
                id: format!("{}:{}:{}", file_path, name, line_num),
                name,
                kind: "variable".into(),
                file_path: file_path.to_string(),
                line_start: line_num,
                line_end: None,
                signature: None,
            });
        }
    }

    symbols
}

/// Extract import edges from Python source code.
/// Returns Vec<(from_file, imported_module)>.
pub fn extract_imports_python(file_path: &str, content: &str) -> Vec<(String, String)> {
    let mut imports = Vec::new();

    for line in content.lines() {
        let trimmed = line.trim();
        if let Some(cap) = RE_FROM_IMPORT.captures(trimmed) {
            imports.push((file_path.to_string(), cap[1].to_string()));
        } else if let Some(cap) = RE_IMPORT.captures(trimmed) {
            imports.push((file_path.to_string(), cap[1].to_string()));
        }
    }

    imports
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_python_function() {
        let content = "def process_data(input: str) -> str:\n    return input.upper()";
        let symbols = extract_symbols_python("test.py", content);
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "process_data");
        assert_eq!(symbols[0].kind, "function");
    }

    #[test]
    fn test_extract_python_async_function() {
        let content = "async def fetch_data(url: str):\n    pass";
        let symbols = extract_symbols_python("test.py", content);
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "fetch_data");
        assert_eq!(symbols[0].kind, "function");
    }

    #[test]
    fn test_extract_python_class_with_methods() {
        let content = "class MyService:\n    def __init__(self):\n        pass\n    def run(self):\n        pass";
        let symbols = extract_symbols_python("test.py", content);
        assert_eq!(symbols.len(), 3);
        assert_eq!(symbols[0].name, "MyService");
        assert_eq!(symbols[0].kind, "class");
        assert_eq!(symbols[1].name, "__init__");
        assert_eq!(symbols[1].kind, "function");
        assert_eq!(symbols[2].name, "run");
    }

    #[test]
    fn test_extract_python_constant() {
        let content = "MAX_RETRIES = 5\nDATABASE_URL = 'sqlite:///db.sqlite'";
        let symbols = extract_symbols_python("test.py", content);
        assert_eq!(symbols.len(), 2);
        assert_eq!(symbols[0].name, "MAX_RETRIES");
        assert_eq!(symbols[0].kind, "variable");
        assert_eq!(symbols[1].name, "DATABASE_URL");
    }

    #[test]
    fn test_extract_python_imports() {
        let content = "from os.path import join\nimport sys\nfrom typing import Optional";
        let imports = extract_imports_python("test.py", content);
        assert_eq!(imports.len(), 3);
        assert_eq!(imports[0].1, "os.path");
        assert_eq!(imports[1].1, "sys");
        assert_eq!(imports[2].1, "typing");
    }

    #[test]
    fn test_extract_python_mixed() {
        let content = "\
import os

MAX_WORKERS = 8

class Handler:
    def __init__(self, name):
        self.name = name

    async def process(self, data):
        pass

def main():
    h = Handler('test')
";
        let symbols = extract_symbols_python("app.py", content);
        let names: Vec<&str> = symbols.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"MAX_WORKERS"));
        assert!(names.contains(&"Handler"));
        assert!(names.contains(&"__init__"));
        assert!(names.contains(&"process"));
        assert!(names.contains(&"main"));
    }
}
