use crate::render::colors::*;
use crate::stdin::StdinData;

/// Line 1: Model, context bar, git branch, rate limits
/// This line is about Claude Code state — same info claude-hud showed
pub fn render_line1(stdin: &StdinData, width: usize) -> String {
    let model = sanitize(&stdin.model_name());
    let ctx = render_context_bar(stdin.context_pct());
    let branch = crate::stdin::git_branch(stdin.cwd_str());
    let git_seg = if !branch.is_empty() {
        let b = sanitize(&branch);
        format!(" {DIM}git:{RESET}{CYAN}{b}{RESET}")
    } else {
        String::new()
    };
    let rate = render_rate(stdin.rate_5h(), stdin.rate_7d());

    if width >= 140 {
        format!("{BOLD}[{model}]{RESET} {ctx}{git_seg} {DIM}|{RESET} {rate}")
    } else if width >= 100 {
        format!("{BOLD}[{model}]{RESET} {ctx}{git_seg}")
    } else {
        format!("{BOLD}[{model}]{RESET} {ctx}")
    }
}

fn render_context_bar(pct: f64) -> String {
    let p = pct.clamp(0.0, 100.0);
    let filled = (p / 10.0).round() as usize;
    let empty = 10_usize.saturating_sub(filled);
    let color = if p >= 85.0 { RED } else if p >= 60.0 { YELLOW } else { GREEN };
    format!(
        "{DIM}Context{RESET} {color}[{}{}]{RESET} {color}{:.0}%{RESET}",
        "█".repeat(filled),
        format!("{DIM}{}{}", "░".repeat(empty), color),
        p
    )
}

fn render_rate(five: f64, seven: f64) -> String {
    if five <= 0.0 && seven <= 0.0 {
        return String::new();
    }
    let fc = if five >= 80.0 { RED } else if five >= 50.0 { YELLOW } else { GREEN };
    let sc = if seven >= 80.0 { RED } else if seven >= 50.0 { YELLOW } else { GREEN };
    format!("{DIM}Usage{RESET} {fc}{:.0}%{RESET} {DIM}(5h){RESET} {sc}{:.0}%{RESET} {DIM}(7d){RESET}", five, seven)
}
