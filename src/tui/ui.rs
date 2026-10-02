use crate::app::App;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Wrap};
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
    // palette overlays above input
    if app.input.buffer.starts_with('/') && app.event_rx.is_none() {
        render_palette(frame, chunks[0], chunks[1], app);
    }
    render_status(frame, chunks[2], app);
}

fn render_chat(frame: &mut Frame, area: Rect, app: &mut App) {
    if app.is_home && app.conversation.messages.is_empty() && app.streaming_text.is_empty() {
        let lines = vec![
            Line::from(Span::raw("")),
            Line::from(Span::styled(
                "  barong",
                Style::default().fg(Color::Green).add_modifier(Modifier::BOLD),
            )),
            Line::from(Span::styled(
                "  Terminal coding agent",
                Style::default().fg(Color::DarkGray),
            )),
            Line::from(Span::raw("")),
            Line::from(Span::styled(
                "  Type a message to start  •  / commands  •  @ files  •  Esc cancel",
                Style::default().fg(Color::DarkGray),
            )),
            Line::from(Span::styled(
                format!("  {}  •  {}  •  {} tools", app.cwd_short(), app.current_model, app.tool_registry.tool_names().len()),
                Style::default().fg(Color::DarkGray),
            )),
        ];
        let p = Paragraph::new(lines);
        frame.render_widget(p, area);
        return;
    }

    let md = crate::tui::markdown::MarkdownRenderer::new();
    let mut all_lines: Vec<Line> = Vec::new();

    for msg in &app.conversation.messages {
        if msg.role == "tool" {
            let content = msg.content.as_deref().unwrap_or("");
            for line in compact_tool_lines(content, app.tool_expanded) {
                all_lines.push(line);
            }
            continue;
        }
        let role_lines = md.render(msg.content.as_deref().unwrap_or(""), &msg.role);
        for line in role_lines {
            all_lines.push(line);
        }
        all_lines.push(Line::from(Span::raw("")));
    }

    if !app.streaming_text.is_empty() {
        for line in md.render(&app.streaming_text, "assistant") {
            all_lines.push(line);
        }
        all_lines.push(Line::from(Span::styled(
            "█",
            Style::default().fg(Color::Green).add_modifier(Modifier::SLOW_BLINK),
        )));
    } else if app.event_rx.is_some() {
        app.spinner_tick = app.spinner_tick.wrapping_add(1);
        let frames = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
        let f = frames[(app.spinner_tick / 4) % frames.len()];
        all_lines.push(Line::from(Span::styled(
            format!("{} working… (Esc to cancel)", f),
            Style::default().fg(Color::Yellow),
        )));
    }

    // bottom-follow scrolling
    let total = all_lines.len();
    let visible = area.height.max(1) as usize;
    if app.should_auto_scroll {
        app.chat_scroll = total.saturating_sub(visible);
    }
    let scroll = app.chat_scroll.min(total.saturating_sub(1).max(0));
    let visible_lines: Vec<Line> = all_lines.into_iter().skip(scroll).take(visible).collect();
    frame.render_widget(Paragraph::new(visible_lines).wrap(Wrap { trim: false }), area);
}

/// Compact tool lines: `● read path` + `└─ ok` collapsed, full when expanded.
fn compact_tool_lines(content: &str, expanded: bool) -> Vec<Line<'_>> {
    let first = content.lines().next().unwrap_or("").trim();
    let is_call = first.starts_with('▸');
    let is_result = first.starts_with('◂') || content.trim_start().starts_with('◂');
    if is_call {
        // `▸ **name** args` -> `● name args…`
        let short = first
            .replace('▸', "●")
            .replace("**", "")
            .trim()
            .to_string();
        let short = truncate(&short, 120);
        return vec![Line::from(Span::styled(short, Style::default().fg(Color::Yellow)))];
    }
    if is_result {
        let is_err = content.to_lowercase().contains("error");
        let color = if is_err { Color::Red } else { Color::Green };
        if !expanded {
            let summary = first.replace('◂', "└─").replace("**", "");
            let summary = truncate(summary.trim(), 120);
            return vec![Line::from(Span::styled(summary, Style::default().fg(color)))];
        }
        // expanded: full but capped
        let mut out = Vec::new();
        for (i, l) in content.lines().take(30).enumerate() {
            if i == 0 {
                out.push(Line::from(Span::styled(
                    l.replace('◂', "└─"),
                    Style::default().fg(color),
                )));
            } else {
                out.push(Line::from(Span::styled(
                    truncate(l, 160),
                    Style::default().fg(Color::DarkGray),
                )));
            }
        }
        if content.lines().count() > 30 {
            out.push(Line::from(Span::styled(
                format!("… {} more (Ctrl+O to collapse)", content.lines().count() - 30),
                Style::default().fg(Color::DarkGray),
            )));
        }
        return out;
    }
    // fallback: dim single block
    vec![Line::from(Span::styled(
        truncate(content, 160),
        Style::default().fg(Color::DarkGray),
    ))]
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let t: String = s.chars().take(max - 1).collect();
    format!("{}…", t)
}

fn render_input(frame: &mut Frame, area: Rect, app: &App) {
    let busy = app.event_rx.is_some();
    let mut title = if busy { " working… " } else { " › " };
    if app.input.buffer.starts_with('/') {
        title = " / ";
    } else if app.input.buffer.contains('@') {
        title = " @ ";
    }
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(ratatui::widgets::BorderType::Rounded)
        .title(title)
        .border_style(if busy {
            Style::default().fg(Color::Yellow)
        } else {
            Style::default().fg(Color::DarkGray)
        });

    if busy {
        let p = Paragraph::new(" (Esc to cancel)").block(block).style(Style::default().fg(Color::DarkGray));
        frame.render_widget(p, area);
        return;
    }

    let mut display = app.input.buffer.clone();
    // block cursor
    if app.input.cursor_pos <= display.len() {
        display.insert(app.input.cursor_pos, '█');
    }
    let hint = if display.is_empty() && !app.input.buffer.starts_with('/') {
        Span::styled("message, / commands, @ files", Style::default().fg(Color::DarkGray))
    } else {
        Span::raw("")
    };
    let inner = if display.is_empty() {
        vec![Line::from(hint)]
    } else {
        vec![Line::from(Span::raw(display))]
    };
    let p = Paragraph::new(inner).block(block).wrap(Wrap { trim: false });
    frame.render_widget(p, area);
}

fn render_palette(frame: &mut Frame, chat: Rect, input: Rect, app: &App) {
    let items = crate::tui::input::command_palette(&app.input.buffer);
    if items.is_empty() {
        return;
    }
    let height = (items.len().min(8) + 2).min(12) as u16;
    let width = input.width.min(60);
    let x = input.x;
    let y = input.y.saturating_sub(height);
    let area = Rect::new(x, y, width, height);
    let _ = chat;
    frame.render_widget(Clear, area);
    let list_items: Vec<ListItem> = items
        .iter()
        .enumerate()
        .map(|(i, (name, desc))| {
            let style = if i == app.palette_idx % items.len() {
                Style::default().fg(Color::Black).bg(Color::Cyan)
            } else {
                Style::default().fg(Color::White)
            };
            ListItem::new(Line::from(vec![
                Span::styled(format!(" {:<10}", name), style),
                Span::styled(format!(" {}", desc), Style::default().fg(Color::DarkGray)),
            ]))
        })
        .collect();
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(ratatui::widgets::BorderType::Rounded)
        .border_style(Style::default().fg(Color::DarkGray));
    frame.render_widget(List::new(list_items).block(block), area);
}

fn render_status(frame: &mut Frame, area: Rect, app: &App) {
    let ctx = (app.context_usage() * 100.0) as u32;
    let ctx_color = if ctx > 85 { Color::Red } else if ctx > 60 { Color::Yellow } else { Color::DarkGray };
    let mut parts = vec![
        Span::styled(format!(" {}", app.cwd_short()), Style::default().fg(Color::DarkGray)),
        Span::styled(" │ ", Style::default().fg(Color::DarkGray)),
        Span::styled(format!("{}", app.current_model), Style::default().fg(Color::Magenta)),
        Span::styled(" │ ", Style::default().fg(Color::DarkGray)),
        Span::styled(format!("msgs:{}", app.conversation.messages.len()), Style::default().fg(Color::DarkGray)),
        Span::styled(" │ ", Style::default().fg(Color::DarkGray)),
        Span::styled(format!("ctx:{}%", ctx), Style::default().fg(ctx_color)),
    ];
    if !app.status.token_count.is_empty() {
        parts.push(Span::styled(" │ ", Style::default().fg(Color::DarkGray)));
        parts.push(Span::styled(format!("{}", app.status.token_count), Style::default().fg(Color::DarkGray)));
    }
    if app.event_rx.is_some() {
        parts.push(Span::styled(" │ working…", Style::default().fg(Color::Yellow)));
    } else if app.status.tool_status != "idle" && !app.status.tool_status.is_empty() {
        parts.push(Span::styled(format!(" │ {}", app.status.tool_status), Style::default().fg(Color::Yellow)));
    }
    if let Some(n) = &app.notice {
        parts.push(Span::styled(" │ ", Style::default().fg(Color::DarkGray)));
        parts.push(Span::styled(format!("{}", truncate(n, 60)), Style::default().fg(Color::Cyan)));
    }
    if !app.tool_expanded {
    }
    let p = Paragraph::new(Line::from(parts));
    frame.render_widget(p, area);
}
