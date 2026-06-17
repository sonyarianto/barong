pub struct MarkdownRenderer;

impl MarkdownRenderer {
    pub fn new() -> Self {
        Self
    }

    pub fn render(&self, text: &str) -> String {
        text.to_string()
    }
}
