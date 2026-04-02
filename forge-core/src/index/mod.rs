pub mod walker;
pub mod parser;
pub mod symbols;
pub mod imports;
pub mod signatures;

use std::path::Path;

pub fn run(path: &str) {
    let root = Path::new(path);
    let files = walker::walk(root);
    eprintln!("Indexing {} files...", files.len());

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
        let symbols = parser::parse(language, &content, file_path);
        for symbol in symbols {
            if let Ok(json) = serde_json::to_string(&symbol) {
                println!("{}", json);
            }
        }
    }
    eprintln!("Done.");
}
