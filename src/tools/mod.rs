pub mod read_file;
pub mod write_file;
pub mod edit_file;
pub mod search;
pub mod run_command;
pub mod glob;
pub mod delegate;

pub use crate::agent::llm::StreamEvent;
use crate::agent::llm::ToolDef;
use anyhow::Result;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::OnceLock;
use tokio::sync::mpsc;

#[async_trait::async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn aliases(&self) -> &[&str] {
        &[]
    }
    fn description(&self) -> &str;
    fn schema(&self) -> Value;
    async fn call(&self, args: Value, tx: Option<mpsc::Sender<StreamEvent>>) -> Result<Value>;
    /// Core tools are always on, extras are opt-in.
    fn is_core(&self) -> bool {
        matches!(self.name(), "read" | "write" | "edit" | "bash")
    }
}

static GLOBAL_TOOLS: OnceLock<HashMap<String, Box<dyn Tool>>> = OnceLock::new();

fn global_tools() -> &'static HashMap<String, Box<dyn Tool>> {
    GLOBAL_TOOLS.get_or_init(|| {
        let mut m: HashMap<String, Box<dyn Tool>> = HashMap::new();
        m.insert("read".into(), Box::new(read_file::ReadFile));
        m.insert("write".into(), Box::new(write_file::WriteFile));
        m.insert("edit".into(), Box::new(edit_file::EditFile));
        m.insert("bash".into(), Box::new(run_command::RunCommand));
        m.insert("grep".into(), Box::new(search::Search));
        m.insert("glob".into(), Box::new(glob::Glob));
        m
    })
}

pub fn get_tool(name: &str) -> Option<&'static dyn Tool> {
    global_tools().get(name).map(|b| b.as_ref())
}

pub struct ToolRegistry {
    tools: HashMap<String, Box<dyn Tool>>,
    aliases: HashMap<String, String>,
}

impl ToolRegistry {
    /// Minimal core: read, write, edit, bash.
    pub fn new() -> Self {
        Self::core()
    }

    pub fn core() -> Self {
        let mut registry = Self {
            tools: HashMap::new(),
            aliases: HashMap::new(),
        };
        registry.register(Box::new(read_file::ReadFile));
        registry.register(Box::new(write_file::WriteFile));
        registry.register(Box::new(edit_file::EditFile));
        registry.register(Box::new(run_command::RunCommand));
        registry
    }

    /// Register opt-in extras: grep (=search), glob, delegate.
    /// `extras` entries are case-insensitive names like "grep", "glob", "delegate", "all".
    pub fn with_extras(mut self, extras: &[String]) -> Self {
        let wants = |n: &str| {
            extras
                .iter()
                .any(|e| e.eq_ignore_ascii_case(n) || e.eq_ignore_ascii_case("all"))
        };
        if wants("grep") || wants("search") {
            self.register(Box::new(search::Search));
        }
        if wants("glob") {
            self.register(Box::new(glob::Glob));
        }
        self
    }

    pub fn register_delegate(
        &mut self,
        api_key: String,
        model: String,
        base_url: String,
        provider: crate::agent::llm::ProviderKind,
    ) {
        self.register(Box::new(delegate::Delegate::new(api_key, model, base_url, provider)));
    }

    pub fn register(&mut self, tool: Box<dyn Tool>) {
        for a in tool.aliases() {
            self.aliases.insert(a.to_string(), tool.name().to_string());
        }
        let name = tool.name().to_string();
        self.tools.insert(name, tool);
    }

    pub fn get(&self, name: &str) -> Option<&dyn Tool> {
        if let Some(t) = self.tools.get(name) {
            return Some(t.as_ref());
        }
        if let Some(canonical) = self.aliases.get(name) {
            return self.tools.get(canonical).map(|b| b.as_ref());
        }
        None
    }

    pub fn tool_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self.tools.keys().cloned().collect();
        names.sort();
        names
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn core_is_minimal_four() {
        let r = ToolRegistry::core();
        let mut names = r.tool_names();
        names.sort();
        assert_eq!(names, vec!["bash", "edit", "read", "write"]);
    }

    #[test]
    fn aliases_resolve_legacy_names() {
        let r = ToolRegistry::core();
        assert!(r.get("read_file").is_some());
        assert!(r.get("write_file").is_some());
        assert!(r.get("edit_file").is_some());
        assert!(r.get("run_command").is_some());
        assert_eq!(r.get("read_file").unwrap().name(), "read");
        assert_eq!(r.get("run_command").unwrap().name(), "bash");
    }

    #[test]
    fn extras_opt_in() {
        let r = ToolRegistry::core().with_extras(&["grep".to_string(), "glob".to_string()]);
        assert!(r.get("grep").is_some());
        assert!(r.get("search").is_some()); // alias
        assert!(r.get("glob").is_some());
        assert!(r.get("delegate").is_none());
    }
}
