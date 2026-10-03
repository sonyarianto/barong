use crate::app::{App, NoticeCause};
use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use std::sync::atomic::Ordering;

pub struct InputState {
    pub buffer: String,
    /// Byte offset into `buffer`, always on a char boundary. Never index
    /// `buffer` with it directly — go through the cursor methods below, they
    /// snap to a boundary first (a raw char count panics on multi-byte UTF-8).
    pub cursor_pos: usize,
    pub focused: bool,
    pub history: Vec<String>,
    pub history_index: Option<usize>,
    pub saved_buffer: String,
}

/// Largest char boundary at or below `at` (idempotent).
fn floor_boundary(s: &str, at: usize) -> usize {
    let at = at.min(s.len());
    if s.is_char_boundary(at) {
        return at;
    }
    prev_boundary(s, at)
}

/// Largest char boundary strictly below `at` — i.e. where the char ending at
/// `at` starts. Distinct from `floor_boundary`, which keeps `at` itself.
fn prev_boundary(s: &str, at: usize) -> usize {
    let mut i = at.min(s.len());
    while i > 0 {
        i -= 1;
        if s.is_char_boundary(i) {
            return i;
        }
    }
    0
}

/// Smallest char boundary strictly above `at`.
fn next_boundary(s: &str, at: usize) -> usize {
    let mut i = at.min(s.len());
    while i < s.len() {
        i += 1;
        if s.is_char_boundary(i) {
            return i;
        }
    }
    s.len()
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

    /// `cursor_pos` clamped into the buffer and onto a char boundary. Safe to
    /// feed straight into slicing or `String::insert`.
    pub fn cursor(&self) -> usize {
        floor_boundary(&self.buffer, self.cursor_pos)
    }

    pub fn set_cursor(&mut self, byte: usize) {
        self.cursor_pos = byte;
        self.cursor_pos = self.cursor();
    }

    pub fn move_cursor_home(&mut self) {
        self.cursor_pos = 0;
    }

    pub fn move_cursor_end(&mut self) {
        self.cursor_pos = self.buffer.len();
    }

    pub fn insert_char(&mut self, c: char) {
        let mut utf8 = [0u8; 4];
        self.insert_str(c.encode_utf8(&mut utf8));
    }

    pub fn insert_str(&mut self, s: &str) {
        if s.is_empty() {
            return;
        }
        let at = self.cursor();
        self.buffer.insert_str(at, s);
        self.cursor_pos = at + s.len();
    }

    pub fn insert_newline(&mut self) {
        self.insert_char('\n');
    }

    /// Delete the char before the cursor.
    pub fn backspace(&mut self) {
        let at = self.cursor();
        if at == 0 {
            return;
        }
        let start = prev_boundary(&self.buffer, at);
        self.buffer.replace_range(start..at, "");
        self.cursor_pos = start;
    }

    /// Delete the char under the cursor.
    pub fn delete(&mut self) {
        let at = self.cursor();
        let end = next_boundary(&self.buffer, at);
        if end > at {
            self.buffer.replace_range(at..end, "");
            self.cursor_pos = at;
        }
    }

    pub fn move_left(&mut self) {
        self.cursor_pos = prev_boundary(&self.buffer, self.cursor());
    }

    pub fn move_right(&mut self) {
        self.cursor_pos = next_boundary(&self.buffer, self.cursor());
    }

    /// Ctrl+U: drop everything before the cursor.
    pub fn delete_before_cursor(&mut self) {
        let at = self.cursor();
        self.buffer.replace_range(..at, "");
        self.cursor_pos = 0;
    }

    /// Ctrl+K: drop everything from the cursor on.
    pub fn delete_to_cursor(&mut self) {
        let at = self.cursor();
        self.buffer.truncate(at);
    }

    /// Ctrl+W: drop the word before the cursor.
    pub fn delete_word_before(&mut self) {
        let at = self.cursor();
        let head = &self.buffer[..at];
        let trimmed = head.trim_end();
        let start = trimmed.rfind([' ', '\n']).map(|i| i + 1).unwrap_or(0);
        self.buffer.replace_range(start..at, "");
        self.cursor_pos = start;
    }

    /// Start of the word left of the cursor (Alt+Left).
    pub fn word_start_before(&self) -> usize {
        let head = &self.buffer[..self.cursor()];
        let trimmed = head.trim_end();
        trimmed.rfind([' ', '\n']).map(|i| i + 1).unwrap_or(0)
    }

    /// First char after the word right of the cursor (Alt+Right).
    pub fn word_end_after(&self) -> usize {
        let at = self.cursor();
        match self.buffer[at..].find([' ', '\n']) {
            Some(i) => at + i + 1,
            None => self.buffer.len(),
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
        ("/model", "pick provider/model — /model [provider/model]"),
        ("/login", "save API key — /login [provider]"),
        ("/logout", "remove saved key — /logout <provider>"),
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
            app.notify("y=allow once · a=always · n=deny");
        }
    }
    return Ok(());
}

/// One entry in the `/login` picker: (provider, key-ready, base_url).
pub fn provider_entries(app: &App, filter: &str) -> Vec<(String, bool, String)> {
    let q = filter.trim().to_lowercase();
    let mut out = Vec::new();
    for pid in app.config.all_provider_ids() {
        if q.is_empty() || pid.contains(&q) {
            let (key, _) = crate::auth::resolve_api_key(&pid, &app.auth, &app.config);
            let rpc = app.config.resolve_provider_config(&pid);
            out.push((pid.clone(), !key.is_empty(), rpc.base_url.clone()));
        }
    }
    // Ready providers first, then alphabetical.
    out.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    out.truncate(12);
    out
}

/// Returns the filter text if buffer is in `/model` picker mode.
pub fn model_filter(buffer: &str) -> Option<&str> {
    buffer.strip_prefix("/model ")
}

/// Display label for a (provider, model) pair. Some catalog ids are already
/// namespaced (`nvidia/llama-3.1-...`); don't double the prefix on screen.
pub fn model_label(provider: &str, model: &str) -> String {
    let p = provider.trim().to_lowercase();
    if model.to_lowercase().starts_with(&format!("{}/", p)) {
        model.to_string()
    } else {
        format!("{}/{}", provider, model)
    }
}

/// Commands whose argument is a fixed choice set get the generic picker.
/// Free-form commands (/branch name, /export path) intentionally don't.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ChoiceMode {
    Theme,
    Approve,
    Compact,
    Allow,
    Resume,
    Logout,
}

/// (mode, filter) when the buffer is in a generic-picker command.
pub fn choice_mode(buffer: &str) -> Option<(ChoiceMode, String)> {
    for (prefix, mode) in [
        ("/theme ", ChoiceMode::Theme),
        ("/approve ", ChoiceMode::Approve),
        ("/compact ", ChoiceMode::Compact),
        ("/allow ", ChoiceMode::Allow),
        ("/resume ", ChoiceMode::Resume),
        ("/logout ", ChoiceMode::Logout),
    ] {
        if let Some(filter) = buffer.strip_prefix(prefix) {
            return Some((mode, filter.to_string()));
        }
    }
    None
}

pub fn choice_title(mode: &ChoiceMode) -> &'static str {
    match mode {
        ChoiceMode::Theme => " theme ",
        ChoiceMode::Approve => " approve ",
        ChoiceMode::Compact => " compact ",
        ChoiceMode::Allow => " allow ",
        ChoiceMode::Resume => " resume ",
        ChoiceMode::Logout => " logout ",
    }
}

pub fn choice_prefix(mode: &ChoiceMode) -> &'static str {
    match mode {
        ChoiceMode::Theme => "/theme ",
        ChoiceMode::Approve => "/approve ",
        ChoiceMode::Compact => "/compact ",
        ChoiceMode::Allow => "/allow ",
        ChoiceMode::Resume => "/resume ",
        ChoiceMode::Logout => "/logout ",
    }
}

/// (value, description) rows for a generic picker. Values submit as-is.
pub fn choice_entries(app: &App, mode: &ChoiceMode, filter: &str) -> Vec<(String, String)> {
    let q = filter.trim().to_lowercase();
    let mut out: Vec<(String, String)> = match mode {
        ChoiceMode::Theme => crate::tui::theme::all_themes()
            .into_iter()
            .map(|(n, d)| {
                let cur = if n == app.theme.name { " (current)" } else { "" };
                (n.to_string(), format!("{}{}", d, cur))
            })
            .collect(),
        ChoiceMode::Approve => {
            let on = app.permission_gate.auto_approve();
            vec![
                ("on".into(), format!("auto-approve mutating tools{}", if on { " (current)" } else { "" })),
                ("off".into(), format!("ask every time{}", if !on { " (current)" } else { "" })),
            ]
        }
        ChoiceMode::Compact => vec![
            ("10".into(), "keep last 10 messages".into()),
            ("20".into(), "keep last 20 messages".into()),
            ("50".into(), "keep last 50 messages".into()),
            ("auto on".into(), "auto-compact at ctx ≥85%".into()),
            ("auto off".into(), "manual /compact only".into()),
        ],
        ChoiceMode::Allow => ["write", "edit", "bash", "delegate", "all"]
            .iter()
            .map(|t| {
                let scope = if *t == "all" { "write,edit,bash,delegate" } else { t };
                let state = if scope.split(',').all(|x| app.permission_gate.is_session_allowed(x)) {
                    " (allowed)"
                } else {
                    ""
                };
                (t.to_string(), format!("allow {}{}", scope, state))
            })
            .collect(),
        ChoiceMode::Resume => {
            let mut sessions = app.session_manager.list_sessions();
            sessions.sort_by(|a, b| b.updated.cmp(&a.updated));
            sessions
                .into_iter()
                .take(10)
                .map(|s| (s.id.clone(), format!("{} msgs · {}", s.messages.len(), crate::session::SessionManager::title(&s))))
                .collect()
        }
        ChoiceMode::Logout => app
            .config
            .all_provider_ids()
            .into_iter()
            .filter(|pid| app.auth.has(pid))
            .map(|pid| (pid.clone(), "remove saved key".into()))
            .collect(),
    };
    if !q.is_empty() {
        out.retain(|(v, d)| v.to_lowercase().contains(&q) || d.to_lowercase().contains(&q));
    }
    out.truncate(12);
    out
}

/// Resolve the effective argument for choice commands: a picked value when
/// the generic picker chose something (arrow-navigated, or empty filter
/// meaning "confirm highlight"). Consumes the navigation flag.
/// Returns None when the typed text should flow through untouched.
fn take_choice(app: &mut App, mode: &ChoiceMode, arg: &str) -> Option<String> {
    if !app.picker_navigated && !arg.is_empty() {
        return None;
    }
    let items = choice_entries(app, mode, arg);
    let idx = app.picker_idx;
    reset_picker(app);
    if items.is_empty() {
        return None;
    }
    Some(items[idx % items.len()].0.clone())
}

/// One entry in the `/model` picker: (provider, model, key-ready).
pub fn model_entries(app: &App, filter: &str) -> Vec<(String, String, bool)> {
    let q = filter.trim().to_lowercase();
    let mut out = Vec::new();
    let mut seen: std::collections::HashSet<(String, String)> = std::collections::HashSet::new();
    let mut push = |pid: &str, m: &str, ready: bool, out: &mut Vec<(String, String, bool)>| {
        let key = (pid.to_string(), m.to_lowercase());
        if seen.insert(key) {
            out.push((pid.to_string(), m.to_string(), ready));
        }
    };
    for pid in app.config.all_provider_ids() {
        let ready = !crate::auth::resolve_api_key(&pid, &app.auth, &app.config).0.is_empty();
        // Live-discovered ids first (verified against the real endpoint),
        // then the built-in catalog as offline fallback.
        for m in app.discovered.get(&pid) {
            if q.is_empty() || pid.contains(&q) || m.to_lowercase().contains(&q) {
                push(&pid, &m, ready, &mut out);
            }
        }
        let rpc = app.config.resolve_provider_config(&pid);
        for m in &rpc.models {
            if q.is_empty() || pid.contains(&q) || m.to_lowercase().contains(&q) {
                push(&pid, m, ready, &mut out);
            }
        }
        // Free-form fallback: typed `provider/model` for a known provider
        // whose catalog doesn't list it (e.g. new OpenRouter ids).
        // Skipped when the full filter already names a catalog model exactly:
        // otherwise the short form shadows it and a bogus id can win.
        let exact_catalog_hit = rpc.models.iter().any(|m| m.eq_ignore_ascii_case(filter.trim()));
        if !exact_catalog_hit && !q.is_empty() && q.starts_with(&format!("{}/", pid)) && q.len() > pid.len() + 1 {
            let custom = filter.trim().to_string();
            let custom = custom[pid.len() + 1..].trim().to_string();
            if !custom.is_empty() && !rpc.models.iter().any(|m| m.eq_ignore_ascii_case(&custom)) {
                let ready = !crate::auth::resolve_api_key(&pid, &app.auth, &app.config).0.is_empty();
                out.push((pid.clone(), custom, ready));
            }
        }
    }
    // Ready providers first, then alphabetical.
    out.sort_by(|a, b| b.2.cmp(&a.2).then(a.0.cmp(&b.0)).then(a.1.cmp(&b.1)));
    // Cap the UNFILTERED view only: filtering above already ran on the full
    // set, so typing still finds anything. Matches discovery's per-provider cap.
    out.truncate(100);
    out
}

/// Enter key-entry mode for a provider: next keys go to the masked prompt.
fn begin_login(app: &mut App, pid: &str) {
    app.pending_login = Some(pid.to_string());
    app.login_buffer.clear();
    app.notify(format!("paste API key for '{}' — Enter saves, Esc cancels", pid));
}

fn handle_login_key(app: &mut App, key: crossterm::event::KeyEvent) -> Result<()> {    use crossterm::event::{KeyCode, KeyModifiers};
    // Esc cancels outright.
    if key.code == KeyCode::Esc {
        app.pending_login = None;
        app.login_buffer.clear();
        app.notify("login cancelled");
        return Ok(());
    }
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        match key.code {
            KeyCode::Char('c') => {
                if app.login_buffer.is_empty() {
                    app.pending_login = None;
                } else {
                    app.login_buffer.clear();
                }
                return Ok(());
            }
            KeyCode::Char('u') | KeyCode::Char('w') => {
                app.login_buffer.clear();
                return Ok(());
            }
            _ => return Ok(()),
        }
    }
    match key.code {
        KeyCode::Char(c) => {
            if !key.modifiers.contains(KeyModifiers::ALT) {
                app.login_buffer.push(c);
            }
        }
        KeyCode::Backspace => {
            app.login_buffer.pop();
        }
        KeyCode::Enter => {
            if let Some(pid) = app.pending_login.take() {
                let entered = std::mem::take(&mut app.login_buffer);
                if entered.trim().is_empty() {
                    app.notify("empty key — login cancelled");
                } else {
                    app.auth.set(&pid, entered.trim());
                    // The point of /login is to USE the provider: switch to it
                    // right away so no separate `/model <provider>` is needed.
                    let def = app.config.resolve_default_model(&pid);
                    // Refresh the live model list now that we have a key.
                    app.refresh_provider(&pid);
                    match app.apply_provider_model(&pid, &def) {
                        Ok(_) => {
                            app.notify(format!("saved key + switched to '{}/{}'", pid, def));
                            app.conversation.add_message(
                                "assistant".into(),
                                format!("API key saved for `{}`. Switched to `{}` — ready to chat.", pid, model_label(&pid, &def)),
                            );
                        }
                        Err(e) => {
                            app.notify(format!("saved key for '{}'", pid));
                            app.conversation.add_message("assistant".into(), e);
                        }
                    }
                    app.save_session();
                    app.is_home = false;
                }
            }
        }
        _ => {}
    }
    return Ok(());
}

/// Shift+Enter means "newline, don't submit" (Ctrl+J is the fallback that
/// works everywhere). Only fires with kitty keyboard enhancement active;
/// otherwise the terminal sends plain Enter and this is unreachable.
fn is_shift_enter(key: crossterm::event::KeyEvent) -> bool {
    key.code == KeyCode::Enter
        && key.modifiers.contains(KeyModifiers::SHIFT)
        && !key.modifiers.contains(KeyModifiers::CONTROL)
        && !key.modifiers.contains(KeyModifiers::ALT)
}

/// What bare-`/xxx` Enter should do: run the command, or complete text first.
///
/// Rules (predictable for every command):
/// - arrow-navigated highlight → complete it into the buffer (never run blindly)
/// - text exactly matching a command (`/log`) → run it immediately
/// - strict prefix (`/lo`) → complete the first match, second Enter runs it
/// - no match → fall through (surfaces "unknown command")
#[derive(Debug, PartialEq)]
enum BareEnter {
    Submit,
    Complete(String),
}

fn bare_enter(buffer: &str, palette_idx: usize, navigated: bool) -> Option<BareEnter> {
    if !buffer.starts_with('/') || buffer.contains(' ') || buffer.contains('\n') {
        // NOTE: trailing-space buffers ("/login ", "/model x") are picker
        // mode, not bare mode — they must fall through to handle_slash.
        // Never .trim() here: "/login " trimmed looks bare but isn't.
        return None;
    }
    // Bare picker commands open their picker instead of submitting.
    // (Free-form commands like /branch and /export submit as before.)
    const PICKER_COMMANDS: &[&str] = &[
        "/login", "/model", "/theme", "/approve", "/compact", "/allow", "/resume", "/logout",
    ];
    if PICKER_COMMANDS.contains(&buffer.trim()) {
        return Some(BareEnter::Complete(format!("{} ", buffer.trim())));
    }
    if navigated {
        let items = command_palette(buffer);
        if items.is_empty() {
            return None;
        }
        let (name, _) = items[palette_idx % items.len()];
        return Some(BareEnter::Complete(format!("{} ", name)));
    }
    let trimmed = buffer.trim();
    if all_commands().iter().any(|(n, _)| *n == trimmed) {
        return Some(BareEnter::Submit);
    }
    let items = command_palette(buffer);
    if items.is_empty() {
        return None;
    }
    let (name, _) = items[palette_idx % items.len()];
    Some(BareEnter::Complete(format!("{} ", name)))
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

/// One row in the unified picker: what Tab/Enter writes, what is shown.
pub struct PickerRow {
    pub complete: String,
    pub left: String,
    pub right: String,
    pub dot: Option<bool>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum PickerKind {
    Commands,
    Models,
    Providers,
    Choice(ChoiceMode),
}

/// Which picker (if any) the input buffer opens, plus its filter text.
pub fn picker_kind(buffer: &str) -> Option<(PickerKind, String)> {
    if let Some(f) = model_filter(buffer) {
        return Some((PickerKind::Models, f.to_string()));
    }
    if let Some(rest) = buffer.strip_prefix("/login ") {
        return Some((PickerKind::Providers, rest.to_string()));
    }
    if let Some((m, f)) = choice_mode(buffer) {
        return Some((PickerKind::Choice(m), f));
    }
    if buffer.starts_with('/') && !buffer.contains([' ', '\n']) {
        return Some((PickerKind::Commands, buffer.to_string()));
    }
    None
}

pub fn picker_title(kind: &PickerKind) -> &'static str {
    match kind {
        PickerKind::Commands => "",
        PickerKind::Models => "model",
        PickerKind::Providers => "login",
        PickerKind::Choice(m) => choice_title(m).trim(),
    }
}

/// Rows for the unified picker. Same window math renders all kinds.
pub fn picker_rows(app: &App, kind: &PickerKind, filter: &str) -> Vec<PickerRow> {
    match kind {
        PickerKind::Commands => command_palette(filter)
            .into_iter()
            .map(|(n, d)| PickerRow {
                complete: format!("{} ", n),
                left: format!("{:<10}", n),
                right: d.to_string(),
                dot: None,
            })
            .collect(),
        PickerKind::Models => model_entries(app, filter)
            .into_iter()
            .map(|(p, m, ready)| {
                // Left column must be the unique key: two models of the same
                // provider look identical if it only shows the provider.
                let bare = m.strip_prefix(&format!("{}/", p)).unwrap_or(&m).to_string();
                PickerRow {
                    complete: format!("/model {} ", model_label(&p, &m)),
                    left: p,
                    right: bare,
                    dot: Some(ready),
                }
            })
            .collect(),
        PickerKind::Providers => provider_entries(app, filter)
            .into_iter()
            .map(|(pid, ready, base)| PickerRow {
                complete: format!("/login {} ", pid),
                left: format!("{:<10}", pid),
                right: if base.is_empty() {
                    "(default endpoint)".into()
                } else {
                    crate::tui::ui::truncate(&base, 30)
                },
                dot: Some(ready),
            })
            .collect(),
        PickerKind::Choice(m) => choice_entries(app, m, filter)
            .into_iter()
            .map(|(v, d)| PickerRow {
                complete: format!("{}{} ", choice_prefix(m), v),
                left: format!("{:<12}", v),
                right: crate::tui::ui::truncate(&d, 44),
                dot: None,
            })
            .collect(),
    }
}

/// Arrow navigation inside whichever picker is open.
/// Returns true when the key is consumed. The bare command palette with zero
/// matches yields to input history; every other open picker swallows the key.
fn handle_picker_nav(app: &mut App, dir: isize) -> bool {
    let Some((kind, filter)) = picker_kind(&app.input.buffer).map(|(k, f)| (k, f)) else {
        return false;
    };
    let rows = picker_rows(app, &kind, &filter);
    if rows.is_empty() {
        return !matches!(kind, PickerKind::Commands);
    }
    let n = rows.len();
    app.picker_idx = if dir < 0 {
        (app.picker_idx + n - 1) % n
    } else {
        (app.picker_idx + 1) % n
    };
    app.picker_navigated = true;
    true
}

/// Tab completes the highlighted row of whichever picker is open.
/// Returns true when consumed (picker was open).
fn handle_picker_tab(app: &mut App) -> bool {
    let Some((kind, filter)) = picker_kind(&app.input.buffer).map(|(k, f)| (k, f)) else {
        return false;
    };
    let rows = picker_rows(app, &kind, &filter);
    if rows.is_empty() {
        return true;
    }
    let idx = app.picker_idx % rows.len();
    app.input.buffer = rows[idx].complete.clone();
    app.input.cursor_pos = app.input.buffer.len();
    app.picker_idx = 0;
    app.picker_navigated = false;
    true
}

fn reset_picker(app: &mut App) {
    app.picker_idx = 0;
    app.picker_navigated = false;
}

fn complete_mention(app: &mut App) {
    // find @partial before cursor (byte offsets throughout)
    let cursor = app.input.cursor();
    let head = app.input.buffer[..cursor].to_string();
    let Some(at) = head.rfind('@') else { return };
    let partial = &head[at + 1..];
    if partial.contains(' ') || partial.contains('\n') {
        return;
    }
    let matches = find_files(&app.workspace.root, partial, 20);
    if matches.is_empty() {
        app.notify("no file match");
        return;
    }
    if matches.len() == 1 {
        // insert remainder + space
        let after = app.input.buffer[cursor..].trim_start().to_string();
        let rest = matches[0].strip_prefix(partial).unwrap_or(&matches[0]);
        let new_cursor = cursor + rest.len() + 1;
        app.input.buffer = format!("{}{}{}", &app.input.buffer[..cursor], rest, after);
        app.input.set_cursor(new_cursor);
        return;
    }
    // multiple: show in notice + complete common prefix. The match filter is
    // `contains`, so `partial` is not always a prefix of the common prefix —
    // only extend when it is, else `common[partial.len()..]` is not a boundary.
    let common = common_prefix(&matches);
    if common.len() > partial.len() && common.starts_with(partial) {
        let extra = common[partial.len()..].to_string();
        app.input.insert_str(&extra);
    }
    app.notify(format!("{} matches: {}", matches.len(), matches.iter().take(3).cloned().collect::<Vec<_>>().join(", ")));
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
                // Cut on a char boundary: a fixed byte offset panics when a
                // multi-byte char straddles it.
                let capped = match crate::text::truncate_bytes(&content, 8000) {
                    Some(head) => format!("{}…\n[truncated]", head),
                    None => content,
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

/// `@file` tokens in the input buffer (pure string scan, no I/O —
/// safe to call every render frame). Mirrors the tokenizer in expand_mentions.
pub fn mention_tokens(buffer: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_at = false;
    for c in buffer.chars() {
        if c == '@' {
            in_at = true;
            cur.clear();
        } else if in_at {
            if c.is_whitespace() || c == ',' || c == ')' || c == ']' {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
                in_at = false;
            } else {
                cur.push(c);
            }
        }
    }
    if in_at && !cur.is_empty() {
        out.push(cur);
    }
    out.truncate(5);
    out
}

pub fn handle_events(app: &mut App) -> Result<()> {
    let ev = event::read()?;
    // Mouse wheel scrolls the transcript (terminal scrollback is unavailable
    // in the alternate screen, so without this the wheel does nothing).
    if let Event::Mouse(m) = ev {
        match m.kind {
            crossterm::event::MouseEventKind::ScrollUp => {
                app.chat_scroll = app.chat_scroll.saturating_sub(3);
                app.should_auto_scroll = false;
            }
            crossterm::event::MouseEventKind::ScrollDown => {
                app.chat_scroll = app.chat_scroll.saturating_add(3);
                app.should_auto_scroll = false;
            }
            _ => {}
        }
        return Ok(());
    }
    if let Event::Key(key) = ev {
        if key.kind != KeyEventKind::Press {
            return Ok(());
        }

        // Permission modal takes over all keys while pending.
        if app.pending_approval.is_some() {
            return handle_permission_key(app, key.code);
        }
        // API-key entry mode takes over all keys while pending.
        if app.pending_login.is_some() {
            return handle_login_key(app, key);
        }
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let alt = key.modifiers.contains(KeyModifiers::ALT);
        app.clear_ephemeral_notice();

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
                    app.input.delete_before_cursor();
                    return Ok(());
                }
                KeyCode::Char('k') => {
                    app.input.delete_to_cursor();
                    return Ok(());
                }
                KeyCode::Char('w') => {
                    app.input.delete_word_before();
                    return Ok(());
                }
                KeyCode::Char('a') => {
                    app.input.move_cursor_home();
                    return Ok(());
                }
                KeyCode::Char('e') => {
                    app.input.move_cursor_end();
                    return Ok(());
                }
                KeyCode::Char('j') => {
                    app.input.insert_newline();
                    return Ok(());
                }
                KeyCode::Char('o') | KeyCode::Char('O') => {
                    app.tool_expanded = !app.tool_expanded;
                    app.notify(if app.tool_expanded { "tool output expanded" } else { "tool output collapsed" });
                    return Ok(());
                }
                KeyCode::Char('t') | KeyCode::Char('T') => {
                    app.workspace = crate::workspace::WorkspaceContext::new();
                    app.tree_visible = !app.tree_visible;
                    app.notify(if app.tree_visible { "tree panel on" } else { "tree panel off" });
                    return Ok(());
                }
                _ => {}
            }
        }
        if alt {
            // Alt+Left/Right word jump (best-effort)
            match key.code {
                KeyCode::Left => {
                    app.input.cursor_pos = app.input.word_start_before();
                    return Ok(());
                }
                KeyCode::Right => {
                    app.input.cursor_pos = app.input.word_end_after();
                    return Ok(());
                }
                _ => {}
            }
        }

        match key.code {
            KeyCode::Char(c) => {
                app.input.insert_char(c);
                reset_picker(app);
            }
            KeyCode::Backspace => {
                app.input.backspace();
                reset_picker(app);
            }
            KeyCode::Delete => {
                app.input.delete();
            }
            KeyCode::Left => {
                app.input.move_left();
            }
            KeyCode::Right => {
                app.input.move_right();
            }
            KeyCode::Home => app.input.move_cursor_home(),
            KeyCode::End => app.input.move_cursor_end(),
            KeyCode::Up => {
                if handle_picker_nav(app, -1) {
                    return Ok(());
                }
                app.input.navigate_history(-1);
            }
            KeyCode::Down => {
                if handle_picker_nav(app, 1) {
                    return Ok(());
                }
                app.input.navigate_history(1);
            }
            KeyCode::Tab => {
                if handle_picker_tab(app) {
                    return Ok(());
                } else if app.input.buffer.contains('@') {
                    complete_mention(app);
                } else {
                    app.input.insert_str("  ");
                }
            }
            KeyCode::Enter => {
                return handle_enter(app, is_shift_enter(key));
            }
            KeyCode::Esc => {
                if app.event_rx.is_some() {
                    app.cancelled.store(true, Ordering::Relaxed);
                    app.event_rx = None;
                    app.save_session();
                    app.streaming_text.clear();
                    app.status.tool_status = "cancelled".into();
                    // Cancelling means "stop", so don't fire queued messages
                    // the moment the run unwinds.
                    if !app.queued_input.is_empty() {
                        let n = app.queued_input.len();
                        app.queued_input.clear();
                        app.notify(format!("cancelled · {} queued dropped", n));
                    }
                } else if app.input.buffer.starts_with('/') {
                    app.input.buffer.clear();
                    app.input.move_cursor_home();
                    reset_picker(app);
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

/// How many messages can wait behind a running agent turn.
const MAX_QUEUED: usize = 8;

/// Park a message typed mid-run. Oldest goes first when full — the alternative
/// is refusing input, which is what made the busy state feel broken.
fn queue_message(app: &mut App, text: String) {
    if app.queued_input.len() >= MAX_QUEUED {
        app.queued_input.remove(0);
        app.notify_blocking_cause(
            format!("queue full ({}) — a queued message was dropped", MAX_QUEUED),
            NoticeCause::QueueDropped,
        );
    }
    let n = app.queued_input.len() + 1;
    app.queued_input.push(text);
    if app.notice.is_none() {
        app.notify(format!("queued (#{} in line) — sends when the agent stops", n));
    }
}

/// The whole Enter path: Shift+Enter newline, bare-picker completion, then
/// submit. The key handler and every test call *this*, so no test can bypass
/// the completion step the way a hand-rolled copy of the arm did — that gap is
/// exactly how "arrow key selects nvidia, Enter stores 9router" shipped.
fn handle_enter(app: &mut App, shift_enter: bool) -> Result<()> {
    if shift_enter {
        app.input.insert_newline();
        return Ok(());
    }
    // Bare `/xxx`: exact command runs, highlight completes; bare picker
    // commands open their picker instead of running.
    if let Some(BareEnter::Complete(text)) =
        bare_enter(&app.input.buffer, app.picker_idx, app.picker_navigated)
    {
        app.input.buffer = text.clone();
        app.input.move_cursor_end();
        reset_picker(app);
        if text == "/model " {
            // Preselect the active model (highlight only).
            let items = model_entries(app, "");
            if let Some(pos) = items
                .iter()
                .position(|(p, m, _)| p == &app.provider_name && m == &app.current_model)
            {
                app.picker_idx = pos;
            }
        } else if text == "/login " {
            // Preselect the first provider that still needs a key, so the
            // highlight matches what Enter is going to do.
            if let Some(pos) = provider_entries(app, "")
                .iter()
                .position(|(_, ready, _)| !ready)
            {
                app.picker_idx = pos;
            }
        }
        return Ok(());
    }
    let input = std::mem::take(&mut app.input.buffer);
    app.input.move_cursor_home();
    app.input.history_index = None;
    if input.trim().is_empty() {
        reset_picker(app);
        return Ok(());
    }
    if app.event_rx.is_some() {
        // The agent is mid-run: keep the text. Sending it now would interleave
        // with the live turn, so it waits in the queue — dropping it silently
        // (what this used to do) lost work.
        queue_message(app, input);
        reset_picker(app);
        return Ok(());
    }
    // Reset only *after* dispatch: `handle_slash` reads picker_idx and
    // picker_navigated to know what the highlighted row means. Resetting first
    // silently discarded every arrow-key selection.
    let result = submit_message(app, input);
    reset_picker(app);
    result
}

/// Run one message: slash command, or a full agent turn. Used both by `Enter`
/// and by the queue drain in `app::handle_stream`.
pub fn submit_message(app: &mut App, input: String) -> Result<()> {
    if input.trim_start().starts_with('/') && handle_slash(app, input.trim())? {
        return Ok(());
    }
    // @file expansion (context attach)
    let (expanded, missing) = expand_mentions(&input, &app.workspace.root);
    if !missing.is_empty() {
        app.notify_blocking_cause(
            format!("@ not found: {}", missing.join(", ")),
            NoticeCause::MissingMention,
        );
    } else {
        // The bad mention is gone from the draft, so its notice has served
        // its purpose — but only that one, never an unrelated "no API key".
        app.resolve_notice(NoticeCause::MissingMention);
    }
    // The turn is actually starting, so the queue is accepting messages again
    // and a previous send failure no longer describes the present.
    app.resolve_notice(NoticeCause::QueueDropped);
    app.resolve_notice(NoticeCause::SendFailed);
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
    Ok(())
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
            // Nothing of the old turn is in the window any more.
            app.last_prompt_tokens = None;
            return Ok(true);
        }
        "/model" => {
            if arg.is_empty() || app.picker_navigated {
                // Picker was open: Enter confirms the highlighted entry.
                // (On open it's preselected to the active model; arrows move it.)
                let items = model_entries(app, arg);
                if items.is_empty() {
                    app.conversation.add_message("assistant".into(), "No matching models.".into());
                } else {
                    let idx = app.picker_idx % items.len();
                    let (p, m, ready) = items[idx].clone();
                    match app.apply_provider_model(&p, &m) {
                        Ok(_) => {
                            let warn = if !ready { " (no key yet — `/login` to activate)".to_string() } else { String::new() };
                            app.conversation.add_message("assistant".into(), format!("**Model set to:** `{}`{}", model_label(&p, &m), warn));
                        }
                        Err(e) => app.conversation.add_message("assistant".into(), e),
                    }
                }
                reset_picker(app);
            } else if let Some(pid) = app
                .config
                .all_provider_ids()
                .into_iter()
                .find(|id| id.eq_ignore_ascii_case(arg.trim()))
            {
                // Bare provider id (`/model nvidia`) → switch provider, default model.
                let def = app.config.resolve_default_model(&pid);
                match app.apply_provider_model(&pid, &def) {
                    Ok(_) => {
                        let (k, _) = crate::auth::resolve_api_key(&pid, &app.auth, &app.config);
                        let warn = if k.is_empty() {
                            " (no key yet — `/login` to activate)".to_string()
                        } else {
                            String::new()
                        };
                        app.conversation.add_message(
                            "assistant".into(),
                            format!("**Model set to:** `{}/{}`{}", pid, def, warn),
                        );
                    }
                    Err(e) => app.conversation.add_message("assistant".into(), e),
                }
            } else if let Some((p, m)) = app.parse_model_arg(arg) {
                let items = model_entries(app, arg);
                // Exact catalog hit, or unambiguous single match.
                let hit = items.iter().find(|(ep, em, _)| ep == &p && em.eq_ignore_ascii_case(&m));
                if let Some((ep, em, _)) = hit {
                    let (ep, em) = (ep.clone(), em.clone());
                    match app.apply_provider_model(&ep, &em) {
                        Ok(_) => app.conversation.add_message("assistant".into(), format!("**Model set to:** `{}`", model_label(&ep, &em))),
                        Err(e) => app.conversation.add_message("assistant".into(), e),
                    }
                } else if items.len() == 1 {
                    let (ep, em, _) = items[0].clone();
                    match app.apply_provider_model(&ep, &em) {
                        Ok(_) => app.conversation.add_message("assistant".into(), format!("**Model set to:** `{}`", model_label(&ep, &em))),
                        Err(e) => app.conversation.add_message("assistant".into(), e),
                    }
                } else if items.is_empty() {
                    // Free-form `provider/model` (e.g. brand-new OpenRouter id).
                    match app.apply_provider_model(&p, &m) {
                        Ok(_) => app.conversation.add_message("assistant".into(), format!("**Model set to:** `{}`\n_custom id — verify it exists on the provider._", model_label(&p, &m))),
                        Err(e) => app.conversation.add_message("assistant".into(), e),
                    }
                } else {
                    let mut out = format!("**{} matches** — refine or ↑↓ + Enter:\n", items.len());
                    for (ep, em, ready) in items.iter().take(10) {
                        out.push_str(&format!("- {} `{}`\n", if *ready { "●" } else { "○" }, model_label(ep, em)));
                    }
                    app.conversation.add_message("assistant".into(), out);
                }
            } else {
                app.conversation.add_message("assistant".into(), "Usage: `/model <provider/model>`".into());
            }
            app.save_session();
            app.is_home = false;
            return Ok(true);
        }
        "/login" => {
            if app.picker_navigated || arg.is_empty() {
                // Enter always acts on the highlighted row. When the picker was
                // only just opened, `picker_idx` was preselected to the first
                // provider that still needs a key (see the Complete arm) — the
                // old code searched for that provider *here* instead, so the
                // highlight pointed at one row and Enter picked another.
                let items = provider_entries(app, arg);
                if items.is_empty() {
                    app.notify_blocking_cause(
                        "no matching providers",
                        NoticeCause::NoProviderMatch,
                    );
                } else if let Some(sel) = items.get(app.picker_idx % items.len()) {
                    // A provider is in hand, so "no matching providers" is
                    // history.
                    app.resolve_notice(NoticeCause::NoProviderMatch);
                    let pid = sel.0.clone();
                    begin_login(app, &pid);
                }
                reset_picker(app);
            } else {
                let word = arg.split_whitespace().next().unwrap_or("");
                // Exact id first, else single fuzzy match (mirror /model).
                let ids = app.config.all_provider_ids();
                if let Some(pid) = ids.iter().find(|id| id.eq_ignore_ascii_case(word)).cloned() {
                    begin_login(app, &pid);
                } else {
                    let items = provider_entries(app, word);
                    if items.len() == 1 {
                        begin_login(app, &items[0].0.clone());
                    } else if items.is_empty() {
                        app.conversation.add_message("assistant".into(), format!("Unknown provider `{}`. Known: {}.\nCustom OpenAI-compatible endpoints go in `barong.jsonc` under `providers`.", word, ids.join(", ")));
                    } else {
                        let mut out = format!("**{} matches** — refine or ↑↓ + Enter:\n", items.len());
                        for (pid, ready, _) in items.iter().take(8) {
                            out.push_str(&format!("- {} `{}`\n", if *ready { "●" } else { "○" }, pid));
                        }
                        app.conversation.add_message("assistant".into(), out);
                    }
                }
            }
            app.save_session();
            app.is_home = false;
            return Ok(true);
        }
        "/logout" => {
            let picked = take_choice(app, &ChoiceMode::Logout, arg);
            let arg = picked.as_deref().unwrap_or(arg);
            if arg.is_empty() {
                app.conversation.add_message("assistant".into(), "Usage: `/logout <provider>`".into());
            } else {
                let pid = arg.split_whitespace().next().unwrap_or("").to_lowercase();
                if app.auth.remove(&pid) {
                    let (_, src) = crate::auth::resolve_api_key(&app.provider_name, &app.auth, &app.config);
                    app.key_source = src;
                    app.conversation.add_message("assistant".into(), format!("Removed saved key for `{}`.", pid));
                } else {
                    app.conversation.add_message("assistant".into(), format!("No saved key for `{}` (auth.json). Env keys are managed outside barong.", pid));
                }
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
            let picked = take_choice(app, &ChoiceMode::Allow, arg);
            let arg = picked.as_deref().unwrap_or(arg);
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
            let picked = take_choice(app, &ChoiceMode::Approve, arg);
            let arg = picked.as_deref().unwrap_or(arg);
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
                "**Session:** `{}`\n- provider: `{}`\n- model: `{}`\n- key: {} ({})\n- msgs: {}\n- cwd: `{}`\n- ctx: {}% of {}k window ({})\n- auto-compact: {} (keep={})\n- theme: `{}`\n- tree: {}",
                app.session_id.as_deref().unwrap_or("(unsaved)"),
                app.provider_name,
                app.current_model,
                if app.api_key().is_empty() { "missing" } else { "set" },
                app.key_source,
                app.conversation.messages.len(),
                app.workspace.root.display(),
                (app.context_usage() * 100.0) as u32,
                app.config.resolve_context_window() / 1000,
                if app.context_is_estimated() {
                    "estimated"
                } else {
                    "provider-reported"
                },
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
            let picked = take_choice(app, &ChoiceMode::Resume, arg);
            let arg = picked.as_deref().unwrap_or(arg);
            if arg.is_empty() {
                let mut sessions = app.session_manager.list_sessions();
                sessions.sort_by(|a, b| b.updated.cmp(&a.updated));
                if sessions.is_empty() {
                    app.conversation.add_message("assistant".into(), "No saved sessions.".into());
                } else {
                    let mut out = String::from("**Recent sessions:**\n");
                    for s in sessions.iter().take(5) {
                        out.push_str(&format!("- `{}` — {} msgs · {}\n", s.id, s.messages.len(), crate::session::SessionManager::title(s)));
                    }
                    out.push_str("\nUse `/resume <id>`");
                    app.conversation.add_message("assistant".into(), out);
                }
            } else if let Some(s) = app.session_manager.load(arg) {
                app.conversation.messages = s.messages;
                app.session_id = Some(s.id.clone());
                // Resume the endpoint the session was saved with, not today's.
                if let (Some(p), Some(m)) = (s.provider.clone(), s.model.clone()) {
                    if app.config.resolve_provider_config(&p).known && !m.trim().is_empty() {
                        let _ = app.apply_provider_model(&p, &m);
                    }
                }
                if let Some(t) = s.theme.clone() {
                    app.adopt_theme(&t);
                }
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
                app.notify("nothing to copy");
            } else if copy_to_clipboard(&last) {
                app.notify("copied last response");
            } else {
                app.conversation.add_message("assistant".into(), format!("```\n{}\n```", last.chars().take(2000).collect::<String>()));
            }
            app.is_home = false;
            return Ok(true);
        }
        "/compact" => {
            let picked = take_choice(app, &ChoiceMode::Compact, arg);
            let arg = picked.as_deref().unwrap_or(arg);
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
            // The file tree just changed size, so the cached prompt overhead
            // (and the provider's count for the previous one) are both stale.
            app.last_prompt_tokens = None;
            app.refresh_prompt_overhead_for_reload();
            app.notify("workspace reloaded");
            app.is_home = false;
            return Ok(true);
        }
        "/theme" => {
            let picked = take_choice(app, &ChoiceMode::Theme, arg);
            let arg = picked.as_deref().unwrap_or(arg);
            if arg.is_empty() {
                app.conversation.add_message("assistant".into(), "**Themes:** dark · light · barong\nUsage: `/theme <name>`".into());
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
            app.notify(if app.tree_visible { "tree panel on" } else { "tree panel off" });
            app.is_home = false;
            return Ok(true);
        }
        "/branch" => {
            let name = if arg.is_empty() { "branch" } else { arg };
            let parent = app.session_id.clone();
            let meta = app.session_meta();
            let id = app.session_manager.fork(&app.conversation.messages, parent.as_deref(), name, &meta);
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
                    out.push_str(&format!("- `{}` — {} msgs · branch `{}` · parent `{}` · {}{}\n", s.id, s.messages.len(), branch, parent, crate::session::SessionManager::title(s), cur));
                }
                out.push_str("\nUse `/resume <id>` or `/branch [name]`");
                app.conversation.add_message("assistant".into(), out);
            }
            app.save_session();
            app.is_home = false;
            return Ok(true);
        }
        "/hotkeys" => {
            app.conversation.add_message("assistant".into(), "**Keys:**\n- `Enter` send (queues while the agent works) · `Shift+Enter/Ctrl+J` newline\n- `Tab` complete `/` or `@` · `Up/Down` palette/history\n- `Ctrl+C` clear/quit · `Ctrl+D` quit · `Ctrl+U/K/W` edit · `Ctrl+A/E` jump\n- `Ctrl+O` expand tools · `Ctrl+T` tree panel · `Esc` cancel · `PgUp/PgDn` or wheel scroll\n- approval modal: `y` once · `a` always · `n`/`Esc` deny".into());
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
            out.push_str("\n**Picker (same everywhere):** type to filter · `↑↓` move · `Tab` complete · `Enter` confirm · `Esc` back out");
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    /// App isolated from the real ~/.barong (unique temp HOME per call, so
    /// session-restore never leaks state between tests).
    /// HOME is process-global: serialize the whole dance so parallel tests
    /// can't observe each other's (or the real) home mid-swap.
    static HOME_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    fn test_app() -> App {
        let _guard = HOME_LOCK.lock().unwrap();
        test_app_locked()
    }

    /// Assumes HOME_LOCK is held.
    fn test_app_locked() -> App {
        use std::sync::atomic::{AtomicU64, Ordering};
        static N: AtomicU64 = AtomicU64::new(0);
        let n = N.fetch_add(1, Ordering::Relaxed);
        let tmp = std::env::temp_dir().join(format!("barong-test-{}-{}", std::process::id(), n));
        let _ = std::fs::create_dir_all(&tmp);
        let orig = std::env::var("HOME").ok();
        // SAFETY: HOME dance holds HOME_LOCK; no other thread observes mid-swap.
        unsafe { std::env::set_var("HOME", &tmp); }
        let app = App::new_with_config(Config::default(), vec![]);
        match orig {
            // SAFETY: restoring what we found, same reasoning as above.
            Some(o) => unsafe { std::env::set_var("HOME", o) },
            None => unsafe { std::env::remove_var("HOME") },
        }
        app
    }

    #[test]
    fn model_bare_provider_switches_with_default() {
        let mut app = test_app();
        app.auth.set("nvidia", "nvapi-test-key");
        assert_eq!(app.provider_name, "openai");
        handle_slash(&mut app, "/model nvidia").unwrap();
        assert_eq!(app.provider_name, "nvidia", "bare provider id must switch provider");
        assert_eq!(app.current_model, "openai/gpt-oss-20b");
    }

    #[test]
    fn model_full_id_switches_provider() {
        let mut app = test_app();
        app.auth.set("nvidia", "nvapi-test-key");
        handle_slash(&mut app, "/model nvidia/openai/gpt-oss-20b").unwrap();
        assert_eq!(app.provider_name, "nvidia");
        assert_eq!(app.current_model, "openai/gpt-oss-20b");
    }

    #[test]
    fn model_freeform_future_id_applies_with_warning() {
        // IDs released after this catalog still work via free-form.
        let mut app = test_app();
        app.auth.set("nvidia", "nvapi-test-key");
        handle_slash(&mut app, "/model nvidia/some-future-model-xyz").unwrap();
        assert_eq!(app.provider_name, "nvidia");
        assert_eq!(app.current_model, "some-future-model-xyz");
        let last = app.conversation.messages.last().and_then(|m| m.content.clone()).unwrap_or_default();
        assert!(last.contains("Model set to"), "got: {}", last);
    }

    #[test]
    fn model_unprefixed_known_id_switches_provider() {
        let mut app = test_app();
        app.auth.set("deepseek", "sk-test-key");
        handle_slash(&mut app, "/model deepseek-reasoner").unwrap();
        assert_eq!(app.provider_name, "deepseek");
        assert_eq!(app.current_model, "deepseek-reasoner");
    }

    #[test]
    fn login_picker_lists_providers_with_key_status() {
        let mut app = test_app();
        // No keys: all present; only keyless ollama is ready.
        let all = provider_entries(&app, "");
        assert!(all.iter().any(|(p, _, _)| p == "nvidia"));
        assert!(all.iter().any(|(p, _, _)| p == "openrouter"));
        assert!(all.iter().filter(|(p, _, _)| p != "ollama").all(|(_, ready, _)| !ready));
        // After login: nvidia sorts first with ready=true.
        app.auth.set("nvidia", "nvapi-test-key");
        let all = provider_entries(&app, "");
        assert_eq!(all[0].0, "nvidia");
        assert!(all[0].1);
        // Filter narrows.
        let f = provider_entries(&app, "deep");
        assert!(!f.is_empty());
        assert!(f.iter().all(|(p, _, _)| p.contains("deep")));
    }

    #[test]
    fn login_partial_name_fuzzy_matches_single() {
        let mut app = test_app();
        handle_slash(&mut app, "/login nvid").unwrap();
        assert_eq!(app.pending_login.as_deref(), Some("nvidia"));
    }

    #[test]
    fn keyless_public_catalogs_are_discoverable() {
        let app = test_app();
        // No keys saved anywhere: openrouter/ollama/9router serve /v1/models
        // publicly, gated ones (deepseek, nvidia) stay skipped.
        assert!(app.discovery_target("9router").is_some());
        assert!(app.discovery_target("openrouter").is_some());
        assert!(app.discovery_target("deepseek").is_none());
        assert!(app.discovery_target("anthropic").is_none());
    }

    #[test]
    fn mention_tokens_scans_without_io() {        assert_eq!(
            mention_tokens("fix @src/main.rs and @lib.rs now"),
            vec!["src/main.rs".to_string(), "lib.rs".to_string()]
        );
        assert!(mention_tokens("no mentions here").is_empty());
        assert_eq!(mention_tokens("@a,@b"), vec!["a".to_string(), "b".to_string()]);
        assert!(mention_tokens("@").is_empty());
        assert_eq!(mention_tokens("@a @a @a @a @a @a"), vec!["a".to_string(); 5], "capped at 5");
    }

    #[test]
    fn shift_enter_means_newline_plain_enter_submits() {
        use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
        let shift = KeyEvent {
            code: KeyCode::Enter,
            modifiers: KeyModifiers::SHIFT,
            kind: KeyEventKind::Press,
            state: KeyEventState::empty(),
        };
        assert!(is_shift_enter(shift));
        let plain = KeyEvent {
            code: KeyCode::Enter,
            modifiers: KeyModifiers::empty(),
            kind: KeyEventKind::Press,
            state: KeyEventState::empty(),
        };
        assert!(!is_shift_enter(plain));
        let ctrl_j = KeyEvent {
            code: KeyCode::Char('j'),
            modifiers: KeyModifiers::CONTROL,
            kind: KeyEventKind::Press,
            state: KeyEventState::empty(),
        };
        assert!(!is_shift_enter(ctrl_j));
    }

    /// Full interactive audit through a headless terminal: picker opens for
    /// every mode, highlight moves with navigation, Enter applies it.
    /// This is the closest we get to clicking through the TUI in CI.
    fn draw_cells(app: &mut App, w: u16, h: u16) -> Vec<Vec<ratatui::buffer::Cell>> {
        use ratatui::{backend::TestBackend, Terminal};
        let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
        terminal.draw(|f| app.render(f)).unwrap();
        let buf = terminal.backend().buffer().clone();
        (0..h)
            .map(|y| (0..w).map(|x| buf[(x, y)].clone()).collect::<Vec<_>>())
            .collect()
    }

    /// Top and bottom border rows of the rounded picker box.
    fn picker_span(rows: &[Vec<ratatui::buffer::Cell>], h: u16) -> Option<(usize, usize)> {
        let top = (0..h as usize).find(|&y| row_text(rows, y).contains('╭'))?;
        let bottom = (0..h as usize).find(|&y| row_text(rows, y).contains('╰'))?;
        Some((top, bottom))
    }

    /// Text rows (y) containing a highlighted (accent-bg) cell.
    fn accent_rows(rows: &[Vec<ratatui::buffer::Cell>], accent: ratatui::style::Color) -> Vec<usize> {
        rows.iter()
            .enumerate()
            .filter(|(_, r)| r.iter().any(|c| c.bg == accent))
            .map(|(y, _)| y)
            .collect()
    }

    /// One drawn row as plain text.
    fn row_text(rows: &[Vec<ratatui::buffer::Cell>], y: usize) -> String {
        rows[y].iter().map(|c| c.symbol().to_string()).collect()
    }

    /// Rows of the prompt box, found by its `▌` accent bar rather than by
    /// counting from the bottom — layout changes must not silently break
    /// assertions.
    fn prompt_box(rows: &[Vec<ratatui::buffer::Cell>], h: u16) -> Vec<String> {
        (0..h as usize)
            .map(|y| row_text(rows, y))
            .filter(|l| l.starts_with('\u{258c}'))
            .collect()
    }

    /// Assert the box anatomy: pad, draft…, breathing row, endpoint, pad.
    fn assert_box_shape(box_rows: &[String], what: &str) {
        assert!(box_rows.len() >= 5, "{}: {} rows: {:?}", what, box_rows.len(), box_rows);
        let bare = |i: usize| box_rows[i].trim_start_matches('\u{258c}').trim().is_empty();
        assert!(bare(0), "{}: top pad missing: {:?}", what, box_rows);
        assert!(!bare(1), "{}: draft row is empty: {:?}", what, box_rows);
        assert!(bare(2), "{}: breathing row missing: {:?}", what, box_rows);
        assert!(!bare(3), "{}: endpoint row is empty: {:?}", what, box_rows);
        assert!(bare(4), "{}: bottom pad missing: {:?}", what, box_rows);
    }

    /// The endpoint row: the second-to-last row of the box.
    fn endpoint_row(rows: &[Vec<ratatui::buffer::Cell>], h: u16) -> String {
        prompt_box(rows, h)
            .get(prompt_box(rows, h).len().saturating_sub(2))
            .cloned()
            .unwrap_or_default()
            .trim_start_matches('\u{258c}')
            .trim()
            .to_string()
    }

    /// Single status line, always the last row.
    fn status_line(rows: &[Vec<ratatui::buffer::Cell>], h: u16) -> String {
        row_text(rows, h as usize - 1)
    }

    /// The dim notice line, when one is showing (row above the prompt box).
    fn notice_line(rows: &[Vec<ratatui::buffer::Cell>], h: u16) -> Option<String> {
        let first_box = (0..h as usize).find(|&y| row_text(rows, y).starts_with('\u{258c}'))?;
        if first_box == 0 {
            return None;
        }
        let above = row_text(rows, first_box - 1);
        if above.trim().is_empty() {
            None
        } else {
            Some(above)
        }
    }

    /// The chat viewport: everything above the notice/prompt chrome.
    fn chat_rows(rows: &[Vec<ratatui::buffer::Cell>], h: u16) -> Vec<String> {
        let first_box = (0..h as usize).find(|&y| row_text(rows, y).starts_with('\u{258c}'));
        let end = match first_box {
            Some(b) => {
                let mut e = b;
                if e > 0 && !row_text(rows, e - 1).trim().is_empty() {
                    e -= 1; // notice line
                }
                e
            }
            None => h as usize - 2,
        };
        (0..end).map(|y| row_text(rows, y)).collect()
    }

    /// A transcript whose every paragraph wraps: the case where counting
    /// logical lines used to leave the newest text off-screen.
    fn wrapping_convo(n: usize) -> Vec<String> {
        (0..n)
            .map(|i| {
                format!(
                    "Jawaban nomor {} yang cukup panjang untuk membuat baris membungkus di terminal eighty dua kolom lebarnya.\n- detail satu\n- detail dua",
                    i
                )
            })
            .collect()
    }

    #[test]
    fn prompt_box_has_accent_bar_endpoint_row_and_one_status_line() {
        let mut app = test_app();
        app.input.buffer = "read @src/main.rs".into();
        app.input.set_cursor(app.input.buffer.len());
        let rows = draw_cells(&mut app, 80, 24);
        let prompt = prompt_box(&rows, 24);
        // pad + draft + breathing + endpoint + pad
        assert_eq!(prompt.len(), 5, "{:?}", prompt);
        assert_box_shape(&prompt, "idle prompt");
        assert!(
            prompt[1].contains("read @src/main.rs"),
            "draft: {:?}",
            prompt
        );
        // The endpoint row lives inside the box, like OpenCode's agent/model line.
        let endpoint = endpoint_row(&rows, 24);
        assert!(
            endpoint.contains(&app.provider_name),
            "provider: {:?}",
            endpoint
        );
        assert!(
            endpoint.contains(&app.current_model),
            "model: {:?}",
            endpoint
        );
        assert!(
            endpoint.contains(if app.permission_gate.auto_approve() { "auto" } else { "ask" }),
            "mode: {:?}",
            endpoint
        );
        // The status line follows the box directly, with no rule in between.
        let status = status_line(&rows, 24);
        assert!(status.contains("msgs"), "status: {:?}", status);
        assert!(status.contains('(') && status.contains('%'), "gauge: {:?}", status);
        assert!(!status.contains("dark"), "theme name is not daily-use info: {:?}", status);
        // Every row of the box carries the panel background — including the
        // endpoint row, which used to sit on the terminal default and showed
        // as a pale band across the box.
        for line in rows.iter().rev().skip(1).take(5) {
            for cell in line.iter().take(70) {
                assert_eq!(
                    cell.bg,
                    app.theme.panel,
                    "box row must be panel-coloured, got {:?}",
                    cell.bg
                );
            }
        }
    }

    #[test]
    fn notice_lands_above_the_prompt_not_in_the_status_row() {
        let mut app = test_app();
        app.notify_blocking("no API key for 'openai' — /login openai or set env");
        let rows = draw_cells(&mut app, 80, 24);
        let notice = notice_line(&rows, 24).expect("notice row");
        assert!(notice.contains("no API key"), "notice: {:?}", notice);
        // Blocking messages get the warning colour and a `!` marker.
        assert!(notice.trim_start().starts_with('!'), "marker: {:?}", notice);
        let y = (0..24usize).find(|y| row_text(&rows, *y).contains("no API key")).unwrap();
        assert_eq!(rows[y][0].fg, app.theme.warning, "blocking colour");
        // The status line stays short regardless.
        assert!(
            !status_line(&rows, 24).contains("no API key"),
            "status must stay short: {:?}",
            status_line(&rows, 24)
        );
    }

    #[test]
    fn blocking_notice_survives_keystrokes_but_info_does_not() {
        let mut app = test_app();

        app.notify("theme: barong");
        assert!(app.notice.is_some());
        app.clear_ephemeral_notice();
        assert!(app.notice.is_none(), "info is cleared by the next keystroke");

        app.notify_blocking("no API key for 'openai'");
        app.clear_ephemeral_notice();
        assert!(
            app.notice.is_some(),
            "a message the user must act on must not vanish on the first keypress"
        );
        // Explicit acknowledgement still clears it.
        app.clear_notice();
        assert!(app.notice.is_none());
    }

    /// Resuming is state, not status. `/resume` already says so in the
    /// transcript, so the status line must stay quiet — otherwise the banner
    /// sits above the prompt looking like an instruction, and restoring the
    /// session's theme used to leave "theme: light" there instead.
    #[test]
    fn resuming_says_it_in_the_transcript_and_not_on_the_status_line() {
        use crate::session::SessionMeta;
        let mut app = test_app();
        let meta = SessionMeta {
            provider: "openai".into(),
            model: "gpt-4o".into(),
            theme: "light".into(),
        };
        let sid = app.session_manager.save(&app.conversation.messages, &meta);
        app.clear_notice();

        app.input.buffer = format!("/resume {sid}");
        crate::tui::input::handle_enter(&mut app, false).unwrap();
        app.adopt_theme("dark"); // undo the restore so the assert below is real

        let rows = draw_cells(&mut app, 84, 24);
        // A blocking "no API key" is a fair thing for the status line to say
        // here; anything about the resume or the theme is not.
        if let Some(line) = notice_line(&rows, 24) {
            assert!(
                !line.contains("esumed") && !line.contains("theme"),
                "resume must not talk about itself on the status line: {:?}",
                line
            );
        }
        // And the id it prints has to be the real one, not a truncated one.
        let convo = chat_rows(&rows, 24).join("\n");
        assert!(
            convo.contains(&format!("Resumed `{sid}`")),
            "full id in the transcript: {:?}",
            convo
        );
    }

    /// A theme the user did not ask about must not appear as a notice, while
    /// the one they did ask for still confirms itself.
    #[test]
    fn adopting_a_theme_quietly_leaves_the_notice_alone() {
        let mut app = test_app();
        app.clear_notice();
        app.adopt_theme("light");
        assert!(app.notice.is_none(), "adopting is not an event worth reporting");

        app.set_theme("dark");
        assert_eq!(app.notice.as_ref().map(|n| n.text.as_str()), Some("theme: dark"));
    }

    #[test]
    fn resolving_one_condition_leaves_unrelated_blocking_notices_alone() {
        let mut app = test_app();

        app.notify_blocking_cause("no API key for 'openai'", NoticeCause::MissingKey);
        app.resolve_notice(NoticeCause::MissingMention);
        assert!(
            app.notice.is_some(),
            "a resolved @mention must not silently clear an unrelated missing key"
        );

        app.resolve_notice(NoticeCause::MissingKey);
        assert!(app.notice.is_none(), "resolving its own cause clears it");
    }

    #[test]
    fn a_bad_mention_is_retired_once_the_draft_is_clean() {
        let mut app = test_app();

        submit_message(&mut app, "@tidak-ada.txt hello".into()).unwrap();
        let notice = app.notice.clone().expect("blocking notice");
        assert!(notice.is_blocking());
        assert!(notice.text.contains("@ not found"), "text: {:?}", notice.text);
        assert_eq!(notice.cause, NoticeCause::MissingMention);
        // Typing must not take it away...
        app.clear_ephemeral_notice();
        assert!(app.notice.is_some(), "still blocking");

        // ...but a draft whose mentions all resolve retires it.
        submit_message(&mut app, "hello".into()).unwrap();
        assert!(
            app.notice.is_none(),
            "the bad mention is gone, so its notice should be too"
        );
    }

    #[test]
    fn a_successful_send_retires_a_previous_send_failure() {
        let mut app = test_app();
        app.notify_blocking_cause("queued message failed: boom", NoticeCause::SendFailed);
        submit_message(&mut app, "hello".into()).unwrap();
        assert!(app.notice.is_none(), "the new turn supersedes the old failure");
    }

    #[test]
    fn info_notice_is_muted_and_blocking_is_not() {
        let mut app = test_app();
        app.notify("copied last response");
        let rows = draw_cells(&mut app, 80, 24);
        let y = (0..24usize)
            .find(|y| row_text(&rows, *y).contains("copied last response"))
            .expect("info notice row");
        assert_eq!(rows[y][0].fg, app.theme.muted, "info is muted");
        assert!(row_text(&rows, y).trim_start().starts_with('\u{258e}'), "marker");
    }

    #[test]
    fn no_notice_means_no_row() {
        let mut app = test_app();
        app.notify("temporary");
        let _ = draw_cells(&mut app, 80, 24);
        app.clear_notice();
        let rows = draw_cells(&mut app, 80, 24);
        assert!(notice_line(&rows, 24).is_none(), "no empty notice row");
    }

    #[test]
    fn picker_columns_stay_inside_the_border() {
        // The two-column layout was one cell too wide, so long descriptions
        // pushed past the right border and drew over it.
        for buffer in ["/", "/model ", "/login "] {
            let mut app = test_app();
            app.input.buffer = buffer.into();
            app.input.set_cursor(app.input.buffer.len());
            for w in [64u16, 80, 100] {
                let rows = draw_cells(&mut app, w, 24);
                let (top, bottom) = picker_span(&rows, 24).expect("picker opens");
                // The right border column, taken from the top rule.
                let rule: Vec<char> = row_text(&rows, top).chars().collect();
                let edge = rule.iter().rposition(|c| *c == '╮').expect("top rule");
                for y in (top + 1)..bottom {
                    let line: Vec<char> = row_text(&rows, y).chars().collect();
                    assert_eq!(
                        line.get(edge),
                        Some(&'│'),
                        "row must close on the border at {} cols for {:?}: {:?}",
                        w,
                        buffer,
                        line.iter().collect::<String>()
                    );
                    // Nothing painted outside the box.
                    assert!(
                        line[edge + 1..].iter().all(|c| c.is_whitespace()),
                        "content spilled past the border at {} cols: {:?}",
                        w,
                        line.iter().collect::<String>()
                    );
                }
            }
        }
    }

    #[test]
    fn tail_stays_visible_while_following() {
        let mut app = test_app();
        app.is_home = false;
        for body in wrapping_convo(6) {
            app.conversation.add_message("assistant".into(), body);
        }
        let rows = draw_cells(&mut app, 82, 22);
        let chat = chat_rows(&rows, 22);
        let joined = chat.join("\n");
        assert!(
            joined.contains("nomor 5"),
            "newest answer must be on screen while following:\n{}",
            joined
        );
        assert!(
            joined.contains("- detail dua"),
            "tail of the newest answer must be on screen:\n{}",
            joined
        );
        assert!(app.should_auto_scroll, "follow mode stays on");
    }

    #[test]
    fn tail_stays_visible_after_resize() {
        let mut app = test_app();
        app.is_home = false;
        for body in wrapping_convo(4) {
            app.conversation.add_message("assistant".into(), body);
        }
        // Shrink the terminal: the follow position must be recomputed, not kept.
        let _ = draw_cells(&mut app, 82, 30);
        let rows = draw_cells(&mut app, 46, 14);
        let joined = chat_rows(&rows, 14).join("\n");
        assert!(joined.contains("nomor 3"), "after resize:\n{}", joined);
    }

    #[test]
    fn pgup_pgdn_move_physical_rows() {
        let mut app = test_app();
        app.is_home = false;
        for body in wrapping_convo(6) {
            app.conversation.add_message("assistant".into(), body);
        }
        let rows = draw_cells(&mut app, 82, 22);
        let bottom = chat_rows(&rows, 22);
        // Jump to the top the way the wheel/PageUp arm does.
        app.chat_scroll = 0;
        app.should_auto_scroll = false;
        let rows = draw_cells(&mut app, 82, 22);
        let top = chat_rows(&rows, 22);
        assert!(!top.join("\n").contains("nomor 5"), "scrolled up:\n{}", top.join("\n"));
        // One PageDown = 10 rows, in physical rows this time: the rows that
        // were at the bottom edge move to the top edge, nothing more.
        app.chat_scroll += 10;
        app.should_auto_scroll = false;
        let rows = draw_cells(&mut app, 82, 22);
        let after = chat_rows(&rows, 22);
        let shifted: Vec<String> = top.iter().skip(10).cloned().collect();
        assert_eq!(
            after.iter().take(shifted.len()).cloned().collect::<Vec<_>>(),
            shifted,
            "PageDown must shift the view by exactly 10 rows"
        );
        // Scrolling back to the bottom resumes following.
        app.chat_scroll = usize::MAX;
        let _ = draw_cells(&mut app, 82, 22);
        let rows = draw_cells(&mut app, 82, 22);
        assert!(app.should_auto_scroll, "bottom → follow again");
        assert_eq!(chat_rows(&rows, 22), bottom, "and shows the same tail");
    }

    #[test]
    fn scrolling_up_survives_new_content() {
        let mut app = test_app();
        app.is_home = false;
        for body in wrapping_convo(4) {
            app.conversation.add_message("assistant".into(), body);
        }
        let _ = draw_cells(&mut app, 82, 22);
        app.chat_scroll = 0;
        app.should_auto_scroll = false;
        let held = app.chat_scroll;
        // Streaming while the user reads history must not yank the view down.
        for body in wrapping_convo(3) {
            app.conversation.add_message("assistant".into(), body);
        }
        let _ = draw_cells(&mut app, 82, 22);
        assert!(!app.should_auto_scroll, "still reading history");
        assert_eq!(app.chat_scroll, held, "scroll position kept");
    }

    #[test]
    fn short_transcript_needs_no_scroll() {
        let mut app = test_app();
        app.is_home = false;
        app.conversation.add_message("assistant".into(), "hi".into());
        let rows = draw_cells(&mut app, 40, 10);
        assert_eq!(app.chat_scroll, 0);
        assert!(chat_rows(&rows, 10).join("\n").contains("hi"));
    }

    // --- busy state: the draft stays visible and Enter queues it ---

    /// An app with a live agent turn (event_rx set, never yields a Done).
    fn busy_app() -> App {
        let mut app = test_app();
        app.is_home = false;
        app.conversation.add_message("assistant".into(), "working on it".into());
        let (_tx, rx) = tokio::sync::mpsc::channel(8);
        app.event_rx = Some(rx);
        app.status.tool_status = "processing...".into();
        app
    }

    /// The Enter arm, minus the parts that need a real terminal: takes the
    /// buffer and routes it exactly like `handle_events` does.
    fn press_enter(app: &mut App) {
        // The real entry point — a copy of it is how the picker bug hid.
        super::handle_enter(app, false).unwrap();
    }

    #[test]
    fn busy_prompt_stays_clean_and_the_interrupt_hint_lives_in_the_status_row() {
        let mut app = busy_app();
        app.input.buffer = "draft while busy".into();
        app.input.set_cursor(app.input.buffer.len());
        let rows = draw_cells(&mut app, 60, 16);
        let prompt = prompt_box(&rows, 16);
        assert_eq!(prompt.len(), 5, "{:?}", prompt);
        assert_box_shape(&prompt, "busy prompt");
        assert!(prompt[1].contains("draft while busy"), "draft visible: {:?}", prompt);
        assert!(prompt[1].contains('\u{2588}'), "cursor visible: {:?}", prompt);
        // No instructional text crammed into the box.
        let whole = prompt.join(" ");
        assert!(!whole.contains("Esc to cancel"), "box must stay clean: {:?}", whole);
        // The activity sweep + interrupt hint are on the status row instead.
        let status = status_line(&rows, 16);
        assert!(status.contains("esc interrupt"), "status: {:?}", status);
        assert!(status.contains('\u{25aa}') || status.contains('\u{00b7}'), "sweep: {:?}", status);
    }

    #[test]
    fn enter_while_busy_queues_instead_of_discarding() {
        let mut app = busy_app();
        app.input.buffer = "pertanyaan kedua".into();
        app.input.set_cursor(app.input.buffer.len());
        press_enter(&mut app);
        assert_eq!(app.queued_input, vec!["pertanyaan kedua".to_string()]);
        assert!(app.input.buffer.is_empty(), "draft moves into the queue");
        // Nothing was sent to the model yet.
        assert_eq!(app.conversation.messages.len(), 1);
        assert_eq!(app.status.tool_status, "processing...", "run untouched");
    }

    #[test]
    fn queue_drains_in_order_when_the_run_ends() {
        let mut app = busy_app();
        for text in ["satu", "dua", "tiga"] {
            app.input.buffer = text.into();
            app.input.set_cursor(app.input.buffer.len());
            press_enter(&mut app);
        }
        assert_eq!(app.queued_input.len(), 3);
        // Finish the run the way `Done` does, then let the drain fire.
        app.event_rx = None;
        app.status.tool_status = "idle".into();
        app.drain_queued_input();
        let sent: Vec<String> = app
            .conversation
            .messages
            .iter()
            .filter(|m| m.role == "user")
            .map(|m| m.content.clone().unwrap_or_default())
            .collect();
        // One turn per drain; this fixture has no API key so the turn aborts
        // immediately and the rest wait for the next drain.
        assert_eq!(sent, vec!["satu".to_string()], "FIFO order");
        assert_eq!(app.queued_input, vec!["dua".to_string(), "tiga".to_string()]);
    }

    #[test]
    fn queue_is_bounded() {
        let mut app = busy_app();
        for i in 0..(super::MAX_QUEUED + 3) {
            app.input.buffer = format!("m{i}");
            app.input.set_cursor(app.input.buffer.len());
            press_enter(&mut app);
        }
        assert_eq!(app.queued_input.len(), super::MAX_QUEUED);
        assert_eq!(app.queued_input.last().unwrap(), "m10", "newest kept");
        assert_eq!(app.queued_input.first().unwrap(), "m3", "oldest dropped");
    }

    #[test]
    fn failed_turn_does_not_stall_the_queue() {
        // No API key in this fixture, so every queued turn refuses to start.
        // The drain must still empty the queue instead of wedging on the first.
        let mut app = busy_app();
        for text in ["satu", "dua", "tiga"] {
            app.input.buffer = text.into();
            app.input.set_cursor(app.input.buffer.len());
            press_enter(&mut app);
        }
        let (_tx, rx) = tokio::sync::mpsc::channel(8);
        app.event_rx = Some(rx);
        app.handle_stream().unwrap();
        app.event_rx = None;
        // One "run finished" tick.
        app.drain_queued_input();
        let mut budget = app.queued_input.len();
        while budget > 0 && app.event_rx.is_none() && !app.queued_input.is_empty() {
            budget -= 1;
            app.drain_queued_input();
        }
        assert!(app.queued_input.is_empty(), "queue must not wedge");
        let sent: Vec<String> = app
            .conversation
            .messages
            .iter()
            .filter(|m| m.role == "user")
            .map(|m| m.content.clone().unwrap_or_default())
            .collect();
        assert_eq!(sent, vec!["satu", "dua", "tiga"], "all sent, in order");
    }

    #[test]
    fn cancel_drops_the_queue() {
        let mut app = busy_app();
        app.input.buffer = "never sent".into();
        app.input.set_cursor(app.input.buffer.len());
        press_enter(&mut app);
        assert_eq!(app.queued_input.len(), 1);
        // Esc cancels the run and must not let the queue fire afterwards.
        app.cancelled.store(true, Ordering::Relaxed);
        app.event_rx = None;
        app.queued_input.clear();
        app.drain_queued_input();
        assert!(app.queued_input.is_empty());
        assert_eq!(app.conversation.messages.len(), 1, "no user message was sent");
    }

    #[test]
    fn status_row_shows_queue_depth() {
        let mut app = busy_app();
        app.queued_input = vec!["a".into(), "b".into()];
        let rows = draw_cells(&mut app, 80, 16);
        let status = status_line(&rows, 16);
        assert!(status.contains("+2 queued"), "status: {}", status);
    }

    // --- input panel: the cursor stays visible on a long wrapped line ---

    #[test]
    fn cursor_stays_visible_on_a_long_single_line() {
        let mut app = test_app();
        app.input.buffer = "q".repeat(200);
        app.input.set_cursor(app.input.buffer.len());
        let rows = draw_cells(&mut app, 40, 24);
        let prompt = prompt_box(&rows, 24);
        assert!(
            prompt.iter().any(|l| l.contains('\u{2588}')),
            "block cursor must be inside the prompt box:\n{}",
            prompt.join("\n")
        );
    }

    #[test]
    fn cursor_visible_at_every_column_of_a_wrapped_line() {
        // Width 40 → inner 39. Sweep the cursor across wrap boundaries.
        for cursor in [0usize, 1, 38, 39, 40, 77, 78, 79, 120, 200] {
            let mut app = test_app();
            app.input.buffer = "q".repeat(200);
            app.input.set_cursor(cursor);
            let rows = draw_cells(&mut app, 40, 24);
            let prompt = prompt_box(&rows, 24);
            assert!(
                prompt.iter().any(|l| l.contains('\u{2588}')),
                "cursor at {} not rendered:\n{}",
                cursor,
                prompt.join("\n")
            );
        }
    }

    #[test]
    fn cursor_visible_in_a_tall_multiline_draft() {
        let mut app = test_app();
        app.input.buffer = "satu\ndua\ntiga\nempat\nlima\nenam\ntujuh".into();
        app.input.set_cursor(app.input.buffer.len());
        let rows = draw_cells(&mut app, 40, 24);
        let prompt = prompt_box(&rows, 24);
        let joined = prompt.join("\n");
        assert!(joined.contains('\u{2588}'), "cursor must show:\n{}", joined);
        assert!(joined.contains("tujuh"), "and the text it sits on:\n{}", joined);
    }

    // --- approval modal: the payload must be reviewable ---

    fn approve(name: &str, args: serde_json::Value) -> App {
        let mut app = test_app();
        app.is_home = false;
        app.conversation.add_message("assistant".into(), "working".into());
        app.pending_approval = Some(crate::agent::permissions::PendingTool {
            id: "1".into(),
            name: name.into(),
            args,
        });
        app
    }

    /// Rows inside the modal's rounded border.
    fn modal_rows(rows: &[Vec<ratatui::buffer::Cell>], h: u16) -> Vec<String> {
        let top = (0..h as usize).find(|&y| row_text(rows, y).contains('╭'));
        let bottom = (0..h as usize).find(|&y| row_text(rows, y).contains('╰'));
        match (top, bottom) {
            (Some(t), Some(b)) => (t..=b).map(|y| row_text(rows, y)).collect(),
            _ => Vec::new(),
        }
    }

    #[test]
    fn modal_shows_the_whole_bash_command() {
        let cmd = "cargo test --all-features && rm -rf build && git commit -am 'a fairly long commit message here'";
        let mut app = approve("bash", serde_json::json!({ "command": cmd }));
        let rows = draw_cells(&mut app, 100, 24);
        let modal = modal_rows(&rows, 24);
        let joined = modal.join("\n");
        assert!(!modal.is_empty(), "modal must render");
        assert!(!joined.contains("{\"command\""), "raw JSON must be gone:\n{}", joined);
        // Every word of the command has to be on screen somewhere.
        for word in cmd.split_whitespace() {
            let needle: String = word.chars().take(6).collect();
            assert!(joined.contains(&needle), "missing {:?} in:\n{}", word, joined);
        }
        assert!(joined.contains('y') && joined.contains("once"), "keys must show");
        assert!(joined.contains("always") && joined.contains("deny"));
    }

    #[test]
    fn modal_write_shows_path_and_content() {
        let app = approve(
            "write",
            serde_json::json!({ "path": "src/main.rs", "content": "fn main() {\n    println!(\"hi\");\n}\n" }),
        );
        let mut app = app;
        let rows = draw_cells(&mut app, 100, 24);
        let joined = modal_rows(&rows, 24).join("\n");
        assert!(joined.contains("src/main.rs"), "path:\n{}", joined);
        assert!(joined.contains("fn main()"), "content:\n{}", joined);
        assert!(joined.contains("3 lines"), "line count:\n{}", joined);
        assert!(!joined.contains("\"content\""), "no raw JSON");
    }

    #[test]
    fn modal_edit_reads_as_a_diff() {
        let mut app = approve(
            "edit",
            serde_json::json!({
                "path": "src/app.rs",
                "old_string": "let ctx = 0;",
                "new_string": "let ctx = compute_context();",
            }),
        );
        let rows = draw_cells(&mut app, 100, 24);
        let joined = modal_rows(&rows, 24).join("\n");
        assert!(joined.contains("src/app.rs"), "path:\n{}", joined);
        assert!(joined.contains("- let ctx = 0;"), "removed line:\n{}", joined);
        assert!(joined.contains("+ let ctx = compute_context();"), "added line:\n{}", joined);
    }

    #[test]
    fn modal_caps_huge_payload_and_says_so() {
        let big: String = (0..500).map(|i| format!("line {i}\n")).collect();
        let mut app = approve("write", serde_json::json!({ "path": "big.rs", "content": big }));
        let rows = draw_cells(&mut app, 100, 24);
        let modal = modal_rows(&rows, 24);
        let joined = modal.join("\n");
        assert!(joined.contains("writes 500 lines"), "total:\n{}", joined);
        assert!(joined.contains("more lines"), "must admit the truncation:\n{}", joined);
        assert!(modal.len() <= 24, "modal must fit the terminal, got {}", modal.len());
    }

    #[test]
    fn modal_fits_a_narrow_terminal() {
        let cmd = "echo a-very-long-single-token-that-cannot-wrap-anywhere-at-all && echo short";
        let mut app = approve("bash", serde_json::json!({ "command": cmd }));
        let rows = draw_cells(&mut app, 40, 16);
        let modal = modal_rows(&rows, 16);
        assert!(!modal.is_empty(), "modal must render at 40 cols");
        for row in &modal {
            assert!(
                row.chars().count() <= 40,
                "row overflows 40 cols ({}): {:?}",
                row.chars().count(),
                row
            );
        }
        assert!(modal.join("\n").contains("deny"), "keys must survive");
    }

    #[test]
    fn modal_has_no_blank_rows_inside() {
        // The old modal was a fixed 9 rows with three of them empty.
        let mut app = approve("bash", serde_json::json!({ "command": "ls" }));
        let rows = draw_cells(&mut app, 100, 24);
        let modal = modal_rows(&rows, 24);
        assert_eq!(modal.len(), 6, "border+name+cmd+blank+keys+border: {:?}", modal);
    }

    #[test]
    fn modal_falls_back_to_key_values_for_unknown_tools() {
        let mut app = approve(
            "delegate",
            serde_json::json!({ "task": "review the diff carefully" }),
        );
        let rows = draw_cells(&mut app, 100, 24);
        let joined = modal_rows(&rows, 24).join("\n");
        assert!(joined.contains("task"), "key shown:\n{}", joined);
        assert!(joined.contains("review the diff carefully"), "value shown:\n{}", joined);
    }

    #[test]
    fn picker_never_covers_the_input() {
        // Small terminals used to get a 10-row picker pasted over the input box.
        for (w, h) in [(20u16, 10u16), (24, 12), (40, 9), (60, 8), (80, 24)] {
            let mut app = test_app();
            app.input.buffer = "/model ".into();
            app.input.set_cursor(app.input.buffer.len());
            let rows = draw_cells(&mut app, w, h);
            let box_top = (0..h as usize)
                .find(|&y| row_text(&rows, y).starts_with('\u{258c}'))
                .unwrap_or_else(|| panic!("prompt box must exist at {}x{}", w, h));
            match picker_span(&rows, h) {
                None => panic!("picker must open at {}x{}", w, h),
                Some((_, bottom)) => assert!(
                    bottom < box_top,
                    "picker bottom {} overlaps the prompt box (top {}) at {}x{}",
                    bottom,
                    box_top,
                    w,
                    h
                ),
            }
            // The draft row keeps its own content.
            let draft = (box_top..box_top + 5)
                .map(|y| row_text(&rows, y))
                .find(|l| l.contains("/model"))
                .unwrap_or_default();
            assert!(
                draft.contains("/model"),
                "prompt must still show the draft at {}x{}: {:?}",
                w,
                h,
                draft
            );
        }
    }

    #[test]
    fn picker_items_never_wrap_to_two_rows() {
        // A too-wide cell used to wrap, doubling every entry and pushing the
        // list off the box.
        for (w, h) in [(20u16, 10u16), (26, 14), (40, 20), (100, 24)] {
            let mut app = test_app();
            app.input.buffer = "/login ".into();
            app.input.set_cursor(app.input.buffer.len());
            let rows = draw_cells(&mut app, w, h);
            let (top, bottom) = picker_span(&rows, h).expect("picker opens");
            let item_rows = (top + 1)..bottom;
            assert!(!item_rows.is_empty(), "{}x{}: no items", w, h);
            for y in item_rows.clone() {
                let line = row_text(&rows, y);
                assert!(
                    !line.trim().is_empty(),
                    "blank item row at {}x{} — an entry wrapped:\n{}",
                    w,
                    h,
                    (top..=bottom).map(|r| row_text(&rows, r)).collect::<Vec<_>>().join("\n")
                );
                assert!(
                    line.chars().take_while(|c| *c == ' ').count() < 3,
                    "wrapped continuation at {}x{}: {:?}",
                    w,
                    h,
                    line
                );
            }
            // One highlighted row per item, no more.
            let accent = app.theme.accent;
            let highlighted = (top..=bottom)
                .filter(|y| rows[*y].iter().any(|c| c.bg == accent))
                .count();
            assert_eq!(highlighted, 1, "{}x{}: exactly one highlight", w, h);
        }
    }

    #[test]
    fn picker_label_readable_on_light_background() {
        let mut app = test_app();
        app.set_theme("light");
        app.input.buffer = "/login ".into();
        app.input.set_cursor(app.input.buffer.len());
        let w = 80u16;
        let h = 24u16;
        let rows = draw_cells(&mut app, w, h);
        let (top, bottom) = picker_span(&rows, h).expect("picker opens");
        // Non-selected label cells must not be hardcoded white.
        let mut checked = 0;
        for row in rows.iter().take(bottom).skip(top + 1) {
            for cell in row {
                let sym = cell.symbol();
                if sym.chars().next().is_some_and(|c| c.is_ascii_alphanumeric()) {
                    assert_ne!(
                        cell.fg,
                        ratatui::style::Color::White,
                        "white text on a light terminal in row {:?}",
                        row_text(&rows, top + 1)
                    );
                    checked += 1;
                }
            }
        }
        assert!(checked > 5, "expected to inspect label cells, saw {}", checked);
    }

    #[test]
    fn empty_prompt_is_only_the_cursor() {
        let mut app = test_app();
        let rows = draw_cells(&mut app, 80, 24);
        let prompt = prompt_box(&rows, 24);
        assert_eq!(prompt.len(), 5, "{:?}", prompt);
        assert_box_shape(&prompt, "empty prompt");
        // No affordance hints anywhere in the input area.
        let whole = prompt.join(" ");
        for hint in ["message", "commands", "@ files", "Esc cancel"] {
            assert!(!whole.contains(hint), "prompt must not hint {:?}: {:?}", hint, whole);
        }
        // The cursor is there and it is the only thing on its row.
        let text = prompt[1].trim_start_matches('\u{258c}').to_string();
        assert_eq!(text.trim(), "\u{2588}", "just the cursor: {:?}", text);
        // Nor on the welcome screen.
        assert!(!chat_rows(&rows, 24).join(" ").contains("Type a message"));
    }

    #[test]
    fn audit_every_picker_opens() {
        // "/resume " has no rows without saved sessions — correct, nothing to pick.
        for buffer in ["/", "/model ", "/login ", "/theme ", "/approve "] {
            let mut app = test_app();
            app.input.buffer = buffer.into();
            app.input.cursor_pos = app.input.buffer.len();
            let rows = draw_cells(&mut app, 80, 24);
            let accent = app.theme.accent;
            assert_eq!(
                accent_rows(&rows, accent).len(),
                1,
                "exactly one highlighted row for {:?}",
                buffer
            );
        }
    }

    #[test]
    fn audit_navigate_then_enter_applies_highlight() {
        let mut app = test_app();
        app.auth.set("nvidia", "nvapi-test-key");
        app.input.buffer = "/model ".into();
        app.input.cursor_pos = app.input.buffer.len();
        // Arrow down twice, exactly like the Up/Down arm does.
        assert!(super::handle_picker_nav(&mut app, 1));
        assert!(super::handle_picker_nav(&mut app, 1));
        // The highlighted row must be the 3rd model entry.
        let rows = draw_cells(&mut app, 100, 30);
        let accent = app.theme.accent;
        let sel = accent_rows(&rows, accent);
        assert_eq!(sel.len(), 1);
        let text: String = rows[sel[0]].iter().map(|c| c.symbol().to_string()).collect();
        let entries = model_entries(&app, "");
        let (ep, em, _) = entries[2].clone();
        assert!(text.contains(&ep), "highlight shows 3rd entry, got: {}", text);
        // Enter goes through the real entry point, not a copy of the key arm.
        super::handle_enter(&mut app, false).unwrap();
        assert_eq!(app.provider_name, ep);
        assert_eq!(app.current_model, em);
    }

    /// Regression: `reset_picker` used to run *before* dispatch, so every
    /// arrow-key selection was thrown away and Enter silently took the first
    /// row. Picking `nvidia` in `/login` stored the key under `9router`.
    #[test]
    fn arrow_selection_survives_enter_in_every_picker() {
        // /login: the provider the user highlighted must be the one that opens
        // the key prompt.
        let mut app = test_app();
        app.input.buffer = "/login ".into();
        app.input.set_cursor(app.input.buffer.len());
        let entries = provider_entries(&app, "");
        let target = entries
            .iter()
            .position(|(pid, _, _)| pid == "nvidia")
            .expect("nvidia must be listed");
        for _ in 0..target {
            assert!(super::handle_picker_nav(&mut app, 1));
        }
        super::handle_enter(&mut app, false).unwrap();
        assert_eq!(
            app.pending_login.as_deref(),
            Some("nvidia"),
            "arrow-selected provider must win, not the first keyless one"
        );

        // /model: same shape, different picker.
        let mut app = test_app();
        app.input.buffer = "/model ".into();
        app.input.set_cursor(app.input.buffer.len());
        for _ in 0..3 {
            assert!(super::handle_picker_nav(&mut app, 1));
        }
        let entries = model_entries(&app, "");
        let (ep, em, _) = entries[3].clone();
        super::handle_enter(&mut app, false).unwrap();
        assert_eq!(app.provider_name, ep);
        assert_eq!(app.current_model, em);

        // /theme: a choice picker, to prove it is not login/model specific.
        let mut app = test_app();
        app.input.buffer = "/theme ".into();
        app.input.set_cursor(app.input.buffer.len());
        for _ in 0..2 {
            assert!(super::handle_picker_nav(&mut app, 1));
        }
        let pick = choice_entries(&app, &ChoiceMode::Theme, "")[2].0.clone();
        super::handle_enter(&mut app, false).unwrap();
        assert_eq!(app.theme.name, pick);
    }

    /// Regression: pressing Up from the top must wrap to the *last* row, not
    /// fall through to input history.
    #[test]
    fn arrow_up_wraps_to_last_provider() {
        let mut app = test_app();
        app.input.buffer = "/login ".into();
        app.input.set_cursor(app.input.buffer.len());
        assert!(super::handle_picker_nav(&mut app, -1));
        let last = provider_entries(&app, "").len() - 1;
        assert_eq!(app.picker_idx, last);
        super::handle_enter(&mut app, false).unwrap();
        let expected = provider_entries(&app, "")[last].0.clone();
        assert_eq!(app.pending_login.as_deref(), Some(expected.as_str()));
    }

    /// Regression: `/model ` + Enter with no arrow press must land on the
    /// *active* model, not the alphabetically first one — the preselect used to
    /// be wiped by the same reset.
    #[test]
    fn model_picker_preselects_the_active_model() {
        let mut app = test_app();
        app.auth.set("nvidia", "nvapi-test-key");
        super::handle_slash(&mut app, "/model nvidia").unwrap();
        app.picker_idx = 0;
        app.picker_navigated = false;
        app.input.buffer = "/model ".into();
        app.input.set_cursor(app.input.buffer.len());
        // Same as pressing Enter on the freshly opened picker.
        let pos = model_entries(&app, "")
            .iter()
            .position(|(p, m, _)| p == &app.provider_name && m == &app.current_model)
            .expect("active model must be listed");
        app.picker_idx = pos;
        super::handle_enter(&mut app, false).unwrap();
        assert_eq!(app.provider_name, "nvidia", "stays on the active provider");
        assert_eq!(app.current_model, "openai/gpt-oss-20b");
    }

    #[test]
    fn discovered_models_merge_first_without_dupes() {
        let _guard = HOME_LOCK.lock().unwrap();
        let app = test_app_locked();
        // Inject discovery results via temp-HOME cache file path (isolated).
        let tmp = std::env::temp_dir().join(format!("barong-model-repro-{}", std::process::id()));
        let orig = std::env::var("HOME").ok();
        // SAFETY: same as test_app — isolated temp dir, disjoint from other tests.
        unsafe { std::env::set_var("HOME", &tmp); }
        app.discovered.put("nvidia", vec!["openai/gpt-oss-20b".into(), "live/custom-1".into()]);
        match orig {
            Some(o) => unsafe { std::env::set_var("HOME", o) },
            None => unsafe { std::env::remove_var("HOME") },
        }
        let items = model_entries(&app, "gpt-oss");
        assert_eq!(items.len(), 1, "live + catalog dupe must collapse, got: {:?}", items);
        assert_eq!(items[0].1, "openai/gpt-oss-20b");
        let live = model_entries(&app, "live/custom");
        assert_eq!(live.len(), 1);
        assert_eq!(live[0], ("nvidia".to_string(), "live/custom-1".to_string(), false));
    }

    #[test]
    fn model_empty_arg_confirms_highlight() {
        let mut app = test_app();
        // Picker open, no navigation: highlight is items[0].
        handle_slash(&mut app, "/model ").unwrap();
        assert_eq!(app.provider_name, "ollama"); // keyless local is ready first
        assert_eq!(app.current_model, "llama3.1:8b");
    }

    #[test]
    fn provider_switch_updates_status_and_endpoint() {
        let mut app = test_app();
        app.auth.set("nvidia", "nvapi-test-key");
        handle_slash(&mut app, "/model nvidia").unwrap();
        assert_eq!(app.status.llm_provider, "openai"); // nvidia speaks OpenAI API
        let ep = app.endpoint.lock().unwrap();
        assert_eq!(ep.model, "openai/gpt-oss-20b");
        assert_eq!(ep.api_key, "nvapi-test-key");
        assert_eq!(ep.base_url, "https://integrate.api.nvidia.com/v1");
    }

    #[test]
    fn login_save_auto_switches_to_provider() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let mut app = test_app();
        // Active provider is openai (no key); we save a key for nvidia.
        assert_eq!(app.provider_name, "openai");
        app.pending_login = Some("nvidia".into());
        for c in "nvapi-test-key".chars() {
            let ev = KeyEvent::new(KeyCode::Char(c), KeyModifiers::empty());
            super::handle_login_key(&mut app, ev).unwrap();
        }
        let ev = KeyEvent::new(KeyCode::Enter, KeyModifiers::empty());
        super::handle_login_key(&mut app, ev).unwrap();
        assert_eq!(app.auth.get("nvidia").as_deref(), Some("nvapi-test-key"));
        // No separate `/model` step needed: session follows the login.
        assert_eq!(app.provider_name, "nvidia");
        assert_eq!(app.current_model, "openai/gpt-oss-20b");
        let last = app.conversation.messages.last().and_then(|m| m.content.clone()).unwrap_or_default();
        assert!(last.contains("Switched to `nvidia/openai/gpt-oss-20b`"), "got: {}", last);
    }

    #[test]
    fn bare_enter_exact_command_submits() {
        // `/log` is a substring of /login and /logout, yet exact text must run.
        assert_eq!(bare_enter("/log", 0, false), Some(BareEnter::Submit));
        assert_eq!(bare_enter("/tree", 0, false), Some(BareEnter::Submit));
        assert_eq!(bare_enter("/quit", 0, false), Some(BareEnter::Submit));
    }

    #[test]
    fn bare_enter_model_and_login_open_pickers() {
        assert_eq!(bare_enter("/model", 0, false), Some(BareEnter::Complete("/model ".into())));
        assert_eq!(bare_enter("/login", 0, false), Some(BareEnter::Complete("/login ".into())));
        // Removed `/models` alias: unknown text falls through.
        assert_eq!(bare_enter("/models", 0, false), None);
    }

    #[test]
    fn bare_enter_all_choice_commands_open_pickers() {
        for cmd in ["/theme", "/approve", "/compact", "/allow", "/resume", "/logout"] {
            assert_eq!(
                bare_enter(cmd, 0, false),
                Some(BareEnter::Complete(format!("{} ", cmd))),
                "{} must open its picker",
                cmd
            );
        }
        // Free-form commands still submit.
        assert_eq!(bare_enter("/branch", 0, false), Some(BareEnter::Submit));
        assert_eq!(bare_enter("/tree", 0, false), Some(BareEnter::Submit));
    }

    #[test]
    fn theme_picker_applies_highlight() {
        let mut app = test_app();
        app.set_theme("light");
        assert_eq!(app.theme.name, "light");
        app.picker_navigated = true;
        app.picker_idx = 2; // dark, light, barong
        handle_slash(&mut app, "/theme ").unwrap();
        assert_eq!(app.theme.name, "barong");
    }

    #[test]
    fn resume_picker_restores_endpoint_too() {
        let mut app = test_app();
        app.auth.set("nvidia", "nvapi-test-key");
        app.conversation.add_message("user".into(), "hello nvidia".into());
        handle_slash(&mut app, "/model nvidia").unwrap();
        let saved_id = app.session_id.clone().expect("saved");
        // Wipe to a fresh state on another provider (poked directly to avoid
        // saving a new session that would shadow the nvidia one), then resume.
        handle_slash(&mut app, "/clear").unwrap();
        app.provider_name = "openai".into();
        app.current_model = "gpt-4o".into();
        assert_eq!(app.provider_name, "openai");
        app.picker_navigated = true;
        app.picker_idx = 0; // most recent first
        handle_slash(&mut app, "/resume ").unwrap();
        assert_eq!(app.session_id.as_deref(), Some(saved_id.as_str()));
        assert_eq!(app.provider_name, "nvidia");
        assert!(app.conversation.messages.iter().any(|m| m.content.as_deref().unwrap_or("").contains("hello nvidia")));
    }

    #[test]
    fn bare_enter_prefix_completes_first_match() {
        assert_eq!(bare_enter("/lo", 0, false), Some(BareEnter::Complete("/login ".into())));
        assert_eq!(bare_enter("/mod", 0, false), Some(BareEnter::Complete("/model ".into())));
    }

    #[test]
    fn bare_enter_highlight_completes_selection() {
        // "/log" palette order: /login, /logout, /log.
        assert_eq!(bare_enter("/log", 1, true), Some(BareEnter::Complete("/logout ".into())));
        assert_eq!(bare_enter("/log", 2, true), Some(BareEnter::Complete("/log ".into())));
    }

    #[test]
    fn bare_enter_non_command_untouched() {
        assert_eq!(bare_enter("hello", 0, false), None);
        assert_eq!(bare_enter("/model nvidia", 0, false), None);
        assert_eq!(bare_enter("/zzz", 0, false), None);
    }

    #[test]
    fn bare_enter_login_opens_picker_not_submit() {
        // No trim-trap: "/login " (picker open) must fall through to handle_slash.
        assert_eq!(bare_enter("/login", 0, false), Some(BareEnter::Complete("/login ".into())));
        assert_eq!(bare_enter("/login ", 0, false), None);
        assert_eq!(bare_enter("/login ", 2, true), None);
    }

    #[test]
    fn login_empty_enter_picks_first_keyless_provider() {
        let mut app = test_app();
        // Drive the real double-Enter flow: the first Enter opens the picker
        // (and preselects), the second confirms. Calling handle_slash directly
        // would skip the preselect and test a fiction.
        app.input.buffer = "/login".into();
        app.input.set_cursor(app.input.buffer.len());
        // First Enter opens the picker; ollama is always ready (dummy) so the
        // preselect must skip it and land on 9router.
        super::handle_enter(&mut app, false).unwrap();
        assert!(
            app.pending_login.is_none(),
            "first Enter only opens the picker"
        );
        assert_eq!(app.input.buffer, "/login ");
        assert_eq!(
            provider_entries(&app, "")[app.picker_idx].0,
            "9router",
            "preselect must be the first provider still missing a key"
        );
        // Second Enter confirms the highlighted row.
        super::handle_enter(&mut app, false).unwrap();
        assert_eq!(app.pending_login.as_deref(), Some("9router"));
    }

    /// Regression: the highlight must never point at one row while Enter picks
    /// another. Opening `/login` preselects the first provider needing a key,
    /// so a plain Enter confirms exactly the row that is drawn.
    #[test]
    fn login_highlight_matches_what_enter_picks() {
        let mut app = test_app();
        app.input.buffer = "/login".into();
        app.input.set_cursor(app.input.buffer.len());
        // First Enter: open the picker through the real entry point.
        super::handle_enter(&mut app, false).unwrap();
        assert!(
            app.pending_login.is_none(),
            "first Enter must not open the prompt"
        );

        // What the screen highlights vs what the key prompt asks for.
        let rows = draw_cells(&mut app, 100, 24);
        let highlighted = highlighted_row_text(&rows, app.theme.accent);
        super::handle_enter(&mut app, false).unwrap();
        let asked = app.pending_login.clone().expect("prompt must open");
        assert!(
            highlighted.contains(&asked),
            "highlight {:?} must be the provider the prompt asks for {:?}",
            highlighted,
            asked
        );
    }

    /// Text of the picker row drawn with the accent background — what the user
    /// actually sees as "selected".
    fn highlighted_row_text(
        rows: &[Vec<ratatui::buffer::Cell>],
        accent: ratatui::style::Color,
    ) -> String {
        let hits = accent_rows(rows, accent);
        match hits.first() {
            Some(&y) => row_text(rows, y).trim().to_string(),
            None => String::new(),
        }
    }

    #[test]
    fn model_single_form_label_applies_full_id() {
        // Display shows `nvidia/llama-3.1-nemotron-70b-instruct` (no doubling);
        // pasting it back must apply the full catalog id, not dump matches.
        let mut app = test_app();
        app.auth.set("nvidia", "nvapi-test-key");
        handle_slash(&mut app, "/model nvidia/llama-3.1-nemotron-70b-instruct").unwrap();
        assert_eq!(app.provider_name, "nvidia");
        assert_eq!(app.current_model, "nvidia/llama-3.1-nemotron-70b-instruct");
    }

    #[test]
    fn model_doubled_form_still_applies() {
        // Old doubled display `nvidia/nvidia/...` (copy-paste) must keep working.
        let mut app = test_app();
        app.auth.set("nvidia", "nvapi-test-key");
        handle_slash(&mut app, "/model nvidia/nvidia/llama-3.1-nemotron-70b-instruct").unwrap();
        assert_eq!(app.provider_name, "nvidia");
        assert_eq!(app.current_model, "nvidia/llama-3.1-nemotron-70b-instruct");
    }

    #[test]
    fn models_alias_is_gone_unknown_command() {
        let mut app = test_app();
        app.auth.set("nvidia", "nvapi-test-key");
        handle_slash(&mut app, "/models nvidia").unwrap();
        // Alias removed: provider must NOT switch.
        assert_eq!(app.provider_name, "openai");
        let last = app.conversation.messages.last().and_then(|m| m.content.clone()).unwrap_or_default();
        assert!(last.contains("Unknown command"), "got: {}", last);
    }

    #[test]
    fn model_navigated_applies_highlight() {
        let mut app = test_app();
        app.auth.set("nvidia", "nvapi-test-key");
        app.picker_navigated = true;
        app.picker_idx = 0;
        handle_slash(&mut app, "/model nvid").unwrap();
        assert_eq!(app.provider_name, "nvidia", "highlighted entry must win");
    }

    // --- multibyte input: cursor_pos is a byte offset, never a char count ---

    #[test]
    fn cursor_clamps_and_snaps_to_boundary() {
        let mut inp = InputState::new();
        inp.buffer = "é".into();
        inp.cursor_pos = 99;
        assert_eq!(inp.cursor(), 2, "past the end clamps to len");
        inp.cursor_pos = 1;
        assert_eq!(inp.cursor(), 0, "mid-char snaps down");
        inp.cursor_pos = 2;
        assert_eq!(inp.cursor(), 2, "already a boundary: unchanged");
    }

    #[test]
    fn multibyte_typing_and_deleting() {
        let mut inp = InputState::new();
        for c in "café".chars() {
            inp.insert_char(c);
        }
        assert_eq!(inp.buffer, "café");
        assert_eq!(inp.cursor(), 5, "byte offset, not char count");
        inp.insert_char('!');
        assert_eq!(inp.buffer, "café!");
        inp.backspace();
        assert_eq!(inp.buffer, "café");
        assert_eq!(inp.cursor(), 5);
        // One Left crosses the whole 2-byte char, never a half-byte.
        inp.move_left();
        assert_eq!(inp.cursor(), 3);
        assert!(inp.buffer.is_char_boundary(inp.cursor()));
        inp.move_right();
        assert_eq!(inp.cursor(), 5);
        assert_eq!(inp.cursor(), inp.buffer.len());
    }

    #[test]
    fn multibyte_kill_commands() {
        let mut inp = InputState::new();
        inp.buffer = "αβγ delta".into();
        inp.set_cursor(inp.buffer.len());
        inp.delete_word_before();
        assert_eq!(inp.buffer, "αβγ ");
        inp.move_cursor_end();
        inp.backspace();
        assert_eq!(inp.buffer, "αβγ");
        inp.set_cursor(2);
        inp.delete();
        assert_eq!(inp.buffer, "αγ", "Delete eats the char under the cursor");
        inp.set_cursor(4);
        inp.delete_before_cursor();
        assert_eq!(inp.buffer, "", "Ctrl+U drops everything before the cursor");
        inp.buffer = "αβγ".into();
        inp.set_cursor(2);
        inp.delete_to_cursor();
        assert_eq!(inp.buffer, "α", "Ctrl+K drops from the cursor on");
        inp.move_cursor_home();
        inp.delete();
        assert_eq!(inp.buffer, "", "Delete at offset 0 eats the first char");
        inp.move_cursor_end();
        inp.delete();
        assert_eq!(inp.buffer, "", "Delete at the end is a no-op");
    }

    #[test]
    fn multibyte_word_jump() {
        let mut inp = InputState::new();
        inp.buffer = "αβγ δεζ".into();
        inp.set_cursor(inp.buffer.len());
        assert_eq!(inp.word_start_before(), 7, "start of δεζ (byte 7)");
        assert_eq!(inp.word_end_after(), inp.buffer.len());
        inp.set_cursor(6);
        assert_eq!(inp.word_start_before(), 0);
        assert_eq!(inp.word_end_after(), 7);
    }

    #[test]
    fn multibyte_buffer_renders_cursor_without_panic() {
        let mut app = test_app();
        // The exact state the old char-index arithmetic produced after typing
        // "café": 4 chars typed, 5 bytes. Rendering used to panic here.
        app.input.buffer = "caf\u{e9}plain".into();
        app.input.cursor_pos = 4;
        let rows = draw_cells(&mut app, 60, 24);
        let prompt = prompt_box(&rows, 24).join("\n");
        assert!(prompt.contains("caf"), "got: {}", prompt);
        assert!(
            prompt.contains('\u{2588}'),
            "block cursor must render, got: {}",
            prompt
        );
    }

    #[test]
    fn mention_expansion_truncates_multibyte_file() {
        // Byte 8000 lands inside the 2-byte 'é' -> a fixed [&..8000] panicked.
        let dir = std::env::temp_dir().join(format!("barong-mb-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut big = "a".repeat(7999);
        big.push('é');
        big.push_str(&"b".repeat(200));
        std::fs::write(dir.join("big.txt"), &big).unwrap();
        let (out, missing) = expand_mentions("@big.txt", &dir);
        assert!(missing.is_empty(), "missing: {:?}", missing);
        assert!(out.contains("[truncated]"), "must cap the file");
        assert!(out.contains("aaa"), "must keep the head");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

