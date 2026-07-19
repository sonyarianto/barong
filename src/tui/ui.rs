use crate::app::App;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use ratatui::Frame;

pub fn render(frame: &mut Frame, app: &mut App) {
    let area = frame.area();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(1),
            Constraint::Length(3),
            Constraint::Length(3),
        ])
        .split(area);

    render_chat(frame, chunks[0], app);
    render_input(frame, chunks[1], app);
    render_status(frame, chunks[2], app);
}

fn render_chat(frame: &mut Frame, area: Rect, app: &mut App) {
    if app.is_home {
        let block_style = Style::default()
            .fg(Color::Rgb(57, 181, 74))
            .add_modifier(Modifier::BOLD);
        let mut splash_lines: Vec<Line> = crate::app::SPLASH
            .lines()
            .filter(|l| !l.is_empty())
            .map(|l| Line::from(Span::styled(l, block_style)))
            .collect();
        splash_lines.push(Line::from(Span::raw("")));
        splash_lines.push(Line::from(Span::styled(
            "  Type a message below to start a new session.",
            Style::default().fg(Color::DarkGray),
        )));

        let paragraph = Paragraph::new(splash_lines)
            .alignment(Alignment::Center)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" KaliCode ")
                    .border_style(Style::default().fg(Color::Rgb(57, 181, 74))),
            );
        frame.render_widget(paragraph, area);
        return;
    }

    let md = crate::tui::markdown::MarkdownRenderer::new();
    let mut all_lines: Vec<Line> = Vec::new();

    for msg in &app.conversation.messages {
        let role_color = match msg.role.as_str() {
            "user" => Color::Blue,
            "tool" => Color::Yellow,
            _ => Color::Rgb(57, 181, 74),
        };
        let role_lines = md.render(msg.content.as_deref().unwrap_or(""), &msg.role);
        for mut line in role_lines {
            line.spans.insert(0, Span::styled("│ ", Style::default().fg(role_color)));
            all_lines.push(line);
        }
        all_lines.push(Line::from(Span::raw("")));
    }

    if !app.streaming_text.is_empty() {
        let stream_lines = md.render(&app.streaming_text, "assistant");
        for mut line in stream_lines {
            line.spans.insert(0, Span::styled("│ ", Style::default().fg(Color::Rgb(57, 181, 74))));
            all_lines.push(line);
        }
        all_lines.push(Line::from(Span::styled(
            "│ █",
            Style::default().fg(Color::Green).add_modifier(Modifier::SLOW_BLINK),
        )));
    }

    let total_lines = all_lines.len();
    let inner = area.inner(ratatui::layout::Margin {
        vertical: 1,
        horizontal: 1,
    });
    let visible_rows = inner.height.max(1);

    // Estimate how many logical lines fit in visible_rows after wrapping
    let fit_count = all_lines
        .iter()
        .scan(0u16, |acc, line| {
            let line_str: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
            let content_width = inner.width.saturating_sub(1).max(1);
            let wrapped = ((line_str.len() as u16) + content_width - 1) / content_width;
            *acc += wrapped.max(1);
            if *acc > visible_rows { None } else { Some(()) }
        })
        .count();

    if app.should_auto_scroll {
        app.chat_scroll = total_lines.saturating_sub(fit_count);
    }

    let scroll = app.chat_scroll.min(total_lines.saturating_sub(1)).max(0);

    // Take enough lines to cover visible_rows after wrapping
    let mut take_count = 0usize;
    let mut row_accum = 0u16;
    for line in all_lines.iter().skip(scroll) {
        if row_accum >= visible_rows { break; }
        let line_str: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        let content_width = inner.width.saturating_sub(1).max(1);
        let wrapped = ((line_str.len() as u16) + content_width - 1) / content_width;
        row_accum += wrapped.max(1);
        take_count += 1;
    }

    let visible_lines: Vec<Line> = all_lines
        .iter()
        .skip(scroll)
        .take(take_count + 1)
        .cloned()
        .collect();

    let title_spans = vec![
        Span::styled(" Chat ", Style::default().fg(Color::White)),
        Span::styled(
            format!(" [{}/{}]", scroll, total_lines),
            Style::default().fg(Color::DarkGray),
        ),
    ];

    let title_block = Block::default()
        .borders(Borders::ALL)
        .title(Line::from(title_spans))
        .border_style(Style::default().fg(Color::White));
    let paragraph = Paragraph::new(visible_lines)
        .block(title_block)
        .wrap(Wrap { trim: false });
    frame.render_widget(paragraph, area);
}

fn render_input(frame: &mut Frame, area: Rect, app: &App) {
    let focused = app.event_rx.is_none();
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Input ")
        .border_style(if focused {
            Style::default().fg(Color::Cyan)
        } else {
            Style::default().fg(Color::DarkGray)
        });

    if focused {
        let mut display = app.input.buffer.clone();
        if app.input.cursor_pos <= display.len() {
            display.insert(app.input.cursor_pos, '█');
        }
        let paragraph = Paragraph::new(display.as_str())
            .block(block)
            .style(Style::default().fg(Color::White));
        frame.render_widget(paragraph, area);
    } else {
        let paragraph = Paragraph::new(" (waiting for response...)")
            .block(block)
            .style(Style::default().fg(Color::DarkGray));
        frame.render_widget(paragraph, area);
    }
}

fn render_status(frame: &mut Frame, area: Rect, app: &App) {
    let parts = vec![
        Span::styled(
            format!(" {} ", app.status.llm_provider),
            Style::default().fg(Color::Cyan),
        ),
        Span::raw("│"),
        Span::styled(
            format!(" {} ", app.status.model),
            Style::default().fg(Color::Magenta),
        ),
        Span::raw("│"),
        Span::styled(
            format!(" {} ", app.status.tool_status),
            Style::default().fg(match app.status.tool_status.as_str() {
                "idle" => Color::Green,
                _ => Color::Yellow,
            }),
        ),
        Span::raw("│"),
        Span::styled(
            format!(" msgs:{} ", app.conversation.messages.len()),
            Style::default().fg(Color::DarkGray),
        ),
        Span::raw("│"),
        Span::styled(
            format!(" {} ", app.status.token_count),
            Style::default().fg(Color::DarkGray),
        ),
    ];

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray));
    let paragraph = Paragraph::new(Line::from(parts)).block(block);
    frame.render_widget(paragraph, area);
}
