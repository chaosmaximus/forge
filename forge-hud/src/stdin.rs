use serde::Deserialize;
use std::io::{self, Read};

#[derive(Deserialize, Default)]
pub struct StdinData {
    #[serde(default)]
    pub model: ModelInfo,
    #[serde(default)]
    pub context_window: ContextWindow,
    #[serde(default)]
    pub rate_limits: RateLimits,
    #[serde(default)]
    pub cwd: String,
    #[serde(default)]
    pub transcript_path: String,
}

#[derive(Deserialize, Default)]
pub struct ModelInfo {
    #[serde(default)]
    pub display_name: String,
    #[serde(default)]
    pub id: String,
}

#[derive(Deserialize, Default)]
pub struct ContextWindow {
    #[serde(default)]
    pub used_percentage: f64,
    #[serde(default)]
    pub context_window_size: u64,
}

#[derive(Deserialize, Default)]
pub struct RateLimits {
    #[serde(default)]
    pub five_hour: RateLimit,
    #[serde(default)]
    pub seven_day: RateLimit,
}

#[derive(Deserialize, Default)]
pub struct RateLimit {
    #[serde(default)]
    pub used_percentage: f64,
    #[serde(default)]
    pub resets_at: String,
}

pub fn read_stdin() -> StdinData {
    let mut input = String::new();
    if io::stdin().read_to_string(&mut input).is_ok() && !input.is_empty() {
        serde_json::from_str(&input).unwrap_or_default()
    } else {
        StdinData::default()
    }
}

/// Get short git branch name from cwd
pub fn git_branch(cwd: &str) -> String {
    if cwd.is_empty() {
        return String::new();
    }
    let output = std::process::Command::new("git")
        .args(["-C", cwd, "rev-parse", "--abbrev-ref", "HEAD"])
        .output();
    match output {
        Ok(o) if o.status.success() => {
            String::from_utf8_lossy(&o.stdout).trim().to_string()
        }
        _ => String::new(),
    }
}
