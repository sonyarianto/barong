use crate::agent::conversation::Message;
use crate::agent::llm::{LLMProvider, StreamEvent, ToolDef};
use crate::tools;
use tokio::sync::mpsc;

pub async fn start_agent_loop(
    provider: Box<dyn LLMProvider>,
    messages: Vec<Message>,
    tools: Vec<ToolDef>,
    tx: mpsc::Sender<StreamEvent>,
) {
    let mut iteration = 0u32;
    let max_iterations = 25u32;
    let mut all_messages = messages;

    while iteration < max_iterations {
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

        let mut tool_calls = Vec::new();
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
                StreamEvent::Done => break,
            }
        }

        if !response_text.is_empty() {
            all_messages.push(Message {
                role: "assistant".into(),
                content: response_text,
            });
        }

        if tool_calls.is_empty() {
            break;
        }

        let calls_for_spawn = tool_calls.clone();
        let tx_clone = tx.clone();
        tokio::spawn(async move {
            for (_id, name, args) in calls_for_spawn {
                let content = format!("**Tool: {}**\n```json\n{}\n```", name, args);
                let _ = tx_clone.send(StreamEvent::Text(content)).await;
            }
        });

        for (_id, name, args) in &tool_calls {
            all_messages.push(Message {
                role: "assistant".into(),
                content: format!("I need to use the {} tool.", name),
            });

            if let Some(tool) = tools::get_tool(name) {
                let result = tool.call(args.clone()).await;
                let content = match result {
                    Ok(output) => serde_json::to_string_pretty(&output).unwrap_or_default(),
                    Err(e) => format!("Error: {}", e),
                };
                all_messages.push(Message {
                    role: "tool".into(),
                    content: content.clone(),
                });

                let truncated = &content[..content.len().min(500)];
                let result_event = StreamEvent::Text(format!(
                    "\n\n**{} result:**\n```\n{}\n```",
                    name, truncated
                ));
                let _ = tx.send(result_event).await;
            }
        }
    }

    let _ = tx.send(StreamEvent::Done).await;
}
