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

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct Config {
    #[serde(
        default = "default_tab_width",
        deserialize_with = "deserialize_tab_width"
    )]
    pub tab_width: usize,
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
}

pub fn default_keys() -> HashMap<String, Action> {
    let mut keys = HashMap::new();
    keys.insert("ctrl+q".to_string(), Action::Quit);
    keys.insert("ctrl+s".to_string(), Action::Save);
    keys.insert("ctrl+z".to_string(), Action::Undo);
    keys.insert("ctrl+y".to_string(), Action::Redo);
    keys.insert("ctrl+f".to_string(), Action::Find);
    keys
}

fn deserialize_keys<'de, D>(deserializer: D) -> Result<HashMap<String, Action>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let raw: HashMap<String, Action> = HashMap::deserialize(deserializer)?;
    let mut merged = default_keys();
    for (k, v) in raw {
        merged.insert(k.trim().to_ascii_lowercase(), v);
    }
    Ok(merged)
}

impl Default for Config {
    fn default() -> Self {
        Self {
            tab_width: DEFAULT_TAB_WIDTH,
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

    /// Resolves candidate configuration file paths on Linux (`$XDG_CONFIG_HOME/omo/config.toml`
    /// and `~/.config/omo/config.toml`).
    pub fn config_paths() -> Vec<PathBuf> {
        let mut paths = Vec::new();
        if let Ok(xdg) = env::var("XDG_CONFIG_HOME")
            && !xdg.trim().is_empty()
        {
            paths.push(PathBuf::from(xdg).join("omo").join("config.toml"));
        }
        if let Ok(home) = env::var("HOME")
            && !home.trim().is_empty()
        {
            paths.push(
                PathBuf::from(home)
                    .join(".config")
                    .join("omo")
                    .join("config.toml"),
            );
        }
        paths
    }

    /// Attempts to read the user's configuration from disk, falling back to `Config::default()`
    /// if missing or invalid.
    pub fn load() -> Self {
        for path in Self::config_paths() {
            if let Ok(contents) = fs::read_to_string(&path)
                && let Ok(config) = Self::from_toml_str(&contents)
            {
                return config;
            }
        }
        Self::default()
    }

    /// Resolves a `KeyEvent` into a configured `Action`, if bound.
    pub fn action_for_key(&self, key: KeyEvent) -> Option<Action> {
        let descriptor = format_key_event(key)?;
        self.keys.get(&descriptor).copied()
    }
}

/// Normalizes a `crossterm::event::KeyEvent` into a canonical lowercase key descriptor
/// such as `"ctrl+s"` or `"alt+f"`.
pub fn format_key_event(key: KeyEvent) -> Option<String> {
    if !key
        .modifiers
        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
    {
        return None;
    }

    let mut parts = Vec::new();
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        parts.push("ctrl".to_string());
    }
    if key.modifiers.contains(KeyModifiers::ALT) {
        parts.push("alt".to_string());
    }

    let key_name = match key.code {
        KeyCode::Char(c) => c.to_ascii_lowercase().to_string(),
        KeyCode::Enter => "enter".to_string(),
        KeyCode::Tab => "tab".to_string(),
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
}
