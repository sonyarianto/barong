use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Clone)]
pub struct Conversation {
    pub messages: Vec<Message>,
    system_prompt: String,
    _max_tokens: usize,
}

impl Conversation {
    pub fn new() -> Self {
        Self {
            messages: Vec::new(),
            system_prompt: include_str!("../../prompts/system.md").to_string(),
            _max_tokens: 128_000,
        }
    }

    pub fn add_message(&mut self, role: String, content: String) {
        self.messages.push(Message { role, content });
    }

    pub fn set_system_prompt(&mut self, prompt: String) {
        self.system_prompt = prompt;
    }

    pub fn full_context(&self) -> Vec<Message> {
        let mut context = Vec::new();
        context.push(Message {
            role: "system".into(),
            content: self.system_prompt.clone(),
        });
        context.extend(self.messages.clone());
        context
    }
}
