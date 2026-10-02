use crate::tui::syntax;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

pub struct MarkdownRenderer;

impl MarkdownRenderer {
    pub fn new() -> Self {
        Self
    }

    pub fn render<'a>(&self, text: &'a str, role: &str) -> Vec<Line<'a>> {
        let mut lines = Vec::new();

        let prefix = match role {
            "user" => Span::styled("› ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
            "assistant" => Span::raw(""),
            "system" => Span::styled("! ", Style::default().fg(Color::Yellow)),
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
                    let line = Line::from(vec![
                        prefix.clone(),
                        Span::styled("```", Style::default().fg(Color::DarkGray)),
                    ]);
                    lines.push(line);
                } else {
                    in_code_block = true;
                    code_block_lang = raw_line.trim_start().trim_start_matches("```").trim().to_string();
                    let line = Line::from(vec![
                        prefix.clone(),
                        Span::styled(
                            if code_block_lang.is_empty() {
                                "```".to_string()
                            } else {
                                format!("```{}", code_block_lang)
                            },
                            Style::default().fg(Color::DarkGray),
                        ),
                    ]);
                    lines.push(line);
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

    fn emit_code_block<'a>(&self, lines: &mut Vec<Line<'a>>, prefix: &Span<'a>, lang: &str, code_lines: &[String]) {
        let lang_opt = if lang.is_empty() { None } else { Some(lang) };
        let highlighted = syntax::highlight_code_block(
            code_lines.iter().map(|s| s.as_str()).collect(),
            lang_opt,
        );

        for (spans, _is_bold) in highlighted {
            let mut line_spans = vec![prefix.clone()];
            for (style, text) in spans {
                line_spans.push(Span::styled(text, style));
            }
            lines.push(Line::from(line_spans));
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

        let table_style = Style::default().fg(Color::LightCyan);
        let sep_style = Style::default().fg(Color::DarkGray);
        let header_style = Style::default()
            .fg(Color::LightCyan)
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
                            .fg(Color::Cyan)
                            .bg(Color::DarkGray),
                    ));
                    for _ in 0..=(end - i) {
                        chars.next();
                    }
                } else {
                    spans.push(Span::raw("`"));
                    chars.next();
                }
            } else if text[i..].starts_with("**") {
                let rest = &text[i + 2..];
                if let Some(end) = rest.find("**") {
                    let bold_text = &rest[..end];
                    let style = if bold_text.starts_with("Tool:") {
                        Style::default().fg(Color::LightYellow).add_modifier(Modifier::BOLD)
                    } else if bold_text.ends_with("result:") || bold_text.ends_with("result") {
                        Style::default().fg(Color::LightGreen).add_modifier(Modifier::BOLD)
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
