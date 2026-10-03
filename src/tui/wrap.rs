//! Physical-row accounting for the chat transcript.
//!
//! `Paragraph` wraps at render time and `Paragraph::scroll` skips *wrapped*
//! rows, so bottom-follow scrolling has to be measured in physical rows too —
//! counting logical lines left the newest text off-screen as soon as a
//! paragraph wrapped.
//!
//! `Paragraph::line_count(width)` is the exact answer, but it is gated behind
//! ratatui's `unstable-rendered-line-info` feature. This module is therefore a
//! width-only port of the same state machine (`ratatui-widgets/src/reflow.rs`,
//! `WordWrapper::process_input` with `trim = false`), so the shipped build
//! needs no unstable API. `oracle_matches_ratatui` below checks the port
//! against `line_count` itself.

use ratatui::text::Line;
use std::collections::VecDeque;
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

/// Non-breaking space: ratatui does not treat it as a wrap opportunity.
const NBSP: &str = "\u{00a0}";
/// Zero-width space: ratatui treats it as whitespace.
const ZWSP: &str = "\u{200b}";

fn is_wrap_whitespace(symbol: &str) -> bool {
    symbol == ZWSP || (symbol.chars().all(char::is_whitespace) && symbol != NBSP)
}

/// ratatui's `CellWidth for str`: a single-byte grapheme is one cell; wider
/// clusters come from unicode-width plus the halfwidth dakuten/handakuten that
/// terminals really do paint in a cell of their own.
fn cell_width(symbol: &str) -> usize {
    if symbol.len() == 1 {
        return 1;
    }
    let marks = symbol
        .chars()
        .filter(|c| matches!(*c, '\u{FF9E}' | '\u{FF9F}'))
        .count();
    UnicodeWidthStr::width(symbol) + marks
}

/// Physical rows a single logical line occupies once wrapped at `max_width`.
fn rows_for_line(text: &str, max_width: u16) -> usize {
    if max_width == 0 {
        return 0;
    }
    let max = max_width as usize;
    let mut rows = 0usize;
    let mut line_width = 0usize; // committed to the row being built
    let mut word_width = 0usize; // pending word
    let mut space_width = 0usize; // total pending whitespace
    let mut pending_space: VecDeque<usize> = VecDeque::new();
    // Mirrors `pending_line.is_empty()`: whitespace alone still fills a row,
    // but committing a row hands the buffer over and empties it again.
    let mut line_has_content = false;
    let mut prev_was_word = false;

    for symbol in text.graphemes(true) {
        // `graphemes` over a `&str` yields `&str` items. `Span::styled_graphemes`
        // drops control characters (tab, CR) before wrapping, so we do too.
        if symbol.chars().any(char::is_control) {
            continue;
        }
        let width = cell_width(symbol);
        if width > max {
            continue; // ratatui drops symbols wider than a whole row
        }
        let is_space = is_wrap_whitespace(symbol);

        // A word just ended, or the pending word no longer fits on an empty
        // row. With trim = false whitespace is kept, so it counts toward width.
        let word_found = prev_was_word && is_space;
        let untrimmed_overflow = line_width == 0 && word_width + space_width + width > max;
        if word_found || untrimmed_overflow {
            line_width += pending_space.drain(..).sum::<usize>() + word_width;
            line_has_content = true;
            pending_space.clear();
            space_width = 0;
            word_width = 0;
        }

        let row_is_full = line_width >= max;
        let word_would_overflow = width > 0 && line_width + space_width + word_width >= max;
        if row_is_full || word_would_overflow {
            // Whitespace that still fits on the row being closed stays there.
            let mut room = max.saturating_sub(line_width);
            while let Some(&w) = pending_space.front() {
                if w > room {
                    break;
                }
                pending_space.pop_front();
                space_width -= w;
                room -= w;
            }
            rows += 1;
            line_width = 0;
            line_has_content = false;
            // The first space of the next row is not charged to that word.
            if is_space && pending_space.is_empty() {
                continue;
            }
        }

        if is_space {
            space_width += width;
            pending_space.push_back(width);
            line_has_content = true;
        } else {
            word_width += width;
        }
        prev_was_word = !is_space;
    }

    if line_has_content || word_width > 0 {
        rows += 1;
    }
    // ratatui emits at least one row per logical line, even an empty one.
    rows.max(1)
}

/// Physical rows one plain string occupies at `width`. Same accounting as
/// `wrapped_row_count`, for callers that hold text rather than `Line`s.
pub fn physical_rows(text: &str, width: u16) -> usize {
    rows_for_line(text, width)
}

/// Greedy word wrap of plain text at `width` cells, returning the rows.
/// Every row keeps the source line's leading indent (a wrapped shell command
/// or diff is unreadable without it) and words longer than a row are
/// hard-broken. Always returns at least one row.
pub fn wrap_text(text: &str, width: u16) -> Vec<String> {
    wrap_plain(text, width)
}

/// Physical rows a plain multi-line string occupies at `width`.
pub fn text_rows(text: &str, width: u16) -> usize {
    text.split('\n').map(|l| rows_for_line(l, width)).sum()
}

/// Greedy word wrap of plain text at `width` cells, one output row per line of
/// input. Every row keeps the source line's leading indent (a wrapped shell
/// command or diff is unreadable without it) and words longer than a row are
/// hard-broken. Always returns at least one row.
pub fn wrap_plain(text: &str, width: u16) -> Vec<String> {
    let max = width.max(1) as usize;
    let mut out: Vec<String> = Vec::new();
    for line in text.split('\n') {
        let indent: String = line
            .chars()
            .take_while(|c| matches!(c, ' ' | '\t'))
            .collect();
        let body = line.trim_start();
        if body.is_empty() {
            out.push(indent);
            continue;
        }
        // An indent at least as wide as the row would push every row past the
        // edge, so drop it rather than emit over-wide lines.
        let indent = if indent.width() >= max {
            String::new()
        } else {
            indent
        };
        let avail = max.saturating_sub(indent.width()).max(1);
        let mut row = indent.clone();
        let mut chunk = String::new();
        let mut chunk_w = 0usize;
        for word in body.split_whitespace() {
            for piece in hard_break(word, avail) {
                let need = usize::from(!chunk.is_empty());
                if !chunk.is_empty() && chunk_w + need + piece.width() > avail {
                    row.push_str(&chunk);
                    out.push(std::mem::take(&mut row));
                    row.push_str(&indent);
                    chunk.clear();
                    chunk_w = 0;
                }
                if !chunk.is_empty() {
                    chunk.push(' ');
                    chunk_w += 1;
                }
                chunk.push_str(piece);
                chunk_w += piece.width();
            }
        }
        row.push_str(&chunk);
        out.push(row);
    }
    if out.is_empty() {
        out.push(String::new());
    }
    out
}

/// Split one word into slices that each fit `avail` display cells.
fn hard_break(word: &str, avail: usize) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0usize;
    let mut w = 0usize;
    for (i, g) in word.char_indices() {
        let gw = UnicodeWidthChar::width(g).unwrap_or(0);
        if w + gw > avail && i > start {
            out.push(&word[start..i]);
            start = i;
            w = gw;
        } else {
            w += gw;
        }
    }
    out.push(&word[start..]);
    out
}

/// Physical rows `Paragraph::new(lines).wrap(Wrap { trim: false })` renders at
/// `width` — the unit `Paragraph::scroll` and `chat_scroll` both count in.
///
/// Called on every frame, so the span buffer is reused and single-span lines
/// (most markdown output) are measured without copying.
pub fn wrapped_row_count<'a, I>(lines: I, width: u16) -> usize
where
    I: IntoIterator<Item = &'a Line<'a>>,
{
    let mut buf = String::new();
    let mut total = 0usize;
    for line in lines {
        if let [only] = &line.spans[..] {
            total += rows_for_line(only.content.as_ref(), width);
            continue;
        }
        buf.clear();
        for span in &line.spans {
            buf.push_str(span.content.as_ref());
        }
        total += rows_for_line(&buf, width);
    }
    total
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::widgets::{Paragraph, Wrap};

    /// The oracle: ratatui's own wrapped-row count. Available because the
    /// dev-dependency enables `unstable-rendered-line-info`.
    fn ratatui_rows(lines: &[Line], width: u16) -> usize {
        Paragraph::new(lines.to_vec())
            .wrap(Wrap { trim: false })
            .line_count(width)
    }

    fn plain(text: &str) -> Line<'static> {
        Line::from(text.to_string())
    }

    fn check(text: &str, width: u16) {
        let lines = vec![plain(text)];
        assert_eq!(
            wrapped_row_count(&lines, width),
            ratatui_rows(&lines, width),
            "row count mismatch for {:?} @ width {}",
            text,
            width
        );
    }

    #[test]
    fn oracle_matches_ratatui_on_prose() {
        for width in [1u16, 2, 7, 20, 40, 80] {
            check("", width);
            check("short", width);
            check(
                "a longer sentence that needs to wrap somewhere in the middle",
                width,
            );
            check(&"word ".repeat(60), width);
            check("supercalifragilisticexpialidocious", width);
            check(&"x".repeat(200), width);
        }
    }

    #[test]
    fn oracle_matches_ratatui_on_markdown_shapes() {
        for width in [4u16, 12, 33, 80] {
            check("  - nested list item that is fairly long indeed", width);
            check(
                "      - deeply indented item that is fairly long indeed",
                width,
            );
            check(
                "- [ ] todo item that keeps going and going and going",
                width,
            );
            check(
                "1. ordered item that keeps going and going and going",
                width,
            );
            check("▎ 12 let x = compute(something, deeply, here);", width);
            check("│ 1 │ a │ b │", width);
            check(
                "> quoted line that is long enough to need a wrap here",
                width,
            );
            check("multiple    spaces     between      words", width);
            check("trailing whitespace   ", width);
            check("   leading whitespace", width);
        }
    }

    #[test]
    fn oracle_matches_ratatui_on_wide_and_odd_chars() {
        for width in [3u16, 10, 41] {
            check("CJK 日本語のテキストは折り返す", width);
            check("emoji 🎉 family 👨‍👩‍👧 done", width);
            check("combining é vs é equal", width);
            check("nbsp\u{00a0}joined\u{00a0}words", width);
            check("zwsp\u{200b}split\u{200b}words", width);
            check("tab\tseparated\tcolumns here", width);
        }
    }

    #[test]
    fn oracle_matches_ratatui_on_realistic_transcript() {
        let mut lines: Vec<Line> = vec![plain(
            "Halo! Ini penjelasan yang cukup panjang untuk memaksa wrapping.",
        )];
        lines.push(plain(""));
        lines.push(plain("- poin pertama yang panjang agar wrap terjadi"));
        lines.push(plain("- poin kedua juga panjang supaya baris membungkus"));
        lines.push(plain(""));
        for i in 1..40 {
            lines.push(plain(&format!(
                "▎ {i:>2} let x{i} = compute({i}, deeply, nested);"
            )));
        }
        lines.push(plain("done."));
        for width in [20u16, 41, 82, 120] {
            assert_eq!(
                wrapped_row_count(&lines, width),
                ratatui_rows(&lines, width),
                "transcript mismatch @ {}",
                width
            );
        }
    }

    #[test]
    fn zero_width_has_no_rows() {
        assert_eq!(wrapped_row_count(&[plain("anything")], 0), 0);
    }

    /// Deterministic fuzz: random text built from the pieces an LLM transcript
    /// actually contains. Any disagreement here is a scroll-position bug.
    #[test]
    fn oracle_matches_ratatui_on_fuzzed_text() {
        const PIECES: [&str; 14] = [
            "a",
            "word",
            "lorem",
            "ipsum",
            "dolor",
            "  ",
            " ",
            "\t",
            "\u{200b}",
            "\u{00a0}",
            "-",
            "\u{65e5}\u{672c}",
            "\u{1f389}",
            "\u{e9}",
        ];
        let mut seed = 0x2545_F491_4F6C_DD1Du64;
        let mut next = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        for case in 0..400 {
            let mut text = String::new();
            let len = (next() % 40) as usize;
            for _ in 0..len {
                text.push_str(PIECES[(next() % PIECES.len() as u64) as usize]);
            }
            let width = 1 + (next() % 60) as u16;
            let lines = vec![plain(&text)];
            assert_eq!(
                wrapped_row_count(&lines, width),
                ratatui_rows(&lines, width),
                "case {} mismatch for {:?} @ {}",
                case,
                text,
                width
            );
        }
    }

    #[test]
    fn sum_is_additive_over_lines() {
        let lines = vec![plain("aaa bbb ccc"), plain(""), plain("ddd")];
        assert_eq!(
            wrapped_row_count(&lines, 40),
            ratatui_rows(&lines, 40),
            "blank lines still occupy a row each"
        );
        assert_eq!(wrapped_row_count(&lines, 40), 3);
    }

    #[test]
    fn plain_wrap_breaks_on_words() {
        assert_eq!(wrap_plain("one two three", 20), vec!["one two three"]);
        assert_eq!(
            wrap_plain("one two three", 8),
            vec!["one two", "three"],
            "rows must fit the width"
        );
        assert_eq!(wrap_plain("", 10), vec![""]);
        assert_eq!(
            wrap_plain("a\n\nb", 10),
            vec!["a", "", "b"],
            "blank lines kept"
        );
    }

    #[test]
    fn plain_wrap_keeps_indent() {
        assert_eq!(
            wrap_plain("  alpha beta gamma", 12),
            vec!["  alpha beta", "  gamma"],
            "continuation rows keep the indent"
        );
    }

    #[test]
    fn plain_wrap_hard_breaks_long_words() {
        assert_eq!(wrap_plain("aaaaaaaaaaaa", 5), vec!["aaaaa", "aaaaa", "aa"]);
    }

    #[test]
    fn plain_wrap_counts_cells_not_chars() {
        // Wide chars are two cells each: 日本語の is 8 cells, not 4.
        assert_eq!(
            wrap_plain("日本語のテキスト", 8),
            vec!["日本語の", "テキスト"]
        );
        for row in wrap_plain("αααααααααα", 6) {
            assert!(UnicodeWidthStr::width(row.as_str()) <= 6, "{:?}", row);
        }
    }

    #[test]
    fn plain_wrap_never_returns_an_empty_first_row() {
        for width in 1..12u16 {
            let rows = wrap_plain("   leading spaces then text", width);
            assert!(!rows.is_empty());
            assert!(
                rows.iter().all(
                    |r| UnicodeWidthStr::width(r.as_str()) <= width.max(1) as usize
                        || r.trim().is_empty()
                ),
                "width {} produced {:?}",
                width,
                rows
            );
        }
    }
}
