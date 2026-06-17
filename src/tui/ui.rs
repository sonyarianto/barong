use crate::app::App;
use ratatui::layout::{Constraint, Direction, Layout, Margin, Rect};
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
            Constraint::Length(1),
        ])
        .split(area);

    render_chat(frame, chunks[0], app);
    render_input(frame, chunks[1], app);
    render_status(frame, chunks[2], app);
}

fn render_chat(frame: &mut Frame, area: Rect, app: &mut App) {
    let md = crate::tui::markdown::MarkdownRenderer::new();
    let mut all_lines: Vec<Line> = Vec::new();

    for msg in &app.conversation.messages {
        let role_lines = md.render(&msg.content, &msg.role);
        all_lines.extend(role_lines);
        all_lines.push(Line::from(Span::raw("")));
    }

    if !app.streaming_text.is_empty() {
        let stream_lines = md.render(&app.streaming_text, "assistant");
        all_lines.extend(stream_lines);
        all_lines.push(Line::from(Span::styled(
            "█",
            Style::default().fg(Color::Green).add_modifier(Modifier::SLOW_BLINK),
        )));
    }

    let total_lines = all_lines.len();
    let inner = area.inner(Margin {
        vertical: 1,
        horizontal: 1,
    });
    let visible_height = inner.height.max(1) as usize;

    if app.should_auto_scroll {
        app.chat_scroll = total_lines.saturating_sub(visible_height);
    }

    let scroll = app.chat_scroll.min(total_lines.saturating_sub(1));
    let scroll = scroll.max(0);

    let visible_lines: Vec<Line> = all_lines
        .iter()
        .skip(scroll)
        .take(visible_height)
        .cloned()
        .collect();

    let scroll_indicator = if total_lines > visible_height {
        format!(
            " [{}/{}]",
            scroll + visible_height.min(total_lines),
            total_lines
        )
    } else {
        String::new()
    };

    let title_spans = vec![
        Span::styled(" Chat ", Style::default().fg(Color::White)),
        Span::styled(scroll_indicator, Style::default().fg(Color::DarkGray)),
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
