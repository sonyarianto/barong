use crate::tools::{Tool, StreamEvent};
use anyhow::Result;
use async_trait::async_trait;
use serde_json::Value;
use tokio::sync::mpsc;

pub struct ReadFile;

#[async_trait]
impl Tool for ReadFile {
    fn name(&self) -> &str {
        "read"
    }

    fn aliases(&self) -> &[&str] {
        &["read_file"]
    }

    fn description(&self) -> &str {
        "Read a file or list a directory. Use offset/limit for large files. Use pattern to filter directory listing."
    }

    fn schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "File or directory path to read (alias: file_path)"
                },
                "file_path": {
                    "type": "string",
                    "description": "Deprecated alias for path"
                },
                "offset": {
                    "type": "integer",
                    "description": "Line offset to start from (1-indexed, default: 1)"
                },
                "limit": {
                    "type": "integer",
                    "description": "Max lines to return (default: all, capped at 2000)"
                },
                "pattern": {
                    "type": "string",
                    "description": "When path is a directory, filter files by substring/glob (e.g. *.rs)"
                }
            },
            "required": []
        })
    }

    async fn call(&self, args: Value, _tx: Option<mpsc::Sender<StreamEvent>>) -> Result<Value> {
        let raw_path = args
            .get("path")
            .or_else(|| args.get("file_path"))
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing path (or file_path)"))?;

        let p = std::path::Path::new(raw_path);
        if p.is_dir() {
            let pattern = args
                .get("pattern")
                .and_then(|v| v.as_str())
                .unwrap_or("*");
            let files = list_dir_filtered(p, pattern, 200)?;
            return Ok(serde_json::json!({ "files": files, "path": raw_path }));
        }

        let content = tokio::fs::read_to_string(raw_path).await?;
        let lines: Vec<&str> = content.lines().collect();
        let total = lines.len();
        let offset = args
            .get("offset")
            .and_then(|v| v.as_u64())
            .unwrap_or(1)
            .max(1) as usize;
        let limit = args
            .get("limit")
            .and_then(|v| v.as_u64())
            .unwrap_or(2000)
            .min(2000) as usize;

        if offset == 1 && limit >= total {
            return Ok(serde_json::json!({ "content": content, "total_lines": total }));
        }

        let start = (offset - 1).min(total);
        let end = (start + limit).min(total);
        let sliced = lines[start..end].join("\n");
        Ok(serde_json::json!({
            "content": sliced,
            "offset": offset,
            "total_lines": total,
            "truncated": end < total,
        }))
    }
}

fn matches_pattern(name: &str, pattern: &str) -> bool {
    if pattern == "*" || pattern.is_empty() {
        return true;
    }
    // minimal glob: support * substring, *.ext, prefix*
    if pattern.starts_with('*') && pattern.ends_with('*') && pattern.len() > 1 {
        let inner = &pattern[1..pattern.len() - 1];
        return name.contains(inner);
    }
    if let Some(ext) = pattern.strip_prefix("*.") {
        return name.ends_with(ext);
    }
    if let Some(prefix) = pattern.strip_suffix('*') {
        return name.starts_with(prefix);
    }
    name.contains(pattern)
}

fn list_dir_filtered(dir: &std::path::Path, pattern: &str, cap: usize) -> Result<Vec<String>> {
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
            if name.starts_with('.') {
                continue;
            }
            if name == "target" || name == "node_modules" || name == ".git" {
                continue;
            }
            if path.is_dir() {
                stack.push(path);
            } else if matches_pattern(&name, pattern) {
                out.push(path.to_string_lossy().to_string());
            }
        }
        if out.len() >= cap {
            break;
        }
    }
    out.sort();
    Ok(out)
}
