use crate::tools::Tool;
use anyhow::Result;
use async_trait::async_trait;
use serde_json::Value;

pub struct EditFile;

#[async_trait]
impl Tool for EditFile {
    fn name(&self) -> &str {
        "edit_file"
    }

    fn description(&self) -> &str {
        "Edit a file by finding and replacing text. Use this for targeted edits."
    }

    fn schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "file_path": {
                    "type": "string",
                    "description": "The path to the file to edit"
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
            "required": ["file_path", "old_string", "new_string"]
        })
    }

    async fn call(&self, args: Value) -> Result<Value> {
        let path = args["file_path"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("missing file_path"))?;
        let old = args["old_string"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("missing old_string"))?;
        let new = args["new_string"]
            .as_str()
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
