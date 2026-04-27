use forge_core::types::reality_engine::{DetectionResult, EngineCapabilities, RealityEngine};
use rusqlite::{params, Connection};
use std::path::Path;

/// Marker file with associated domain and confidence.
struct MarkerFile {
    filename: &'static str,
    domain: &'static str,
    confidence: f64,
    metadata_key: &'static str,
    metadata_value: &'static str,
}

/// Primary marker files checked in priority order.
const MARKERS: &[MarkerFile] = &[
    MarkerFile {
        filename: "Cargo.toml",
        domain: "rust",
        confidence: 0.95,
        metadata_key: "build_system",
        metadata_value: "cargo",
    },
    MarkerFile {
        filename: "tsconfig.json",
        domain: "typescript",
        confidence: 0.95,
        metadata_key: "build_system",
        metadata_value: "tsc",
    },
    MarkerFile {
        filename: "package.json",
        domain: "javascript",
        confidence: 0.7,
        metadata_key: "build_system",
        metadata_value: "npm",
    },
    MarkerFile {
        filename: "pyproject.toml",
        domain: "python",
        confidence: 0.95,
        metadata_key: "build_system",
        metadata_value: "pyproject",
    },
    MarkerFile {
        filename: "setup.py",
        domain: "python",
        confidence: 0.7,
        metadata_key: "build_system",
        metadata_value: "setuptools",
    },
    MarkerFile {
        filename: "go.mod",
        domain: "go",
        confidence: 0.95,
        metadata_key: "build_system",
        metadata_value: "go_modules",
    },
    MarkerFile {
        filename: "Gemfile",
        domain: "ruby",
        confidence: 0.95,
        metadata_key: "build_system",
        metadata_value: "bundler",
    },
    MarkerFile {
        filename: "pom.xml",
        domain: "java",
        confidence: 0.95,
        metadata_key: "build_system",
        metadata_value: "maven",
    },
    MarkerFile {
        filename: "build.gradle",
        domain: "java",
        confidence: 0.95,
        metadata_key: "build_system",
        metadata_value: "gradle",
    },
    MarkerFile {
        filename: "CMakeLists.txt",
        domain: "cpp",
        confidence: 0.95,
        metadata_key: "build_system",
        metadata_value: "cmake",
    },
    MarkerFile {
        filename: "Makefile",
        domain: "c",
        confidence: 0.7,
        metadata_key: "build_system",
        metadata_value: "make",
    },
    MarkerFile {
        filename: "pubspec.yaml",
        domain: "dart",
        confidence: 0.95,
        metadata_key: "build_system",
        metadata_value: "pub",
    },
];

/// Code Reality Engine — detects and indexes code projects.
///
/// Uses marker file detection to determine project language/domain.
/// In future waves, will also index code symbols via LSP and regex.
pub struct CodeRealityEngine;

impl RealityEngine for CodeRealityEngine {
    fn name(&self) -> &str {
        "code"
    }

    fn version(&self) -> &str {
        "1.0.0"
    }

    fn reality_type(&self) -> &str {
        "code"
    }

    fn detect(&self, path: &Path) -> Option<DetectionResult> {
        // Check marker files in priority order.
        // Return the first (highest-confidence) match.
        for marker in MARKERS {
            if path.join(marker.filename).exists() {
                return Some(DetectionResult {
                    confidence: marker.confidence,
                    detected_from: marker.filename.to_string(),
                    domain: marker.domain.to_string(),
                    reality_type: "code".to_string(),
                    metadata: serde_json::json!({
                        marker.metadata_key: marker.metadata_value,
                        "language": marker.domain,
                    }),
                });
            }
        }
        None
    }

    fn capabilities(&self) -> EngineCapabilities {
        EngineCapabilities {
            graph_node_types: vec![
                "file".into(),
                "function".into(),
                "class".into(),
                "module".into(),
                "cluster".into(),
            ],
            graph_edge_types: vec![
                "calls".into(),
                "imports".into(),
                "belongs_to_cluster".into(),
            ],
            perception_kinds: vec![
                "file_change".into(),
                "diagnostic".into(),
                "build_result".into(),
            ],
            supports_embeddings: true,
            supports_search: true,
        }
    }
}

impl CodeRealityEngine {
    /// Search code symbols by name pattern with optional kind filter.
    ///
    /// Returns matching symbols as JSON values with id, name, kind, path, line_start.
    pub fn search(
        conn: &Connection,
        query: &str,
        kind: Option<&str>,
        limit: usize,
    ) -> Vec<serde_json::Value> {
        let pattern = format!("%{query}%");
        let effective_limit = limit.min(100);

        if let Some(kind_filter) = kind {
            conn.prepare(
                "SELECT id, name, kind, file_path, line_start FROM code_symbol WHERE name LIKE ?1 AND kind = ?2 LIMIT ?3",
            )
            .and_then(|mut stmt| {
                stmt.query_map(params![pattern, kind_filter, effective_limit], |row| {
                    Ok(serde_json::json!({
                        "id": row.get::<_, String>(0)?,
                        "name": row.get::<_, String>(1)?,
                        "kind": row.get::<_, String>(2)?,
                        "path": row.get::<_, String>(3)?,
                        "line_start": row.get::<_, Option<i64>>(4)?,
                    }))
                })?
                .collect()
            })
            .unwrap_or_default()
        } else {
            conn.prepare(
                "SELECT id, name, kind, file_path, line_start FROM code_symbol WHERE name LIKE ?1 LIMIT ?2",
            )
            .and_then(|mut stmt| {
                stmt.query_map(params![pattern, effective_limit], |row| {
                    Ok(serde_json::json!({
                        "id": row.get::<_, String>(0)?,
                        "name": row.get::<_, String>(1)?,
                        "kind": row.get::<_, String>(2)?,
                        "path": row.get::<_, String>(3)?,
                        "line_start": row.get::<_, Option<i64>>(4)?,
                    }))
                })?
                .collect()
            })
            .unwrap_or_default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use forge_core::types::reality_engine::RealityEngine;
    use tempfile::tempdir;

    #[test]
    fn test_code_engine_detect_rust() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]").unwrap();

        let engine = CodeRealityEngine;
        let result = engine.detect(dir.path());
        assert!(result.is_some(), "should detect Rust project");
        let r = result.unwrap();
        assert_eq!(r.domain, "rust");
        assert_eq!(r.reality_type, "code");
        assert!((r.confidence - 0.95).abs() < f64::EPSILON);
        assert_eq!(r.detected_from, "Cargo.toml");
    }

    #[test]
    fn test_code_engine_detect_python() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("pyproject.toml"), "[project]").unwrap();

        let engine = CodeRealityEngine;
        let result = engine.detect(dir.path());
        assert!(result.is_some(), "should detect Python project");
        let r = result.unwrap();
        assert_eq!(r.domain, "python");
        assert!((r.confidence - 0.95).abs() < f64::EPSILON);
    }

    #[test]
    fn test_code_engine_detect_typescript() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("tsconfig.json"), "{}").unwrap();

        let engine = CodeRealityEngine;
        let result = engine.detect(dir.path());
        assert!(result.is_some(), "should detect TypeScript project");
        let r = result.unwrap();
        assert_eq!(r.domain, "typescript");
        assert!((r.confidence - 0.95).abs() < f64::EPSILON);
    }

    #[test]
    fn test_code_engine_detect_go() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("go.mod"), "module example.com/foo").unwrap();

        let engine = CodeRealityEngine;
        let result = engine.detect(dir.path());
        assert!(result.is_some(), "should detect Go project");
        let r = result.unwrap();
        assert_eq!(r.domain, "go");
        assert!((r.confidence - 0.95).abs() < f64::EPSILON);
    }

    #[test]
    fn test_code_engine_detect_none() {
        let dir = tempdir().unwrap();
        // Empty directory: no marker files

        let engine = CodeRealityEngine;
        let result = engine.detect(dir.path());
        assert!(
            result.is_none(),
            "should not detect any project in empty dir"
        );
    }

    #[test]
    fn test_code_engine_capabilities() {
        let engine = CodeRealityEngine;
        let caps = engine.capabilities();

        assert!(caps.graph_node_types.contains(&"file".to_string()));
        assert!(caps.graph_node_types.contains(&"function".to_string()));
        assert!(caps.graph_node_types.contains(&"class".to_string()));
        assert!(caps.graph_node_types.contains(&"module".to_string()));
        assert!(caps.graph_node_types.contains(&"cluster".to_string()));

        assert!(caps.graph_edge_types.contains(&"calls".to_string()));
        assert!(caps.graph_edge_types.contains(&"imports".to_string()));
        assert!(caps
            .graph_edge_types
            .contains(&"belongs_to_cluster".to_string()));

        assert!(caps.perception_kinds.contains(&"file_change".to_string()));
        assert!(caps.perception_kinds.contains(&"diagnostic".to_string()));
        assert!(caps.perception_kinds.contains(&"build_result".to_string()));

        assert!(caps.supports_embeddings);
        assert!(caps.supports_search);
    }

    #[test]
    fn test_code_engine_is_object_safe() {
        let _: Box<dyn RealityEngine> = Box::new(CodeRealityEngine);
    }

    #[test]
    fn test_code_engine_metadata_format() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]").unwrap();

        let engine = CodeRealityEngine;
        let result = engine.detect(dir.path()).unwrap();
        assert_eq!(result.metadata["build_system"], "cargo");
        assert_eq!(result.metadata["language"], "rust");
    }

    #[test]
    fn test_code_engine_priority_order() {
        // When multiple markers exist, higher-priority (Cargo.toml) wins over
        // lower-priority (Makefile) because we check in order.
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]").unwrap();
        std::fs::write(dir.path().join("Makefile"), "all:").unwrap();

        let engine = CodeRealityEngine;
        let result = engine.detect(dir.path()).unwrap();
        assert_eq!(
            result.domain, "rust",
            "Cargo.toml should take priority over Makefile"
        );
    }
}
