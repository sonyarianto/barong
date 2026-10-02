use crate::tui::syntax;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

pub struct MarkdownRenderer {
    code_theme: &'static str,
    accent: Color,
    muted: Color,
    warning: Color,
    success: Color,
    panel: Color,
    /// Visible terminal width for full-width code rules (0 = compact fallback).
    width: u16,
}

impl MarkdownRenderer {
    pub fn new() -> Self {
        Self {
            code_theme: "base16-ocean.dark",
            accent: Color::Cyan,
            muted: Color::DarkGray,
            warning: Color::Yellow,
            success: Color::LightGreen,
            panel: Color::Rgb(24, 25, 30),
            width: 0,
        }
    }

    pub fn with_code_theme(code_theme: &'static str) -> Self {
        Self { code_theme, ..Self::new() }
    }

    pub fn with_theme(theme: &crate::tui::theme::Theme) -> Self {
        Self {
            code_theme: theme.code_theme,
            accent: theme.accent,
            muted: theme.muted,
            warning: theme.warning,
            success: theme.success,
            panel: theme.panel,
            width: 0,
        }
    }

    pub fn with_width(mut self, width: u16) -> Self {
        self.width = width;
        self
    }

    pub fn render<'a>(&self, text: &'a str, role: &str) -> Vec<Line<'a>> {
        let mut lines = Vec::new();

        let prefix = match role {
            "user" => Span::styled("› ", Style::default().fg(self.accent).add_modifier(Modifier::BOLD)),
            "assistant" => Span::raw(""),
            "system" => Span::styled("! ", Style::default().fg(self.warning)),
            "tool" => Span::raw(""),
            _ => Span::raw(""),
        };

        let mut in_code_block = false;
        let mut code_block_lang = String::new();
        let mut code_block_lines: Vec<String> = Vec::new();
        let mut in_table = false;
        let mut table_lines: Vec<String> = Vec::new();

        for raw_line in text.lines() {
            if raw_line.trim_start().starts_with("```") {
                self.flush_table(&mut lines, &prefix, &mut in_table, &mut table_lines);
                if in_code_block {
                    in_code_block = false;
                    self.emit_code_block(&mut lines, &prefix, &code_block_lang, &code_block_lines);
                    code_block_lines.clear();
                    code_block_lang.clear();
                    // No footer rule: the container simply ends (OpenCode-style
                    // calm — one accent, no four-border cage).
                } else {
                    in_code_block = true;
                    code_block_lang = raw_line.trim_start().trim_start_matches("```").trim().to_string();
                    let lang = if code_block_lang.is_empty() { "code".to_string() } else { code_block_lang.clone() };
                    lines.push(self.code_label(&prefix, &lang));
                    if let Some(gap) = self.code_gap(&prefix) {
                        lines.push(gap);
                    }
                }
                continue;
            }

            if in_code_block {
                code_block_lines.push(raw_line.to_string());
                continue;
            }

            let trimmed = raw_line.trim();
            if trimmed.starts_with('|') && trimmed.ends_with('|') {
                in_table = true;
                table_lines.push(raw_line.to_string());
                continue;
            }

            if in_table {
                self.flush_table(&mut lines, &prefix, &mut in_table, &mut table_lines);
            }

            // Horizontal rule: `---`, `***`, `___` (3+ of the same).
            if is_hr(trimmed) {
                lines.push(Line::from(vec![
                    prefix.clone(),
                    Span::styled("─".repeat(32), Style::default().fg(self.muted)),
                ]));
                continue;
            }

            // Blockquote: `> ...` (nesting flattened).
            if trimmed.starts_with('>') {
                let inner = trimmed.trim_start_matches(['>', ' ']);
                let mut spans = vec![
                    prefix.clone(),
                    Span::styled("▍ ", Style::default().fg(self.accent)),
                ];
                spans.extend(self.parse_inline(inner));
                lines.push(Line::from(spans));
                continue;
            }

            // List item: `-`, `*`, `+`, `1.` … — recolor the marker.
            // Task list (`- [ ]` / `- [x]`) swaps the bullet for a box.
            if let Some((indent, marker, rest)) = split_list_marker(raw_line) {
                let mut spans = vec![prefix.clone()];
                if !indent.is_empty() {
                    spans.push(Span::raw(indent));
                }
                if let Some(after) = rest.strip_prefix("[ ] ") {
                    spans.push(Span::styled("☐ ", Style::default().fg(self.muted)));
                    spans.extend(self.parse_inline(after));
                } else if rest.strip_prefix("[x] ").is_some() || rest.strip_prefix("[X] ").is_some() {
                    spans.push(Span::styled("☑ ", Style::default().fg(self.success)));
                    spans.extend(self.parse_inline(&rest[4..]));
                } else {
                    spans.push(Span::styled(marker, Style::default().fg(self.accent).add_modifier(Modifier::BOLD)));
                    spans.push(Span::raw(" "));
                    spans.extend(self.parse_inline(rest));
                }
                lines.push(Line::from(spans));
                continue;
            }

            let styled_spans = self.parse_inline(raw_line);
            let mut spans = vec![prefix.clone()];
            spans.extend(styled_spans);
            lines.push(Line::from(spans));
        }

        if in_code_block && !code_block_lines.is_empty() {
            self.emit_code_block(&mut lines, &prefix, &code_block_lang, &code_block_lines);
        }
        self.flush_table(&mut lines, &prefix, &mut in_table, &mut table_lines);

        lines
    }

    /// Container label row: dim language tag on panel background.
    /// Falls back to the compact header when the width is unknown (< 20).
    /// The bar is `▎` (left-quarter block): unlike `┃` it hugs the left edge
    /// of its cell in most fonts instead of floating right.
    fn code_label<'a>(&self, prefix: &Span<'a>, lang: &str) -> Line<'a> {
        let inner = self.width.saturating_sub(prefix.width() as u16) as usize;
        if inner < 20 {
            return Line::from(vec![
                prefix.clone(),
                Span::styled("── ", Style::default().fg(self.muted)),
                Span::styled(lang.to_string(), Style::default().fg(self.accent).add_modifier(Modifier::BOLD)),
                Span::styled(" ──", Style::default().fg(self.muted)),
            ]);
        }
        let pad = inner.saturating_sub(2 + lang.chars().count());
        Line::from(vec![
            prefix.clone(),
            Span::styled("▎ ", Style::default().fg(self.muted).bg(self.panel)),
            Span::styled(lang.to_string(), Style::default().fg(self.muted).bg(self.panel)),
            Span::styled(" ".repeat(pad), Style::default().bg(self.panel)),
        ])
    }

    /// Blank breathing row between label and body (panel bg, bar continues).
    /// Empty vec when narrow (compact path has no container to breathe in).
    fn code_gap<'a>(&self, prefix: &Span<'a>) -> Option<Line<'a>> {
        let inner = self.width.saturating_sub(prefix.width() as u16) as usize;
        if inner < 20 {
            return None;
        }
        Some(Line::from(vec![
            prefix.clone(),
            Span::styled("▎ ", Style::default().fg(self.muted).bg(self.panel)),
            Span::styled(" ".repeat(inner.saturating_sub(2)), Style::default().bg(self.panel)),
        ]))
    }

    fn emit_code_block<'a>(&self, lines: &mut Vec<Line<'a>>, prefix: &Span<'a>, lang: &str, code_lines: &[String]) {
        // Tabs break width math — expand first (highlighting is unaffected).
        let expanded: Vec<String> = code_lines.iter().map(|l| l.replace('\t', "    ")).collect();
        let lang_opt = if lang.is_empty() { None } else { Some(lang) };
        let highlighted = syntax::highlight_code_block_with_theme(
            expanded.iter().map(|s| s.as_str()).collect(),
            lang_opt,
            self.code_theme,
        );

        let gutter_w = code_lines.len().to_string().len().max(1);
        let inner = self.width.saturating_sub(prefix.width() as u16) as usize;
        // content = inner − "┃ "(2) − gutter − no right border (open container)
        let content_w = inner.saturating_sub(3 + gutter_w);
        if inner < 20 || content_w < 10 {
            // Compact fallback: no wrapping, left border only.
            for (i, (spans, _is_bold)) in highlighted.into_iter().enumerate() {
                let mut line_spans = vec![
                    prefix.clone(),
                    Span::styled("│ ", Style::default().fg(self.muted)),
                    Span::styled(
                        format!("{:>width$} ", i + 1, width = gutter_w),
                        Style::default().fg(self.muted),
                    ),
                ];
                for (style, text) in spans {
                    line_spans.push(Span::styled(text, style));
                }
                lines.push(Line::from(line_spans));
            }
            return;
        }
        for (i, (spans, _is_bold)) in highlighted.into_iter().enumerate() {
            for (vi, seg) in wrap_spans(spans, content_w).into_iter().enumerate() {
                let gutter = if vi == 0 {
                    format!("{:>width$} ", i + 1, width = gutter_w)
                } else {
                    " ".repeat(gutter_w + 1)
                };
                let w: usize = seg.iter().map(|(_, t)| display_width(t)).sum();
                let mut line_spans = vec![
                    prefix.clone(),
                    Span::styled("▎ ", Style::default().fg(self.muted).bg(self.panel)),
                    Span::styled(gutter, Style::default().fg(self.muted).bg(self.panel)),
                ];
                for (style, text) in seg {
                    line_spans.push(Span::styled(text, style.bg(self.panel)));
                }
                line_spans.push(Span::styled(" ".repeat(content_w.saturating_sub(w)), Style::default().bg(self.panel)));
                lines.push(Line::from(line_spans));
            }
        }
        // Bottom breathing room mirrors the top gap.
        if let Some(gap) = self.code_gap(prefix) {
            lines.push(gap);
        }
    }

    fn flush_table<'a>(&self, lines: &mut Vec<Line<'a>>, prefix: &Span<'a>, in_table: &mut bool, table_lines: &mut Vec<String>) {
        if !*in_table {
            return;
        }
        *in_table = false;
        let rows = std::mem::take(table_lines);

        // Determine column widths
        let mut col_widths: Vec<usize> = Vec::new();
        for row in &rows {
            let cells: Vec<&str> = row
                .split('|')
                .filter(|c| !c.is_empty())
                .collect();
            for (i, cell) in cells.iter().enumerate() {
                let clean = cell.trim();
                if i >= col_widths.len() {
                    col_widths.push(clean.len());
                } else {
                    col_widths[i] = col_widths[i].max(clean.len());
                }
            }
        }

        let table_style = Style::default().fg(self.accent);
        let sep_style = Style::default().fg(self.muted);
        let header_style = Style::default()
            .fg(self.accent)
            .add_modifier(Modifier::BOLD);

        for (row_idx, row) in rows.iter().enumerate() {
            let cells: Vec<&str> = row
                .split('|')
                .filter(|c| !c.is_empty())
                .collect();

            let mut spans = vec![prefix.clone()];

            // Detect separator row (|---|)
            if row.trim().chars().all(|c| c == '|' || c == '-' || c == ':') {
                spans.push(Span::styled("├─", sep_style));
                for (i, w) in col_widths.iter().enumerate() {
                    let sep = "─".repeat(*w);
                    spans.push(Span::styled(sep, sep_style));
                    if i < col_widths.len() - 1 {
                        spans.push(Span::styled("─┼─", sep_style));
                    }
                }
                spans.push(Span::styled("─┤", sep_style));
                lines.push(Line::from(spans));
                continue;
            }

            let style = if row_idx == 0 { header_style } else { table_style };
            spans.push(Span::styled("│ ", style));
            for (i, cell) in cells.iter().enumerate() {
                let clean = cell.trim();
                let w = col_widths.get(i).copied().unwrap_or(clean.len());
                let padded = format!("{:<width$}", clean, width = w);
                spans.push(Span::styled(padded, style));
                if i < col_widths.len() - 1 {
                    spans.push(Span::styled(" │ ", style));
                }
            }
            spans.push(Span::styled(" │", style));
            lines.push(Line::from(spans));
        }
        lines.push(Line::from(vec![
            prefix.clone(),
            Span::raw(""),
        ]));
    }

    fn parse_inline<'a>(&self, text: &'a str) -> Vec<Span<'a>> {
        let mut spans = Vec::new();
        let mut chars = text.char_indices().peekable();

        while let Some(&(i, c)) = chars.peek() {
            if c == '`' {
                let mut end = i + 1;
                let mut next = chars.clone();
                next.next();
                while let Some((j, ch)) = next.next() {
                    if ch == '`' {
                        end = j;
                        break;
                    }
                }
                if end > i + 1 {
                    let code = &text[i + 1..end];
                    spans.push(Span::styled(
                        format!("`{}`", code),
                        Style::default()
                            .fg(self.accent)
                            .bg(self.muted),
                    ));
                    for _ in 0..=(end - i) {
                        chars.next();
                    }
                } else {
                    spans.push(Span::raw("`"));
                    chars.next();
                }
            } else if c == '*' && !text[i..].starts_with("**") {
                // Single `*italic*` (double-star bold is handled above).
                // Guards mirror CommonMark: opener not followed by space,
                // closer not preceded by space, so `2 * 3` stays literal.
                let rest = &text[i + 1..];
                let ok_open = !(rest.starts_with(' ') || rest.starts_with('\t'));
                match find_closing_single_star(rest).filter(|&k| {
                    k > 0 && ok_open && !(rest[..k].ends_with(' ') || rest[..k].ends_with('\t'))
                }) {
                    Some(k) => {
                        let content = &rest[..k];
                        spans.push(Span::styled(
                            content,
                            Style::default().add_modifier(Modifier::ITALIC),
                        ));
                        for _ in 0..1 + content.chars().count() + 1 {
                            chars.next();
                        }
                    }
                    None => {
                        spans.push(Span::raw("*"));
                        chars.next();
                    }
                }
            } else if text[i..].starts_with("**") {                let rest = &text[i + 2..];
                if let Some(end) = rest.find("**") {
                    let bold_text = &rest[..end];
                    let style = if bold_text.starts_with("Tool:") {
                        Style::default().fg(self.warning).add_modifier(Modifier::BOLD)
                    } else if bold_text.ends_with("result:") || bold_text.ends_with("result") {
                        Style::default().fg(self.success).add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().add_modifier(Modifier::BOLD)
                    };
                    spans.push(Span::styled(bold_text, style));
                    for _ in 0..=(end + 3) {
                        chars.next();
                    }
                } else {
                    spans.push(Span::raw("**"));
                    chars.next();
                    chars.next();
                }
            } else if text[i..].starts_with("# ") {
                spans.push(Span::styled(
                    &text[i + 2..],
                    Style::default()
                        .fg(Color::LightMagenta)
                        .add_modifier(Modifier::BOLD),
                ));
                break;
            } else if text[i..].starts_with("## ") {
                spans.push(Span::styled(
                    &text[i + 3..],
                    Style::default()
                        .fg(Color::Magenta)
                        .add_modifier(Modifier::BOLD),
                ));
                break;
            } else if text[i..].starts_with("### ") {
                spans.push(Span::styled(
                    &text[i + 4..],
                    Style::default()
                        .fg(Color::LightBlue)
                        .add_modifier(Modifier::BOLD),
                ));
                break;
            } else {
                spans.push(Span::raw(&text[i..i + c.len_utf8()]));
                chars.next();
            }
        }

        spans
    }
}

/// Display width in terminal cells (CJK-safe).
fn display_width(s: &str) -> usize {
    use unicode_width::UnicodeWidthStr;
    s.width()
}

/// Wrap owned highlight spans to `max_width` cells, splitting styles across
/// wrap points. Greedy with space backtrack; overlong words break mid-word.
fn wrap_spans(spans: Vec<(Style, String)>, max_width: usize) -> Vec<Vec<(Style, String)>> {
    use unicode_width::UnicodeWidthChar;
    let mut cells: Vec<(Style, char)> = Vec::new();
    for (st, text) in &spans {
        for c in text.chars() {
            if c == '\n' || c == '\r' {
                continue;
            }
            cells.push((*st, c));
        }
    }
    let row_width = |row: &[(Style, char)]| -> usize {
        row.iter().map(|(_, c)| UnicodeWidthChar::width(*c).unwrap_or(0)).sum()
    };
    let mut rows: Vec<Vec<(Style, char)>> = vec![Vec::new()];
    let mut w = 0usize;
    let mut last_space: Option<usize> = None;
    for (st, c) in cells {
        let cw = UnicodeWidthChar::width(c).unwrap_or(0);
        let cur_empty = rows.last().map(|r| r.is_empty()).unwrap_or(true);
        if w + cw > max_width && !cur_empty {
            if let Some(si) = last_space {
                let cur = rows.last_mut().unwrap();
                let tail: Vec<(Style, char)> = cur.split_off(si + 1);
                cur.pop(); // drop the space we broke at
                rows.push(tail);
                w = row_width(rows.last().unwrap());
                last_space = rows.last().unwrap().iter().rposition(|(_, ch)| *ch == ' ');
            } else {
                rows.push(Vec::new());
                w = 0;
                last_space = None;
            }
        }
        rows.last_mut().unwrap().push((st, c));
        w += cw;
        if c == ' ' {
            last_space = Some(rows.last().unwrap().len() - 1);
        }
    }
    rows
        .into_iter()
        .map(|row| {
            let mut out: Vec<(Style, String)> = Vec::new();
            for (st, c) in row {
                match out.last_mut() {
                    Some(last) if last.0 == st => last.1.push(c),
                    _ => out.push((st, c.to_string())),
                }
            }
            out
        })
        .collect()
}

/// Byte index of a lone closing `*` in `rest` (text after the opener).
/// `**` runs are skipped whole so bold markers never match as italic.
fn find_closing_single_star(rest: &str) -> Option<usize> {
    let b = rest.as_bytes();
    let mut k = 0;
    while k < b.len() {
        if b[k] == b'*' {
            if k + 1 < b.len() && b[k + 1] == b'*' {
                while k < b.len() && b[k] == b'*' {
                    k += 1;
                }
                continue;
            }
            return Some(k);
        }
        k += 1;
    }
    None
}

fn is_hr(trimmed: &str) -> bool {    trimmed.len() >= 3
        && (trimmed.chars().all(|c| c == '-')
            || trimmed.chars().all(|c| c == '*')
            || trimmed.chars().all(|c| c == '_'))
}

/// Split `  - item` / `  1. item` into (indent, marker, rest).
/// Returns None for plain text (`-item` without space is not a list).
fn split_list_marker(line: &str) -> Option<(&str, &str, &str)> {    let indent_len = line.len() - line.trim_start().len();
    let (indent, rest) = line.split_at(indent_len);
    for m in ['-', '*', '+'] {
        if let Some(after) = rest.strip_prefix(m) {
            if after.starts_with(' ') || after.starts_with('\t') {
                return Some((indent, &rest[..m.len_utf8()], after.trim_start()));
            }
        }
    }
    let mut num_end = 0;
    for (i, c) in rest.char_indices() {
        if c.is_ascii_digit() {
            num_end = i + 1;
        } else {
            break;
        }
    }
    if num_end > 0 {
        let after = &rest[num_end..];
        if let Some(stripped) = after.strip_prefix('.').or_else(|| after.strip_prefix(')')) {
            if stripped.is_empty() || stripped.starts_with(' ') || stripped.starts_with('\t') {
                return Some((indent, &rest[..num_end + 1], stripped.trim_start()));
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat(lines: &[Line]) -> Vec<String> {
        lines
            .iter()
            .map(|l| l.spans.iter().map(|s| s.content.to_string()).collect::<String>())
            .collect()
    }

    #[test]
    fn go_block_gets_header_gutter_no_raw_fences() {
        let md = MarkdownRenderer::new();
        let text = "Here:\n```go\npackage main\n\nfunc main() {}\n```\nDone.";
        let out = flat(&md.render(text, "assistant"));
        let joined = out.join("\n");
        assert!(!joined.contains("```"), "raw fences must go, got:\n{}", joined);
        assert!(joined.contains("go"), "lang label missing:\n{}", joined);
        // gutter numbers for 3 code lines
        assert!(out.iter().any(|l| l.contains('1') && l.contains("package main")), "got:\n{}", joined);
        assert!(out.iter().any(|l| l.contains('3') && l.contains("func main")), "got:\n{}", joined);
    }

    #[test]
    fn unclosed_fence_still_renders() {
        let md = MarkdownRenderer::new();
        let out = flat(&md.render("```python\nprint(1)", "assistant"));
        assert!(out.iter().any(|l| l.contains("print(1)")), "got:\n{}", out.join("\n"));
    }

    #[test]
    fn long_lines_wrap_inside_borders() {
        use unicode_width::UnicodeWidthStr;
        let md = MarkdownRenderer::new().with_width(40);
        let long = "x".repeat(100);
        let text = format!("```go\n{}\n```", long);
        let out = md.render(&text, "assistant");
        // label + gap + wrapped body; every row exactly 40 cells.
        assert!(out.len() > 3, "must wrap, got {} rows", out.len());
        for (i, l) in out.iter().enumerate() {
            let s: String = l.spans.iter().map(|s| s.content.to_string()).collect();
            assert_eq!(s.width(), 40, "row {} width, got: {}", i, s);
        }
        assert!(out[0].spans.iter().any(|s| s.content.contains("go")));
        // First body row keeps the gutter number, continuations don't.
        // (index 0 = label, 1 = gap.)
        assert!(out[2].spans.iter().any(|s| s.content.contains('1')));
        assert!(out[3].spans.iter().all(|s| !s.content.contains('2') || s.content.contains('x')));
    }

    #[test]
    fn wrap_prefers_spaces_and_keeps_styles() {
        let md = MarkdownRenderer::new().with_width(30);
        let out = md.render("```\nfoo bar baz qux quux corge grault\n```", "assistant");
        // No row may split inside a word when a space break exists.
        for l in out.iter().skip(1).take(out.len().saturating_sub(2)) {
            let s: String = l.spans.iter().map(|s| s.content.to_string()).collect();
            assert!(!s.contains("fo o") && !s.contains("ba r"), "split mid-word: {}", s);
        }
    }

    #[test]
    fn quote_list_hr_task() {
        let md = MarkdownRenderer::new();
        let out = flat(&md.render("> be quoted\n- item\n1. first\n- [ ] todo\n- [x] done\n---", "assistant"));
        let joined = out.join("\n");
        assert!(joined.contains("▍") && joined.contains("be quoted"), "quote, got:\n{}", joined);
        assert!(joined.contains('-') && joined.contains("item"), "list, got:\n{}", joined);
        assert!(joined.contains("1.") && joined.contains("first"), "ordered, got:\n{}", joined);
        assert!(joined.contains('☐') && joined.contains("todo"), "task open, got:\n{}", joined);
        assert!(joined.contains('☑') && !joined.contains("- ☑"), "task done replaces bullet, got:\n{}", joined);
        assert!(joined.contains('─'), "hr, got:\n{}", joined);
    }

    #[test]
    fn berlin_italic_renders_without_stars() {
        use ratatui::style::Modifier;
        let md = MarkdownRenderer::new();
        let out = md.render("Ibukota *Germany* adalah Berlin.", "assistant");
        assert_eq!(out.len(), 1);
        let text: String = out[0].spans.iter().map(|s| s.content.to_string()).collect();
        assert_eq!(text, "Ibukota Germany adalah Berlin.", "stars must go, got: {}", text);
        let germany = out[0].spans.iter().find(|s| s.content == "Germany").expect("Germany span");
        assert!(germany.style.add_modifier.contains(Modifier::ITALIC), "must be italic");
    }

    #[test]
    fn italic_edge_cases_stay_literal() {
        let md = MarkdownRenderer::new();
        let flat = |t: &str| {
            md.render(t, "assistant")
                .iter()
                .map(|l| l.spans.iter().map(|s| s.content.to_string()).collect::<String>())
                .collect::<Vec<_>>()
                .join("\n")
        };
        // Unclosed, spaced, and bold all keep prior behavior.
        assert!(flat("*Germany adalah Berlin.").contains('*'));
        assert!(flat("2 * 3 * 4").contains('*'));
        assert!(flat("**Germany**").contains("Germany"));
        assert!(!flat("**Germany**").contains('*'));
    }

    #[test]
    fn hyphen_text_is_not_a_list() {
        let md = MarkdownRenderer::new();
        let out = flat(&md.render("well-known fact", "assistant"));
        assert_eq!(out, vec!["well-known fact".to_string()]);
    }
    #[test]
    fn wide_container_rows_span_full_width() {
        let md = MarkdownRenderer::new().with_width(60);
        let out = flat(&md.render("```go\nx\n```", "assistant"));
        // label + top gap + 1 code + bottom gap, no rules anywhere.
        assert_eq!(out.len(), 4, "label + gap + 1 code + gap, got: {:?}", out);
        assert!(out[0].contains("go"), "label, got: {}", out[0]);
        assert!(!out[0].contains('╭') && !out[0].contains('─'), "no rules, got: {}", out[0]);
        for (i, l) in out.iter().enumerate() {
            use unicode_width::UnicodeWidthStr;
            assert_eq!(l.width(), 60, "row {} full width, got: {}", i, l);
            assert!(l.starts_with("▎ "), "left-hugging bar, got: {}", l);
        }
        // Gap row carries no content (top and bottom).
        assert!(out[1].trim_start_matches(['▎', ' ']).is_empty(), "gap, got: {}", out[1]);
        assert!(out[3].trim_start_matches(['▎', ' ']).is_empty(), "gap, got: {}", out[3]);
        assert!(out[2].contains('1'), "gutter, got: {}", out[2]);
    }
}
