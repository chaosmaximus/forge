use serde::Deserialize;

#[derive(Deserialize, Default)]
pub struct HudConfig {
    #[serde(default = "default_true")] pub show_graph: bool,
    #[serde(default = "default_true")] pub show_tokens: bool,
    #[serde(default = "default_true")] pub show_team: bool,
    #[serde(default = "default_true")] pub show_security: bool,
}
fn default_true() -> bool { true }
