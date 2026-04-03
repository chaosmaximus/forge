use crate::index::symbols::Symbol;
use std::path::Path;
use tree_sitter::Parser;

pub fn parse(language: &str, source: &str, file_path: &Path) -> Vec<Symbol> {
    let mut parser = Parser::new();
    let ts_language = match language {
        "python" => tree_sitter_python::LANGUAGE.into(),
        "typescript" => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        "javascript" => tree_sitter_javascript::LANGUAGE.into(),
        _ => return Vec::new(),
    };
    parser.set_language(&ts_language).expect("Failed to set language");
    let tree = match parser.parse(source, None) {
        Some(t) => t,
        None => return Vec::new(),
    };

    let fp = file_path.to_string_lossy().to_string();
    let mut symbols = Vec::new();

    symbols.push(Symbol::file(
        &fp,
        file_path.file_name().unwrap_or_default().to_string_lossy().as_ref(),
        language,
        source.len(),
    ));

    extract_symbols(&tree.root_node(), source, &fp, None, &mut symbols);
    symbols
}

fn extract_symbols(
    node: &tree_sitter::Node, source: &str, fp: &str,
    parent_class: Option<&str>, symbols: &mut Vec<Symbol>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "class_definition" | "class_declaration" => {
                if let Some(name_node) = child.child_by_field_name("name") {
                    let name = &source[name_node.byte_range()];
                    symbols.push(Symbol::class(name, fp,
                        child.start_position().row + 1, child.end_position().row + 1));
                    if let Some(body) = child.child_by_field_name("body") {
                        extract_symbols(&body, source, fp, Some(name), symbols);
                    }
                }
            }
            "function_definition" | "function_declaration" | "method_definition" => {
                if let Some(name_node) = child.child_by_field_name("name") {
                    let name = &source[name_node.byte_range()];
                    let sig = &source[child.start_byte()..child.end_byte().min(child.start_byte() + 200)];
                    let first_line = sig.lines().next().unwrap_or("");
                    if let Some(class_name) = parent_class {
                        symbols.push(Symbol::method(name, class_name, fp,
                            child.start_position().row + 1, child.end_position().row + 1, first_line));
                    } else {
                        symbols.push(Symbol::function(name, fp,
                            child.start_position().row + 1, child.end_position().row + 1, first_line));
                    }
                }
            }
            _ => {
                extract_symbols(&child, source, fp, parent_class, symbols);
            }
        }
    }
}
