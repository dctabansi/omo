use anyhow::Result;
use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyEvent, KeyModifiers},
    queue,
    style::{
        Attribute, Color, Print, ResetColor, SetAttribute, SetBackgroundColor, SetForegroundColor,
    },
    terminal::{Clear, ClearType},
};
use std::io::{Write, stdout};

use crate::buffer::Buffer;
use crate::config::{Action, Config};
use crate::terminal::Terminal;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PromptState {
    None,
    SaveAs {
        input: String,
    },
    Search {
        query: String,
        saved_cx: usize,
        saved_cy: usize,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditorCommand {
    Action(Action),
    Move(KeyCode),
    InsertChar(char),
    InsertNewline,
    Backspace,
    Delete,
    InsertTab,
    Noop,
}

pub struct Editor {
    should_quit: bool,
    buffer: Buffer,
    pub config: Config,
    cx: usize,
    cy: usize,
    rx: usize,
    desired_cx: usize,
    row_offset: usize,
    col_offset: usize,
    quit_confirm_pending: bool,
    pub status_message: Option<String>,
    pub prompt: PromptState,
}

impl Editor {
    #[allow(dead_code)]
    pub fn new(buffer: Buffer) -> Self {
        Self::with_config(buffer, Config::default())
    }

    pub fn with_config(mut buffer: Buffer, config: Config) -> Self {
        buffer.set_tab_width(config.tab_width);
        Self {
            should_quit: false,
            buffer,
            config,
            cx: 0,
            cy: 0,
            rx: 0,
            desired_cx: 0,
            row_offset: 0,
            col_offset: 0,
            quit_confirm_pending: false,
            status_message: None,
            prompt: PromptState::None,
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

    /// Returns the character length of the row currently under the cursor.
    fn current_row_len(&self) -> usize {
        self.buffer.get_row(self.cy).map_or(0, |row| row.len)
    }

    /// Computes the visual render column `rx` corresponding to `(self.cx, self.cy)`
    /// after expanding `\t` characters to configured tab stops.
    pub fn visual_cx(&self) -> usize {
        self.buffer
            .get_row(self.cy)
            .map_or(self.cx, |row| row.cx_to_rx(self.cx))
    }

    /// Computes the dynamic width of the line-number gutter (`digits + 2`).
    pub fn gutter_width(&self) -> usize {
        self.buffer.len().max(1).to_string().len() + 2
    }

    /// Formats a 1-based line number right-aligned within the dynamic gutter width.
    pub fn format_gutter_line(&self, file_row: usize) -> String {
        let digits = self.buffer.len().max(1).to_string().len();
        format!(" {:>width$} ", file_row + 1, width = digits)
    }

    /// Computes the text viewport dimensions `(text_cols, text_rows)` after reserving the gutter
    /// and the bottom status bar row.
    pub fn text_viewport(&self, cols: u16, rows: u16) -> (usize, usize) {
        let text_cols = (cols as usize).saturating_sub(self.gutter_width());
        let text_rows = (rows as usize).saturating_sub(1);
        (text_cols, text_rows)
    }

    /// Computes the hardware cursor screen coordinates `(screen_cx, screen_cy)` using visual tab-expanded `rx`.
    pub fn cursor_screen_position(&self) -> (u16, u16) {
        let rx = self.visual_cx();
        let screen_cx = (self.gutter_width() + rx.saturating_sub(self.col_offset)) as u16;
        let screen_cy = self.cy.saturating_sub(self.row_offset) as u16;
        (screen_cx, screen_cy)
    }

    /// Formats the status bar string padded to `cols` terminal columns.
    pub fn format_status_bar(&self, cols: usize) -> String {
        if cols == 0 {
            return String::new();
        }

        let left = match &self.prompt {
            PromptState::SaveAs { input } => format!(" Save as: {input}"),
            PromptState::Search { query, .. } => {
                format!(" Search (Esc to cancel, arrows/Enter to jump): {query}")
            }
            PromptState::None => {
                let name = self.buffer.filename.as_deref().unwrap_or("[No Name]");
                let dirty = if self.buffer.dirty { " [+]" } else { "" };
                if let Some(msg) = &self.status_message {
                    format!(" {name}{dirty} | {msg}")
                } else {
                    format!(" {name}{dirty}")
                }
            }
        };

        let right = format!(
            "Ln {}, Col {} | {} lines ",
            self.cy + 1,
            self.cx + 1,
            self.buffer.len()
        );

        let left_len = left.chars().count();
        let right_len = right.chars().count();

        if left_len + right_len <= cols {
            let padding = " ".repeat(cols - left_len - right_len);
            format!("{left}{padding}{right}")
        } else if right_len < cols {
            let keep_left = cols - right_len;
            let truncated_left: String = left.chars().take(keep_left).collect();
            format!("{truncated_left}{right}")
        } else {
            right.chars().take(cols).collect()
        }
    }

    /// Adjusts vertical and horizontal viewport offsets so the cursor stays visible.
    pub fn scroll(&mut self, screen_cols: usize, screen_rows: usize) {
        self.rx = self.visual_cx();

        if self.cy < self.row_offset {
            self.row_offset = self.cy;
        }
        if screen_rows > 0 && self.cy >= self.row_offset + screen_rows {
            self.row_offset = self.cy - screen_rows + 1;
        }

        if self.rx < self.col_offset {
            self.col_offset = self.rx;
        }
        if screen_cols > 0 && self.rx >= self.col_offset + screen_cols {
            self.col_offset = self.rx - screen_cols + 1;
        }
    }

    /// Moves the cursor according to a navigation key and terminal viewport height.
    pub fn move_cursor(&mut self, code: KeyCode, viewport_rows: usize) {
        match code {
            KeyCode::Up if self.cy > 0 => {
                self.cy -= 1;
                self.cx = self.desired_cx.min(self.current_row_len());
            }
            KeyCode::Down if self.cy + 1 < self.buffer.len() => {
                self.cy += 1;
                self.cx = self.desired_cx.min(self.current_row_len());
            }
            KeyCode::Left => {
                if self.cx > 0 {
                    self.cx -= 1;
                    self.desired_cx = self.cx;
                } else if self.cy > 0 {
                    self.cy -= 1;
                    self.cx = self.current_row_len();
                    self.desired_cx = self.cx;
                }
            }
            KeyCode::Right => {
                let row_len = self.current_row_len();
                if self.cx < row_len {
                    self.cx += 1;
                    self.desired_cx = self.cx;
                } else if self.cy + 1 < self.buffer.len() {
                    self.cy += 1;
                    self.cx = 0;
                    self.desired_cx = 0;
                }
            }
            KeyCode::Home => {
                self.cx = 0;
                self.desired_cx = 0;
            }
            KeyCode::End => {
                self.cx = self.current_row_len();
                self.desired_cx = self.cx;
            }
            KeyCode::PageUp => {
                let step = viewport_rows.max(1);
                self.cy = self.cy.saturating_sub(step);
                self.cx = self.desired_cx.min(self.current_row_len());
            }
            KeyCode::PageDown if !self.buffer.is_empty() => {
                let step = viewport_rows.max(1);
                self.cy = (self.cy + step).min(self.buffer.len() - 1);
                self.cx = self.desired_cx.min(self.current_row_len());
            }
            _ => {}
        }
    }

    /// Inserts a single character at the cursor position and advances `cx`.
    pub fn insert_char(&mut self, c: char) {
        self.buffer.insert_char(self.cx, self.cy, c);
        self.cx += 1;
        self.desired_cx = self.cx;
    }

    /// Splits the current line at the cursor and moves to the start of the new line.
    pub fn insert_newline(&mut self) {
        self.buffer.insert_newline(self.cx, self.cy);
        self.cy += 1;
        self.cx = 0;
        self.desired_cx = 0;
    }

    /// Deletes the character to the left of the cursor or joins with the previous line.
    pub fn backspace(&mut self) {
        if let Some((new_cx, new_cy)) = self.buffer.delete_char(self.cx, self.cy) {
            self.cx = new_cx;
            self.cy = new_cy;
            self.desired_cx = self.cx;
        }
    }

    /// Deletes the character under the cursor or joins the next line.
    pub fn delete_forward(&mut self) {
        self.buffer.delete_forward(self.cx, self.cy);
        self.desired_cx = self.cx;
    }

    /// Saves the buffer or opens the inline Save-As prompt if the buffer is unnamed.
    pub fn trigger_save(&mut self) {
        if self.buffer.filename.is_some() {
            match self.buffer.save() {
                Ok(bytes) => {
                    let name = self.buffer.filename.as_deref().unwrap_or("[No Name]");
                    self.status_message = Some(format!("Wrote {bytes} bytes to {name}"));
                }
                Err(err) => {
                    self.status_message = Some(format!("Save error: {err}"));
                }
            }
        } else {
            self.prompt = PromptState::SaveAs {
                input: String::new(),
            };
            self.status_message = None;
        }
    }

    /// Handles a quit request, requiring a second consecutive Quit action when dirty.
    pub fn trigger_quit(&mut self) {
        if self.buffer.dirty && !self.quit_confirm_pending {
            self.quit_confirm_pending = true;
            self.status_message =
                Some("WARNING! Unsaved changes. Press Ctrl+Q again to quit.".to_string());
        } else {
            self.should_quit = true;
        }
    }

    /// Reverts the most recent buffer edit and restores its cursor coordinates.
    pub fn trigger_undo(&mut self) {
        if let Some((cx, cy)) = self.buffer.undo(self.cx, self.cy) {
            self.cx = cx;
            self.cy = cy;
            self.desired_cx = cx;
            self.status_message = Some("Undo.".to_string());
        } else {
            self.status_message = Some("Already at oldest change.".to_string());
        }
    }

    /// Reapplies the most recently undone buffer edit and restores its cursor coordinates.
    pub fn trigger_redo(&mut self) {
        if let Some((cx, cy)) = self.buffer.redo(self.cx, self.cy) {
            self.cx = cx;
            self.cy = cy;
            self.desired_cx = cx;
            self.status_message = Some("Redo.".to_string());
        } else {
            self.status_message = Some("Already at newest change.".to_string());
        }
    }

    /// Opens the interactive status-bar search prompt.
    pub fn trigger_find(&mut self) {
        self.prompt = PromptState::Search {
            query: String::new(),
            saved_cx: self.cx,
            saved_cy: self.cy,
        };
        self.status_message = None;
    }

    /// Translates a `KeyEvent` into a unified `EditorCommand`, checking `Config` action bindings first.
    pub fn resolve_command(&self, key: KeyEvent) -> EditorCommand {
        if let Some(action) = self.config.action_for_key(key) {
            return EditorCommand::Action(action);
        }

        match (key.code, key.modifiers) {
            (KeyCode::Enter, _) => EditorCommand::InsertNewline,
            (KeyCode::Backspace, _) => EditorCommand::Backspace,
            (KeyCode::Delete, _) => EditorCommand::Delete,
            (KeyCode::Tab, _) => EditorCommand::InsertTab,
            (
                KeyCode::Up
                | KeyCode::Down
                | KeyCode::Left
                | KeyCode::Right
                | KeyCode::Home
                | KeyCode::End
                | KeyCode::PageUp
                | KeyCode::PageDown,
                _,
            ) => EditorCommand::Move(key.code),
            (KeyCode::Char(c), modifiers)
                if !modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                EditorCommand::InsertChar(c)
            }
            _ => EditorCommand::Noop,
        }
    }

    /// Processes a single key event against the current editor state.
    pub fn handle_key_event(&mut self, key: KeyEvent, viewport_rows: usize) {
        match self.prompt.clone() {
            PromptState::SaveAs { mut input } => {
                match key.code {
                    KeyCode::Esc => {
                        self.prompt = PromptState::None;
                        self.status_message = Some("Save aborted.".to_string());
                    }
                    KeyCode::Enter => {
                        let trimmed = input.trim().to_string();
                        self.prompt = PromptState::None;
                        if trimmed.is_empty() {
                            self.status_message =
                                Some("Save aborted: empty file name.".to_string());
                        } else {
                            match self.buffer.save_as(&trimmed) {
                                Ok(bytes) => {
                                    self.status_message =
                                        Some(format!("Wrote {bytes} bytes to {trimmed}"));
                                }
                                Err(err) => {
                                    self.status_message = Some(format!("Save error: {err}"));
                                }
                            }
                        }
                    }
                    KeyCode::Backspace => {
                        input.pop();
                        self.prompt = PromptState::SaveAs { input };
                    }
                    KeyCode::Char(c)
                        if !key
                            .modifiers
                            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
                    {
                        input.push(c);
                        self.prompt = PromptState::SaveAs { input };
                    }
                    _ => {}
                }
                return;
            }
            PromptState::Search {
                mut query,
                saved_cx,
                saved_cy,
            } => {
                match key.code {
                    KeyCode::Esc => {
                        self.cx = saved_cx;
                        self.cy = saved_cy;
                        self.desired_cx = saved_cx;
                        self.prompt = PromptState::None;
                        self.status_message = Some("Search canceled.".to_string());
                    }
                    KeyCode::Enter | KeyCode::Down | KeyCode::Right => {
                        if let Some((match_cx, match_cy)) =
                            self.buffer.find(&query, self.cx + 1, self.cy, true)
                        {
                            self.cx = match_cx;
                            self.cy = match_cy;
                            self.desired_cx = match_cx;
                        }
                    }
                    KeyCode::Up | KeyCode::Left => {
                        if let Some((match_cx, match_cy)) =
                            self.buffer.find(&query, self.cx, self.cy, false)
                        {
                            self.cx = match_cx;
                            self.cy = match_cy;
                            self.desired_cx = match_cx;
                        }
                    }
                    KeyCode::Backspace => {
                        query.pop();
                        if let Some((match_cx, match_cy)) =
                            self.buffer.find(&query, saved_cx, saved_cy, true)
                        {
                            self.cx = match_cx;
                            self.cy = match_cy;
                            self.desired_cx = match_cx;
                        } else {
                            self.cx = saved_cx;
                            self.cy = saved_cy;
                            self.desired_cx = saved_cx;
                        }
                        self.prompt = PromptState::Search {
                            query,
                            saved_cx,
                            saved_cy,
                        };
                    }
                    KeyCode::Char(c)
                        if !key
                            .modifiers
                            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
                    {
                        query.push(c);
                        if let Some((match_cx, match_cy)) =
                            self.buffer.find(&query, saved_cx, saved_cy, true)
                        {
                            self.cx = match_cx;
                            self.cy = match_cy;
                            self.desired_cx = match_cx;
                        }
                        self.prompt = PromptState::Search {
                            query,
                            saved_cx,
                            saved_cy,
                        };
                    }
                    _ => {}
                }
                return;
            }
            PromptState::None => {}
        }

        let command = self.resolve_command(key);

        if command == EditorCommand::Action(Action::Quit) {
            self.trigger_quit();
            return;
        }

        self.quit_confirm_pending = false;

        match command {
            EditorCommand::Action(Action::Quit) => self.trigger_quit(),
            EditorCommand::Action(Action::Save) => self.trigger_save(),
            EditorCommand::Action(Action::Undo) => self.trigger_undo(),
            EditorCommand::Action(Action::Redo) => self.trigger_redo(),
            EditorCommand::Action(Action::Find) => self.trigger_find(),
            EditorCommand::Move(code) => self.move_cursor(code, viewport_rows),
            EditorCommand::InsertChar(c) => self.insert_char(c),
            EditorCommand::InsertNewline => self.insert_newline(),
            EditorCommand::Backspace => self.backspace(),
            EditorCommand::Delete => self.delete_forward(),
            EditorCommand::InsertTab => self.insert_char('\t'),
            EditorCommand::Noop => {}
        }
    }

    /// Queues configured background and foreground theme colors onto `stdout`.
    fn apply_editor_theme(&self, stdout: &mut std::io::Stdout) -> Result<()> {
        if let Some((r, g, b)) = self.config.theme.background {
            queue!(stdout, SetBackgroundColor(Color::Rgb { r, g, b }))?;
        }
        if let Some((r, g, b)) = self.config.theme.foreground {
            queue!(stdout, SetForegroundColor(Color::Rgb { r, g, b }))?;
        }
        Ok(())
    }

    /// Flushes all visual updates to the terminal in a single batch.
    fn refresh_screen(&mut self) -> Result<()> {
        let (cols, rows) = Terminal::size()?;
        let (text_cols, text_rows) = self.text_viewport(cols, rows);
        self.scroll(text_cols, text_rows);

        let mut stdout = stdout();

        queue!(stdout, cursor::Hide, cursor::MoveTo(0, 0))?;
        self.draw_rows(&mut stdout, cols, rows)?;
        self.draw_status_bar(&mut stdout, cols)?;

        let (screen_cx, screen_cy) = self.cursor_screen_position();
        queue!(stdout, cursor::MoveTo(screen_cx, screen_cy), cursor::Show)?;

        stdout.flush()?;
        Ok(())
    }

    /// Handles drawing the line-number gutter and text buffer rows.
    fn draw_rows(&self, stdout: &mut std::io::Stdout, cols: u16, rows: u16) -> Result<()> {
        let (text_cols, text_rows) = self.text_viewport(cols, rows);
        let gutter_w = self.gutter_width();

        for row in 0..text_rows {
            let file_row = self.row_offset + row;
            self.apply_editor_theme(stdout)?;

            if let Some(buffer_row) = self.buffer.get_row(file_row) {
                let gutter = self.format_gutter_line(file_row);
                let render_text = buffer_row.slice_chars(self.col_offset, text_cols);
                queue!(
                    stdout,
                    SetAttribute(Attribute::Dim),
                    Print(gutter),
                    SetAttribute(Attribute::Reset)
                )?;
                self.apply_editor_theme(stdout)?;
                queue!(stdout, Print(render_text))?;
            } else if self.buffer.is_empty() && file_row == 0 {
                let gutter = self.format_gutter_line(0);
                queue!(
                    stdout,
                    SetAttribute(Attribute::Dim),
                    Print(gutter),
                    SetAttribute(Attribute::Reset)
                )?;
                self.apply_editor_theme(stdout)?;
            } else {
                let tilde_gutter = format!(" {:>width$} ", "~", width = gutter_w.saturating_sub(2));
                queue!(
                    stdout,
                    SetAttribute(Attribute::Dim),
                    Print(tilde_gutter),
                    SetAttribute(Attribute::Reset)
                )?;
                self.apply_editor_theme(stdout)?;
            }

            queue!(stdout, Clear(ClearType::UntilNewLine), Print("\r\n"))?;
        }
        queue!(stdout, ResetColor)?;
        Ok(())
    }

    /// Draws the inverted or custom-themed status bar on the bottom terminal row.
    fn draw_status_bar(&self, stdout: &mut std::io::Stdout, cols: u16) -> Result<()> {
        let status = self.format_status_bar(cols as usize);
        let has_custom_status_theme =
            self.config.theme.status_bg.is_some() || self.config.theme.status_fg.is_some();

        if has_custom_status_theme {
            if let Some((r, g, b)) = self.config.theme.status_bg {
                queue!(stdout, SetBackgroundColor(Color::Rgb { r, g, b }))?;
            }
            if let Some((r, g, b)) = self.config.theme.status_fg {
                queue!(stdout, SetForegroundColor(Color::Rgb { r, g, b }))?;
            }
        } else {
            queue!(stdout, SetAttribute(Attribute::Reverse))?;
        }

        queue!(
            stdout,
            Print(status),
            Clear(ClearType::UntilNewLine),
            ResetColor,
            SetAttribute(Attribute::Reset)
        )?;
        Ok(())
    }

    /// Blocks and waits for user input.
    fn process_keypress(&mut self) -> Result<()> {
        if let Event::Key(key) = event::read()? {
            let (_, rows) = Terminal::size().unwrap_or((80, 24));
            let text_rows = (rows as usize).saturating_sub(1);
            self.handle_key_event(key, text_rows);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_buffer(lines: &[&str]) -> Buffer {
        Buffer::from_lines(lines)
    }

    #[test]
    fn vertical_navigation_clamps_and_restores_desired_cx() {
        let buf = sample_buffer(&[
            "01234567890123456789", // len 20
            "short",                // len 5
            "0123456789012345",     // len 16
        ]);
        let mut editor = Editor::new(buf);

        editor.move_cursor(KeyCode::End, 24);
        assert_eq!((editor.cx, editor.cy, editor.desired_cx), (20, 0, 20));

        editor.move_cursor(KeyCode::Down, 24);
        assert_eq!((editor.cx, editor.cy, editor.desired_cx), (5, 1, 20));

        editor.move_cursor(KeyCode::Down, 24);
        assert_eq!((editor.cx, editor.cy, editor.desired_cx), (16, 2, 20));

        editor.move_cursor(KeyCode::Up, 24);
        assert_eq!((editor.cx, editor.cy, editor.desired_cx), (5, 1, 20));

        editor.move_cursor(KeyCode::Up, 24);
        assert_eq!((editor.cx, editor.cy, editor.desired_cx), (20, 0, 20));
    }

    #[test]
    fn horizontal_navigation_wraps_across_lines_and_updates_desired_cx() {
        let buf = sample_buffer(&["abc", "de"]);
        let mut editor = Editor::new(buf);

        editor.move_cursor(KeyCode::End, 24);
        assert_eq!((editor.cx, editor.cy), (3, 0));

        editor.move_cursor(KeyCode::Right, 24);
        assert_eq!((editor.cx, editor.cy, editor.desired_cx), (0, 1, 0));

        editor.move_cursor(KeyCode::Left, 24);
        assert_eq!((editor.cx, editor.cy, editor.desired_cx), (3, 0, 3));
    }

    #[test]
    fn viewport_scroll_updates_row_and_col_offsets() {
        let buf = sample_buffer(&[
            "0123456789abcdef",
            "line1",
            "line2",
            "line3",
            "line4",
            "line5",
        ]);
        let mut editor = Editor::new(buf);
        editor.cx = 12;
        editor.cy = 5;

        editor.scroll(10, 4);
        assert_eq!(editor.col_offset, 3);
        assert_eq!(editor.row_offset, 2);

        editor.cx = 1;
        editor.cy = 1;
        editor.scroll(10, 4);
        assert_eq!(editor.col_offset, 1);
        assert_eq!(editor.row_offset, 1);
    }

    #[test]
    fn page_up_and_page_down_clamp_to_buffer_bounds() {
        let buf = sample_buffer(&["l0", "l1", "l2", "l3", "l4", "l5"]);
        let mut editor = Editor::new(buf);

        editor.move_cursor(KeyCode::PageDown, 3);
        assert_eq!(editor.cy, 3);

        editor.move_cursor(KeyCode::PageDown, 3);
        assert_eq!(editor.cy, 5);

        editor.move_cursor(KeyCode::PageUp, 4);
        assert_eq!(editor.cy, 1);

        editor.move_cursor(KeyCode::PageUp, 4);
        assert_eq!(editor.cy, 0);
    }

    #[test]
    fn dirty_quit_requires_two_consecutive_ctrl_q_presses() {
        let mut editor = Editor::new(Buffer::new());
        editor.handle_key_event(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE), 24);
        assert!(editor.buffer.dirty);

        editor.handle_key_event(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::CONTROL), 24);
        assert!(!editor.should_quit);
        assert!(editor.quit_confirm_pending);

        // Any intervening key resets the quit confirmation state
        editor.handle_key_event(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE), 24);
        assert!(!editor.quit_confirm_pending);

        editor.handle_key_event(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::CONTROL), 24);
        assert!(!editor.should_quit);

        editor.handle_key_event(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::CONTROL), 24);
        assert!(editor.should_quit);
    }

    #[test]
    fn amodal_editing_and_unnamed_save_prompt_flow() {
        let mut editor = Editor::new(Buffer::new());
        editor.handle_key_event(KeyEvent::new(KeyCode::Char('h'), KeyModifiers::NONE), 24);
        editor.handle_key_event(KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE), 24);
        editor.handle_key_event(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), 24);
        editor.handle_key_event(KeyEvent::new(KeyCode::Char('!'), KeyModifiers::NONE), 24);

        assert_eq!(editor.buffer.len(), 2);
        assert_eq!(editor.buffer.contents(), "hi\n!");
        assert_eq!((editor.cx, editor.cy), (1, 1));

        // Pressing Ctrl+S on an unnamed buffer opens the SaveAs prompt
        editor.handle_key_event(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL), 24);
        assert_eq!(
            editor.prompt,
            PromptState::SaveAs {
                input: String::new()
            }
        );

        editor.handle_key_event(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE), 24);
        assert_eq!(editor.prompt, PromptState::None);
    }

    #[test]
    fn gutter_width_and_cursor_screen_coordinates_scale_with_line_count() {
        let mut lines = Vec::new();
        for i in 0..9 {
            lines.push(format!("line {i}"));
        }
        let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
        let mut editor = Editor::new(Buffer::from_lines(&refs));
        assert_eq!(editor.gutter_width(), 3);
        assert_eq!(editor.format_gutter_line(0), " 1 ");

        let mut hundred_lines = Vec::new();
        for i in 0..100 {
            hundred_lines.push(format!("line {i}"));
        }
        let hundred_refs: Vec<&str> = hundred_lines.iter().map(String::as_str).collect();
        editor = Editor::new(Buffer::from_lines(&hundred_refs));
        assert_eq!(editor.gutter_width(), 5);
        assert_eq!(editor.format_gutter_line(0), "   1 ");
        assert_eq!(editor.format_gutter_line(99), " 100 ");

        editor.cx = 8;
        editor.cy = 12;
        editor.col_offset = 3;
        editor.row_offset = 5;
        assert_eq!(editor.cursor_screen_position(), (5 + (8 - 3), 12 - 5));
    }

    #[test]
    fn status_bar_formats_left_and_right_sections_to_exact_width() {
        let mut buf = Buffer::from_lines(&["first", "second"]);
        buf.filename = Some("notes.txt".to_string());
        buf.dirty = true;
        let mut editor = Editor::new(buf);
        editor.cx = 3;
        editor.cy = 1;

        let bar = editor.format_status_bar(50);
        assert_eq!(bar.chars().count(), 50);
        assert!(bar.starts_with(" notes.txt [+]"));
        assert!(bar.ends_with("Ln 2, Col 4 | 2 lines "));
    }

    #[test]
    fn undo_and_redo_actions_work_via_key_events() {
        let mut editor = Editor::new(Buffer::from_lines(&["hello"]));
        editor.move_cursor(KeyCode::End, 24);
        editor.handle_key_event(KeyEvent::new(KeyCode::Char('!'), KeyModifiers::NONE), 24);
        assert_eq!(editor.buffer.contents(), "hello!");
        assert_eq!(editor.cx, 6);

        // Ctrl+Z -> Undo
        editor.handle_key_event(KeyEvent::new(KeyCode::Char('z'), KeyModifiers::CONTROL), 24);
        assert_eq!(editor.buffer.contents(), "hello");
        assert_eq!(editor.cx, 5);

        // Ctrl+Y -> Redo
        editor.handle_key_event(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::CONTROL), 24);
        assert_eq!(editor.buffer.contents(), "hello!");
        assert_eq!(editor.cx, 6);
    }

    #[test]
    fn find_prompt_jumps_between_matches_and_esc_restores_cursor() {
        let mut editor = Editor::new(Buffer::from_lines(&[
            "first target here",
            "second target there",
        ]));
        editor.cx = 2;
        editor.cy = 0;

        // Open search with Ctrl+F
        editor.handle_key_event(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::CONTROL), 24);
        for ch in "target".chars() {
            editor.handle_key_event(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE), 24);
        }
        assert_eq!((editor.cx, editor.cy), (6, 0));

        // Advance to next match with Down
        editor.handle_key_event(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE), 24);
        assert_eq!((editor.cx, editor.cy), (7, 1));

        // Move back to previous match with Up
        editor.handle_key_event(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE), 24);
        assert_eq!((editor.cx, editor.cy), (6, 0));

        // Esc cancels and restores original (2, 0) cursor position
        editor.handle_key_event(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE), 24);
        assert_eq!(editor.prompt, PromptState::None);
        assert_eq!((editor.cx, editor.cy), (2, 0));
    }

    #[test]
    fn tab_insertion_and_visual_cursor_align_with_expanded_tab_stops() {
        let mut editor = Editor::new(Buffer::new());
        editor.handle_key_event(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE), 24);
        editor.handle_key_event(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE), 24);

        // Logical buffer stores raw '\t' + 'x' (2 chars), while render expands '\t' to 4 spaces (5 visual cols)
        assert_eq!(editor.buffer.contents(), "\tx");
        assert_eq!(editor.buffer.get_row(0).unwrap().render, "    x");
        assert_eq!(editor.cx, 2);
        assert_eq!(editor.visual_cx(), 5);
        // Gutter width for 1 line is 3 (" 1 "), so screen_cx is 3 + 5 = 8
        assert_eq!(editor.cursor_screen_position(), (8, 0));
    }
}
