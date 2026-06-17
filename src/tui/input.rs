use crate::app::App;
use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEventKind};

pub struct InputState {
    pub buffer: String,
    pub cursor_pos: usize,
    pub focused: bool,
}

impl InputState {
    pub fn new() -> Self {
        Self {
            buffer: String::new(),
            cursor_pos: 0,
            focused: true,
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
            KeyCode::Enter => {
                let input = std::mem::take(&mut app.input.buffer);
                app.input.cursor_pos = 0;
                if !input.is_empty() {
                    app.conversation.messages.push(
                        crate::agent::conversation::Message {
                            role: "user".into(),
                            content: input,
                        },
                    );
                    app.status.tool_status = "processing...".into();
                }
            }
            KeyCode::Esc => {
                app.should_quit = true;
            }
            _ => {}
        }
    }
    Ok(())
}
