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
    /// Returns a UTF-8 character-safe horizontal slice of the row for viewport rendering.
    pub fn slice_chars(&self, col_offset: usize, max_cols: usize) -> String {
        self.text.chars().skip(col_offset).take(max_cols).collect()
    }
}

/// Holds the entirety of a text buffer.
#[derive(Default)]
pub struct Buffer {
    rows: Vec<Row>,
}

impl Buffer {
    /// Creates a new empty buffer.
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates a buffer populated from a slice of string lines.
    #[allow(dead_code)]
    pub fn from_lines(lines: &[&str]) -> Self {
        Self {
            rows: lines.iter().copied().map(Row::from).collect(),
        }
    }

    /// Loads a buffer from a file path.
    pub fn open(filename: &str) -> Result<Self> {
        let contents = fs::read_to_string(filename)?;
        let mut rows = Vec::new();

        for line in contents.lines() {
            rows.push(Row::from(line));
        }

        Ok(Self { rows })
    }

    /// Returns a reference to the row at the given index if it exists.
    pub fn get_row(&self, index: usize) -> Option<&Row> {
        self.rows.get(index)
    }

    /// Returns the number of rows in the buffer.
    #[allow(dead_code)]
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
}
