# Barong Design Decisions

## Why Rust?

- Performance — near-zero startup time, low memory footprint
- Safety — ownership model prevents data races
- Ecosystem — Ratatui, Tokio, reqwest
- Single binary distribution

## Why Ratatui + Crossterm?

- De facto standard for Rust TUIs (21k+ GitHub stars)
- Immediate mode rendering with buffer diffing
- Cross-platform (Windows, macOS, Linux)
- Rich widget library

## Single Crate vs Workspace

**Chosen: Single crate** for faster iteration. Can split later if needed.

## Streaming-First Design

LLM responses are streamed token-by-token and rendered incrementally for immediate visual feedback.

## Provider-Agnostic LLM Abstraction

Both OpenAI-compatible and Anthropic providers emit the same event types:
- Text(String)
- ToolCall { id, name, args }
- ToolResult { id, result }
- Done

## Tool Design Philosophy

Tools are: self-describing (JSON schema), atomic (one thing each), auditable (logged to history).

## State Management

Single App struct owned by main.rs, no global mutable state.
