mod buffer;
mod config;
mod editor;
mod terminal;

use anyhow::Result;
use buffer::Buffer;
use config::Config;
use editor::Editor;
use std::env;
use terminal::Terminal;

fn main() -> Result<()> {
    let config = Config::load();
    let args: Vec<String> = env::args().collect();

    let buffer = if args.len() > 1 {
        Buffer::open(&args[1]).unwrap_or_else(|_| Buffer::with_filename(&args[1]))
    } else {
        Buffer::new()
    };

    let _term = Terminal::initialize()?;

    let mut editor = Editor::with_config(buffer, config);
    editor.run()?;

    Ok(())
}
