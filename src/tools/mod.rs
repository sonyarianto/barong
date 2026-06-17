pub mod read_file;
pub mod write_file;
pub mod edit_file;
pub mod search;
pub mod run_command;

use crate::agent::llm::ToolDef;
use anyhow::Result;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::OnceLock;

#[async_trait::async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn schema(&self) -> Value;
    async fn call(&self, args: Value) -> Result<Value>;
}

static GLOBAL_TOOLS: OnceLock<HashMap<String, Box<dyn Tool>>> = OnceLock::new();

fn global_tools() -> &'static HashMap<String, Box<dyn Tool>> {
    GLOBAL_TOOLS.get_or_init(|| {
        let mut m: HashMap<String, Box<dyn Tool>> = HashMap::new();
        m.insert("read_file".into(), Box::new(read_file::ReadFile));
        m.insert("write_file".into(), Box::new(write_file::WriteFile));
        m.insert("edit_file".into(), Box::new(edit_file::EditFile));
        m.insert("search".into(), Box::new(search::Search));
        m.insert("run_command".into(), Box::new(run_command::RunCommand));
        m
    })
}

pub fn get_tool(name: &str) -> Option<&'static dyn Tool> {
    global_tools().get(name).map(|b| b.as_ref())
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

    pub fn definitions(&self) -> Vec<ToolDef> {
        self.tools
            .values()
            .map(|t| ToolDef {
                name: t.name().to_string(),
                description: t.description().to_string(),
                input_schema: t.schema(),
            })
            .collect()
    }
}
