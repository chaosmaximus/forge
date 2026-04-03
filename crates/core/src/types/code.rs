use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodeFile {
    pub id: String,
    pub path: String,
    pub language: String,
    pub project: String,
    pub hash: String,
    pub indexed_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodeSymbol {
    pub id: String,
    pub name: String,
    pub kind: String,        // "function" | "class" | "method"
    pub file_path: String,
    pub line_start: usize,
    pub line_end: Option<usize>,
    pub signature: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodeImport {
    pub source_file: String,
    pub target_module: String,
    pub names: Vec<String>,
    pub line: usize,
}

/// Output format from v1 `forge index` command (NDJSON, one line per symbol).
#[derive(Debug, Clone, Deserialize)]
pub struct V1IndexSymbol {
    pub kind: String,
    pub id: String,
    pub name: String,
    pub file_path: String,
    pub line_start: Option<usize>,
    pub line_end: Option<usize>,
    pub language: Option<String>,
    pub hash: Option<String>,
    pub signature: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_code_file_serde() {
        let file = CodeFile {
            id: "f1".into(),
            path: "src/main.rs".into(),
            language: "rust".into(),
            project: "forge".into(),
            hash: "abc123".into(),
            indexed_at: "2026-04-02".into(),
        };

        let json = serde_json::to_string(&file).expect("serialize CodeFile");
        let restored: CodeFile = serde_json::from_str(&json).expect("deserialize CodeFile");

        assert_eq!(file.id, restored.id);
        assert_eq!(file.path, restored.path);
        assert_eq!(file.language, restored.language);
        assert_eq!(file.project, restored.project);
        assert_eq!(file.hash, restored.hash);
        assert_eq!(file.indexed_at, restored.indexed_at);
    }

    #[test]
    fn test_v1_index_symbol_parse() {
        let ndjson_line = r#"{"kind":"function","id":"main_fn","name":"main","file_path":"src/main.rs","line_start":1,"line_end":10,"language":"rust","hash":"deadbeef","signature":"fn main()"}"#;
        let sym: V1IndexSymbol = serde_json::from_str(ndjson_line).expect("parse V1IndexSymbol");

        assert_eq!(sym.kind, "function");
        assert_eq!(sym.id, "main_fn");
        assert_eq!(sym.name, "main");
        assert_eq!(sym.file_path, "src/main.rs");
        assert_eq!(sym.line_start, Some(1));
        assert_eq!(sym.line_end, Some(10));
        assert_eq!(sym.language, Some("rust".into()));
        assert_eq!(sym.hash, Some("deadbeef".into()));
        assert_eq!(sym.signature, Some("fn main()".into()));
    }

    #[test]
    fn test_v1_index_symbol_parse_minimal() {
        // V1 symbols may omit optional fields
        let ndjson_line = r#"{"kind":"class","id":"Foo","name":"Foo","file_path":"src/foo.py"}"#;
        let sym: V1IndexSymbol = serde_json::from_str(ndjson_line).expect("parse minimal V1IndexSymbol");

        assert_eq!(sym.kind, "class");
        assert_eq!(sym.name, "Foo");
        assert!(sym.line_start.is_none());
        assert!(sym.line_end.is_none());
        assert!(sym.language.is_none());
        assert!(sym.hash.is_none());
        assert!(sym.signature.is_none());
    }

    #[test]
    fn test_code_symbol_serde() {
        let sym = CodeSymbol {
            id: "s1".into(),
            name: "main".into(),
            kind: "function".into(),
            file_path: "src/main.rs".into(),
            line_start: 1,
            line_end: Some(10),
            signature: Some("fn main()".into()),
        };

        let json = serde_json::to_string(&sym).expect("serialize CodeSymbol");
        let restored: CodeSymbol = serde_json::from_str(&json).expect("deserialize CodeSymbol");

        assert_eq!(sym.id, restored.id);
        assert_eq!(sym.name, restored.name);
        assert_eq!(sym.kind, restored.kind);
        assert_eq!(sym.file_path, restored.file_path);
        assert_eq!(sym.line_start, restored.line_start);
        assert_eq!(sym.line_end, restored.line_end);
        assert_eq!(sym.signature, restored.signature);
    }

    #[test]
    fn test_code_import_serde() {
        let imp = CodeImport {
            source_file: "src/main.rs".into(),
            target_module: "std::collections".into(),
            names: vec!["HashMap".into(), "HashSet".into()],
            line: 1,
        };

        let json = serde_json::to_string(&imp).expect("serialize CodeImport");
        let restored: CodeImport = serde_json::from_str(&json).expect("deserialize CodeImport");

        assert_eq!(imp.source_file, restored.source_file);
        assert_eq!(imp.target_module, restored.target_module);
        assert_eq!(imp.names, restored.names);
        assert_eq!(imp.line, restored.line);
    }
}
