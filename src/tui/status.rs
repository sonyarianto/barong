pub struct StatusBar {
    pub llm_provider: String,
    pub tool_status: String,
    pub token_count: String,
}

impl StatusBar {
    pub fn new() -> Self {
        Self {
            llm_provider: "LLM: none".into(),
            tool_status: "idle".into(),
            token_count: "tokens: 0".into(),
        }
    }

    pub fn new_with_provider(provider: &str) -> Self {
        Self {
            llm_provider: format!("LLM: {}", provider),
            tool_status: "idle".into(),
            token_count: String::new(),
        }
    }
}
