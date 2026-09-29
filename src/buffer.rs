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
    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }
}
