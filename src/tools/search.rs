use crate::tools::{Tool, StreamEvent};
use anyhow::Result;
use async_trait::async_trait;
use serde_json::Value;
use tokio::sync::mpsc;

pub struct Search;

#[async_trait]
impl Tool for Search {
    fn name(&self) -> &str {
        "grep"
    }

    fn aliases(&self) -> &[&str] {
        &["search"]
    }

    fn description(&self) -> &str {
        "Search for text patterns across files (extra tool: prefer bash+rg when possible)."
    }

    fn schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "The regex pattern to search for"
                },
                "path": {
                    "type": "string",
                    "description": "The directory to search in (default: current directory)"
                },
                "include": {
                    "type": "string",
                    "description": "File pattern to filter (e.g. *.rs, *.{ts,tsx})"
                }
            },
            "required": ["pattern"]
        })
    }

    async fn call(&self, args: Value, _tx: Option<mpsc::Sender<StreamEvent>>) -> Result<Value> {
        let pattern = args["pattern"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("missing pattern"))?;
        let search_path = args.get("path").and_then(|v| v.as_str()).unwrap_or(".");
        let include = args.get("include").and_then(|v| v.as_str());

        // Prefer rg, fall back to grep -rn for portability.
        if which_rg() {
            let mut cmd = tokio::process::Command::new("rg");
            cmd.arg("--json");
            cmd.arg("-n");
            cmd.arg(pattern);
            cmd.arg(search_path);

            if let Some(ext) = include {
                cmd.arg("--glob");
                cmd.arg(ext);
            }

            let output = cmd.output().await?;
            let stdout = String::from_utf8_lossy(&output.stdout).to_string();

            if stdout.is_empty() {
                return Ok(serde_json::json!({ "results": [] }));
            }

            let mut matches = Vec::new();
            for line in stdout.lines() {
                if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(line) {
                    if parsed["type"] == "match" {
                        matches.push(serde_json::json!({
                            "path": parsed["data"]["path"]["text"],
                            "line_number": parsed["data"]["line_number"],
                            "line": parsed["data"]["lines"]["text"],
                        }));
                    }
                }
            }

            return Ok(serde_json::json!({ "results": matches }));
        }

        let mut cmd = tokio::process::Command::new("grep");
        cmd.args(["-rn", "--exclude-dir=.git", "--exclude-dir=target", "--exclude-dir=node_modules"]);
        if let Some(ext) = include {
            cmd.args(["--include", ext]);
        }
        cmd.arg(pattern);
        cmd.arg(search_path);
        let output = cmd.output().await?;
        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let mut matches = Vec::new();
        for line in stdout.lines().take(200) {
            matches.push(serde_json::json!({ "line": line }));
        }
        Ok(serde_json::json!({ "results": matches }))
    }
}

fn which_rg() -> bool {
    std::process::Command::new("rg")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}
