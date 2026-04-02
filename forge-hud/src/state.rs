use serde::Deserialize;
use std::collections::HashMap;
use std::fs;
use std::path::Path;

#[derive(Deserialize, Default)]
pub struct HudState {
    #[serde(default)] pub graph: GraphStats,
    #[serde(default)] pub memory: MemoryStats,
    #[serde(default)] pub session: SessionInfo,
    #[serde(default)] pub tokens: TokenStats,
    #[serde(default)] pub skills: SkillStats,
    #[serde(default)] pub team: HashMap<String, AgentInfo>,
    #[serde(default)] pub security: SecurityStats,
}

#[derive(Deserialize, Default)]
#[serde(default)]
pub struct GraphStats { pub nodes: u64, pub edges: u64 }

#[derive(Deserialize, Default)]
#[serde(default)]
pub struct MemoryStats { pub decisions: u64, pub patterns: u64, pub lessons: u64, pub secrets: u64 }

#[derive(Deserialize, Default)]
#[serde(default)]
pub struct SessionInfo { pub mode: Option<String>, pub phase: Option<String>, pub wave: Option<String> }

#[derive(Deserialize, Default)]
#[serde(default)]
pub struct TokenStats { pub input: u64, pub output: u64, pub llm_calls: u64, pub deterministic_ratio: f64 }

#[derive(Deserialize, Default)]
#[serde(default)]
pub struct SkillStats { pub active: u64, pub fix_candidates: u64 }

#[derive(Deserialize, Default)]
#[serde(default)]
pub struct AgentInfo { pub status: Option<String>, pub last_tool: Option<String>, pub current_file: Option<String> }

#[derive(Deserialize, Default)]
#[serde(default)]
pub struct SecurityStats { pub total: u64, pub stale: u64, pub exposed: u64 }

pub fn read_state(state_dir: &str) -> HudState {
    let path = Path::new(state_dir).join("hud").join("hud-state.json");
    match fs::read_to_string(&path) {
        Ok(content) => serde_json::from_str(&content).unwrap_or_default(),
        Err(_) => HudState::default(),
    }
}
