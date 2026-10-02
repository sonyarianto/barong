# Barong

Minimal terminal coding agent in Rust (Ratatui TUI + headless modes).
Edition 2024, Ratatui 0.30.

## Run

```bash
cargo run                      # TUI (auto-resumes last session)
cargo run -- -p "explain main.rs"
cargo run -- --provider openrouter --model "deepseek/deepseek-chat-v3-0324" -p "..."
cargo run -- --yes -p "fix the failing test"   # auto-approve tools
```

## API keys

Keys live in `~/.barong/auth.json` (0600) — never in project files.

- TUI: `/login` → pick provider → paste key (switches to it immediately)
- Env: `OPENAI_API_KEY` / `ANTHROPIC_API_KEY` / `OPENROUTER_API_KEY` /
  `DEEPSEEK_API_KEY` / `NVIDIA_API_KEY` (or generic `BARONG_API_KEY`)
- `/logout <provider>` removes a saved key

Supported out of the box: `openai`, `anthropic`, `openrouter`,
`deepseek`, `nvidia`, `9router` (local gateway at
`http://localhost:20128/v1`, key from its dashboard via
`NINEROUTER_API_KEY` or `/login 9router`), `ollama` (keyless local). Any other
OpenAI-compatible endpoint goes in `barong.jsonc`:

```jsonc
{
  "provider": "openrouter",
  "providers": {
    "kantor": {
      "base_url": "https://proxy.local/v1",
      "models": ["team-model-v1"]
    }
  }
}
```

`/model` lists live models (fetched from `{base_url}/models`, cached
24h in `~/.barong/models-cache.json`) with `●` ready / `○` needs-key.
Unknown future ids still work free-form: `/model nvidia/some-new-id`.

## Slash commands

`/model` pick endpoint · `/login` `/logout` keys · `/allow` `/approve`
permissions · `/compact` (+`auto on|off`) context · `/theme` dark/light/barong ·
`/tree` (Ctrl+T) file panel · `/branch` `/log` `/resume` sessions ·
`/session` status · `/tools` · `/copy` `/export` · `/help` `/quit`

Keys: `Enter` send · `Tab` complete · `↑↓` pick/history · `PgUp/PgDn` or
mouse wheel scroll · `Ctrl+O` expand tools · `Esc` cancel · approval:
`y` once, `a` always, `n` deny.

## Layout

- `src/agent/` loop, LLM providers (OpenAI-compatible + Anthropic),
  permissions gate, live model discovery, conversation + auto-compact
- `src/tools/` read/write/edit/bash (+grep/glob/delegate opt-in)
- `src/tui/` markdown renderer (themed code blocks), pickers, themes
- `src/auth.rs` key store · `src/session.rs` sessions/branches ·
  `src/config.rs` `barong.jsonc` + env · `src/headless.rs` print/json/rpc
- `docs/` architecture, design, roadmap
