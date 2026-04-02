pub mod colors;
pub mod graph;
pub mod session;

use crate::state::HudState;
use crate::stdin::StdinData;

pub fn render(stdin: &StdinData, state: &HudState) -> String {
    let width = std::env::var("COLUMNS").ok().and_then(|s| s.parse().ok()).unwrap_or(120);
    let line1 = graph::render_line1(stdin, width);
    let line2 = session::render_line2(state, width);
    format!("{line1}\n{line2}")
}
