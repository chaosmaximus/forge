use crate::render::colors::*;
use crate::stdin::StdinData;

/// Line 2: context bar + usage bars with time remaining
///   Context █░░░░░░░░░ 11% │ Usage █░░░░░░░░░ 5% (3h 37m / 5h) | ░░░░░░░░░░ 1% (3d 21h / 7d)
pub fn render_line2(stdin: &StdinData, _width: usize) -> String {
    let sep = "\u{2502}"; // │
    let ctx = render_context_bar(stdin.context_pct());
    let usage_5h = render_usage_bar(stdin.rate_5h(), stdin.rate_5h_resets_at(), "5h");
    let usage_7d = render_usage_bar(stdin.rate_7d(), stdin.rate_7d_resets_at(), "7d");

    format!("  {ctx} {DIM}{sep}{RESET} {DIM}Usage{RESET} {usage_5h} {DIM}|{RESET} {usage_7d}")
}

fn render_context_bar(pct: f64) -> String {
    let p = pct.clamp(0.0, 100.0);
    let filled = (p / 10.0).round() as usize;
    let empty = 10_usize.saturating_sub(filled);
    let color = context_color(p);
    let bar_str = format!(
        "{}{}",
        "\u{2588}".repeat(filled),
        "\u{2591}".repeat(empty),
    );
    format!("{DIM}Context{RESET} {color}{bar_str}{RESET} {color}{:.0}%{RESET}", p)
}

fn render_usage_bar(pct: f64, resets_at: Option<f64>, window: &str) -> String {
    let p = pct.clamp(0.0, 100.0);
    let filled = (p / 10.0).round() as usize;
    let empty = 10_usize.saturating_sub(filled);
    let color = usage_color(p);
    let bar_str = format!(
        "{}{}",
        "\u{2588}".repeat(filled),
        "\u{2591}".repeat(empty),
    );
    let time_str = match resets_at {
        Some(ts) if ts > 0.0 => {
            let remaining = crate::stdin::format_time_remaining(ts);
            format!(" ({remaining} / {window})")
        }
        _ => format!(" (- / {window})"),
    };
    format!("{color}{bar_str}{RESET} {color}{:.0}%{RESET}{DIM}{time_str}{RESET}", p)
}

fn context_color(pct: f64) -> &'static str {
    if pct >= 85.0 { RED } else if pct >= 70.0 { YELLOW } else { GREEN }
}

fn usage_color(pct: f64) -> &'static str {
    if pct >= 90.0 { RED } else if pct >= 75.0 { BRIGHT_MAGENTA } else { BRIGHT_BLUE }
}
