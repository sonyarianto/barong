use crate::app::App;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Style};
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

fn render_chat(frame: &mut Frame, area: Rect, app: &App) {
    let messages: Vec<Line> = app
        .conversation
        .messages
        .iter()
        .map(|msg| {
            let prefix = match msg.role.as_str() {
                "user" => "You: ",
                "assistant" => "Kali: ",
                "system" => "System: ",
                "tool" => "  -> ",
                _ => "",
            };
            let style = match msg.role.as_str() {
                "user" => Style::default().fg(Color::Cyan),
                "assistant" => Style::default().fg(Color::Green),
                "system" => Style::default().fg(Color::Yellow),
                "tool" => Style::default().fg(Color::DarkGray),
                _ => Style::default(),
            };
            Line::from(Span::styled(format!("{}{}", prefix, msg.content), style))
        })
        .collect();

    let block = Block::default().borders(Borders::ALL).title(" Chat ");
    let paragraph = Paragraph::new(messages)
        .block(block)
        .wrap(Wrap { trim: false });
    frame.render_widget(paragraph, area);
}

fn render_input(frame: &mut Frame, area: Rect, app: &App) {
    let block = Block::default().borders(Borders::ALL).title(" Input ");
    let paragraph = Paragraph::new(app.input.buffer.as_str())
        .block(block)
        .style(Style::default().fg(Color::White));
    frame.render_widget(paragraph, area);

    if app.input.focused {
        frame.set_cursor_position((
            area.x + 1 + app.input.cursor_pos as u16,
            area.y + 1,
        ));
    }
}

fn render_status(frame: &mut Frame, area: Rect, app: &App) {
    let status_text = format!(
        " {} | {} | {}",
        app.status.llm_provider, app.status.tool_status, app.status.token_count,
    );
    let block = Block::default().borders(Borders::ALL).title(" Status ");
    let paragraph = Paragraph::new(Span::raw(status_text)).block(block);
    frame.render_widget(paragraph, area);
}
