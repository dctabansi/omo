# omo — One Mode Only

**omo** ("One Mode Only") is a lightweight, strictly amodal terminal text editor written in Rust.

Unlike modal editors that split input across normal, insert, and visual states, `omo` drops you straight into text editing: printable keys immediately insert characters, and commands are triggered with standard modifier keys (`Ctrl` / `Alt`).

## Features

- **Strictly Amodal:** Start typing immediately — no mode switching required.
- **Fast & Lightweight:** Hand-rolled batched ANSI rendering over `crossterm` with zero TUI framework overhead and zero screen flicker.
- **Line-Level Gap Buffer:** $O(1)$ amortized character insertions and deletions with full multi-byte UTF-8 safety and visual tab-stop expansion.
- **Essential Editing Controls:** Built-in undo/redo history, incremental forward/backward search, unnamed buffer save prompts, and dirty-buffer quit protection.
- **Declarative Configuration:** Optional `~/.config/omo/config.toml` for custom keybindings, tab width, and 24-bit RGB theme colors, with zero-config defaults out of the box.
- **Crash-Safe Terminal State:** RAII terminal guard restores raw mode, alternate screen, cursor visibility, and colors cleanly on exit or panic.

## Installation

Install from [crates.io](https://crates.io/crates/omo):

```bash
cargo install omo
```

Or build and install from source:

```bash
git clone https://github.com/dctabansi/omo.git
cd omo
cargo install --path .
```

## Usage

Open an existing file (or specify a new file path to create on save):

```bash
omo path/to/file.txt
```

Start with an empty buffer:

```bash
omo
```

### Use as Default Git Editor

```bash
git config --global core.editor "omo"
```

### Use as Default System Editor

Add to your `~/.bashrc` or `~/.zshrc`:

```bash
export EDITOR="omo"
export VISUAL="omo"
```

## Default Keybindings

| Key | Action |
| :--- | :--- |
| `Up` / `Down` / `Left` / `Right` | Move cursor |
| `Home` / `End` | Jump to start / end of line |
| `PageUp` / `PageDown` | Scroll viewport up / down by page |
| `Ctrl+S` / `Alt+S` / `F2` | Save file (prompts for filename if unnamed) |
| `Ctrl+Q` / `Alt+Q` | Quit (press twice to discard unsaved changes) |
| `Ctrl+Z` / `Alt+Z` | Undo |
| `Ctrl+Y` / `Alt+Y` / `Ctrl+Shift+Z` | Redo |
| `Ctrl+F` / `Alt+F` / `F3` | Incremental search (`Enter` accept, `Up`/`Down` prev/next, `Alt+C` toggle case, `Esc` cancel) |

## Configuration

`omo` requires no configuration to run. To customize settings, create `~/.config/omo/config.toml` (or `$XDG_CONFIG_HOME/omo/config.toml`, or set `$OMO_CONFIG` to a custom file path):

```toml
tab_width = 4

[theme]
background = [30, 30, 46]
foreground = [205, 214, 244]
status_bg  = [49, 50, 68]
status_fg  = [180, 190, 254]

[keys]
"ctrl+q" = "quit"
"ctrl+s" = "save"
"ctrl+z" = "undo"
"ctrl+y" = "redo"
"ctrl+f" = "find"
```

### Supported Key Actions

- `"quit"` — Exit the editor (with unsaved-change protection)
- `"save"` — Save the current buffer
- `"undo"` — Undo the last edit
- `"redo"` — Redo the last undone edit
- `"find"` — Open the interactive search prompt

## License

Licensed under either the MIT License or the Apache License, Version 2.0, at your option:

- [MIT License](LICENSE-MIT)
- [Apache License, Version 2.0](LICENSE-APACHE)
