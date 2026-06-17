use crate::tools::Tool;
use anyhow::Result;
use async_trait::async_trait;
use serde_json::Value;

pub struct ReadFile;

#[async_trait]
impl Tool for ReadFile {
    fn name(&self) -> &str {
        "read_file"
    }

    fn description(&self) -> &str {
        "Read the contents of a file. Use this to view file contents."
    }

    fn schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "file_path": {
                    "type": "string",
                    "description": "The path to the file to read"
                }
            },
            "required": ["file_path"]
        })
    }

    async fn call(&self, args: Value) -> Result<Value> {
        let path = args["file_path"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("missing file_path"))?;

        let content = tokio::fs::read_to_string(path).await?;
        Ok(serde_json::json!({ "content": content }))
    }
}
