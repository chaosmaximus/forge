use serde::Deserialize;
use std::io::{self, Read};

#[derive(Deserialize, Default)]
pub struct StdinData {
    #[serde(default)]
    pub model: ModelInfo,
    #[serde(default)]
    pub context_window: ContextWindow,
    #[serde(default)]
    pub cwd: String,
}

#[derive(Deserialize, Default)]
pub struct ModelInfo {
    #[serde(default)]
    pub display_name: String,
}

#[derive(Deserialize, Default)]
pub struct ContextWindow {
    #[serde(default)]
    pub used_percentage: f64,
}

pub fn read_stdin() -> StdinData {
    let mut input = String::new();
    if io::stdin().read_to_string(&mut input).is_ok() && !input.is_empty() {
        serde_json::from_str(&input).unwrap_or_default()
    } else {
        StdinData::default()
    }
}
