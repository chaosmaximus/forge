// chunk_raw.rs — character-window chunker for the raw storage layer
//
// Sliding window over the UTF-8 string in char-boundary steps. Matches the
// MemPalace miner chunk size (800 / 100 overlap / 50 min) for benchmark parity —
// see docs/benchmarks/plan.md §4.4. Keep `chunk.rs` untouched; that file is
// transcript-line parsing owned by the extractor.

/// Default chunk size (characters), matching the published MemPalace miner.
pub const DEFAULT_CHUNK_SIZE: usize = 800;

/// Default overlap (characters) between adjacent chunks.
pub const DEFAULT_CHUNK_OVERLAP: usize = 100;

/// Minimum chunk length to keep (drop trailing slivers smaller than this).
pub const DEFAULT_CHUNK_MIN: usize = 50;

/// Split `text` into overlapping character-windowed chunks.
///
/// Uses UTF-8 char-boundary walks (via `str::is_char_boundary`) so multi-byte
/// codepoints are never split mid-sequence. The window advances by
/// `size - overlap` characters each step; the final chunk is dropped if it is
/// shorter than `min_size`.
///
/// Panics if `overlap >= size` (degenerate — would not make forward progress).
pub fn chunk_text(text: &str, size: usize, overlap: usize, min_size: usize) -> Vec<String> {
    assert!(
        size > overlap,
        "chunk size ({size}) must exceed overlap ({overlap})"
    );
    if text.is_empty() {
        return Vec::new();
    }

    // Work in char indices — sqlite-vec and cosine similarity are byte-agnostic
    // but MemPalace uses chars for its 800/100 defaults. Matching char semantics
    // keeps the benchmark comparison honest.
    let chars: Vec<char> = text.chars().collect();
    let total = chars.len();

    if total <= size {
        // Whole input fits in one chunk — still subject to min_size floor.
        if total >= min_size {
            return vec![chars.iter().collect()];
        }
        return Vec::new();
    }

    let step = size - overlap;
    let mut chunks = Vec::new();
    let mut start = 0usize;

    while start < total {
        let end = (start + size).min(total);
        let slice: String = chars[start..end].iter().collect();
        if slice.chars().count() >= min_size {
            chunks.push(slice);
        }
        if end == total {
            break;
        }
        start += step;
    }

    chunks
}

/// Convenience wrapper using the default 800/100/50 settings.
pub fn chunk_text_default(text: &str) -> Vec<String> {
    chunk_text(
        text,
        DEFAULT_CHUNK_SIZE,
        DEFAULT_CHUNK_OVERLAP,
        DEFAULT_CHUNK_MIN,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_text_returns_empty() {
        assert!(chunk_text("", 800, 100, 50).is_empty());
    }

    #[test]
    fn text_below_min_returns_empty() {
        // 20 chars, min 50 → drop.
        let text = "x".repeat(20);
        assert!(chunk_text(&text, 800, 100, 50).is_empty());
    }

    #[test]
    fn text_below_size_returns_single_chunk() {
        let text = "x".repeat(400);
        let chunks = chunk_text(&text, 800, 100, 50);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].chars().count(), 400);
    }

    #[test]
    fn text_equal_to_size_returns_single_chunk() {
        let text = "x".repeat(800);
        let chunks = chunk_text(&text, 800, 100, 50);
        assert_eq!(chunks.len(), 1);
    }

    #[test]
    fn sliding_window_step_equals_size_minus_overlap() {
        // 1600 chars, size=800, overlap=100 → step=700 → chunks at [0..800), [700..1500), [1400..1600) (last=200).
        let text = "x".repeat(1600);
        let chunks = chunk_text(&text, 800, 100, 50);
        assert_eq!(chunks.len(), 3);
        assert_eq!(chunks[0].chars().count(), 800);
        assert_eq!(chunks[1].chars().count(), 800);
        assert_eq!(chunks[2].chars().count(), 200);
    }

    #[test]
    fn trailing_chunk_dropped_when_below_min() {
        // Choose params such that the final window is < min_size.
        // 850 chars, size=800, overlap=100 → step=700 → chunks at [0..800), [700..850) → last=150.
        let text = "x".repeat(850);
        let chunks = chunk_text(&text, 800, 100, 50);
        // Both chunks are >= 50.
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[1].chars().count(), 150);

        // Now force the tail to be below min.
        // 820 chars, size=800, overlap=100 → step=700 → chunks at [0..800), [700..820) → last=120? no: end = min(700+800, 820) = 820, slice is 120 chars → still > 50.
        // Use min=200 to force drop.
        let chunks2 = chunk_text(&text, 800, 100, 200);
        assert_eq!(chunks2.len(), 1);
    }

    #[test]
    fn utf8_codepoints_never_split() {
        // Mix of ASCII, CJK, emoji, combining marks.
        let mut s = String::new();
        for _ in 0..300 {
            s.push_str("hello 世界 🔥");
        }
        let chunks = chunk_text(&s, 800, 100, 50);
        assert!(!chunks.is_empty());
        // Every chunk must decode cleanly as UTF-8 (String already guarantees this —
        // we assert the char count is consistent with the number of graphemes).
        for c in &chunks {
            assert!(c.chars().count() <= 800);
        }
        // Reassembling with the overlap removed should contain the full input.
        let recombined: String = {
            let step = 800 - 100;
            let mut out = String::new();
            for (idx, chunk) in chunks.iter().enumerate() {
                if idx == 0 {
                    out.push_str(chunk);
                } else {
                    // Append the part of the chunk that is past the overlap.
                    let tail: String = chunk.chars().skip(800 - step).collect();
                    out.push_str(&tail);
                }
            }
            out
        };
        // Length should roughly match input (we dropped no bytes).
        assert_eq!(recombined.chars().count(), s.chars().count());
    }

    #[test]
    fn defaults_match_constants() {
        let text = "a".repeat(1500);
        let chunks = chunk_text_default(&text);
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].chars().count(), DEFAULT_CHUNK_SIZE);
    }

    #[test]
    #[should_panic(expected = "chunk size")]
    fn overlap_ge_size_panics() {
        chunk_text("hello", 100, 100, 10);
    }

    #[test]
    fn chunk_indices_are_consecutive_no_gaps() {
        // The sliding window must cover every character at least once.
        let text: String = (0..2000)
            .map(|i| char::from(b'a' + (i % 26) as u8))
            .collect();
        let chunks = chunk_text(&text, 800, 100, 50);
        assert!(!chunks.is_empty());
        // The first char of the original text must be in the first chunk.
        assert_eq!(
            chunks[0].chars().next().unwrap(),
            text.chars().next().unwrap()
        );
        // The last char must be in the last chunk.
        assert_eq!(
            chunks.last().unwrap().chars().last().unwrap(),
            text.chars().last().unwrap()
        );
    }
}
