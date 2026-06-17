# KaliCode Roadmap

## Phase 0: Skeleton
- [x] Define architecture & design docs
- [x] Initialize Cargo project with dependencies
- [x] Git init with .gitignore
- [ ] Basic Ratatui TUI with hello world
- [ ] Event loop (keyboard input, quit handling)

## Phase 1: TUI Chat (MVP-1)
- [ ] Chat panel rendering
- [ ] Text input bar with cursor
- [ ] Message model (user/assistant/system)
- [ ] Simple markdown rendering
- [ ] Status bar component

## Phase 2: LLM Integration (MVP-2)
- [ ] LLMProvider trait
- [ ] OpenAI-compatible client (streaming SSE)
- [ ] Anthropic client
- [ ] Provider selection via config/env
- [ ] Stream token rendering
- [ ] Error handling (retries)

## Phase 3: Tool System (MVP-3)
- [ ] Tool trait + ToolRegistry
- [ ] read_file, write_file, edit_file
- [ ] search (grep), run_command
- [ ] Tool call visualization in TUI

## Phase 4: Agent Loop (MVP-4)
- [ ] Connect LLM → tools → results → LLM loop
- [ ] System prompt for coding tasks
- [ ] Iteration limit and safety guardrails
- [ ] Interrupt/cancel mid-execution

## Phase 5: Workspace Intelligence
- [ ] File tree panel
- [ ] Git status awareness
- [ ] Project context injection

## Phase 6: Polish
- [ ] Syntax highlighting
- [ ] Session persistence
- [ ] Config file
- [ ] MCP support
- [ ] Sub-agents
