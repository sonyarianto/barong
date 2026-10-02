use crate::app::App;
use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use std::sync::atomic::Ordering;

pub struct InputState {
    pub buffer: String,
    pub cursor_pos: usize,
    pub focused: bool,
    pub history: Vec<String>,
    pub history_index: Option<usize>,
    pub saved_buffer: String,
}

impl InputState {
    pub fn new() -> Self {
        Self {
            buffer: String::new(),
            cursor_pos: 0,
            focused: true,
            history: Vec::new(),
            history_index: None,
            saved_buffer: String::new(),
        }
    }

    pub fn push_history(&mut self, input: String) {
        if self.history.last().is_some_and(|l| l == &input) {
            self.history_index = None;
            return;
        }
        self.history.push(input);
        if self.history.len() > 200 {
            self.history.remove(0);
        }
        self.history_index = None;
    }

    fn navigate_history(&mut self, direction: isize) {
        if self.history.is_empty() {
            return;
        }
        match self.history_index {
            None => {
                self.saved_buffer = self.buffer.clone();
                if direction < 0 {
                    self.history_index = Some(self.history.len() - 1);
                } else {
                    self.history_index = Some(0);
                }
            }
            Some(idx) => {
                let new_idx = if direction < 0 {
                    if idx == 0 {
                        self.history_index = None;
                        self.buffer = std::mem::take(&mut self.saved_buffer);
                        self.cursor_pos = self.buffer.len();
                        return;
                    }
                    idx - 1
                } else {
                    if idx >= self.history.len() - 1 {
                        self.history_index = None;
                        self.buffer = std::mem::take(&mut self.saved_buffer);
                        self.cursor_pos = self.buffer.len();
                        return;
                    }
                    idx + 1
                };
                self.history_index = Some(new_idx);
            }
        }
        if let Some(idx) = self.history_index {
            self.buffer = self.history[idx].clone();
            self.cursor_pos = self.buffer.len();
        }
    }
}

/// Slash commands: (name, description)
pub fn all_commands() -> Vec<(&'static str, &'static str)> {
    vec![
        ("/new", "start a new session"),
        ("/resume", "switch session — /resume [id]"),
        ("/session", "show current session info"),
        ("/export", "export session — /export [path]"),
        ("/copy", "copy last assistant message"),
        ("/compact", "compact context — /compact [keep=20] | /compact auto on|off"),
        ("/clear", "clear screen (same as /new)"),
        ("/model", "show or set model — /model [name]"),
        ("/tools", "list available tools"),
        ("/allow", "allow tool — /allow <tool|all>"),
        ("/approve", "auto-approve mode — /approve on|off"),
        ("/theme", "switch theme — /theme [dark|light|barong]"),
        ("/tree", "toggle file tree panel"),
        ("/branch", "fork session — /branch [name]"),
        ("/log", "list session branches"),
        ("/reload", "reload workspace context"),
        ("/hotkeys", "show keyboard shortcuts"),
        ("/help", "show this help"),
        ("/quit", "quit"),
    ]
}

fn handle_permission_key(app: &mut App, code: crossterm::event::KeyCode) -> Result<()> {
    use crate::agent::permissions::Decision;
    use crossterm::event::KeyCode;
    match code {
        KeyCode::Char('y') | KeyCode::Char('Y') => {
            app.approve_pending(Decision::AllowOnce);
        }
        KeyCode::Char('a') | KeyCode::Char('A') => {
            app.approve_pending(Decision::AllowSession);
        }
        KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
            // Esc during approval denies (does not cancel whole agent).
            app.approve_pending(Decision::Deny);
        }
        _ => {
            app.notice = Some("y=allow once · a=always · n=deny".into());
        }
    }
    return Ok(());
}

pub fn command_palette(filter: &str) -> Vec<(&'static str, &'static str)> {
    let q = filter.trim().to_lowercase();
    let q = q.strip_prefix('/').unwrap_or(&q);
    all_commands()
        .into_iter()
        .filter(|(name, desc)| {
            q.is_empty() || name.contains(q) || desc.to_lowercase().contains(&q)
        })
        .collect()
}

fn complete_palette(app: &mut App) {
    let items = command_palette(&app.input.buffer);
    if items.is_empty() {
        return;
    }
    let idx = app.palette_idx % items.len();
    let (name, _) = items[idx];
    app.input.buffer = format!("{} ", name);
    app.input.cursor_pos = app.input.buffer.len();
    app.palette_idx = 0;
}

fn complete_mention(app: &mut App) {
    // find @partial before cursor
    let before: String = app.input.buffer.chars().take(app.input.cursor_pos).collect();
    let Some(at) = before.rfind('@') else { return };
    let partial = &before[at + 1..];
    if partial.contains(' ') || partial.contains('\n') {
        return;
    }
    let matches = find_files(&app.workspace.root, partial, 20);
    if matches.is_empty() {
        app.notice = Some("no file match".into());
        return;
    }
    if matches.len() == 1 {
        // insert remainder + space (byte-safe rebuild)
        let buf = app.input.buffer.clone();
        let pos = app.input.cursor_pos;
        // recompute byte index of cursor
        let mut byte_cursor = buf.len();
        let mut cpos = 0usize;
        for (b, _) in buf.char_indices() {
            if cpos == pos {
                byte_cursor = b;
                break;
            }
            cpos += 1;
        }
        let before = &buf[..byte_cursor.min(buf.len())];
        let after = &buf[byte_cursor.min(buf.len())..];
        let rest = matches[0].strip_prefix(partial).unwrap_or(&matches[0]);
        let new_buf = format!("{}{} {}", before, rest, after.trim_start());
        let new_cursor = before.len() + rest.len() + 1;
        app.input.buffer = new_buf;
        app.input.cursor_pos = new_cursor.min(app.input.buffer.len());
        return;
    }
    // multiple: show in notice + complete common prefix
    let common = common_prefix(&matches);
    if common.len() > partial.len() {
        let extra = &common[partial.len()..];
        app.input.buffer.insert_str(app.input.cursor_pos, extra);
        app.input.cursor_pos += extra.len();
    }
    app.notice = Some(format!("{} matches: {}", matches.len(), matches.iter().take(3).cloned().collect::<Vec<_>>().join(", ")));
}

fn find_files(root: &std::path::Path, partial: &str, cap: usize) -> Vec<String> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    let needle = partial.to_lowercase();
    while let Some(d) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&d) else { continue };
        for e in entries.flatten() {
            if out.len() >= cap {
                break;
            }
            let p = e.path();
            let name = e.file_name().to_string_lossy().to_string();
            if name.starts_with('.') || name == "target" || name == "node_modules" {
                if p.is_dir() {
                    continue;
                }
            }
            if p.is_dir() {
                stack.push(p);
                continue;
            }
            let rel = p.strip_prefix(root).map(|r| r.to_string_lossy().to_string()).unwrap_or(name.clone());
            if needle.is_empty() || rel.to_lowercase().contains(&needle) || name.to_lowercase().contains(&needle) {
                out.push(rel);
            }
        }
    }
    out.sort();
    out
}

fn common_prefix(items: &[String]) -> String {
    if items.is_empty() {
        return String::new();
    }
    let mut p = items[0].clone();
    for s in &items[1..] {
        while !s.starts_with(&p) && !p.is_empty() {
            p.pop();
        }
    }
    p
}

/// Expand @path mentions by inlining file contents (capped). Returns (expanded, Vec<missing>).
pub fn expand_mentions(text: &str, root: &std::path::Path) -> (String, Vec<String>) {
    let mut out = text.to_string();
    let mut attached = Vec::new();
    let mut missing = Vec::new();
    // find @tokens (no spaces)
    let mut tokens: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut in_at = false;
    for c in text.chars() {
        if c == '@' {
            in_at = true;
            cur.clear();
        } else if in_at {
            if c.is_whitespace() || c == ',' || c == ')' || c == ']' {
                if !cur.is_empty() {
                    tokens.push(cur.clone());
                }
                in_at = false;
            } else {
                cur.push(c);
            }
        }
    }
    if in_at && !cur.is_empty() {
        tokens.push(cur);
    }
    for t in tokens.iter().take(5) {
        let cand = root.join(t);
        let p = if cand.exists() { cand } else { std::path::PathBuf::from(t) };
        match std::fs::read_to_string(&p) {
            Ok(content) => {
                let capped = if content.len() > 8000 {
                    format!("{}…\n[truncated]", &content[..8000])
                } else {
                    content
                };
                attached.push(format!("\n\n<file path=\"{}\">\n{}\n</file>", t, capped));
            }
            Err(_) => missing.push(t.clone()),
        }
    }
    for a in attached {
        out.push_str(&a);
    }
    (out, missing)
}

pub fn handle_events(app: &mut App) -> Result<()> {
    if let Event::Key(key) = event::read()? {
        if key.kind != KeyEventKind::Press {
            return Ok(());
        }
        // Permission modal takes over all keys while pending.
        if app.pending_approval.is_some() {
            return handle_permission_key(app, key.code);
        }
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let alt = key.modifiers.contains(KeyModifiers::ALT);
        app.notice = None;

        // --- Ctrl combos first ---
        if ctrl {
            match key.code {
                KeyCode::Char('c') => {
                    if !app.input.buffer.is_empty() {
                        app.input.buffer.clear();
                        app.input.cursor_pos = 0;
                    } else {
                        app.should_quit = true;
                    }
                    return Ok(());
                }
                KeyCode::Char('d') => {
                    if app.input.buffer.is_empty() {
                        app.should_quit = true;
                    }
                    return Ok(());
                }
                KeyCode::Char('u') => {
                    app.input.buffer.drain(..app.input.cursor_pos.min(app.input.buffer.len()));
                    app.input.cursor_pos = 0;
                    return Ok(());
                }
                KeyCode::Char('k') => {
                    app.input.buffer.truncate(app.input.cursor_pos.min(app.input.buffer.len()));
                    return Ok(());
                }
                KeyCode::Char('w') => {
                    let end = app.input.cursor_pos.min(app.input.buffer.len());
                    let start = app.input.buffer[..end].rfind([' ', '\n']).map(|i| i + 1).unwrap_or(0);
                    app.input.buffer.drain(start..end);
                    app.input.cursor_pos = start;
                    return Ok(());
                }
                KeyCode::Char('a') => {
                    app.input.cursor_pos = 0;
                    return Ok(());
                }
                KeyCode::Char('e') => {
                    app.input.cursor_pos = app.input.buffer.len();
                    return Ok(());
                }
                KeyCode::Char('j') => {
                    app.input.buffer.insert(app.input.cursor_pos, '\n');
                    app.input.cursor_pos += 1;
                    return Ok(());
                }
                KeyCode::Char('o') | KeyCode::Char('O') => {
                    app.tool_expanded = !app.tool_expanded;
                    app.notice = Some(if app.tool_expanded { "tool output expanded" } else { "tool output collapsed" }.into());
                    return Ok(());
                }
                KeyCode::Char('t') | KeyCode::Char('T') => {
                    app.workspace = crate::workspace::WorkspaceContext::new();
                    app.tree_visible = !app.tree_visible;
                    app.notice = Some(if app.tree_visible { "tree panel on" } else { "tree panel off" }.into());
                    return Ok(());
                }
                _ => {}
            }
        }
        if alt {
            // Alt+Left/Right word jump (best-effort)
            match key.code {
                KeyCode::Left => {
                    let end = app.input.cursor_pos.min(app.input.buffer.len());
                    let prev = app.input.buffer[..end].rfind([' ', '\n']).map(|i| i + 1).unwrap_or(0);
                    app.input.cursor_pos = prev;
                    return Ok(());
                }
                KeyCode::Right => {
                    let rest = &app.input.buffer[app.input.cursor_pos.min(app.input.buffer.len())..];
                    if let Some(i) = rest.find([' ', '\n']) {
                        app.input.cursor_pos += i + 1;
                    } else {
                        app.input.cursor_pos = app.input.buffer.len();
                    }
                    return Ok(());
                }
                _ => {}
            }
        }

        match key.code {
            KeyCode::Char(c) => {
                app.input.buffer.insert(app.input.cursor_pos, c);
                app.input.cursor_pos += 1;
                app.palette_idx = 0;
            }
            KeyCode::Backspace => {
                if app.input.cursor_pos > 0 && !app.input.buffer.is_empty() {
                    // handle byte boundary safely
                    let mut idx = app.input.cursor_pos - 1;
                    while idx > 0 && !app.input.buffer.is_char_boundary(idx) {
                        idx -= 1;
                    }
                    if app.input.cursor_pos <= app.input.buffer.len() && app.input.buffer.is_char_boundary(app.input.cursor_pos) {
                        app.input.buffer.remove(idx);
                        app.input.cursor_pos = idx;
                    }
                }
                app.palette_idx = 0;
            }
            KeyCode::Delete => {
                if app.input.cursor_pos < app.input.buffer.len() {
                    app.input.buffer.remove(app.input.cursor_pos);
                }
            }
            KeyCode::Left => {
                app.input.cursor_pos = app.input.cursor_pos.saturating_sub(1);
            }
            KeyCode::Right => {
                if app.input.cursor_pos < app.input.buffer.len() {
                    app.input.cursor_pos += 1;
                }
            }
            KeyCode::Home => app.input.cursor_pos = 0,
            KeyCode::End => app.input.cursor_pos = app.input.buffer.len(),
            KeyCode::Up => {
                if app.input.buffer.starts_with('/') {
                    let items = command_palette(&app.input.buffer);
                    if !items.is_empty() {
                        app.palette_idx = (app.palette_idx + items.len() - 1) % items.len();
                        return Ok(());
                    }
                }
                app.input.navigate_history(-1);
            }
            KeyCode::Down => {
                if app.input.buffer.starts_with('/') {
                    let items = command_palette(&app.input.buffer);
                    if !items.is_empty() {
                        app.palette_idx = (app.palette_idx + 1) % items.len();
                        return Ok(());
                    }
                }
                app.input.navigate_history(1);
            }
            KeyCode::Tab => {
                if app.input.buffer.starts_with('/') {
                    complete_palette(app);
                } else if app.input.buffer.contains('@') {
                    complete_mention(app);
                } else {
                    app.input.buffer.insert_str(app.input.cursor_pos, "  ");
                    app.input.cursor_pos += 2;
                }
            }
            KeyCode::Enter => {
                // first Enter on bare `/xx` completes palette instead of submitting
                if app.input.buffer.starts_with('/') && !app.input.buffer.contains(' ') && !app.input.buffer.contains('\n') {
                    let items = command_palette(&app.input.buffer);
                    if items.len() == 1 || (app.palette_idx == 0 && !items.is_empty() && format!("/{}", app.input.buffer.trim_start_matches('/').split_whitespace().next().unwrap_or("")) != items[0].0) {
                        // only auto-complete when buffer is a strict prefix, not an exact command
                        let exact = items.iter().any(|(n, _)| format!("{} ", n) == format!("{} ", app.input.buffer.trim()));
                        if !exact {
                            complete_palette(app);
                            return Ok(());
                        }
                    }
                }
                let input = std::mem::take(&mut app.input.buffer);
                app.input.cursor_pos = 0;
                app.input.history_index = None;
                app.palette_idx = 0;
                if input.trim().is_empty() || app.event_rx.is_some() {
                    return Ok(());
                }
                if input.trim_start().starts_with('/') {
                    if handle_slash(app, input.trim())? {
                        return Ok(());
                    }
                }
                // @file expansion (context attach)
                let (expanded, missing) = expand_mentions(&input, &app.workspace.root);
                if !missing.is_empty() {
                    app.notice = Some(format!("@ not found: {}", missing.join(", ")));
                }
                app.input.push_history(input.clone());
                app.conversation.add_message("user".into(), input);
                // stash expanded for the agent without polluting transcript:
                // temporarily push expanded as last message clone for LLM only
                // -> implement by replacing last message content during start_agent
                app.is_home = false;
                app.save_session();
                app.should_auto_scroll = true;
                app.chat_scroll = 0;
                start_agent_with_expanded(app, expanded);
            }
            KeyCode::Esc => {
                if app.event_rx.is_some() {
                    app.cancelled.store(true, Ordering::Relaxed);
                    app.event_rx = None;
                    app.save_session();
                    app.streaming_text.clear();
                    app.status.tool_status = "cancelled".into();
                } else if app.input.buffer.starts_with('/') {
                    app.input.buffer.clear();
                    app.input.cursor_pos = 0;
                }
            }
            KeyCode::PageUp => {
                let h = 10usize;
                app.chat_scroll = app.chat_scroll.saturating_sub(h);
                app.should_auto_scroll = false;
            }
            KeyCode::PageDown => {
                app.chat_scroll = app.chat_scroll.saturating_add(10);
                // if near bottom, re-follow
                app.should_auto_scroll = false;
            }
            _ => {}
        }
    }
    return Ok(());
}

fn start_agent_with_expanded(app: &mut App, expanded: String) {
    // If @-expansion added file blocks, swap last user message content for the LLM run,
    // then restore display text afterwards? Minimal: keep expanded in transcript (transparent).
    if expanded != app.conversation.messages.last().map(|m| m.content.clone().unwrap_or_default()).unwrap_or_default() {
        if let Some(last) = app.conversation.messages.last_mut() {
            last.content = Some(expanded);
        }
    }
    app.start_agent();
}

/// Returns true if handled as command (no agent run).
fn handle_slash(app: &mut App, trimmed: &str) -> Result<bool> {
    let mut parts = trimmed.splitn(2, char::is_whitespace);
    let cmd = parts.next().unwrap_or("");
    let arg = parts.next().unwrap_or("").trim();
    match cmd {
        "/quit" | "/exit" | "/q" => {
            app.should_quit = true;
            return Ok(true);
        }
        "/clear" | "/new" => {
            app.conversation = crate::agent::conversation::Conversation::new();
            app.session_id = None;
            app.streaming_text.clear();
            app.chat_scroll = 0;
            app.should_auto_scroll = true;
            app.is_home = true;
            app.status.tool_status = "idle".into();
            return Ok(true);
        }
        "/model" => {
            if arg.is_empty() {
                app.conversation.add_message("assistant".into(), format!("**Model:** `{}`\nUsage: `/model <name>`", app.current_model));
            } else {
                app.current_model = arg.to_string();
                app.status.model = app.current_model.clone();
                app.conversation.add_message("assistant".into(), format!("**Model set to:** `{}`", app.current_model));
            }
            app.save_session();
            app.is_home = false;
            return Ok(true);
        }
        "/tools" => {
            let names = app.tool_registry.tool_names().join(", ");
            app.conversation.add_message("assistant".into(), format!("**Tools:** {}", names));
            app.save_session();
            app.is_home = false;
            return Ok(true);
        }
        "/allow" => {
            if arg.is_empty() {
                let allowed: Vec<String> = ["write", "edit", "bash", "delegate"]
                    .iter()
                    .filter(|t| app.permission_gate.is_session_allowed(t))
                    .map(|s| s.to_string())
                    .collect();
                let auto = app.permission_gate.auto_approve();
                app.conversation.add_message("assistant".into(), format!("**Permissions:** auto_approve={}\nsession-allowed: {}\nUsage: `/allow <write|edit|bash|delegate|all>`", auto, if allowed.is_empty() { "(none)".into() } else { allowed.join(", ") }));
            } else {
                let targets: Vec<String> = if arg.eq_ignore_ascii_case("all") {
                    vec!["write".into(), "edit".into(), "bash".into(), "delegate".into()]
                } else {
                    vec![arg.to_string()]
                };
                for t in targets {
                    app.permission_gate.allow_session(&t);
                }
                app.conversation.add_message("assistant".into(), format!("Allowed `{}` for this session.", arg));
            }
            app.save_session();
            app.is_home = false;
            return Ok(true);
        }
        "/approve" => {
            if arg.eq_ignore_ascii_case("on") || arg == "1" || arg.eq_ignore_ascii_case("yes") {
                app.permission_gate.set_auto_approve(true);
                app.conversation.add_message("assistant".into(), "Auto-approve **on** — mutating tools run without asking.".into());
            } else if arg.eq_ignore_ascii_case("off") || arg == "0" || arg.eq_ignore_ascii_case("no") {
                app.permission_gate.set_auto_approve(false);
                app.conversation.add_message("assistant".into(), "Auto-approve **off** — will ask for write/edit/bash.".into());
            } else {
                let auto = app.permission_gate.auto_approve();
                app.conversation.add_message("assistant".into(), format!("**Auto-approve:** `{}`\nUsage: `/approve on|off`", auto));
            }
            app.save_session();
            app.is_home = false;
            return Ok(true);
        }
        "/session" => {
            let info = format!(
                "**Session:** `{}`\n- msgs: {}\n- model: `{}`\n- cwd: `{}`\n- ctx: ~{}%\n- auto-compact: {} (keep={})\n- theme: `{}`\n- tree: {}",
                app.session_id.as_deref().unwrap_or("(unsaved)"),
                app.conversation.messages.len(),
                app.current_model,
                app.workspace.root.display(),
                (app.context_usage() * 100.0) as u32,
                app.config.resolve_auto_compact(),
                app.config.resolve_compact_keep(),
                app.theme.name,
                if app.tree_visible { "on" } else { "off" },
            );
            app.conversation.add_message("assistant".into(), info);
            app.save_session();
            app.is_home = false;
            return Ok(true);
        }
        "/resume" => {
            if arg.is_empty() {
                let mut sessions = app.session_manager.list_sessions();
                sessions.sort_by(|a, b| b.updated.cmp(&a.updated));
                if sessions.is_empty() {
                    app.conversation.add_message("assistant".into(), "No saved sessions.".into());
                } else {
                    let mut out = String::from("**Recent sessions:**\n");
                    for s in sessions.iter().take(5) {
                        out.push_str(&format!("- `{}` — {} msgs\n", s.id, s.messages.len()));
                    }
                    out.push_str("\nUse `/resume <id>`");
                    app.conversation.add_message("assistant".into(), out);
                }
            } else if let Some(s) = app.session_manager.load(arg) {
                app.conversation.messages = s.messages;
                app.session_id = Some(s.id.clone());
                app.is_home = false;
                app.streaming_text.clear();
                app.conversation.add_message("assistant".into(), format!("Resumed `{}`", s.id));
            } else {
                app.conversation.add_message("assistant".into(), format!("Session not found: `{}`", arg));
            }
            app.save_session();
            app.is_home = false;
            return Ok(true);
        }
        "/export" => {
            let dest = if arg.is_empty() {
                format!("barong-session-{}.jsonl", app.session_id.as_deref().unwrap_or("export"))
            } else {
                arg.to_string()
            };
            match app.session_id.clone() {
                Some(id) => match app.session_manager.export_jsonl(&id, std::path::Path::new(&dest)) {
                    Ok(_) => app.conversation.add_message("assistant".into(), format!("Exported to `{}`", dest)),
                    Err(e) => app.conversation.add_message("assistant".into(), format!("Export failed: {}", e)),
                },
                None => app.conversation.add_message("assistant".into(), "Nothing to export yet.".into()),
            }
            app.save_session();
            app.is_home = false;
            return Ok(true);
        }
        "/copy" => {
            let last = app.conversation.messages.iter().rev().find(|m| m.role == "assistant").and_then(|m| m.content.clone()).unwrap_or_default();
            if last.is_empty() {
                app.notice = Some("nothing to copy".into());
            } else if copy_to_clipboard(&last) {
                app.notice = Some("copied last response".into());
            } else {
                app.conversation.add_message("assistant".into(), format!("```\n{}\n```", last.chars().take(2000).collect::<String>()));
            }
            app.is_home = false;
            return Ok(true);
        }
        "/compact" => {
            let mut parts = arg.split_whitespace();
            let first = parts.next().unwrap_or("");
            if first.eq_ignore_ascii_case("auto") {
                let mode = parts.next().unwrap_or("").to_lowercase();
                if ["on", "1", "yes", "true"].contains(&mode.as_str()) {
                    app.config.auto_compact = Some(true);
                    app.conversation.add_message("assistant".into(), "Auto-compact **on** (triggers at ctx ≥85% or >100 msgs).".into());
                } else if ["off", "0", "no", "false"].contains(&mode.as_str()) {
                    app.config.auto_compact = Some(false);
                    app.conversation.add_message("assistant".into(), "Auto-compact **off**.".into());
                } else {
                    let on = app.config.resolve_auto_compact();
                    let keep = app.config.resolve_compact_keep();
                    app.conversation.add_message("assistant".into(), format!("**Auto-compact:** `{}` (keep={}, threshold=85%)\nUsage: `/compact auto on|off` or `/compact [keep]`", on, keep));
                }
            } else {
                let keep: usize = first.parse().unwrap_or_else(|_| app.config.resolve_compact_keep());
                match app.conversation.compact(keep) {
                    Some((_, report)) => app.conversation.add_message("assistant".into(), report),
                    None => app.conversation.add_message("assistant".into(), format!("Nothing to compact ({} msgs)", app.conversation.messages.len())),
                }
            }
            app.save_session();
            app.is_home = false;
            return Ok(true);
        }
        "/reload" => {
            app.workspace = crate::workspace::WorkspaceContext::new();
            app.notice = Some("workspace reloaded".into());
            app.is_home = false;
            return Ok(true);
        }
        "/theme" => {
            if arg.is_empty() {
                let mut out = String::from("**Themes:**\n");
                for (n, d) in crate::tui::theme::all_themes() {
                    let cur = if n == app.theme.name { " (current)" } else { "" };
                    out.push_str(&format!("- `{}` — {}{}\n", n, d, cur));
                }
                out.push_str("\nUse `/theme <name>`");
                app.conversation.add_message("assistant".into(), out);
            } else {
                let names: Vec<&str> = crate::tui::theme::all_themes().iter().map(|(n, _)| *n).collect();
                if names.contains(&arg.to_lowercase().as_str()) {
                    app.set_theme(&arg);
                    app.conversation.add_message("assistant".into(), format!("Theme set to `{}`", app.theme.name));
                } else {
                    app.conversation.add_message("assistant".into(), format!("Unknown theme `{}`. Try `/theme`.", arg));
                }
            }
            app.save_session();
            app.is_home = false;
            return Ok(true);
        }
        "/tree" => {
            app.workspace = crate::workspace::WorkspaceContext::new();
            app.tree_visible = !app.tree_visible;
            app.notice = Some(if app.tree_visible { "tree panel on" } else { "tree panel off" }.into());
            app.is_home = false;
            return Ok(true);
        }
        "/branch" => {
            let name = if arg.is_empty() { "branch" } else { arg };
            let parent = app.session_id.clone();
            let id = app.session_manager.fork(&app.conversation.messages, parent.as_deref(), name);
            app.session_id = Some(id.clone());
            app.conversation.add_message("assistant".into(), format!("Branched `{}` from `{}`", id, parent.as_deref().unwrap_or("(unsaved)")));
            app.save_session();
            app.is_home = false;
            return Ok(true);
        }
        "/log" => {
            let mut sessions = app.session_manager.list_sessions();
            sessions.sort_by(|a, b| b.updated.cmp(&a.updated));
            if sessions.is_empty() {
                app.conversation.add_message("assistant".into(), "No saved sessions.".into());
            } else {
                let mut out = String::from("**Sessions:**\n");
                for s in sessions.iter().take(10) {
                    let cur = if Some(&s.id) == app.session_id.as_ref() { " ← current" } else { "" };
                    let branch = s.branch.as_deref().unwrap_or("-");
                    let parent = s.parent_id.as_deref().unwrap_or("-");
                    out.push_str(&format!("- `{}` — {} msgs · branch `{}` · parent `{}`{}\n", s.id, s.messages.len(), branch, parent, cur));
                }
                out.push_str("\nUse `/resume <id>` or `/branch [name]`");
                app.conversation.add_message("assistant".into(), out);
            }
            app.save_session();
            app.is_home = false;
            return Ok(true);
        }
        "/hotkeys" => {
            app.conversation.add_message("assistant".into(), "**Keys:**\n- `Enter` send · `Shift+Enter/Ctrl+J` newline\n- `Tab` complete `/` or `@` · `Up/Down` palette/history\n- `Ctrl+C` clear/quit · `Ctrl+D` quit · `Ctrl+U/K/W` edit · `Ctrl+A/E` jump\n- `Ctrl+O` expand tools · `Ctrl+T` tree panel · `Esc` cancel · `PgUp/PgDn` scroll\n- approval modal: `y` once · `a` always · `n`/`Esc` deny".into());
            app.save_session();
            app.is_home = false;
            return Ok(true);
        }
        "/help" => {
            let mut out = String::from("**Commands:**\n");
            for (n, d) in all_commands() {
                out.push_str(&format!("- `{}` — {}\n", n, d));
            }
            out.push_str("\n**Tips:** `@path` attaches files · `Ctrl+O` toggles tool output");
            app.conversation.add_message("assistant".into(), out);
            app.save_session();
            app.is_home = false;
            return Ok(true);
        }
        _ => {
            app.conversation.add_message("assistant".into(), format!("Unknown command: `{}`. Try `/help`.", cmd));
            app.save_session();
            app.is_home = false;
            return Ok(true);
        }
    }
}

fn copy_to_clipboard(text: &str) -> bool {
    for (cmd, args) in [
        ("wl-copy", vec![]),
        ("xclip", vec!["-selection", "clipboard"]),
        ("pbcopy", vec![]),
    ] {
        if let Ok(mut child) = std::process::Command::new(cmd).args(&args).stdin(std::process::Stdio::piped()).spawn() {
            if let Some(stdin) = child.stdin.take() {
                use std::io::Write;
                let mut stdin = stdin;
                let _ = stdin.write_all(text.as_bytes());
            }
            if child.wait().map(|s| s.success()).unwrap_or(false) {
                return true;
            }
        }
    }
    false
}
