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
            Constraint::Length(1),
        ])
        .split(area);

    if app.tree_visible {
        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(32), Constraint::Min(1)])
            .split(chunks[0]);
        render_tree(frame, cols[0], app);
        render_chat(frame, cols[1], app);
    } else {
        render_chat(frame, chunks[0], app);
    }
    render_input(frame, chunks[1], app);
    // pickers overlay above input (model/login/choice pickers take over palette)
    let model_mode = crate::tui::input::model_filter(&app.input.buffer).is_some();
    let login_mode = app.input.buffer.starts_with("/login ");
    let choice = crate::tui::input::choice_mode(&app.input.buffer);
    let idle = app.event_rx.is_none() && app.pending_approval.is_none() && app.pending_login.is_none();
    if model_mode && idle {
        render_model_picker(frame, chunks[0], chunks[1], app);
    } else if login_mode && idle {
        render_login_picker(frame, chunks[0], chunks[1], app);
    } else if let Some((mode, _)) = choice {
        if idle {
            render_choice_picker(frame, chunks[0], chunks[1], app, &mode);
        }
    } else if app.input.buffer.starts_with('/') && idle {
        render_palette(frame, chunks[0], chunks[1], app);
    }
    render_input_footer(frame, chunks[2], app);
    render_status(frame, chunks[3], app);
    if app.pending_approval.is_some() {
        render_permission_modal(frame, area, app);
    }
}

/// OpenCode-style input footer: active `provider/model` at the typing point
/// (status bar no longer duplicates it) + approval state + `@file` chips.
fn render_input_footer(frame: &mut Frame, area: Rect, app: &App) {
    let t = &app.theme;
    let mut parts = vec![
        Span::styled(
            format!(" {}/{}", app.provider_name, truncate(&app.current_model, 32)),
            Style::default().fg(t.accent),
        ),
    ];
    parts.push(Span::styled(" · ", Style::default().fg(t.muted)));
    if app.permission_gate.auto_approve() {
        parts.push(Span::styled("auto", Style::default().fg(t.muted)));
    } else {
        parts.push(Span::styled("ask", Style::default().fg(t.warning)));
    }
    for f in crate::tui::input::mention_tokens(&app.input.buffer) {
        parts.push(Span::styled(" · ", Style::default().fg(t.muted)));
        parts.push(Span::styled(format!("@{}", truncate(&f, 24)), Style::default().fg(t.accent)));
    }
    if app.pending_login.is_some() {
        parts.push(Span::styled(" · login…", Style::default().fg(t.warning)));
    } else if app.pending_approval.is_some() {
        parts.push(Span::styled(" · approval…", Style::default().fg(t.warning)));
    }
    frame.render_widget(Paragraph::new(Line::from(parts)), area);
}

fn render_tree(frame: &mut Frame, area: Rect, app: &App) {
    let t = &app.theme;
    let mut lines: Vec<Line> = Vec::new();
    if app.workspace.is_git_repo {
        lines.push(Line::from(vec![
            Span::styled("⎇ ", Style::default().fg(t.accent)),
            Span::styled(
                app.workspace.git_branch.clone(),
                Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
            ),
        ]));
        if !app.workspace.git_status.is_empty() {
            lines.push(Line::from(Span::styled(
                format!("~{} changed", app.workspace.git_status.len()),
                Style::default().fg(t.warning),
            )));
        }
    }
    let tree_lines: Vec<&str> = app.workspace.file_tree.lines().take(60).collect();
    if tree_lines.is_empty() {
        lines.push(Line::from(Span::styled("(empty)", Style::default().fg(t.muted))));
    } else {
        for l in tree_lines {
            lines.push(Line::from(Span::styled(
                truncate(l, 30),
                Style::default().fg(t.muted),
            )));
        }
    }
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(ratatui::widgets::BorderType::Rounded)
        .title(" tree ")
        .border_style(Style::default().fg(t.muted));
    frame.render_widget(Paragraph::new(lines).block(block), area);
}

fn render_chat(frame: &mut Frame, area: Rect, app: &mut App) {
    let t = app.theme.clone();
    if app.is_home && app.conversation.messages.is_empty() && app.streaming_text.is_empty() {
        let lines = vec![
            Line::from(Span::raw("")),
            Line::from(Span::styled(
                "  barong",
                Style::default().fg(t.primary).add_modifier(Modifier::BOLD),
            )),
            Line::from(Span::styled(
                "  Terminal coding agent",
                Style::default().fg(t.muted),
            )),
            Line::from(Span::raw("")),
            Line::from(Span::styled(
                "  Type a message to start  •  / commands  •  @ files  •  Esc cancel",
                Style::default().fg(t.muted),
            )),
            Line::from(Span::styled(
                format!("  {}  •  {}  •  {} tools  •  {} theme", app.cwd_short(), app.current_model, app.tool_registry.tool_names().len(), t.name),
                Style::default().fg(t.muted),
            )),
        ];
        let p = Paragraph::new(lines);
        frame.render_widget(p, area);
        return;
    }

    let md = crate::tui::markdown::MarkdownRenderer::with_theme(&app.theme).with_width(area.width);
    let mut all_lines: Vec<Line> = Vec::new();

    for msg in &app.conversation.messages {
        if msg.role == "tool" {
            let content = msg.content.as_deref().unwrap_or("");
            for line in compact_tool_lines(content, app.tool_expanded, &t) {
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
            Style::default().fg(t.primary).add_modifier(Modifier::SLOW_BLINK),
        )));
    } else if app.event_rx.is_some() {
        app.spinner_tick = app.spinner_tick.wrapping_add(1);
        let frames = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
        let f = frames[(app.spinner_tick / 4) % frames.len()];
        all_lines.push(Line::from(Span::styled(
            format!("{} working… (Esc to cancel)", f),
            Style::default().fg(t.warning),
        )));
    }

    // bottom-follow scrolling
    let total = all_lines.len();
    let visible = area.height.max(1) as usize;
    if app.should_auto_scroll {
        app.chat_scroll = total.saturating_sub(visible);
    }
    let scroll = app.chat_scroll.min(total.saturating_sub(1).max(0));
    // Scrolled back to the bottom (wheel/PgDn) → resume follow mode.
    if scroll + visible >= total {
        app.should_auto_scroll = true;
    }
    let visible_lines: Vec<Line> = all_lines.into_iter().skip(scroll).take(visible).collect();
    frame.render_widget(Paragraph::new(visible_lines).wrap(Wrap { trim: false }), area);
}

/// Compact tool lines: `● read path` + `└─ ok` collapsed, full when expanded.
fn compact_tool_lines<'a>(content: &'a str, expanded: bool, t: &'a crate::tui::theme::Theme) -> Vec<Line<'a>> {
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
        return vec![Line::from(Span::styled(short, Style::default().fg(t.warning)))];
    }
    if is_result {
        let is_err = content.to_lowercase().contains("error") || content.to_lowercase().contains("denied");
        let color = if is_err { t.error } else { t.success };
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
                    Style::default().fg(t.muted),
                )));
            }
        }
        if content.lines().count() > 30 {
            out.push(Line::from(Span::styled(
                format!("… {} more (Ctrl+O to collapse)", content.lines().count() - 30),
                Style::default().fg(t.muted),
            )));
        }
        return out;
    }
    // fallback: dim single block
    vec![Line::from(Span::styled(
        truncate(content, 160),
        Style::default().fg(t.muted),
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
    let t = &app.theme;
    let busy = app.event_rx.is_some();
    // OpenCode-style: no border box — full-bleed panel backdrop + text.
    // The backdrop is a borderless block so wrap/clip never breaks alignment.
    frame.render_widget(
        Block::default().style(Style::default().bg(t.panel)),
        area,
    );
    // Small horizontal inset so text doesn't touch the terminal edge.
    let inner = Rect::new(
        area.x.saturating_add(1),
        area.y,
        area.width.saturating_sub(1),
        area.height,
    );

    if let Some(pid) = &app.pending_login {
        let masked = "•".repeat(app.login_buffer.chars().count().min(48));
        let line = Line::from(vec![
            Span::styled(format!("key for {}: ", pid), Style::default().fg(t.accent).bg(t.panel)),
            Span::raw(masked),
            Span::styled("█", Style::default().fg(t.accent).bg(t.panel).add_modifier(Modifier::SLOW_BLINK)),
        ]);
        frame.render_widget(Paragraph::new(line), inner);
        return;
    }

    if busy {
        let label = if app.pending_approval.is_some() {
            "(y/a/n to decide)"
        } else {
            "(Esc to cancel)"
        };
        let line = Line::from(Span::styled(label, Style::default().fg(t.muted).bg(t.panel)));
        frame.render_widget(Paragraph::new(line), inner);
        return;
    }

    let mut display = app.input.buffer.clone();
    // block cursor
    if app.input.cursor_pos <= display.len() {
        display.insert(app.input.cursor_pos, '█');
    }
    let bg = Style::default().bg(t.panel);
    // Breathing room: one empty row on top, content below it.
    let mut rows = vec![Line::from(Span::styled("", bg))];
    if app.input.buffer.is_empty() && !app.input.buffer.starts_with('/') {
        rows.push(Line::from(vec![
            Span::styled("█", Style::default().fg(t.accent).bg(t.panel)),
            Span::styled("message, / commands, @ files", Style::default().fg(t.muted).bg(t.panel)),
        ]));
    } else {
        rows.extend(
            display
                .split('\n')
                .map(|row| Line::from(Span::styled(row.to_string(), bg))),
        );
    }
    // Keep the cursor's row visible (Shift+Enter can grow past the box).
    let cursor_row = 1 + display
        .chars()
        .take(app.input.cursor_pos)
        .filter(|&c| c == '\n')
        .count();
    let visible = area.height.max(1) as usize;
    let scroll = cursor_row.saturating_sub(visible.saturating_sub(1)) as u16;
    let p = Paragraph::new(rows)
        .wrap(Wrap { trim: false })
        .scroll((scroll, 0));
    frame.render_widget(p, inner);
}

fn render_palette(frame: &mut Frame, chat: Rect, input: Rect, app: &App) {
    let items = crate::tui::input::command_palette(&app.input.buffer);
    if items.is_empty() {
        return;
    }
    // Scrolling window so selected item is always visible.
    let visible = items.len().min(8);
    let selected = app.palette_idx % items.len();
    // Keep selected inside window: pin start so window contains selected.
    let max_start = items.len().saturating_sub(visible);
    let start = (selected.saturating_sub(visible.saturating_sub(1))).min(max_start);
    let end = (start + visible).min(items.len());
    let height = (visible + 2) as u16;
    let width = input.width.min(60);
    let x = input.x;
    let y = input.y.saturating_sub(height);
    let area = Rect::new(x, y, width, height);
    let _ = chat;
    frame.render_widget(Clear, area);
    let list_items: Vec<ListItem> = items[start..end]
        .iter()
        .enumerate()
        .map(|(i, (name, desc))| {
            let style = if start + i == selected {
                Style::default().fg(Color::Black).bg(app.theme.accent)
            } else {
                Style::default().fg(Color::White)
            };
            ListItem::new(Line::from(vec![
                Span::styled(format!(" {:<10}", name), style),
                Span::styled(format!(" {}", desc), Style::default().fg(app.theme.muted)),
            ]))
        })
        .collect();
    let title = if items.len() > visible {
        format!(" {}/{} ", selected + 1, items.len())
    } else {
        String::new()
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(ratatui::widgets::BorderType::Rounded)
        .title(title)
        .border_style(Style::default().fg(app.theme.muted));
    frame.render_widget(List::new(list_items).block(block), area);
}

fn render_model_picker(frame: &mut Frame, chat: Rect, input: Rect, app: &App) {
    let filter = crate::tui::input::model_filter(&app.input.buffer).unwrap_or("").to_string();
    let items = crate::tui::input::model_entries(app, &filter);
    if items.is_empty() {
        return;
    }
    let visible = items.len().min(8);
    let selected = app.model_idx % items.len();
    let max_start = items.len().saturating_sub(visible);
    let start = (selected.saturating_sub(visible.saturating_sub(1))).min(max_start);
    let end = (start + visible).min(items.len());
    let height = (visible + 2) as u16;
    let width = input.width.min(64);
    let x = input.x;
    let y = input.y.saturating_sub(height);
    let area = Rect::new(x, y, width, height);
    let _ = chat;
    frame.render_widget(Clear, area);
    let list_items: Vec<ListItem> = items[start..end]
        .iter()
        .enumerate()
        .map(|(i, (p, m, ready))| {
            let style = if start + i == selected {
                Style::default().fg(Color::Black).bg(app.theme.accent)
            } else {
                Style::default().fg(Color::White)
            };
            let dot = if *ready { "●" } else { "○" };
            let dot_color = if *ready { app.theme.success } else { app.theme.muted };
            ListItem::new(Line::from(vec![
                Span::styled(format!(" {} ", dot), Style::default().fg(dot_color)),
                Span::styled(format!("{:<10}", p), style),
                Span::styled(format!(" {}", truncate(&crate::tui::input::model_label(p, m), 34)), Style::default().fg(app.theme.muted)),
            ]))
        })
        .collect();
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(ratatui::widgets::BorderType::Rounded)
        .title(format!(" model {}/{} — ● key ready ", selected + 1, items.len()))
        .border_style(Style::default().fg(app.theme.muted));
    frame.render_widget(List::new(list_items).block(block), area);
}

fn render_login_picker(frame: &mut Frame, chat: Rect, input: Rect, app: &App) {
    let filter = app.input.buffer["/login ".len()..].to_string();
    let items = crate::tui::input::provider_entries(app, &filter);
    if items.is_empty() {
        return;
    }
    let visible = items.len().min(8);
    let selected = app.login_idx % items.len();
    let max_start = items.len().saturating_sub(visible);
    let start = (selected.saturating_sub(visible.saturating_sub(1))).min(max_start);
    let end = (start + visible).min(items.len());
    let height = (visible + 2) as u16;
    let width = input.width.min(64);
    let x = input.x;
    let y = input.y.saturating_sub(height);
    let area = Rect::new(x, y, width, height);
    let _ = chat;
    frame.render_widget(Clear, area);
    let list_items: Vec<ListItem> = items[start..end]
        .iter()
        .enumerate()
        .map(|(i, (pid, ready, base))| {
            let style = if start + i == selected {
                Style::default().fg(Color::Black).bg(app.theme.accent)
            } else {
                Style::default().fg(Color::White)
            };
            let dot = if *ready { "●" } else { "○" };
            let dot_color = if *ready { app.theme.success } else { app.theme.muted };
            let endpoint = if base.is_empty() { "(default endpoint)".to_string() } else { truncate(base, 30) };
            ListItem::new(Line::from(vec![
                Span::styled(format!(" {} ", dot), Style::default().fg(dot_color)),
                Span::styled(format!("{:<10}", pid), style),
                Span::styled(format!(" {}", endpoint), Style::default().fg(app.theme.muted)),
            ]))
        })
        .collect();
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(ratatui::widgets::BorderType::Rounded)
        .title(format!(" login {}/{} — ● key saved ", selected + 1, items.len()))
        .border_style(Style::default().fg(app.theme.muted));
    frame.render_widget(List::new(list_items).block(block), area);
}

fn render_choice_picker(frame: &mut Frame, chat: Rect, input: Rect, app: &App, mode: &crate::tui::input::ChoiceMode) {
    let filter = crate::tui::input::choice_mode(&app.input.buffer).map(|(_, f)| f).unwrap_or_default();
    let items = crate::tui::input::choice_entries(app, mode, &filter);
    if items.is_empty() {
        return;
    }
    let visible = items.len().min(8);
    let selected = app.choice_idx % items.len();
    let max_start = items.len().saturating_sub(visible);
    let start = (selected.saturating_sub(visible.saturating_sub(1))).min(max_start);
    let end = (start + visible).min(items.len());
    let height = (visible + 2) as u16;
    let width = input.width.min(64);
    let x = input.x;
    let y = input.y.saturating_sub(height);
    let area = Rect::new(x, y, width, height);
    let _ = chat;
    frame.render_widget(Clear, area);
    let list_items: Vec<ListItem> = items[start..end]
        .iter()
        .enumerate()
        .map(|(i, (value, desc))| {
            let style = if start + i == selected {
                Style::default().fg(Color::Black).bg(app.theme.accent)
            } else {
                Style::default().fg(Color::White)
            };
            ListItem::new(Line::from(vec![
                Span::styled(format!(" {:<12}", value), style),
                Span::styled(format!(" {}", truncate(desc, 44)), Style::default().fg(app.theme.muted)),
            ]))
        })
        .collect();
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(ratatui::widgets::BorderType::Rounded)
        .title(format!("{} {}/{} ", crate::tui::input::choice_title(mode).trim(), selected + 1, items.len()))
        .border_style(Style::default().fg(app.theme.muted));
    frame.render_widget(List::new(list_items).block(block), area);
}

fn render_status(frame: &mut Frame, area: Rect, app: &App) {
    let t = &app.theme;
    let ctx = (app.context_usage() * 100.0) as u32;
    let ctx_color = if ctx > 85 { t.error } else if ctx > 60 { t.warning } else { t.muted };
    let mut parts = vec![
        Span::styled(format!(" {}", app.cwd_short()), Style::default().fg(t.muted)),
        Span::styled(" │ ", Style::default().fg(t.muted)),
        Span::styled(format!("msgs:{}", app.conversation.messages.len()), Style::default().fg(t.muted)),
        Span::styled(" │ ", Style::default().fg(t.muted)),
        Span::styled(format!("ctx:{}%", ctx), Style::default().fg(ctx_color)),
        Span::styled(" │ ", Style::default().fg(t.muted)),
        Span::styled(format!("{}", t.name), Style::default().fg(t.muted)),
    ];
    if app.tree_visible {
        parts.push(Span::styled(" │ tree", Style::default().fg(t.accent)));
    }
    if !app.status.token_count.is_empty() {
        parts.push(Span::styled(" │ ", Style::default().fg(t.muted)));
        parts.push(Span::styled(format!("{}", app.status.token_count), Style::default().fg(t.muted)));
    }
    if app.event_rx.is_some() {
        parts.push(Span::styled(" │ working…", Style::default().fg(t.warning)));
    } else if app.status.tool_status != "idle" && !app.status.tool_status.is_empty() {
        parts.push(Span::styled(format!(" │ {}", app.status.tool_status), Style::default().fg(t.warning)));
    }
    if let Some(n) = &app.notice {
        parts.push(Span::styled(" │ ", Style::default().fg(t.muted)));
        parts.push(Span::styled(format!("{}", truncate(n, 60)), Style::default().fg(t.accent)));
    }
    let p = Paragraph::new(Line::from(parts));
    frame.render_widget(p, area);
}

fn render_permission_modal(frame: &mut Frame, area: Rect, app: &App) {
    let Some(p) = &app.pending_approval else { return };
    let t = &app.theme;
    let width = area.width.min(76);
    let height = 9u16;
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    let modal = Rect::new(x, y, width, height);
    frame.render_widget(Clear, modal);
    let args_str = serde_json::to_string(&p.args).unwrap_or_default();
    let args_short = truncate(&args_str, 120);
    let lines = vec![
        Line::from(Span::styled(
            format!(" Allow `{}` ?", p.name),
            Style::default().fg(t.warning).add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(args_short, Style::default().fg(t.muted))),
        Line::from(Span::raw("")),
        Line::from(vec![
            Span::styled(" [y] once ", Style::default().fg(Color::Black).bg(t.success)),
            Span::raw(" "),
            Span::styled(" [a] always this session ", Style::default().fg(Color::Black).bg(t.accent)),
            Span::raw(" "),
            Span::styled(" [n] deny ", Style::default().fg(Color::Black).bg(t.error)),
        ]),
        Line::from(Span::styled(" Esc = deny", Style::default().fg(t.muted))),
    ];
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(ratatui::widgets::BorderType::Rounded)
        .title(" permission ")
        .border_style(Style::default().fg(t.warning));
    frame.render_widget(Paragraph::new(lines).block(block), modal);
}
