use crate::agent::conversation::Message;
use crate::agent::llm::{AnthropicProvider, LLMProvider, OpenAIProvider, ProviderKind, StreamEvent};
use crate::tools::Tool;
use anyhow::Result;
use serde_json::Value;
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;

/// Credentials + endpoint the delegate sub-agent uses.
/// Shared with App so `/login` and `/model` switches apply to it too —
/// otherwise it would keep calling the old provider with a stale key.
#[derive(Debug, Clone)]
pub struct ActiveEndpoint {
    pub kind: ProviderKind,
    pub api_key: String,
    pub model: String,
    pub base_url: String,
}

pub struct Delegate {
    endpoint: Arc<Mutex<ActiveEndpoint>>,
}

impl Delegate {
    pub fn new(endpoint: Arc<Mutex<ActiveEndpoint>>) -> Self {
        Self { endpoint }
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

    async fn call(&self, args: Value, tx: Option<mpsc::Sender<StreamEvent>>) -> Result<Value> {
        let task = args["task"].as_str().unwrap_or("");
        let ep = self.endpoint.lock().map(|e| e.clone()).unwrap_or(ActiveEndpoint {
            kind: ProviderKind::OpenAI,
            api_key: String::new(),
            model: "gpt-4o".into(),
            base_url: "https://api.openai.com/v1".into(),
        });

        let llm_provider: Box<dyn LLMProvider> = match ep.kind {
            ProviderKind::OpenAI => Box::new(OpenAIProvider::new(
                ep.api_key.clone(),
                ep.model.clone(),
                ep.base_url.clone(),
            )),
            ProviderKind::Anthropic => Box::new(AnthropicProvider::new(
                ep.api_key.clone(),
                ep.model.clone(),
                4096,
            )),
        };

        let messages = vec![
            Message {
                role: "system".into(),
                content: Some("You are a focused sub-agent. Complete the assigned task concisely. Do not ask questions — just do it and report the result.".into()),
                tool_calls: None,
                tool_call_id: None,
            },
            Message {
                role: "user".into(),
                content: Some(task.to_string()),
                tool_calls: None,
                tool_call_id: None,
            },
        ];

        let mut stream = llm_provider.stream_chat(&messages, &[]).await?;
        let mut response = String::new();

        while let Some(event) = stream.recv().await {
            match event {
                StreamEvent::Text(token) => {
                    response.push_str(&token);
                    if let Some(ref t) = tx {
                        let _ = t.send(StreamEvent::Text(token)).await;
                    }
                }
                StreamEvent::Done => break,
                _ => {}
            }
        }

        Ok(serde_json::json!({ "result": response }))
    }
}
