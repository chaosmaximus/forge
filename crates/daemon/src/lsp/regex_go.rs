// lsp/regex_go.rs — Regex-based symbol extraction for Go
//
// Fallback when gopls is not installed.
// Extracts functions, methods, structs, interfaces, and import edges.

use forge_core::types::CodeSymbol;
use regex::Regex;
use std::sync::LazyLock;

// ─── Compiled regex patterns ─────────────────────────────────────────────────

/// Function: `func FooBar(args) returnType {`
static RE_FUNCTION: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^func\s+(\w+)\s*\(").unwrap()
});

/// Method: `func (r *Receiver) Method(args) returnType {`
static RE_METHOD: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^func\s+\([^)]+\)\s+(\w+)\s*\(").unwrap()
});

/// Struct: `type FooBar struct {`
static RE_STRUCT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^type\s+(\w+)\s+struct\b").unwrap()
});

/// Interface: `type FooBar interface {`
static RE_INTERFACE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^type\s+(\w+)\s+interface\b").unwrap()
});

// Type alias and constant patterns are defined but reserved for future use
// when the indexer is extended to extract these symbols.

/// Import: `import "package"` or entries in `import (...)` block
static RE_IMPORT_SINGLE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"^import\s+"([^"]+)""#).unwrap()
});

static RE_IMPORT_ENTRY: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"^\s+(?:\w+\s+)?"([^"]+)""#).unwrap()
});

// ─── Symbol extraction ───────────────────────────────────────────────────────

/// Extract symbols from Go source code using regex patterns.
pub fn extract_symbols_go(file_path: &str, content: &str) -> Vec<CodeSymbol> {
    let mut symbols = Vec::new();

    for (line_idx, line) in content.lines().enumerate() {
        let line_num = line_idx + 1;
        let trimmed = line.trim_start();

        // Function
        if let Some(cap) = RE_FUNCTION.captures(trimmed) {
            let name = cap[1].to_string();
            symbols.push(CodeSymbol {
                id: format!("{file_path}:{name}:{line_num}"),
                name,
                kind: "function".into(),
                file_path: file_path.to_string(),
                line_start: line_num,
                line_end: None,
                signature: Some(trimmed.trim_end_matches('{').trim().to_string()),
            });
            continue;
        }

        // Method
        if let Some(cap) = RE_METHOD.captures(trimmed) {
            let name = cap[1].to_string();
            symbols.push(CodeSymbol {
                id: format!("{file_path}:{name}:{line_num}"),
                name,
                kind: "function".into(),
                file_path: file_path.to_string(),
                line_start: line_num,
                line_end: None,
                signature: Some(trimmed.trim_end_matches('{').trim().to_string()),
            });
            continue;
        }

        // Struct
        if let Some(cap) = RE_STRUCT.captures(trimmed) {
            let name = cap[1].to_string();
            symbols.push(CodeSymbol {
                id: format!("{file_path}:{name}:{line_num}"),
                name,
                kind: "class".into(),
                file_path: file_path.to_string(),
                line_start: line_num,
                line_end: None,
                signature: None,
            });
            continue;
        }

        // Interface
        if let Some(cap) = RE_INTERFACE.captures(trimmed) {
            let name = cap[1].to_string();
            symbols.push(CodeSymbol {
                id: format!("{file_path}:{name}:{line_num}"),
                name,
                kind: "interface".into(),
                file_path: file_path.to_string(),
                line_start: line_num,
                line_end: None,
                signature: None,
            });
            continue;
        }
    }

    symbols
}

/// Extract import edges from Go source code.
pub fn extract_imports_go(file_path: &str, content: &str) -> Vec<(String, String)> {
    let mut imports = Vec::new();
    let mut in_import_block = false;

    for line in content.lines() {
        let trimmed = line.trim();

        if trimmed == "import (" {
            in_import_block = true;
            continue;
        }
        if in_import_block && trimmed == ")" {
            in_import_block = false;
            continue;
        }

        if in_import_block {
            if let Some(cap) = RE_IMPORT_ENTRY.captures(line) {
                imports.push((file_path.to_string(), cap[1].to_string()));
            }
        } else if let Some(cap) = RE_IMPORT_SINGLE.captures(trimmed) {
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
    fn test_extract_go_function() {
        let content = "func ProcessData(input string) string {\n\treturn input\n}";
        let symbols = extract_symbols_go("main.go", content);
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "ProcessData");
        assert_eq!(symbols[0].kind, "function");
    }

    #[test]
    fn test_extract_go_method() {
        let content = "func (s *Server) HandleRequest(w http.ResponseWriter, r *http.Request) {";
        let symbols = extract_symbols_go("server.go", content);
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "HandleRequest");
        assert_eq!(symbols[0].kind, "function");
    }

    #[test]
    fn test_extract_go_struct() {
        let content = "type Config struct {\n\tHost string\n\tPort int\n}";
        let symbols = extract_symbols_go("config.go", content);
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "Config");
        assert_eq!(symbols[0].kind, "class");
    }

    #[test]
    fn test_extract_go_interface() {
        let content = "type Handler interface {\n\tHandle(ctx context.Context) error\n}";
        let symbols = extract_symbols_go("handler.go", content);
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "Handler");
        assert_eq!(symbols[0].kind, "interface");
    }

    #[test]
    fn test_extract_go_imports() {
        let content = "import (\n\t\"fmt\"\n\t\"net/http\"\n\tlog \"github.com/sirupsen/logrus\"\n)";
        let imports = extract_imports_go("main.go", content);
        assert_eq!(imports.len(), 3);
        assert_eq!(imports[0].1, "fmt");
        assert_eq!(imports[1].1, "net/http");
        assert_eq!(imports[2].1, "github.com/sirupsen/logrus");
    }

    #[test]
    fn test_extract_go_single_import() {
        let content = "import \"fmt\"";
        let imports = extract_imports_go("main.go", content);
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].1, "fmt");
    }

    #[test]
    fn test_extract_go_mixed() {
        let content = "\
package main

import (
\t\"fmt\"
\t\"os\"
)

type App struct {
\tName string
}

func (a *App) Run() error {
\treturn nil
}

func main() {
\tapp := &App{Name: \"test\"}
\tapp.Run()
}
";
        let symbols = extract_symbols_go("main.go", content);
        let names: Vec<&str> = symbols.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"App"));
        assert!(names.contains(&"Run"));
        assert!(names.contains(&"main"));
    }
}
