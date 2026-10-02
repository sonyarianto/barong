use clap::{Parser, ValueEnum};

#[derive(Debug, Clone, Copy, PartialEq, ValueEnum)]
pub enum Mode {
    Interactive,
    Print,
    Json,
    Rpc,
}

#[derive(Debug, Parser)]
#[command(name = "barong", version, about = "Barong — minimal terminal coding agent")]
pub struct Cli {
    /// Prompt to run headless (positional). `barong "explain main.rs"`
    pub prompt: Option<String>,

    /// One-shot prompt (print mode). `barong -p "query"` / stdin with `-p -`
    #[arg(short = 'p', long = "print")]
    pub print_prompt: Option<String>,

    /// Execution mode. Defaults: interactive unless -p/prompt given (-> print).
    #[arg(long = "mode", value_enum)]
    pub mode: Option<Mode>,

    /// Override provider (openai | anthropic)
    #[arg(long = "provider")]
    pub provider: Option<String>,

    /// Override model name
    #[arg(long = "model")]
    pub model: Option<String>,

    /// Working directory (workspace root)
    #[arg(long = "cwd")]
    pub cwd: Option<String>,

    /// Extra tools beyond core (comma-separated): grep,glob,delegate,all
    #[arg(long = "tools")]
    pub tools: Option<String>,

    /// List available tools and exit
    #[arg(long = "list-tools")]
    pub list_tools: bool,

    /// Max agent iterations (default 25)
    #[arg(long = "max-iterations")]
    pub max_iterations: Option<u32>,

    /// Auto-approve mutating tools (write/edit/bash). Same as BARONG_AUTO_APPROVE=1.
    #[arg(long = "yes", short = 'y')]
    pub yes: bool,
}

impl Cli {
    pub fn effective_prompt(&self) -> Option<String> {
        if let Some(p) = &self.print_prompt {
            if p == "-" {
                // read piped stdin fully
                use std::io::Read;
                let mut buf = String::new();
                if std::io::stdin().read_to_string(&mut buf).is_ok() {
                    let t = buf.trim().to_string();
                    if !t.is_empty() {
                        return Some(t);
                    }
                }
                return None;
            }
            return Some(p.clone());
        }
        self.prompt.clone()
    }

    pub fn effective_mode(&self) -> Mode {
        if let Some(m) = self.mode {
            return m;
        }
        if self.print_prompt.is_some() || self.prompt.is_some() {
            return Mode::Print;
        }
        // piped stdout without TTY -> print-friendly default
        if !atty_stdin() || !atty_stdout() {
            // only auto-switch if there's something to do; interactive otherwise
            // keep interactive as default to avoid breaking TUI when launched bare
        }
        Mode::Interactive
    }

    pub fn extra_tools(&self) -> Vec<String> {
        self.tools
            .as_deref()
            .unwrap_or("")
            .split([',', ' '])
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .collect()
    }
}

fn atty_stdin() -> bool {
    use std::io::IsTerminal;
    std::io::stdin().is_terminal()
}

fn atty_stdout() -> bool {
    use std::io::IsTerminal;
    std::io::stdout().is_terminal()
}
