use crate::app::App;
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Wrap};

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
    // One picker overlay for every mode (same chrome, same keys).
    let idle =
        app.event_rx.is_none() && app.pending_approval.is_none() && app.pending_login.is_none();
    if idle {
        if let Some((kind, filter)) = crate::tui::input::picker_kind(&app.input.buffer) {
            let rows = crate::tui::input::picker_rows(app, &kind, &filter);
            if !rows.is_empty() {
                let title = crate::tui::input::picker_title(&kind);
                render_picker(frame, chunks[0], chunks[1], app, title, &rows);
            }
        } else if app.input.buffer.starts_with('/') {
            // Text after a space ("/foo bar") matches no picker: show nothing.
            let items = crate::tui::input::command_palette(&app.input.buffer);
            if !items.is_empty() && !app.input.buffer.contains([' ', '\n']) {
                let rows: Vec<crate::tui::input::PickerRow> = items
                    .into_iter()
                    .map(|(n, d)| crate::tui::input::PickerRow {
                        complete: format!("{} ", n),
                        left: format!("{:<12}", n),
                        right: d.to_string(),
                        dot: None,
                    })
                    .collect();
                render_picker(frame, chunks[0], chunks[1], app, "", &rows);
            }
        }
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
    let mut parts = vec![Span::styled(
        format!(
            " {}/{}",
            app.provider_name,
            truncate(&app.current_model, 32)
        ),
        Style::default().fg(t.accent),
    )];
    parts.push(Span::styled(" · ", Style::default().fg(t.muted)));
    if app.permission_gate.auto_approve() {
        parts.push(Span::styled("auto", Style::default().fg(t.muted)));
    } else {
        parts.push(Span::styled("ask", Style::default().fg(t.warning)));
    }
    for f in crate::tui::input::mention_tokens(&app.input.buffer) {
        parts.push(Span::styled(" · ", Style::default().fg(t.muted)));
        parts.push(Span::styled(
            format!("@{}", truncate(&f, 24)),
            Style::default().fg(t.accent),
        ));
    }
    if !app.queued_input.is_empty() {
        parts.push(Span::styled(" · ", Style::default().fg(t.muted)));
        parts.push(Span::styled(
            format!("{} queued", app.queued_input.len()),
            Style::default().fg(t.accent),
        ));
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
        lines.push(Line::from(Span::styled(
            "(empty)",
            Style::default().fg(t.muted),
        )));
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
                format!(
                    "  {}  •  {}  •  {} tools  •  {} theme",
                    app.cwd_short(),
                    app.current_model,
                    app.tool_registry.tool_names().len(),
                    t.name
                ),
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
            Style::default()
                .fg(t.primary)
                .add_modifier(Modifier::SLOW_BLINK),
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

    // Bottom-follow scrolling.
    //
    // `Paragraph` wraps at render time and `Paragraph::scroll` skips *wrapped*
    // rows, so the transcript has to be measured in physical rows. Counting
    // logical lines left the newest text off-screen as soon as a paragraph
    // wrapped, and made PgUp/PgDn move a different number of rows per press.
    let wrap = Wrap { trim: false };
    let total = crate::tui::wrap::wrapped_row_count(&all_lines, area.width);
    let visible = area.height.max(1) as usize;
    // Start row of the last page — the furthest down that still fills the view.
    let max_scroll = total.saturating_sub(visible);
    if app.should_auto_scroll {
        app.chat_scroll = max_scroll;
    }
    // Back in the last page (wheel/PgDn) → resume following the tail.
    if app.chat_scroll >= max_scroll {
        app.chat_scroll = max_scroll;
        app.should_auto_scroll = true;
    }
    let scroll = u16::try_from(app.chat_scroll).unwrap_or(u16::MAX);
    frame.render_widget(
        Paragraph::new(all_lines).wrap(wrap).scroll((scroll, 0)),
        area,
    );
}

/// Compact tool lines: `● read path` + `└─ ok` collapsed, full when expanded.
fn compact_tool_lines<'a>(
    content: &'a str,
    expanded: bool,
    t: &'a crate::tui::theme::Theme,
) -> Vec<Line<'a>> {
    let first = content.lines().next().unwrap_or("").trim();
    let is_call = first.starts_with('▸');
    let is_result = first.starts_with('◂') || content.trim_start().starts_with('◂');
    if is_call {
        // `▸ **name** args` -> `● name args…`
        let short = first.replace('▸', "●").replace("**", "").trim().to_string();
        let short = truncate(&short, 120);
        return vec![Line::from(Span::styled(
            short,
            Style::default().fg(t.warning),
        ))];
    }
    if is_result {
        let is_err =
            content.to_lowercase().contains("error") || content.to_lowercase().contains("denied");
        let color = if is_err { t.error } else { t.success };
        if !expanded {
            let summary = first.replace('◂', "└─").replace("**", "");
            let summary = truncate(summary.trim(), 120);
            return vec![Line::from(Span::styled(
                summary,
                Style::default().fg(color),
            ))];
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
                format!(
                    "… {} more (Ctrl+O to collapse)",
                    content.lines().count() - 30
                ),
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

pub(crate) fn truncate(s: &str, max: usize) -> String {
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
    frame.render_widget(Block::default().style(Style::default().bg(t.panel)), area);
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
            Span::styled(
                format!("key for {}: ", pid),
                Style::default().fg(t.accent).bg(t.panel),
            ),
            Span::raw(masked),
            Span::styled(
                "█",
                Style::default()
                    .fg(t.accent)
                    .bg(t.panel)
                    .add_modifier(Modifier::SLOW_BLINK),
            ),
        ]);
        frame.render_widget(Paragraph::new(line), inner);
        return;
    }

    let mut display = app.input.buffer.clone();
    // block cursor — cursor_pos is a byte offset, snap it to a boundary first
    let cursor = app.input.cursor();
    display.insert(cursor, '█');
    let bg = Style::default().bg(t.panel);
    // Row 0 is chrome, the draft starts on row 1: a blank breathing row when
    // idle, the cancel hint while the agent works (the draft stays visible and
    // editable — Enter queues it instead of throwing it away).
    let mut rows = vec![if busy {
        let label = if app.pending_approval.is_some() {
            "(y/a/n to decide)"
        } else {
            "(Esc to cancel · Enter queues)"
        };
        Line::from(Span::styled(
            label,
            Style::default().fg(t.muted).bg(t.panel),
        ))
    } else {
        Line::from(Span::styled("", bg))
    }];
    let placeholder = if busy {
        "type the next message, Enter queues it"
    } else {
        "message, / commands, @ files"
    };
    if app.input.buffer.is_empty() && !app.input.buffer.starts_with('/') {
        rows.push(Line::from(vec![
            Span::styled("█", Style::default().fg(t.accent).bg(t.panel)),
            Span::styled(placeholder, Style::default().fg(t.muted).bg(t.panel)),
        ]));
    } else {
        rows.extend(
            display
                .split('\n')
                .map(|row| Line::from(Span::styled(row.to_string(), bg))),
        );
    }
    // Keep the cursor's row visible. `Paragraph` wraps and scrolls in PHYSICAL
    // rows, so the cursor's row has to be counted the same way: a long single
    // line is many rows tall and used to scroll the cursor clean off the panel.
    let cursor_row = 1 + physical_row_of_cursor(&display, cursor, inner.width.max(1));
    let visible = area.height.max(1) as usize;
    // Park the cursor on the last visible row.
    let scroll = (cursor_row + 1).saturating_sub(visible);
    let p = Paragraph::new(rows)
        .wrap(Wrap { trim: false })
        .scroll((scroll as u16, 0));
    frame.render_widget(p, inner);
}

/// Physical row (0-based, inside the input panel) holding the block cursor.
///
/// `cursor` is the byte offset where the block glyph was inserted into
/// `display`. The glyph itself is included in the measured prefix, so a cursor
/// sitting exactly on a wrap boundary lands on the row it is painted on rather
/// than the one before it.
fn physical_row_of_cursor(display: &str, cursor: usize, width: u16) -> usize {
    const BLOCK: char = '█';
    let end = (floor_boundary(display, cursor) + BLOCK.len_utf8()).min(display.len());
    let head = &display[..end];
    let mut rows = 0usize;
    let mut rest = head;
    // Complete lines before the cursor's own line.
    while let Some(idx) = rest.find('\n') {
        rows += crate::tui::wrap::physical_rows(&rest[..idx], width);
        rest = &rest[idx + 1..];
    }
    rows + crate::tui::wrap::physical_rows(rest, width).saturating_sub(1)
}

/// Largest char boundary at or below `at`.
fn floor_boundary(s: &str, at: usize) -> usize {
    let mut i = at.min(s.len());
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

/// The one picker overlay for every mode: same window math, same highlight,
/// same keys. `title` empty = bare command list; otherwise `kind n/m`.
fn render_picker(
    frame: &mut Frame,
    chat: Rect,
    input: Rect,
    app: &App,
    title: &str,
    rows: &[crate::tui::input::PickerRow],
) {
    if rows.is_empty() {
        return;
    }
    // The picker floats just above the input, so it may never grow taller than
    // the space up to it — on a short terminal the old fixed 8-row window
    // covered the input box completely.
    let room = input.y.saturating_sub(2); // room for the two borders
    let visible = rows.len().min(8).min(room.max(1) as usize);
    let selected = app.picker_idx % rows.len();
    // Keep selected inside window: pin start so window contains selected.
    let max_start = rows.len().saturating_sub(visible);
    let start = (selected.saturating_sub(visible.saturating_sub(1))).min(max_start);
    let end = (start + visible).min(rows.len());
    let height = (visible + 2) as u16;
    let width = input.width.min(64);
    let x = input.x;
    let y = input.y.saturating_sub(height);
    let area = Rect::new(x, y, width, height);
    let _ = chat;
    frame.render_widget(Clear, area);

    // Lay each row out to the inner width so an item is always exactly one
    // terminal row: a too-long cell used to wrap and double every entry.
    let inner_w = width.saturating_sub(2).max(1) as usize;
    let list_items: Vec<ListItem> = rows[start..end]
        .iter()
        .enumerate()
        .map(|(i, row)| {
            let is_selected = start + i == selected;
            // Terminal-default foreground, so the list stays readable on light
            // backgrounds (hardcoded white vanished there).
            let label_style = if is_selected {
                Style::default().fg(Color::Black).bg(app.theme.accent)
            } else {
                Style::default()
            };
            let detail_style = if is_selected {
                Style::default().fg(Color::Black).bg(app.theme.accent)
            } else {
                Style::default().fg(app.theme.muted)
            };
            let mut spans = Vec::new();
            let dot_w = if row.dot.is_some() { 3 } else { 0 };
            if let Some(ready) = row.dot {
                // On the accent background a muted dot disappears.
                let color = if is_selected {
                    Color::Black
                } else if ready {
                    app.theme.success
                } else {
                    app.theme.muted
                };
                spans.push(Span::styled(
                    if ready { " \u{25cf} " } else { " \u{25cb} " },
                    Style::default().fg(color),
                ));
            }
            let avail = inner_w.saturating_sub(dot_w);
            let left_w = avail.min(12);
            spans.push(Span::styled(
                format!(" {}", truncate(&row.left, left_w.max(1))),
                label_style,
            ));
            // Only spend cells on the description if it gets a readable run.
            let right_w = avail.saturating_sub(left_w.min(avail));
            if right_w >= 8 {
                let room = right_w.saturating_sub(1);
                let text = if row.right.is_empty() {
                    String::new()
                } else {
                    format!(" {}", truncate(&row.right, room))
                };
                spans.push(Span::styled(text, detail_style));
            }
            // Pad out to the full inner width so the highlight spans the row.
            let painted: usize = spans.iter().map(|s| display_cells(&s.content)).sum();
            spans.push(Span::styled(
                " ".repeat(inner_w.saturating_sub(painted)),
                label_style,
            ));
            ListItem::new(Line::from(spans))
        })
        .collect();
    let title = if title.is_empty() {
        if rows.len() > visible {
            format!(" {}/{} ", selected + 1, rows.len())
        } else {
            String::new()
        }
    } else {
        format!(" {} {}/{} ", title, selected + 1, rows.len())
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(ratatui::widgets::BorderType::Rounded)
        .title(title)
        .border_style(Style::default().fg(app.theme.muted));
    frame.render_widget(List::new(list_items).block(block), area);
}

fn render_status(frame: &mut Frame, area: Rect, app: &App) {
    let t = &app.theme;
    let ctx = (app.context_usage() * 100.0) as u32;
    let ctx_color = if ctx > 85 {
        t.error
    } else if ctx > 60 {
        t.warning
    } else {
        t.muted
    };
    // `~` marks the reading as an estimate; without it the provider reported it.
    let ctx_label = if app.context_is_estimated() {
        format!("ctx:~{}%", ctx)
    } else {
        format!("ctx:{}%", ctx)
    };
    let mut parts = vec![
        Span::styled(
            format!(" {}", app.cwd_short()),
            Style::default().fg(t.muted),
        ),
        Span::styled(" │ ", Style::default().fg(t.muted)),
        Span::styled(
            format!("msgs:{}", app.conversation.messages.len()),
            Style::default().fg(t.muted),
        ),
        Span::styled(" │ ", Style::default().fg(t.muted)),
        Span::styled(ctx_label, Style::default().fg(ctx_color)),
        Span::styled(" │ ", Style::default().fg(t.muted)),
        Span::styled(t.name.clone(), Style::default().fg(t.muted)),
    ];
    if app.tree_visible {
        parts.push(Span::styled(" │ tree", Style::default().fg(t.accent)));
    }
    if !app.status.token_count.is_empty() {
        parts.push(Span::styled(" │ ", Style::default().fg(t.muted)));
        parts.push(Span::styled(
            app.status.token_count.clone(),
            Style::default().fg(t.muted),
        ));
    }
    if app.event_rx.is_some() {
        parts.push(Span::styled(" │ working…", Style::default().fg(t.warning)));
    } else if app.status.tool_status != "idle" && !app.status.tool_status.is_empty() {
        parts.push(Span::styled(
            format!(" │ {}", app.status.tool_status),
            Style::default().fg(t.warning),
        ));
    }
    if let Some(n) = &app.notice {
        parts.push(Span::styled(" │ ", Style::default().fg(t.muted)));
        parts.push(Span::styled(truncate(n, 60), Style::default().fg(t.accent)));
    }
    let p = Paragraph::new(Line::from(parts));
    frame.render_widget(p, area);
}

/// Rows of chrome around the body: top border, blank, key row, bottom border.
const APPROVAL_CHROME: u16 = 4;/// Payload lines shown before the modal admits what it hid.
const APPROVAL_MAX_LINES: usize = 24;

/// Cap a payload at `APPROVAL_MAX_LINES`, returning the head and how many
/// lines were dropped.
fn capped(body: &str) -> (String, usize) {
    let lines: Vec<&str> = body.lines().collect();
    if lines.len() <= APPROVAL_MAX_LINES {
        return (body.to_string(), 0);
    }
    (
        lines[..APPROVAL_MAX_LINES].join("\n"),
        lines.len() - APPROVAL_MAX_LINES,
    )
}

/// Terminal cells a string occupies (CJK-safe).
fn display_cells(s: &str) -> usize {
    use unicode_width::UnicodeWidthStr;
    UnicodeWidthStr::width(s)
}

fn arg_str(args: &serde_json::Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|k| args.get(*k).and_then(|v| v.as_str()))
        .map(|s| s.to_string())
        .filter(|s| !s.trim().is_empty())
}

fn push_wrapped(out: &mut Vec<Line<'static>>, text: &str, style: Style, width: u16) {
    for row in crate::tui::wrap::wrap_plain(text, width) {
        out.push(Line::from(Span::styled(row, style)));
    }
}

/// Payload lines with a `+`/`-`/`│` gutter, so an edit reads as a diff and a
/// write reads as the file body.
fn push_payload(out: &mut Vec<Line<'static>>, body: &str, sign: char, color: Color, width: u16) {
    let gutter = format!(" {} ", sign);
    let body_w = width.saturating_sub(gutter.len() as u16);
    let sign_style = Style::default().fg(color).add_modifier(Modifier::BOLD);
    for row in crate::tui::wrap::wrap_plain(body, body_w) {
        if row.trim().is_empty() {
            out.push(Line::from(Span::styled(gutter.clone(), sign_style)));
        } else {
            out.push(Line::from(vec![
                Span::styled(gutter.clone(), sign_style),
                Span::styled(row, Style::default()),
            ]));
        }
    }
}

/// Everything the user needs to judge the request. The old modal showed
/// `serde_json` output clipped at the modal edge, which for `bash` hid the
/// command being approved — the one thing that must never be unreadable.
fn approval_body(
    p: &crate::agent::permissions::PendingTool,
    width: u16,
    t: &crate::tui::theme::Theme,
) -> Vec<Line<'static>> {
    let mut out: Vec<Line<'static>> = Vec::new();
    let title = Style::default().fg(t.warning).add_modifier(Modifier::BOLD);
    out.push(Line::from(Span::styled(format!(" {} ", p.name), title)));

    let muted = Style::default().fg(t.muted);
    let path = arg_str(&p.args, &["path", "file_path"]);
    match p.name.as_str() {
        "bash" => {
            let cmd =
                arg_str(&p.args, &["command", "cmd"]).unwrap_or_else(|| "(no command)".into());
            push_wrapped(&mut out, &cmd, Style::default(), width);
            if let Some(dir) = arg_str(&p.args, &["workdir"]) {
                out.push(Line::from(Span::styled(format!("  in {}", dir), muted)));
            }
        }
        "write" => {
            if let Some(p) = &path {
                out.push(Line::from(Span::styled(
                    p.clone(),
                    Style::default().fg(t.accent),
                )));
            }
            let content = arg_str(&p.args, &["content"]).unwrap_or_default();
            out.push(Line::from(Span::styled(
                format!("  writes {} lines", content.lines().count()),
                muted,
            )));
            let (head, more) = capped(&content);
            push_payload(&mut out, &head, '+', t.success, width);
            if more > 0 {
                out.push(Line::from(Span::styled(
                    format!("  … {more} more lines",),
                    muted,
                )));
            }
        }
        "edit" => {
            if let Some(p) = &path {
                out.push(Line::from(Span::styled(
                    p.clone(),
                    Style::default().fg(t.accent),
                )));
            }
            if let Some(old) = arg_str(&p.args, &["old_string"]) {
                let (head, more) = capped(&old);
                push_payload(&mut out, &head, '-', t.error, width);
                if more > 0 {
                    out.push(Line::from(Span::styled(
                        format!("  … {more} more lines"),
                        muted,
                    )));
                }
            }
            if let Some(new) = arg_str(&p.args, &["new_string"]) {
                let (head, more) = capped(&new);
                push_payload(&mut out, &head, '+', t.success, width);
                if more > 0 {
                    out.push(Line::from(Span::styled(
                        format!("  … {more} more lines"),
                        muted,
                    )));
                }
            }
        }
        _ => {
            // delegate + any MCP tool: key/value rows beat a JSON blob.
            let obj = p.args.as_object().cloned().unwrap_or_default();
            if obj.is_empty() {
                out.push(Line::from(Span::styled("  (no arguments)", muted)));
            }
            for (k, v) in obj {
                let text = match &v {
                    serde_json::Value::String(s) => s.clone(),
                    other => other.to_string(),
                };
                out.push(Line::from(Span::styled(
                    format!("  {k}"),
                    Style::default().fg(t.accent),
                )));
                push_wrapped(&mut out, &text, Style::default(), width);
            }
        }
    }
    out
}

fn render_permission_modal(frame: &mut Frame, area: Rect, app: &App) {
    let Some(p) = &app.pending_approval else {
        return;
    };
    let t = &app.theme;
    let width = area.width.clamp(30, 92);
    let body_w = width.saturating_sub(4); // borders + inner inset
    // Grow with the content instead of the old fixed 9 rows (three of which
    // were blank), but never taller than the terminal.
    let room = area.height.saturating_sub(APPROVAL_CHROME + 1).max(1) as usize;
    let mut body = approval_body(p, body_w, t);
    let mut shown = body.len().min(room);
    let hidden = body.len().saturating_sub(shown);
    if hidden > 0 {
        // Say what is missing instead of silently clipping the payload.
        shown = shown.saturating_sub(1).max(1);
        body.truncate(shown);
        body.push(Line::from(Span::styled(
            format!("  … {} more lines (widen the terminal)", hidden),
            Style::default().fg(t.muted),
        )));
    }
    let height = APPROVAL_CHROME + shown as u16;
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    let modal = Rect::new(x, y, width, height);
    frame.render_widget(Clear, modal);

    let mut lines = body;
    lines.push(Line::from(Span::raw("")));
    let mut keys = vec![
        Span::styled(
            " y ",
            Style::default()
                .fg(Color::Black)
                .bg(t.success)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" once  ", Style::default().fg(t.muted)),
        Span::styled(
            " a ",
            Style::default()
                .fg(Color::Black)
                .bg(t.accent)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" always  ", Style::default().fg(t.muted)),
        Span::styled(
            " n ",
            Style::default()
                .fg(Color::Black)
                .bg(t.error)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" deny", Style::default().fg(t.muted)),
    ];
    // Esc also denies — worth saying when the row has room for it.
    if body_w >= 46 {
        keys.push(Span::styled(
            "  ·  Esc denies",
            Style::default().fg(t.muted),
        ));
    }
    lines.push(Line::from(keys));
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(ratatui::widgets::BorderType::Rounded)
        .title(" permission ")
        .border_style(Style::default().fg(t.warning));
    frame.render_widget(block, modal);
    // Same 1-column inset as the input panel so text never hugs the border.
    let padded = Rect::new(
        modal.x.saturating_add(2),
        modal.y.saturating_add(1),
        modal.width.saturating_sub(3),
        modal.height.saturating_sub(2),
    );
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), padded);
}
