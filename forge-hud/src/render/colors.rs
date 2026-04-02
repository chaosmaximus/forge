pub const RESET: &str = "\x1b[0m";
pub const BOLD: &str = "\x1b[1m";
pub const DIM: &str = "\x1b[2m";
pub const GREEN: &str = "\x1b[32m";
pub const YELLOW: &str = "\x1b[33m";
pub const RED: &str = "\x1b[31m";
pub const CYAN: &str = "\x1b[36m";
pub const BLUE: &str = "\x1b[34m";
pub const MAGENTA: &str = "\x1b[35m";

pub fn security_color(stale: u64, exposed: u64) -> &'static str {
    if exposed > 0 { RED } else if stale > 0 { YELLOW } else { GREEN }
}
pub fn ratio_color(ratio: f64) -> &'static str {
    if ratio >= 0.8 { GREEN } else if ratio >= 0.5 { YELLOW } else { RED }
}

/// Strip control characters and ANSI escape sequences from untrusted input.
pub fn sanitize(s: &str) -> String {
    s.chars()
        .filter(|c| !c.is_control() || *c == '\n')
        .collect()
}
