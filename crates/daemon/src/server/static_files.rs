use axum::Router;
use std::path::PathBuf;
use tower_http::services::{ServeDir, ServeFile};

/// Build an Axum router that serves static files from `ui_dir` with SPA fallback.
///
/// Returns `None` if `ui_dir/index.html` does not exist, so the caller can
/// skip merging this into the main router.
pub fn static_file_router(ui_dir: &str) -> Option<Router> {
    let path = PathBuf::from(ui_dir);
    if !path.join("index.html").exists() {
        tracing::warn!(ui_dir, "UI dir missing index.html — static serving disabled");
        return None;
    }
    tracing::info!(ui_dir, "Serving web UI from static files");
    let serve_dir = ServeDir::new(ui_dir)
        .not_found_service(ServeFile::new(path.join("index.html")));
    Some(Router::new().fallback_service(serve_dir))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_none_when_dir_missing() {
        assert!(static_file_router("/nonexistent/path/to/ui").is_none());
    }

    #[test]
    fn returns_none_when_no_index_html() {
        // temp dir exists but has no index.html
        let dir = std::env::temp_dir().join("forge-static-test-empty");
        let _ = std::fs::create_dir_all(&dir);
        // make sure there's no index.html leftover
        let _ = std::fs::remove_file(dir.join("index.html"));
        assert!(static_file_router(dir.to_str().unwrap()).is_none());
    }

    #[test]
    fn returns_some_when_index_html_exists() {
        let dir = std::env::temp_dir().join("forge-static-test-with-index");
        let _ = std::fs::create_dir_all(&dir);
        std::fs::write(dir.join("index.html"), "<html></html>").unwrap();
        let result = static_file_router(dir.to_str().unwrap());
        assert!(result.is_some());
        // cleanup
        let _ = std::fs::remove_dir_all(&dir);
    }
}
