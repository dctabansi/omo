use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

pub const DEFAULT_TAB_WIDTH: usize = 4;

pub const DEFAULT_CONFIG_TEMPLATE: &str = r#"# omo configuration file
# Global fallback settings for files not matched in [filetypes]
tab_width = 4
expand_tabs = true
auto_indent = true

[filetypes]
# Per-filetype tab_width and expand_tabs (true = spaces, false = literal '\t')
bash        = { tab_width = 4, expand_tabs = true }
c           = { tab_width = 4, expand_tabs = true }
cc          = { tab_width = 4, expand_tabs = true }
cjs         = { tab_width = 2, expand_tabs = true }
cpp         = { tab_width = 4, expand_tabs = true }
css         = { tab_width = 2, expand_tabs = true }
gnumakefile = { tab_width = 4, expand_tabs = false }
go          = { tab_width = 4, expand_tabs = false }
h           = { tab_width = 4, expand_tabs = true }
hpp         = { tab_width = 4, expand_tabs = true }
htm         = { tab_width = 2, expand_tabs = true }
html        = { tab_width = 2, expand_tabs = true }
java        = { tab_width = 4, expand_tabs = true }
js          = { tab_width = 2, expand_tabs = true }
json        = { tab_width = 2, expand_tabs = true }
jsonc       = { tab_width = 2, expand_tabs = true }
jsx         = { tab_width = 2, expand_tabs = true }
justfile    = { tab_width = 4, expand_tabs = true }
kt          = { tab_width = 4, expand_tabs = true }
kts         = { tab_width = 4, expand_tabs = true }
lua         = { tab_width = 2, expand_tabs = true }
makefile    = { tab_width = 4, expand_tabs = false }
markdown    = { tab_width = 2, expand_tabs = true }
md          = { tab_width = 2, expand_tabs = true }
mjs         = { tab_width = 2, expand_tabs = true }
mk          = { tab_width = 4, expand_tabs = false }
nix         = { tab_width = 2, expand_tabs = true }
py          = { tab_width = 4, expand_tabs = true }
rb          = { tab_width = 2, expand_tabs = true }
rs          = { tab_width = 4, expand_tabs = true }
scss        = { tab_width = 2, expand_tabs = true }
sh          = { tab_width = 4, expand_tabs = true }
svg         = { tab_width = 2, expand_tabs = true }
toml        = { tab_width = 2, expand_tabs = true }
ts          = { tab_width = 2, expand_tabs = true }
tsx         = { tab_width = 2, expand_tabs = true }
xml         = { tab_width = 2, expand_tabs = true }
yaml        = { tab_width = 2, expand_tabs = true }
yml         = { tab_width = 2, expand_tabs = true }
zsh         = { tab_width = 4, expand_tabs = true }

[keys]
"ctrl+q"           = "quit"
"alt+q"            = "quit"
"ctrl+s"           = "save"
"alt+s"            = "save"
"f2"               = "save"
"ctrl+z"           = "undo"
"alt+z"            = "undo"
"ctrl+y"           = "redo"
"alt+y"            = "redo"
"ctrl+shift+z"     = "redo"
"ctrl+d"           = "duplicate_line"
"alt+d"            = "duplicate_line"
"ctrl+shift+d"     = "delete_line"
"alt+shift+d"      = "delete_line"
"shift+enter"      = "insert_line_below"
"ctrl+enter"       = "insert_line_below"
"alt+enter"        = "insert_line_below"
"ctrl+j"           = "insert_line_below"
"ctrl+shift+enter" = "insert_line_above"
"ctrl+alt+enter"   = "insert_line_above"
"alt+shift+enter"  = "insert_line_above"
"shift+tab"        = "unindent"
"ctrl+left"        = "word_left"
"alt+left"         = "word_left"
"ctrl+right"       = "word_right"
"alt+right"        = "word_right"
"ctrl+backspace"   = "delete_word_left"
"alt+backspace"    = "delete_word_left"
"ctrl+w"           = "delete_word_left"
"ctrl+h"           = "delete_word_left"
"ctrl+delete"      = "delete_word_right"
"alt+delete"       = "delete_word_right"
"ctrl+g"           = "goto_line"
"alt+g"            = "goto_line"
"ctrl+f"           = "find"
"alt+f"            = "find"
"f3"               = "find"

[theme]
# Optional 24-bit RGB colors: [R, G, B]
# background = [30, 30, 46]
# foreground = [205, 214, 244]
# status_bg  = [49, 50, 68]
# status_fg  = [180, 190, 254]
"#;

fn default_tab_width() -> usize {
    DEFAULT_TAB_WIDTH
}

fn deserialize_tab_width<'de, D>(deserializer: D) -> Result<usize, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let raw = usize::deserialize(deserializer)?;
    Ok(if raw == 0 { DEFAULT_TAB_WIDTH } else { raw })
}

fn deserialize_optional_tab_width<'de, D>(deserializer: D) -> Result<Option<usize>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let raw = Option::<usize>::deserialize(deserializer)?;
    Ok(raw.map(|w| if w == 0 { DEFAULT_TAB_WIDTH } else { w }))
}

fn default_expand_tabs() -> bool {
    true
}

fn default_auto_indent() -> bool {
    true
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct FileTypeConfig {
    #[serde(
        default,
        deserialize_with = "deserialize_optional_tab_width",
        skip_serializing_if = "Option::is_none"
    )]
    pub tab_width: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expand_tabs: Option<bool>,
}

fn serialize_sorted_map<S, V>(map: &HashMap<String, V>, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
    V: Serialize,
{
    let ordered: BTreeMap<&String, &V> = map.iter().collect();
    ordered.serialize(serializer)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    #[serde(
        default = "default_tab_width",
        deserialize_with = "deserialize_tab_width"
    )]
    pub tab_width: usize,
    #[serde(default = "default_expand_tabs")]
    pub expand_tabs: bool,
    #[serde(default = "default_auto_indent")]
    pub auto_indent: bool,
    #[serde(
        default = "default_filetypes",
        deserialize_with = "deserialize_filetypes",
        serialize_with = "serialize_sorted_map"
    )]
    pub filetypes: HashMap<String, FileTypeConfig>,
    #[serde(
        default = "default_keys",
        deserialize_with = "deserialize_keys",
        serialize_with = "serialize_sorted_map"
    )]
    pub keys: HashMap<String, Action>,
    pub theme: ThemeConfig,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ThemeConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub background: Option<(u8, u8, u8)>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub foreground: Option<(u8, u8, u8)>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status_bg: Option<(u8, u8, u8)>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status_fg: Option<(u8, u8, u8)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    Quit,
    Save,
    Undo,
    Redo,
    Find,
    DuplicateLine,
    DeleteLine,
    InsertLineBelow,
    InsertLineAbove,
    Unindent,
    WordLeft,
    WordRight,
    DeleteWordLeft,
    DeleteWordRight,
    GotoLine,
}

pub fn default_filetypes() -> HashMap<String, FileTypeConfig> {
    let mut map = HashMap::new();
    let two_space = FileTypeConfig {
        tab_width: Some(2),
        expand_tabs: Some(true),
    };
    for ext in [
        "yaml", "yml", "json", "jsonc", "toml", "html", "htm", "xml", "svg", "css", "scss", "js",
        "mjs", "cjs", "jsx", "ts", "tsx", "rb", "lua", "nix", "md", "markdown",
    ] {
        map.insert(ext.to_string(), two_space);
    }

    let four_space = FileTypeConfig {
        tab_width: Some(4),
        expand_tabs: Some(true),
    };
    for ext in [
        "rs", "py", "java", "kt", "kts", "c", "h", "cpp", "hpp", "cc", "sh", "bash", "zsh",
        "justfile",
    ] {
        map.insert(ext.to_string(), four_space);
    }

    let hard_tab = FileTypeConfig {
        tab_width: Some(4),
        expand_tabs: Some(false),
    };
    for ext in ["makefile", "gnumakefile", "mk", "go"] {
        map.insert(ext.to_string(), hard_tab);
    }

    map
}

fn normalize_filetype_key(raw: &str) -> String {
    raw.trim().trim_start_matches('.').to_ascii_lowercase()
}

fn deserialize_filetypes<'de, D>(
    deserializer: D,
) -> Result<HashMap<String, FileTypeConfig>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let raw: HashMap<String, FileTypeConfig> = HashMap::deserialize(deserializer)?;
    let mut merged = default_filetypes();
    for (k, v) in raw {
        let key = normalize_filetype_key(&k);
        if key.is_empty() {
            continue;
        }
        let entry = merged.entry(key).or_default();
        if v.tab_width.is_some() {
            entry.tab_width = v.tab_width;
        }
        if v.expand_tabs.is_some() {
            entry.expand_tabs = v.expand_tabs;
        }
    }
    Ok(merged)
}

pub fn default_keys() -> HashMap<String, Action> {
    let mut keys = HashMap::new();
    keys.insert("ctrl+q".to_string(), Action::Quit);
    keys.insert("alt+q".to_string(), Action::Quit);
    keys.insert("ctrl+s".to_string(), Action::Save);
    keys.insert("alt+s".to_string(), Action::Save);
    keys.insert("f2".to_string(), Action::Save);
    keys.insert("ctrl+z".to_string(), Action::Undo);
    keys.insert("alt+z".to_string(), Action::Undo);
    keys.insert("ctrl+y".to_string(), Action::Redo);
    keys.insert("alt+y".to_string(), Action::Redo);
    keys.insert("ctrl+shift+z".to_string(), Action::Redo);
    keys.insert("ctrl+f".to_string(), Action::Find);
    keys.insert("alt+f".to_string(), Action::Find);
    keys.insert("f3".to_string(), Action::Find);
    keys.insert("ctrl+d".to_string(), Action::DuplicateLine);
    keys.insert("alt+d".to_string(), Action::DuplicateLine);
    keys.insert("ctrl+shift+d".to_string(), Action::DeleteLine);
    keys.insert("alt+shift+d".to_string(), Action::DeleteLine);
    keys.insert("shift+enter".to_string(), Action::InsertLineBelow);
    keys.insert("ctrl+enter".to_string(), Action::InsertLineBelow);
    keys.insert("alt+enter".to_string(), Action::InsertLineBelow);
    keys.insert("ctrl+j".to_string(), Action::InsertLineBelow);
    keys.insert("ctrl+shift+enter".to_string(), Action::InsertLineAbove);
    keys.insert("ctrl+alt+enter".to_string(), Action::InsertLineAbove);
    keys.insert("alt+shift+enter".to_string(), Action::InsertLineAbove);
    keys.insert("shift+tab".to_string(), Action::Unindent);
    keys.insert("ctrl+left".to_string(), Action::WordLeft);
    keys.insert("alt+left".to_string(), Action::WordLeft);
    keys.insert("ctrl+right".to_string(), Action::WordRight);
    keys.insert("alt+right".to_string(), Action::WordRight);
    keys.insert("ctrl+backspace".to_string(), Action::DeleteWordLeft);
    keys.insert("alt+backspace".to_string(), Action::DeleteWordLeft);
    keys.insert("ctrl+w".to_string(), Action::DeleteWordLeft);
    keys.insert("ctrl+h".to_string(), Action::DeleteWordLeft);
    keys.insert("ctrl+delete".to_string(), Action::DeleteWordRight);
    keys.insert("alt+delete".to_string(), Action::DeleteWordRight);
    keys.insert("ctrl+g".to_string(), Action::GotoLine);
    keys.insert("alt+g".to_string(), Action::GotoLine);
    keys
}

fn normalize_key_descriptor(raw: &str) -> String {
    let mut has_ctrl = false;
    let mut has_alt = false;
    let mut has_shift = false;
    let mut key_part = String::new();

    for token in raw.trim().split('+') {
        let lower = token.trim().to_ascii_lowercase();
        match lower.as_str() {
            "ctrl" | "control" => has_ctrl = true,
            "alt" => has_alt = true,
            "shift" => has_shift = true,
            other if !other.is_empty() => key_part = other.to_string(),
            _ => {}
        }
    }

    if key_part.is_empty() {
        return raw.trim().to_ascii_lowercase();
    }

    let mut parts = Vec::new();
    if has_ctrl {
        parts.push("ctrl".to_string());
    }
    if has_alt {
        parts.push("alt".to_string());
    }
    if has_shift {
        parts.push("shift".to_string());
    }
    parts.push(key_part);
    parts.join("+")
}

fn deserialize_keys<'de, D>(deserializer: D) -> Result<HashMap<String, Action>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let raw: HashMap<String, Action> = HashMap::deserialize(deserializer)?;
    let mut merged = default_keys();
    for (k, v) in raw {
        merged.insert(normalize_key_descriptor(&k), v);
    }
    Ok(merged)
}

impl Default for Config {
    fn default() -> Self {
        Self {
            tab_width: DEFAULT_TAB_WIDTH,
            expand_tabs: true,
            auto_indent: true,
            theme: ThemeConfig::default(),
            filetypes: default_filetypes(),
            keys: default_keys(),
        }
    }
}

impl Config {
    /// Parses a TOML configuration string, merging user keybindings and filetype settings with defaults.
    pub fn from_toml_str(contents: &str) -> Result<Self, toml::de::Error> {
        toml::from_str(contents)
    }

    /// Serializes this configuration into a formatted TOML string.
    pub fn to_toml_string(&self) -> Result<String, toml::ser::Error> {
        toml::to_string_pretty(self)
    }

    /// Returns the primary configuration file path where settings are saved/restored.
    pub fn primary_config_path() -> PathBuf {
        Self::config_paths()
            .into_iter()
            .next()
            .unwrap_or_else(|| PathBuf::from("config.toml"))
    }

    /// Resolves `(tab_width, expand_tabs)` for the given `filename`, checking `[filetypes]`
    /// by file extension and basename before falling back to top-level `tab_width` and `expand_tabs`.
    pub fn resolve_filetype_settings(&self, filename: Option<&str>) -> (usize, bool) {
        if let Some(name) = filename {
            let path = Path::new(name);
            let ext_key = path
                .extension()
                .and_then(|e| e.to_str())
                .map(normalize_filetype_key);
            let base_key = path
                .file_name()
                .and_then(|b| b.to_str())
                .map(normalize_filetype_key);

            let matched = ext_key
                .as_deref()
                .and_then(|k| self.filetypes.get(k))
                .or_else(|| base_key.as_deref().and_then(|k| self.filetypes.get(k)));

            if let Some(ft) = matched {
                let tw = ft.tab_width.unwrap_or(self.tab_width).max(1);
                let expand = ft.expand_tabs.unwrap_or(self.expand_tabs);
                return (tw, expand);
            }
        }
        (self.tab_width.max(1), self.expand_tabs)
    }

    /// Resolves candidate configuration file paths across Unix and Windows environments
    /// (`$OMO_CONFIG`, `$XDG_CONFIG_HOME/omo/config.toml`, `~/.config/omo/config.toml`,
    /// `%USERPROFILE%\.config\omo\config.toml`, and `%APPDATA%\omo\config.toml`).
    pub fn config_paths() -> Vec<PathBuf> {
        Self::config_paths_from_env(
            env::var("OMO_CONFIG").ok().as_deref(),
            env::var("XDG_CONFIG_HOME").ok().as_deref(),
            env::var("HOME").ok().as_deref(),
            env::var("USERPROFILE").ok().as_deref(),
            env::var("APPDATA").ok().as_deref(),
        )
    }

    fn config_paths_from_env(
        omo_config: Option<&str>,
        xdg: Option<&str>,
        home: Option<&str>,
        userprofile: Option<&str>,
        appdata: Option<&str>,
    ) -> Vec<PathBuf> {
        let mut paths = Vec::new();
        if let Some(custom) = omo_config.map(str::trim).filter(|s| !s.is_empty()) {
            paths.push(PathBuf::from(custom));
        }

        let mut has_posix_dir = false;
        if let Some(xdg) = xdg.map(str::trim).filter(|s| !s.is_empty()) {
            has_posix_dir = true;
            let candidate = PathBuf::from(xdg).join("omo").join("config.toml");
            if !paths.contains(&candidate) {
                paths.push(candidate);
            }
        }
        if let Some(home) = home.map(str::trim).filter(|s| !s.is_empty()) {
            has_posix_dir = true;
            let candidate = PathBuf::from(home)
                .join(".config")
                .join("omo")
                .join("config.toml");
            if !paths.contains(&candidate) {
                paths.push(candidate);
            }
        }
        // Only fall back to Windows environment variables when Linux/POSIX variables are unset
        if !has_posix_dir {
            if let Some(profile) = userprofile.map(str::trim).filter(|s| !s.is_empty()) {
                let candidate = PathBuf::from(profile)
                    .join(".config")
                    .join("omo")
                    .join("config.toml");
                if !paths.contains(&candidate) {
                    paths.push(candidate);
                }
            }
            if let Some(appdata) = appdata.map(str::trim).filter(|s| !s.is_empty()) {
                let candidate = PathBuf::from(appdata).join("omo").join("config.toml");
                if !paths.contains(&candidate) {
                    paths.push(candidate);
                }
            }
        }
        paths
    }

    /// Attempts to read the user's configuration from disk, falling back to `Config::default()`
    /// if missing or invalid.
    #[allow(dead_code)]
    pub fn load() -> Self {
        Self::load_with_warning().0
    }

    /// Attempts to read the user's configuration from disk, returning the active `Config`
    /// along with a warning message if a config file existed but failed to read or parse.
    pub fn load_with_warning() -> (Self, Option<String>) {
        let mut first_warning = None;
        for path in Self::config_paths() {
            match fs::read_to_string(&path) {
                Ok(contents) => match Self::from_toml_str(&contents) {
                    Ok(config) => return (config, None),
                    Err(err) => {
                        if first_warning.is_none() {
                            first_warning =
                                Some(format!("Config error in {}: {err}", path.display()));
                        }
                    }
                },
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
                Err(err) => {
                    if first_warning.is_none() {
                        first_warning =
                            Some(format!("Config read error in {}: {err}", path.display()));
                    }
                }
            }
        }
        (Self::default(), first_warning)
    }

    /// Resolves a `KeyEvent` into a configured `Action`, if bound.
    pub fn action_for_key(&self, key: KeyEvent) -> Option<Action> {
        let descriptor = format_key_event(key)?;
        self.keys.get(&descriptor).copied()
    }
}

/// Normalizes a `crossterm::event::KeyEvent` into a canonical lowercase key descriptor
/// such as `"ctrl+s"`, `"ctrl+shift+z"`, `"alt+f"`, or `"f3"`.
pub fn format_key_event(key: KeyEvent) -> Option<String> {
    let has_ctrl_or_alt = key
        .modifiers
        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT);
    let is_standalone_bindable = matches!(key.code, KeyCode::F(_) | KeyCode::Esc);
    let is_shifted_non_char =
        key.modifiers.contains(KeyModifiers::SHIFT) && !matches!(key.code, KeyCode::Char(_));

    if !has_ctrl_or_alt && !is_standalone_bindable && !is_shifted_non_char {
        return None;
    }

    let mut parts = Vec::new();
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        parts.push("ctrl".to_string());
    }
    if key.modifiers.contains(KeyModifiers::ALT) {
        parts.push("alt".to_string());
    }
    if key.modifiers.contains(KeyModifiers::SHIFT) {
        parts.push("shift".to_string());
    }

    let key_name = match key.code {
        KeyCode::Char(c) => c.to_ascii_lowercase().to_string(),
        KeyCode::Enter => "enter".to_string(),
        KeyCode::Tab => "tab".to_string(),
        KeyCode::BackTab => {
            if !parts.iter().any(|p| p == "shift") {
                parts.push("shift".to_string());
            }
            "tab".to_string()
        }
        KeyCode::Backspace => "backspace".to_string(),
        KeyCode::Delete => "delete".to_string(),
        KeyCode::Esc => "esc".to_string(),
        KeyCode::Up => "up".to_string(),
        KeyCode::Down => "down".to_string(),
        KeyCode::Left => "left".to_string(),
        KeyCode::Right => "right".to_string(),
        KeyCode::Home => "home".to_string(),
        KeyCode::End => "end".to_string(),
        KeyCode::PageUp => "pageup".to_string(),
        KeyCode::PageDown => "pagedown".to_string(),
        KeyCode::F(n) => format!("f{n}"),
        _ => return None,
    };

    parts.push(key_name);
    Some(parts.join("+"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_includes_standard_action_bindings() {
        let cfg = Config::default();
        assert_eq!(
            cfg.action_for_key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::CONTROL)),
            Some(Action::Quit)
        );
        assert_eq!(
            cfg.action_for_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL)),
            Some(Action::Save)
        );
        assert_eq!(
            cfg.action_for_key(KeyEvent::new(KeyCode::Char('z'), KeyModifiers::CONTROL)),
            Some(Action::Undo)
        );
        assert_eq!(
            cfg.action_for_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::CONTROL)),
            Some(Action::Redo)
        );
        assert_eq!(
            cfg.action_for_key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::CONTROL)),
            Some(Action::Find)
        );
        // Built-in fallbacks for IDE/host terminals
        assert_eq!(
            cfg.action_for_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::ALT)),
            Some(Action::Save)
        );
        assert_eq!(
            cfg.action_for_key(KeyEvent::new(KeyCode::F(2), KeyModifiers::NONE)),
            Some(Action::Save)
        );
        assert_eq!(
            cfg.action_for_key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::ALT)),
            Some(Action::Quit)
        );
        assert_eq!(
            cfg.action_for_key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::ALT)),
            Some(Action::Find)
        );
        assert_eq!(
            cfg.action_for_key(KeyEvent::new(KeyCode::F(3), KeyModifiers::NONE)),
            Some(Action::Find)
        );
        assert_eq!(
            cfg.action_for_key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL)),
            Some(Action::DuplicateLine)
        );
        assert_eq!(
            cfg.action_for_key(KeyEvent::new(
                KeyCode::Char('D'),
                KeyModifiers::CONTROL | KeyModifiers::SHIFT
            )),
            Some(Action::DeleteLine)
        );
        assert_eq!(
            cfg.action_for_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::SHIFT)),
            Some(Action::InsertLineBelow)
        );
        assert_eq!(
            cfg.action_for_key(KeyEvent::new(
                KeyCode::Enter,
                KeyModifiers::CONTROL | KeyModifiers::SHIFT
            )),
            Some(Action::InsertLineAbove)
        );
        assert_eq!(
            cfg.action_for_key(KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT)),
            Some(Action::Unindent)
        );
        assert_eq!(
            cfg.action_for_key(KeyEvent::new(KeyCode::Left, KeyModifiers::CONTROL)),
            Some(Action::WordLeft)
        );
        assert_eq!(
            cfg.action_for_key(KeyEvent::new(KeyCode::Right, KeyModifiers::CONTROL)),
            Some(Action::WordRight)
        );
        assert_eq!(
            cfg.action_for_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::CONTROL)),
            Some(Action::DeleteWordLeft)
        );
        assert_eq!(
            cfg.action_for_key(KeyEvent::new(KeyCode::Delete, KeyModifiers::CONTROL)),
            Some(Action::DeleteWordRight)
        );
        assert_eq!(
            cfg.action_for_key(KeyEvent::new(KeyCode::Char('g'), KeyModifiers::CONTROL)),
            Some(Action::GotoLine)
        );
    }

    #[test]
    fn toml_deserialization_parses_theme_and_merges_keys() {
        let toml_input = r#"
[theme]
background = [30, 30, 46]
foreground = [205, 214, 244]
status_bg = [49, 50, 68]
status_fg = [186, 194, 222]

[keys]
"ctrl+x" = "quit"
"alt+s" = "save"
"#;

        let cfg = Config::from_toml_str(toml_input).unwrap();
        assert_eq!(cfg.theme.background, Some((30, 30, 46)));
        assert_eq!(cfg.theme.foreground, Some((205, 214, 244)));
        assert_eq!(cfg.theme.status_bg, Some((49, 50, 68)));
        assert_eq!(cfg.theme.status_fg, Some((186, 194, 222)));

        assert_eq!(
            cfg.action_for_key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::CONTROL)),
            Some(Action::Quit)
        );
        assert_eq!(
            cfg.action_for_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::ALT)),
            Some(Action::Save)
        );
        // Unspecified defaults remain intact
        assert_eq!(
            cfg.action_for_key(KeyEvent::new(KeyCode::Char('z'), KeyModifiers::CONTROL)),
            Some(Action::Undo)
        );
    }

    #[test]
    fn invalid_toml_returns_err_allowing_default_fallback() {
        let bad_toml = "this is not = [valid toml";
        let parsed = Config::from_toml_str(bad_toml).unwrap_or_default();
        assert_eq!(parsed, Config::default());
    }

    #[test]
    fn format_key_event_supports_shift_modifier_and_standalone_function_keys() {
        let toml_input = r#"
[keys]
"ctrl+shift+z" = "redo"
"f3" = "find"
"#;
        let cfg = Config::from_toml_str(toml_input).unwrap();
        assert_eq!(
            cfg.action_for_key(KeyEvent::new(
                KeyCode::Char('Z'),
                KeyModifiers::CONTROL | KeyModifiers::SHIFT
            )),
            Some(Action::Redo)
        );
        assert_eq!(
            cfg.action_for_key(KeyEvent::new(KeyCode::Char('z'), KeyModifiers::CONTROL)),
            Some(Action::Undo)
        );
        assert_eq!(
            cfg.action_for_key(KeyEvent::new(KeyCode::F(3), KeyModifiers::NONE)),
            Some(Action::Find)
        );
    }

    #[test]
    fn config_paths_linux_xdg_and_home_variables_take_priority() {
        let paths = Config::config_paths_from_env(
            Some("/etc/omo/custom.toml"),
            Some("/home/alice/.config/custom"),
            Some("/home/alice"),
            Some(r"C:\Users\Alice"),
            Some(r"C:\Users\Alice\AppData\Roaming"),
        );
        assert_eq!(paths.len(), 3);
        assert_eq!(paths[0], PathBuf::from("/etc/omo/custom.toml"));
        assert_eq!(
            paths[1],
            PathBuf::from("/home/alice/.config/custom")
                .join("omo")
                .join("config.toml")
        );
        assert_eq!(
            paths[2],
            PathBuf::from("/home/alice")
                .join(".config")
                .join("omo")
                .join("config.toml")
        );
    }

    #[test]
    fn filetype_settings_resolve_built_in_defaults_and_user_overrides() {
        let toml_input = r#"
tab_width = 4
expand_tabs = true

[filetypes]
go = { expand_tabs = true }
yaml = { tab_width = 4 }
custom = { tab_width = 8, expand_tabs = false }
"#;
        let cfg = Config::from_toml_str(toml_input).unwrap();
        // Built-in defaults
        assert_eq!(
            cfg.resolve_filetype_settings(Some("config.json")),
            (2, true)
        );
        assert_eq!(cfg.resolve_filetype_settings(Some("Makefile")), (4, false));
        assert_eq!(
            cfg.resolve_filetype_settings(Some("src/main.rs")),
            (4, true)
        );
        // User overrides merged with built-in defaults
        assert_eq!(cfg.resolve_filetype_settings(Some("main.go")), (4, true));
        assert_eq!(
            cfg.resolve_filetype_settings(Some("docker-compose.yaml")),
            (4, true)
        );
        assert_eq!(
            cfg.resolve_filetype_settings(Some("data.custom")),
            (8, false)
        );
        // Default template parses cleanly
        let template_cfg = Config::from_toml_str(DEFAULT_CONFIG_TEMPLATE).unwrap();
        assert_eq!(template_cfg, Config::default());
    }

    #[test]
    fn config_paths_windows_userprofile_and_appdata_fallback() {
        let paths = Config::config_paths_from_env(
            None,
            None,
            None,
            Some(r"C:\Users\Alice"),
            Some(r"C:\Users\Alice\AppData\Roaming"),
        );
        assert_eq!(paths.len(), 2);
        assert_eq!(
            paths[0],
            PathBuf::from(r"C:\Users\Alice")
                .join(".config")
                .join("omo")
                .join("config.toml")
        );
        assert_eq!(
            paths[1],
            PathBuf::from(r"C:\Users\Alice\AppData\Roaming")
                .join("omo")
                .join("config.toml")
        );
    }
}
