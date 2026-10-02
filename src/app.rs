use crate::agent::r#loop::start_agent_loop;
use crate::agent::conversation::Conversation;
use crate::agent::llm::{LLMProvider, OpenAIProvider, AnthropicProvider, StreamEvent, ProviderKind};
use crate::config::Config;
use crate::mcp::{McpServer, McpToolAdapter};
use crate::session::SessionManager;
use crate::tui::input::InputState;
use crate::tui::status::StatusBar;
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
    pub current_model: String,
    pub cancelled: Arc<AtomicBool>,
    pub is_home: bool,
    // UX state (minimal)
    pub palette_idx: usize,
    pub tool_expanded: bool,
    pub notice: Option<String>,
    pub spinner_tick: usize,
}

impl App {
    pub fn new() -> Self {
        let config = Config::load();
        Self::new_with_config(config, vec![])
    }

    pub fn new_with_config(config: Config, cli_extra_tools: Vec<String>) -> Self {
        let provider_str = config.resolve_provider();
        let provider = ProviderKind::from_str(&provider_str);
        let current_model = config.resolve_model(&provider_str);
        let mut extras = config.resolve_extra_tools();
        extras.extend(cli_extra_tools);
        let mut tool_registry = ToolRegistry::new().with_extras(&extras);
        let session_manager = SessionManager::new();

        let conversation = Conversation::new();
        let session_id = session_manager.most_recent_session().map(|s| s.id);

        let mcp_servers = Self::init_mcp(&config, &mut tool_registry);
        // delegate is opt-in extra (no sub-agents in core)
        if extras.iter().any(|e| {
            e.eq_ignore_ascii_case("delegate") || e.eq_ignore_ascii_case("all")
        }) {
            tool_registry.register_delegate(
                config.resolve_api_key(&provider_str),
                config.resolve_model(&provider_str),
                config.resolve_base_url(),
                provider,
            );
        }

        let tool_registry = Arc::new(tool_registry);

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
            current_model: current_model.clone(),
            cancelled: Arc::new(AtomicBool::new(false)),
            is_home: true,
            palette_idx: 0,
            tool_expanded: false,
            notice: None,
            spinner_tick: 0,
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
                        let content = format!("▸ **{}** `{}`", name, args);
                        self.conversation.add_message("tool".into(), content);
                        self.should_auto_scroll = true;
                    }
                    Ok(StreamEvent::ToolResult { id: _, name, result }) => {
                        let pretty = serde_json::to_string_pretty(&result).unwrap_or_default();
                        let summary = crate::agent::r#loop::summarize_result(&name, &pretty, 3);
                        self.conversation
                            .add_message("tool".into(), format!("\n{}", summary));
                        self.should_auto_scroll = true;
                    }
                    Ok(StreamEvent::Done) => {
                        if !self.streaming_text.is_empty() {
                            let text = std::mem::take(&mut self.streaming_text);
                            self.conversation.add_message("assistant".into(), text);
                        }
                        self.save_session();
                        self.event_rx = None;
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
                        self.status.tool_status = "idle".into();
                        break;
                    }
                }
            }
        }
        Ok(())
    }

    pub fn save_session(&mut self) {
        match self.session_id.as_ref() {
            Some(id) => self.session_manager.update(id, &self.conversation.messages),
            None => {
                let id = self.session_manager.save(&self.conversation.messages);
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
        let context_block = self.workspace.summary();
        let system_prompt = format!(
            "{}\n\n## Workspace Context\n{}",
            include_str!("../prompts/system.md"),
            context_block,
        );
        self.conversation.set_system_prompt(system_prompt);

        let provider: Box<dyn LLMProvider> = match self.provider {
            ProviderKind::OpenAI => Box::new(OpenAIProvider::new(
                self.config.resolve_api_key("openai"),
                self.current_model.clone(),
                self.config.resolve_base_url(),
            )),
            ProviderKind::Anthropic => Box::new(AnthropicProvider::new(
                self.config.resolve_api_key("anthropic"),
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

        tokio::spawn(async move {
            start_agent_loop(provider, context, tools, tool_registry, tx, cancel_flag).await;
        });

        self.event_rx = Some(rx);
        self.streaming_text = String::new();
        self.status.tool_status = "processing...".into();
    }
}
