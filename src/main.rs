mod buffer;
mod editor;
mod terminal;

use anyhow::Result;
use buffer::Buffer;
use editor::Editor;
use std::env;
use terminal::Terminal;

fn main() -> Result<()> {
    let args: Vec<String> = env::args().collect();

    let buffer = if args.len() > 1 {
        Buffer::open(&args[1]).unwrap_or_else(|_| Buffer::new())
    } else {
        Buffer::new()
    };

    let _term = Terminal::initialize()?;

    let mut editor = Editor::new(buffer);
    editor.run()?;

    Ok(())
}
