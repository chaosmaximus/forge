use ignore::WalkBuilder;
use std::path::{Path, PathBuf};

const CODE_EXTENSIONS: &[&str] = &["py", "ts", "tsx", "js", "jsx", "mjs", "cjs"];

pub fn walk(root: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let walker = WalkBuilder::new(root)
        .hidden(true)
        .git_ignore(true)
        .git_global(true)
        .build();
    for entry in walker.flatten() {
        let path = entry.path();
        if !path.is_file() { continue; }
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        if CODE_EXTENSIONS.contains(&ext) {
            files.push(path.to_path_buf());
        }
    }
    files
}
