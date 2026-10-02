use crate::agent::conversation::Conversation;
use crate::agent::llm::{AnthropicProvider, LLMProvider, OpenAIProvider, ProviderKind, StreamEvent};
use crate::agent::r#loop::start_agent_loop_with_limit;
use crate::cli::Cli;
use crate::config::Config;
use crate::mcp::{McpServer, McpToolAdapter};
use crate::session::SessionManager;
use crate::tools::ToolRegistry;
use crate::workspace::WorkspaceContext;
use anyhow::Result;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use tokio::sync::mpsc;

pub struct HeadlessCtx {
    pub config: Config,
    pub provider_kind: ProviderKind,
    pub model: String,
    pub tool_registry: Arc<ToolRegistry>,
    // keep MCP servers alive for the run
    #[allow(dead_code)]
    pub _mcp_servers: Vec<Arc<std::sync::Mutex<McpServer>>>,
    pub workspace: WorkspaceContext,
    pub system_prompt: String,
    pub max_iterations: u32,
}

pub fn build_ctx(cli: &Cli) -> Result<HeadlessCtx> {
    let mut config = Config::load();
    if let Some(p) = &cli.provider {
        config.provider = Some(p.clone());
    }
    if let Some(m) = &cli.model {
        config.model = Some(m.clone());
    }
    if let Some(t) = &cli.tools {
        let extras: Vec<String> = t
            .split([',', ' '])
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .collect();
        let mut base = config.resolve_extra_tools();
        base.extend(extras);
        config.extra_tools = base;
    }

    let provider_str = config.resolve_provider();
    let provider_kind = ProviderKind::from_str(&provider_str);
    let model = config.resolve_model(&provider_str);
    let extras = config.resolve_extra_tools();

    let mut registry = ToolRegistry::new().with_extras(&extras);
    // delegate is opt-in extra
    if extras.iter().any(|e| {
        e.eq_ignore_ascii_case("delegate") || e.eq_ignore_ascii_case("all")
    }) {
        registry.register_delegate(
            config.resolve_api_key(&provider_str),
            config.resolve_model(&provider_str),
            config.resolve_base_url(),
            provider_kind,
        );
    }
    // MCP servers (only those configured; failures warn)
    let mut mcp_servers = Vec::new();
    for sc in &config.mcp_servers {
        match McpServer::spawn(&sc.name, &sc.command, &sc.args) {
            Ok(server) => {
                let server = Arc::new(std::sync::Mutex::new(server));
                if let Ok(mut locked) = server.lock() {
                    if let Ok(tools) = locked.list_tools() {
                        for t in tools {
                            registry.register(Box::new(McpToolAdapter {
                                server_name: sc.name.clone(),
                                tool_name: t.name,
                                description: t.description,
                                schema: t.input_schema,
                                server: server.clone(),
                            }));
                        }
                    }
                }
                mcp_servers.push(server);
            }
            Err(e) => tracing::warn!("MCP '{}' failed: {}", sc.name, e),
        }
    }

    let workspace = WorkspaceContext::new();
    let system_prompt = format!(
        "{}\n\n## Workspace Context\n{}",
        include_str!("../prompts/system.md"),
        workspace.summary()
    );
    let max_iterations = cli.max_iterations.unwrap_or(25).clamp(1, 100);

    Ok(HeadlessCtx {
        config,
        provider_kind,
        model,
        tool_registry: Arc::new(registry),
        _mcp_servers: mcp_servers,
        workspace,
        system_prompt,
        max_iterations,
    })
}

fn make_provider(ctx: &HeadlessCtx) -> Box<dyn LLMProvider> {
    match ctx.provider_kind {
        ProviderKind::OpenAI => Box::new(OpenAIProvider::new(
            ctx.config.resolve_api_key("openai"),
            ctx.model.clone(),
            ctx.config.resolve_base_url(),
        )),
        ProviderKind::Anthropic => Box::new(AnthropicProvider::new(
            ctx.config.resolve_api_key("anthropic"),
            ctx.model.clone(),
            ctx.config.resolve_max_tokens(),
        )),
    }
}

fn base_messages(ctx: &HeadlessCtx, prompt: &str) -> Vec<crate::agent::conversation::Message> {
    let mut conv = Conversation::new();
    conv.set_system_prompt(ctx.system_prompt.clone());
    conv.add_message("user".into(), prompt.to_string());
    conv.full_context()
}

/// Print mode: run agent, output final assistant text only.
pub async fn run_print(prompt: &str, cli: &Cli) -> Result<()> {
    let ctx = build_ctx(cli)?;
    let provider = make_provider(&ctx);
    let messages = base_messages(&ctx, prompt);
    let tools = ctx.tool_registry.definitions();
    let (tx, mut rx) = mpsc::channel(128);
    let cancel = Arc::new(AtomicBool::new(false));
    let registry = ctx.tool_registry.clone();
    let max_iter = ctx.max_iterations;

    tokio::spawn(async move {
        start_agent_loop_with_limit(provider, messages, tools, registry, tx, cancel, max_iter).await;
    });

    let mut final_text = String::new();
    let mut usage: Option<(u64, u64)> = None;
    while let Some(ev) = rx.recv().await {
        match ev {
            StreamEvent::Text(t) => final_text.push_str(&t),
            StreamEvent::Usage { input_tokens, output_tokens } => {
                usage = Some((input_tokens, output_tokens));
            }
            StreamEvent::Done => break,
            _ => {}
        }
    }

    // Persist session
    {
        let sm = SessionManager::new();
        let mut conv = Conversation::new();
        conv.set_system_prompt(ctx.system_prompt.clone());
        conv.add_message("user".into(), prompt.to_string());
        if !final_text.is_empty() {
            conv.add_message("assistant".into(), final_text.clone());
        }
        sm.save(&conv.messages);
    }

    print!("{}", final_text);
    if !final_text.ends_with('\n') {
        println!();
    }
    if let Some((i, o)) = usage {
        eprintln!("[tokens in:{} out:{}]", i, o);
    }
    Ok(())
}

/// JSON mode: stream JSONL events.
pub async fn run_json(prompt: &str, cli: &Cli) -> Result<()> {
    use serde_json::json;
    let ctx = build_ctx(cli)?;
    let provider = make_provider(&ctx);
    let messages = base_messages(&ctx, prompt);
    let tools = ctx.tool_registry.definitions();
    let (tx, mut rx) = mpsc::channel(128);
    let cancel = Arc::new(AtomicBool::new(false));
    let registry = ctx.tool_registry.clone();
    let max_iter = ctx.max_iterations;

    // session accumulation
    let mut final_text = String::new();
    let mut all_tool_msgs: Vec<crate::agent::conversation::Message> = Vec::new();

    tokio::spawn(async move {
        start_agent_loop_with_limit(provider, messages, tools, registry, tx, cancel, max_iter).await;
    });

    while let Some(ev) = rx.recv().await {
        match ev {
            StreamEvent::Text(t) => {
                final_text.push_str(&t);
                println!("{}", json!({"type": "text", "delta": t}));
            }
            StreamEvent::ToolCall { id, name, args } => {
                println!("{}", json!({"type": "tool_call", "id": id, "name": name, "args": args}));
            }
            StreamEvent::ToolResult { id, name, result } => {
                all_tool_msgs.push(crate::agent::conversation::Message {
                    role: "tool".into(),
                    content: Some(result.to_string()),
                    tool_calls: None,
                    tool_call_id: Some(id.clone()),
                });
                println!("{}", json!({"type": "tool_result", "id": id, "name": name, "result": result}));
            }
            StreamEvent::Usage { input_tokens, output_tokens } => {
                println!("{}", json!({"type": "usage", "input_tokens": input_tokens, "output_tokens": output_tokens}));
            }
            StreamEvent::Done => break,
        }
    }
    println!("{}", json!({"type": "done", "final": final_text}));

    let sm = SessionManager::new();
    let mut conv = Conversation::new();
    conv.set_system_prompt(ctx.system_prompt.clone());
    conv.add_message("user".into(), prompt.to_string());
    for m in all_tool_msgs {
        conv.messages.push(m);
    }
    if !final_text.is_empty() {
        conv.add_message("assistant".into(), final_text);
    }
    sm.save(&conv.messages);
    Ok(())
}

/// RPC mode: JSONL request/response over stdio (minimal).
/// Requests (one JSON per line):
///   {"cmd":"ping"} -> {"type":"pong"}
///   {"cmd":"list_tools"} -> {"type":"tools","tools":[...]}
///   {"cmd":"prompt","text":"..."} -> streams text/tool events then {"type":"settled","final":"..."}
///   {"cmd":"shutdown"} -> {"type":"bye"} + exit
pub async fn run_rpc(cli: &Cli) -> Result<()> {
    use serde_json::json;
    use tokio::io::{AsyncBufReadExt, BufReader};

    let ctx = build_ctx(cli)?;
    // announce
    println!(
        "{}",
        json!({"type":"ready","model": ctx.model, "tools": ctx.tool_registry.tool_names()})
    );

    let stdin = tokio::io::stdin();
    let mut reader = BufReader::new(stdin);
    let mut line = String::new();

    loop {
        line.clear();
        let n = reader.read_line(&mut line).await?;
        if n == 0 {
            break;
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let req: serde_json::Value = match serde_json::from_str(trimmed) {
            Ok(v) => v,
            Err(e) => {
                println!("{}", json!({"type":"error","error": format!("bad json: {}", e)}));
                continue;
            }
        };
        let cmd = req.get("cmd").and_then(|v| v.as_str()).unwrap_or("");
        match cmd {
            "ping" => println!("{}", json!({"type":"pong"})),
            "list_tools" => {
                println!(
                    "{}",
                    json!({"type":"tools","tools": ctx.tool_registry.definitions()})
                );
            }
            "prompt" => {
                let text = req.get("text").and_then(|v| v.as_str()).unwrap_or("");
                if text.is_empty() {
                    println!("{}", json!({"type":"error","error":"missing text"}));
                    continue;
                }
                let provider = make_provider(&ctx);
                let messages = base_messages(&ctx, text);
                let tools = ctx.tool_registry.definitions();
                let (tx, mut rx) = mpsc::channel(128);
                let cancel = Arc::new(AtomicBool::new(false));
                let registry = ctx.tool_registry.clone();
                let max_iter = ctx.max_iterations;
                tokio::spawn(async move {
                    start_agent_loop_with_limit(provider, messages, tools, registry, tx, cancel, max_iter).await;
                });
                let mut final_text = String::new();
                while let Some(ev) = rx.recv().await {
                    match ev {
                        StreamEvent::Text(t) => {
                            final_text.push_str(&t);
                            println!("{}", json!({"type":"text","delta": t}));
                        }
                        StreamEvent::ToolCall { id, name, args } => {
                            println!("{}", json!({"type":"tool_call","id":id,"name":name,"args":args}));
                        }
                        StreamEvent::ToolResult { id, name, result } => {
                            println!("{}", json!({"type":"tool_result","id":id,"name":name,"result":result}));
                        }
                        StreamEvent::Usage { input_tokens, output_tokens } => {
                            println!("{}", json!({"type":"usage","input_tokens":input_tokens,"output_tokens":output_tokens}));
                        }
                        StreamEvent::Done => break,
                    }
                }
                println!("{}", json!({"type":"settled","final": final_text}));
            }
            "shutdown" => {
                println!("{}", json!({"type":"bye"}));
                break;
            }
            _ => println!("{}", json!({"type":"error","error": format!("unknown cmd: {}", cmd)})),
        }
    }
    Ok(())
}
