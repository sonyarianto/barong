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

    /// Rough context usage 0..1 based on chars (~4 chars/token) vs 128k.
    pub fn context_usage(&self) -> f32 {
        let mut chars = self.workspace.file_tree.len() + self.workspace.summary().len();
        for m in &self.conversation.messages {
            chars += m.content.as_deref().unwrap_or("").len();
        }
        chars += self.streaming_text.len();
        (chars as f32 / 4.0 / 128_000.0).clamp(0.0, 1.0)
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
                        break;
                    }
                }
            }
        }
        if dirty {
            self.save_session();
        }
        Ok(())
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

    pub fn start_agent(&mut self) {
        if let Some(report) = self.maybe_auto_compact() {
            self.notice = Some(report.clone());
            self.conversation.add_message("assistant".into(), format!("_{}_", report));
        }
        let context_block = self.workspace.summary();
        let system_prompt = format!(
            "{}\n\n## Workspace Context\n{}",
            include_str!("../prompts/system.md"),
            context_block,
        );
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
