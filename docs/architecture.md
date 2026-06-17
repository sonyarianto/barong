# KaliCode Architecture

## Overview

KaliCode is a terminal-based coding agent built in Rust using Ratatui and Crossterm.
It follows the standard agent loop pattern: **User Input → LLM Reasoning → Tool Execution → Result → Repeat**.

## System Architecture

`
┌─────────────────────────────────────────────────────────┐
│                       TUI Layer                         │
│  (ratatui + crossterm)                                  │
│  ┌─────────┐ ┌──────────┐ ┌──────────┐ ┌────────────┐  │
│  │ Chat    │ │ Input    │ │ Markdown │ │ Status     │  │
│  │ Panel   │ │ Bar      │ │ Renderer │ │ Bar        │  │
│  └─────────┘ └──────────┘ └──────────┘ └────────────┘  │
└──────────────────────┬──────────────────────────────────┘
                       │ events
┌──────────────────────▼──────────────────────────────────┐
│                    App State                            │
│  ┌────────────┐ ┌──────────────┐ ┌──────────────────┐  │
│  │ Agent Loop │ │ Conversation │ │ Workspace        │  │
│  │ Runner     │ │ Manager      │ │ Context          │  │
│  └──────┬─────┘ └──────────────┘ └──────────────────┘  │
└─────────┼───────────────────────────────────────────────┘
          │
┌─────────▼───────────────────────────────────────────────┐
│                  Agent Layer                             │
│  ┌──────────────────┐ ┌──────────────────────────────┐  │
│  │  Agent Loop      │ │  Conversation Manager        │  │
│  │  (orchestration)  │ │  (messages, context window)  │  │
│  └──────┬───────────┘ └──────────────────────────────┘  │
│         │                                                │
│  ┌──────▼───────────┐                                    │
│  │  LLM Provider    │  ◀── trait (OpenAI, Anthropic)     │
│  └──────────────────┘                                    │
└─────────┬───────────────────────────────────────────────┘
          │ tool calls
┌─────────▼───────────────────────────────────────────────┐
│                  Tool Layer                              │
│  ┌──────────┐ ┌───────────┐ ┌─────────┐ ┌───────────┐  │
│  │ ReadFile │ │ WriteFile │ │ EditFile│ │ Search    │  │
│  ├──────────┤ ├───────────┤ ├─────────┤ ├───────────┤  │
│  │ RunBash  │ │ Glob      │ │ ...     │ │           │  │
│  └──────────┘ └───────────┘ └─────────┘ └───────────┘  │
└─────────────────────────────────────────────────────────┘
`

## Core Components

### 1. Agent Loop
The central orchestrator implementing the observe-think-act cycle:

1. Receive user input
2. Build prompt with conversation history + system prompt
3. Send to LLM, stream response
4. Parse tool calls from stream
5. Execute requested tools
6. Feed tool results back to LLM
7. Repeat until LLM produces final text or max iterations reached

### 2. LLM Provider
Abstract trait for LLM providers (OpenAI-compatible + Anthropic).

### 3. Tool System
Every tool implements a shared trait with name, description, JSON schema, and async call method.

### 4. TUI
Built with Ratatui + Crossterm: Chat Panel, Input Bar, Markdown Renderer, Status Bar.

### 5. Conversation Manager
Maintains message history, handles context window management.

### 6. Workspace Context
Tracks project structure, git state, provides context to LLM.

## Data Flow

`
User types message
       │
       ▼
TUI sends event to App
       │
       ▼
Agent Loop prepares prompt (system + history + user msg)
       │
       ▼
LLM Provider streams response tokens
       │
       ▼
Is response a tool_call?
   YES → Execute tool, append result → Send back to LLM
   NO  → Display text, wait for user
`

## Error Handling

- LLM errors → retry with backoff
- Tool errors → return error message to LLM for self-correction
- TUI errors → graceful degradation
