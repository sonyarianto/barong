use crate::app::App;
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Wrap};

pub fn render(frame: &mut Frame, app: &mut App) {
    let area = frame.area();
    // Bottom chrome, OpenCode-style: `┃` prompt rows, a `╹▀` closing rule and
    // one status line. Three rows when idle instead of five — the frame is the
    // padding, so there are no blank filler rows.
    let box_rows = prompt_box_rows(app, area.width);
    let notice_rows = u16::from(app.notice.is_some());
    let chrome = notice_rows + box_rows + 1; // status
    let chat_rows = area.height.saturating_sub(chrome).max(1);
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(chat_rows),
            Constraint::Length(notice_rows),
            Constraint::Length(box_rows),
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
    render_notice(frame, chunks[1], app);
    render_prompt(frame, chunks[2], app);
    // One picker overlay for every mode (same chrome, same keys).
    let idle =
        app.event_rx.is_none() && app.pending_approval.is_none() && app.pending_login.is_none();
    if idle {
        if let Some((kind, filter)) = crate::tui::input::picker_kind(&app.input.buffer) {
            let rows = crate::tui::input::picker_rows(app, &kind, &filter);
            if !rows.is_empty() {
                let title = crate::tui::input::picker_title(&kind);
                render_picker(frame, chunks[0], chunks[2], app, title, &rows);
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
                render_picker(frame, chunks[0], chunks[2], app, "", &rows);
            }
        }
    }
    render_status(frame, chunks[3], app);
    if app.pending_approval.is_some() {
        render_permission_modal(frame, area, app);
    }
}

/// OpenCode-style input footer: active `provider/model` at the typing point
/// (status bar no longer duplicates it) + approval state + `@file` chips.
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

/// Longest the prompt box grows before it starts scrolling.
const MAX_PROMPT_ROWS: u16 = 8;
/// `┃` + one space of inset on each side.
const PROMPT_INSET: u16 = 3;
const PROMPT_PLACEHOLDER: &str = "message · / commands · @ files";

/// The row inside the box that names the endpoint, mirroring OpenCode's
/// `agent · model · variant` line.
struct EndpointLine {
    provider: String,
    model: String,
    mode: String,
    mode_style: Style,
}

impl EndpointLine {
    fn build(app: &App) -> Self {
        let t = &app.theme;
        let (mode, mode_style) = if app.permission_gate.auto_approve() {
            ("auto".to_string(), Style::default().fg(t.success))
        } else {
            ("ask".to_string(), Style::default().fg(t.warning))
        };
        Self {
            provider: app.provider_name.clone(),
            model: app.current_model.clone(),
            mode,
            mode_style,
        }
    }

    /// One row that degrades by dropping whole segments rather than clipping
    /// mid-word: `provider · model · mode` → `model · mode` → `model`.
    fn spans(&self, width: u16, t: &crate::tui::theme::Theme) -> Vec<Span<'static>> {
        let dot = Style::default().fg(t.muted);
        let model = self.model.clone();
        let mut segs: Vec<(String, Style)> = vec![
            (self.provider.clone(), Style::default().fg(t.accent)),
            (model.clone(), Style::default()),
            (self.mode.clone(), self.mode_style),
        ];
        // Keep the last segments first, then prepend while there is room.
        let w = width as usize;
        loop {
            let total: usize =
                segs.iter().map(|(t2, _)| display_cells(t2)).sum::<usize>() + 3 * (segs.len() - 1);
            if total <= w || segs.len() <= 1 {
                break;
            }
            segs.remove(0);
        }
        // Whatever is left may still need shortening.
        let total: usize =
            segs.iter().map(|(t2, _)| display_cells(t2)).sum::<usize>() + 3 * (segs.len() - 1);
        if total > w && segs.len() == 1 {
            segs[0].0 = truncate_middle(&segs[0].0, w);
        }
        let mut out: Vec<Span<'static>> = Vec::new();
        for (i, (text, style)) in segs.into_iter().enumerate() {
            if i > 0 {
                out.push(Span::styled(" · ", dot));
            }
            out.push(Span::styled(text, style));
        }
        out
    }
}

/// Busy indicator: a single segment sweeping across dim cells. Indeterminate on
/// purpose — we have no completion percentage, and a bar that fills to 100%
/// would be a lie.
fn progress_sweep<'a>(tick: usize, t: &'a crate::tui::theme::Theme) -> Vec<Span<'a>> {
    const CELLS: usize = 8;
    // Indeterminate: one accent head sweeping across dim dots, pausing a beat
    // at each end. A bar that filled to 100% would imply progress we don't have.
    let pos = tick % (CELLS + 2);
    let head = if pos < CELLS { Some(pos) } else { None };
    let mut out: Vec<Span<'a>> = Vec::new();
    match head {
        Some(h) => {
            if h > 0 {
                out.push(Span::styled("·".repeat(h), Style::default().fg(t.muted)));
            }
            out.push(Span::styled("▪", Style::default().fg(t.accent)));
            let rest = CELLS - h - 1;
            if rest > 0 {
                out.push(Span::styled("·".repeat(rest), Style::default().fg(t.muted)));
            }
        }
        None => out.push(Span::styled(
            "·".repeat(CELLS),
            Style::default().fg(t.muted),
        )),
    }
    out
}

/// Truncate from the middle so both ends of a long model id stay readable.
fn truncate_middle(s: &str, max: usize) -> String {
    if display_cells(s) <= max {
        return s.to_string();
    }
    let keep = max.saturating_sub(1);
    let head = keep / 2;
    let tail = keep - head;
    let head: String = s.chars().take(head).collect();
    let tail: String = {
        let all: Vec<char> = s.chars().collect();
        if tail == 0 {
            String::new()
        } else {
            all[all.len() - tail..].iter().collect()
        }
    };
    format!("{}…{}", head, tail)
}

/// Rows the prompt box needs: the wrapped draft plus a blank breathing row and
/// the endpoint row.
fn prompt_box_rows(app: &App, area_width: u16) -> u16 {
    let inner = area_width.saturating_sub(PROMPT_INSET).max(8);
    let draft = crate::tui::wrap::text_rows(&prompt_draft_text(app), inner) as u16;
    draft.saturating_add(2).clamp(3, MAX_PROMPT_ROWS)
}

/// What the prompt line shows when there is nothing typed.
fn prompt_draft_text(app: &App) -> String {
    if let Some(pid) = &app.pending_login {
        let masked = "•".repeat(app.login_buffer.chars().count().min(48));
        return format!("key for {}: {}", pid, masked);
    }
    if app.input.buffer.is_empty() && !app.input.buffer.starts_with('/') {
        return PROMPT_PLACEHOLDER.to_string();
    }
    let cursor = app.input.cursor();
    let mut display = app.input.buffer.clone();
    display.insert(cursor, '█');
    display
}

/// Transient warning line above the prompt. Notices used to sit in the status
/// bar, where a long message pushed everything else off screen.
fn render_notice(frame: &mut Frame, area: Rect, app: &App) {
    let Some(text) = &app.notice else { return };
    if area.height == 0 || area.width == 0 {
        return;
    }
    let t = &app.theme;
    let line = Line::from(vec![
        Span::styled("▎ ", Style::default().fg(t.warning)),
        Span::styled(
            truncate(text, area.width.saturating_sub(3) as usize),
            Style::default().fg(t.warning),
        ),
    ]);
    frame.render_widget(Paragraph::new(line), area);
}

/// The prompt box: thick accent bar, panel background, draft on top and the
/// endpoint row at the bottom (OpenCode's shape).
fn render_prompt(frame: &mut Frame, area: Rect, app: &App) {
    if area.height == 0 || area.width == 0 {
        return;
    }
    let t = &app.theme;
    let inner_w = area.width.saturating_sub(PROMPT_INSET).max(1);
    let draft_text = prompt_draft_text(app);
    let rows = crate::tui::wrap::wrap_text(&draft_text, inner_w);
    let height = area.height;
    let endpoint = EndpointLine::build(app);

    // Keep the cursor on the last visible row when the draft outgrows the box.
    let cursor_row = prompt_cursor_row(app, inner_w);
    let scroll = cursor_row
        .saturating_sub(height.saturating_sub(3))
        .min((rows.len() as u16).saturating_sub(1));

    let is_placeholder = app.input.buffer.is_empty() && app.pending_login.is_none();
    let draft_style = Style::default().bg(t.panel).fg(if is_placeholder {
        t.muted
    } else {
        Color::Reset
    });
    let bar = Style::default().fg(t.accent).bg(t.panel);

    for row in 0..height {
        // Last two rows are the breathing row and the endpoint row.
        let text = if row + 2 >= height {
            String::new()
        } else {
            rows.get((scroll + row) as usize)
                .cloned()
                .unwrap_or_default()
        };
        let mut spans = vec![
            Span::styled("\u{258c} ", bar),
            Span::styled(text, draft_style),
        ];
        if row + 1 == height {
            // Endpoint row lives inside the box, hard left.
            let mut line = endpoint.spans(area.width.saturating_sub(PROMPT_INSET), t);
            let used: usize = 2 + line
                .iter()
                .map(|s| display_cells(&s.content))
                .sum::<usize>();
            spans.append(&mut line);
            spans.push(Span::styled(
                " ".repeat((area.width as usize).saturating_sub(used)),
                Style::default().bg(t.panel),
            ));
        } else {
            let used = 2 + display_cells(&spans[1].content) as usize;
            spans.push(Span::styled(
                " ".repeat((area.width as usize).saturating_sub(used)),
                Style::default().bg(t.panel),
            ));
        }
        frame.render_widget(
            Paragraph::new(Line::from(spans)),
            Rect::new(area.x, area.y + row, area.width, 1),
        );
    }
}

/// Row index (within the wrapped prompt rows) that the block cursor sits on.
fn prompt_cursor_row(app: &App, inner: u16) -> u16 {
    if app.pending_login.is_some() {
        return 0;
    }
    if app.input.buffer.is_empty() {
        return 0;
    }
    let cursor = app.input.cursor();
    let display = {
        let mut d = app.input.buffer.clone();
        d.insert(cursor, '█');
        d
    };
    let head = &display[..floor_boundary(&display, cursor + '█'.len_utf8())];
    let mut row = 0u16;
    let mut rest = head;
    while let Some(idx) = rest.find('\n') {
        row += crate::tui::wrap::text_rows(&rest[..idx], inner) as u16;
        rest = &rest[idx + 1..];
    }
    row + crate::tui::wrap::text_rows(rest, inner).saturating_sub(1) as u16
}

/// Largest char boundary at or below `at`.
fn floor_boundary(s: &str, at: usize) -> usize {
    let mut i = at.min(s.len());
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

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
            let right_w = display_cells(&row.right);
            // Two columns only when both get a readable run. Otherwise merge
            // them: a model list that cannot tell its entries apart is useless.
            if right_w > 0 && avail >= 12 && avail - 12 >= 8 {
                spans.push(Span::styled(
                    format!(" {}", truncate(&row.left, 12)),
                    label_style,
                ));
                spans.push(Span::styled(
                    format!(" {}", truncate(&row.right, avail - 13)),
                    detail_style,
                ));
            } else {
                let merged = if row.right.is_empty() {
                    row.left.clone()
                } else if row.left.is_empty() {
                    row.right.clone()
                } else {
                    format!("{} {}", row.left, row.right)
                };
                spans.push(Span::styled(
                    format!(" {}", truncate(&merged, avail.max(1))),
                    label_style,
                ));
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

/// `1.2k` / `18.1k` / `1.4M` — compact, like OpenCode's context readout.
fn format_tokens(n: usize) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f32 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.1}k", n as f32 / 1_000.0)
    } else {
        format!("{}", n)
    }
}

/// One row under the box. While the agent works the left side becomes the
/// activity sweep plus `esc interrupt`; otherwise it is `path · branch`.
/// Right side is `msgs · tokens (pct)`.
fn render_status(frame: &mut Frame, area: Rect, app: &mut App) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let t = &app.theme;
    let dim = Style::default().fg(t.muted);

    let mut left: Vec<Span> = vec![Span::styled(" ", dim)];
    if app.event_rx.is_some() {
        app.spinner_tick = app.spinner_tick.wrapping_add(1);
        left.extend(progress_sweep(app.spinner_tick, t));
        left.push(Span::styled(" esc interrupt", Style::default()));
        if !app.queued_input.is_empty() {
            left.push(Span::styled(
                format!("  +{} queued", app.queued_input.len()),
                Style::default().fg(t.accent),
            ));
        }
    } else {
        let mut path = app.cwd_short();
        if app.workspace.is_git_repo && !app.workspace.git_branch.is_empty() {
            path.push_str(&format!(" · {}", app.workspace.git_branch));
        }
        left.push(Span::styled(path, dim));
    }

    let msgs = app.conversation.messages.len();
    let ctx_pct = (app.context_usage() * 100.0) as u32;
    let ctx_color = if ctx_pct > 85 {
        t.error
    } else if ctx_pct > 60 {
        t.warning
    } else {
        t.muted
    };
    let tokens = if app.context_is_estimated() {
        format!("~{}", format_tokens(app.estimated_prompt_tokens()))
    } else {
        format_tokens(app.last_prompt_tokens.unwrap_or(0))
    };
    let right = format!("{} msgs · {} ({}%)", msgs, tokens, ctx_pct);

    let left_w: u16 = left.iter().map(|s| display_cells(&s.content) as u16).sum();
    let right_w = display_cells(&right) as u16;
    let mut spans = left;
    if left_w + right_w + 2 <= area.width {
        spans.push(Span::styled(
            " ".repeat((area.width - left_w - right_w) as usize),
            dim,
        ));
    }
    spans.push(Span::styled(right, Style::default().fg(ctx_color)));
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

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

/// Rows of chrome around the body: top border, blank, key row, bottom border.
const APPROVAL_CHROME: u16 = 4;
/// Payload lines shown before the modal admits what it hid.
const APPROVAL_MAX_LINES: usize = 24;

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
