pub mod colors;
pub mod graph;
pub mod session;
pub mod usage;

use crate::state::HudState;
use crate::stdin::StdinData;

pub fn render(stdin: &StdinData, state: &HudState) -> String {
    let width = std::env::var("COLUMNS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(120);
    let line1 = graph::render_line1(stdin, state, width);
    let line2 = usage::render_line2(stdin, width);
    let line3 = session::render_line3(stdin, state, width);
    format!("{line1}\n{line2}\n{line3}")
}
