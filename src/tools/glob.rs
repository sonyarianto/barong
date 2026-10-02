use crate::tools::{Tool, StreamEvent};
use anyhow::Result;
use async_trait::async_trait;
use serde_json::Value;
use tokio::sync::mpsc;

pub struct Glob;

#[async_trait]
impl Tool for Glob {
    fn name(&self) -> &str {
        "glob"
    }

    fn description(&self) -> &str {
        "List files matching a pattern (extra tool: prefer read <dir> or bash)."
    }

    fn schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "File pattern filter (e.g. *.rs, *.ts, *test*). Use * for all."
                },
                "path": {
                    "type": "string",
                    "description": "The directory to search in (default: current directory)"
                }
            },
            "required": ["pattern"]
        })
    }

    async fn call(&self, args: Value, _tx: Option<mpsc::Sender<StreamEvent>>) -> Result<Value> {
        let pattern = args["pattern"].as_str().unwrap_or("*");
        let search_path = args.get("path").and_then(|v| v.as_str()).unwrap_or(".");

        let files = walk_filtered(std::path::Path::new(search_path), pattern, 500)?;
        Ok(serde_json::json!({ "files": files }))
    }
}

fn matches(name: &str, pattern: &str) -> bool {
    if pattern == "*" || pattern.is_empty() {
        return true;
    }
    if let Some(ext) = pattern.strip_prefix("*.") {
        // support "*.rs" and "*.{ts,tsx}"
        if ext.starts_with('{') && ext.ends_with('}') {
            let inner = &ext[1..ext.len() - 1];
            return inner.split(',').any(|e| name.ends_with(e.trim()));
        }
        return name.ends_with(ext);
    }
    if pattern.starts_with('*') && pattern.ends_with('*') && pattern.len() > 1 {
        return name.contains(&pattern[1..pattern.len() - 1]);
    }
    if let Some(prefix) = pattern.strip_suffix('*') {
        return name.starts_with(prefix);
    }
    name.contains(pattern)
}

fn walk_filtered(dir: &std::path::Path, pattern: &str, cap: usize) -> Result<Vec<String>> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let entries = match std::fs::read_dir(&d) {
            Ok(e) => e,
            Err(_) => continue,
        };
        for e in entries.flatten() {
            if out.len() >= cap {
                break;
            }
            let path = e.path();
            let name = e.file_name().to_string_lossy().to_string();
            if name.starts_with('.') || name == "target" || name == "node_modules" {
                if path.is_dir() {
                    continue;
                }
            }
            if path.is_dir() {
                stack.push(path);
            } else if matches(&name, pattern) {
                out.push(path.to_string_lossy().to_string());
            }
        }
    }
    out.sort();
    Ok(out)
}
