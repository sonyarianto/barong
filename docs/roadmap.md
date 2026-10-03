# Barong Roadmap

## Phase 0: Skeleton
- [x] Define architecture & design docs
- [x] Initialize Cargo project with dependencies
- [x] Git init with .gitignore
- [x] Basic Ratatui TUI with hello world
- [x] Event loop (keyboard input, quit handling)

## Phase 1: TUI Chat (MVP-1)
- [x] Chat panel rendering
- [x] Text input bar with cursor
- [x] Message model (user/assistant/system)
- [x] Simple markdown rendering
- [x] Status bar component

## Phase 2: LLM Integration (MVP-2)
- [x] LLMProvider trait
- [x] OpenAI-compatible client (streaming SSE)
- [x] Anthropic client
- [x] Provider selection via config/env
- [x] Stream token rendering
- [x] Error handling (retries)

## Phase 3: Tool System (MVP-3)
- [x] Tool trait + ToolRegistry
- [x] read_file, write_file, edit_file
- [x] search (grep), run_command, glob
- [x] Tool call visualization in TUI

## Phase 4: Agent Loop (MVP-4)
- [x] Connect LLM → tools → results → LLM loop
- [x] System prompt for coding tasks
- [x] Iteration limit and safety guardrails
- [x] Interrupt/cancel mid-execution
- [x] Token usage tracking (provider-reported prompt tokens drive the `ctx` gauge)

## Phase 5: Workspace Intelligence
- [x] File tree panel (`/tree`, Ctrl+T)
- [x] Git status awareness (branch + changed count in tree panel + prompt)
- [x] Project context injection (AGENTS.md + file tree in system prompt)

## Phase 6: Polish
- [x] Syntax highlighting (syntect, per-theme)
- [x] Session persistence (resume + endpoint snapshot + crash-safe saves)
- [x] Config file (`barong.jsonc`, env overrides, `--provider/--model/--yes`)
- [x] MCP support
- [x] Sub-agents (opt-in `delegate` tool)

## Phase 7: Provider UX (done, unplanned)
- [x] Provider registry (openrouter/deepseek/nvidia/ollama/custom)
- [x] `auth.json` key store + `/login` picker + masked entry
- [x] `/model` picker (auth status, free-form ids) + live `/v1/models` discovery
- [x] Permission confirm modal (`y`/`a`/`n`) + headless deny-by-default
- [x] Auto-compact at 85% ctx, theme system, session branching

## Backlog (honest gaps)
- [ ] Per-model context windows (today: one `context_window` setting, default 128k)
- [ ] Agent-loop integration test with mock provider
- [ ] Copy-code button on code blocks (pi has `[Copy]`)
- [ ] Real unified diff for edit/write approvals (modal now shows per-tool
  `+`/`-` payloads; still not a computed diff)
- [ ] Retry with backoff on transient LLM errors
