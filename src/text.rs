//! Small text helpers shared by the TUI, tools and workspace loader.

/// Cut `s` to at most `max` bytes, landing on a char boundary.
/// Returns `None` when `s` already fits.
///
/// A fixed byte offset (`&s[..8000]`) panics whenever a multi-byte char
/// straddles it, which is the normal case for UTF-8 source files.
pub fn truncate_bytes(s: &str, max: usize) -> Option<&str> {
    if s.len() <= max {
        return None;
    }
    let mut end = max.min(s.len());
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    Some(&s[..end])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_string_is_not_truncated() {
        assert_eq!(truncate_bytes("hello", 8), None);
        assert_eq!(truncate_bytes("hello", 5), None);
    }

    #[test]
    fn cut_lands_on_char_boundary() {
        // "aaéaa": a a é(2 bytes) a a -> len 6. Cuts at 3/5 would split é.
        let s = "aaéaa";
        assert_eq!(s.len(), 6);
        assert_eq!(truncate_bytes(s, 3), Some("aa"));
        assert_eq!(truncate_bytes(s, 4), Some("aaé"));
        assert_eq!(truncate_bytes(s, 5), Some("aaéa"));
    }

    #[test]
    fn multibyte_stress_never_panics() {
        let s: String = "é".repeat(4000);
        for max in 0..s.len() + 2 {
            if let Some(head) = truncate_bytes(&s, max) {
                assert!(head.len() <= max, "max={} got {}", max, head.len());
            }
        }
    }

    #[test]
    fn emoji_is_not_split() {
        let s = "hi 🎉🎉🎉";
        let head = truncate_bytes(s, 8).expect("truncated");
        assert!(s.starts_with(head));
        assert!(head.chars().all(|c| c != '\u{fffd}'));
    }
}
