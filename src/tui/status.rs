pub struct StatusBar {
    pub llm_provider: String,
    pub model: String,
    pub tool_status: String,
    pub token_count: String,
}

impl StatusBar {
    pub fn new() -> Self {
        Self {
            llm_provider: "none".into(),
            model: "none".into(),
            tool_status: "idle".into(),
            token_count: "tokens: 0".into(),
        }
    }

    pub fn new_with_provider(provider: &str, model: &str) -> Self {
        Self {
            llm_provider: format!("{}", provider),
            model: model.into(),
            tool_status: "idle".into(),
            token_count: String::new(),
        }
    }
}
