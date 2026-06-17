use crate::agent::conversation::{Conversation, Message};
use crate::agent::llm::{LLMProvider, StreamEvent};
use crate::tools::ToolRegistry;
use anyhow::Result;
use std::sync::Arc;

pub struct AgentLoopState {
    pub is_running: bool,
    pub iteration: u32,
    pub max_iterations: u32,
}

impl AgentLoopState {
    pub fn new() -> Self {
        Self {
            is_running: false,
            iteration: 0,
            max_iterations: 25,
        }
    }
}

pub struct AgentLoop {
    provider: Box<dyn LLMProvider>,
    tools: Arc<ToolRegistry>,
    pub state: AgentLoopState,
}

impl AgentLoop {
    pub fn new(
        provider: Box<dyn LLMProvider>,
        tools: Arc<ToolRegistry>,
    ) -> Self {
        Self {
            provider,
            tools,
            state: AgentLoopState::new(),
        }
    }

    pub async fn run_iteration(
        &mut self,
        conversation: &mut Conversation,
    ) -> Result<()> {
        self.state.is_running = true;
        self.state.iteration = 0;

        while self.state.iteration < self.state.max_iterations {
            self.state.iteration += 1;

            let tool_defs = self.tools.definitions();
            let context = conversation.full_context();
            let mut stream = self
                .provider
                .stream_chat(&context, &tool_defs)
                .await?;

            let mut tool_calls = Vec::new();
            let mut response_text = String::new();

            while let Some(event) = stream.recv().await {
                match event {
                    StreamEvent::Text(token) => {
                        response_text.push_str(&token);
                    }
                    StreamEvent::ToolCall { id, name, args } => {
                        tool_calls.push((id, name, args));
                    }
                    StreamEvent::Done => break,
                }
            }

            if !response_text.is_empty() {
                conversation.messages.push(Message {
                    role: "assistant".into(),
                    content: response_text,
                });
            }

            if tool_calls.is_empty() {
                break;
            }

            for (_id, name, args) in tool_calls {
                if let Some(tool) = self.tools.get(&name) {
                    let result = tool.call(args).await;
                    let content = match result {
                        Ok(output) => output.to_string(),
                        Err(e) => format!("Error: {}", e),
                    };
                    conversation.messages.push(Message {
                        role: "tool".into(),
                        content,
                    });
                }
            }
        }

        self.state.is_running = false;
        Ok(())
    }
}
