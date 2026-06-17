use crate::app::App;
use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use std::sync::atomic::Ordering;

pub struct InputState {
    pub buffer: String,
    pub cursor_pos: usize,
    pub focused: bool,
    pub history: Vec<String>,
    pub history_index: Option<usize>,
    pub saved_buffer: String,
}

impl InputState {
    pub fn new() -> Self {
        Self {
            buffer: String::new(),
            cursor_pos: 0,
            focused: true,
            history: Vec::new(),
            history_index: None,
            saved_buffer: String::new(),
        }
    }

    pub fn push_history(&mut self, input: String) {
        self.history.push(input);
        self.history_index = None;
    }

    fn navigate_history(&mut self, direction: isize) {
        if self.history.is_empty() {
            return;
        }

        match self.history_index {
            None => {
                self.saved_buffer = self.buffer.clone();
                if direction < 0 {
                    self.history_index = Some(self.history.len() - 1);
                } else {
                    self.history_index = Some(0);
                }
            }
            Some(idx) => {
                let new_idx = if direction < 0 {
                    if idx == 0 {
                        self.history_index = None;
                        self.buffer = std::mem::take(&mut self.saved_buffer);
                        self.cursor_pos = self.buffer.len();
                        return;
                    }
                    idx - 1
                } else {
                    if idx >= self.history.len() - 1 {
                        self.history_index = None;
                        self.buffer = std::mem::take(&mut self.saved_buffer);
                        self.cursor_pos = self.buffer.len();
                        return;
                    }
                    idx + 1
                };
                self.history_index = Some(new_idx);
            }
        }

        if let Some(idx) = self.history_index {
            self.buffer = self.history[idx].clone();
            self.cursor_pos = self.buffer.len();
        }
    }
}

pub fn handle_events(app: &mut App) -> Result<()> {
    if let Event::Key(key) = event::read()? {
        if key.kind != KeyEventKind::Press {
            return Ok(());
        }

        match key.code {
            KeyCode::Char(c) => {
                app.input.buffer.insert(app.input.cursor_pos, c);
                app.input.cursor_pos += 1;
            }
            KeyCode::Backspace => {
                if app.input.cursor_pos > 0 {
                    app.input.cursor_pos -= 1;
                    app.input.buffer.remove(app.input.cursor_pos);
                }
            }
            KeyCode::Delete => {
                if app.input.cursor_pos < app.input.buffer.len() {
                    app.input.buffer.remove(app.input.cursor_pos);
                }
            }
            KeyCode::Left => {
                app.input.cursor_pos = app.input.cursor_pos.saturating_sub(1);
            }
            KeyCode::Right => {
                if app.input.cursor_pos < app.input.buffer.len() {
                    app.input.cursor_pos += 1;
                }
            }
            KeyCode::Home => {
                app.input.cursor_pos = 0;
            }
            KeyCode::End => {
                app.input.cursor_pos = app.input.buffer.len();
            }
            KeyCode::Up => {
                app.input.navigate_history(-1);
            }
            KeyCode::Down => {
                app.input.navigate_history(1);
            }
            KeyCode::Enter => {
                let input = std::mem::take(&mut app.input.buffer);
                app.input.cursor_pos = 0;
                app.input.history_index = None;
                if !input.is_empty() && app.event_rx.is_none() {
                    if input.starts_with('/') {
                        match input.trim() {
                            "/quit" | "/exit" => { app.should_quit = true; return Ok(()); }
                            "/clear" | "/new" => {
                                app.conversation = crate::agent::conversation::Conversation::new();
                                app.session_id = None;
                                app.streaming_text.clear();
                                app.chat_scroll = 0;
                                app.should_auto_scroll = true;
                                app.is_home = true;
                                return Ok(());
                            }
                            "/help" => {
                                app.conversation.add_message("assistant".into(),
                                    "**Commands:**\n- `/quit` or `/exit` — quit\n- `/clear` or `/new` — new session\n- `/help` — this message".into());
                                app.save_session();
                                return Ok(());
                            }
                            _ => {
                                app.conversation.add_message("assistant".into(),
                                    format!("Unknown command: `{}`. Try `/help`.", input));
                                app.save_session();
                                return Ok(());
                            }
                        }
                    }
                    app.input.push_history(input.clone());
                    app.conversation.add_message("user".into(), input);
                    app.is_home = false;
                    app.save_session();
                    app.should_auto_scroll = true;
                    app.chat_scroll = 0;
                    app.start_agent();
                }
            }
            KeyCode::Tab => {
                app.input.buffer.push_str("  ");
                app.input.cursor_pos += 2;
            }
            KeyCode::Esc => {
                if app.event_rx.is_some() {
                    app.cancelled.store(true, Ordering::Relaxed);
                    app.event_rx = None;
                    app.save_session();
                    app.streaming_text.clear();
                    app.status.tool_status = "cancelled".into();
                }
            }
            KeyCode::PageUp => {
                app.chat_scroll = app.chat_scroll.saturating_add(5);
                app.should_auto_scroll = false;
            }
            KeyCode::PageDown => {
                app.chat_scroll = app.chat_scroll.saturating_sub(5);
            }
            _ => {}
        }
    }
    Ok(())
}
