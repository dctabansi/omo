# omo — One Mode Only

**omo** ("One Mode Only") is a lightweight, strictly amodal terminal text editor written in Rust.

Unlike modal editors that split input across normal, insert, and visual states, `omo` drops you straight into text editing: printable keys immediately insert characters, and commands are triggered with standard modifier keys (`Ctrl` / `Alt`).

## Features

- **Strictly Amodal:** Start typing immediately — no mode switching required.
- **Fast & Lightweight:** Hand-rolled batched ANSI rendering over `crossterm` with zero TUI framework overhead and zero screen flicker.
- **Line-Level Gap Buffer:** $O(1)$ amortized character insertions and deletions with full multi-byte UTF-8 safety and visual tab-stop expansion.
- **Essential Editing Controls:** Built-in undo/redo history, word navigation & deletion, line duplication/deletion, smart auto-indentation, go-to-line, case-insensitive/case-sensitive incremental search, unnamed buffer save prompts, and dirty-buffer quit protection.
- **Declarative Configuration:** Optional `~/.config/omo/config.toml` (or `$XDG_CONFIG_HOME/omo/config.toml` / `$OMO_CONFIG`) for custom keybindings, tab width, auto-indent, and 24-bit RGB theme colors, with zero-config defaults out of the box.
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

Check version or usage help:

```bash
omo --version   # or: omo -v
omo --help      # or: omo -h
```

Manage configuration from the CLI:

```bash
omo --config                  # or: omo -c (opens config.toml in omo, creating it with defaults if missing)
omo --default                 # or: omo -d (restores default config.toml after confirmation)
omo --export [FILE.toml]      # or: omo -e (exports active config to FILE.toml or ./omo-config.toml)
omo --import <FILE.toml>      # or: omo -i (validates and imports FILE.toml after confirmation)
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
| `Ctrl+Left` / `Ctrl+Right` (`Alt+Left` / `Alt+Right`) | Jump cursor by word |
| `Home` / `End` | Smart jump to first non-whitespace / start / end of line |
| `PageUp` / `PageDown` | Scroll viewport up / down by page |
| `Enter` | Split line with smart auto-indentation |
| `Shift+Tab` | Unindent current line by one tab stop |
| `Ctrl+Backspace` / `Ctrl+Delete` (`Alt+Backspace` / `Alt+Delete` / `Ctrl+W`) | Delete previous / next word |
| `Ctrl+S` / `Alt+S` / `F2` | Save file (prompts for filename if unnamed) |
| `Ctrl+Q` / `Alt+Q` | Quit (press twice to discard unsaved changes) |
| `Ctrl+Z` / `Alt+Z` | Undo |
| `Ctrl+Y` / `Alt+Y` / `Ctrl+Shift+Z` | Redo |
| `Ctrl+D` / `Alt+D` | Duplicate current line |
| `Ctrl+Shift+D` / `Alt+Shift+D` | Delete current line |
| `Shift+Enter` / `Alt+Enter` / `Ctrl+Enter` | Insert new line below at current indentation |
| `Ctrl+Shift+Enter` / `Ctrl+Alt+Enter` / `Alt+Shift+Enter` | Insert new line above at current indentation |
| `Ctrl+G` / `Alt+G` | Go to line number |
| `Ctrl+F` / `Alt+F` / `F3` | Incremental search (`Enter` accept, `Up`/`Down` prev/next, `Alt+C` toggle case, `Esc` cancel) |

## Configuration

`omo` requires no configuration to run. To customize settings, create `~/.config/omo/config.toml` (or `$XDG_CONFIG_HOME/omo/config.toml`, or set `$OMO_CONFIG` to a custom file path):

```toml
tab_width = 4
expand_tabs = true
auto_indent = true

[filetypes]
go       = { tab_width = 4, expand_tabs = false }
js       = { tab_width = 2, expand_tabs = true }
json     = { tab_width = 2, expand_tabs = true }
makefile = { tab_width = 4, expand_tabs = false }
py       = { tab_width = 4, expand_tabs = true }
rs       = { tab_width = 4, expand_tabs = true }
toml     = { tab_width = 2, expand_tabs = true }
ts       = { tab_width = 2, expand_tabs = true }
yaml     = { tab_width = 2, expand_tabs = true }

[keys]
"ctrl+q" = "quit"
"ctrl+s" = "save"
"ctrl+z" = "undo"
"ctrl+y" = "redo"
"ctrl+d" = "duplicate_line"
"ctrl+shift+d" = "delete_line"
"shift+enter" = "insert_line_below"
"ctrl+shift+enter" = "insert_line_above"
"shift+tab" = "unindent"
"ctrl+left" = "word_left"
"ctrl+right" = "word_right"
"ctrl+backspace" = "delete_word_left"
"ctrl+delete" = "delete_word_right"
"ctrl+g" = "goto_line"
"ctrl+f" = "find"

[theme]
background = [30, 30, 46]
foreground = [205, 214, 244]
status_bg  = [49, 50, 68]
status_fg  = [180, 190, 254]
```

### Supported Key Actions

- `"quit"` — Exit the editor (with unsaved-change protection)
- `"save"` — Save the current buffer
- `"undo"` — Undo the last edit
- `"redo"` — Redo the last undone edit
- `"duplicate_line"` — Duplicate the current line below
- `"delete_line"` — Delete the entire current line
- `"insert_line_below"` — Insert an indented new line below without splitting the current line
- `"insert_line_above"` — Insert an indented new line above without splitting the current line
- `"unindent"` — Remove one tab stop of leading indentation from the current line
- `"word_left"` — Move the cursor to the previous word boundary
- `"word_right"` — Move the cursor to the next word boundary
- `"delete_word_left"` — Delete from the cursor to the previous word boundary
- `"delete_word_right"` — Delete from the cursor to the next word boundary
- `"goto_line"` — Open the interactive Go-to-Line prompt
- `"find"` — Open the interactive search prompt

## License

Licensed under either of:

- Apache License, Version 2.0 ([LICENSE-APACHE](http://www.apache.org/licenses/LICENSE-2.0))
- MIT license ([LICENSE-MIT](http://opensource.org/licenses/MIT))

at your option.
