//! Small CLI rendering helpers shared across `commands::*` modules.

/// Char-boundary-safe truncation. Returns the input verbatim when its byte
/// length fits in `max_bytes`; otherwise returns a `String` containing as much
/// of the prefix as fits (rounded down to the nearest UTF-8 codepoint
/// boundary) followed by `…` (U+2026 horizontal ellipsis, 3 bytes).
///
/// Pre-Phase-10D (audit B-MED-2) the call sites used raw byte-slice
/// indexing (`&s[..80]`) which panics when `80` lands inside a multibyte
/// codepoint — e.g. a Japanese identifier name in `agent-template get` or
/// a Cyrillic message body in `messages` would crash the entire CLI
/// instead of rendering a preview.
pub fn truncate_preview(s: &str, max_bytes: usize) -> String {
    if s.len() <= max_bytes {
        return s.to_string();
    }
    // Find the largest byte index ≤ max_bytes that lies on a char boundary.
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    let mut out = String::with_capacity(end + 3);
    out.push_str(&s[..end]);
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ascii_under_limit_unchanged() {
        assert_eq!(truncate_preview("hello", 10), "hello");
    }

    #[test]
    fn ascii_over_limit_truncated() {
        assert_eq!(truncate_preview("hello world", 5), "hello…");
    }

    #[test]
    fn multibyte_does_not_panic_at_boundary() {
        // "日本語" = 3 chars × 3 bytes = 9 bytes. Asking for 4 bytes
        // would land mid-codepoint with raw slicing — must round down to 3.
        let s = "日本語";
        let out = truncate_preview(s, 4);
        assert_eq!(out, "日…");
    }

    #[test]
    fn cyrillic_preview_safe() {
        // Each Cyrillic letter is 2 bytes. 5 bytes → round down to 4 → 2 chars.
        let s = "Привет, мир";
        let out = truncate_preview(s, 5);
        assert_eq!(out, "Пр…");
    }

    #[test]
    fn empty_input_passes_through() {
        assert_eq!(truncate_preview("", 80), "");
    }

    #[test]
    fn exact_boundary_is_unchanged() {
        // Exact length match — no truncation, no ellipsis.
        assert_eq!(truncate_preview("abc", 3), "abc");
    }
}
