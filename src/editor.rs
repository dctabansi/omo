use anyhow::Result;
use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyEvent, KeyModifiers},
    queue,
};
use std::io::{Write, stdout};

use crate::buffer::Buffer;
use crate::terminal::Terminal;

pub struct Editor {
    should_quit: bool,
    buffer: Buffer,
    cx: u16,
    cy: u16,
}

impl Editor {
    pub fn new(buffer: Buffer) -> Self {
        Self {
            should_quit: false,
            buffer,
            cx: 0,
            cy: 0,
        }
    }

    /// The main event loop.
    pub fn run(&mut self) -> Result<()> {
        loop {
            self.refresh_screen()?;

            if self.should_quit {
                break;
            }

            self.process_keypress()?;
        }
        Ok(())
    }

    /// Flushes all visual updates to the terminal in a single batch.
    fn refresh_screen(&self) -> Result<()> {
        let mut stdout = stdout();

        queue!(stdout, cursor::Hide, cursor::MoveTo(0, 0))?;
        self.draw_rows(&mut stdout)?;
        queue!(stdout, cursor::MoveTo(self.cx, self.cy), cursor::Show)?;

        stdout.flush()?;
        Ok(())
    }

    /// Handles drawing the editor background and text buffer.
    fn draw_rows(&self, stdout: &mut std::io::Stdout) -> Result<()> {
        let (cols, rows) = Terminal::size()?;

        for row in 0..rows {
            if let Some(buffer_row) = self.buffer.get_row(row as usize) {
                let render_text: String = buffer_row.text.chars().take(cols as usize).collect();
                queue!(stdout, crossterm::style::Print(render_text))?;
            } else {
                queue!(stdout, crossterm::style::Print("~"))?;
            }

            if row < rows - 1 {
                queue!(stdout, crossterm::style::Print("\r\n"))?;
            }
        }
        Ok(())
    }

    /// Blocks and waits for user input.
    fn process_keypress(&mut self) -> Result<()> {
        if let Event::Key(KeyEvent {
            code, modifiers, ..
        }) = event::read()?
            && let (KeyCode::Char('q'), KeyModifiers::CONTROL) = (code, modifiers)
        {
            self.should_quit = true;
        }
        Ok(())
    }
}
