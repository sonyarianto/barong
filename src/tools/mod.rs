pub mod read_file;
pub mod write_file;
pub mod edit_file;
pub mod search;
pub mod run_command;

use anyhow::Result;
use serde_json::Value;
use std::collections::HashMap;

#[async_trait::async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn schema(&self) -> Value;
    async fn call(&self, args: Value) -> Result<Value>;
}

pub struct ToolRegistry {
    tools: HashMap<String, Box<dyn Tool>>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        let mut registry = Self {
            tools: HashMap::new(),
        };
        registry.register(Box::new(read_file::ReadFile));
        registry.register(Box::new(write_file::WriteFile));
        registry.register(Box::new(edit_file::EditFile));
        registry.register(Box::new(search::Search));
        registry.register(Box::new(run_command::RunCommand));
        registry
    }

    pub fn register(&mut self, tool: Box<dyn Tool>) {
        let name = tool.name().to_string();
        self.tools.insert(name, tool);
    }

    pub fn get(&self, name: &str) -> Option<&dyn Tool> {
        self.tools.get(name).map(|b| b.as_ref())
    }

    pub fn definitions(&self) -> Vec<crate::agent::llm::ToolDef> {
        self.tools
            .values()
            .map(|t| crate::agent::llm::ToolDef {
                name: t.name().to_string(),
                description: t.description().to_string(),
                input_schema: t.schema(),
            })
            .collect()
    }
}
