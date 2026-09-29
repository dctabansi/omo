use anyhow::Result;
use crossterm::{
    cursor, execute,
    style::ResetColor,
    terminal::{
        Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode,
        enable_raw_mode,
    },
};
use std::io::stdout;

/// RAII Guard for the terminal state.
/// While alive, the terminal is in raw mode on the alternate screen.
pub struct Terminal;

impl Terminal {
    /// Takes over the terminal: enters raw mode and switches to the alternate screen.
    pub fn initialize() -> Result<Self> {
        enable_raw_mode()?;
        execute!(stdout(), EnterAlternateScreen, Clear(ClearType::All))?;
        Ok(Self)
    }

    /// Alias for [`Terminal::initialize`].
    #[allow(dead_code)]
    pub fn init() -> Result<Self> {
        Self::initialize()
    }

    /// Returns the current terminal dimensions as (columns, rows).
    pub fn size() -> Result<(u16, u16)> {
        let size = crossterm::terminal::size()?;
        Ok(size)
    }
}

/// Restores the terminal to its normal state, guaranteeing cleanup even if the program panics.
impl Drop for Terminal {
    fn drop(&mut self) {
        let _ = execute!(stdout(), ResetColor, cursor::Show, LeaveAlternateScreen);
        let _ = disable_raw_mode();
    }
}
