use serde::{Deserialize, Serialize};
use std::path::Path;

/// Result of auto-detecting what kind of project a given path represents.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DetectionResult {
    pub confidence: f64,
    pub detected_from: String,
    pub domain: String,
    pub reality_type: String,
    pub metadata: serde_json::Value,
}

/// Capabilities that a Project Engine advertises.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EngineCapabilities {
    pub graph_node_types: Vec<String>,
    pub graph_edge_types: Vec<String>,
    pub perception_kinds: Vec<String>,
    pub supports_embeddings: bool,
    pub supports_search: bool,
}

/// Summary of an indexing run.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct IndexResult {
    pub files_indexed: usize,
    pub symbols_indexed: usize,
    pub edges_created: usize,
    pub duration_ms: u64,
}

/// A search hit from the Project Engine.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SearchHit {
    pub id: String,
    pub kind: String,
    pub path: String,
    pub name: String,
    pub score: f64,
    pub context: String,
}

/// Project Engine's contribution to blast radius analysis.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct BlastRadiusContribution {
    pub callers: usize,
    pub calling_files: Vec<String>,
    pub cluster_name: Option<String>,
    pub cluster_files: Vec<String>,
}

/// The core trait for domain-specific perception engines.
///
/// Lives in forge-core (not forge-daemon) so future Tier 2 (subprocess+gRPC)
/// and Tier 3 (WASM) engines can implement it without daemon dependencies.
///
/// The first implementation is CodeProjectEngine (code analysis via LSP + regex).
/// Future engines: SensorProjectEngine, MedicalProjectEngine, DataProjectEngine.
pub trait ProjectEngine: Send + Sync {
    /// Engine name (e.g., "code", "sensor", "medical")
    fn name(&self) -> &str;

    /// Engine version (semver)
    fn version(&self) -> &str;

    /// Engine kind matching the project table's reality_type column
    fn reality_type(&self) -> &str;

    /// Detect if this engine can handle the given project path.
    /// Returns None if the path doesn't match this engine's domain.
    fn detect(&self, path: &Path) -> Option<DetectionResult>;

    /// Advertise what this engine can produce.
    fn capabilities(&self) -> EngineCapabilities;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Verify that the ProjectEngine trait is object-safe
    /// (can be used as Box<dyn ProjectEngine>).
    struct MockEngine;

    impl ProjectEngine for MockEngine {
        fn name(&self) -> &str {
            "mock"
        }
        fn version(&self) -> &str {
            "0.1.0"
        }
        fn reality_type(&self) -> &str {
            "mock"
        }
        fn detect(&self, _path: &Path) -> Option<DetectionResult> {
            None
        }
        fn capabilities(&self) -> EngineCapabilities {
            EngineCapabilities {
                graph_node_types: vec![],
                graph_edge_types: vec![],
                perception_kinds: vec![],
                supports_embeddings: false,
                supports_search: false,
            }
        }
    }

    #[test]
    fn test_project_engine_is_object_safe() {
        let _: Box<dyn ProjectEngine> = Box::new(MockEngine);
    }

    #[test]
    fn test_detection_result_serialization_roundtrip() {
        let result = DetectionResult {
            confidence: 0.95,
            detected_from: "Cargo.toml".to_string(),
            domain: "rust".to_string(),
            reality_type: "code".to_string(),
            metadata: serde_json::json!({"build_system": "cargo"}),
        };
        let json = serde_json::to_string(&result).unwrap();
        let decoded: DetectionResult = serde_json::from_str(&json).unwrap();
        assert_eq!(result, decoded);
    }

    #[test]
    fn test_engine_capabilities_serialization_roundtrip() {
        let caps = EngineCapabilities {
            graph_node_types: vec!["file".into(), "function".into()],
            graph_edge_types: vec!["calls".into()],
            perception_kinds: vec!["file_change".into()],
            supports_embeddings: true,
            supports_search: true,
        };
        let json = serde_json::to_string(&caps).unwrap();
        let decoded: EngineCapabilities = serde_json::from_str(&json).unwrap();
        assert_eq!(caps, decoded);
    }
}
