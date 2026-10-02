use crate::tools::{Tool, StreamEvent};
use anyhow::Result;
use async_trait::async_trait;
use serde_json::Value;
use tokio::sync::mpsc;

pub struct EditFile;

#[async_trait]
impl Tool for EditFile {
    fn name(&self) -> &str {
        "edit"
    }

    fn aliases(&self) -> &[&str] {
        &["edit_file"]
    }

    fn description(&self) -> &str {
        "Edit a file by finding and replacing text. Use this for targeted edits."
    }

    fn schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "The path to the file to edit (alias: file_path)"
                },
                "file_path": {
                    "type": "string",
                    "description": "Deprecated alias for path"
                },
                "old_string": {
                    "type": "string",
                    "description": "The text to find and replace"
                },
                "new_string": {
                    "type": "string",
                    "description": "The text to replace it with"
                }
            },
            "required": ["old_string", "new_string"]
        })
    }

    async fn call(&self, args: Value, _tx: Option<mpsc::Sender<StreamEvent>>) -> Result<Value> {
        let path = args
            .get("path")
            .or_else(|| args.get("file_path"))
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing path (or file_path)"))?;
        let old = args["old_string"]
            .as_str()
            .or_else(|| args["old_str"].as_str())
            .or_else(|| args["old_text"].as_str())
            .ok_or_else(|| anyhow::anyhow!("missing old_string"))?;
        let new = args["new_string"]
            .as_str()
            .or_else(|| args["new_str"].as_str())
            .or_else(|| args["new_text"].as_str())
            .ok_or_else(|| anyhow::anyhow!("missing new_string"))?;

        let content = tokio::fs::read_to_string(path).await?;
        if !content.contains(old) {
            anyhow::bail!("old_string not found in file");
        }
        let new_content = content.replace(old, new);
        tokio::fs::write(path, new_content).await?;
        Ok(serde_json::json!({ "success": true }))
    }
}
