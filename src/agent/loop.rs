use crate::agent::conversation::{Message, ToolCallFunction, ToolCallMessage};
use crate::agent::llm::{LLMProvider, StreamEvent, ToolDef};
use crate::tools::ToolRegistry;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::mpsc;

pub async fn start_agent_loop(
    provider: Box<dyn LLMProvider>,
    messages: Vec<Message>,
    tools: Vec<ToolDef>,
    tool_registry: Arc<ToolRegistry>,
    tx: mpsc::Sender<StreamEvent>,
    cancelled: Arc<AtomicBool>,
) {
    let mut iteration = 0u32;
    let max_iterations = 25u32;
    let mut all_messages = messages;

    while iteration < max_iterations && !cancelled.load(Ordering::Relaxed) {
        iteration += 1;

        let mut stream = match provider.stream_chat(&all_messages, &tools).await {
            Ok(s) => s,
            Err(e) => {
                let _ = tx
                    .send(StreamEvent::Text(format!("\n\n**Agent error:** {}", e)))
                    .await;
                break;
            }
        };

        let mut tool_calls: Vec<(String, String, serde_json::Value)> = Vec::new();
        let mut response_text = String::new();

        while let Some(event) = stream.recv().await {
            match event {
                StreamEvent::Text(token) => {
                    response_text.push_str(&token);
                    let _ = tx.send(StreamEvent::Text(token)).await;
                }
                StreamEvent::ToolCall { id, name, args } => {
                    tool_calls.push((id, name, args));
                }
                StreamEvent::Usage { input_tokens, output_tokens } => {
                    let _ = tx
                        .send(StreamEvent::Usage { input_tokens, output_tokens })
                        .await;
                }
                StreamEvent::Done => break,
            }
        }

        if cancelled.load(Ordering::Relaxed) {
            let _ = tx
                .send(StreamEvent::Text("\n\n*Cancelled by user*".into()))
                .await;
            break;
        }

        if !response_text.is_empty() || !tool_calls.is_empty() {
            let tool_call_messages = if tool_calls.is_empty() {
                None
            } else {
                Some(
                    tool_calls
                        .iter()
                        .map(|(id, name, args)| ToolCallMessage {
                            id: id.clone(),
                            type_: "function".into(),
                            function: ToolCallFunction {
                                name: name.clone(),
                                arguments: args.to_string(),
                            },
                        })
                        .collect(),
                )
            };

            all_messages.push(Message {
                role: "assistant".into(),
                content: if response_text.is_empty() { None } else { Some(response_text) },
                tool_calls: tool_call_messages,
                tool_call_id: None,
            });
        }

        if tool_calls.is_empty() {
            break;
        }

        let calls_for_spawn = tool_calls.clone();
        let tx_clone = tx.clone();
        tokio::spawn(async move {
            for (_id, name, args) in calls_for_spawn {
                let content = format!("▸ **{}** `{}`", name, args);
                let _ = tx_clone.send(StreamEvent::Text(content)).await;
            }
        });

        for (id, name, args) in &tool_calls {
            if cancelled.load(Ordering::Relaxed) {
                break;
            }

            if let Some(tool) = tool_registry.get(name) {
                let result = tool.call(args.clone(), Some(tx.clone())).await;
                let content = match result {
                    Ok(output) => serde_json::to_string_pretty(&output).unwrap_or_default(),
                    Err(e) => format!("Error: {}", e),
                };
                all_messages.push(Message {
                    role: "tool".into(),
                    content: Some(content.clone()),
                    tool_calls: None,
                    tool_call_id: Some(id.clone()),
                });

                let result_event = StreamEvent::Text(format!(
                    "\n{}",
                    summarize_result(name, &content, 3)
                ));
                let _ = tx.send(result_event).await;
            }
        }
    }

    let _ = tx.send(StreamEvent::Done).await;
}

fn summarize_result(tool: &str, content: &str, max_lines: usize) -> String {
    let lines: Vec<&str> = content.lines().collect();
    let total_lines = lines.len();

    let summary = if let Ok(val) = serde_json::from_str::<serde_json::Value>(content) {
        summarize_json(&val)
    } else {
        format!("{} chars", content.len())
    };

    if total_lines <= max_lines + 1 {
        return format!(
            "◂ **{}** — {}\n```\n{}```",
            tool, summary, content
        );
    }

    let head: Vec<&str> = lines.iter().take(max_lines).copied().collect();
    let rest = total_lines - max_lines;
    format!(
        "◂ **{}** — {} ({} lines)\n```\n{}\n```\n_... {} more lines_",
        tool,
        summary,
        total_lines,
        head.join("\n"),
        rest
    )
}

fn summarize_json(val: &serde_json::Value) -> String {
    match val {
        serde_json::Value::Object(map) => {
            if map.len() == 1 {
                for (k, v) in map {
                    if let serde_json::Value::Array(arr) = v {
                        return format!("{} items in `{}`", arr.len(), k);
                    }
                    if let serde_json::Value::String(s) = v {
                        if s.len() < 80 {
                            return format!("`{}` = \"{}\"", k, s);
                        }
                        return format!("`{}` ({} chars)", k, s.len());
                    }
                }
            }
            let parts: Vec<String> = map
                .iter()
                .map(|(k, v)| match v {
                    serde_json::Value::Array(a) => format!("{}:{} items", k, a.len()),
                    serde_json::Value::String(s) => format!("{}:{}c", k, s.len()),
                    _ => format!("{}:{}", k, v),
                })
                .collect();
            parts.join(", ")
        }
        serde_json::Value::Array(arr) => format!("{} items", arr.len()),
        serde_json::Value::String(s) => {
            if s.len() <= 80 {
                format!("\"{}\"", s)
            } else {
                format!("{} chars", s.len())
            }
        }
        _ => format!("{}", val),
    }
}
