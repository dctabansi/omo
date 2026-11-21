use anyhow::Result;
use crossterm::{
    cursor,
    event::{
        self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent,
        MouseEventKind,
    },
    queue,
    style::{
        Attribute, Color, Print, ResetColor, SetAttribute, SetBackgroundColor, SetForegroundColor,
    },
    terminal::{Clear, ClearType},
};
use std::io::{Write, stdout};
use std::time::Instant;

use crate::buffer::{
    Buffer, Position, Selection, char_display_width, sanitize_render_char, str_display_width,
};
use crate::clipboard::ClipboardManager;
use crate::config::{Action, Config};
use crate::terminal::Terminal;

fn truncate_to_display_width(s: &str, max_cols: usize) -> (String, usize) {
    let mut out = String::new();
    let mut width = 0;
    for c in s.chars() {
        let safe = sanitize_render_char(c);
        let w = char_display_width(safe);
        if width + w > max_cols {
            break;
        }
        out.push(safe);
        width += w;
    }
    (out, width)
}

fn tail_to_display_width(s: &str, max_cols: usize) -> (String, usize) {
    let chars: Vec<char> = s.chars().map(sanitize_render_char).collect();
    let mut width = 0;
    let mut start_idx = chars.len();
    for (idx, &c) in chars.iter().enumerate().rev() {
        let w = char_display_width(c);
        if width + w > max_cols {
            break;
        }
        width += w;
        start_idx = idx;
    }
    (chars[start_idx..].iter().collect(), width)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PromptState {
    None,
    SaveAs {
        input: String,
    },
    ConfirmSudoSave {
        path: String,
    },
    Search {
        query: String,
        saved_cx: usize,
        saved_cy: usize,
        case_sensitive: bool,
    },
    GotoLine {
        input: String,
    },
}

fn matching_closing_delimiter(c: char) -> Option<char> {
    match c {
        '(' => Some(')'),
        '{' => Some('}'),
        '[' => Some(']'),
        '"' => Some('"'),
        '\'' => Some('\''),
        '`' => Some('`'),
        _ => None,
    }
}

fn is_closing_delimiter(c: char) -> bool {
    matches!(c, ')' | '}' | ']' | '"' | '\'' | '`')
}

fn is_matching_empty_pair(left: char, right: char) -> bool {
    matches!(
        (left, right),
        ('(', ')') | ('{', '}') | ('[', ']') | ('"', '"') | ('\'', '\'') | ('`', '`')
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditorCommand {
    Action(Action),
    Move { code: KeyCode, select: bool },
    MoveWord { forward: bool, select: bool },
    InsertChar(char),
    InsertNewline,
    Backspace,
    Delete,
    InsertTab,
    Noop,
}

const DOUBLE_CLICK_MS: u128 = 450;
const MOUSE_SCROLL_LINES: usize = 3;

pub struct Editor {
    should_quit: bool,
    buffer: Buffer,
    pub config: Config,
    pub selection: Option<Selection>,
    pub clipboard: ClipboardManager,
    last_click: Option<(Instant, Position, u8)>,
    pub viewport_scrolled_by_mouse: bool,
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
        let (tab_width, _) = config.resolve_filetype_settings(buffer.filename.as_deref());
        buffer.set_tab_width(tab_width);
        let status_message = if buffer.read_only {
            let name = buffer.filename.as_deref().unwrap_or("[No Name]");
            Some(format!(
                "WARNING: '{name}' is read-only (saving will require sudo)."
            ))
        } else {
            None
        };
        Self {
            should_quit: false,
            buffer,
            config,
            selection: None,
            clipboard: ClipboardManager::new(),
            last_click: None,
            viewport_scrolled_by_mouse: false,
            cx: 0,
            cy: 0,
            rx: 0,
            desired_cx: 0,
            row_offset: 0,
            col_offset: 0,
            quit_confirm_pending: false,
            status_message,
            prompt: PromptState::None,
        }
    }

    /// Returns the active non-empty selection, if any.
    pub fn active_selection(&self) -> Option<Selection> {
        self.selection.filter(|s| !s.is_empty())
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

    /// Returns the 0-based terminal screen row where the text viewport begins.
    pub fn first_text_screen_row(&self) -> u16 {
        0
    }

    /// Computes the text viewport dimensions `(text_cols, text_rows)` after reserving the gutter
    /// and the bottom status bar row.
    pub fn text_viewport(&self, cols: u16, rows: u16) -> (usize, usize) {
        let text_cols = (cols as usize).saturating_sub(self.gutter_width());
        let text_rows = (rows as usize).saturating_sub(1);
        (text_cols, text_rows)
    }

    /// Returns `true` if the active cursor should be shown on screen for a viewport of `text_rows` lines.
    /// When the user has scrolled the viewport away from `self.cy` with the mouse wheel (and no
    /// bottom-bar prompt is active), the hardware cursor is hidden until the cursor re-enters view.
    pub fn is_cursor_visible(&self, text_rows: usize) -> bool {
        if matches!(
            self.prompt,
            PromptState::SaveAs { .. }
                | PromptState::ConfirmSudoSave { .. }
                | PromptState::GotoLine { .. }
        ) {
            return true;
        }
        let rows_bound = text_rows.max(1);
        self.cy >= self.row_offset && self.cy < self.row_offset + rows_bound
    }

    /// Translates terminal screen coordinates `(screen_col, screen_row)` into buffer coordinates
    /// `Position { row, col }`, returning `None` if the click occurred outside the text viewport.
    /// Coordinates inside the line-number gutter (`screen_col < gutter_width`) clamp to column `0`.
    pub fn screen_to_buffer_pos(
        &self,
        screen_col: u16,
        screen_row: u16,
        total_rows: u16,
    ) -> Option<Position> {
        let top = self.first_text_screen_row();
        let (_, text_rows) = self.text_viewport(80, total_rows);
        if screen_row < top || (screen_row - top) as usize >= text_rows {
            return None;
        }
        if self.buffer.is_empty() {
            return Some(Position::new(0, 0));
        }

        let rel_row = (screen_row - top) as usize;
        let file_row = (self.row_offset + rel_row).min(self.buffer.len() - 1);
        let gutter_w = self.gutter_width();
        let cx = if (screen_col as usize) < gutter_w {
            0
        } else {
            let target_rx = self.col_offset + (screen_col as usize - gutter_w);
            self.buffer
                .get_row(file_row)
                .map_or(0, |row| row.rx_to_cx(target_rx))
        };
        Some(Position::new(file_row, cx))
    }

    /// Computes the hardware cursor screen coordinates `(screen_cx, screen_cy)` using visual tab-expanded `rx`.
    pub fn cursor_screen_position(&self) -> (u16, u16) {
        let rx = self.visual_cx();
        let screen_cx = (self.gutter_width() + rx.saturating_sub(self.col_offset))
            .min(u16::MAX as usize) as u16;
        let screen_cy = (self.first_text_screen_row() as usize
            + self.cy.saturating_sub(self.row_offset))
        .min(u16::MAX as usize) as u16;
        (screen_cx, screen_cy)
    }

    /// Computes the active hardware cursor screen coordinates, placing the cursor on the bottom
    /// status bar when typing in the `SaveAs`, `ConfirmSudoSave`, or `GotoLine` prompt and clamping to terminal bounds.
    pub fn active_cursor_screen_position(&self, cols: u16, rows: u16) -> (u16, u16) {
        let max_col = cols.saturating_sub(1);
        let max_row = rows.saturating_sub(1);
        match &self.prompt {
            PromptState::SaveAs { input } => {
                let prompt_prefix = format!(" Save as: {input}");
                let width = str_display_width(&prompt_prefix).min(u16::MAX as usize) as u16;
                return (width.min(max_col), max_row);
            }
            PromptState::ConfirmSudoSave { .. } => {
                let prompt_prefix = " Permission denied. Save with sudo? (y/n): ";
                let width = str_display_width(prompt_prefix).min(u16::MAX as usize) as u16;
                return (width.min(max_col), max_row);
            }
            PromptState::GotoLine { input } => {
                let prompt_prefix = format!(" Go to line: {input}");
                let width = str_display_width(&prompt_prefix).min(u16::MAX as usize) as u16;
                return (width.min(max_col), max_row);
            }
            _ => {}
        }
        let (screen_cx, screen_cy) = self.cursor_screen_position();
        let max_text_row = rows.saturating_sub(2);
        (screen_cx.min(max_col), screen_cy.min(max_text_row))
    }

    /// Formats the status bar string padded to `cols` terminal columns.
    pub fn format_status_bar(&self, cols: usize) -> String {
        if cols == 0 {
            return String::new();
        }

        let left = match &self.prompt {
            PromptState::SaveAs { input } => format!(" Save as: {input}"),
            PromptState::ConfirmSudoSave { .. } => {
                " Permission denied. Save with sudo? (y/n): ".to_string()
            }
            PromptState::Search {
                query,
                case_sensitive,
                ..
            } => {
                let mode = if *case_sensitive { "Aa" } else { "aa" };
                format!(" Search [{mode}] (Alt+C case, arrows/Enter): {query}")
            }
            PromptState::GotoLine { input } => format!(" Go to line: {input}"),
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

        let (clean_left, left_len) = truncate_to_display_width(&left, usize::MAX);
        let (clean_right, right_len) = truncate_to_display_width(&right, usize::MAX);

        if left_len + right_len <= cols {
            let padding = " ".repeat(cols - left_len - right_len);
            format!("{clean_left}{padding}{clean_right}")
        } else if !matches!(self.prompt, PromptState::None) {
            if left_len <= cols {
                let padding = " ".repeat(cols - left_len);
                format!("{clean_left}{padding}")
            } else {
                let (tail, tail_w) = tail_to_display_width(&clean_left, cols);
                let padding = " ".repeat(cols - tail_w);
                format!("{tail}{padding}")
            }
        } else if right_len < cols {
            let keep_left = cols - right_len;
            let (truncated_left, trunc_w) = truncate_to_display_width(&clean_left, keep_left);
            let padding = " ".repeat(keep_left - trunc_w);
            format!("{truncated_left}{padding}{clean_right}")
        } else {
            let (truncated_right, trunc_w) = truncate_to_display_width(&clean_right, cols);
            let padding = " ".repeat(cols - trunc_w);
            format!("{truncated_right}{padding}")
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
        self.buffer.break_undo_coalesce();
        match code {
            KeyCode::Up if self.cy > 0 => {
                self.cy -= 1;
                self.cx = self
                    .buffer
                    .get_row(self.cy)
                    .map_or(0, |row| row.rx_to_cx(self.desired_cx));
            }
            KeyCode::Down if self.cy + 1 < self.buffer.len() => {
                self.cy += 1;
                self.cx = self
                    .buffer
                    .get_row(self.cy)
                    .map_or(0, |row| row.rx_to_cx(self.desired_cx));
            }
            KeyCode::Left => {
                if self.cx > 0 {
                    self.cx -= 1;
                    self.desired_cx = self.visual_cx();
                } else if self.cy > 0 {
                    self.cy -= 1;
                    self.cx = self.current_row_len();
                    self.desired_cx = self.visual_cx();
                }
            }
            KeyCode::Right => {
                let row_len = self.current_row_len();
                if self.cx < row_len {
                    self.cx += 1;
                    self.desired_cx = self.visual_cx();
                } else if self.cy + 1 < self.buffer.len() {
                    self.cy += 1;
                    self.cx = 0;
                    self.desired_cx = 0;
                }
            }
            KeyCode::Home => {
                let first_non_ws = self
                    .buffer
                    .get_row(self.cy)
                    .map_or(0, |row| row.first_non_whitespace_cx());
                if self.cx == first_non_ws {
                    self.cx = 0;
                } else {
                    self.cx = first_non_ws;
                }
                self.desired_cx = self.visual_cx();
            }
            KeyCode::End => {
                self.cx = self.current_row_len();
                self.desired_cx = self.visual_cx();
            }
            KeyCode::PageUp => {
                let step = viewport_rows.max(1);
                self.cy = self.cy.saturating_sub(step);
                self.cx = self
                    .buffer
                    .get_row(self.cy)
                    .map_or(0, |row| row.rx_to_cx(self.desired_cx));
            }
            KeyCode::PageDown if !self.buffer.is_empty() => {
                let step = viewport_rows.max(1);
                self.cy = (self.cy + step).min(self.buffer.len() - 1);
                self.cx = self
                    .buffer
                    .get_row(self.cy)
                    .map_or(0, |row| row.rx_to_cx(self.desired_cx));
            }
            _ => {}
        }
    }

    /// Moves the cursor with optional selection extension (`Shift` held).
    pub fn move_cursor_with_selection(
        &mut self,
        code: KeyCode,
        viewport_rows: usize,
        select: bool,
    ) {
        if select {
            let anchor = self
                .selection
                .map_or(Position::new(self.cy, self.cx), |s| s.anchor);
            self.move_cursor(code, viewport_rows);
            let head = Position::new(self.cy, self.cx);
            self.selection = if anchor == head {
                None
            } else {
                Some(Selection::new(anchor, head))
            };
        } else {
            self.selection = None;
            self.move_cursor(code, viewport_rows);
        }
    }

    /// Moves the cursor by one word left or right with optional selection extension.
    pub fn move_word_with_selection(&mut self, forward: bool, select: bool) {
        if select {
            let anchor = self
                .selection
                .map_or(Position::new(self.cy, self.cx), |s| s.anchor);
            if forward {
                self.trigger_word_right_raw();
            } else {
                self.trigger_word_left_raw();
            }
            let head = Position::new(self.cy, self.cx);
            self.selection = if anchor == head {
                None
            } else {
                Some(Selection::new(anchor, head))
            };
        } else {
            self.selection = None;
            if forward {
                self.trigger_word_right_raw();
            } else {
                self.trigger_word_left_raw();
            }
        }
    }

    /// Inserts a single character at the cursor position and advances `cx`.
    /// When `auto_pairs` is enabled for the active buffer, handles closing-delimiter skip-over
    /// and auto-inserts matching closing brackets/quotes with context awareness.
    pub fn insert_char(&mut self, c: char) {
        if let Some(sel) = self.active_selection() {
            let (start, end) = sel.ordered();
            self.buffer.record_snapshot(self.cx, self.cy);
            let pos = self.buffer.delete_range_no_snapshot(start, end);
            self.selection = None;
            self.cx = pos.col;
            self.cy = pos.row;
            self.buffer.insert_char_no_snapshot(self.cx, self.cy, c);
            self.cx += 1;
            self.desired_cx = self.visual_cx();
            return;
        }

        let auto_pairs = self
            .config
            .resolve_auto_pairs(self.buffer.filename.as_deref());
        if auto_pairs && c != '\t' {
            let next_ch = self
                .buffer
                .get_row(self.cy)
                .and_then(|r| r.char_at(self.cx));
            let prev_ch = if self.cx > 0 {
                self.buffer
                    .get_row(self.cy)
                    .and_then(|r| r.char_at(self.cx - 1))
            } else {
                None
            };

            // Closing delimiter skip-over (overtype) when the exact closing delimiter is already under cursor
            if is_closing_delimiter(c) && next_ch == Some(c) {
                self.buffer.break_undo_coalesce();
                self.cx += 1;
                self.desired_cx = self.visual_cx();
                return;
            }

            if let Some(close_ch) = matching_closing_delimiter(c) {
                let suppress_for_quote_after_word = matches!(c, '\'' | '`')
                    && prev_ch.is_some_and(|p| p.is_alphanumeric() || p == '_');
                let suppress_before_word = next_ch.is_some_and(|n| n.is_alphanumeric() || n == '_');

                if !suppress_for_quote_after_word && !suppress_before_word {
                    self.buffer.insert_char(self.cx, self.cy, c);
                    self.buffer
                        .insert_char_no_snapshot(self.cx + 1, self.cy, close_ch);
                    self.cx += 1;
                    self.desired_cx = self.visual_cx();
                    return;
                }
            }
        }

        self.buffer.insert_char(self.cx, self.cy, c);
        self.cx += 1;
        self.desired_cx = self.visual_cx();
    }

    /// Inserts either soft-tab spaces up to the next tab stop (when `expand_tabs` is `true`
    /// for the active file type) or a literal `'\t'` character (when `expand_tabs` is `false`).
    /// When a selection is active, indents all lines covered by the selection by one tab stop.
    pub fn insert_tab(&mut self) {
        if let Some(sel) = self.active_selection() {
            let (start_row, end_row) = sel.line_range();
            let (tab_width, expand_tabs) = self
                .config
                .resolve_filetype_settings(self.buffer.filename.as_deref());
            self.cx = self.buffer.indent_lines(
                start_row,
                end_row,
                tab_width,
                expand_tabs,
                self.cx,
                self.cy,
            );
            let end_col = self.buffer.get_row(end_row).map_or(0, |r| r.len);
            self.selection = Some(Selection::new(
                Position::new(start_row, 0),
                Position::new(end_row, end_col),
            ));
            self.desired_cx = self.visual_cx();
            return;
        }

        let (tab_width, expand_tabs) = self
            .config
            .resolve_filetype_settings(self.buffer.filename.as_deref());
        if expand_tabs {
            let step = tab_width.max(1);
            let spaces = step - (self.visual_cx() % step);
            self.buffer.break_undo_coalesce();
            self.buffer.record_snapshot(self.cx, self.cy);
            for _ in 0..spaces {
                self.buffer.insert_char_no_snapshot(self.cx, self.cy, ' ');
                self.cx += 1;
            }
            self.desired_cx = self.visual_cx();
        } else {
            self.insert_char('\t');
        }
    }

    /// Splits the current line at the cursor (preserving indentation when `auto_indent` is enabled)
    /// and moves to the new line. Replaces any active selection first.
    pub fn insert_newline(&mut self) {
        if let Some(sel) = self.active_selection() {
            let (start, end) = sel.ordered();
            self.buffer.record_snapshot(self.cx, self.cy);
            let pos = self.buffer.delete_range_no_snapshot(start, end);
            self.selection = None;
            let (new_cx, new_cy) = self.buffer.insert_str_no_snapshot(pos.col, pos.row, "\n");
            self.cx = new_cx;
            self.cy = new_cy;
            self.desired_cx = self.visual_cx();
            return;
        }

        let (new_cx, new_cy) =
            self.buffer
                .insert_newline_with_indent(self.cx, self.cy, self.config.auto_indent);
        self.cx = new_cx;
        self.cy = new_cy;
        self.desired_cx = self.visual_cx();
    }

    /// Deletes the character to the left of the cursor (or the active selection) or joins with the previous line.
    /// When `auto_pairs` is enabled and the cursor sits inside an empty delimiter pair `(|)`,
    /// deletes both delimiters in a single undo step.
    pub fn backspace(&mut self) {
        if let Some(sel) = self.active_selection() {
            let (start, end) = sel.ordered();
            let pos = self.buffer.delete_range(start, end, self.cx, self.cy);
            self.selection = None;
            self.cx = pos.col;
            self.cy = pos.row;
            self.desired_cx = self.visual_cx();
            return;
        }

        if self
            .config
            .resolve_auto_pairs(self.buffer.filename.as_deref())
            && self.cx > 0
            && let Some(row) = self.buffer.get_row(self.cy)
            && let (Some(left), Some(right)) = (row.char_at(self.cx - 1), row.char_at(self.cx))
            && is_matching_empty_pair(left, right)
        {
            self.buffer.record_snapshot(self.cx, self.cy);
            self.buffer.delete_forward_no_snapshot(self.cx, self.cy);
            if let Some((new_cx, new_cy)) = self.buffer.delete_char_no_snapshot(self.cx, self.cy) {
                self.cx = new_cx;
                self.cy = new_cy;
                self.desired_cx = self.visual_cx();
            }
            return;
        }

        if let Some((new_cx, new_cy)) = self.buffer.delete_char(self.cx, self.cy) {
            self.cx = new_cx;
            self.cy = new_cy;
            self.desired_cx = self.visual_cx();
        }
    }

    /// Deletes the character under the cursor (or the active selection) or joins the next line.
    pub fn delete_forward(&mut self) {
        if let Some(sel) = self.active_selection() {
            let (start, end) = sel.ordered();
            let pos = self.buffer.delete_range(start, end, self.cx, self.cy);
            self.selection = None;
            self.cx = pos.col;
            self.cy = pos.row;
            self.desired_cx = self.visual_cx();
            return;
        }

        self.buffer.delete_forward(self.cx, self.cy);
        self.desired_cx = self.visual_cx();
    }

    fn handle_save_result(&mut self, path: &str, result: Result<usize>) -> bool {
        match result {
            Ok(bytes) => {
                let (tab_width, _) = self
                    .config
                    .resolve_filetype_settings(self.buffer.filename.as_deref());
                self.buffer.set_tab_width(tab_width);
                self.status_message = Some(format!("Wrote {bytes} bytes to {path}"));
                true
            }
            Err(err) => {
                let is_perm_denied = err
                    .downcast_ref::<std::io::Error>()
                    .is_some_and(|e| e.kind() == std::io::ErrorKind::PermissionDenied);
                if is_perm_denied {
                    self.prompt = PromptState::ConfirmSudoSave {
                        path: path.to_string(),
                    };
                    self.status_message = None;
                } else {
                    self.status_message = Some(format!("Save error: {err}"));
                }
                false
            }
        }
    }

    /// Saves the buffer or opens the inline Save-As prompt if the buffer is unnamed.
    /// Returns `true` if the buffer was saved immediately without error or prompt.
    pub fn trigger_save(&mut self) -> bool {
        if let Some(name) = self.buffer.filename.clone() {
            let res = self.buffer.save();
            self.handle_save_result(&name, res)
        } else {
            self.prompt = PromptState::SaveAs {
                input: String::new(),
            };
            self.status_message = None;
            false
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
        self.selection = None;
        if let Some((cx, cy)) = self.buffer.undo(self.cx, self.cy) {
            self.cx = cx;
            self.cy = cy;
            self.desired_cx = self.visual_cx();
            self.status_message = Some("Undo.".to_string());
        } else {
            self.status_message = Some("Already at oldest change.".to_string());
        }
    }

    /// Reapplies the most recently undone buffer edit and restores its cursor coordinates.
    pub fn trigger_redo(&mut self) {
        self.selection = None;
        if let Some((cx, cy)) = self.buffer.redo(self.cx, self.cy) {
            self.cx = cx;
            self.cy = cy;
            self.desired_cx = self.visual_cx();
            self.status_message = Some("Redo.".to_string());
        } else {
            self.status_message = Some("Already at newest change.".to_string());
        }
    }

    /// Copies the active selection (or the current line when no selection is active) to the clipboard.
    pub fn trigger_copy(&mut self) {
        if let Some(sel) = self.active_selection() {
            let (start, end) = sel.ordered();
            let text = self.buffer.extract_range(start, end);
            let char_count = text.chars().count();
            self.clipboard.set_text(text);
            self.status_message = Some(format!("Copied {char_count} chars."));
        } else if let Some(row) = self.buffer.get_row(self.cy) {
            let text = format!("{}\n", row.text());
            self.clipboard.set_text(text);
            self.status_message = Some("Copied 1 line.".to_string());
        }
    }

    /// Cuts the active selection (or the current line when no selection is active) to the clipboard.
    pub fn trigger_cut(&mut self) {
        if let Some(sel) = self.active_selection() {
            let (start, end) = sel.ordered();
            let text = self.buffer.extract_range(start, end);
            let char_count = text.chars().count();
            self.clipboard.set_text(text);
            let pos = self.buffer.delete_range(start, end, self.cx, self.cy);
            self.selection = None;
            self.cx = pos.col;
            self.cy = pos.row;
            self.desired_cx = self.visual_cx();
            self.status_message = Some(format!("Cut {char_count} chars."));
        } else if let Some(row) = self.buffer.get_row(self.cy) {
            let text = format!("{}\n", row.text());
            self.clipboard.set_text(text);
            if let Some((_, new_cy)) = self.buffer.delete_line(self.cx, self.cy) {
                self.cy = new_cy;
                self.cx = self
                    .buffer
                    .get_row(self.cy)
                    .map_or(0, |r| r.rx_to_cx(self.desired_cx));
            }
            self.status_message = Some("Cut 1 line.".to_string());
        }
    }

    /// Pastes clipboard contents at the cursor (replacing any active selection) in a single undo step.
    pub fn trigger_paste(&mut self) {
        let text = self.clipboard.get_text();
        if text.is_empty() {
            return;
        }
        self.buffer.break_undo_coalesce();
        if let Some(sel) = self.active_selection() {
            let (start, end) = sel.ordered();
            self.buffer.record_snapshot(self.cx, self.cy);
            let pos = self.buffer.delete_range_no_snapshot(start, end);
            self.selection = None;
            let (new_cx, new_cy) = self.buffer.insert_str_no_snapshot(pos.col, pos.row, &text);
            self.cx = new_cx;
            self.cy = new_cy;
            self.desired_cx = self.visual_cx();
        } else {
            let (new_cx, new_cy) = self.buffer.insert_str(self.cx, self.cy, &text);
            self.cx = new_cx;
            self.cy = new_cy;
            self.desired_cx = self.visual_cx();
        }
    }

    /// Selects the entire buffer and places the cursor at the end of the document.
    pub fn trigger_select_all(&mut self) {
        if self.buffer.is_empty() {
            self.selection = None;
            return;
        }
        let last_row = self.buffer.len() - 1;
        let last_col = self.buffer.get_row(last_row).map_or(0, |r| r.len);
        let start = Position::new(0, 0);
        let end = Position::new(last_row, last_col);
        if start == end {
            self.selection = None;
        } else {
            self.selection = Some(Selection::new(start, end));
        }
        self.cy = last_row;
        self.cx = last_col;
        self.desired_cx = self.visual_cx();
    }

    /// Duplicates the current line directly below and moves the cursor to the duplicated line.
    pub fn trigger_duplicate_line(&mut self) {
        self.selection = None;
        if let Some((new_cx, new_cy)) = self.buffer.duplicate_line(self.cx, self.cy) {
            self.cx = new_cx;
            self.cy = new_cy;
            self.desired_cx = self.visual_cx();
        }
    }

    /// Deletes the current line and clamps the cursor to the remaining buffer rows.
    pub fn trigger_delete_line(&mut self) {
        self.selection = None;
        if let Some((_, new_cy)) = self.buffer.delete_line(self.cx, self.cy) {
            self.cy = new_cy;
            self.cx = self
                .buffer
                .get_row(self.cy)
                .map_or(0, |row| row.rx_to_cx(self.desired_cx));
        }
    }

    /// Moves the current line (or selected line block) up by one row (`Alt+Up`).
    pub fn trigger_move_line_up(&mut self) {
        if let Some(sel) = self.active_selection() {
            let (start_row, end_row) = sel.line_range();
            if self
                .buffer
                .move_lines_up(start_row, end_row, self.cx, self.cy)
            {
                self.cy = self.cy.saturating_sub(1);
                let new_start = start_row - 1;
                let new_end = end_row - 1;
                let end_col = self.buffer.get_row(new_end).map_or(0, |r| r.len);
                self.selection = Some(Selection::new(
                    Position::new(new_start, 0),
                    Position::new(new_end, end_col),
                ));
                self.desired_cx = self.visual_cx();
            }
            return;
        }

        if let Some((new_cx, new_cy)) = self.buffer.move_line_up(self.cx, self.cy) {
            self.cx = new_cx;
            self.cy = new_cy;
            self.desired_cx = self.visual_cx();
        }
    }

    /// Moves the current line (or selected line block) down by one row (`Alt+Down`).
    pub fn trigger_move_line_down(&mut self) {
        if let Some(sel) = self.active_selection() {
            let (start_row, end_row) = sel.line_range();
            if self
                .buffer
                .move_lines_down(start_row, end_row, self.cx, self.cy)
            {
                self.cy = (self.cy + 1).min(self.buffer.len().saturating_sub(1));
                let new_start = start_row + 1;
                let new_end = end_row + 1;
                let end_col = self.buffer.get_row(new_end).map_or(0, |r| r.len);
                self.selection = Some(Selection::new(
                    Position::new(new_start, 0),
                    Position::new(new_end, end_col),
                ));
                self.desired_cx = self.visual_cx();
            }
            return;
        }

        if let Some((new_cx, new_cy)) = self.buffer.move_line_down(self.cx, self.cy) {
            self.cx = new_cx;
            self.cy = new_cy;
            self.desired_cx = self.visual_cx();
        }
    }

    /// Inserts a new line below the current row at the same indentation level without splitting
    /// the current line, and moves the cursor to the end of the new line's indentation.
    pub fn trigger_insert_line_below(&mut self) {
        self.selection = None;
        let (new_cx, new_cy) = self.buffer.insert_line_below(self.cx, self.cy);
        self.cx = new_cx;
        self.cy = new_cy;
        self.desired_cx = self.visual_cx();
    }

    /// Inserts a new line above the current row at the same indentation level without splitting
    /// the current line, and moves the cursor to the end of the new line's indentation.
    pub fn trigger_insert_line_above(&mut self) {
        self.selection = None;
        let (new_cx, new_cy) = self.buffer.insert_line_above(self.cx, self.cy);
        self.cx = new_cx;
        self.cy = new_cy;
        self.desired_cx = self.visual_cx();
    }

    /// Indents the active selection block (or inserts a tab at the cursor when no selection is active).
    pub fn trigger_indent(&mut self) {
        self.insert_tab();
    }

    /// Removes one tab stop of leading indentation from the active selection block or current line.
    pub fn trigger_unindent(&mut self) {
        if let Some(sel) = self.active_selection() {
            let (start_row, end_row) = sel.line_range();
            self.cx = self
                .buffer
                .unindent_lines(start_row, end_row, self.cx, self.cy);
            let end_col = self.buffer.get_row(end_row).map_or(0, |r| r.len);
            let new_sel =
                Selection::new(Position::new(start_row, 0), Position::new(end_row, end_col));
            self.selection = if new_sel.is_empty() {
                None
            } else {
                Some(new_sel)
            };
            self.desired_cx = self.visual_cx();
            return;
        }

        if let Some(new_cx) = self.buffer.unindent_line(self.cx, self.cy) {
            self.cx = new_cx;
            self.desired_cx = self.visual_cx();
        }
    }

    fn snap_viewport_to_cursor(&mut self, viewport_rows: usize) {
        self.viewport_scrolled_by_mouse = false;
        if self.cy < self.row_offset {
            self.row_offset = self.cy;
        }
        if viewport_rows > 0 && self.cy >= self.row_offset + viewport_rows {
            self.row_offset = self.cy - viewport_rows + 1;
        }
    }

    /// Handles a terminal bracketed paste event, normalizing line endings and inserting the
    /// payload in a single atomic undo snapshot without auto-indent or `auto_pairs`.
    pub fn handle_paste_event(&mut self, text: &str) {
        let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
        if normalized.is_empty() {
            return;
        }

        match self.prompt.clone() {
            PromptState::SaveAs { mut input } => {
                let first_line = normalized.lines().next().unwrap_or("");
                input.push_str(first_line);
                self.prompt = PromptState::SaveAs { input };
                return;
            }
            PromptState::Search {
                mut query,
                saved_cx,
                saved_cy,
                case_sensitive,
            } => {
                let first_line = normalized.lines().next().unwrap_or("");
                query.push_str(first_line);
                if let Some((match_cx, match_cy)) =
                    self.buffer
                        .find_with_case(&query, saved_cx, saved_cy, true, case_sensitive)
                {
                    self.cx = match_cx;
                    self.cy = match_cy;
                    self.desired_cx = self.visual_cx();
                } else {
                    self.cx = saved_cx;
                    self.cy = saved_cy;
                    self.desired_cx = self.visual_cx();
                }
                self.prompt = PromptState::Search {
                    query,
                    saved_cx,
                    saved_cy,
                    case_sensitive,
                };
                return;
            }
            PromptState::GotoLine { mut input } => {
                for c in normalized.chars().filter(char::is_ascii_digit) {
                    input.push(c);
                }
                self.prompt = PromptState::GotoLine { input };
                return;
            }
            PromptState::ConfirmSudoSave { .. } => return,
            PromptState::None => {}
        }

        self.quit_confirm_pending = false;
        self.status_message = None;
        self.viewport_scrolled_by_mouse = false;
        self.buffer.break_undo_coalesce();

        if let Some(sel) = self.active_selection() {
            let (start, end) = sel.ordered();
            self.buffer.record_snapshot(self.cx, self.cy);
            let pos = self.buffer.delete_range_no_snapshot(start, end);
            self.selection = None;
            let (new_cx, new_cy) =
                self.buffer
                    .insert_str_no_snapshot(pos.col, pos.row, &normalized);
            self.cx = new_cx;
            self.cy = new_cy;
            self.desired_cx = self.visual_cx();
        } else {
            let (new_cx, new_cy) = self.buffer.insert_str(self.cx, self.cy, &normalized);
            self.cx = new_cx;
            self.cy = new_cy;
            self.desired_cx = self.visual_cx();
        }
    }

    /// Processes a mouse event (`Down`, `Drag`, `ScrollUp`, `ScrollDown`) against the editor state.
    pub fn handle_mouse_event(&mut self, mouse: MouseEvent, _cols: u16, rows: u16) {
        match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                let Some(pos) = self.screen_to_buffer_pos(mouse.column, mouse.row, rows) else {
                    return;
                };
                self.quit_confirm_pending = false;
                self.status_message = None;
                self.viewport_scrolled_by_mouse = false;
                self.buffer.break_undo_coalesce();

                if mouse.modifiers.contains(KeyModifiers::SHIFT) {
                    let anchor = self
                        .selection
                        .map_or(Position::new(self.cy, self.cx), |s| s.anchor);
                    self.cy = pos.row;
                    self.cx = pos.col;
                    self.desired_cx = self.visual_cx();
                    self.selection = if anchor == pos {
                        None
                    } else {
                        Some(Selection::new(anchor, pos))
                    };
                    self.last_click = None;
                    return;
                }

                let now = Instant::now();
                let click_count = match self.last_click {
                    Some((prev_time, prev_pos, count))
                        if now.duration_since(prev_time).as_millis() <= DOUBLE_CLICK_MS
                            && prev_pos.row == pos.row
                            && (count == 2 || prev_pos.col == pos.col) =>
                    {
                        if count >= 3 {
                            1
                        } else {
                            count + 1
                        }
                    }
                    _ => 1,
                };
                self.last_click = Some((now, pos, click_count));

                match click_count {
                    1 => {
                        self.selection = None;
                        self.cy = pos.row;
                        self.cx = pos.col;
                        self.desired_cx = self.visual_cx();
                    }
                    2 => {
                        let (w_start, w_end) = self.buffer.word_span_at(pos.col, pos.row);
                        self.cy = pos.row;
                        self.cx = w_end;
                        self.desired_cx = self.visual_cx();
                        let sel = Selection::new(
                            Position::new(pos.row, w_start),
                            Position::new(pos.row, w_end),
                        );
                        self.selection = if sel.is_empty() { None } else { Some(sel) };
                    }
                    _ => {
                        let row_len = self.buffer.get_row(pos.row).map_or(0, |r| r.len);
                        self.cy = pos.row;
                        self.cx = row_len;
                        self.desired_cx = self.visual_cx();
                        let sel = Selection::new(
                            Position::new(pos.row, 0),
                            Position::new(pos.row, row_len),
                        );
                        self.selection = if sel.is_empty() { None } else { Some(sel) };
                    }
                }
            }
            MouseEventKind::Drag(MouseButton::Left) => {
                let Some(pos) = self.screen_to_buffer_pos(mouse.column, mouse.row, rows) else {
                    return;
                };
                self.viewport_scrolled_by_mouse = false;
                let anchor = self
                    .selection
                    .map_or(Position::new(self.cy, self.cx), |s| s.anchor);
                self.cy = pos.row;
                self.cx = pos.col;
                self.desired_cx = self.visual_cx();
                self.selection = if anchor == pos {
                    None
                } else {
                    Some(Selection::new(anchor, pos))
                };
            }
            MouseEventKind::ScrollUp => {
                self.viewport_scrolled_by_mouse = true;
                self.row_offset = self.row_offset.saturating_sub(MOUSE_SCROLL_LINES);
            }
            MouseEventKind::ScrollDown => {
                self.viewport_scrolled_by_mouse = true;
                let max_offset = self.buffer.len().saturating_sub(1);
                self.row_offset = (self.row_offset + MOUSE_SCROLL_LINES).min(max_offset);
            }
            _ => {}
        }
    }

    fn trigger_word_left_raw(&mut self) {
        self.buffer.break_undo_coalesce();
        let (new_cx, new_cy) = self.buffer.word_boundary_left(self.cx, self.cy);
        self.cx = new_cx;
        self.cy = new_cy;
        self.desired_cx = self.visual_cx();
    }

    fn trigger_word_right_raw(&mut self) {
        self.buffer.break_undo_coalesce();
        let (new_cx, new_cy) = self.buffer.word_boundary_right(self.cx, self.cy);
        self.cx = new_cx;
        self.cy = new_cy;
        self.desired_cx = self.visual_cx();
    }

    /// Moves the cursor to the previous word boundary, clearing any active selection.
    pub fn trigger_word_left(&mut self) {
        self.selection = None;
        self.trigger_word_left_raw();
    }

    /// Moves the cursor to the next word boundary, clearing any active selection.
    pub fn trigger_word_right(&mut self) {
        self.selection = None;
        self.trigger_word_right_raw();
    }

    /// Deletes the word to the left of the cursor (or the active selection).
    pub fn trigger_delete_word_left(&mut self) {
        if self.active_selection().is_some() {
            self.backspace();
            return;
        }
        if let Some((new_cx, new_cy)) = self.buffer.delete_word_left(self.cx, self.cy) {
            self.cx = new_cx;
            self.cy = new_cy;
            self.desired_cx = self.visual_cx();
        }
    }

    /// Deletes the word to the right of the cursor (or the active selection).
    pub fn trigger_delete_word_right(&mut self) {
        if self.active_selection().is_some() {
            self.delete_forward();
            return;
        }
        self.buffer.delete_word_right(self.cx, self.cy);
        self.desired_cx = self.visual_cx();
    }

    /// Opens the interactive status-bar Go-To-Line prompt.
    pub fn trigger_goto_line(&mut self) {
        self.prompt = PromptState::GotoLine {
            input: String::new(),
        };
        self.status_message = None;
    }

    /// Opens the interactive status-bar search prompt (case-insensitive by default).
    pub fn trigger_find(&mut self) {
        self.prompt = PromptState::Search {
            query: String::new(),
            saved_cx: self.cx,
            saved_cy: self.cy,
            case_sensitive: false,
        };
        self.status_message = None;
    }

    /// Translates a `KeyEvent` into a unified `EditorCommand`, checking shifted selection
    /// movement and `Config` action bindings.
    pub fn resolve_command(&self, key: KeyEvent) -> EditorCommand {
        if key.modifiers.contains(KeyModifiers::SHIFT)
            && key
                .modifiers
                .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
            && matches!(key.code, KeyCode::Left | KeyCode::Right)
        {
            return EditorCommand::MoveWord {
                forward: key.code == KeyCode::Right,
                select: true,
            };
        }

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
                modifiers,
            ) if !modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) => {
                EditorCommand::Move {
                    code: key.code,
                    select: modifiers.contains(KeyModifiers::SHIFT),
                }
            }
            (KeyCode::Char(c), modifiers)
                if !modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                EditorCommand::InsertChar(c)
            }
            _ => EditorCommand::Noop,
        }
    }

    /// Executes a single `Action` and returns `true` if subsequent chained actions should continue.
    pub fn execute_action(&mut self, action: Action) -> bool {
        match action {
            Action::Quit => {
                self.trigger_quit();
                false
            }
            Action::Save => self.trigger_save(),
            Action::Undo => {
                self.trigger_undo();
                true
            }
            Action::Redo => {
                self.trigger_redo();
                true
            }
            Action::Find => {
                self.trigger_find();
                false
            }
            Action::Copy => {
                self.trigger_copy();
                true
            }
            Action::Cut => {
                self.trigger_cut();
                true
            }
            Action::Paste => {
                self.trigger_paste();
                true
            }
            Action::SelectAll => {
                self.trigger_select_all();
                true
            }
            Action::DuplicateLine => {
                self.trigger_duplicate_line();
                true
            }
            Action::DeleteLine => {
                self.trigger_delete_line();
                true
            }
            Action::MoveLineUp => {
                self.trigger_move_line_up();
                true
            }
            Action::MoveLineDown => {
                self.trigger_move_line_down();
                true
            }
            Action::InsertLineBelow => {
                self.trigger_insert_line_below();
                true
            }
            Action::InsertLineAbove => {
                self.trigger_insert_line_above();
                true
            }
            Action::Indent => {
                self.trigger_indent();
                true
            }
            Action::Unindent => {
                self.trigger_unindent();
                true
            }
            Action::WordLeft => {
                self.trigger_word_left();
                true
            }
            Action::WordRight => {
                self.trigger_word_right();
                true
            }
            Action::DeleteWordLeft => {
                self.trigger_delete_word_left();
                true
            }
            Action::DeleteWordRight => {
                self.trigger_delete_word_right();
                true
            }
            Action::GotoLine => {
                self.trigger_goto_line();
                false
            }
        }
    }

    /// Processes a single key event against the current editor state.
    pub fn handle_key_event(&mut self, key: KeyEvent, viewport_rows: usize) {
        if !matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) {
            return;
        }

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
                            let res = self.buffer.save_as(&trimmed);
                            self.handle_save_result(&trimmed, res);
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
            PromptState::ConfirmSudoSave { path } => {
                match key.code {
                    KeyCode::Char('y' | 'Y') | KeyCode::Enter
                        if !key
                            .modifiers
                            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
                    {
                        self.prompt = PromptState::None;
                        match self.buffer.save_with_sudo(&path) {
                            Ok(bytes) => {
                                let (tab_width, _) = self
                                    .config
                                    .resolve_filetype_settings(self.buffer.filename.as_deref());
                                self.buffer.set_tab_width(tab_width);
                                self.status_message =
                                    Some(format!("Wrote {bytes} bytes to {path} (sudo)"));
                            }
                            Err(err) => {
                                self.status_message = Some(format!("Sudo save error: {err}"));
                            }
                        }
                    }
                    KeyCode::Char('n' | 'N') | KeyCode::Esc => {
                        self.prompt = PromptState::None;
                        self.status_message = Some("Save aborted (permission denied).".to_string());
                    }
                    _ => {}
                }
                return;
            }
            PromptState::Search {
                mut query,
                saved_cx,
                saved_cy,
                mut case_sensitive,
            } => {
                match key.code {
                    KeyCode::Esc => {
                        self.cx = saved_cx;
                        self.cy = saved_cy;
                        self.desired_cx = self.visual_cx();
                        self.prompt = PromptState::None;
                        self.status_message = Some("Search canceled.".to_string());
                    }
                    KeyCode::Enter => {
                        self.desired_cx = self.visual_cx();
                        self.prompt = PromptState::None;
                        self.status_message = None;
                    }
                    KeyCode::Char('c' | 'C') if key.modifiers.contains(KeyModifiers::ALT) => {
                        case_sensitive = !case_sensitive;
                        if let Some((match_cx, match_cy)) = self.buffer.find_with_case(
                            &query,
                            saved_cx,
                            saved_cy,
                            true,
                            case_sensitive,
                        ) {
                            self.cx = match_cx;
                            self.cy = match_cy;
                            self.desired_cx = self.visual_cx();
                        } else {
                            self.cx = saved_cx;
                            self.cy = saved_cy;
                            self.desired_cx = self.visual_cx();
                        }
                        self.prompt = PromptState::Search {
                            query,
                            saved_cx,
                            saved_cy,
                            case_sensitive,
                        };
                    }
                    KeyCode::Down | KeyCode::Right => {
                        if let Some((match_cx, match_cy)) = self.buffer.find_with_case(
                            &query,
                            self.cx + 1,
                            self.cy,
                            true,
                            case_sensitive,
                        ) {
                            self.cx = match_cx;
                            self.cy = match_cy;
                            self.desired_cx = self.visual_cx();
                        }
                    }
                    KeyCode::Up | KeyCode::Left => {
                        if let Some((match_cx, match_cy)) = self.buffer.find_with_case(
                            &query,
                            self.cx,
                            self.cy,
                            false,
                            case_sensitive,
                        ) {
                            self.cx = match_cx;
                            self.cy = match_cy;
                            self.desired_cx = self.visual_cx();
                        }
                    }
                    KeyCode::Backspace => {
                        query.pop();
                        if let Some((match_cx, match_cy)) = self.buffer.find_with_case(
                            &query,
                            saved_cx,
                            saved_cy,
                            true,
                            case_sensitive,
                        ) {
                            self.cx = match_cx;
                            self.cy = match_cy;
                            self.desired_cx = self.visual_cx();
                        } else {
                            self.cx = saved_cx;
                            self.cy = saved_cy;
                            self.desired_cx = self.visual_cx();
                        }
                        self.prompt = PromptState::Search {
                            query,
                            saved_cx,
                            saved_cy,
                            case_sensitive,
                        };
                    }
                    KeyCode::Char(c)
                        if !key
                            .modifiers
                            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
                    {
                        query.push(c);
                        if let Some((match_cx, match_cy)) = self.buffer.find_with_case(
                            &query,
                            saved_cx,
                            saved_cy,
                            true,
                            case_sensitive,
                        ) {
                            self.cx = match_cx;
                            self.cy = match_cy;
                            self.desired_cx = self.visual_cx();
                        } else {
                            self.cx = saved_cx;
                            self.cy = saved_cy;
                            self.desired_cx = self.visual_cx();
                        }
                        self.prompt = PromptState::Search {
                            query,
                            saved_cx,
                            saved_cy,
                            case_sensitive,
                        };
                    }
                    _ => {}
                }
                return;
            }
            PromptState::GotoLine { mut input } => {
                match key.code {
                    KeyCode::Esc => {
                        self.prompt = PromptState::None;
                        self.status_message = Some("Go to line canceled.".to_string());
                    }
                    KeyCode::Enter => {
                        let trimmed = input.trim();
                        self.prompt = PromptState::None;
                        if let Ok(line_num) = trimmed.parse::<usize>()
                            && line_num > 0
                        {
                            if self.buffer.is_empty() {
                                self.cy = 0;
                                self.cx = 0;
                                self.desired_cx = 0;
                            } else {
                                self.cy = (line_num - 1).min(self.buffer.len() - 1);
                                self.cx = self
                                    .buffer
                                    .get_row(self.cy)
                                    .map_or(0, |row| row.rx_to_cx(self.desired_cx));
                            }
                            self.status_message = None;
                        } else {
                            self.status_message = Some("Invalid line number.".to_string());
                        }
                    }
                    KeyCode::Backspace => {
                        input.pop();
                        self.prompt = PromptState::GotoLine { input };
                    }
                    KeyCode::Char(c)
                        if c.is_ascii_digit()
                            && !key
                                .modifiers
                                .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
                    {
                        input.push(c);
                        self.prompt = PromptState::GotoLine { input };
                    }
                    _ => {}
                }
                return;
            }
            PromptState::None => {}
        }

        let is_shifted_word_move = key.modifiers.contains(KeyModifiers::SHIFT)
            && key
                .modifiers
                .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
            && matches!(key.code, KeyCode::Left | KeyCode::Right);

        if !is_shifted_word_move
            && let Some(actions) = self.config.actions_for_key(key).map(|s| s.to_vec())
        {
            if actions.first() != Some(&Action::Quit) {
                self.quit_confirm_pending = false;
                self.status_message = None;
            }
            for action in actions {
                let can_continue = self.execute_action(action);
                if !can_continue || self.should_quit || self.prompt != PromptState::None {
                    break;
                }
            }
            self.snap_viewport_to_cursor(viewport_rows);
            return;
        }

        let command = self.resolve_command(key);

        if command != EditorCommand::Action(Action::Quit) {
            self.quit_confirm_pending = false;
            if command != EditorCommand::Noop {
                self.status_message = None;
            }
        }

        match command {
            EditorCommand::Action(action) => {
                self.execute_action(action);
                self.snap_viewport_to_cursor(viewport_rows);
            }
            EditorCommand::Move { code, select } => {
                self.move_cursor_with_selection(code, viewport_rows, select);
                self.snap_viewport_to_cursor(viewport_rows);
            }
            EditorCommand::MoveWord { forward, select } => {
                self.move_word_with_selection(forward, select);
                self.snap_viewport_to_cursor(viewport_rows);
            }
            EditorCommand::InsertChar(c) => {
                self.insert_char(c);
                self.snap_viewport_to_cursor(viewport_rows);
            }
            EditorCommand::InsertNewline => {
                self.insert_newline();
                self.snap_viewport_to_cursor(viewport_rows);
            }
            EditorCommand::Backspace => {
                self.backspace();
                self.snap_viewport_to_cursor(viewport_rows);
            }
            EditorCommand::Delete => {
                self.delete_forward();
                self.snap_viewport_to_cursor(viewport_rows);
            }
            EditorCommand::InsertTab => {
                self.insert_tab();
                self.snap_viewport_to_cursor(viewport_rows);
            }
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
        if !self.viewport_scrolled_by_mouse {
            self.scroll(text_cols, text_rows);
        }

        let mut stdout = stdout();

        if let Some(osc52) = self.clipboard.take_osc52_sequence() {
            queue!(stdout, Print(osc52))?;
        }

        queue!(stdout, cursor::Hide, cursor::MoveTo(0, 0))?;
        self.draw_rows(&mut stdout, cols, rows)?;
        self.draw_status_bar(&mut stdout, cols)?;

        if self.is_cursor_visible(text_rows) {
            let (screen_cx, screen_cy) = self.active_cursor_screen_position(cols, rows);
            queue!(stdout, cursor::MoveTo(screen_cx, screen_cy), cursor::Show)?;
        }

        stdout.flush()?;
        Ok(())
    }

    fn draw_row_content(
        &self,
        stdout: &mut std::io::Stdout,
        file_row: usize,
        buffer_row: &crate::buffer::Row,
        text_cols: usize,
    ) -> Result<()> {
        let Some(sel) = self.active_selection() else {
            let render_text = buffer_row.slice_chars(self.col_offset, text_cols);
            queue!(stdout, Print(render_text))?;
            return Ok(());
        };

        let (start, end) = sel.ordered();
        if file_row < start.row || file_row > end.row {
            let render_text = buffer_row.slice_chars(self.col_offset, text_cols);
            queue!(stdout, Print(render_text))?;
            return Ok(());
        }

        let sel_start_cx = if file_row == start.row { start.col } else { 0 };
        let sel_end_cx = if file_row == end.row {
            end.col
        } else {
            buffer_row.len
        };
        let sel_start_rx = buffer_row.cx_to_rx(sel_start_cx);
        let sel_end_rx = buffer_row.cx_to_rx(sel_end_cx);

        if text_cols == 0 {
            return Ok(());
        }
        let end_col = self.col_offset.saturating_add(text_cols);
        let mut rx = 0;
        let mut current_selected = false;

        for c in buffer_row.render.chars() {
            let w = char_display_width(c);
            let next_rx = rx + w;
            if next_rx <= self.col_offset {
                rx = next_rx;
                continue;
            }
            if next_rx > end_col {
                break;
            }
            let is_sel = rx >= sel_start_rx && rx < sel_end_rx;
            if is_sel != current_selected {
                if is_sel {
                    if let Some((r, g, b)) = self.config.theme.selection_bg {
                        queue!(stdout, SetBackgroundColor(Color::Rgb { r, g, b }))?;
                    } else {
                        queue!(stdout, SetAttribute(Attribute::Reverse))?;
                    }
                    if let Some((r, g, b)) = self.config.theme.selection_fg {
                        queue!(stdout, SetForegroundColor(Color::Rgb { r, g, b }))?;
                    }
                } else {
                    queue!(stdout, SetAttribute(Attribute::NoReverse), ResetColor)?;
                    self.apply_editor_theme(stdout)?;
                }
                current_selected = is_sel;
            }
            queue!(stdout, Print(c))?;
            rx = next_rx;
        }

        if current_selected {
            queue!(stdout, SetAttribute(Attribute::NoReverse), ResetColor)?;
            self.apply_editor_theme(stdout)?;
        }
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
                queue!(
                    stdout,
                    SetAttribute(Attribute::Dim),
                    Print(gutter),
                    SetAttribute(Attribute::Reset)
                )?;
                self.apply_editor_theme(stdout)?;
                self.draw_row_content(stdout, file_row, buffer_row, text_cols)?;
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

        self.apply_editor_theme(stdout)?;
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
        loop {
            match event::read()? {
                Event::Key(key) => {
                    let (cols, rows) = Terminal::size().unwrap_or((80, 24));
                    let (_, text_rows) = self.text_viewport(cols, rows);
                    self.handle_key_event(key, text_rows);
                    break;
                }
                Event::Mouse(mouse) => {
                    if matches!(mouse.kind, MouseEventKind::Moved | MouseEventKind::Up(_)) {
                        continue;
                    }
                    let (cols, rows) = Terminal::size().unwrap_or((80, 24));
                    self.handle_mouse_event(mouse, cols, rows);
                    break;
                }
                Event::Paste(text) => {
                    let (cols, rows) = Terminal::size().unwrap_or((80, 24));
                    let (_, text_rows) = self.text_viewport(cols, rows);
                    self.handle_paste_event(&text);
                    self.snap_viewport_to_cursor(text_rows);
                    break;
                }
                Event::Resize(_, _) => {
                    break;
                }
                _ => continue,
            }
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

        // Any intervening key resets the quit confirmation state and clears the warning message
        editor.handle_key_event(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE), 24);
        assert!(!editor.quit_confirm_pending);
        assert_eq!(editor.status_message, None);

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
        let mut editor = Editor::new(Buffer::with_filename("Makefile"));
        editor.handle_key_event(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE), 24);
        editor.handle_key_event(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE), 24);

        // Makefile uses hard tabs (expand_tabs = false): stores raw '\t' + 'x', renders 4 spaces + 'x'
        assert_eq!(editor.buffer.contents(), "\tx");
        assert_eq!(editor.buffer.get_row(0).unwrap().render, "    x");
        assert_eq!(editor.cx, 2);
        assert_eq!(editor.visual_cx(), 5);
        // Gutter width for 1 line is 3 (" 1 "), so screen_cx is 3 + 5 = 8
        assert_eq!(editor.cursor_screen_position(), (8, 0));

        // YAML file defaults to 2-space soft tabs (expand_tabs = true, tab_width = 2)
        let mut yaml_editor = Editor::new(Buffer::with_filename("config.yaml"));
        yaml_editor.handle_key_event(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE), 24);
        yaml_editor.handle_key_event(KeyEvent::new(KeyCode::Char('k'), KeyModifiers::NONE), 24);
        assert_eq!(yaml_editor.buffer.contents(), "  k");
        assert_eq!(yaml_editor.cx, 3);
    }

    #[test]
    fn find_prompt_enter_accepts_match_and_non_matching_char_resets_cursor() {
        let mut editor = Editor::new(Buffer::from_lines(&[
            "first target here",
            "second target there",
        ]));
        editor.cx = 0;
        editor.cy = 0;

        editor.handle_key_event(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::CONTROL), 24);
        for ch in "target".chars() {
            editor.handle_key_event(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE), 24);
        }
        assert_eq!((editor.cx, editor.cy), (6, 0));

        // Typing a non-matching character resets cursor back to saved position (0, 0)
        editor.handle_key_event(KeyEvent::new(KeyCode::Char('z'), KeyModifiers::NONE), 24);
        assert_eq!((editor.cx, editor.cy), (0, 0));

        // Backspace restores the match for "target" at (6, 0)
        editor.handle_key_event(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE), 24);
        assert_eq!((editor.cx, editor.cy), (6, 0));

        // Pressing Enter exits Search prompt while keeping cursor at the matched position
        editor.handle_key_event(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), 24);
        assert_eq!(editor.prompt, PromptState::None);
        assert_eq!((editor.cx, editor.cy), (6, 0));
    }

    #[test]
    fn key_release_events_are_ignored() {
        let mut editor = Editor::new(Buffer::new());
        let release_event = KeyEvent::new_with_kind(
            KeyCode::Char('a'),
            KeyModifiers::NONE,
            KeyEventKind::Release,
        );
        editor.handle_key_event(release_event, 24);
        assert!(editor.buffer.is_empty());
    }

    #[test]
    fn vertical_navigation_aligns_visual_columns_across_tabs_and_spaces() {
        let buf = sample_buffer(&["\t\tfoo", "        bar"]);
        let mut editor = Editor::new(buf);
        // Move to 'f' on line 0: cx = 2, visual rx = 8
        editor.move_cursor(KeyCode::Right, 24);
        editor.move_cursor(KeyCode::Right, 24);
        assert_eq!((editor.cx, editor.visual_cx()), (2, 8));

        // Moving down to space-indented line lands on 'b' at cx = 8 (visual rx = 8)
        editor.move_cursor(KeyCode::Down, 24);
        assert_eq!((editor.cx, editor.visual_cx()), (8, 8));

        // Moving back up returns to 'f' at cx = 2 (visual rx = 8)
        editor.move_cursor(KeyCode::Up, 24);
        assert_eq!((editor.cx, editor.visual_cx()), (2, 8));
    }

    #[test]
    fn save_as_prompt_positions_cursor_on_status_bar() {
        let mut editor = Editor::new(Buffer::new());
        editor.trigger_save();
        editor.handle_key_event(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE), 24);
        // " Save as: a" has display width 11, placed on bottom row (24 - 1 = 23)
        assert_eq!(editor.active_cursor_screen_position(80, 24), (11, 23));
    }

    #[test]
    fn status_bar_preserves_active_prompt_tail_on_narrow_terminals() {
        let mut editor = Editor::new(Buffer::from_lines(&["hello"]));
        editor.trigger_find();
        for ch in "very_long_search_query_tail".chars() {
            editor.handle_key_event(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE), 24);
        }
        let bar = editor.format_status_bar(40);
        assert_eq!(bar.chars().count(), 40);
        assert!(bar.ends_with("very_long_search_query_tail"));
    }

    #[test]
    fn find_prompt_defaults_to_case_insensitive_and_alt_c_toggles_case_sensitivity() {
        let mut editor = Editor::new(Buffer::from_lines(&["first TARGET here", "second target"]));
        editor.handle_key_event(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::CONTROL), 24);
        for ch in "target".chars() {
            editor.handle_key_event(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE), 24);
        }
        // Case-insensitive by default: matches "TARGET" on line 0 at cx = 6
        assert_eq!((editor.cx, editor.cy), (6, 0));

        // Pressing Alt+C enables case-sensitive mode, jumping to lowercase "target" on line 1 at cx = 7
        editor.handle_key_event(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::ALT), 24);
        assert_eq!((editor.cx, editor.cy), (7, 1));

        // Pressing Alt+C again returns to case-insensitive mode, matching line 0 at cx = 6
        editor.handle_key_event(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::ALT), 24);
        assert_eq!((editor.cx, editor.cy), (6, 0));
    }

    #[test]
    fn ctrl_d_duplicates_line_and_ctrl_shift_d_deletes_line() {
        let mut editor = Editor::new(Buffer::from_lines(&["line one", "line two", "line three"]));
        editor.move_cursor(KeyCode::Down, 24);
        editor.move_cursor(KeyCode::End, 24);
        assert_eq!((editor.cx, editor.cy), (8, 1));

        // Ctrl+D duplicates "line two" onto row 2 and moves cursor to row 2
        editor.handle_key_event(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL), 24);
        assert_eq!(
            editor.buffer.contents(),
            "line one\nline two\nline two\nline three"
        );
        assert_eq!((editor.cx, editor.cy), (8, 2));

        // Ctrl+Shift+D deletes the duplicated "line two"
        editor.handle_key_event(
            KeyEvent::new(
                KeyCode::Char('D'),
                KeyModifiers::CONTROL | KeyModifiers::SHIFT,
            ),
            24,
        );
        assert_eq!(editor.buffer.contents(), "line one\nline two\nline three");
        assert_eq!((editor.cx, editor.cy), (8, 2));

        // Ctrl+Z restores the deleted line
        editor.handle_key_event(KeyEvent::new(KeyCode::Char('z'), KeyModifiers::CONTROL), 24);
        assert_eq!(
            editor.buffer.contents(),
            "line one\nline two\nline two\nline three"
        );
        assert_eq!((editor.cx, editor.cy), (8, 2));
    }

    #[test]
    fn shift_enter_and_ctrl_shift_enter_insert_indented_lines_below_and_above() {
        let mut editor = Editor::new(Buffer::from_lines(&["    fn main() {", "    }"]));
        // Place cursor in the middle of line 0 ("    fn| main() {")
        editor.cx = 6;
        editor.cy = 0;

        editor.handle_key_event(KeyEvent::new(KeyCode::Enter, KeyModifiers::SHIFT), 24);
        assert_eq!(editor.buffer.len(), 3);
        assert_eq!(editor.buffer.get_row(0).unwrap().text(), "    fn main() {");
        assert_eq!(editor.buffer.get_row(1).unwrap().text(), "    ");
        assert_eq!(editor.buffer.get_row(2).unwrap().text(), "    }");
        assert_eq!((editor.cx, editor.cy), (4, 1));

        // Move to closing brace line ("    }") and insert line above with Ctrl+Shift+Enter
        editor.cy = 2;
        editor.cx = 5;
        editor.handle_key_event(
            KeyEvent::new(KeyCode::Enter, KeyModifiers::CONTROL | KeyModifiers::SHIFT),
            24,
        );
        assert_eq!(editor.buffer.len(), 4);
        assert_eq!(editor.buffer.get_row(2).unwrap().text(), "    ");
        assert_eq!(editor.buffer.get_row(3).unwrap().text(), "    }");
        assert_eq!((editor.cx, editor.cy), (4, 2));
    }

    #[test]
    fn auto_indent_unindent_smart_home_word_ops_and_goto_line_work() {
        let mut editor = Editor::new(Buffer::from_lines(&[
            "    let alpha_beta = 42;",
            "second line",
            "third line",
        ]));
        editor.cx = 10;
        editor.cy = 0;

        // Smart Home: first press jumps to first non-whitespace char (cx = 4), second press jumps to 0
        editor.handle_key_event(KeyEvent::new(KeyCode::Home, KeyModifiers::NONE), 24);
        assert_eq!(editor.cx, 4);
        editor.handle_key_event(KeyEvent::new(KeyCode::Home, KeyModifiers::NONE), 24);
        assert_eq!(editor.cx, 0);
        editor.handle_key_event(KeyEvent::new(KeyCode::Home, KeyModifiers::NONE), 24);
        assert_eq!(editor.cx, 4);

        // Word Right jumps past "let" to cx = 7, then past " alpha_beta" to cx = 18
        editor.handle_key_event(KeyEvent::new(KeyCode::Right, KeyModifiers::CONTROL), 24);
        assert_eq!(editor.cx, 7);
        editor.handle_key_event(KeyEvent::new(KeyCode::Right, KeyModifiers::CONTROL), 24);
        assert_eq!(editor.cx, 18);

        // Delete Word Left deletes "alpha_beta" back to cx = 8
        editor.handle_key_event(KeyEvent::new(KeyCode::Backspace, KeyModifiers::CONTROL), 24);
        assert_eq!(editor.cx, 8);
        assert_eq!(editor.buffer.get_row(0).unwrap().text(), "    let  = 42;");

        // Shift+Tab (BackTab) unindents line 0 by 4 spaces
        editor.handle_key_event(KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT), 24);
        assert_eq!(editor.buffer.get_row(0).unwrap().text(), "let  = 42;");
        assert_eq!(editor.cx, 4);

        // Ctrl+G opens GotoLine prompt; typing "3" + Enter jumps to line 3 (cy = 2)
        editor.handle_key_event(KeyEvent::new(KeyCode::Char('g'), KeyModifiers::CONTROL), 24);
        editor.handle_key_event(KeyEvent::new(KeyCode::Char('3'), KeyModifiers::NONE), 24);
        editor.handle_key_event(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), 24);
        assert_eq!(editor.cy, 2);
    }

    #[test]
    fn auto_pairs_inserts_skips_over_backspaces_and_respects_quote_context() {
        let mut editor = Editor::new(Buffer::new());

        // Typing '(' inserts "()" and places cursor at cx = 1
        editor.handle_key_event(KeyEvent::new(KeyCode::Char('('), KeyModifiers::NONE), 24);
        assert_eq!(editor.buffer.contents(), "()");
        assert_eq!(editor.cx, 1);

        // Typing ')' skips over existing ')' without inserting a duplicate
        editor.handle_key_event(KeyEvent::new(KeyCode::Char(')'), KeyModifiers::NONE), 24);
        assert_eq!(editor.buffer.contents(), "()");
        assert_eq!(editor.cx, 2);

        // Move back inside "()" and press Backspace -> deletes both '(' and ')'
        editor.handle_key_event(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE), 24);
        assert_eq!(editor.cx, 1);
        editor.handle_key_event(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE), 24);
        assert!(editor.buffer.is_empty());
        assert_eq!(editor.cx, 0);

        // Quote context awareness: typing "don't" does not auto-close the apostrophe after 'n'
        for ch in "don't".chars() {
            editor.handle_key_event(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE), 24);
        }
        assert_eq!(editor.buffer.contents(), "don't");
    }

    #[test]
    fn action_chaining_executes_save_and_quit_or_aborts_when_prompt_opens() {
        let cfg = Config::from_toml_str(
            r#"
[keys]
"ctrl+shift+q" = ["save", "quit"]
"#,
        )
        .unwrap();

        // Unnamed dirty buffer: Save opens SaveAs prompt and aborts the chained Quit
        let mut unnamed_editor = Editor::with_config(Buffer::from_lines(&["unsaved"]), cfg.clone());
        unnamed_editor.buffer.dirty = true;
        unnamed_editor.handle_key_event(
            KeyEvent::new(
                KeyCode::Char('Q'),
                KeyModifiers::CONTROL | KeyModifiers::SHIFT,
            ),
            24,
        );
        assert!(!unnamed_editor.should_quit);
        assert_eq!(
            unnamed_editor.prompt,
            PromptState::SaveAs {
                input: String::new()
            }
        );

        // Named dirty buffer: Save succeeds and chained Quit immediately exits
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("omo_chain_save_quit_{nanos}.txt"));
        let path_str = path.to_string_lossy().to_string();

        let mut named_buf = Buffer::from_lines(&["saved content"]);
        named_buf.filename = Some(path_str.clone());
        named_buf.dirty = true;
        let mut named_editor = Editor::with_config(named_buf, cfg);
        named_editor.handle_key_event(
            KeyEvent::new(
                KeyCode::Char('Q'),
                KeyModifiers::CONTROL | KeyModifiers::SHIFT,
            ),
            24,
        );
        assert!(named_editor.should_quit);
        assert!(!named_editor.buffer.dirty);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "saved content\n");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn alt_up_and_alt_down_move_current_line_and_permission_denied_prompts_sudo() {
        let mut editor = Editor::new(Buffer::from_lines(&["line 1", "line 2", "line 3"]));
        editor.handle_key_event(KeyEvent::new(KeyCode::Down, KeyModifiers::ALT), 24);
        assert_eq!(editor.buffer.contents(), "line 2\nline 1\nline 3");
        assert_eq!(editor.cy, 1);

        editor.handle_key_event(KeyEvent::new(KeyCode::Up, KeyModifiers::ALT), 24);
        assert_eq!(editor.buffer.contents(), "line 1\nline 2\nline 3");
        assert_eq!(editor.cy, 0);

        // Simulate PermissionDenied on save -> transitions to ConfirmSudoSave prompt
        let perm_err = std::io::Error::new(std::io::ErrorKind::PermissionDenied, "denied");
        assert!(!editor.handle_save_result("/etc/hosts", Err(perm_err.into())));
        assert_eq!(
            editor.prompt,
            PromptState::ConfirmSudoSave {
                path: "/etc/hosts".to_string()
            }
        );

        // Pressing 'n' cancels the sudo save prompt cleanly
        editor.handle_key_event(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE), 24);
        assert_eq!(editor.prompt, PromptState::None);
    }

    #[test]
    fn shift_navigation_and_ctrl_a_create_selection_and_typing_replaces_atomically() {
        let mut editor = Editor::new(Buffer::from_lines(&["hello world", "second line"]));

        // Shift+Ctrl+Right selects "hello"
        editor.handle_key_event(
            KeyEvent::new(KeyCode::Right, KeyModifiers::CONTROL | KeyModifiers::SHIFT),
            24,
        );
        assert_eq!(
            editor.selection,
            Some(Selection::new(Position::new(0, 0), Position::new(0, 5)))
        );

        // Unshifted Right collapses selection
        editor.handle_key_event(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE), 24);
        assert_eq!(editor.selection, None);

        // Ctrl+A selects the entire buffer
        editor.handle_key_event(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::CONTROL), 24);
        assert_eq!(
            editor.selection,
            Some(Selection::new(Position::new(0, 0), Position::new(1, 11)))
        );

        // Typing 'X' replaces the entire selection in a single undo step
        editor.handle_key_event(KeyEvent::new(KeyCode::Char('X'), KeyModifiers::NONE), 24);
        assert_eq!(editor.buffer.contents(), "X");
        assert_eq!(editor.selection, None);

        // Single Ctrl+Z restores the full multi-line buffer
        editor.handle_key_event(KeyEvent::new(KeyCode::Char('z'), KeyModifiers::CONTROL), 24);
        assert_eq!(editor.buffer.contents(), "hello world\nsecond line");
    }

    #[test]
    fn copy_cut_paste_and_block_tab_indentation_work_with_selections() {
        let mut editor = Editor::new(Buffer::from_lines(&["first", "second", "third"]));

        // With no selection, Ctrl+C copies the current line ("first\n")
        editor.handle_key_event(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL), 24);
        assert_eq!(editor.clipboard.get_text(), "first\n");

        // Select lines 0 and 1 with Shift+Down and Shift+End, then press Tab to block-indent
        editor.handle_key_event(KeyEvent::new(KeyCode::Down, KeyModifiers::SHIFT), 24);
        editor.handle_key_event(KeyEvent::new(KeyCode::End, KeyModifiers::SHIFT), 24);
        editor.handle_key_event(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE), 24);
        assert_eq!(editor.buffer.contents(), "    first\n    second\nthird");

        // Shift+Tab block-unindents both selected lines
        editor.handle_key_event(KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT), 24);
        assert_eq!(editor.buffer.contents(), "first\nsecond\nthird");

        // Cut the selection ("first\nsecond") with Ctrl+X and paste at the end with Ctrl+V
        editor.handle_key_event(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::CONTROL), 24);
        assert_eq!(editor.buffer.contents(), "\nthird");
        editor.handle_key_event(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE), 24);
        editor.handle_key_event(KeyEvent::new(KeyCode::End, KeyModifiers::NONE), 24);
        editor.handle_key_event(KeyEvent::new(KeyCode::Char('v'), KeyModifiers::CONTROL), 24);
        assert_eq!(editor.buffer.contents(), "\nthirdfirst\nsecond");
    }

    #[test]
    fn mouse_click_drag_gutter_clamping_and_multi_click_selection() {
        let mut editor = Editor::new(Buffer::from_lines(&["hello world", "second line"]));
        // Gutter width for 2 lines is 3 (" 1 ")
        assert_eq!(editor.gutter_width(), 3);

        // Clicking inside the gutter (column 1, row 1) clamps to cx = 0 on row 1
        editor.handle_mouse_event(
            MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: 1,
                row: 1,
                modifiers: KeyModifiers::NONE,
            },
            80,
            24,
        );
        assert_eq!((editor.cx, editor.cy), (0, 1));

        // Single click at screen column 3 + 2 = 5 on row 0 ("he|llo world"), then double-click selects "hello", triple-click selects entire line
        let click_row0 = MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 5,
            row: 0,
            modifiers: KeyModifiers::NONE,
        };
        editor.handle_mouse_event(click_row0, 80, 24);
        assert_eq!((editor.cx, editor.cy), (2, 0));
        assert_eq!(editor.selection, None);

        editor.handle_mouse_event(click_row0, 80, 24);
        assert_eq!(
            editor.selection,
            Some(Selection::new(Position::new(0, 0), Position::new(0, 5)))
        );

        editor.handle_mouse_event(click_row0, 80, 24);
        assert_eq!(
            editor.selection,
            Some(Selection::new(Position::new(0, 0), Position::new(0, 11)))
        );

        // Dragging from (0, 2) into the gutter on row 1 clamps end.col to 0
        editor.last_click = None;
        editor.handle_mouse_event(click_row0, 80, 24);
        editor.handle_mouse_event(
            MouseEvent {
                kind: MouseEventKind::Drag(MouseButton::Left),
                column: 0,
                row: 1,
                modifiers: KeyModifiers::NONE,
            },
            80,
            24,
        );
        assert_eq!(
            editor.selection,
            Some(Selection::new(Position::new(0, 2), Position::new(1, 0)))
        );
    }

    #[test]
    fn mouse_wheel_scrolls_viewport_without_moving_cursor_or_clearing_selection_and_snaps_back() {
        let mut lines = Vec::new();
        for i in 0..20 {
            lines.push(format!("line {i}"));
        }
        let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
        let mut editor = Editor::new(Buffer::from_lines(&refs));

        // Select "line" on row 0
        editor.handle_key_event(
            KeyEvent::new(KeyCode::Right, KeyModifiers::CONTROL | KeyModifiers::SHIFT),
            5,
        );
        let original_selection = editor.selection;
        assert_eq!((editor.cx, editor.cy, editor.row_offset), (4, 0, 0));
        assert!(editor.is_cursor_visible(5));

        // Scroll down with mouse wheel twice (6 rows total): row_offset becomes 6, cursor stays at (4, 0) and selection is preserved
        for _ in 0..2 {
            editor.handle_mouse_event(
                MouseEvent {
                    kind: MouseEventKind::ScrollDown,
                    column: 5,
                    row: 2,
                    modifiers: KeyModifiers::NONE,
                },
                80,
                6,
            );
        }
        assert_eq!(editor.row_offset, 6);
        assert_eq!((editor.cx, editor.cy), (4, 0));
        assert_eq!(editor.selection, original_selection);
        assert!(!editor.is_cursor_visible(5));

        // Pressing Right arrow collapses selection, moves cursor to (5, 0), and snaps row_offset back to 0
        editor.handle_key_event(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE), 5);
        assert_eq!((editor.cx, editor.cy, editor.row_offset), (5, 0, 0));
        assert!(editor.is_cursor_visible(5));
    }

    #[test]
    fn bracketed_paste_normalizes_crlf_and_inserts_without_auto_pairs_in_single_undo() {
        let mut editor = Editor::new(Buffer::from_lines(&["    start"]));
        editor.move_cursor(KeyCode::End, 24);

        editor.handle_paste_event("(\r\nnext_line");
        assert_eq!(editor.buffer.contents(), "    start(\nnext_line");

        // Single Ctrl+Z reverts the entire multi-line paste
        editor.handle_key_event(KeyEvent::new(KeyCode::Char('z'), KeyModifiers::CONTROL), 24);
        assert_eq!(editor.buffer.contents(), "    start");
    }
}
