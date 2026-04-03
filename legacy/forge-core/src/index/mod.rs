pub mod walker;
pub mod parser;
pub mod symbols;
pub mod imports;
pub mod signatures;

use std::path::Path;
use tree_sitter::Parser;

/// Run the indexer. Outputs NDJSON symbols to stdout.
/// If state_dir is provided, also populates import + signature caches
/// for cross-file breakage detection.
pub fn run(path: &str) {
    run_with_state(path, None);
}

pub fn run_with_state(path: &str, state_dir: Option<&str>) {
    let root = Path::new(path);
    let files = walker::walk(root);
    eprintln!("Indexing {} files...", files.len());

    let mut all_imports: Vec<imports::Import> = Vec::new();
    let mut sig_cache: signatures::SignatureCache = state_dir
        .map(|sd| signatures::read_cache(sd))
        .unwrap_or_default();

    for file_path in &files {
        let ext = file_path.extension().and_then(|e| e.to_str()).unwrap_or("");
        let language = match ext {
            "py" => "python",
            "ts" | "tsx" => "typescript",
            "js" | "jsx" | "mjs" | "cjs" => "javascript",
            _ => continue,
        };
        let content = match std::fs::read_to_string(file_path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        // 1. Extract symbols (existing behavior — NDJSON to stdout)
        let symbols = parser::parse(language, &content, file_path);
        for symbol in &symbols {
            if let Ok(json) = serde_json::to_string(symbol) {
                println!("{}", json);
            }
        }

        // 2. Extract imports (new — for cross-file detection)
        let ts_lang = match language {
            "python" => Some(tree_sitter_python::LANGUAGE),
            "typescript" => Some(tree_sitter_typescript::LANGUAGE_TYPESCRIPT),
            "javascript" => Some(tree_sitter_javascript::LANGUAGE),
            _ => None,
        };

        if let Some(lang) = ts_lang {
            let mut ts_parser = Parser::new();
            if ts_parser.set_language(&lang.into()).is_ok() {
                if let Some(tree) = ts_parser.parse(&content, None) {
                    let fp = file_path.to_string_lossy().to_string();

                    // Extract imports
                    let file_imports = imports::extract_imports(
                        &tree.root_node(), &content, &fp, language,
                    );
                    all_imports.extend(file_imports);

                    // Extract signatures
                    let file_sigs = signatures::extract_signatures(
                        &tree.root_node(), &content, language,
                    );
                    sig_cache.insert(fp, file_sigs);
                }
            }
        }
    }

    // Write caches if state_dir provided
    if let Some(sd) = state_dir {
        // Write import cache
        let import_dir = Path::new(sd).join("index");
        std::fs::create_dir_all(&import_dir).ok();
        let import_path = import_dir.join("imports.json");
        let tmp = import_path.with_extension("tmp");
        if let Ok(json) = serde_json::to_string(&all_imports) {
            if std::fs::write(&tmp, &json).is_ok() {
                std::fs::rename(&tmp, &import_path).ok();
            }
        }

        // Write signature cache
        signatures::write_cache(sd, &sig_cache);

        eprintln!(
            "Cached {} imports, {} file signatures.",
            all_imports.len(),
            sig_cache.len(),
        );
    }

    eprintln!("Done.");
}
