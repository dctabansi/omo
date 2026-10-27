use anyhow::Result;
use std::fmt;
use std::fs;

use crate::config::DEFAULT_TAB_WIDTH;

const DEFAULT_GAP_SIZE: usize = 16;

/// A character-indexed gap buffer with a movable gap `[gap_start..gap_end]` that provides
/// O(1) amortized character insertions and deletions at the cursor column.
#[derive(Clone, Debug)]
pub struct GapBuffer {
    buf: Vec<char>,
    gap_start: usize,
    gap_end: usize,
}

impl Default for GapBuffer {
    fn default() -> Self {
        Self::new()
    }
}

impl From<&str> for GapBuffer {
    fn from(slice: &str) -> Self {
        let mut buf: Vec<char> = slice.chars().collect();
        let len = buf.len();
        buf.resize(len + DEFAULT_GAP_SIZE, '\0');
        Self {
            buf,
            gap_start: len,
            gap_end: len + DEFAULT_GAP_SIZE,
        }
    }
}

impl PartialEq for GapBuffer {
    fn eq(&self, other: &Self) -> bool {
        self.len() == other.len() && self.chars().eq(other.chars())
    }
}

impl Eq for GapBuffer {}

impl fmt::Display for GapBuffer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for c in self.chars() {
            f.write_char(c)?;
        }
        Ok(())
    }
}

use std::fmt::Write as _;

impl GapBuffer {
    /// Creates an empty `GapBuffer` pre-allocated with the default gap capacity.
    pub fn new() -> Self {
        Self {
            buf: vec!['\0'; DEFAULT_GAP_SIZE],
            gap_start: 0,
            gap_end: DEFAULT_GAP_SIZE,
        }
    }

    /// Returns the number of logical characters in the buffer (excluding the gap).
    pub fn len(&self) -> usize {
        self.buf.len() - self.gap_len()
    }

    /// Returns true if the buffer contains no logical characters.
    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Returns the current capacity of the movable gap.
    pub fn gap_len(&self) -> usize {
        self.gap_end - self.gap_start
    }

    /// Returns the logical character index where the gap currently starts.
    #[allow(dead_code)]
    pub fn gap_position(&self) -> usize {
        self.gap_start
    }

    /// Moves the gap to the logical character index `index` in O(|index - gap_start|) time.
    /// When `index == gap_start`, this is a zero-copy O(1) no-op.
    pub fn move_gap(&mut self, index: usize) {
        let target = index.min(self.len());
        if target < self.gap_start {
            let delta = self.gap_start - target;
            self.buf
                .copy_within(target..self.gap_start, self.gap_end - delta);
            self.gap_start = target;
            self.gap_end -= delta;
        } else if target > self.gap_start {
            let delta = target - self.gap_start;
            self.buf
                .copy_within(self.gap_end..self.gap_end + delta, self.gap_start);
            self.gap_start += delta;
            self.gap_end += delta;
        }
    }

    /// Ensures the gap has at least `min_gap` free slots, growing the backing storage if needed.
    fn ensure_gap(&mut self, min_gap: usize) {
        if self.gap_len() >= min_gap {
            return;
        }
        let extra = min_gap.max(DEFAULT_GAP_SIZE).max(self.len() / 2);
        let old_len = self.buf.len();
        let tail_len = old_len - self.gap_end;
        self.buf.resize(old_len + extra, '\0');
        if tail_len > 0 {
            self.buf
                .copy_within(self.gap_end..old_len, self.gap_end + extra);
        }
        self.gap_end += extra;
    }

    /// Inserts a character at logical character index `at` in O(1) amortized time for localized edits.
    pub fn insert(&mut self, at: usize, c: char) {
        let clamped = at.min(self.len());
        self.move_gap(clamped);
        self.ensure_gap(1);
        self.buf[self.gap_start] = c;
        self.gap_start += 1;
    }

    /// Removes and returns the character at logical character index `at` in O(1) time for localized edits.
    pub fn remove(&mut self, at: usize) -> Option<char> {
        if at >= self.len() {
            return None;
        }
        if at + 1 == self.gap_start {
            self.gap_start -= 1;
            return Some(self.buf[self.gap_start]);
        }
        self.move_gap(at);
        let removed = self.buf[self.gap_end];
        self.gap_end += 1;
        Some(removed)
    }

    /// Splits the buffer at logical character index `at`, retaining `[..at]` in `self`
    /// and returning `[at..]` as a new `GapBuffer`.
    pub fn split_off(&mut self, at: usize) -> Self {
        let clamped = at.min(self.len());
        self.move_gap(clamped);
        let right_len = self.buf.len() - self.gap_end;
        let mut right_buf = vec!['\0'; DEFAULT_GAP_SIZE + right_len];
        right_buf[DEFAULT_GAP_SIZE..].copy_from_slice(&self.buf[self.gap_end..]);
        self.gap_end = self.buf.len();
        Self {
            buf: right_buf,
            gap_start: 0,
            gap_end: DEFAULT_GAP_SIZE,
        }
    }

    /// Appends all characters from `other` onto the end of `self`.
    pub fn append(&mut self, other: &Self) {
        let end = self.len();
        self.move_gap(end);
        self.ensure_gap(other.len());
        for c in other.chars() {
            self.buf[self.gap_start] = c;
            self.gap_start += 1;
        }
    }

    /// Iterates over the logical characters in order, skipping the internal gap.
    pub fn chars(&self) -> impl Iterator<Item = char> + '_ {
        self.buf[..self.gap_start]
            .iter()
            .chain(self.buf[self.gap_end..].iter())
            .copied()
    }
}

/// Represents a single line of text in the editor backed by a line-level [`GapBuffer`]
/// and a visual tab-expanded `render` string.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub buffer: GapBuffer,
    /// Visual representation of the row with `\t` expanded to `tab_width` spaces.
    pub render: String,
    /// Cached logical character count in `buffer` (not byte count).
    pub len: usize,
    /// Cached visual character count in `render`.
    pub render_len: usize,
    /// Number of visual columns per tab stop.
    pub tab_width: usize,
}

impl From<&str> for Row {
    fn from(slice: &str) -> Self {
        Self::with_tab_width(slice, DEFAULT_TAB_WIDTH)
    }
}

impl Row {
    /// Constructs a `Row` from a string slice with an explicit tab stop width.
    pub fn with_tab_width(slice: &str, tab_width: usize) -> Self {
        let buffer = GapBuffer::from(slice);
        let len = buffer.len();
        let mut row = Self {
            buffer,
            render: String::new(),
            len,
            render_len: 0,
            tab_width: tab_width.max(1),
        };
        row.update_render();
        row
    }

    /// Returns the raw logical text of the row (preserving unexpanded `\t` characters).
    pub fn text(&self) -> String {
        self.buffer.to_string()
    }

    /// Updates the row's tab stop width and refreshes `self.render`.
    pub fn set_tab_width(&mut self, tab_width: usize) {
        let normalized = tab_width.max(1);
        if self.tab_width != normalized {
            self.tab_width = normalized;
            self.update_render();
        }
    }

    /// Recomputes `self.render`, `self.render_len`, and `self.len` from `self.buffer`,
    /// expanding `\t` characters to `self.tab_width` tab stops.
    pub fn update_render(&mut self) {
        let tab_w = self.tab_width.max(1);
        self.len = self.buffer.len();
        let mut rendered = String::with_capacity(self.len);
        let mut rx = 0;
        for c in self.buffer.chars() {
            if c == '\t' {
                let spaces = tab_w - (rx % tab_w);
                for _ in 0..spaces {
                    rendered.push(' ');
                }
                rx += spaces;
            } else {
                rendered.push(c);
                rx += 1;
            }
        }
        self.render = rendered;
        self.render_len = rx;
    }

    /// Converts a logical character column `cx` into a visual render column `rx`,
    /// accounting for expanded tab stops.
    pub fn cx_to_rx(&self, cx: usize) -> usize {
        let tab_w = self.tab_width.max(1);
        let mut rx = 0;
        for c in self.buffer.chars().take(cx) {
            if c == '\t' {
                rx += tab_w - (rx % tab_w);
            } else {
                rx += 1;
            }
        }
        if cx > self.len {
            rx += cx - self.len;
        }
        rx
    }

    /// Converts a visual render column `target_rx` into the corresponding logical character column `cx`.
    #[allow(dead_code)]
    pub fn rx_to_cx(&self, target_rx: usize) -> usize {
        let tab_w = self.tab_width.max(1);
        let mut rx = 0;
        for (cx, c) in self.buffer.chars().enumerate() {
            let next_rx = if c == '\t' {
                rx + (tab_w - (rx % tab_w))
            } else {
                rx + 1
            };
            if next_rx > target_rx {
                return cx;
            }
            rx = next_rx;
        }
        self.len
    }

    /// Inserts a character at the given character index `at` using the underlying `GapBuffer`.
    pub fn insert_char(&mut self, at: usize, c: char) {
        self.buffer.insert(at, c);
        self.update_render();
    }

    /// Removes and returns the character at the given character index `at`.
    pub fn delete_char(&mut self, at: usize) -> Option<char> {
        let removed = self.buffer.remove(at)?;
        self.update_render();
        Some(removed)
    }

    /// Splits the row at character index `at`, retaining `[..at]` and returning `[at..]`.
    pub fn split(&mut self, at: usize) -> Row {
        let right_buffer = self.buffer.split_off(at);
        self.update_render();
        let right_len = right_buffer.len();
        let mut right = Row {
            buffer: right_buffer,
            render: String::new(),
            len: right_len,
            render_len: 0,
            tab_width: self.tab_width,
        };
        right.update_render();
        right
    }

    /// Appends the contents of `other` onto the end of this row.
    pub fn append(&mut self, other: &Row) {
        self.buffer.append(&other.buffer);
        self.update_render();
    }

    /// Returns a UTF-8 character-safe horizontal slice of the tab-expanded `render` string
    /// for viewport rendering.
    pub fn slice_chars(&self, col_offset: usize, max_cols: usize) -> String {
        self.render
            .chars()
            .skip(col_offset)
            .take(max_cols)
            .collect()
    }

    /// Returns all character start indices where `query` matches within this row.
    pub fn find_matches(&self, query: &str) -> Vec<usize> {
        if query.is_empty() {
            return Vec::new();
        }
        let text = self.text();
        text.match_indices(query)
            .map(|(byte_idx, _)| text[..byte_idx].chars().count())
            .collect()
    }
}

const MAX_UNDO_HISTORY: usize = 200;

/// Captures a reversible snapshot of buffer rows, cursor position, and dirty state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BufferSnapshot {
    pub rows: Vec<Row>,
    pub cx: usize,
    pub cy: usize,
    pub dirty: bool,
}

/// Holds the entirety of a text buffer.
pub struct Buffer {
    rows: Vec<Row>,
    pub filename: Option<String>,
    pub dirty: bool,
    pub tab_width: usize,
    undo_stack: Vec<BufferSnapshot>,
    redo_stack: Vec<BufferSnapshot>,
}

impl Default for Buffer {
    fn default() -> Self {
        Self {
            rows: Vec::new(),
            filename: None,
            dirty: false,
            tab_width: DEFAULT_TAB_WIDTH,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
        }
    }
}

impl Buffer {
    /// Creates a new empty buffer.
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates a new empty buffer associated with a target filename.
    pub fn with_filename(filename: impl Into<String>) -> Self {
        Self {
            rows: Vec::new(),
            filename: Some(filename.into()),
            dirty: false,
            tab_width: DEFAULT_TAB_WIDTH,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
        }
    }

    /// Creates a buffer populated from a slice of string lines.
    #[allow(dead_code)]
    pub fn from_lines(lines: &[&str]) -> Self {
        Self {
            rows: lines.iter().copied().map(Row::from).collect(),
            filename: None,
            dirty: false,
            tab_width: DEFAULT_TAB_WIDTH,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
        }
    }

    /// Loads a buffer from a file path. If the file does not exist yet, returns an empty buffer
    /// bound to `filename`.
    pub fn open(filename: &str) -> Result<Self> {
        match fs::read_to_string(filename) {
            Ok(contents) => {
                let mut rows = Vec::new();
                for line in contents.lines() {
                    rows.push(Row::from(line));
                }
                Ok(Self {
                    rows,
                    filename: Some(filename.to_string()),
                    dirty: false,
                    tab_width: DEFAULT_TAB_WIDTH,
                    undo_stack: Vec::new(),
                    redo_stack: Vec::new(),
                })
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                Ok(Self::with_filename(filename))
            }
            Err(err) => Err(err.into()),
        }
    }

    /// Updates the tab stop width across the buffer and re-renders all existing rows.
    pub fn set_tab_width(&mut self, tab_width: usize) {
        let normalized = tab_width.max(1);
        self.tab_width = normalized;
        for row in &mut self.rows {
            row.set_tab_width(normalized);
        }
    }

    /// Records the current buffer state onto `undo_stack` and clears `redo_stack`.
    pub fn record_snapshot(&mut self, cx: usize, cy: usize) {
        if self.undo_stack.len() >= MAX_UNDO_HISTORY {
            self.undo_stack.remove(0);
        }
        self.undo_stack.push(BufferSnapshot {
            rows: self.rows.clone(),
            cx,
            cy,
            dirty: self.dirty,
        });
        self.redo_stack.clear();
    }

    /// Restores the most recent snapshot from `undo_stack`, pushing the current state to
    /// `redo_stack`, and returns the restored `(cx, cy)` cursor coordinates.
    pub fn undo(&mut self, current_cx: usize, current_cy: usize) -> Option<(usize, usize)> {
        let snapshot = self.undo_stack.pop()?;
        self.redo_stack.push(BufferSnapshot {
            rows: self.rows.clone(),
            cx: current_cx,
            cy: current_cy,
            dirty: self.dirty,
        });
        self.rows = snapshot.rows;
        self.dirty = snapshot.dirty;
        Some((snapshot.cx, snapshot.cy))
    }

    /// Reapplies the most recent snapshot from `redo_stack`, pushing the current state to
    /// `undo_stack`, and returns the restored `(cx, cy)` cursor coordinates.
    pub fn redo(&mut self, current_cx: usize, current_cy: usize) -> Option<(usize, usize)> {
        let snapshot = self.redo_stack.pop()?;
        self.undo_stack.push(BufferSnapshot {
            rows: self.rows.clone(),
            cx: current_cx,
            cy: current_cy,
            dirty: self.dirty,
        });
        self.rows = snapshot.rows;
        self.dirty = snapshot.dirty;
        Some((snapshot.cx, snapshot.cy))
    }

    /// Searches for `query` starting from `(start_cx, start_cy)` in `forward` or backward order,
    /// wrapping around the buffer and returning `(match_cx, match_cy)` in character coordinates.
    pub fn find(
        &self,
        query: &str,
        start_cx: usize,
        start_cy: usize,
        forward: bool,
    ) -> Option<(usize, usize)> {
        if query.is_empty() || self.rows.is_empty() {
            return None;
        }

        let num_rows = self.rows.len();
        let clamped_cy = start_cy.min(num_rows - 1);

        if forward {
            if let Some(cx) = self.rows[clamped_cy]
                .find_matches(query)
                .into_iter()
                .find(|&m| m >= start_cx)
            {
                return Some((cx, clamped_cy));
            }

            for offset in 1..num_rows {
                let row_idx = (clamped_cy + offset) % num_rows;
                if let Some(cx) = self.rows[row_idx].find_matches(query).into_iter().next() {
                    return Some((cx, row_idx));
                }
            }

            if let Some(cx) = self.rows[clamped_cy]
                .find_matches(query)
                .into_iter()
                .find(|&m| m < start_cx)
            {
                return Some((cx, clamped_cy));
            }
        } else {
            if let Some(cx) = self.rows[clamped_cy]
                .find_matches(query)
                .into_iter()
                .rev()
                .find(|&m| m < start_cx)
            {
                return Some((cx, clamped_cy));
            }

            for offset in 1..num_rows {
                let row_idx = (clamped_cy + num_rows - offset) % num_rows;
                if let Some(cx) = self.rows[row_idx]
                    .find_matches(query)
                    .into_iter()
                    .next_back()
                {
                    return Some((cx, row_idx));
                }
            }

            if let Some(cx) = self.rows[clamped_cy]
                .find_matches(query)
                .into_iter()
                .rev()
                .find(|&m| m >= start_cx)
            {
                return Some((cx, clamped_cy));
            }
        }

        None
    }

    /// Inserts a character at buffer coordinates `(cx, cy)`.
    pub fn insert_char(&mut self, cx: usize, cy: usize, c: char) {
        self.record_snapshot(cx, cy);
        while self.rows.len() <= cy {
            self.rows.push(Row::with_tab_width("", self.tab_width));
        }
        self.rows[cy].insert_char(cx, c);
        self.dirty = true;
    }

    /// Splits the row at `(cx, cy)` or inserts a new empty line when the buffer is empty.
    pub fn insert_newline(&mut self, cx: usize, cy: usize) {
        self.record_snapshot(cx, cy);
        if self.rows.is_empty() {
            self.rows.push(Row::with_tab_width("", self.tab_width));
            self.rows.push(Row::with_tab_width("", self.tab_width));
        } else if cy >= self.rows.len() {
            self.rows.push(Row::with_tab_width("", self.tab_width));
        } else {
            let new_row = self.rows[cy].split(cx);
            self.rows.insert(cy + 1, new_row);
        }
        self.dirty = true;
    }

    /// Performs a backspace operation from `(cx, cy)`, returning the new `(cx, cy)` cursor
    /// position if a character or line break was deleted.
    pub fn delete_char(&mut self, cx: usize, cy: usize) -> Option<(usize, usize)> {
        if cy >= self.rows.len() {
            return None;
        }

        if cx > 0 {
            self.record_snapshot(cx, cy);
            let target_cx = cx.min(self.rows[cy].len) - 1;
            self.rows[cy].delete_char(target_cx);
            self.dirty = true;
            Some((target_cx, cy))
        } else if cy > 0 {
            self.record_snapshot(cx, cy);
            let current_row = self.rows.remove(cy);
            let prev_len = self.rows[cy - 1].len;
            self.rows[cy - 1].append(&current_row);
            self.dirty = true;
            Some((prev_len, cy - 1))
        } else {
            None
        }
    }

    /// Deletes the character under `(cx, cy)` or joins the next line onto the current row.
    pub fn delete_forward(&mut self, cx: usize, cy: usize) -> bool {
        if cy >= self.rows.len() {
            return false;
        }

        if cx < self.rows[cy].len {
            self.record_snapshot(cx, cy);
            self.rows[cy].delete_char(cx);
            self.dirty = true;
            true
        } else if cy + 1 < self.rows.len() {
            self.record_snapshot(cx, cy);
            let next_row = self.rows.remove(cy + 1);
            self.rows[cy].append(&next_row);
            self.dirty = true;
            true
        } else {
            false
        }
    }

    /// Serializes the buffer rows joined by `\n`.
    pub fn contents(&self) -> String {
        self.rows
            .iter()
            .map(Row::text)
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Writes the buffer to its associated `filename` and clears `dirty`.
    pub fn save(&mut self) -> Result<usize> {
        let filename = self
            .filename
            .clone()
            .ok_or_else(|| anyhow::anyhow!("No file name"))?;
        self.save_as(filename)
    }

    /// Writes the buffer to `filename`, updates `self.filename`, and clears `dirty`.
    pub fn save_as(&mut self, filename: impl Into<String>) -> Result<usize> {
        let filename = filename.into();
        let data = self.contents();
        fs::write(&filename, &data)?;
        self.filename = Some(filename);
        self.dirty = false;
        for snap in &mut self.undo_stack {
            snap.dirty = true;
        }
        for snap in &mut self.redo_stack {
            snap.dirty = true;
        }
        Ok(data.len())
    }

    /// Returns a reference to the row at the given index if it exists.
    pub fn get_row(&self, index: usize) -> Option<&Row> {
        self.rows.get(index)
    }

    /// Returns the number of rows in the buffer.
    pub fn len(&self) -> usize {
        self.rows.len()
    }

    /// Returns true if the buffer is empty.
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn row_tracks_character_count_for_utf8() {
        let row = Row::from("héllo 🦀 world");
        assert_eq!(row.len, 13);
        assert!(row.text().len() > row.len);
    }

    #[test]
    fn row_slice_chars_handles_multibyte_boundaries_safely() {
        let row = Row::from("αβγ🦀δεζ");
        assert_eq!(row.slice_chars(0, 4), "αβγ🦀");
        assert_eq!(row.slice_chars(2, 3), "γ🦀δ");
        assert_eq!(row.slice_chars(10, 5), "");
    }

    #[test]
    fn row_insert_and_delete_char_handle_utf8() {
        let mut row = Row::from("a🦀b");
        assert_eq!(row.len, 3);

        row.insert_char(1, 'é');
        assert_eq!(row.text(), "aé🦀b");
        assert_eq!(row.len, 4);

        row.insert_char(4, '!');
        assert_eq!(row.text(), "aé🦀b!");
        assert_eq!(row.len, 5);

        assert_eq!(row.delete_char(2), Some('🦀'));
        assert_eq!(row.text(), "aéb!");
        assert_eq!(row.len, 4);
        assert_eq!(row.delete_char(99), None);
    }

    #[test]
    fn row_split_and_append_preserve_utf8_and_len() {
        let mut row = Row::from("héllo🦀world");
        let right = row.split(6);
        assert_eq!(row.text(), "héllo🦀");
        assert_eq!(row.len, 6);
        assert_eq!(right.text(), "world");
        assert_eq!(right.len, 5);

        row.append(&right);
        assert_eq!(row.text(), "héllo🦀world");
        assert_eq!(row.len, 11);
    }

    #[test]
    fn gap_buffer_localized_edits_move_and_grow_gap_correctly() {
        let mut gb = GapBuffer::from("abcdef");
        assert_eq!(gb.len(), 6);
        assert_eq!(gb.gap_position(), 6);

        // Move gap to middle and insert multiple characters sequentially (O(1) localized)
        for (offset, ch) in "01234567890123456789".chars().enumerate() {
            gb.insert(3 + offset, ch);
        }
        assert_eq!(gb.to_string(), "abc01234567890123456789def");
        assert_eq!(gb.gap_position(), 23);

        // Consecutive backspaces right before gap_start
        assert_eq!(gb.remove(22), Some('9'));
        assert_eq!(gb.remove(21), Some('8'));
        assert_eq!(gb.to_string(), "abc012345678901234567def");

        // Equality ignores internal gap placement
        let mut other = GapBuffer::from("abc012345678901234567def");
        other.move_gap(1);
        assert_eq!(gb, other);
    }

    #[test]
    fn row_expands_tabs_to_configurable_tab_stops_in_render() {
        let row = Row::with_tab_width("\thello\t!", 4);
        assert_eq!(row.text(), "\thello\t!");
        assert_eq!(row.len, 8);
        // Leading '\t' -> 4 spaces (cols 0..4), "hello" -> cols 4..9, second '\t' at col 9 -> 3 spaces to col 12, '!' -> col 12
        assert_eq!(row.render, "    hello   !");
        assert_eq!(row.render_len, 13);
        assert_eq!(row.cx_to_rx(0), 0);
        assert_eq!(row.cx_to_rx(1), 4);
        assert_eq!(row.cx_to_rx(6), 9);
        assert_eq!(row.cx_to_rx(7), 12);
        assert_eq!(row.rx_to_cx(0), 0);
        assert_eq!(row.rx_to_cx(3), 0);
        assert_eq!(row.rx_to_cx(4), 1);
        assert_eq!(row.rx_to_cx(12), 7);
    }

    #[test]
    fn buffer_line_split_and_join_mutations() {
        let mut buf = Buffer::from_lines(&["hello world"]);
        buf.insert_newline(5, 0);
        assert_eq!(buf.len(), 2);
        assert_eq!(buf.get_row(0).unwrap().text(), "hello");
        assert_eq!(buf.get_row(1).unwrap().text(), " world");
        assert!(buf.dirty);

        let pos = buf.delete_char(0, 1);
        assert_eq!(pos, Some((5, 0)));
        assert_eq!(buf.len(), 1);
        assert_eq!(buf.get_row(0).unwrap().text(), "hello world");
    }

    #[test]
    fn buffer_save_as_and_open_roundtrip() {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("omo_test_{nanos}.txt"));
        let path_str = path.to_string_lossy().to_string();

        let mut buf = Buffer::from_lines(&["first 🦀 line", "second line"]);
        buf.dirty = true;
        let bytes = buf.save_as(&path_str).unwrap();
        assert!(bytes > 0);
        assert!(!buf.dirty);
        assert_eq!(buf.filename.as_deref(), Some(path_str.as_str()));

        let loaded = Buffer::open(&path_str).unwrap();
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded.get_row(0).unwrap().text(), "first 🦀 line");
        assert_eq!(loaded.get_row(1).unwrap().text(), "second line");
        assert!(!loaded.dirty);

        let _ = fs::remove_file(&path);
    }

    #[test]
    fn buffer_undo_and_redo_restore_text_cursor_and_dirty_state() {
        let mut buf = Buffer::from_lines(&["abc"]);
        assert!(!buf.dirty);

        buf.insert_char(3, 0, '!');
        assert_eq!(buf.contents(), "abc!");
        assert!(buf.dirty);

        let restored = buf.undo(4, 0);
        assert_eq!(restored, Some((3, 0)));
        assert_eq!(buf.contents(), "abc");
        assert!(!buf.dirty);

        let redone = buf.redo(3, 0);
        assert_eq!(redone, Some((4, 0)));
        assert_eq!(buf.contents(), "abc!");
        assert!(buf.dirty);
    }

    #[test]
    fn buffer_find_returns_utf8_character_indices_forward_and_backward() {
        let buf = Buffer::from_lines(&["αβ🦀 needle here", "second needle"]);

        let first = buf.find("needle", 0, 0, true);
        assert_eq!(first, Some((4, 0)));

        let second = buf.find("needle", 5, 0, true);
        assert_eq!(second, Some((7, 1)));

        // Wrap around forward
        let wrapped = buf.find("needle", 8, 1, true);
        assert_eq!(wrapped, Some((4, 0)));

        // Backward search from second line finds first line match
        let backward = buf.find("needle", 7, 1, false);
        assert_eq!(backward, Some((4, 0)));
    }
}
