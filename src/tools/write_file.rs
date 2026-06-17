use crate::tools::Tool;
use anyhow::Result;
use async_trait::async_trait;
use serde_json::Value;

pub struct WriteFile;

#[async_trait]
impl Tool for WriteFile {
    fn name(&self) -> &str {
        "write_file"
    }

    fn description(&self) -> &str {
        "Write content to a file. Creates the file if it doesn't exist, overwrites if it does."
    }

    fn schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "file_path": {
                    "type": "string",
                    "description": "The path to the file to write"
                },
                "content": {
                    "type": "string",
                    "description": "The content to write to the file"
                }
            },
            "required": ["file_path", "content"]
        })
    }

    async fn call(&self, args: Value) -> Result<Value> {
        let path = args["file_path"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("missing file_path"))?;
        let content = args["content"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("missing content"))?;

        tokio::fs::write(path, content).await?;
        Ok(serde_json::json!({ "success": true }))
    }
}
