use serde::Serialize;

#[derive(Serialize)]
pub struct ReviewContext {
    pub path: String,
    pub base: String,
    pub diff_lines: usize,
    pub files_changed: usize,
    pub files: Vec<String>,
    pub diff_preview: String,
}

pub fn to_json(ctx: &ReviewContext) -> String {
    serde_json::to_string_pretty(ctx).unwrap_or_else(|_| "{}".to_string())
}

pub fn to_markdown(ctx: &ReviewContext) -> String {
    let mut md = format!("## Council Review\n\n**Base:** `{}`\n**Files:** {}\n**Diff lines:** {}\n\n### Files\n\n",
        ctx.base, ctx.files_changed, ctx.diff_lines);
    for f in &ctx.files { md.push_str(&format!("- `{}`\n", f)); }
    md.push_str("\n### Diff Preview\n\n```diff\n");
    md.push_str(&ctx.diff_preview);
    md.push_str("\n```\n");
    md
}
