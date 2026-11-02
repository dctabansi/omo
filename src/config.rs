use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use serde::Deserialize;
use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::PathBuf;

pub const DEFAULT_TAB_WIDTH: usize = 4;

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

fn default_auto_indent() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct Config {
    #[serde(
        default = "default_tab_width",
        deserialize_with = "deserialize_tab_width"
    )]
    pub tab_width: usize,
    #[serde(default = "default_auto_indent")]
    pub auto_indent: bool,
    pub theme: ThemeConfig,
    #[serde(deserialize_with = "deserialize_keys")]
    pub keys: HashMap<String, Action>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct ThemeConfig {
    pub background: Option<(u8, u8, u8)>,
    pub foreground: Option<(u8, u8, u8)>,
    pub status_bg: Option<(u8, u8, u8)>,
    pub status_fg: Option<(u8, u8, u8)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
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
            auto_indent: true,
            theme: ThemeConfig::default(),
            keys: default_keys(),
        }
    }
}

impl Config {
    /// Parses a TOML configuration string, merging user keybindings with defaults.
    pub fn from_toml_str(contents: &str) -> Result<Self, toml::de::Error> {
        toml::from_str(contents)
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
