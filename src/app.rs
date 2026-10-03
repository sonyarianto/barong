use crate::agent::r#loop::start_agent_loop_with_limit;
use crate::agent::conversation::Conversation;
use crate::agent::llm::{LLMProvider, OpenAIProvider, AnthropicProvider, StreamEvent, ProviderKind};
use crate::agent::permissions::{Decision, PendingTool, PermissionGate};
use crate::agent::models::{self, Discovered};
use crate::auth::{resolve_api_key, AuthStore};
use crate::config::Config;
use crate::mcp::{McpServer, McpToolAdapter};
use crate::session::SessionManager;
use crate::tui::input::InputState;
use crate::tui::status::StatusBar;
use crate::tui::theme::{self, Theme};
use crate::tools::ToolRegistry;
use crate::workspace::WorkspaceContext;
use anyhow::Result;
use ratatui::Frame;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::mpsc;

pub struct App {
    pub conversation: Conversation,
    pub input: InputState,
    pub workspace: WorkspaceContext,
    pub status: StatusBar,
    pub should_quit: bool,
    pub tool_registry: Arc<ToolRegistry>,
    pub config: Config,
    pub session_manager: SessionManager,
    pub session_id: Option<String>,
    pub mcp_servers: Vec<std::sync::Arc<std::sync::Mutex<McpServer>>>,
    pub chat_scroll: usize,
    pub should_auto_scroll: bool,
    /// Messages typed while the agent was working. `Enter` queues instead of
    /// discarding the draft; the queue is drained when the run finishes.
    pub queued_input: Vec<String>,
    pub event_rx: Option<mpsc::Receiver<StreamEvent>>,
    pub streaming_text: String,
    pub provider: ProviderKind,
    pub provider_name: String,
    pub base_url: String,
    pub endpoint: std::sync::Arc<std::sync::Mutex<crate::tools::delegate::ActiveEndpoint>>,
    pub current_model: String,
    pub cancelled: Arc<AtomicBool>,
    pub is_home: bool,
    // UX state (minimal)
    /// Single picker state for every overlay (commands, models, login,
    /// choices). One interaction model everywhere: type to filter,
    /// Up/Down to move, Tab to complete, Enter to confirm, Esc to back out.
    pub picker_idx: usize,
    pub picker_navigated: bool,
    pub tool_expanded: bool,
    pub notice: Option<String>,
    pub spinner_tick: usize,
    pub permission_gate: PermissionGate,
    pub pending_approval: Option<PendingTool>,
    /// Prompt tokens the provider reported for the last request. This is the
    /// only number we get that is not a guess, so it wins over the estimate.
    pub last_prompt_tokens: Option<usize>,
    /// Cached size of the prompt prefix that never changes between turns
    /// (system prompt + tool schemas). Measured when a run starts instead of
    /// rebuilt on every frame.
    pub prompt_overhead_chars: usize,
    pub theme: Theme,
    pub tree_visible: bool,
    pub auth: AuthStore,
    pub key_source: &'static str,
    pub discovered: Discovered,
    pub pending_login: Option<String>,
    pub login_buffer: String,
}

impl App {
    pub fn new() -> Self {
        let config = Config::load();
        Self::new_with_config(config, vec![])
    }

    pub fn new_with_config(config: Config, cli_extra_tools: Vec<String>) -> Self {
        let session_manager = SessionManager::new();
        // Resume where you left off: restore transcript + endpoint + theme.
        // Falls back to config defaults for fresh starts or old session files.
        let restored = session_manager.most_recent_nonempty();

        let provider_name = restored
            .as_ref()
            .and_then(|s| s.provider.clone())
            .map(|p| p.trim().to_lowercase())
            .filter(|p| !p.is_empty() && config.resolve_provider_config(p).known)
            .unwrap_or_else(|| config.resolve_provider());
        let rpc = config.resolve_provider_config(&provider_name);
        let provider = ProviderKind::from_str(&rpc.api);
        let current_model = restored
            .as_ref()
            .and_then(|s| s.model.clone())
            .map(|m| m.trim().to_string())
            .filter(|m| !m.is_empty())
            .or_else(|| {
                config.model.clone().map(|m| m.trim().to_string()).filter(|m| !m.is_empty())
            })
            .unwrap_or_else(|| config.resolve_default_model(&provider_name));
        let base_url = if rpc.base_url.is_empty() {
            config.resolve_base_url()
        } else {
            rpc.base_url.clone()
        };
        let mut extras = config.resolve_extra_tools();
        extras.extend(cli_extra_tools);
        let mut tool_registry = ToolRegistry::new().with_extras(&extras);

        let restored_theme = restored
            .as_ref()
            .and_then(|s| s.theme.clone())
            .map(|t| theme::resolve(&t))
            .unwrap_or_else(|| theme::resolve(&config.resolve_theme()));
        let (conversation, session_id, restored_id) = match restored {
            Some(s) => {
                let mut c = Conversation::new();
                c.messages = s.messages;
                let id = s.id.clone();
                (c, Some(id.clone()), Some(id))
            }
            None => (Conversation::new(), None, None),
        };
        // NOTE: `restored` was moved by the match above; theme came from `restored_theme`.

        let mcp_servers = Self::init_mcp(&config, &mut tool_registry);
        let auth = AuthStore::new();
        let (delegate_key, _) = resolve_api_key(&provider_name, &auth, &config);
        let endpoint = std::sync::Arc::new(std::sync::Mutex::new(
            crate::tools::delegate::ActiveEndpoint {
                kind: provider,
                api_key: delegate_key,
                model: current_model.clone(),
                base_url: base_url.clone(),
            },
        ));
        // delegate is opt-in extra (no sub-agents in core)
        if extras.iter().any(|e| {
            e.eq_ignore_ascii_case("delegate") || e.eq_ignore_ascii_case("all")
        }) {
            tool_registry.register_delegate(endpoint.clone());
        }

        let tool_registry = Arc::new(tool_registry);
        let permission_gate = PermissionGate::new(config.resolve_auto_approve());
        let theme = restored_theme;
        let (initial_key, key_source) = resolve_api_key(&provider_name, &auth, &config);
        let is_home = conversation.messages.is_empty();

        Self {
            conversation,
            input: InputState::new(),
            workspace: WorkspaceContext::new(),
            status: StatusBar::new_with_provider(&provider.to_string(), &current_model),
            should_quit: false,
            tool_registry,
            config,
            session_manager,
            session_id,
            mcp_servers,
            chat_scroll: 0,
            should_auto_scroll: true,
            queued_input: Vec::new(),
            event_rx: None,
            streaming_text: String::new(),
            provider,
            provider_name: provider_name.clone(),
            base_url: base_url.clone(),
            endpoint,
            current_model: current_model.clone(),
            cancelled: Arc::new(AtomicBool::new(false)),
            is_home,
            picker_idx: 0,
            picker_navigated: false,
            tool_expanded: false,
            notice: if initial_key.is_empty() {
                Some(format!("no API key for '{}' — /login {} or set env", provider_name, provider_name))
            } else if let Some(id) = restored_id {
                let short: String = id.chars().take(12).collect();
                Some(format!("resumed session {} — /new for fresh", short))
            } else {
                None
            },
            spinner_tick: 0,
            permission_gate,
            pending_approval: None,
            last_prompt_tokens: None,
            prompt_overhead_chars: 0,
            theme,
            tree_visible: false,
            auth,
            key_source,
            discovered: Discovered::load(),
            pending_login: None,
            login_buffer: String::new(),
        }
    }

    /// Switch provider+model for this session (from `/model`).
    /// `provider_id` must be known or user-configured; model may be free-form.
    pub fn apply_provider_model(&mut self, provider_id: &str, model: &str) -> Result<(), String> {
        let id = provider_id.trim().to_lowercase();
        let rpc = self.config.resolve_provider_config(&id);
        if !rpc.known {
            return Err(format!("unknown provider '{}'. Known: {}. Custom endpoints go in barong.jsonc `providers`.", id, self.config.all_provider_ids().join(", ")));
        }
        self.provider = ProviderKind::from_str(&rpc.api);
        self.provider_name = id.clone();
        self.base_url = if rpc.base_url.is_empty() {
            self.config.resolve_base_url()
        } else {
            rpc.base_url.clone()
        };
        self.current_model = model.trim().to_string();
        self.status.model = self.current_model.clone();
        self.status.llm_provider = self.provider.to_string();
        let (key, src) = resolve_api_key(&id, &self.auth, &self.config);
        self.key_source = src;
        if let Ok(mut ep) = self.endpoint.lock() {
            ep.kind = self.provider;
            ep.api_key = key.clone();
            ep.model = self.current_model.clone();
            ep.base_url = self.base_url.clone();
        }
        if key.is_empty() {
            self.notice = Some(format!("no API key for '{}' — /login {}", id, id));
        }
        Ok(())
    }

    /// Parse `provider/model...` (split at first `/`) or a bare model id
    /// for the current provider.
    pub fn parse_model_arg(&self, arg: &str) -> Option<(String, String)> {
        let t = arg.trim();
        if t.is_empty() {
            return None;
        }
        if let Some(i) = t.find('/') {
            let p = t[..i].trim().to_lowercase();
            let m = t[i + 1..].trim().to_string();
            if p.is_empty() || m.is_empty() {
                return None;
            }
            Some((p, m))
        } else {
            Some((self.provider_name.clone(), t.to_string()))
        }
    }

    /// Apply `--provider` / `--model` CLI overrides in TUI mode.
    pub fn apply_cli_overrides(&mut self, provider: Option<&str>, model: Option<&str>) {
        if let Some(p) = provider.map(str::trim).filter(|s| !s.is_empty()) {
            let def = self.config.resolve_default_model(p);
            if let Err(e) = self.apply_provider_model(p, &def) {
                self.notice = Some(e);
            }
        }
        if let Some(m) = model.map(str::trim).filter(|s| !s.is_empty()) {
            match self.parse_model_arg(m) {
                Some((p, mm)) => {
                    if let Err(e) = self.apply_provider_model(&p, &mm) {
                        self.notice = Some(e);
                    }
                }
                None => self.notice = Some(format!("bad --model '{}', use provider/model", m)),
            }
        }
    }

    pub fn api_key(&self) -> String {
        resolve_api_key(&self.provider_name, &self.auth, &self.config).0
    }

    pub fn discovery_target(&self, provider_id: &str) -> Option<(String, String)> {
        let id = provider_id.trim().to_lowercase();
        let rpc = self.config.resolve_provider_config(&id);
        if rpc.api != "openai" || rpc.base_url.is_empty() {
            return None; // anthropic has no list endpoint; unknown has no URL
        }
        let (key, _) = resolve_api_key(&id, &self.auth, &self.config);
        // Most catalogs are key-gated, but these serve /v1/models publicly.
        let public_catalog = matches!(id.as_str(), "openrouter" | "ollama" | "9router");
        if key.is_empty() && !public_catalog {
            return None;
        }
        Some((rpc.base_url.clone(), key))
    }

    /// Refresh one provider's model list in the background.
    pub fn refresh_provider(&self, provider_id: &str) {
        // Never hit the network under `cargo test`.
        if cfg!(test) {
            return;
        }
        let id = provider_id.trim().to_lowercase();
        if let Some((base_url, key)) = self.discovery_target(&id) {
            models::refresh_in_background(self.discovered.clone(), id, base_url, key);
        }
    }

    /// Refresh stale providers at startup (prod only — tests never call this,
    /// so no network happens under `cargo test`).
    pub fn refresh_stale_models(&self) {
        for pid in self.config.all_provider_ids() {
            let stale = self.discovered.cache_age(&pid).map(|a| a > models::CACHE_TTL_SECS).unwrap_or(true);
            if !stale {
                continue;
            }
            // Only touch endpoints we can actually query.
            if self.discovery_target(&pid).is_some() {
                self.refresh_provider(&pid);
            }
        }
    }

    pub fn set_theme(&mut self, name: &str) {
        self.theme = theme::resolve(name);
        self.config.theme = Some(self.theme.name.clone());
        self.notice = Some(format!("theme: {}", self.theme.name));
    }

    pub fn approve_pending(&mut self, decision: Decision) {
        if let Some(p) = self.pending_approval.take() {
            self.permission_gate.resolve(&p.id, decision.clone());
            self.notice = Some(match decision {
                Decision::AllowOnce => format!("allowed {} once", p.name),
                Decision::AllowSession => format!("always allow {} this session", p.name),
                Decision::Deny => format!("denied {}", p.name),
            });
        }
    }

    /// Fraction of the model's context window in use, 0..1.
    ///
    /// Uses the provider's own prompt-token count when it has reported one;
    /// otherwise estimates from characters (~4 per token). The estimate covers
    /// *everything we would send* — system prompt, tool schemas and the
    /// transcript — because all of it occupies the window on every request.
    pub fn context_usage(&self) -> f32 {
        let window = self.config.resolve_context_window() as f32;
        let tokens = match self.last_prompt_tokens {
            Some(reported) => {
                // The answer so far is not in the reported count yet.
                reported as f32 + self.streaming_text.len() as f32 / 4.0
            }
            None => self.estimated_prompt_tokens() as f32,
        };
        (tokens / window).clamp(0.0, 1.0)
    }

    /// True while the number on screen is an estimate rather than a count the
    /// provider gave us.
    pub fn context_is_estimated(&self) -> bool {
        self.last_prompt_tokens.is_none()
    }

    /// chars/4 over the whole outgoing payload. `prompt_overhead_chars` is
    /// refreshed on every run; the file tree is counted once, not twice.
    pub fn estimated_prompt_tokens(&self) -> usize {
        let mut chars = self.prompt_overhead_chars;
        for m in &self.conversation.messages {
            chars += m.content.as_deref().unwrap_or("").len();
        }
        chars += self.streaming_text.len();
        chars / 4
    }

    /// The system prompt sent with every request: instructions + workspace
    /// context (AGENTS.md, git state, file tree).
    fn build_system_prompt(&self) -> String {
        format!(
            "{}\n\n## Workspace Context\n{}",
            include_str!("../prompts/system.md"),
            self.workspace.summary(),
        )
    }

    /// Measure the constant part of the prompt. Called when a run starts (and
    /// after `/reload`), never per frame — `workspace.summary()` rebuilds the
    /// whole file tree on every call.
    fn refresh_prompt_overhead(&mut self, system_prompt: &str) {
        let mut chars = system_prompt.len();
        for def in self.tool_registry.definitions() {
            chars += def.name.len() + def.description.len();
            chars += def.input_schema.to_string().len();
        }
        self.prompt_overhead_chars = chars;
    }

    pub fn auto_compact_threshold(&self) -> f32 {
        0.85
    }

    /// Compact before an agent run if context is hot. Returns report if compacted.
    pub fn maybe_auto_compact(&mut self) -> Option<String> {
        if !self.config.resolve_auto_compact() {
            return None;
        }
        let usage = self.context_usage();
        let n = self.conversation.messages.len();
        if usage < self.auto_compact_threshold() && n <= 100 {
            return None;
        }
        let keep = self.config.resolve_compact_keep();
        if let Some((dropped, _)) = self.conversation.compact(keep) {
            self.save_session();
            return Some(format!(
                "auto-compacted: dropped {} msgs, kept last {} (ctx was {:.0}%)",
                dropped,
                keep,
                usage * 100.0
            ));
        }
        None
    }

    pub fn cwd_short(&self) -> String {
        let s = self.workspace.root.to_string_lossy().to_string();
        if let Some(home) = std::env::var("HOME").ok() {
            if s.starts_with(&home) {
                return format!("~{}", &s[home.len()..]);
            }
        }
        // show last 2 components for brevity
        let parts: Vec<&str> = s.split('/').filter(|p| !p.is_empty()).collect();
        if parts.len() > 2 {
            format!("…/{}/{}", parts[parts.len() - 2], parts[parts.len() - 1])
        } else {
            s
        }
    }

    pub fn render(&mut self, frame: &mut Frame) {
        crate::tui::ui::render(frame, self);
    }

    pub fn handle_events(&mut self) -> Result<()> {
        crate::tui::input::handle_events(self)
    }

    pub fn handle_stream(&mut self) -> Result<()> {
        // Deferred persist: the rx borrow below forbids &mut self calls inline.
        let mut dirty = false;
        let mut run_finished = false;
        if let Some(rx) = &mut self.event_rx {
            loop {
                match rx.try_recv() {
                    Ok(StreamEvent::Text(token)) => {
                        self.streaming_text.push_str(&token);
                        self.should_auto_scroll = true;
                    }
                    Ok(StreamEvent::Usage { input_tokens, output_tokens }) => {
                        self.status.token_count =
                            format!("in:{} out:{}", input_tokens, output_tokens);
                        // Authoritative context reading for the ctx gauge.
                        self.last_prompt_tokens = Some(input_tokens as usize);
                    }
                    Ok(StreamEvent::ToolCall { id: _, name, args }) => {
                        // Flush pending assistant text before tool block.
                        if !self.streaming_text.is_empty() {
                            let text = std::mem::take(&mut self.streaming_text);
                            self.conversation.add_message("assistant".into(), text);
                        }
                        let content = crate::agent::r#loop::describe_tool_call(&name, &args);
                        self.conversation.add_message("tool".into(), content);
                        self.should_auto_scroll = true;
                    }
                    Ok(StreamEvent::ToolResult { id: _, name, result }) => {
                        let pretty = serde_json::to_string_pretty(&result).unwrap_or_default();
                        let summary = crate::agent::r#loop::summarize_result(&name, &pretty, 3);
                        self.conversation
                            .add_message("tool".into(), format!("\n{}", summary));
                        self.should_auto_scroll = true;
                        // Crash-safe: persist progress as tools complete.
                        dirty = true;
                    }
                    Ok(StreamEvent::PermissionRequest { id, name, args }) => {
                        self.pending_approval = Some(PendingTool { id, name, args });
                        self.status.tool_status = "waiting approval".into();
                        self.should_auto_scroll = true;
                    }
                    Ok(StreamEvent::PermissionResult { id: _, approved }) => {
                        if !approved {
                            self.notice = Some("tool denied".into());
                        }
                        self.status.tool_status = "processing...".into();
                    }
                    Ok(StreamEvent::Done) => {
                        if !self.streaming_text.is_empty() {
                            let text = std::mem::take(&mut self.streaming_text);
                            self.conversation.add_message("assistant".into(), text);
                        }
                        self.save_session();
                        self.event_rx = None;
                        self.pending_approval = None;
                        self.permission_gate.clear_pending();
                        self.status.tool_status = "idle".into();
                        self.should_auto_scroll = true;
                        run_finished = true;
                        break;
                    }
                    Err(tokio::sync::mpsc::error::TryRecvError::Empty) => break,
                    Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => {
                        if !self.streaming_text.is_empty() {
                            let text = std::mem::take(&mut self.streaming_text);
                            self.conversation.add_message("assistant".into(), text);
                        }
                        self.save_session();
                        self.event_rx = None;
                        self.pending_approval = None;
                        self.permission_gate.clear_pending();
                        self.status.tool_status = "idle".into();
                        run_finished = true;
                        break;
                    }
                }
            }
        }
        if dirty {
            self.save_session();
        }
        if run_finished {
            // Fire queued messages one live turn at a time. A turn that refuses
            // to start (missing key, bad provider) must not stall the rest, so
            // keep draining while idle — bounded by the queue length, and each
            // pass either starts a run (loop ends) or shrinks the queue.
            let mut budget = self.queued_input.len();
            while budget > 0 && self.event_rx.is_none() && !self.queued_input.is_empty() {
                budget -= 1;
                self.drain_queued_input();
            }
        }
        Ok(())
    }

    /// Send the next queued message once the agent goes idle. Runs at most one
    /// per call so each drain re-enters the same stream → Done → drain path.
    pub fn drain_queued_input(&mut self) {
        if self.event_rx.is_some() || self.queued_input.is_empty() {
            return;
        }
        let next = self.queued_input.remove(0);
        // A failure here (e.g. missing key) leaves event_rx None, so the next
        // frame drains the rest — no spin, the queue always shrinks.
        if let Err(e) = crate::tui::input::submit_message(self, next) {
            self.notice = Some(format!("queued message failed: {}", e));
        }
    }

    pub fn session_meta(&self) -> crate::session::SessionMeta {
        crate::session::SessionMeta {
            provider: self.provider_name.clone(),
            model: self.current_model.clone(),
            theme: self.theme.name.clone(),
        }
    }

    pub fn save_session(&mut self) {
        let meta = self.session_meta();
        match self.session_id.as_ref() {
            Some(id) => self.session_manager.update(id, &self.conversation.messages, &meta),
            None => {
                let id = self.session_manager.save(&self.conversation.messages, &meta);
                self.session_id = Some(id);
            }
        }
    }

    fn init_mcp(config: &Config, tool_registry: &mut ToolRegistry) -> Vec<std::sync::Arc<std::sync::Mutex<McpServer>>> {
        let mut servers = Vec::new();
        for sc in &config.mcp_servers {
            match McpServer::spawn(&sc.name, &sc.command, &sc.args) {
                Ok(server) => {
                    let server = std::sync::Arc::new(std::sync::Mutex::new(server));
                    if let Ok(mut locked) = server.lock() {
                        if let Ok(tools) = locked.list_tools() {
                            for t in tools {
                                let adapter = McpToolAdapter {
                                    server_name: sc.name.clone(),
                                    tool_name: t.name,
                                    description: t.description,
                                    schema: t.input_schema,
                                    server: server.clone(),
                                };
                                tool_registry.register(Box::new(adapter));
                            }
                        }
                    }
                    servers.push(server);
                }
                Err(e) => {
                    tracing::warn!("Failed to start MCP server '{}': {}", sc.name, e);
                }
            }
        }
        servers
    }

    /// Re-measure the prompt overhead after the workspace changed on disk.
    pub fn refresh_prompt_overhead_for_reload(&mut self) {
        let prompt = self.build_system_prompt();
        self.refresh_prompt_overhead(&prompt);
    }

    pub fn start_agent(&mut self) {
        if let Some(report) = self.maybe_auto_compact() {
            self.notice = Some(report.clone());
            self.conversation.add_message("assistant".into(), format!("_{}_", report));
        }
        let system_prompt = self.build_system_prompt();
        // The reported count belongs to the request we are about to replace.
        self.last_prompt_tokens = None;
        self.refresh_prompt_overhead(&system_prompt);
        self.conversation.set_system_prompt(system_prompt);

        let api_key = self.api_key();
        if api_key.is_empty() {
            let msg = format!("No API key for '{}'. Run `/login {}` or set env.", self.provider_name, self.provider_name);
            self.conversation.add_message("assistant".into(), msg);
            self.save_session();
            self.is_home = false;
            return;
        }
        let provider: Box<dyn LLMProvider> = match self.provider {
            ProviderKind::OpenAI => Box::new(OpenAIProvider::new(
                api_key,
                self.current_model.clone(),
                self.base_url.clone(),
            )),
            ProviderKind::Anthropic => Box::new(AnthropicProvider::new(
                api_key,
                self.current_model.clone(),
                self.config.resolve_max_tokens(),
            )),
        };

        let context = self.conversation.full_context();
        let tools = self.tool_registry.definitions();
        let (tx, rx) = mpsc::channel(64);
        self.cancelled.store(false, Ordering::Relaxed);
        let cancel_flag = self.cancelled.clone();
        let tool_registry = self.tool_registry.clone();
        let perm = self.permission_gate.clone();

        tokio::spawn(async move {
            start_agent_loop_with_limit(provider, context, tools, tool_registry, tx, cancel_flag, 25, perm).await;
        });

        self.event_rx = Some(rx);
        self.streaming_text = String::new();
        self.status.tool_status = "processing...".into();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    /// App with an isolated HOME so session restore never leaks into a test.
    fn app_with(config: Config) -> App {
        use std::sync::atomic::{AtomicU64, Ordering};
        static N: AtomicU64 = AtomicU64::new(0);
        let tmp = std::env::temp_dir()
            .join(format!("barong-ctx-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
        std::fs::create_dir_all(&tmp).unwrap();
        let orig = std::env::var("HOME").ok();
        // SAFETY: single-threaded test body; HOME is restored before returning.
        unsafe { std::env::set_var("HOME", &tmp) };
        let app = App::new_with_config(config, vec![]);
        match orig {
            Some(o) => unsafe { std::env::set_var("HOME", o) },
            None => unsafe { std::env::remove_var("HOME") },
        }
        app
    }

    fn with_window(window: usize) -> Config {
        Config {
            context_window: Some(window),
            ..Config::default()
        }
    }

    #[test]
    fn window_is_configurable_and_sane() {
        assert_eq!(Config::default().resolve_context_window(), 128_000);
        assert_eq!(with_window(1_000_000).resolve_context_window(), 1_000_000);
        assert_eq!(with_window(8_000).resolve_context_window(), 8_000);
        // Nonsense values fall back to the default rather than dividing by ~0.
        assert_eq!(with_window(0).resolve_context_window(), 128_000);
        assert_eq!(with_window(10).resolve_context_window(), 128_000);
    }

    #[test]
    fn gauge_uses_the_configured_window() {
        let mut app = app_with(with_window(10_000));
        // ~40k chars of transcript ≈ 10k tokens → 100% of a 10k window.
        app.conversation
            .add_message("user".into(), "x".repeat(40_000));
        let usage = app.context_usage();
        assert!(usage > 0.9, "small window must look full, got {}", usage);
        // The same transcript against a 1M window is a rounding error.
        let mut big = app_with(with_window(1_000_000));
        big.conversation.add_message("user".into(), "x".repeat(40_000));
        assert!(
            big.context_usage() < 0.02,
            "1M window must not look full, got {}",
            big.context_usage()
        );
    }

    #[test]
    fn provider_count_beats_the_estimate() {
        let mut app = app_with(with_window(100_000));
        app.conversation.add_message("user".into(), "short".into());
        assert!(app.context_is_estimated());
        // What the provider reported for the request it just handled.
        app.last_prompt_tokens = Some(75_000);
        assert!(!app.context_is_estimated());
        let usage = app.context_usage();
        assert!((0.74..=0.77).contains(&usage), "got {}", usage);
        // Streamed text counts on top of the reported prompt.
        app.streaming_text = "x".repeat(4_000); // ~1k tokens
        assert!(app.context_usage() > usage, "streaming must add up");
    }

    #[test]
    fn overhead_counts_the_prompt_once() {
        let mut app = app_with(with_window(100_000));
        app.refresh_prompt_overhead_for_reload();
        let overhead = app.prompt_overhead_chars;
        assert!(overhead > 0, "system prompt must be measured");

        // Exact composition: instructions + workspace summary + tool schemas.
        // The old code added `file_tree.len()` on top of `summary()`, which
        // already contains the tree — so this would be over by exactly that.
        let tools: usize = app
            .tool_registry
            .definitions()
            .iter()
            .map(|d| d.name.len() + d.description.len() + d.input_schema.to_string().len())
            .sum();
        let expected = include_str!("../prompts/system.md").len()
            + "\n\n## Workspace Context\n".len()
            + app.workspace.summary().len()
            + tools;
        assert_eq!(
            overhead, expected,
            "overhead must be the sum of its parts, nothing counted twice"
        );

        // Token estimate is chars/4 and stays below the raw char count.
        assert!(app.estimated_prompt_tokens() <= overhead / 4);
    }

    #[test]
    fn clearing_resets_the_gauge() {
        let mut app = app_with(with_window(100_000));
        app.last_prompt_tokens = Some(90_000);
        assert!(app.context_usage() > 0.8);
        app.conversation = crate::agent::conversation::Conversation::new();
        app.last_prompt_tokens = None;
        assert!(app.context_usage() < 0.01, "got {}", app.context_usage());
        assert!(app.context_is_estimated());
    }
}
