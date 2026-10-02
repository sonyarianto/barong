pub mod tui;
pub mod agent;
pub mod auth;
pub mod tools;
pub mod workspace;
pub mod config;
pub mod session;
pub mod mcp;
pub mod cli;
pub mod headless;

use anyhow::Result;
use clap::Parser;
use ratatui::DefaultTerminal;
use std::time::Duration;

mod app;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_target(false)
        .init();

    let cli = cli::Cli::parse();

    if let Some(cwd) = &cli.cwd {
        std::env::set_current_dir(cwd)?;
    }

    if cli.list_tools {
        let ctx = headless::build_ctx(&cli)?;
        for name in ctx.tool_registry.tool_names() {
            println!("{}", name);
        }
        return Ok(());
    }

    match cli.effective_mode() {
        cli::Mode::Print => {
            let Some(prompt) = cli.effective_prompt() else {
                anyhow::bail!("no prompt given. Usage: barong -p \"query\" or barong \"query\"");
            };
            headless::run_print(&prompt, &cli).await
        }
        cli::Mode::Json => {
            let Some(prompt) = cli.effective_prompt() else {
                anyhow::bail!("no prompt given. Usage: barong --mode json -p \"query\"");
            };
            headless::run_json(&prompt, &cli).await
        }
        cli::Mode::Rpc => headless::run_rpc(&cli).await,
        cli::Mode::Interactive => {
            // If a positional prompt was given with explicit interactive mode,
            // boot TUI with it pre-sent is out of scope for MVP; just run TUI.
            run_tui(cli.yes, cli.provider.as_deref(), cli.model.as_deref()).await
        }
    }
}

async fn run_tui(auto_approve: bool, provider: Option<&str>, model: Option<&str>) -> Result<()> {
    // ratatui is sync; run it on blocking thread scope via existing loop.
    // We are already inside tokio runtime, App::start_agent spawns tasks onto it.
    let terminal = ratatui::init();
    let result = run_blocking(terminal, auto_approve, provider, model);
    ratatui::restore();
    result
}

fn run_blocking(
    mut terminal: DefaultTerminal,
    auto_approve: bool,
    provider: Option<&str>,
    model: Option<&str>,
) -> Result<()> {
    let mut app = app::App::new();
    if auto_approve {
        app.permission_gate.set_auto_approve(true);
    }
    app.apply_cli_overrides(provider, model);
    // Refresh stale model catalogs in the background (login also triggers).
    app.refresh_stale_models();
    while !app.should_quit {
        terminal.draw(|frame| app.render(frame))?;
        app.handle_stream()?;
        if crossterm::event::poll(Duration::from_millis(50))? {
            app.handle_events()?;
        }
    }
    Ok(())
}
