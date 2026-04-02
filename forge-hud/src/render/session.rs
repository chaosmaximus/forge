use crate::render::colors::*;
use crate::state::HudState;
use crate::stdin::StdinData;

pub fn render_session_line(stdin: &StdinData, state: &HudState, width: usize) -> String {
    let t = &state.tokens;
    let rc = ratio_color(t.deterministic_ratio);
    let pct = (t.deterministic_ratio * 100.0) as u64;
    let tokens = format!(
        "{rc}{}K/{}K tok ({}% det){RESET}",
        t.input / 1000,
        t.output / 1000,
        pct
    );

    // Rate limits (from claude-hud stdin)
    let rate = render_rate_limits(stdin);

    let skills = if state.skills.fix_candidates > 0 {
        format!("{}sk {YELLOW}{}fix{RESET}", state.skills.active, state.skills.fix_candidates)
    } else {
        format!("{}sk", state.skills.active)
    };

    let team = render_team(&state.team);

    if width >= 160 {
        format!("{tokens} {DIM}|{RESET} {rate}{DIM}|{RESET} {skills} {DIM}|{RESET} {team}")
    } else if width >= 100 {
        format!("{tokens} {DIM}|{RESET} {rate}{skills} {DIM}|{RESET} {team}")
    } else {
        format!("{tokens} {DIM}|{RESET} {team}")
    }
}

fn render_rate_limits(stdin: &StdinData) -> String {
    let five = stdin.rate_limits.five_hour.used_percentage;
    let seven = stdin.rate_limits.seven_day.used_percentage;
    if five <= 0.0 && seven <= 0.0 {
        return String::new();
    }
    let fc = if five >= 80.0 { RED } else if five >= 50.0 { YELLOW } else { GREEN };
    format!("{DIM}Usage{RESET} {fc}{:.0}%{RESET}/{:.0}% ", five, seven)
}

fn render_team(team: &std::collections::HashMap<String, crate::state::AgentInfo>) -> String {
    if team.is_empty() {
        return format!("{DIM}no team{RESET}");
    }
    let mut parts = Vec::new();
    for (name, info) in team {
        let short = sanitize(name.strip_prefix("forge-").unwrap_or(name));
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
