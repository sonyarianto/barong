use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallFunction {
    pub name: String,
    pub arguments: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallMessage {
    pub id: String,
    #[serde(rename = "type")]
    pub type_: String,
    pub function: ToolCallFunction,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCallMessage>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
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
        self.messages.push(Message {
            role,
            content: Some(content),
            tool_calls: None,
            tool_call_id: None,
        });
    }

    pub fn set_system_prompt(&mut self, prompt: String) {
        self.system_prompt = prompt;
    }

    pub fn full_context(&self) -> Vec<Message> {
        let mut context = Vec::new();
        context.push(Message {
            role: "system".into(),
            content: Some(self.system_prompt.clone()),
            tool_calls: None,
            tool_call_id: None,
        });
        context.extend(self.messages.clone());
        context
    }
}
