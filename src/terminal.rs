use anyhow::Result;
use crossterm::{
    cursor,
    event::{KeyboardEnhancementFlags, PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags},
    execute,
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
        let term = Self;
        execute!(stdout(), EnterAlternateScreen, Clear(ClearType::All))?;
        let _ = execute!(
            stdout(),
            PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES),
            crossterm::style::Print("\x1b[>4;1m")
        );
        Ok(term)
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

    /// Temporarily restores the terminal to normal cooked mode on the main screen to run an
    /// interactive subprocess (such as `sudo -v`), then re-enters raw mode and the alternate screen.
    pub fn with_suspended<F, R>(f: F) -> Result<R>
    where
        F: FnOnce() -> Result<R>,
    {
        let _ = execute!(
            stdout(),
            crossterm::style::Print("\x1b[>4;0m"),
            PopKeyboardEnhancementFlags
        );
        let _ = execute!(stdout(), ResetColor, cursor::Show, LeaveAlternateScreen);
        let _ = disable_raw_mode();

        let result = f();

        enable_raw_mode()?;
        let _ = execute!(stdout(), EnterAlternateScreen, Clear(ClearType::All));
        let _ = execute!(
            stdout(),
            PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES),
            crossterm::style::Print("\x1b[>4;1m")
        );
        result
    }
}

/// Restores the terminal to its normal state, guaranteeing cleanup even if the program panics.
impl Drop for Terminal {
    fn drop(&mut self) {
        let _ = execute!(
            stdout(),
            crossterm::style::Print("\x1b[>4;0m"),
            PopKeyboardEnhancementFlags
        );
        let _ = execute!(stdout(), ResetColor, cursor::Show, LeaveAlternateScreen);
        let _ = disable_raw_mode();
    }
}
