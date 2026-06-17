pub mod tui;
pub mod agent;
pub mod tools;
pub mod workspace;

use anyhow::Result;
use ratatui::DefaultTerminal;

mod app;

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_target(false)
        .init();

    let terminal = ratatui::init();
    let result = run(terminal);
    ratatui::restore();
    result
}

fn run(mut terminal: DefaultTerminal) -> Result<()> {
    let mut app = app::App::new();
    while !app.should_quit {
        terminal.draw(|frame| app.render(frame))?;
        app.handle_events()?;
    }
    Ok(())
}
