use anyhow::Result;
use std::fs;

/// Represents a single line of text in the editor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub text: String,
    /// Cached character count (not byte count) for UTF-8 safe UI rendering.
    pub len: usize,
}

impl From<&str> for Row {
    fn from(slice: &str) -> Self {
        Self {
            text: String::from(slice),
            len: slice.chars().count(),
        }
    }
}

impl Row {
    /// Converts a character index into a UTF-8 byte index within `self.text`.
    pub fn byte_index(&self, char_idx: usize) -> usize {
        self.text
            .char_indices()
            .nth(char_idx)
            .map_or(self.text.len(), |(idx, _)| idx)
    }

    /// Inserts a character at the given character index `at`.
    pub fn insert_char(&mut self, at: usize, c: char) {
        let clamped = at.min(self.len);
        let byte_idx = self.byte_index(clamped);
        self.text.insert(byte_idx, c);
        self.len += 1;
    }

    /// Removes and returns the character at the given character index `at`.
    pub fn delete_char(&mut self, at: usize) -> Option<char> {
        if at >= self.len {
            return None;
        }
        let byte_idx = self.byte_index(at);
        let removed = self.text.remove(byte_idx);
        self.len -= 1;
        Some(removed)
    }

    /// Splits the row at character index `at`, retaining `[..at]` and returning `[at..]`.
    pub fn split(&mut self, at: usize) -> Row {
        let clamped = at.min(self.len);
        let byte_idx = self.byte_index(clamped);
        let remainder = self.text.split_off(byte_idx);
        let remainder_len = self.len - clamped;
        self.len = clamped;
        Row {
            text: remainder,
            len: remainder_len,
        }
    }

    /// Appends the contents of `other` onto the end of this row.
    pub fn append(&mut self, other: &Row) {
        self.text.push_str(&other.text);
        self.len += other.len;
    }

    /// Returns a UTF-8 character-safe horizontal slice of the row for viewport rendering.
    pub fn slice_chars(&self, col_offset: usize, max_cols: usize) -> String {
        self.text.chars().skip(col_offset).take(max_cols).collect()
    }
}

/// Holds the entirety of a text buffer.
#[derive(Default)]
pub struct Buffer {
    rows: Vec<Row>,
    pub filename: Option<String>,
    pub dirty: bool,
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
        }
    }

    /// Creates a buffer populated from a slice of string lines.
    #[allow(dead_code)]
    pub fn from_lines(lines: &[&str]) -> Self {
        Self {
            rows: lines.iter().copied().map(Row::from).collect(),
            filename: None,
            dirty: false,
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
                })
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                Ok(Self::with_filename(filename))
            }
            Err(err) => Err(err.into()),
        }
    }

    /// Inserts a character at buffer coordinates `(cx, cy)`.
    pub fn insert_char(&mut self, cx: usize, cy: usize, c: char) {
        while self.rows.len() <= cy {
            self.rows.push(Row::from(""));
        }
        self.rows[cy].insert_char(cx, c);
        self.dirty = true;
    }

    /// Splits the row at `(cx, cy)` or inserts a new empty line when the buffer is empty.
    pub fn insert_newline(&mut self, cx: usize, cy: usize) {
        if self.rows.is_empty() {
            self.rows.push(Row::from(""));
            self.rows.push(Row::from(""));
        } else if cy >= self.rows.len() {
            self.rows.push(Row::from(""));
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
            let target_cx = cx.min(self.rows[cy].len) - 1;
            self.rows[cy].delete_char(target_cx);
            self.dirty = true;
            Some((target_cx, cy))
        } else if cy > 0 {
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
            self.rows[cy].delete_char(cx);
            self.dirty = true;
            true
        } else if cy + 1 < self.rows.len() {
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
            .map(|row| row.text.as_str())
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
        assert!(row.text.len() > row.len);
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
        assert_eq!(row.text, "aé🦀b");
        assert_eq!(row.len, 4);

        row.insert_char(4, '!');
        assert_eq!(row.text, "aé🦀b!");
        assert_eq!(row.len, 5);

        assert_eq!(row.delete_char(2), Some('🦀'));
        assert_eq!(row.text, "aéb!");
        assert_eq!(row.len, 4);
        assert_eq!(row.delete_char(99), None);
    }

    #[test]
    fn row_split_and_append_preserve_utf8_and_len() {
        let mut row = Row::from("héllo🦀world");
        let right = row.split(6);
        assert_eq!(row.text, "héllo🦀");
        assert_eq!(row.len, 6);
        assert_eq!(right.text, "world");
        assert_eq!(right.len, 5);

        row.append(&right);
        assert_eq!(row.text, "héllo🦀world");
        assert_eq!(row.len, 11);
    }

    #[test]
    fn buffer_line_split_and_join_mutations() {
        let mut buf = Buffer::from_lines(&["hello world"]);
        buf.insert_newline(5, 0);
        assert_eq!(buf.len(), 2);
        assert_eq!(buf.get_row(0).unwrap().text, "hello");
        assert_eq!(buf.get_row(1).unwrap().text, " world");
        assert!(buf.dirty);

        let pos = buf.delete_char(0, 1);
        assert_eq!(pos, Some((5, 0)));
        assert_eq!(buf.len(), 1);
        assert_eq!(buf.get_row(0).unwrap().text, "hello world");
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
        assert_eq!(loaded.get_row(0).unwrap().text, "first 🦀 line");
        assert_eq!(loaded.get_row(1).unwrap().text, "second line");
        assert!(!loaded.dirty);

        let _ = fs::remove_file(&path);
    }
}
