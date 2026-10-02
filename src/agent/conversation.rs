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

    /// Drop oldest messages, keep last `keep`, insert an extractive summary
    /// so the LLM still sees what happened. Returns (dropped, summary).
    pub fn compact(&mut self, keep: usize) -> Option<(usize, String)> {
        let n = self.messages.len();
        if n <= keep {
            return None;
        }
        let drop = n - keep;
        let dropped: Vec<Message> = self.messages.drain(..drop).collect();
        let summary = summarize_messages(&dropped);
        self.messages.insert(
            0,
            Message {
                role: "assistant".into(),
                content: Some(format!(
                    "[Compacted {} older messages]\n{}",
                    drop, summary
                )),
                tool_calls: None,
                tool_call_id: None,
            },
        );
        Some((drop, format!("Compacted: dropped {} messages, kept {}", drop, keep)))
    }
}

fn one_line(s: &str, max: usize) -> String {
    let t: String = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if t.chars().count() <= max {
        return t;
    }
    let short: String = t.chars().take(max.saturating_sub(1)).collect();
    format!("{}…", short)
}

fn summarize_messages(msgs: &[Message]) -> String {
    let mut lines = Vec::new();
    for m in msgs.iter().take(40) {
        let body = m.content.as_deref().unwrap_or("");
        if body.trim().is_empty() {
            continue;
        }
        // Skip previous compaction markers to avoid nesting bloat.
        if body.starts_with("[Compacted") {
            lines.push("- (earlier compaction)".to_string());
            continue;
        }
        let role = match m.role.as_str() {
            "user" => "user",
            "assistant" => "assistant",
            "tool" => "tool",
            other => other,
        };
        lines.push(format!("- {}: {}", role, one_line(body, 140)));
        if lines.join("\n").len() > 2000 {
            lines.push(format!("- … ({} more dropped, truncated)", msgs.len().saturating_sub(lines.len())));
            break;
        }
    }
    if lines.is_empty() {
        "(no text content)".to_string()
    } else {
        lines.join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compact_keeps_last_and_summarizes() {
        let mut c = Conversation::new();
        for i in 0..10 {
            c.add_message("user".into(), format!("msg {}", i));
        }
        let res = c.compact(4);
        assert!(res.is_some());
        // 4 kept + 1 summary + 1 report? compact() only adds summary (report added by caller).
        // Our compact adds 1 summary marker, keeps 4 => total 5.
        assert_eq!(c.messages.len(), 5);
        assert!(c.messages[0].content.as_deref().unwrap_or("").starts_with("[Compacted 6 older"));
    }

    #[test]
    fn compact_noop_when_short() {
        let mut c = Conversation::new();
        c.add_message("user".into(), "hi".into());
        assert!(c.compact(10).is_none());
    }
}
