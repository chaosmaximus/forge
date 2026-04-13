// lsp/regex_symbols.rs — Regex-based symbol extraction for TypeScript/JavaScript
//
// Fallback when typescript-language-server is not installed.
// Extracts exported and non-exported functions, classes, interfaces, types,
// arrow functions, and import edges using simple regex patterns.

use forge_core::types::CodeSymbol;
use regex::Regex;
use std::sync::LazyLock;

// ─── Compiled regex patterns (static, compiled once) ──────────────────────────

static RE_EXPORT_FUNCTION: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^export\s+function\s+(\w+)\s*\(").unwrap());

static RE_EXPORT_DEFAULT_FUNCTION: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^export\s+default\s+function\s+(\w+)\s*\(").unwrap());

static RE_EXPORT_CONST: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^export\s+const\s+(\w+)\s*=").unwrap());

static RE_EXPORT_CLASS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^export\s+(?:default\s+)?class\s+(\w+)").unwrap());

static RE_EXPORT_INTERFACE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^export\s+(?:default\s+)?interface\s+(\w+)").unwrap());

static RE_EXPORT_TYPE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^export\s+type\s+(\w+)\s*=").unwrap());

static RE_FUNCTION: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^function\s+(\w+)\s*\(").unwrap());

static RE_CLASS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^class\s+(\w+)").unwrap());

static RE_ARROW_FUNCTION: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^const\s+(\w+)\s*=\s*(?:async\s+)?\(").unwrap());

static RE_IMPORT_NAMED: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"^import\s+\{[^}]*\}\s+from\s+['"]([^'"]+)['"]"#).unwrap());

static RE_IMPORT_DEFAULT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"^import\s+(\w+)\s+from\s+['"]([^'"]+)['"]"#).unwrap());

static RE_IMPORT_STAR: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"^import\s+\*\s+as\s+\w+\s+from\s+['"]([^'"]+)['"]"#).unwrap());

// ─── Symbol extraction ────────────────────────────────────────────────────────

/// Extract code symbols from TypeScript/JavaScript source using regex patterns.
///
/// This is a fallback for when the typescript-language-server is not available.
/// It extracts functions, classes, interfaces, types, and arrow functions.
/// The `language` parameter should be "typescript" or "javascript".
pub fn extract_symbols_regex(content: &str, file_path: &str, language: &str) -> Vec<CodeSymbol> {
    if !matches!(language, "typescript" | "javascript") {
        return Vec::new();
    }

    let mut symbols = Vec::new();

    for (line_idx, line) in content.lines().enumerate() {
        let trimmed = line.trim();

        // Skip comment lines
        if trimmed.starts_with("//") || trimmed.starts_with("/*") || trimmed.starts_with('*') {
            continue;
        }

        // Try each pattern in order (more specific first to avoid double-matching)
        if let Some(caps) = RE_EXPORT_DEFAULT_FUNCTION.captures(trimmed) {
            let name = caps.get(1).unwrap().as_str();
            symbols.push(make_symbol(file_path, name, "function", line_idx));
        } else if let Some(caps) = RE_EXPORT_FUNCTION.captures(trimmed) {
            let name = caps.get(1).unwrap().as_str();
            symbols.push(make_symbol(file_path, name, "function", line_idx));
        } else if let Some(caps) = RE_EXPORT_INTERFACE.captures(trimmed) {
            let name = caps.get(1).unwrap().as_str();
            symbols.push(make_symbol(file_path, name, "interface", line_idx));
        } else if let Some(caps) = RE_EXPORT_TYPE.captures(trimmed) {
            let name = caps.get(1).unwrap().as_str();
            symbols.push(make_symbol(file_path, name, "type", line_idx));
        } else if let Some(caps) = RE_EXPORT_CLASS.captures(trimmed) {
            let name = caps.get(1).unwrap().as_str();
            symbols.push(make_symbol(file_path, name, "class", line_idx));
        } else if let Some(caps) = RE_EXPORT_CONST.captures(trimmed) {
            let name = caps.get(1).unwrap().as_str();
            // Check if this is actually an arrow function
            let kind = if is_arrow_or_async_arrow(trimmed) {
                "function"
            } else {
                "variable"
            };
            symbols.push(make_symbol(file_path, name, kind, line_idx));
        } else if let Some(caps) = RE_FUNCTION.captures(trimmed) {
            let name = caps.get(1).unwrap().as_str();
            symbols.push(make_symbol(file_path, name, "function", line_idx));
        } else if let Some(caps) = RE_CLASS.captures(trimmed) {
            let name = caps.get(1).unwrap().as_str();
            symbols.push(make_symbol(file_path, name, "class", line_idx));
        } else if let Some(caps) = RE_ARROW_FUNCTION.captures(trimmed) {
            // Non-exported arrow function (const NAME = ( or const NAME = async ()
            let name = caps.get(1).unwrap().as_str();
            symbols.push(make_symbol(file_path, name, "function", line_idx));
        }
    }

    symbols
}

/// Check if a line like `export const NAME = ...` is an arrow function.
fn is_arrow_or_async_arrow(line: &str) -> bool {
    // Look for patterns like `= (` or `= async (`
    line.contains("= (") || line.contains("= async (")
}

fn make_symbol(file_path: &str, name: &str, kind: &str, line_idx: usize) -> CodeSymbol {
    CodeSymbol {
        id: format!("sym:{file_path}:{name}"),
        name: name.to_string(),
        kind: kind.to_string(),
        file_path: file_path.to_string(),
        line_start: line_idx,
        line_end: None,
        signature: None,
    }
}

// ─── Import extraction ────────────────────────────────────────────────────────

/// Extract import edges from TypeScript/JavaScript source.
///
/// Returns `Vec<(source_file_path, imported_module)>` — the same format used
/// by `extract_imports` in `lsp/symbols.rs`.
pub fn extract_imports_regex(content: &str, file_path: &str) -> Vec<(String, String)> {
    let mut results = Vec::new();

    for line in content.lines() {
        let trimmed = line.trim();

        // Skip comment lines
        if trimmed.starts_with("//") || trimmed.starts_with("/*") || trimmed.starts_with('*') {
            continue;
        }

        if let Some(caps) = RE_IMPORT_NAMED.captures(trimmed) {
            let module = caps.get(1).unwrap().as_str();
            results.push((file_path.to_string(), module.to_string()));
        } else if let Some(caps) = RE_IMPORT_DEFAULT.captures(trimmed) {
            // Group 2 is the module path
            let module = caps.get(2).unwrap().as_str();
            results.push((file_path.to_string(), module.to_string()));
        } else if let Some(caps) = RE_IMPORT_STAR.captures(trimmed) {
            let module = caps.get(1).unwrap().as_str();
            results.push((file_path.to_string(), module.to_string()));
        }
    }

    results
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_export_function() {
        let src = "export function handleRequest(req: Request) {\n  return null;\n}\n";
        let syms = extract_symbols_regex(src, "api/handler.ts", "typescript");
        assert_eq!(syms.len(), 1);
        assert_eq!(syms[0].name, "handleRequest");
        assert_eq!(syms[0].kind, "function");
        assert_eq!(syms[0].line_start, 0);
    }

    #[test]
    fn test_extract_export_default_function() {
        let src = "export default function App() {\n  return <div/>;\n}\n";
        let syms = extract_symbols_regex(src, "App.tsx", "typescript");
        assert_eq!(syms.len(), 1);
        assert_eq!(syms[0].name, "App");
        assert_eq!(syms[0].kind, "function");
    }

    #[test]
    fn test_extract_export_const_variable() {
        let src = "export const API_URL = 'https://example.com';\n";
        let syms = extract_symbols_regex(src, "config.ts", "typescript");
        assert_eq!(syms.len(), 1);
        assert_eq!(syms[0].name, "API_URL");
        assert_eq!(syms[0].kind, "variable");
    }

    #[test]
    fn test_extract_export_const_arrow_function() {
        let src = "export const fetchData = async (url: string) => {\n  return fetch(url);\n}\n";
        let syms = extract_symbols_regex(src, "utils.ts", "typescript");
        assert_eq!(syms.len(), 1);
        assert_eq!(syms[0].name, "fetchData");
        assert_eq!(syms[0].kind, "function");
    }

    #[test]
    fn test_extract_export_class() {
        let src = "export class UserService {\n  constructor() {}\n}\n";
        let syms = extract_symbols_regex(src, "service.ts", "typescript");
        assert_eq!(syms.len(), 1);
        assert_eq!(syms[0].name, "UserService");
        assert_eq!(syms[0].kind, "class");
    }

    #[test]
    fn test_extract_export_interface() {
        let src = "export interface UserProps {\n  name: string;\n}\n";
        let syms = extract_symbols_regex(src, "types.ts", "typescript");
        assert_eq!(syms.len(), 1);
        assert_eq!(syms[0].name, "UserProps");
        assert_eq!(syms[0].kind, "interface");
    }

    #[test]
    fn test_extract_export_type() {
        let src = "export type Status = 'active' | 'inactive';\n";
        let syms = extract_symbols_regex(src, "types.ts", "typescript");
        assert_eq!(syms.len(), 1);
        assert_eq!(syms[0].name, "Status");
        assert_eq!(syms[0].kind, "type");
    }

    #[test]
    fn test_extract_non_exported_function() {
        let src = "function helperFn(x: number) {\n  return x * 2;\n}\n";
        let syms = extract_symbols_regex(src, "helpers.ts", "typescript");
        assert_eq!(syms.len(), 1);
        assert_eq!(syms[0].name, "helperFn");
        assert_eq!(syms[0].kind, "function");
    }

    #[test]
    fn test_extract_non_exported_class() {
        let src = "class InternalCache {\n  private data = new Map();\n}\n";
        let syms = extract_symbols_regex(src, "cache.ts", "typescript");
        assert_eq!(syms.len(), 1);
        assert_eq!(syms[0].name, "InternalCache");
        assert_eq!(syms[0].kind, "class");
    }

    #[test]
    fn test_extract_arrow_function() {
        let src = "const processItem = (item: Item) => {\n  return item;\n}\n";
        let syms = extract_symbols_regex(src, "process.ts", "typescript");
        assert_eq!(syms.len(), 1);
        assert_eq!(syms[0].name, "processItem");
        assert_eq!(syms[0].kind, "function");
    }

    #[test]
    fn test_extract_async_arrow_function() {
        let src = "const fetchUser = async (id: string) => {\n  return await db.get(id);\n}\n";
        let syms = extract_symbols_regex(src, "db.ts", "typescript");
        assert_eq!(syms.len(), 1);
        assert_eq!(syms[0].name, "fetchUser");
        assert_eq!(syms[0].kind, "function");
    }

    #[test]
    fn test_extract_multiple_symbols() {
        let src = r#"export interface Config {
  port: number;
}

export class Server {
  constructor(config: Config) {}
}

export function startServer(config: Config) {
  return new Server(config);
}

const helper = (x: number) => {
  return x;
}

export type Mode = 'dev' | 'prod';
"#;
        let syms = extract_symbols_regex(src, "server.ts", "typescript");
        let names: Vec<&str> = syms.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"Config"), "should find Config interface");
        assert!(names.contains(&"Server"), "should find Server class");
        assert!(
            names.contains(&"startServer"),
            "should find startServer function"
        );
        assert!(names.contains(&"helper"), "should find helper arrow fn");
        assert!(names.contains(&"Mode"), "should find Mode type");
        assert_eq!(syms.len(), 5);
    }

    #[test]
    fn test_skips_comments() {
        let src = "// export function commented(x) {}\n/* export class Disabled {} */\nexport function real() {}\n";
        let syms = extract_symbols_regex(src, "file.ts", "typescript");
        assert_eq!(syms.len(), 1);
        assert_eq!(syms[0].name, "real");
    }

    #[test]
    fn test_ignores_non_ts_language() {
        let src = "export function test() {}\n";
        let syms = extract_symbols_regex(src, "file.rs", "rust");
        assert!(
            syms.is_empty(),
            "should not extract from non-TS/JS languages"
        );
    }

    #[test]
    fn test_javascript_language() {
        let src = "export function jsFunc() {}\nclass JsClass {}\n";
        let syms = extract_symbols_regex(src, "file.js", "javascript");
        assert_eq!(syms.len(), 2);
        assert_eq!(syms[0].name, "jsFunc");
        assert_eq!(syms[1].name, "JsClass");
    }

    #[test]
    fn test_extract_imports_named() {
        let src = r#"import { useState, useEffect } from 'react';
import { fetchUser } from './api/users';
"#;
        let imports = extract_imports_regex(src, "app.tsx");
        assert_eq!(imports.len(), 2);
        assert_eq!(imports[0].1, "react");
        assert_eq!(imports[1].1, "./api/users");
    }

    #[test]
    fn test_extract_imports_default() {
        let src = r#"import React from 'react';
import express from 'express';
"#;
        let imports = extract_imports_regex(src, "index.ts");
        assert_eq!(imports.len(), 2);
        assert_eq!(imports[0].1, "react");
        assert_eq!(imports[1].1, "express");
    }

    #[test]
    fn test_extract_imports_star() {
        let src = r#"import * as path from 'path';
"#;
        let imports = extract_imports_regex(src, "utils.ts");
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].1, "path");
    }

    #[test]
    fn test_extract_imports_skips_comments() {
        let src = r#"// import { old } from 'deprecated';
import { real } from 'actual';
"#;
        let imports = extract_imports_regex(src, "file.ts");
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].1, "actual");
    }

    #[test]
    fn test_extract_imports_source_path() {
        let src = r#"import { foo } from 'bar';"#;
        let imports = extract_imports_regex(src, "/project/src/index.ts");
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].0, "/project/src/index.ts");
        assert_eq!(imports[0].1, "bar");
    }

    #[test]
    fn test_symbol_id_format() {
        let src = "export function myFunc() {}\n";
        let syms = extract_symbols_regex(src, "/project/src/lib.ts", "typescript");
        assert_eq!(syms[0].id, "sym:/project/src/lib.ts:myFunc");
    }

    #[test]
    fn test_line_numbers_correct() {
        let src =
            "// header comment\n\nexport function first() {}\n\nexport function second() {}\n";
        let syms = extract_symbols_regex(src, "file.ts", "typescript");
        assert_eq!(syms.len(), 2);
        assert_eq!(syms[0].name, "first");
        assert_eq!(syms[0].line_start, 2);
        assert_eq!(syms[1].name, "second");
        assert_eq!(syms[1].line_start, 4);
    }
}
