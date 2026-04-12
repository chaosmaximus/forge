use serde::Deserialize;
use std::io::{self, Read};

#[derive(Deserialize, Default)]
pub struct StdinData {
    #[serde(default)]
    pub model: Option<ModelInfo>,
    #[serde(default)]
    pub context_window: Option<ContextWindow>,
    #[serde(default)]
    pub rate_limits: Option<RateLimits>,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub transcript_path: Option<String>,
}

#[derive(Deserialize, Default)]
pub struct ModelInfo {
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub id: Option<String>,
}

#[derive(Deserialize, Default)]
pub struct ContextWindow {
    #[serde(default)]
    pub context_window_size: Option<u64>,
    #[serde(default)]
    pub used_percentage: Option<f64>,
    #[serde(default)]
    pub remaining_percentage: Option<f64>,
    #[serde(default)]
    pub current_usage: Option<CurrentUsage>,
}

#[derive(Deserialize, Default)]
pub struct CurrentUsage {
    #[serde(default)]
    pub input_tokens: Option<u64>,
    #[serde(default)]
    pub output_tokens: Option<u64>,
    #[serde(default)]
    pub cache_creation_input_tokens: Option<u64>,
    #[serde(default)]
    pub cache_read_input_tokens: Option<u64>,
}

#[derive(Deserialize, Default)]
pub struct RateLimits {
    #[serde(default)]
    pub five_hour: Option<RateLimit>,
    #[serde(default)]
    pub seven_day: Option<RateLimit>,
}

#[derive(Deserialize, Default)]
pub struct RateLimit {
    #[serde(default)]
    pub used_percentage: Option<f64>,
    #[serde(default)]
    pub resets_at: Option<f64>,
}

impl StdinData {
    pub fn model_name(&self) -> String {
        self.model.as_ref()
            .and_then(|m| m.display_name.as_ref())
            .cloned()
            .unwrap_or_else(|| "Claude".to_string())
    }

    pub fn context_pct(&self) -> f64 {
        // Prefer native used_percentage (v2.1.6+)
        if let Some(cw) = &self.context_window {
            if let Some(pct) = cw.used_percentage {
                if pct > 0.0 {
                    return pct.clamp(0.0, 100.0);
                }
            }
            // Fallback: manual calc from token counts
            if let (Some(size), Some(usage)) = (cw.context_window_size, &cw.current_usage) {
                if size > 0 {
                    let total = usage.input_tokens.unwrap_or(0)
                        + usage.cache_creation_input_tokens.unwrap_or(0)
                        + usage.cache_read_input_tokens.unwrap_or(0);
                    return ((total as f64 / size as f64) * 100.0).clamp(0.0, 100.0);
                }
            }
        }
        0.0
    }

    pub fn rate_5h(&self) -> f64 {
        self.rate_limits.as_ref()
            .and_then(|r| r.five_hour.as_ref())
            .and_then(|r| r.used_percentage)
            .unwrap_or(0.0)
    }

    pub fn rate_7d(&self) -> f64 {
        self.rate_limits.as_ref()
            .and_then(|r| r.seven_day.as_ref())
            .and_then(|r| r.used_percentage)
            .unwrap_or(0.0)
    }

    pub fn cwd_str(&self) -> &str {
        self.cwd.as_deref().unwrap_or("")
    }

    /// Project name: last path segment of cwd
    pub fn project_name(&self) -> String {
        let cwd = self.cwd_str();
        if cwd.is_empty() {
            return String::new();
        }
        std::path::Path::new(cwd)
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default()
    }

    /// Detect plan name from environment. API key present -> "API", else "Max".
    pub fn plan_name(&self) -> &'static str {
        if std::env::var("ANTHROPIC_API_KEY").map(|v| !v.is_empty()).unwrap_or(false) {
            "API"
        } else {
            "Max"
        }
    }

    /// Epoch seconds when 5h rate limit resets
    pub fn rate_5h_resets_at(&self) -> Option<f64> {
        self.rate_limits.as_ref()
            .and_then(|r| r.five_hour.as_ref())
            .and_then(|r| r.resets_at)
    }

    /// Epoch seconds when 7d rate limit resets
    pub fn rate_7d_resets_at(&self) -> Option<f64> {
        self.rate_limits.as_ref()
            .and_then(|r| r.seven_day.as_ref())
            .and_then(|r| r.resets_at)
    }
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
    if cwd.is_empty() { return String::new(); }
    let output = std::process::Command::new("git")
        .args(["-C", cwd, "rev-parse", "--abbrev-ref", "HEAD"])
        .output();
    match output {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).trim().to_string(),
        _ => String::new(),
    }
}

/// Check if git working tree is dirty (uncommitted changes)
pub fn git_dirty(cwd: &str) -> bool {
    if cwd.is_empty() { return false; }
    let output = std::process::Command::new("git")
        .args(["-C", cwd, "status", "--porcelain"])
        .output();
    match output {
        Ok(o) if o.status.success() => !o.stdout.is_empty(),
        _ => false,
    }
}

/// Format seconds remaining as human-readable string.
/// Uses "Xh Ym" for < 1 day, "Xd Yh" for >= 1 day.
pub fn format_time_remaining(resets_at: f64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0);
    let remaining_secs = (resets_at - now).max(0.0) as u64;
    let days = remaining_secs / 86400;
    let hours = (remaining_secs % 86400) / 3600;
    let minutes = (remaining_secs % 3600) / 60;
    if days > 0 {
        format!("{days}d {hours}h")
    } else {
        format!("{hours}h {minutes}m")
    }
}
