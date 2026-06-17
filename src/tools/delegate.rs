use crate::agent::conversation::Message;
use crate::agent::llm::{AnthropicProvider, LLMProvider, OpenAIProvider, ProviderKind, StreamEvent};
use crate::tools::Tool;
use anyhow::Result;
use serde_json::Value;

pub struct Delegate {
    api_key: String,
    model: String,
    base_url: String,
    provider: ProviderKind,
}

impl Delegate {
    pub fn new(api_key: String, model: String, base_url: String, provider: ProviderKind) -> Self {
        Self { api_key, model, base_url, provider }
    }
}

#[async_trait::async_trait]
impl Tool for Delegate {
    fn name(&self) -> &str {
        "delegate"
    }

    fn description(&self) -> &str {
        "Delegate a complex sub-task to a sub-agent. Use this for tasks that benefit from independent analysis."
    }

    fn schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "task": {
                    "type": "string",
                    "description": "Detailed instructions for the sub-agent"
                }
            },
            "required": ["task"]
        })
    }

    async fn call(&self, args: Value) -> Result<Value> {
        let task = args["task"].as_str().unwrap_or("");

        let llm_provider: Box<dyn LLMProvider> = match self.provider {
            ProviderKind::OpenAI => Box::new(OpenAIProvider::new(
                self.api_key.clone(),
                self.model.clone(),
                self.base_url.clone(),
            )),
            ProviderKind::Anthropic => Box::new(AnthropicProvider::new(
                self.api_key.clone(),
                self.model.clone(),
                4096,
            )),
        };

        let messages = vec![
            Message {
                role: "system".into(),
                content: "You are a focused sub-agent. Complete the assigned task concisely. Do not ask questions — just do it and report the result.".into(),
            },
            Message {
                role: "user".into(),
                content: task.to_string(),
            },
        ];

        let mut stream = llm_provider.stream_chat(&messages, &[]).await?;
        let mut response = String::new();

        while let Some(event) = stream.recv().await {
            match event {
                StreamEvent::Text(t) => response.push_str(&t),
                StreamEvent::Done => break,
                _ => {}
            }
        }

        Ok(serde_json::json!({ "result": response }))
    }
}
