use crate::tools::{StreamEvent, Tool};
use anyhow::{anyhow, Result};
use serde_json::Value;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use tokio::sync::mpsc;

pub struct McpServer {
    _name: String,
    child: Child,
    stdin: ChildStdin,
    next_id: u64,
}

impl McpServer {
    pub fn spawn(name: &str, cmd: &str, args: &[String]) -> Result<Self> {
        let mut child = Command::new(cmd)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()?;

        let stdin = child.stdin.take().ok_or_else(|| anyhow!("no stdin"))?;
        let mut server = Self {
            _name: name.to_string(),
            child,
            stdin,
            next_id: 1,
        };

        server.initialize()?;
        Ok(server)
    }

    fn initialize(&mut self) -> Result<()> {
        let req = self.build_request("initialize", serde_json::json!({
            "protocolVersion": "2025-03-26",
            "capabilities": {},
            "clientInfo": { "name": "barong", "version": "0.1.0" },
        }));
        self.send(&req)?;
        let _resp = self.read_response()?;
        self.send(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#)?;
        Ok(())
    }

    pub fn list_tools(&mut self) -> Result<Vec<McpToolDef>> {
        let req = self.build_request("tools/list", serde_json::json!({}));
        self.send(&req)?;
        let raw = self.read_response()?;
        let parsed: Value = serde_json::from_str(&raw)?;
        let tools = parsed["result"]["tools"]
            .as_array()
            .ok_or_else(|| anyhow!("no tools in response"))?;

        let mut result = Vec::new();
        for t in tools {
            result.push(McpToolDef {
                name: t["name"].as_str().unwrap_or("").to_string(),
                description: t["description"].as_str().unwrap_or("").to_string(),
                input_schema: t["inputSchema"].clone(),
            });
        }
        Ok(result)
    }

    pub fn call_tool(&mut self, tool_name: &str, args: Value) -> Result<Value> {
        let req = self.build_request("tools/call", serde_json::json!({
            "name": tool_name,
            "arguments": args,
        }));
        self.send(&req)?;

        loop {
            let raw = self.read_response()?;
            let parsed: Value = serde_json::from_str(&raw)?;

            if parsed.get("result").is_some() {
                return Ok(parsed["result"].clone());
            }
            if let Some(err) = parsed.get("error") {
                return Err(anyhow!("MCP error: {}", err));
            }
        }
    }

    fn build_request(&mut self, method: &str, params: Value) -> String {
        let id = self.next_id;
        self.next_id += 1;
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        })
        .to_string()
    }

    fn send(&mut self, msg: &str) -> Result<()> {
        writeln!(self.stdin, "{}", msg)?;
        self.stdin.flush()?;
        Ok(())
    }

    fn read_response(&mut self) -> Result<String> {
        let stdout = self.child.stdout.as_mut().ok_or_else(|| anyhow!("no stdout"))?;
        let mut reader = BufReader::new(stdout);
        let mut line = String::new();
        loop {
            line.clear();
            let n = reader.read_line(&mut line)?;
            if n == 0 {
                return Err(anyhow!("MCP server closed connection"));
            }
            let trimmed = line.trim();
            if !trimmed.is_empty() {
                return Ok(trimmed.to_string());
            }
        }
    }
}

impl Drop for McpServer {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

pub struct McpToolDef {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

pub struct McpToolAdapter {
    pub server_name: String,
    pub tool_name: String,
    pub description: String,
    pub schema: Value,
    pub server: std::sync::Arc<std::sync::Mutex<McpServer>>,
}

#[async_trait::async_trait]
impl Tool for McpToolAdapter {
    fn name(&self) -> &str {
        &self.tool_name
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn schema(&self) -> Value {
        self.schema.clone()
    }

    async fn call(&self, args: Value, _tx: Option<mpsc::Sender<StreamEvent>>) -> Result<Value> {
        let mut server = self.server.lock().unwrap();
        server.call_tool(&self.tool_name, args)
    }
}
