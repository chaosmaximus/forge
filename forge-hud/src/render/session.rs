use crate::render::colors::*;
use crate::state::HudState;

pub fn render_session_line(state: &HudState, width: usize) -> String {
    let t = &state.tokens;
    let rc = ratio_color(t.deterministic_ratio);
    let pct = (t.deterministic_ratio * 100.0) as u64;
    let tokens = format!("{rc}{}K/{}K tok ({}% det){RESET}", t.input / 1000, t.output / 1000, pct);
    let skills = if state.skills.fix_candidates > 0 {
        format!("{}sk {YELLOW}{}fix{RESET}", state.skills.active, state.skills.fix_candidates)
    } else {
        format!("{}sk", state.skills.active)
    };
    let team = render_team(&state.team);
    if width >= 100 {
        format!("{tokens} {DIM}|{RESET} {skills} {DIM}|{RESET} {team}")
    } else {
        format!("{tokens} {DIM}|{RESET} {team}")
    }
}

fn render_team(team: &std::collections::HashMap<String, crate::state::AgentInfo>) -> String {
    if team.is_empty() { return format!("{DIM}no team{RESET}"); }
    let mut parts = Vec::new();
    for (name, info) in team {
        let short = name.strip_prefix("forge-").unwrap_or(name);
        let icon = match info.status.as_deref() {
            Some("done") => format!("{GREEN}v{RESET}"),
            Some("running") => format!("{YELLOW}*{RESET}"),
            Some("pending") => format!("{DIM}~{RESET}"),
            _ => format!("{DIM}?{RESET}"),
        };
        parts.push(format!("{short}{icon}"));
    }
    parts.join(" ")
}
