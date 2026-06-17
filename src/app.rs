use crate::agent::r#loop::start_agent_loop;
use crate::agent::conversation::Conversation;
use crate::agent::llm::{LLMProvider, OpenAIProvider, AnthropicProvider, StreamEvent, ProviderKind};
use crate::tui::input::InputState;
use crate::tui::status::StatusBar;
use crate::tools::ToolRegistry;
use crate::workspace::WorkspaceContext;
use anyhow::Result;
use ratatui::Frame;
use std::sync::Arc;
use tokio::sync::mpsc;

pub struct App {
    pub conversation: Conversation,
    pub input: InputState,
    pub workspace: WorkspaceContext,
    pub status: StatusBar,
    pub should_quit: bool,
    pub tool_registry: Arc<ToolRegistry>,
    pub chat_scroll: usize,
    pub should_auto_scroll: bool,
    pub event_rx: Option<mpsc::Receiver<StreamEvent>>,
    pub streaming_text: String,
    pub provider: ProviderKind,
}

impl App {
    pub fn new() -> Self {
        let provider = ProviderKind::from_env();
        let status_str = format!("LLM: {}", provider);
        let tool_registry = Arc::new(ToolRegistry::new());

        Self {
            conversation: Conversation::new(),
            input: InputState::new(),
            workspace: WorkspaceContext::new(),
            status: StatusBar::new_with_provider(&status_str),
            should_quit: false,
            tool_registry,
            chat_scroll: 0,
            should_auto_scroll: true,
            event_rx: None,
            streaming_text: String::new(),
            provider,
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
                    Ok(StreamEvent::ToolCall { id: _, name, args }) => {
                        let content = format!("Tool call: {} ({})", name, args);
                        self.conversation.add_message("tool".into(), content);
                        self.streaming_text.clear();
                    }
                    Ok(StreamEvent::Done) => {
                        if !self.streaming_text.is_empty() {
                            let text = std::mem::take(&mut self.streaming_text);
                            self.conversation.add_message("assistant".into(), text);
                        }
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
                        self.event_rx = None;
                        self.status.tool_status = "idle".into();
                        break;
                    }
                }
            }
        }
        Ok(())
    }

    pub fn start_agent(&mut self) {
        let provider: Box<dyn LLMProvider> = match self.provider {
            ProviderKind::OpenAI => Box::new(OpenAIProvider::from_env()),
            ProviderKind::Anthropic => Box::new(AnthropicProvider::from_env()),
        };

        let context = self.conversation.full_context();
        let tools = self.tool_registry.definitions();
        let (tx, rx) = mpsc::channel(64);

        tokio::spawn(async move {
            start_agent_loop(provider, context, tools, tx).await;
        });

        self.event_rx = Some(rx);
        self.streaming_text = String::new();
        self.status.tool_status = "processing...".into();
    }
}
