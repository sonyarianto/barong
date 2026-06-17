use anyhow::Result;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

#[derive(Debug, Clone, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ProviderKind {
    OpenAI,
    Anthropic,
}

impl ProviderKind {
    pub fn from_env() -> Self {
        match std::env::var("KALICODE_PROVIDER")
            .unwrap_or_default()
            .to_lowercase()
            .as_str()
        {
            "anthropic" => ProviderKind::Anthropic,
            _ => ProviderKind::OpenAI,
        }
    }
}

impl std::fmt::Display for ProviderKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProviderKind::OpenAI => write!(f, "openai"),
            ProviderKind::Anthropic => write!(f, "anthropic"),
        }
    }
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
    pub fn from_env() -> Self {
        Self {
            api_key: std::env::var("OPENAI_API_KEY")
                .or_else(|_| std::env::var("KALICODE_API_KEY"))
                .unwrap_or_default(),
            model: std::env::var("OPENAI_MODEL").unwrap_or_else(|_| "gpt-4o".into()),
            base_url: std::env::var("OPENAI_BASE_URL")
                .unwrap_or_else(|_| "https://api.openai.com/v1".into()),
        }
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

        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(120))
            .build()?;
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

        let body = serde_json::json!({
            "model": self.model,
            "messages": messages,
            "stream": true,
            "tools": tools_json,
        });

        let api_key = self.api_key.clone();
        let url_clone = url.clone();

        tokio::spawn(async move {
            let result = Self::do_stream(&client, &url_clone, &api_key, &body, &tx).await;
            if let Err(e) = result {
                let _ = tx
                    .send(StreamEvent::Text(format!("\n\n**Error:** {}", e)))
                    .await;
            }
            let _ = tx.send(StreamEvent::Done).await;
        });

        Ok(rx)
    }
}

impl OpenAIProvider {
    async fn do_stream(
        client: &reqwest::Client,
        url: &str,
        api_key: &str,
        body: &serde_json::Value,
        tx: &mpsc::Sender<StreamEvent>,
    ) -> Result<()> {
        let response = client
            .post(url)
            .header("Authorization", format!("Bearer {}", api_key))
            .header("Content-Type", "application/json")
            .json(body)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            anyhow::bail!("API error {}: {}", status, text);
        }

        use futures::StreamExt;
        let mut stream = response.bytes_stream();
        let mut buffer = String::new();

        while let Some(chunk) = stream.next().await {
            let bytes = chunk?;
            buffer.push_str(&String::from_utf8_lossy(&bytes));

            let mut processed = 0;
            for line in buffer.split_inclusive('\n') {
                if !line.ends_with('\n') {
                    break;
                }
                processed += line.len();
                let line = line.trim();
                if line.is_empty() || line == "data: [DONE]" {
                    continue;
                }
                if let Some(data) = line.strip_prefix("data: ") {
                    if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(data) {
                        if let Some(delta) = parsed["choices"][0]["delta"].as_object() {
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
                                        if let Some(func) = call["function"].as_object() {
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
            buffer.drain(..processed);
        }

        Ok(())
    }
}

pub struct AnthropicProvider {
    api_key: String,
    model: String,
}

impl AnthropicProvider {
    pub fn from_env() -> Self {
        Self {
            api_key: std::env::var("ANTHROPIC_API_KEY")
                .or_else(|_| std::env::var("KALICODE_API_KEY"))
                .unwrap_or_default(),
            model: std::env::var("ANTHROPIC_MODEL")
                .unwrap_or_else(|_| "claude-sonnet-4-20250514".into()),
        }
    }
}

#[async_trait::async_trait]
impl LLMProvider for AnthropicProvider {
    async fn stream_chat(
        &self,
        messages: &[super::conversation::Message],
        tools: &[ToolDef],
    ) -> Result<mpsc::Receiver<StreamEvent>> {
        let (tx, rx) = mpsc::channel(64);

        let api_key = self.api_key.clone();
        let model = self.model.clone();
        let messages_clone = messages.to_vec();

        let tools_json: Vec<serde_json::Value> = tools
            .iter()
            .map(|t| {
                serde_json::json!({
                    "name": t.name,
                    "description": t.description,
                    "input_schema": t.input_schema,
                })
            })
            .collect();

        tokio::spawn(async move {
            let client = match reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(120))
                .build()
            {
                Ok(c) => c,
                Err(e) => {
                    let _ = tx.send(StreamEvent::Text(format!("\n\n**Error:** {}", e))).await;
                    let _ = tx.send(StreamEvent::Done).await;
                    return;
                }
            };

            let mut body = serde_json::json!({
                "model": model,
                "messages": messages_clone,
                "max_tokens": 4096,
                "stream": true,
            });

            if !tools_json.is_empty() {
                body["tools"] = serde_json::Value::Array(tools_json);
            }

            let response = match client
                .post("https://api.anthropic.com/v1/messages")
                .header("x-api-key", &api_key)
                .header("anthropic-version", "2023-06-01")
                .header("Content-Type", "application/json")
                .json(&body)
                .send()
                .await
            {
                Ok(r) => r,
                Err(e) => {
                    let _ = tx.send(StreamEvent::Text(format!("\n\n**Error:** {}", e))).await;
                    let _ = tx.send(StreamEvent::Done).await;
                    return;
                }
            };

            if !response.status().is_success() {
                let status = response.status();
                let text = response.text().await.unwrap_or_default();
                let _ = tx
                    .send(StreamEvent::Text(format!("\n\n**API Error {}:** {}", status, text)))
                    .await;
                let _ = tx.send(StreamEvent::Done).await;
                return;
            }

            use futures::StreamExt;
            let mut stream = response.bytes_stream();
            let mut buffer = String::new();
            let mut current_tool_name = String::new();
            let mut current_tool_input = String::new();
            let mut current_tool_id = String::new();
            let mut in_tool_block = false;

            while let Some(chunk) = stream.next().await {
                let bytes = match chunk {
                    Ok(b) => b,
                    Err(e) => {
                        let _ = tx
                            .send(StreamEvent::Text(format!("\n\n**Stream error:** {}", e)))
                            .await;
                        break;
                    }
                };

                buffer.push_str(&String::from_utf8_lossy(&bytes));

                while let Some(line_end) = buffer.find('\n') {
                    let line = buffer[..line_end].trim().to_string();
                    buffer.drain(..=line_end);

                    if line.is_empty() {
                        continue;
                    }

                    if line.starts_with("event: ") {
                        let event_type = line.strip_prefix("event: ").unwrap_or("").to_string();
                        let mut data = String::new();

                        if let Some(d) = buffer.find('\n') {
                            let data_line = buffer[..d].trim().to_string();
                            if data_line.starts_with("data: ") {
                                data = data_line.strip_prefix("data: ").unwrap_or("").to_string();
                            }
                            buffer.drain(..=d);
                        }

                        match event_type.as_str() {
                            "content_block_start" => {
                                if let Ok(parsed) =
                                    serde_json::from_str::<serde_json::Value>(&data)
                                {
                                    let block = &parsed["content_block"];
                                    if let Some(name) = block["name"].as_str() {
                                        in_tool_block = true;
                                        current_tool_name = name.to_string();
                                        current_tool_input.clear();
                                        current_tool_id =
                                            block["id"].as_str().unwrap_or("").to_string();
                                        if let Some(input) = block["input"].as_object() {
                                            current_tool_input =
                                                serde_json::to_string(input).unwrap_or_default();
                                        }
                                    } else {
                                        in_tool_block = false;
                                    }
                                }
                            }
                            "content_block_delta" => {
                                if let Ok(parsed) =
                                    serde_json::from_str::<serde_json::Value>(&data)
                                {
                                    if let Some(text) = parsed["delta"]["text"].as_str() {
                                        let _ = tx
                                            .send(StreamEvent::Text(text.to_string()))
                                            .await;
                                    }
                                    if let Some(partial) = parsed["delta"]["partial_json"].as_str()
                                    {
                                        current_tool_input.push_str(partial);
                                    }
                                }
                            }
                            "content_block_stop" => {
                                if in_tool_block && !current_tool_name.is_empty() {
                                    let args: serde_json::Value =
                                        serde_json::from_str(&current_tool_input)
                                            .unwrap_or(serde_json::json!({}));
                                    let _ = tx
                                        .send(StreamEvent::ToolCall {
                                            id: current_tool_id.clone(),
                                            name: std::mem::take(&mut current_tool_name),
                                            args,
                                        })
                                        .await;
                                    current_tool_input.clear();
                                    in_tool_block = false;
                                }
                            }
                            "message_delta" => {
                                if let Ok(parsed) =
                                    serde_json::from_str::<serde_json::Value>(&data)
                                {
                                    if let Some(stop_reason) =
                                        parsed["delta"]["stop_reason"].as_str()
                                    {
                                        if stop_reason == "tool_use"
                                            && !current_tool_name.is_empty()
                                        {
                                            let args: serde_json::Value =
                                                serde_json::from_str(&current_tool_input)
                                                    .unwrap_or(serde_json::json!({}));
                                            let _ = tx
                                                .send(StreamEvent::ToolCall {
                                                    id: current_tool_id.clone(),
                                                    name: std::mem::take(&mut current_tool_name),
                                                    args,
                                                })
                                                .await;
                                            current_tool_input.clear();
                                        }
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }

            let _ = tx.send(StreamEvent::Done).await;
        });

        Ok(rx)
    }
}
