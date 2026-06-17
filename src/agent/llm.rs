use anyhow::Result;
use tokio::sync::mpsc;

#[derive(Debug, Clone)]
pub struct ToolDef {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
}

#[derive(Debug)]
pub enum StreamEvent {
    Text(String),
    ToolCall {
        id: String,
        name: String,
        args: serde_json::Value,
    },
    Done,
}

#[async_trait::async_trait]
pub trait LLMProvider: Send + Sync {
    async fn stream_chat(
        &self,
        messages: &[super::conversation::Message],
        tools: &[ToolDef],
    ) -> Result<mpsc::Receiver<StreamEvent>>;
}

pub struct OpenAIProvider {
    api_key: String,
    model: String,
    base_url: String,
}

impl OpenAIProvider {
    pub fn new(api_key: String, model: String) -> Self {
        Self {
            api_key,
            model,
            base_url: "https://api.openai.com/v1".into(),
        }
    }

    pub fn with_base_url(mut self, base_url: String) -> Self {
        self.base_url = base_url;
        self
    }
}

#[async_trait::async_trait]
impl LLMProvider for OpenAIProvider {
    async fn stream_chat(
        &self,
        messages: &[super::conversation::Message],
        tools: &[ToolDef],
    ) -> Result<mpsc::Receiver<StreamEvent>> {
        let (tx, rx) = mpsc::channel(64);

        let client = reqwest::Client::new();
        let url = format!("{}/chat/completions", self.base_url);

        let tools_json: Vec<serde_json::Value> = tools
            .iter()
            .map(|t| {
                serde_json::json!({
                    "type": "function",
                    "function": {
                        "name": t.name,
                        "description": t.description,
                        "parameters": t.input_schema,
                    }
                })
            })
            .collect();

        let mut body = serde_json::json!({
            "model": self.model,
            "messages": messages,
            "stream": true,
        });

        if !tools_json.is_empty() {
            body["tools"] = serde_json::Value::Array(tools_json);
        }

        let api_key = self.api_key.clone();

        tokio::spawn(async move {
            let response = match client
                .post(&url)
                .header("Authorization", format!("Bearer {}", api_key))
                .header("Content-Type", "application/json")
                .json(&body)
                .send()
                .await
            {
                Ok(r) => r,
                Err(e) => {
                    let _ = tx.send(StreamEvent::Text(format!("Error: {}", e))).await;
                    let _ = tx.send(StreamEvent::Done).await;
                    return;
                }
            };

            use futures::StreamExt;
            let mut stream = response.bytes_stream();
            let mut buffer = String::new();

            while let Some(chunk) = stream.next().await {
                match chunk {
                    Ok(bytes) => {
                        buffer.push_str(&String::from_utf8_lossy(&bytes));
                        for line in buffer.split_inclusive('\n') {
                            if !line.ends_with('\n') {
                                continue;
                            }
                            let line = line.trim();
                            if line.is_empty() || line == "data: [DONE]" {
                                continue;
                            }
                            if let Some(data) = line.strip_prefix("data: ") {
                                if let Ok(parsed) =
                                    serde_json::from_str::<serde_json::Value>(data)
                                {
                                    if let Some(delta) = parsed["choices"][0]["delta"].as_object()
                                    {
                                        if let Some(content) = delta.get("content") {
                                            let _ = tx
                                                .send(StreamEvent::Text(
                                                    content.as_str().unwrap_or("").to_string(),
                                                ))
                                                .await;
                                        }
                                        if let Some(tool_calls) = delta.get("tool_calls") {
                                            if let Some(calls) = tool_calls.as_array() {
                                                for call in calls {
                                                    if let Some(func) = call["function"].as_object()
                                                    {
                                                        let name = func["name"]
                                                            .as_str()
                                                            .unwrap_or("")
                                                            .to_string();
                                                        let args = func["arguments"]
                                                            .as_str()
                                                            .unwrap_or("{}");
                                                        if !name.is_empty() {
                                                            let parsed_args: serde_json::Value =
                                                                serde_json::from_str(args)
                                                                    .unwrap_or(serde_json::json!({}));
                                                            let _ = tx
                                                                .send(StreamEvent::ToolCall {
                                                                    id: call["id"]
                                                                        .as_str()
                                                                        .unwrap_or("")
                                                                        .to_string(),
                                                                    name,
                                                                    args: parsed_args,
                                                                })
                                                                .await;
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        buffer.clear();
                    }
                    Err(e) => {
                        let _ = tx
                            .send(StreamEvent::Text(format!("Stream error: {}", e)))
                            .await;
                        break;
                    }
                }
            }

            let _ = tx.send(StreamEvent::Done).await;
        });

        Ok(rx)
    }
}

pub struct AnthropicProvider {
    _api_key: String,
    _model: String,
}

impl AnthropicProvider {
    pub fn new(api_key: String, model: String) -> Self {
        Self {
            _api_key: api_key,
            _model: model,
        }
    }
}

#[async_trait::async_trait]
impl LLMProvider for AnthropicProvider {
    async fn stream_chat(
        &self,
        _messages: &[super::conversation::Message],
        _tools: &[ToolDef],
    ) -> Result<mpsc::Receiver<StreamEvent>> {
        let (tx, rx) = mpsc::channel(64);

        tokio::spawn(async move {
            let _ = tx
                .send(StreamEvent::Text(
                    "Anthropic provider not yet implemented".into(),
                ))
                .await;
            let _ = tx.send(StreamEvent::Done).await;
        });

        Ok(rx)
    }
}
