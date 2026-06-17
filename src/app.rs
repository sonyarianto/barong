use crate::agent::r#loop::AgentLoopState;
use crate::agent::conversation::Conversation;
use crate::tui::input::InputState;
use crate::tui::status::StatusBar;
use crate::tools::ToolRegistry;
use crate::workspace::WorkspaceContext;
use anyhow::Result;
use ratatui::Frame;
use std::sync::Arc;

pub struct App {
    pub conversation: Conversation,
    pub input: InputState,
    pub agent_loop: AgentLoopState,
    pub workspace: WorkspaceContext,
    pub status: StatusBar,
    pub should_quit: bool,
    pub tool_registry: Arc<ToolRegistry>,
    pub chat_scroll: usize,
    pub should_auto_scroll: bool,
}

impl App {
    pub fn new() -> Self {
        let tool_registry = Arc::new(ToolRegistry::new());
        Self {
            conversation: Conversation::new(),
            input: InputState::new(),
            agent_loop: AgentLoopState::new(),
            workspace: WorkspaceContext::new(),
            status: StatusBar::new(),
            should_quit: false,
            tool_registry,
            chat_scroll: 0,
            should_auto_scroll: true,
        }
    }

    pub fn render(&mut self, frame: &mut Frame) {
        crate::tui::ui::render(frame, self);
    }

    pub fn handle_events(&mut self) -> Result<()> {
        crate::tui::input::handle_events(self)
    }
}
