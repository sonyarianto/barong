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
- [ ] Token usage tracking

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
