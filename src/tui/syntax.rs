use ratatui::style::{Color, Modifier, Style};
use std::sync::OnceLock;
use syntect::highlighting::{FontStyle, ThemeSet};
use syntect::parsing::SyntaxSet;

struct Highlighter {
    ss: SyntaxSet,
    ts: ThemeSet,
}

static HIGHLIGHTER: OnceLock<Highlighter> = OnceLock::new();

fn init() -> &'static Highlighter {
    HIGHLIGHTER.get_or_init(|| Highlighter {
        ss: SyntaxSet::load_defaults_newlines(),
        ts: ThemeSet::load_defaults(),
    })
}

pub fn highlight_code_block(lines: Vec<&str>, lang: Option<&str>) -> Vec<(Vec<(Style, String)>, bool)> {
    let h = init();
    let syntax = lang
        .and_then(|l| h.ss.find_syntax_by_token(l))
        .unwrap_or_else(|| h.ss.find_syntax_plain_text());

    let theme = &h.ts.themes["base16-ocean.dark"];

    use syntect::easy::HighlightLines;
    let mut highlighter = HighlightLines::new(syntax, theme);

    let mut result = Vec::new();

    for line in lines {
        let ranges = highlighter.highlight_line(line, &h.ss).unwrap_or_default();
        let mut spans = Vec::new();
        for (style, text) in &ranges {
            spans.push((syntect_style_to_ratatui(style), text.to_string()));
        }
        result.push((spans, ranges.iter().any(|(s, _)| s.font_style.contains(FontStyle::BOLD))));
    }

    result
}

fn syntect_style_to_ratatui(style: &syntect::highlighting::Style) -> Style {
    let fg = style.foreground;
    let color = Color::Rgb(fg.r, fg.g, fg.b);

    let mut modifier = Modifier::empty();
    if style.font_style.contains(FontStyle::BOLD) {
        modifier |= Modifier::BOLD;
    }
    if style.font_style.contains(FontStyle::ITALIC) {
        modifier |= Modifier::ITALIC;
    }
    if style.font_style.contains(FontStyle::UNDERLINE) {
        modifier |= Modifier::UNDERLINED;
    }

    Style::default().fg(color).add_modifier(modifier)
}
