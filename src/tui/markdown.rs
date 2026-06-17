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
            "user" => Span::styled("You  ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
            "assistant" => Span::styled("Kali ", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
            "system" => Span::styled("Sys  ", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
            "tool" => Span::raw("     "),
            _ => Span::raw("     "),
        };

        let mut in_code_block = false;
        let mut code_block_lang = String::new();
        let mut code_block_lines: Vec<String> = Vec::new();

        for raw_line in text.lines() {
            if raw_line.trim_start().starts_with("```") {
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

            let styled_spans = self.parse_inline(raw_line);
            let mut spans = vec![prefix.clone()];
            spans.extend(styled_spans);
            lines.push(Line::from(spans));
        }

        if in_code_block && !code_block_lines.is_empty() {
            self.emit_code_block(&mut lines, &prefix, &code_block_lang, &code_block_lines);
        }

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
