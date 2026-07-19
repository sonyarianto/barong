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
        "List files recursively matching a path pattern. Use this to discover project structure."
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

        let output = tokio::process::Command::new("pwsh")
            .args([
                "-NoProfile",
                "-Command",
                &format!(
                    "Get-ChildItem -Path '{}' -Recurse -Filter '{}' -Force -ErrorAction SilentlyContinue \
                     | Where-Object {{ !$_.PSIsContainer }} \
                     | Resolve-Path -Relative \
                     | ForEach-Object {{ $_ -replace '^\\\\\\.\\\\', '' }}",
                    search_path, pattern
                ),
            ])
            .output()
            .await?;

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let files: Vec<String> = stdout
            .lines()
            .map(|l| l.trim().to_string())
            .filter(|l| !l.is_empty())
            .collect();

        Ok(serde_json::json!({ "files": files }))
    }
}
