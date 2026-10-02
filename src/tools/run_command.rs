use crate::tools::{Tool, StreamEvent};
use anyhow::Result;
use async_trait::async_trait;
use serde_json::Value;
use tokio::sync::mpsc;

pub struct RunCommand;

#[async_trait]
impl Tool for RunCommand {
    fn name(&self) -> &str {
        "bash"
    }

    fn aliases(&self) -> &[&str] {
        &["run_command"]
    }

    fn description(&self) -> &str {
        "Execute a shell command and return its output. Use this to run builds, tests, grep, or any shell command."
    }

    fn schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "The shell command to execute (alias: cmd)"
                },
                "cmd": {
                    "type": "string",
                    "description": "Deprecated alias for command"
                },
                "workdir": {
                    "type": "string",
                    "description": "Working directory for the command (default: current directory)"
                },
                "timeout": {
                    "type": "integer",
                    "description": "Timeout in seconds (default: 30, max: 300)"
                }
            },
            "required": []
        })
    }

    async fn call(&self, args: Value, _tx: Option<mpsc::Sender<StreamEvent>>) -> Result<Value> {
        let command = args
            .get("command")
            .or_else(|| args.get("cmd"))
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing command (or cmd)"))?;
        let workdir = args.get("workdir").and_then(|v| v.as_str()).unwrap_or(".");
        let timeout_secs = args
            .get("timeout")
            .and_then(|v| v.as_u64())
            .unwrap_or(30)
            .clamp(1, 300);

        let output = run_shell(command, workdir, timeout_secs).await?;

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();

        Ok(serde_json::json!({
            "stdout": stdout,
            "stderr": stderr,
            "exit_code": output.status.code().unwrap_or(-1),
        }))
    }
}

async fn run_shell(
    command: &str,
    workdir: &str,
    timeout_secs: u64,
) -> Result<std::process::Output> {
    #[cfg(target_os = "windows")]
    let mut cmd = {
        let mut c = tokio::process::Command::new("pwsh");
        c.args(["-NoProfile", "-Command", command]);
        c
    };
    #[cfg(not(target_os = "windows"))]
    let mut cmd = {
        let mut c = tokio::process::Command::new("sh");
        c.args(["-c", command]);
        c
    };

    cmd.current_dir(workdir);
    let fut = cmd.output();
    match tokio::time::timeout(std::time::Duration::from_secs(timeout_secs), fut).await {
        Ok(res) => Ok(res?),
        Err(_) => anyhow::bail!("command timed out after {}s", timeout_secs),
    }
}
