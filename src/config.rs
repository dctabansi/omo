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
auto_pairs = true

[filetypes]
# Per-filetype tab_width, expand_tabs (true = spaces, false = literal '\t'), and auto_pairs
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
"alt+up"           = "move_line_up"
"alt+down"         = "move_line_down"
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

fn default_auto_pairs() -> bool {
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_pairs: Option<bool>,
}

fn serialize_sorted_map<S, V>(map: &HashMap<String, V>, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
    V: Serialize,
{
    let ordered: BTreeMap<&String, &V> = map.iter().collect();
    ordered.serialize(serializer)
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
enum RawKeyBinding {
    Single(String),
    Multiple(Vec<Action>),
}

fn parse_action_name(name: &str) -> Result<Action, String> {
    let quoted = format!("\"{}\"", name.trim());
    toml::from_str::<Action>(&format!("action = {quoted}"))
        .or_else(|_| {
            #[derive(Deserialize)]
            struct Wrapper {
                action: Action,
            }
            toml::from_str::<Wrapper>(&format!("action = {quoted}")).map(|w| w.action)
        })
        .map_err(|_| format!("unknown action '{}'", name.trim()))
}

fn serialize_keys<S>(map: &HashMap<String, Vec<Action>>, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    use serde::ser::SerializeMap;
    let ordered: BTreeMap<&String, &Vec<Action>> = map.iter().collect();
    let mut ser_map = serializer.serialize_map(Some(ordered.len()))?;
    for (k, actions) in ordered {
        if actions.len() == 1 {
            ser_map.serialize_entry(k, &actions[0])?;
        } else {
            ser_map.serialize_entry(k, actions)?;
        }
    }
    ser_map.end()
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
    #[serde(default = "default_auto_pairs")]
    pub auto_pairs: bool,
    #[serde(
        default = "default_filetypes",
        deserialize_with = "deserialize_filetypes",
        serialize_with = "serialize_sorted_map"
    )]
    pub filetypes: HashMap<String, FileTypeConfig>,
    #[serde(
        default = "default_keys",
        deserialize_with = "deserialize_keys",
        serialize_with = "serialize_keys"
    )]
    pub keys: HashMap<String, Vec<Action>>,
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
    MoveLineUp,
    MoveLineDown,
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
        auto_pairs: None,
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
        auto_pairs: None,
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
        auto_pairs: None,
    };
    for ext in ["makefile", "gnumakefile", "mk", "go"] {
        map.insert(ext.to_string(), hard_tab);
    }

    map
}

fn normalize_filetype_key(raw: &str) -> String {
    let trimmed = raw.trim().to_ascii_lowercase();
    if let Some(rest) = trimmed.strip_prefix('.')
        && !rest.is_empty()
        && !rest.contains('.')
        && !rest.contains('*')
        && !rest.contains('?')
    {
        return rest.to_string();
    }
    trimmed
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
        if v.auto_pairs.is_some() {
            entry.auto_pairs = v.auto_pairs;
        }
    }
    Ok(merged)
}

pub fn default_keys() -> HashMap<String, Vec<Action>> {
    let mut keys = HashMap::new();
    let mut bind = |k: &str, a: Action| {
        keys.insert(k.to_string(), vec![a]);
    };
    bind("ctrl+q", Action::Quit);
    bind("alt+q", Action::Quit);
    bind("ctrl+s", Action::Save);
    bind("alt+s", Action::Save);
    bind("f2", Action::Save);
    bind("ctrl+z", Action::Undo);
    bind("alt+z", Action::Undo);
    bind("ctrl+y", Action::Redo);
    bind("alt+y", Action::Redo);
    bind("ctrl+shift+z", Action::Redo);
    bind("ctrl+f", Action::Find);
    bind("alt+f", Action::Find);
    bind("f3", Action::Find);
    bind("ctrl+d", Action::DuplicateLine);
    bind("alt+d", Action::DuplicateLine);
    bind("ctrl+shift+d", Action::DeleteLine);
    bind("alt+shift+d", Action::DeleteLine);
    bind("alt+up", Action::MoveLineUp);
    bind("alt+down", Action::MoveLineDown);
    bind("shift+enter", Action::InsertLineBelow);
    bind("ctrl+enter", Action::InsertLineBelow);
    bind("alt+enter", Action::InsertLineBelow);
    bind("ctrl+j", Action::InsertLineBelow);
    bind("ctrl+shift+enter", Action::InsertLineAbove);
    bind("ctrl+alt+enter", Action::InsertLineAbove);
    bind("alt+shift+enter", Action::InsertLineAbove);
    bind("shift+tab", Action::Unindent);
    bind("ctrl+left", Action::WordLeft);
    bind("alt+left", Action::WordLeft);
    bind("ctrl+right", Action::WordRight);
    bind("alt+right", Action::WordRight);
    bind("ctrl+backspace", Action::DeleteWordLeft);
    bind("alt+backspace", Action::DeleteWordLeft);
    bind("ctrl+w", Action::DeleteWordLeft);
    bind("ctrl+h", Action::DeleteWordLeft);
    bind("ctrl+delete", Action::DeleteWordRight);
    bind("alt+delete", Action::DeleteWordRight);
    bind("ctrl+g", Action::GotoLine);
    bind("alt+g", Action::GotoLine);
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

fn deserialize_keys<'de, D>(deserializer: D) -> Result<HashMap<String, Vec<Action>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let raw: HashMap<String, RawKeyBinding> = HashMap::deserialize(deserializer)?;
    let mut merged = default_keys();
    for (k, binding) in raw {
        let actions = match binding {
            RawKeyBinding::Single(s) => {
                let mut list = Vec::new();
                for part in s.split(',') {
                    let action = parse_action_name(part).map_err(serde::de::Error::custom)?;
                    list.push(action);
                }
                if list.is_empty() {
                    return Err(serde::de::Error::custom(format!(
                        "keybinding for '{k}' must contain at least one action"
                    )));
                }
                list
            }
            RawKeyBinding::Multiple(list) => {
                if list.is_empty() {
                    return Err(serde::de::Error::custom(format!(
                        "keybinding array for '{k}' cannot be empty"
                    )));
                }
                list
            }
        };
        merged.insert(normalize_key_descriptor(&k), actions);
    }
    Ok(merged)
}

/// Matches a glob pattern supporting `*` (zero or more characters) and `?` (any single character).
pub fn matches_glob(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let t: Vec<char> = text.chars().collect();
    let (mut pi, mut ti) = (0, 0);
    let mut star_idx: Option<usize> = None;
    let mut match_idx = 0;

    while ti < t.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == t[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star_idx = Some(pi);
            match_idx = ti;
            pi += 1;
        } else if let Some(star) = star_idx {
            pi = star + 1;
            match_idx += 1;
            ti = match_idx;
        } else {
            return false;
        }
    }

    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }

    pi == p.len()
}

impl Default for Config {
    fn default() -> Self {
        Self {
            tab_width: DEFAULT_TAB_WIDTH,
            expand_tabs: true,
            auto_indent: true,
            auto_pairs: true,
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

    /// Resolves `(tab_width, expand_tabs, auto_pairs)` for the given `filename` using strict
    /// precedence:
    /// 1. Exact basename match (e.g., `tmux.conf`, `Cargo.lock`, `Makefile`)
    /// 2. Glob pattern match (`*` / `?`, e.g., `*.conf`, `.env*`, `*.env.*`)
    /// 3. File extension match (e.g., `rs`, `toml`, `py`)
    /// 4. Global fallback (`tab_width`, `expand_tabs`, `auto_pairs`)
    pub fn resolve_filetype_config(&self, filename: Option<&str>) -> (usize, bool, bool) {
        if let Some(name) = filename {
            let path = Path::new(name);
            let full_lower = name.replace('\\', "/").to_ascii_lowercase();
            let base_lower = path
                .file_name()
                .and_then(|b| b.to_str())
                .map(|s| s.to_ascii_lowercase());
            let ext_key = path
                .extension()
                .and_then(|e| e.to_str())
                .map(normalize_filetype_key);

            // 1. Exact basename match (checked before globs and extensions so dotted filenames like
            // `tmux.conf` or `Cargo.lock` are never shadowed by `.conf` or `.lock` extension rules)
            let exact_match = base_lower.as_deref().and_then(|base| {
                self.filetypes.get(base).or_else(|| {
                    let stripped = base.trim_start_matches('.');
                    if !stripped.is_empty() && !stripped.contains('.') {
                        self.filetypes.get(stripped)
                    } else {
                        None
                    }
                })
            });

            // 2. Glob pattern match (keys containing `*` or `?`, ordered most-specific first)
            let glob_match = if exact_match.is_none() {
                let mut glob_entries: Vec<(&String, &FileTypeConfig)> = self
                    .filetypes
                    .iter()
                    .filter(|(k, _)| k.contains('*') || k.contains('?'))
                    .collect();
                glob_entries.sort_by(|(a, _), (b, _)| {
                    let a_lit = a.chars().filter(|&c| c != '*' && c != '?').count();
                    let b_lit = b.chars().filter(|&c| c != '*' && c != '?').count();
                    b_lit
                        .cmp(&a_lit)
                        .then_with(|| b.len().cmp(&a.len()))
                        .then_with(|| a.cmp(b))
                });
                glob_entries
                    .into_iter()
                    .find(|(pattern, _)| {
                        base_lower
                            .as_deref()
                            .is_some_and(|base| matches_glob(pattern, base))
                            || matches_glob(pattern, &full_lower)
                    })
                    .map(|(_, cfg)| cfg)
            } else {
                None
            };

            // 3. Extension match
            let ext_match = if exact_match.is_none() && glob_match.is_none() {
                ext_key.as_deref().and_then(|k| self.filetypes.get(k))
            } else {
                None
            };

            if let Some(ft) = exact_match.or(glob_match).or(ext_match) {
                let tw = ft.tab_width.unwrap_or(self.tab_width).max(1);
                let expand = ft.expand_tabs.unwrap_or(self.expand_tabs);
                let pairs = ft.auto_pairs.unwrap_or(self.auto_pairs);
                return (tw, expand, pairs);
            }
        }
        (self.tab_width.max(1), self.expand_tabs, self.auto_pairs)
    }

    /// Resolves `(tab_width, expand_tabs)` for the given `filename`.
    pub fn resolve_filetype_settings(&self, filename: Option<&str>) -> (usize, bool) {
        let (tw, expand, _) = self.resolve_filetype_config(filename);
        (tw, expand)
    }

    /// Resolves whether `auto_pairs` is enabled for the given `filename`.
    pub fn resolve_auto_pairs(&self, filename: Option<&str>) -> bool {
        self.resolve_filetype_config(filename).2
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

    /// Resolves a `KeyEvent` into a configured slice of `Action`s, if bound.
    pub fn actions_for_key(&self, key: KeyEvent) -> Option<&[Action]> {
        let descriptor = format_key_event(key)?;
        self.keys.get(&descriptor).map(Vec::as_slice)
    }

    /// Resolves a `KeyEvent` into the first configured `Action`, if bound.
    pub fn action_for_key(&self, key: KeyEvent) -> Option<Action> {
        self.actions_for_key(key)
            .and_then(|actions| actions.first().copied())
    }
}

fn split_toml_key_value_and_comment(line: &str) -> Option<(&str, &str, &str)> {
    let trimmed = line.trim_start();
    if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with('[') {
        return None;
    }

    let mut in_single = false;
    let mut in_double = false;
    let mut escape = false;
    let mut eq_byte_idx = None;

    for (idx, c) in line.char_indices() {
        if escape {
            escape = false;
            continue;
        }
        if in_double && c == '\\' {
            escape = true;
            continue;
        }
        if c == '\'' && !in_double {
            in_single = !in_single;
        } else if c == '"' && !in_single {
            in_double = !in_double;
        } else if c == '=' && !in_single && !in_double {
            eq_byte_idx = Some(idx);
            break;
        } else if c == '#' && !in_single && !in_double {
            break;
        }
    }

    let eq_idx = eq_byte_idx?;
    let key_part = &line[..eq_idx];
    let after_eq = &line[eq_idx + 1..];

    in_single = false;
    in_double = false;
    escape = false;
    let mut comment_idx = None;
    for (idx, c) in after_eq.char_indices() {
        if escape {
            escape = false;
            continue;
        }
        if in_double && c == '\\' {
            escape = true;
            continue;
        }
        if c == '\'' && !in_double {
            in_single = !in_single;
        } else if c == '"' && !in_single {
            in_double = !in_double;
        } else if c == '#' && !in_single && !in_double {
            comment_idx = Some(idx);
            break;
        }
    }

    let (val_part, comment_part) = match comment_idx {
        Some(c_idx) => (&after_eq[..c_idx], &after_eq[c_idx..]),
        None => (after_eq, ""),
    };
    Some((key_part, val_part, comment_part))
}

fn unquote_toml_key(raw_key: &str) -> String {
    let trimmed = raw_key.trim();
    if (trimmed.starts_with('"') && trimmed.ends_with('"') && trimmed.len() >= 2)
        || (trimmed.starts_with('\'') && trimmed.ends_with('\'') && trimmed.len() >= 2)
    {
        trimmed[1..trimmed.len() - 1].to_string()
    } else {
        trimmed.to_string()
    }
}

fn format_toml_key_for_insert(key: &str) -> String {
    if key
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        key.to_string()
    } else {
        format!("\"{key}\"")
    }
}

/// Updates one or more `(key_path, raw_toml_value)` entries inside `existing_toml` in-place
/// while preserving all `#` comments, blank lines, and section ordering.
///
/// `key_path` may be a top-level key (e.g., `"tab_width"`) or a sectioned key
/// (`"filetypes.rs"`, `"keys.ctrl+q"`, `"theme.background"`).
#[allow(dead_code)]
pub fn update_toml_preserving_format(existing_toml: &str, updates: &[(&str, &str)]) -> String {
    let has_crlf = existing_toml.contains("\r\n");
    let line_ending = if has_crlf { "\r\n" } else { "\n" };
    let ends_with_newline = existing_toml.ends_with('\n');

    let mut lines: Vec<String> = existing_toml
        .lines()
        .map(|l| l.strip_suffix('\r').unwrap_or(l).to_string())
        .collect();

    for &(key_path, new_val) in updates {
        let (target_section, target_key) = match key_path.split_once('.') {
            Some((sec, rest))
                if matches!(sec.trim(), "filetypes" | "keys" | "theme") && !rest.is_empty() =>
            {
                (Some(sec.trim()), unquote_toml_key(rest))
            }
            _ => (None, unquote_toml_key(key_path)),
        };

        let normalized_target = match target_section {
            Some("keys") => normalize_key_descriptor(&target_key),
            Some("filetypes") => normalize_filetype_key(&target_key),
            _ => target_key.to_ascii_lowercase(),
        };

        let mut current_section: Option<String> = None;
        let mut updated = false;
        let mut section_found = target_section.is_none();
        let mut last_key_line_in_section: Option<usize> = None;
        let mut section_header_line: Option<usize> = None;

        for (idx, line) in lines.iter_mut().enumerate() {
            let trimmed = line.trim();
            if trimmed.starts_with('[') && trimmed.ends_with(']') && !trimmed.starts_with("[[") {
                let sec_name = trimmed[1..trimmed.len() - 1].trim().to_ascii_lowercase();
                current_section = Some(sec_name.clone());
                if target_section == Some(sec_name.as_str()) {
                    section_found = true;
                    section_header_line = Some(idx);
                }
                continue;
            }

            let in_target_section = match (&current_section, target_section) {
                (None, None) => true,
                (Some(cur), Some(target)) => cur == target,
                _ => false,
            };

            if !in_target_section {
                continue;
            }

            if let Some((key_part, val_part, comment_part)) = split_toml_key_value_and_comment(line)
            {
                last_key_line_in_section = Some(idx);
                let raw_k = unquote_toml_key(key_part);
                let norm_k = match target_section {
                    Some("keys") => normalize_key_descriptor(&raw_k),
                    Some("filetypes") => normalize_filetype_key(&raw_k),
                    _ => raw_k.to_ascii_lowercase(),
                };
                if norm_k == normalized_target {
                    let leading_ws: String =
                        val_part.chars().take_while(|c| c.is_whitespace()).collect();
                    let ws_before_val = if leading_ws.is_empty() {
                        " "
                    } else {
                        leading_ws.as_str()
                    };
                    if comment_part.is_empty() {
                        *line = format!("{key_part}={ws_before_val}{new_val}");
                    } else {
                        *line = format!("{key_part}={ws_before_val}{new_val} {comment_part}");
                    }
                    updated = true;
                    break;
                }
            }
        }

        if !updated {
            let formatted_key = format_toml_key_for_insert(&target_key);
            let new_line = format!("{formatted_key} = {new_val}");
            if section_found {
                let insert_idx = last_key_line_in_section
                    .or(section_header_line)
                    .map_or(lines.len(), |i| i + 1);
                lines.insert(insert_idx, new_line);
            } else if let Some(sec) = target_section {
                if !lines.is_empty() && !lines.last().unwrap().trim().is_empty() {
                    lines.push(String::new());
                }
                lines.push(format!("[{sec}]"));
                lines.push(new_line);
            }
        }
    }

    let mut out = lines.join(line_ending);
    if ends_with_newline {
        out.push_str(line_ending);
    }
    out
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

    #[test]
    fn action_chaining_deserializes_arrays_and_comma_separated_strings() {
        let toml_input = r#"
[keys]
"ctrl+shift+q" = ["save", "quit"]
"alt+x" = "save, quit"
"ctrl+s" = "save"
"#;
        let cfg = Config::from_toml_str(toml_input).unwrap();
        let ctrl_shift_q = KeyEvent::new(
            KeyCode::Char('Q'),
            KeyModifiers::CONTROL | KeyModifiers::SHIFT,
        );
        assert_eq!(
            cfg.actions_for_key(ctrl_shift_q),
            Some(&[Action::Save, Action::Quit][..])
        );
        assert_eq!(cfg.action_for_key(ctrl_shift_q), Some(Action::Save));

        let alt_x = KeyEvent::new(KeyCode::Char('x'), KeyModifiers::ALT);
        assert_eq!(
            cfg.actions_for_key(alt_x),
            Some(&[Action::Save, Action::Quit][..])
        );
    }

    #[test]
    fn filetype_precedence_prioritizes_exact_filename_then_glob_then_extension() {
        let toml_input = r#"
tab_width = 4
expand_tabs = true
auto_pairs = true

[filetypes]
"tmux.conf" = { tab_width = 2, expand_tabs = true, auto_pairs = false }
"*.conf"    = { tab_width = 4, expand_tabs = false }
"conf"      = { tab_width = 8, expand_tabs = true }
".env*"     = { tab_width = 2, expand_tabs = true, auto_pairs = false }
"#;
        let cfg = Config::from_toml_str(toml_input).unwrap();

        // 1. Exact basename "tmux.conf" wins over "*.conf" and extension "conf"
        assert_eq!(
            cfg.resolve_filetype_config(Some("/etc/tmux.conf")),
            (2, true, false)
        );
        // 2. Glob "*.conf" wins over extension "conf"
        assert_eq!(
            cfg.resolve_filetype_config(Some("/etc/nginx.conf")),
            (4, false, true)
        );
        // 3. Glob ".env*" matches hidden files like ".env.local"
        assert_eq!(
            cfg.resolve_filetype_config(Some("/app/.env.local")),
            (2, true, false)
        );
    }

    #[test]
    fn update_toml_preserving_format_retains_comments_and_updates_keys() {
        let updated = update_toml_preserving_format(
            DEFAULT_CONFIG_TEMPLATE,
            &[
                ("tab_width", "2"),
                ("auto_pairs", "false"),
                ("keys.\"ctrl+q\"", "[\"save\", \"quit\"]"),
            ],
        );
        assert!(updated.contains("# omo configuration file"));
        assert!(updated.contains("# Global fallback settings"));
        assert!(updated.contains("# Optional 24-bit RGB colors: [R, G, B]"));

        let parsed = Config::from_toml_str(&updated).unwrap();
        assert_eq!(parsed.tab_width, 2);
        assert!(!parsed.auto_pairs);
        assert_eq!(
            parsed.actions_for_key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::CONTROL)),
            Some(&[Action::Save, Action::Quit][..])
        );
    }
}
