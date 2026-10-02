use crate::agent::conversation::{Message, ToolCallFunction, ToolCallMessage};
use crate::agent::llm::{LLMProvider, StreamEvent, ToolDef};
use crate::agent::permissions::{Decision, PermissionGate};
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
    start_agent_loop_with_limit(
        provider,
        messages,
        tools,
        tool_registry,
        tx,
        cancelled,
        25,
        PermissionGate::new(true),
    )
    .await;
}

pub async fn start_agent_loop_with_limit(
    provider: Box<dyn LLMProvider>,
    messages: Vec<Message>,
    tools: Vec<ToolDef>,
    tool_registry: Arc<ToolRegistry>,
    tx: mpsc::Sender<StreamEvent>,
    cancelled: Arc<AtomicBool>,
    max_iterations: u32,
    perm: PermissionGate,
) {
    let mut iteration = 0u32;
    let max_iterations = max_iterations.clamp(1, 100);
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
                    tool_calls.push((id.clone(), name.clone(), args.clone()));
                    // Forward structured event for TUI + JSON/RPC modes.
                    let _ = tx
                        .send(StreamEvent::ToolCall { id, name, args })
                        .await;
                }
                StreamEvent::ToolResult { id, name, result } => {
                    let _ = tx.send(StreamEvent::ToolResult { id, name, result }).await;
                }
                StreamEvent::Usage { input_tokens, output_tokens } => {
                    let _ = tx
                        .send(StreamEvent::Usage { input_tokens, output_tokens })
                        .await;
                }
                StreamEvent::Done => break,
                StreamEvent::PermissionRequest { .. } | StreamEvent::PermissionResult { .. } => {}
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

        for (id, name, args) in &tool_calls {
            if cancelled.load(Ordering::Relaxed) {
                break;
            }

            // Permission gate: mutating tools need explicit approval unless
            // auto-approve is on or the tool was allowed for this session.
            if PermissionGate::requires_approval(name)
                && !perm.auto_approve()
                && !perm.is_session_allowed(name)
            {
                perm.request(id.clone(), name.clone(), args.clone());
                let _ = tx
                    .send(StreamEvent::PermissionRequest {
                        id: id.clone(),
                        name: name.clone(),
                        args: args.clone(),
                    })
                    .await;
                let approved = wait_for_approval(&perm, id, &cancelled).await;
                let _ = tx
                    .send(StreamEvent::PermissionResult {
                        id: id.clone(),
                        approved,
                    })
                    .await;
                if !approved {
                    let value =
                        serde_json::json!({ "error": format!("denied by user: {}", name) });
                    all_messages.push(Message {
                        role: "tool".into(),
                        content: Some(value.to_string()),
                        tool_calls: None,
                        tool_call_id: Some(id.clone()),
                    });
                    let _ = tx
                        .send(StreamEvent::ToolResult {
                            id: id.clone(),
                            name: name.clone(),
                            result: value,
                        })
                        .await;
                    continue;
                }
            }

            if let Some(tool) = tool_registry.get(name) {
                let result = tool.call(args.clone(), Some(tx.clone())).await;
                let value = match result {
                    Ok(output) => output,
                    Err(e) => serde_json::json!({ "error": e.to_string() }),
                };
                let content = serde_json::to_string_pretty(&value).unwrap_or_default();
                all_messages.push(Message {
                    role: "tool".into(),
                    content: Some(content.clone()),
                    tool_calls: None,
                    tool_call_id: Some(id.clone()),
                });

                // Structured result for JSON/RPC + TUI rendering.
                let _ = tx
                    .send(StreamEvent::ToolResult {
                        id: id.clone(),
                        name: name.clone(),
                        result: value,
                    })
                    .await;
            } else {
                let err = serde_json::json!({ "error": format!("unknown tool: {}", name) });
                all_messages.push(Message {
                    role: "tool".into(),
                    content: Some(err.to_string()),
                    tool_calls: None,
                    tool_call_id: Some(id.clone()),
                });
                let _ = tx
                    .send(StreamEvent::ToolResult {
                        id: id.clone(),
                        name: name.clone(),
                        result: err,
                    })
                    .await;
            }
        }
    }

    let _ = tx.send(StreamEvent::Done).await;
}

async fn wait_for_approval(
    perm: &PermissionGate,
    id: &str,
    cancelled: &Arc<AtomicBool>,
) -> bool {
    loop {
        if cancelled.load(Ordering::Relaxed) {
            return false;
        }
        if let Some(d) = perm.poll(id) {
            return match d {
                Decision::AllowOnce | Decision::AllowSession => true,
                Decision::Deny => false,
            };
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
}

/// Human-readable one-liner for a tool call (`read src/main.rs:1-200`
/// instead of raw JSON). First line feeds the collapsed view; the raw args
/// follow for the expanded (Ctrl+O) view.
pub fn describe_tool_call(name: &str, args: &serde_json::Value) -> String {
    fn s(v: &serde_json::Value, keys: &[&str]) -> Option<String> {
        keys.iter()
            .filter_map(|k| v.get(k).and_then(|x| x.as_str()))
            .next()
            .map(|x| x.to_string())
    }
    fn n(v: &serde_json::Value, keys: &[&str]) -> Option<u64> {
        keys.iter().filter_map(|k| v.get(k).and_then(|x| x.as_u64())).next()
    }
    let summary = match name {
        "read" => {
            let p = s(args, &["path", "file_path"]).unwrap_or_else(|| "?".into());
            if let Some(pat) = args.get("pattern").and_then(|v| v.as_str()) {
                format!("{} ({})", p, pat)
            } else {
                let start = n(args, &["offset", "line_start", "start_line"]).unwrap_or(1).max(1);
                let end = n(args, &["line_end", "end_line"])
                    .or_else(|| n(args, &["limit"]).map(|l| start + l.saturating_sub(1)));
                match end {
                    Some(e) if e != start => format!("{}:{}-{}", p, start, e),
                    _ if start > 1 => format!("{}:{}-…", p, start),
                    _ => p,
                }
            }
        }
        "write" => {
            let p = s(args, &["path", "file_path"]).unwrap_or_else(|| "?".into());
            let len = args
                .get("content")
                .or_else(|| args.get("text"))
                .and_then(|v| v.as_str())
                .map(|t| t.chars().count())
                .unwrap_or(0);
            format!("{} ({} chars)", p, len)
        }
        "edit" => s(args, &["path", "file_path"]).unwrap_or_else(|| "?".into()),
        "bash" => s(args, &["command", "cmd"]).map(|c| truncate_middle(&c, 100)).unwrap_or_else(|| "?".into()),
        "grep" | "search" => {
            let pat = s(args, &["pattern"]).unwrap_or_else(|| "?".into());
            let dir = s(args, &["path"]).unwrap_or_else(|| ".".into());
            format!("{} in {}", pat, dir)
        }
        "glob" => {
            let pat = s(args, &["pattern"]).unwrap_or_else(|| "*".into());
            let dir = s(args, &["path"]).unwrap_or_else(|| ".".into());
            format!("{} in {}", pat, dir)
        }
        "delegate" => s(args, &["task"]).map(|t| truncate_middle(&t, 80)).unwrap_or_else(|| "?".into()),
        _ => truncate_middle(&args.to_string(), 100),
    };
    let raw = serde_json::to_string_pretty(args).unwrap_or_default();
    format!("▸ **{}** {}\n```json\n{}\n```", name, summary, raw)
}

fn truncate_middle(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    format!("{}…", s.chars().take(max.saturating_sub(1)).collect::<String>())
}

pub fn summarize_result(tool: &str, content: &str, max_lines: usize) -> String {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn first_line(s: &str) -> &str {
        s.lines().next().unwrap_or("")
    }

    #[test]
    fn describe_read_variants() {
        // The exact call from the bug report.
        let args = serde_json::json!({"line_end": 200, "line_start": 1, "path": "src/main.rs"});
        assert!(first_line(&describe_tool_call("read", &args)).contains("src/main.rs:1-200"), "{}", describe_tool_call("read", &args));
        let args = serde_json::json!({"path": "src/main.rs", "offset": 50, "limit": 10});
        assert!(first_line(&describe_tool_call("read", &args)).contains("src/main.rs:50-59"));
        let args = serde_json::json!({"path": "src"});
        assert!(first_line(&describe_tool_call("read", &args)).contains("src"));
        assert!(!first_line(&describe_tool_call("read", &args)).contains('{'), "first line must be clean");
    }

    #[test]
    fn describe_other_tools() {
        let args = serde_json::json!({"path": "a.txt", "content": "hi"});
        assert!(first_line(&describe_tool_call("write", &args)).contains("a.txt (2 chars)"));
        let args = serde_json::json!({"command": "cargo test"});
        assert!(first_line(&describe_tool_call("bash", &args)).contains("cargo test"));
        let args = serde_json::json!({"pattern": "foo", "path": "src"});
        assert!(first_line(&describe_tool_call("grep", &args)).contains("foo in src"));
        // Raw JSON stays available for the expanded view.
        let full = describe_tool_call("read", &serde_json::json!({"path": "x"}));
        assert!(full.contains("\"path\""));
    }
}
