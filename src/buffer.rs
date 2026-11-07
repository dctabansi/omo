use anyhow::{Result, bail};
use std::collections::VecDeque;
use std::fmt;
use std::fs;
use std::io::Write as _;
use std::process::{Command, Stdio};
use std::sync::Arc;

use crate::config::DEFAULT_TAB_WIDTH;

const DEFAULT_GAP_SIZE: usize = 16;

/// Replaces non-printable control characters (other than `\t`) with the Unicode replacement
/// character so raw-mode terminal rendering cannot execute control codes or escape sequences.
pub fn sanitize_render_char(c: char) -> char {
    if c != '\t' && c.is_control() {
        '\u{FFFD}'
    } else {
        c
    }
}

/// Returns the terminal display column width (`0`, `1`, or `2`) of a single character.
pub fn char_display_width(c: char) -> usize {
    if c.is_control() {
        return 0;
    }
    let u = c as u32;
    // Zero-width combining marks, format characters, and variation selectors
    if matches!(
        u,
        0x0300..=0x036F
            | 0x0483..=0x0489
            | 0x0591..=0x05BD
            | 0x05BF
            | 0x05C1..=0x05C2
            | 0x05C4..=0x05C5
            | 0x05C7
            | 0x0610..=0x061A
            | 0x064B..=0x065F
            | 0x0670
            | 0x06D6..=0x06DC
            | 0x06DF..=0x06E4
            | 0x06E7..=0x06E8
            | 0x06EA..=0x06ED
            | 0x0711
            | 0x0730..=0x074A
            | 0x07A6..=0x07B0
            | 0x07EB..=0x07F3
            | 0x0E31
            | 0x0E34..=0x0E3A
            | 0x0E47..=0x0E4E
            | 0x1AB0..=0x1AFF
            | 0x1DC0..=0x1DFF
            | 0x200B..=0x200F
            | 0x202A..=0x202E
            | 0x2060..=0x206F
            | 0x20D0..=0x20FF
            | 0xFE00..=0xFE0F
            | 0xFE20..=0xFE2F
            | 0xFEFF
            | 0xE0100..=0xE01EF
    ) {
        return 0;
    }

    // Wide (2-column) East Asian, Fullwidth, and Emoji characters
    if matches!(
        u,
        0x1100..=0x115F
            | 0x231A..=0x231B
            | 0x2329..=0x232A
            | 0x23E9..=0x23F3
            | 0x23F8..=0x23FA
            | 0x25FD..=0x25FE
            | 0x2614..=0x2615
            | 0x2648..=0x2653
            | 0x267F
            | 0x2693
            | 0x26A1
            | 0x26AA..=0x26AB
            | 0x26BD..=0x26BE
            | 0x26C4..=0x26C5
            | 0x26CE
            | 0x26D4
            | 0x26EA
            | 0x26F2..=0x26F3
            | 0x26F5
            | 0x26FA
            | 0x26FD
            | 0x2705
            | 0x270A..=0x270B
            | 0x2728
            | 0x274C
            | 0x274E
            | 0x2753..=0x2755
            | 0x2757
            | 0x2795..=0x2797
            | 0x27B0
            | 0x27BF
            | 0x2B1B..=0x2B1C
            | 0x2B50
            | 0x2B55
            | 0x2E80..=0x303E
            | 0x3040..=0x33BF
            | 0x3400..=0x4DBF
            | 0x4E00..=0xA4CF
            | 0xA960..=0xA97F
            | 0xAC00..=0xD7A3
            | 0xF900..=0xFAFF
            | 0xFE10..=0xFE19
            | 0xFE30..=0xFE6F
            | 0xFF01..=0xFF60
            | 0xFFE0..=0xFFE6
            | 0x1F000..=0x1FAFF
            | 0x20000..=0x2FFFD
            | 0x30000..=0x3FFFD
    ) {
        return 2;
    }

    1
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CharClass {
    Whitespace,
    Word,
    Punctuation,
}

fn classify_char(c: char) -> CharClass {
    if c.is_whitespace() {
        CharClass::Whitespace
    } else if c.is_alphanumeric() || c == '_' {
        CharClass::Word
    } else {
        CharClass::Punctuation
    }
}

/// Returns the total terminal display column width of a string slice.
pub fn str_display_width(s: &str) -> usize {
    s.chars()
        .map(|c| char_display_width(sanitize_render_char(c)))
        .sum()
}

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
    /// expanding `\t` characters to `self.tab_width` tab stops and replacing control characters.
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
                let safe = sanitize_render_char(c);
                rendered.push(safe);
                rx += char_display_width(safe);
            }
        }
        self.render = rendered;
        self.render_len = rx;
    }

    /// Converts a logical character column `cx` into a visual render column `rx`,
    /// accounting for expanded tab stops and character display widths.
    pub fn cx_to_rx(&self, cx: usize) -> usize {
        let tab_w = self.tab_width.max(1);
        let mut rx = 0;
        for c in self.buffer.chars().take(cx) {
            if c == '\t' {
                rx += tab_w - (rx % tab_w);
            } else {
                rx += char_display_width(sanitize_render_char(c));
            }
        }
        if cx > self.len {
            rx += cx - self.len;
        }
        rx
    }

    /// Converts a visual render column `target_rx` into the corresponding logical character column `cx`.
    pub fn rx_to_cx(&self, target_rx: usize) -> usize {
        let tab_w = self.tab_width.max(1);
        let mut rx = 0;
        for (cx, c) in self.buffer.chars().enumerate() {
            let next_rx = if c == '\t' {
                rx + (tab_w - (rx % tab_w))
            } else {
                rx + char_display_width(sanitize_render_char(c))
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

    /// Returns the character index of the first non-whitespace character (`' '` or `'\t'`),
    /// or `0` if the row is empty or consists solely of whitespace.
    pub fn first_non_whitespace_cx(&self) -> usize {
        self.buffer
            .chars()
            .position(|c| c != ' ' && c != '\t')
            .unwrap_or(0)
    }

    /// Returns a display-column-safe horizontal slice of the tab-expanded `render` string
    /// for viewport rendering within `max_cols` terminal columns.
    pub fn slice_chars(&self, col_offset: usize, max_cols: usize) -> String {
        if max_cols == 0 {
            return String::new();
        }
        let end_col = col_offset.saturating_add(max_cols);
        let mut out = String::new();
        let mut col = 0;
        let mut started = false;

        for c in self.render.chars() {
            let w = char_display_width(c);
            if w == 0 {
                if started && col <= end_col {
                    out.push(c);
                }
                continue;
            }
            let next_col = col + w;
            if next_col <= col_offset {
                col = next_col;
                continue;
            }
            if col < col_offset && next_col > col_offset {
                for _ in col_offset..next_col.min(end_col) {
                    out.push(' ');
                }
                started = true;
                col = next_col;
                continue;
            }
            if next_col > end_col {
                break;
            }
            out.push(c);
            started = true;
            col = next_col;
        }
        out
    }

    /// Returns all character start indices (including overlapping occurrences) where `query`
    /// matches within this row (case-insensitive by default).
    #[allow(dead_code)]
    pub fn find_matches(&self, query: &str) -> Vec<usize> {
        self.find_matches_with_case(query, false)
    }

    /// Returns all character start indices (including overlapping occurrences) where `query`
    /// matches within this row, honoring `case_sensitive`.
    pub fn find_matches_with_case(&self, query: &str, case_sensitive: bool) -> Vec<usize> {
        if query.is_empty() {
            return Vec::new();
        }
        let row_chars: Vec<char> = self.buffer.chars().collect();
        let query_chars: Vec<char> = query.chars().collect();
        let qlen = query_chars.len();
        if qlen > row_chars.len() {
            return Vec::new();
        }

        let mut matches = Vec::new();
        for start in 0..=(row_chars.len() - qlen) {
            let matched = if case_sensitive {
                row_chars[start..start + qlen] == query_chars[..]
            } else {
                row_chars[start..start + qlen]
                    .iter()
                    .zip(query_chars.iter())
                    .all(|(rc, qc)| rc.to_lowercase().eq(qc.to_lowercase()))
            };
            if matched {
                matches.push(start);
            }
        }
        matches
    }

    /// Returns the logical character at `index`, if within bounds.
    pub fn char_at(&self, index: usize) -> Option<char> {
        self.buffer.chars().nth(index)
    }
}

const MAX_UNDO_HISTORY: usize = 200;

/// Detects whether an existing file path is read-only or not writable by the current process.
pub fn detect_file_read_only(filename: &str) -> bool {
    let Ok(meta) = fs::metadata(filename) else {
        return false;
    };
    if meta.permissions().readonly() {
        return true;
    }
    match fs::OpenOptions::new().write(true).open(filename) {
        Ok(_) => false,
        Err(err) => err.kind() == std::io::ErrorKind::PermissionDenied,
    }
}

/// Captures a reversible snapshot of buffer rows, cursor position, and dirty state.
/// Rows are stored via `Arc<Row>` so unchanged lines share memory across snapshots.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BufferSnapshot {
    pub rows: Vec<Arc<Row>>,
    pub cx: usize,
    pub cy: usize,
    pub dirty: bool,
}

/// Holds the entirety of a text buffer.
pub struct Buffer {
    rows: Vec<Arc<Row>>,
    pub filename: Option<String>,
    pub dirty: bool,
    pub read_only: bool,
    pub tab_width: usize,
    pub trailing_newline: bool,
    pub crlf: bool,
    undo_stack: VecDeque<BufferSnapshot>,
    redo_stack: Vec<BufferSnapshot>,
    coalesce_insert_pos: Option<(usize, usize)>,
}

impl Default for Buffer {
    fn default() -> Self {
        Self {
            rows: Vec::new(),
            filename: None,
            dirty: false,
            read_only: false,
            tab_width: DEFAULT_TAB_WIDTH,
            trailing_newline: true,
            crlf: false,
            undo_stack: VecDeque::new(),
            redo_stack: Vec::new(),
            coalesce_insert_pos: None,
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
        let filename = filename.into();
        let read_only = detect_file_read_only(&filename);
        Self {
            rows: Vec::new(),
            filename: Some(filename),
            dirty: false,
            read_only,
            tab_width: DEFAULT_TAB_WIDTH,
            trailing_newline: true,
            crlf: false,
            undo_stack: VecDeque::new(),
            redo_stack: Vec::new(),
            coalesce_insert_pos: None,
        }
    }

    /// Creates a buffer populated from a slice of string lines.
    #[allow(dead_code)]
    pub fn from_lines(lines: &[&str]) -> Self {
        Self {
            rows: lines.iter().copied().map(Row::from).map(Arc::new).collect(),
            filename: None,
            dirty: false,
            read_only: false,
            tab_width: DEFAULT_TAB_WIDTH,
            trailing_newline: true,
            crlf: false,
            undo_stack: VecDeque::new(),
            redo_stack: Vec::new(),
            coalesce_insert_pos: None,
        }
    }

    /// Loads a buffer from a file path. If the file does not exist yet, returns an empty buffer
    /// bound to `filename`.
    pub fn open(filename: &str) -> Result<Self> {
        match fs::read_to_string(filename) {
            Ok(contents) => {
                let crlf = contents.contains("\r\n");
                let trailing_newline = contents.is_empty() || contents.ends_with('\n');
                let read_only = detect_file_read_only(filename);
                let mut rows = Vec::new();
                if !contents.is_empty() {
                    let body = contents
                        .strip_suffix("\r\n")
                        .or_else(|| contents.strip_suffix('\n'))
                        .unwrap_or(&contents);
                    for line in body.split('\n') {
                        let clean = line.strip_suffix('\r').unwrap_or(line);
                        rows.push(Arc::new(Row::from(clean)));
                    }
                }
                Ok(Self {
                    rows,
                    filename: Some(filename.to_string()),
                    dirty: false,
                    read_only,
                    tab_width: DEFAULT_TAB_WIDTH,
                    trailing_newline,
                    crlf,
                    undo_stack: VecDeque::new(),
                    redo_stack: Vec::new(),
                    coalesce_insert_pos: None,
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
            if row.tab_width != normalized {
                Arc::make_mut(row).set_tab_width(normalized);
            }
        }
    }

    /// Breaks any active character-insertion undo coalescing group.
    pub fn break_undo_coalesce(&mut self) {
        self.coalesce_insert_pos = None;
    }

    /// Records the current buffer state onto `undo_stack` and clears `redo_stack`.
    pub fn record_snapshot(&mut self, cx: usize, cy: usize) {
        if self.undo_stack.len() >= MAX_UNDO_HISTORY {
            self.undo_stack.pop_front();
        }
        self.undo_stack.push_back(BufferSnapshot {
            rows: self.rows.clone(),
            cx,
            cy,
            dirty: self.dirty,
        });
        self.redo_stack.clear();
        self.coalesce_insert_pos = None;
    }

    /// Restores the most recent snapshot from `undo_stack`, pushing the current state to
    /// `redo_stack`, and returns the restored `(cx, cy)` cursor coordinates.
    pub fn undo(&mut self, current_cx: usize, current_cy: usize) -> Option<(usize, usize)> {
        let snapshot = self.undo_stack.pop_back()?;
        self.redo_stack.push(BufferSnapshot {
            rows: self.rows.clone(),
            cx: current_cx,
            cy: current_cy,
            dirty: self.dirty,
        });
        self.rows = snapshot.rows;
        self.dirty = snapshot.dirty;
        self.coalesce_insert_pos = None;
        Some((snapshot.cx, snapshot.cy))
    }

    /// Reapplies the most recent snapshot from `redo_stack`, pushing the current state to
    /// `undo_stack`, and returns the restored `(cx, cy)` cursor coordinates.
    pub fn redo(&mut self, current_cx: usize, current_cy: usize) -> Option<(usize, usize)> {
        let snapshot = self.redo_stack.pop()?;
        if self.undo_stack.len() >= MAX_UNDO_HISTORY {
            self.undo_stack.pop_front();
        }
        self.undo_stack.push_back(BufferSnapshot {
            rows: self.rows.clone(),
            cx: current_cx,
            cy: current_cy,
            dirty: self.dirty,
        });
        self.rows = snapshot.rows;
        self.dirty = snapshot.dirty;
        self.coalesce_insert_pos = None;
        Some((snapshot.cx, snapshot.cy))
    }

    /// Searches for `query` (case-insensitive) starting from `(start_cx, start_cy)` in `forward`
    /// or backward order, wrapping around the buffer and returning `(match_cx, match_cy)`.
    #[allow(dead_code)]
    pub fn find(
        &self,
        query: &str,
        start_cx: usize,
        start_cy: usize,
        forward: bool,
    ) -> Option<(usize, usize)> {
        self.find_with_case(query, start_cx, start_cy, forward, false)
    }

    /// Searches for `query` starting from `(start_cx, start_cy)` in `forward` or backward order
    /// with configurable `case_sensitive` matching, wrapping around the buffer.
    pub fn find_with_case(
        &self,
        query: &str,
        start_cx: usize,
        start_cy: usize,
        forward: bool,
        case_sensitive: bool,
    ) -> Option<(usize, usize)> {
        if query.is_empty() || self.rows.is_empty() {
            return None;
        }

        let num_rows = self.rows.len();
        let clamped_cy = start_cy.min(num_rows - 1);

        if forward {
            if let Some(cx) = self.rows[clamped_cy]
                .find_matches_with_case(query, case_sensitive)
                .into_iter()
                .find(|&m| m >= start_cx)
            {
                return Some((cx, clamped_cy));
            }

            for offset in 1..num_rows {
                let row_idx = (clamped_cy + offset) % num_rows;
                if let Some(cx) = self.rows[row_idx]
                    .find_matches_with_case(query, case_sensitive)
                    .into_iter()
                    .next()
                {
                    return Some((cx, row_idx));
                }
            }

            if let Some(cx) = self.rows[clamped_cy]
                .find_matches_with_case(query, case_sensitive)
                .into_iter()
                .find(|&m| m < start_cx)
            {
                return Some((cx, clamped_cy));
            }
        } else {
            if let Some(cx) = self.rows[clamped_cy]
                .find_matches_with_case(query, case_sensitive)
                .into_iter()
                .rev()
                .find(|&m| m < start_cx)
            {
                return Some((cx, clamped_cy));
            }

            for offset in 1..num_rows {
                let row_idx = (clamped_cy + num_rows - offset) % num_rows;
                if let Some(cx) = self.rows[row_idx]
                    .find_matches_with_case(query, case_sensitive)
                    .into_iter()
                    .next_back()
                {
                    return Some((cx, row_idx));
                }
            }

            if let Some(cx) = self.rows[clamped_cy]
                .find_matches_with_case(query, case_sensitive)
                .into_iter()
                .rev()
                .find(|&m| m >= start_cx)
            {
                return Some((cx, clamped_cy));
            }
        }

        None
    }

    /// Inserts a character at buffer coordinates `(cx, cy)` without recording a new undo snapshot.
    pub fn insert_char_no_snapshot(&mut self, cx: usize, cy: usize, c: char) {
        while self.rows.len() <= cy {
            self.rows
                .push(Arc::new(Row::with_tab_width("", self.tab_width)));
        }
        Arc::make_mut(&mut self.rows[cy]).insert_char(cx, c);
        self.dirty = true;
        self.coalesce_insert_pos = None;
    }

    /// Inserts a character at buffer coordinates `(cx, cy)`, coalescing contiguous
    /// non-whitespace typing on the same line into a single undo snapshot.
    pub fn insert_char(&mut self, cx: usize, cy: usize, c: char) {
        let should_coalesce = self.coalesce_insert_pos == Some((cx, cy)) && !c.is_whitespace();
        if !should_coalesce {
            self.record_snapshot(cx, cy);
        }
        while self.rows.len() <= cy {
            self.rows
                .push(Arc::new(Row::with_tab_width("", self.tab_width)));
        }
        Arc::make_mut(&mut self.rows[cy]).insert_char(cx, c);
        self.dirty = true;
        self.coalesce_insert_pos = if c.is_whitespace() {
            None
        } else {
            Some((cx + 1, cy))
        };
    }

    /// Splits the row at `(cx, cy)` or inserts a new empty line when the buffer is empty.
    #[allow(dead_code)]
    pub fn insert_newline(&mut self, cx: usize, cy: usize) {
        let _ = self.insert_newline_with_indent(cx, cy, false);
    }

    /// Splits the row at `(cx, cy)`, optionally copying the current line's leading indentation
    /// (`' '` and `'\t'` up to `cx`) onto the new row, and returns the new `(cx, cy)` cursor position.
    pub fn insert_newline_with_indent(
        &mut self,
        cx: usize,
        cy: usize,
        auto_indent: bool,
    ) -> (usize, usize) {
        self.record_snapshot(cx, cy);
        if self.rows.is_empty() {
            self.rows
                .push(Arc::new(Row::with_tab_width("", self.tab_width)));
            self.rows
                .push(Arc::new(Row::with_tab_width("", self.tab_width)));
            self.dirty = true;
            return (0, 1);
        }
        if cy >= self.rows.len() {
            self.rows
                .push(Arc::new(Row::with_tab_width("", self.tab_width)));
            self.dirty = true;
            return (0, self.rows.len() - 1);
        }

        let indent: String = if auto_indent {
            self.rows[cy]
                .buffer
                .chars()
                .take(cx)
                .take_while(|&c| c == ' ' || c == '\t')
                .collect()
        } else {
            String::new()
        };
        let indent_len = indent.chars().count();
        let tail = Arc::make_mut(&mut self.rows[cy]).split(cx);
        let mut new_row = Row::with_tab_width(&indent, self.tab_width);
        new_row.append(&tail);
        self.rows.insert(cy + 1, Arc::new(new_row));
        self.dirty = true;
        (indent_len, cy + 1)
    }

    /// Inserts a new line below `cy` indented to match `cy`'s leading whitespace without
    /// modifying or splitting row `cy`, returning the new `(cx, cy)` cursor coordinates.
    pub fn insert_line_below(&mut self, cx: usize, cy: usize) -> (usize, usize) {
        self.record_snapshot(cx, cy);
        if self.rows.is_empty() {
            self.rows
                .push(Arc::new(Row::with_tab_width("", self.tab_width)));
            self.rows
                .push(Arc::new(Row::with_tab_width("", self.tab_width)));
            self.dirty = true;
            return (0, 1);
        }

        let target_cy = cy.min(self.rows.len() - 1);
        let indent: String = self.rows[target_cy]
            .buffer
            .chars()
            .take_while(|&c| c == ' ' || c == '\t')
            .collect();
        let new_cx = indent.chars().count();
        let new_row = Row::with_tab_width(&indent, self.tab_width);
        self.rows.insert(target_cy + 1, Arc::new(new_row));
        self.dirty = true;
        (new_cx, target_cy + 1)
    }

    /// Inserts a new line above `cy` indented to match `cy`'s leading whitespace without
    /// modifying or splitting row `cy`, returning the new `(cx, cy)` cursor coordinates.
    pub fn insert_line_above(&mut self, cx: usize, cy: usize) -> (usize, usize) {
        self.record_snapshot(cx, cy);
        if self.rows.is_empty() {
            self.rows
                .push(Arc::new(Row::with_tab_width("", self.tab_width)));
            self.dirty = true;
            return (0, 0);
        }

        let target_cy = cy.min(self.rows.len() - 1);
        let indent: String = self.rows[target_cy]
            .buffer
            .chars()
            .take_while(|&c| c == ' ' || c == '\t')
            .collect();
        let new_cx = indent.chars().count();
        let new_row = Row::with_tab_width(&indent, self.tab_width);
        self.rows.insert(target_cy, Arc::new(new_row));
        self.dirty = true;
        (new_cx, target_cy)
    }

    /// Duplicates the row at `cy` directly below it, returning `(cx, cy + 1)` if a row existed.
    pub fn duplicate_line(&mut self, cx: usize, cy: usize) -> Option<(usize, usize)> {
        if cy >= self.rows.len() {
            return None;
        }
        self.record_snapshot(cx, cy);
        let cloned = Arc::clone(&self.rows[cy]);
        self.rows.insert(cy + 1, cloned);
        self.dirty = true;
        Some((cx.min(self.rows[cy + 1].len), cy + 1))
    }

    /// Moves the row at `cy` up by one line (`cy - 1`), returning the new `(cx, cy - 1)`.
    pub fn move_line_up(&mut self, cx: usize, cy: usize) -> Option<(usize, usize)> {
        if cy == 0 || cy >= self.rows.len() {
            return None;
        }
        self.record_snapshot(cx, cy);
        self.rows.swap(cy - 1, cy);
        self.dirty = true;
        Some((cx.min(self.rows[cy - 1].len), cy - 1))
    }

    /// Moves the row at `cy` down by one line (`cy + 1`), returning the new `(cx, cy + 1)`.
    pub fn move_line_down(&mut self, cx: usize, cy: usize) -> Option<(usize, usize)> {
        if cy + 1 >= self.rows.len() {
            return None;
        }
        self.record_snapshot(cx, cy);
        self.rows.swap(cy, cy + 1);
        self.dirty = true;
        Some((cx.min(self.rows[cy + 1].len), cy + 1))
    }

    /// Removes one leading tab stop (`'\t'` or up to `self.tab_width` spaces) from row `cy`,
    /// returning the updated `cx` if any indentation was removed.
    pub fn unindent_line(&mut self, cx: usize, cy: usize) -> Option<usize> {
        if cy >= self.rows.len() {
            return None;
        }
        let first = self.rows[cy].buffer.chars().next()?;
        let remove_count = if first == '\t' {
            1
        } else if first == ' ' {
            let max_spaces = self.tab_width.max(1);
            self.rows[cy]
                .buffer
                .chars()
                .take(max_spaces)
                .take_while(|&c| c == ' ')
                .count()
        } else {
            0
        };

        if remove_count == 0 {
            return None;
        }

        self.record_snapshot(cx, cy);
        let row_mut = Arc::make_mut(&mut self.rows[cy]);
        for _ in 0..remove_count {
            row_mut.delete_char(0);
        }
        if self.rows.len() == 1 && self.rows[0].len == 0 {
            self.rows.clear();
        }
        self.dirty = true;
        Some(cx.saturating_sub(remove_count))
    }

    /// Computes the `(cx, cy)` position of the previous word boundary to the left of `(cx, cy)`.
    pub fn word_boundary_left(&self, cx: usize, cy: usize) -> (usize, usize) {
        if self.rows.is_empty() {
            return (0, 0);
        }
        let clamped_cy = cy.min(self.rows.len() - 1);
        let chars: Vec<char> = self.rows[clamped_cy].buffer.chars().collect();
        let mut idx = cx.min(chars.len());

        if idx == 0 {
            if clamped_cy > 0 {
                return (self.rows[clamped_cy - 1].len, clamped_cy - 1);
            }
            return (0, 0);
        }

        while idx > 0 && classify_char(chars[idx - 1]) == CharClass::Whitespace {
            idx -= 1;
        }
        if idx > 0 {
            let target_class = classify_char(chars[idx - 1]);
            while idx > 0 && classify_char(chars[idx - 1]) == target_class {
                idx -= 1;
            }
        }
        (idx, clamped_cy)
    }

    /// Computes the `(cx, cy)` position of the next word boundary to the right of `(cx, cy)`.
    pub fn word_boundary_right(&self, cx: usize, cy: usize) -> (usize, usize) {
        if self.rows.is_empty() {
            return (0, 0);
        }
        let clamped_cy = cy.min(self.rows.len() - 1);
        let chars: Vec<char> = self.rows[clamped_cy].buffer.chars().collect();
        let len = chars.len();
        let mut idx = cx.min(len);

        if idx >= len {
            if clamped_cy + 1 < self.rows.len() {
                return (0, clamped_cy + 1);
            }
            return (len, clamped_cy);
        }

        while idx < len && classify_char(chars[idx]) == CharClass::Whitespace {
            idx += 1;
        }
        if idx < len {
            let target_class = classify_char(chars[idx]);
            while idx < len && classify_char(chars[idx]) == target_class {
                idx += 1;
            }
        }
        (idx, clamped_cy)
    }

    /// Deletes from `(cx, cy)` back to the previous word boundary, returning the new `(cx, cy)`.
    pub fn delete_word_left(&mut self, cx: usize, cy: usize) -> Option<(usize, usize)> {
        if cy >= self.rows.len() {
            return None;
        }
        let (target_cx, target_cy) = self.word_boundary_left(cx, cy);
        if (target_cx, target_cy) == (cx, cy) {
            return None;
        }
        if target_cy < cy {
            return self.delete_char(0, cy);
        }

        let clamped_cx = cx.min(self.rows[cy].len);
        let count = clamped_cx.saturating_sub(target_cx);
        if count == 0 {
            return None;
        }
        self.record_snapshot(cx, cy);
        let row_mut = Arc::make_mut(&mut self.rows[cy]);
        for _ in 0..count {
            row_mut.delete_char(target_cx);
        }
        if self.rows.len() == 1 && self.rows[0].len == 0 {
            self.rows.clear();
        }
        self.dirty = true;
        Some((target_cx, cy))
    }

    /// Deletes from `(cx, cy)` forward to the next word boundary, returning `true` if text was removed.
    pub fn delete_word_right(&mut self, cx: usize, cy: usize) -> bool {
        if cy >= self.rows.len() {
            return false;
        }
        let (target_cx, target_cy) = self.word_boundary_right(cx, cy);
        if (target_cx, target_cy) == (cx, cy) {
            return false;
        }
        if target_cy > cy {
            return self.delete_forward(self.rows[cy].len, cy);
        }

        let clamped_cx = cx.min(self.rows[cy].len);
        let count = target_cx.saturating_sub(clamped_cx);
        if count == 0 {
            return false;
        }
        self.record_snapshot(cx, cy);
        let row_mut = Arc::make_mut(&mut self.rows[cy]);
        for _ in 0..count {
            row_mut.delete_char(clamped_cx);
        }
        if self.rows.len() == 1 && self.rows[0].len == 0 {
            self.rows.clear();
        }
        self.dirty = true;
        true
    }

    /// Performs a backspace operation from `(cx, cy)` without recording a new snapshot.
    pub fn delete_char_no_snapshot(&mut self, cx: usize, cy: usize) -> Option<(usize, usize)> {
        if cy >= self.rows.len() {
            return None;
        }
        if cx > 0 {
            let target_cx = cx.min(self.rows[cy].len) - 1;
            Arc::make_mut(&mut self.rows[cy]).delete_char(target_cx);
            if self.rows.len() == 1 && self.rows[0].len == 0 {
                self.rows.clear();
            }
            self.dirty = true;
            self.coalesce_insert_pos = None;
            Some((target_cx, cy))
        } else if cy > 0 {
            let current_row = self.rows.remove(cy);
            let prev_len = self.rows[cy - 1].len;
            Arc::make_mut(&mut self.rows[cy - 1]).append(&current_row);
            if self.rows.len() == 1 && self.rows[0].len == 0 {
                self.rows.clear();
            }
            self.dirty = true;
            self.coalesce_insert_pos = None;
            Some((prev_len, cy - 1))
        } else {
            None
        }
    }

    /// Performs a backspace operation from `(cx, cy)`, returning the new `(cx, cy)` cursor
    /// position if a character or line break was deleted.
    pub fn delete_char(&mut self, cx: usize, cy: usize) -> Option<(usize, usize)> {
        if cy >= self.rows.len() || (cx == 0 && cy == 0) {
            return None;
        }
        self.record_snapshot(cx, cy);
        self.delete_char_no_snapshot(cx, cy)
    }

    /// Deletes the character under `(cx, cy)` without recording a new snapshot.
    pub fn delete_forward_no_snapshot(&mut self, cx: usize, cy: usize) -> bool {
        if cy >= self.rows.len() {
            return false;
        }
        if cx < self.rows[cy].len {
            Arc::make_mut(&mut self.rows[cy]).delete_char(cx);
            if self.rows.len() == 1 && self.rows[0].len == 0 {
                self.rows.clear();
            }
            self.dirty = true;
            self.coalesce_insert_pos = None;
            true
        } else if cy + 1 < self.rows.len() {
            let next_row = self.rows.remove(cy + 1);
            Arc::make_mut(&mut self.rows[cy]).append(&next_row);
            if self.rows.len() == 1 && self.rows[0].len == 0 {
                self.rows.clear();
            }
            self.dirty = true;
            self.coalesce_insert_pos = None;
            true
        } else {
            false
        }
    }

    /// Deletes the character under `(cx, cy)` or joins the next line onto the current row.
    pub fn delete_forward(&mut self, cx: usize, cy: usize) -> bool {
        if cy >= self.rows.len() || (cx >= self.rows[cy].len && cy + 1 >= self.rows.len()) {
            return false;
        }
        self.record_snapshot(cx, cy);
        self.delete_forward_no_snapshot(cx, cy)
    }

    /// Deletes the entire row at `cy`, recording an undo snapshot and returning the clamped
    /// `(new_cx, new_cy)` cursor position if a line was removed.
    pub fn delete_line(&mut self, cx: usize, cy: usize) -> Option<(usize, usize)> {
        if cy >= self.rows.len() {
            return None;
        }
        self.record_snapshot(cx, cy);
        self.rows.remove(cy);
        self.dirty = true;
        if self.rows.is_empty() {
            Some((0, 0))
        } else {
            let new_cy = cy.min(self.rows.len() - 1);
            let new_cx = cx.min(self.rows[new_cy].len);
            Some((new_cx, new_cy))
        }
    }

    /// Serializes the buffer rows joined by `\n`.
    #[allow(dead_code)]
    pub fn contents(&self) -> String {
        self.rows
            .iter()
            .map(|r| r.text())
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Serializes the buffer for saving to disk, honoring `self.crlf` and `self.trailing_newline`.
    pub fn serialized_bytes(&self) -> String {
        let line_ending = if self.crlf { "\r\n" } else { "\n" };
        let mut data = self
            .rows
            .iter()
            .map(|r| r.text())
            .collect::<Vec<_>>()
            .join(line_ending);
        if !self.rows.is_empty() && self.trailing_newline {
            data.push_str(line_ending);
        }
        data
    }

    fn mark_saved(&mut self, filename: String) {
        self.read_only = detect_file_read_only(&filename);
        self.filename = Some(filename);
        self.dirty = false;
        self.coalesce_insert_pos = None;
        for snap in &mut self.undo_stack {
            snap.dirty = true;
        }
        for snap in &mut self.redo_stack {
            snap.dirty = true;
        }
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
        let data = self.serialized_bytes();
        fs::write(&filename, &data)?;
        let len = data.len();
        self.mark_saved(filename);
        Ok(len)
    }

    fn pipe_to_sudo_tee(filename: &str, data: &str) -> Result<()> {
        let mut child = Command::new("sudo")
            .args(["-n", "tee", "--", filename])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()?;
        if let Some(mut stdin) = child.stdin.take() {
            stdin.write_all(data.as_bytes())?;
        }
        let output = child.wait_with_output()?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            bail!(if stderr.is_empty() {
                "sudo tee failed".to_string()
            } else {
                stderr
            });
        }
        Ok(())
    }

    /// Writes the buffer to `filename` using `sudo tee`, suspending the terminal to run
    /// interactive `sudo -v` if cached credentials are not available.
    pub fn save_with_sudo(&mut self, filename: impl Into<String>) -> Result<usize> {
        let filename = filename.into();
        let data = self.serialized_bytes();
        if Self::pipe_to_sudo_tee(&filename, &data).is_err() {
            crate::terminal::Terminal::with_suspended(|| {
                let status = Command::new("sudo").arg("-v").status()?;
                if !status.success() {
                    bail!("sudo authentication failed");
                }
                Self::pipe_to_sudo_tee(&filename, &data)
            })?;
        }
        let len = data.len();
        self.mark_saved(filename);
        Ok(len)
    }

    /// Returns a reference to the row at the given index if it exists.
    pub fn get_row(&self, index: usize) -> Option<&Row> {
        self.rows.get(index).map(Arc::as_ref)
    }

    /// Returns a reference to the `Arc<Row>` at the given index if it exists.
    #[allow(dead_code)]
    pub fn row_arc(&self, index: usize) -> Option<&Arc<Row>> {
        self.rows.get(index)
    }

    /// Returns a reference to the most recent snapshot on `undo_stack` (for testing CoW sharing).
    #[cfg(test)]
    pub fn last_undo_snapshot(&self) -> Option<&BufferSnapshot> {
        self.undo_stack.back()
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
        // 'α', 'β', 'γ' are 1 col each; '🦀' is 2 cols (total 5 cols for "αβγ🦀")
        assert_eq!(row.slice_chars(0, 5), "αβγ🦀");
        assert_eq!(row.slice_chars(0, 4), "αβγ");
        assert_eq!(row.slice_chars(2, 4), "γ🦀δ");
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

    #[test]
    fn row_sanitizes_control_chars_and_tracks_wide_unicode_columns() {
        let row = Row::from("a\x1b\r🦀b");
        assert_eq!(row.render, "a\u{FFFD}\u{FFFD}🦀b");
        // 'a' (1) + '\x1b' (1) + '\r' (1) + '🦀' (2) + 'b' (1) = 6 display cols
        assert_eq!(row.render_len, 6);
        assert_eq!(row.cx_to_rx(3), 3);
        assert_eq!(row.cx_to_rx(4), 5);
        assert_eq!(row.rx_to_cx(4), 3);
        assert_eq!(row.rx_to_cx(5), 4);
    }

    #[test]
    fn buffer_preserves_trailing_blank_lines_and_crlf_on_roundtrip() {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("omo_crlf_test_{nanos}.txt"));
        let path_str = path.to_string_lossy().to_string();

        fs::write(&path, "line1\r\nline2\r\n\r\n").unwrap();
        let mut loaded = Buffer::open(&path_str).unwrap();
        assert!(loaded.crlf);
        assert!(loaded.trailing_newline);
        assert_eq!(loaded.len(), 3);
        assert_eq!(loaded.get_row(2).unwrap().text(), "");

        loaded.save().unwrap();
        let disk = fs::read_to_string(&path).unwrap();
        assert_eq!(disk, "line1\r\nline2\r\n\r\n");

        let _ = fs::remove_file(&path);
    }

    #[test]
    fn row_find_matches_includes_overlapping_occurrences() {
        let row = Row::from("ababa");
        assert_eq!(row.find_matches("aba"), vec![0, 2]);
    }

    #[test]
    fn buffer_coalesces_word_typing_and_normalizes_empty_buffer_rows() {
        let mut buf = Buffer::new();
        assert_eq!(buf.len(), 0);

        // Contiguous word typing coalesces into a single undo snapshot
        for (i, ch) in "hello".chars().enumerate() {
            buf.insert_char(i, 0, ch);
        }
        assert_eq!(buf.contents(), "hello");
        assert_eq!(buf.len(), 1);

        let restored = buf.undo(5, 0);
        assert_eq!(restored, Some((0, 0)));
        assert!(buf.is_empty());
        assert_eq!(buf.len(), 0);

        // Typing a single character and deleting it returns buffer to 0 rows
        buf.insert_char(0, 0, 'x');
        assert_eq!(buf.len(), 1);
        buf.delete_char(1, 0);
        assert!(buf.is_empty());
        assert_eq!(buf.len(), 0);
    }

    #[test]
    fn buffer_delete_line_removes_row_clamps_cursor_and_supports_undo() {
        let mut buf = Buffer::from_lines(&["first line", "second", "third line"]);
        // Delete middle line
        let pos = buf.delete_line(4, 1);
        assert_eq!(pos, Some((4, 1)));
        assert_eq!(buf.len(), 2);
        assert_eq!(buf.contents(), "first line\nthird line");
        assert!(buf.dirty);

        // Undo restores the deleted middle line and cursor position
        let restored = buf.undo(4, 1);
        assert_eq!(restored, Some((4, 1)));
        assert_eq!(buf.len(), 3);
        assert_eq!(buf.contents(), "first line\nsecond\nthird line");

        // Deleting the last line clamps cy to the new last line
        let last_pos = buf.delete_line(8, 2);
        assert_eq!(last_pos, Some((6, 1)));
        assert_eq!(buf.len(), 2);
    }

    #[test]
    fn cow_arc_row_snapshots_share_unchanged_rows_and_clone_only_mutated_row() {
        let mut buf = Buffer::from_lines(&["unchanged 0", "mutated 1", "unchanged 2"]);
        let row0_before = Arc::clone(buf.row_arc(0).unwrap());
        let row1_before = Arc::clone(buf.row_arc(1).unwrap());
        let row2_before = Arc::clone(buf.row_arc(2).unwrap());

        buf.insert_char(9, 1, '!');

        let snap = buf.last_undo_snapshot().unwrap();
        assert!(Arc::ptr_eq(&snap.rows[0], &row0_before));
        assert!(Arc::ptr_eq(&snap.rows[1], &row1_before));
        assert!(Arc::ptr_eq(&snap.rows[2], &row2_before));

        // Unchanged rows in active buffer still point to the exact same Arc allocation
        assert!(Arc::ptr_eq(buf.row_arc(0).unwrap(), &row0_before));
        assert!(Arc::ptr_eq(buf.row_arc(2).unwrap(), &row2_before));
        // Only the mutated row was cloned-on-write
        assert!(!Arc::ptr_eq(buf.row_arc(1).unwrap(), &row1_before));
        assert_eq!(buf.get_row(1).unwrap().text(), "mutated 1!");
    }

    #[test]
    fn move_line_up_and_down_swap_rows_and_support_undo() {
        let mut buf = Buffer::from_lines(&["first", "second", "third"]);
        assert_eq!(buf.move_line_up(2, 0), None);
        assert_eq!(buf.move_line_down(2, 0), Some((2, 1)));
        assert_eq!(buf.contents(), "second\nfirst\nthird");

        assert_eq!(buf.move_line_up(2, 1), Some((2, 0)));
        assert_eq!(buf.contents(), "first\nsecond\nthird");
    }

    #[test]
    fn detect_file_read_only_identifies_readonly_permissions() {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("omo_ro_test_{nanos}.txt"));
        let path_str = path.to_string_lossy().to_string();

        fs::write(&path, "protected\n").unwrap();
        let mut perms = fs::metadata(&path).unwrap().permissions();
        perms.set_readonly(true);
        fs::set_permissions(&path, perms.clone()).unwrap();

        let loaded = Buffer::open(&path_str).unwrap();
        assert!(loaded.read_only);

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut writable = perms;
            writable.set_mode(0o644);
            let _ = fs::set_permissions(&path, writable);
        }
        let _ = fs::remove_file(&path);
    }
}
