//! The program configuration file, `moly.toml`.
//!
//! One file holds what a user or a deployment sets before a program starts:
//! where resources and master tables come from, the server address, the
//! server's listen address and data directory, the launcher's choices,
//! display switches and developer switches. The launcher, the game, the
//! developer window and the server read the same file. State a program writes
//! while it runs (settings, server documents, saves) is kept in its own
//! documents and is not part of this file.
//!
//! Every key is declared once in [`KEYS`]: its path (`section.name`), the
//! environment variable that overrides it, the kind of value and what it
//! does. A value is taken from its environment variable when that is set,
//! else from the file, else the caller decides (a default, or a named refusal
//! through [`Config::require_text`]). A variable set to an empty value counts
//! as set for text keys (the caller refuses it by name) and as unset for
//! flags.
//!
//! The file is checked when it is loaded: a key that is not declared, a
//! top-level value outside a section, or a value of the wrong kind is an
//! error that names the key. [`set_values`] changes keys in place and keeps
//! the file's comments and order.

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// The kind of value a key holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// A string.
    Text,
    /// A file or directory path.
    Path,
    /// An `http://` or `https://` address, or an address template.
    Url,
    /// `true` or `false` in the file; `1` or `0` in the environment.
    Flag,
    /// A whole number.
    Integer,
}

impl Kind {
    fn name(self) -> &'static str {
        match self {
            Self::Text => "a string",
            Self::Path => "a path",
            Self::Url => "an http:// or https:// address",
            Self::Flag => "true or false",
            Self::Integer => "a whole number",
        }
    }
}

/// One declared key.
#[derive(Debug, PartialEq, Eq)]
pub struct Key {
    /// `section.name` in `moly.toml`.
    pub path: &'static str,
    /// The environment variable that overrides the file, if any.
    pub env: Option<&'static str>,
    pub kind: Kind,
    /// Which programs read it.
    pub readers: &'static str,
    pub doc: &'static str,
}

impl Key {
    fn section(&self) -> &'static str {
        self.path.split_once('.').map_or(self.path, |(section, _)| section)
    }

    fn name(&self) -> &'static str {
        self.path.split_once('.').map_or(self.path, |(_, name)| name)
    }
}

macro_rules! keys {
    ($($ident:ident = $path:literal, $env:expr, $kind:ident, $readers:literal, $doc:literal;)*) => {
        /// The declared keys, one constant each.
        pub mod keys {
            use super::{Key, Kind};
            $(
                #[doc = $doc]
                pub const $ident: Key = Key {
                    path: $path,
                    env: $env,
                    kind: Kind::$kind,
                    readers: $readers,
                    doc: $doc,
                };
            )*
        }
        /// Every declared key, in the order of the file's sections.
        pub const KEYS: &[&Key] = &[$(&keys::$ident),*];
    };
}

keys! {
    SETTINGS_FILE = "paths.settings", Some("MOLY_SETTINGS_FILE"), Path, "game, developer window",
        "The settings file. Default: `settings.json` in the data directory.";
    RESOURCE_ROOT = "resources.root", Some("MOLY_ASSET_ROOT"), Path, "game, developer window, stage",
        "The resource root directory. Default: the version selected in the launcher, else the newest installed one.";
    RESOURCE_STORE = "resources.store", Some("MOLY_ASSET_STORE_URL"), Url, "game",
        "A public asset store directory read over HTTPS instead of a local resource root. Setting both is refused.";
    RESOURCE_CATALOG = "resources.catalog", Some("MOLY_ASSET_CATALOG"), Text, "game",
        "The catalog release (a lowercase SHA-256) read from the resource root or the asset store.";
    RESOURCE_CACHE = "resources.cache", Some("MOLY_ASSET_CACHE"), Path, "game",
        "Where documents and files read from `resources.store` are kept. Created when missing.";
    RESOURCE_BASE = "resources.base", None, Url, "launcher",
        "The address resources are downloaded from, ending with `/`.";
    RESOURCE_DIR = "resources.dir", None, Path, "launcher, game",
        "Where the launcher installs resources. Default: the `resources` directory beside the data directory.";
    RESOURCE_SNAPSHOT = "resources.snapshot", None, Text, "launcher, game",
        "The installed resource version the game starts with. Default: the newest complete install.";
    STORAGE_BASE = "resources.storage", Some("MOLY_STORAGE_BASE"), Url, "game, developer window",
        "The address of the storage that remote resource files are read from.";
    STORAGE_MIRROR = "resources.storage_mirror", Some("MOLY_STORAGE_MIRROR_BASE"), Url, "game, developer window",
        "The storage tried when `resources.storage` does not answer.";
    MASTER_REPOSITORY = "masters.repository", Some("MOLY_MASTER_HARUKI_BASE"), Url, "game, developer window",
        "The first master table source: an address template with a `{repo}` placeholder.";
    MASTER_BASE = "masters.base", Some("MOLY_MASTER_BASE"), Url, "game, developer window",
        "The second master table source.";
    MASTER_MIRROR = "masters.mirror", Some("MOLY_MASTER_MIRROR_BASE"), Url, "game, developer window",
        "The third master table source.";
    PLAYER_API = "player_data.api", Some("MOLY_PLAYER_API"), Url, "developer window",
        "The player data address template, with `{region}` and `{target_user_id}` placeholders.";
    PLAYER_REGION = "player_data.region", Some("MOLY_PLAYER_REGION"), Text, "developer window",
        "The region of the player data: `cn` or `jp`.";
    PLAYER_UID = "player_data.uid", Some("MOLY_PLAYER_UID"), Text, "developer window",
        "Fetches and previews the data of this user id at start.";
    PLAYER_DATA_FILE = "player_data.file", Some("MOLY_PLAYER_DATA_FILE"), Path, "developer window",
        "Imports this player data file at start. Set at most one of `uid` and `file`.";
    SERVER_URL = "server.url", Some("MOLY_SERVER_URL"), Url, "game, developer window",
        "The server the game connects to.";
    SERVER_LISTEN = "server.listen", Some("MOLY_SERVER_ADDR"), Text, "server",
        "The server's listen address (`host:port`).";
    SERVER_CONTROL_PORT = "server.control_port", Some("MOLY_SERVER_CONTROL_PORT"), Integer, "server",
        "The server's control port.";
    SERVER_DATA = "server.data", Some("MOLY_SERVER_DATA"), Path, "server",
        "The directory the server saves player documents in. Unset: documents are kept in memory only.";
    UPDATE_SOURCE = "update.source", None, Url, "launcher",
        "The release source the launcher checks for program updates.";
    LAUNCHER_JOBS = "launcher.jobs", None, Integer, "launcher",
        "Parallel transfers while downloading. Default: 16.";
    LAUNCHER_GAME = "launcher.game", None, Path, "launcher",
        "The game program. Default: the one beside the launcher.";
    HIDE_HUD = "display.hide_hud", Some("MOLY_HIDE_HUD"), Flag, "game, developer window, stage",
        "Hides every HUD, button and dialog view; the scene, the balloons and the talk window remain.";
    HIDE_PLAYER = "display.hide_player", Some("MOLY_HIDE_PLAYER"), Flag, "game, developer window, stage",
        "Hides the player's own body.";
    DEV_WINDOW = "dev.window", Some("MOLY_DEV"), Flag, "game, server",
        "Starts the developer window instead of the game.";
}

/// Finds a declared key by its path.
pub fn key(path: &str) -> Option<&'static Key> {
    KEYS.iter().copied().find(|key| key.path == path)
}

/// Where a value came from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Source {
    Env(&'static str),
    File(Option<PathBuf>),
}

impl fmt::Display for Source {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Env(name) => write!(f, "environment variable {name}"),
            Self::File(Some(path)) => write!(f, "{}", path.display()),
            Self::File(None) => f.write_str("the configuration"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConfigError {
    /// The file exists but cannot be read or written.
    Io { path: PathBuf, message: String },
    /// The file is not valid TOML.
    Parse { path: Option<PathBuf>, message: String },
    /// A key, or a value outside a section, that is not declared.
    Unknown { path: Option<PathBuf>, key: String },
    /// A value of the wrong kind.
    Kind { key: &'static str, expected: Kind, source: Source },
    /// A required key with no value.
    Missing { key: &'static str, env: Option<&'static str> },
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, message } => write!(f, "{}: {message}", path.display()),
            Self::Parse { path: Some(path), message } => write!(f, "{}: {message}", path.display()),
            Self::Parse { path: None, message } => f.write_str(message),
            Self::Unknown { path, key } => {
                let path = path.as_deref().map_or("moly.toml".into(), |path| path.display().to_string());
                write!(f, "{path}: `{key}` is not a configuration key")
            }
            Self::Kind { key, expected, source } => {
                write!(f, "`{key}` in {source} must be {}", expected.name())
            }
            Self::Missing { key, env } => {
                let (section, name) = key.split_once('.').unwrap_or(("", key));
                write!(f, "`{key}` is not configured: set `{name}` in the [{section}] section of moly.toml")?;
                match env {
                    Some(env) => write!(f, " or the environment variable {env}"),
                    None => Ok(()),
                }
            }
        }
    }
}

impl std::error::Error for ConfigError {}

/// A loaded configuration file. An empty one (no file) still reads the
/// environment.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Config {
    path: Option<PathBuf>,
    values: BTreeMap<&'static str, toml::Value>,
}

impl Config {
    /// No file: every value comes from the environment or the caller.
    pub fn empty() -> Self {
        Self::default()
    }

    /// Reads `path`. A missing file is an empty configuration that remembers
    /// the path.
    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        match std::fs::read_to_string(path) {
            Ok(text) => Self::parse(&text, Some(path)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Self {
                path: Some(path.to_owned()),
                values: BTreeMap::new(),
            }),
            Err(error) => Err(ConfigError::Io { path: path.to_owned(), message: error.to_string() }),
        }
    }

    /// Parses and checks the text of a file.
    pub fn parse(text: &str, path: Option<&Path>) -> Result<Self, ConfigError> {
        let path = path.map(Path::to_owned);
        let table: toml::Table = text
            .parse()
            .map_err(|error: toml::de::Error| ConfigError::Parse { path: path.clone(), message: error.to_string() })?;
        let mut values = BTreeMap::new();
        for (section, entries) in table {
            let toml::Value::Table(entries) = entries else {
                return Err(ConfigError::Unknown { path, key: section });
            };
            for (name, value) in entries {
                let full = format!("{section}.{name}");
                let Some(key) = key(&full) else {
                    return Err(ConfigError::Unknown { path, key: full });
                };
                if !fits(key.kind, &value) {
                    return Err(ConfigError::Kind {
                        key: key.path,
                        expected: key.kind,
                        source: Source::File(path),
                    });
                }
                values.insert(key.path, value);
            }
        }
        Ok(Self { path, values })
    }

    /// The file this configuration was loaded from.
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// The key's value as text (`Text`, `Path` and `Url` keys), from the
    /// environment, else the file.
    pub fn text(&self, key: &'static Key) -> Option<String> {
        self.text_in(key, &process_env)
    }

    /// [`Self::text`], refusing with the key's name and where to set it when
    /// there is no value.
    pub fn require_text(&self, key: &'static Key) -> Result<String, ConfigError> {
        self.text(key).ok_or(ConfigError::Missing { key: key.path, env: key.env })
    }

    /// A `Flag` key: `1` or `0` in the environment, `true` or `false` in the
    /// file. Anything else is refused by name.
    pub fn flag(&self, key: &'static Key) -> Result<Option<bool>, ConfigError> {
        self.flag_in(key, &process_env)
    }

    /// An `Integer` key.
    pub fn integer(&self, key: &'static Key) -> Result<Option<i64>, ConfigError> {
        self.integer_in(key, &process_env)
    }

    /// Whether the file sets the key.
    pub fn has(&self, key: &Key) -> bool {
        self.values.contains_key(key.path)
    }

    /// The file's own value as text, ignoring the environment. For a program
    /// that edits the file: what it shows and saves back must be the file's.
    pub fn file_text(&self, key: &Key) -> Option<String> {
        match self.values.get(key.path)? {
            toml::Value::String(value) => Some(value.clone()),
            other => Some(other.to_string()),
        }
    }

    /// The file's own `Flag` value, ignoring the environment.
    pub fn file_flag(&self, key: &Key) -> Option<bool> {
        self.values.get(key.path).and_then(toml::Value::as_bool)
    }

    /// The file's own `Integer` value, ignoring the environment.
    pub fn file_integer(&self, key: &Key) -> Option<i64> {
        self.values.get(key.path).and_then(toml::Value::as_integer)
    }

    fn env_value(key: &'static Key, env: &dyn Fn(&str) -> Option<String>) -> Option<(&'static str, String)> {
        let name = key.env?;
        env(name).map(|value| (name, value))
    }

    fn text_in(&self, key: &'static Key, env: &dyn Fn(&str) -> Option<String>) -> Option<String> {
        if let Some((_, value)) = Self::env_value(key, env) {
            return Some(value);
        }
        self.file_text(key)
    }

    fn flag_in(&self, key: &'static Key, env: &dyn Fn(&str) -> Option<String>) -> Result<Option<bool>, ConfigError> {
        if let Some((name, value)) = Self::env_value(key, env).filter(|(_, value)| !value.is_empty()) {
            return match value.as_str() {
                "1" => Ok(Some(true)),
                "0" => Ok(Some(false)),
                _ => Err(ConfigError::Kind { key: key.path, expected: Kind::Flag, source: Source::Env(name) }),
            };
        }
        Ok(self.file_flag(key))
    }

    fn integer_in(&self, key: &'static Key, env: &dyn Fn(&str) -> Option<String>) -> Result<Option<i64>, ConfigError> {
        if let Some((name, value)) = Self::env_value(key, env) {
            return value
                .parse()
                .map(Some)
                .map_err(|_| ConfigError::Kind { key: key.path, expected: Kind::Integer, source: Source::Env(name) });
        }
        Ok(self.file_integer(key))
    }
}

fn process_env(name: &str) -> Option<String> {
    std::env::var(name).ok()
}

fn fits(kind: Kind, value: &toml::Value) -> bool {
    match (kind, value) {
        (Kind::Text | Kind::Path, toml::Value::String(_)) => true,
        (Kind::Url, toml::Value::String(text)) => is_address(text),
        (Kind::Flag, toml::Value::Boolean(_)) => true,
        (Kind::Integer, toml::Value::Integer(_)) => true,
        _ => false,
    }
}

fn is_address(text: &str) -> bool {
    text.strip_prefix("https://")
        .or_else(|| text.strip_prefix("http://"))
        .is_some_and(|rest| !rest.is_empty())
}

static GLOBAL: OnceLock<Config> = OnceLock::new();

/// Makes `config` the program's configuration. Each program installs it once,
/// at start, before anything reads it; a second call is refused.
pub fn install(config: Config) -> Result<(), Config> {
    GLOBAL.set(config)
}

/// The program's configuration: the installed one, else an empty one that
/// reads the environment only (tests, and the browser build, which has no
/// file).
pub fn global() -> &'static Config {
    GLOBAL.get_or_init(Config::empty)
}

/// A value to write with [`set_values`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Value {
    Text(String),
    Flag(bool),
    Integer(i64),
}

/// Sets (`Some`) or removes (`None`) keys in the file at `path`, keeping its
/// comments and the order of what it already holds. The file is created when
/// it does not exist, and replaced in one step (written beside it, then
/// renamed). The result is checked like a loaded file before it is written.
pub fn set_values(path: &Path, values: &[(&'static Key, Option<Value>)]) -> Result<(), ConfigError> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(ConfigError::Io { path: path.to_owned(), message: error.to_string() }),
    };
    let text = edit_text(&text, values, Some(path))?;
    if let Some(parent) = path.parent().filter(|parent| !parent.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)
            .map_err(|error| ConfigError::Io { path: parent.to_owned(), message: error.to_string() })?;
    }
    let mut part = path.as_os_str().to_owned();
    part.push(".part");
    let part = PathBuf::from(part);
    std::fs::write(&part, text)
        .and_then(|()| std::fs::rename(&part, path))
        .map_err(|error| ConfigError::Io { path: path.to_owned(), message: error.to_string() })
}

fn edit_text(text: &str, values: &[(&'static Key, Option<Value>)], path: Option<&Path>) -> Result<String, ConfigError> {
    let mut document: toml_edit::DocumentMut = text
        .parse()
        .map_err(|error: toml_edit::TomlError| ConfigError::Parse { path: path.map(Path::to_owned), message: error.to_string() })?;
    for (key, value) in values {
        match value {
            Some(value) => {
                let section = document
                    .entry(key.section())
                    .or_insert_with(|| toml_edit::Item::Table(toml_edit::Table::new()));
                let Some(section) = section.as_table_like_mut() else {
                    return Err(ConfigError::Unknown { path: path.map(Path::to_owned), key: key.section().to_owned() });
                };
                let mut new = match value {
                    Value::Text(text) => toml_edit::Value::from(text.as_str()),
                    Value::Flag(flag) => toml_edit::Value::from(*flag),
                    Value::Integer(number) => toml_edit::Value::from(*number),
                };
                // A key already in the file keeps its comments and spacing;
                // only its value changes.
                if let Some(toml_edit::Item::Value(existing)) = section.get_mut(key.name()) {
                    *new.decor_mut() = existing.decor().clone();
                    *existing = new;
                } else {
                    section.insert(key.name(), toml_edit::Item::Value(new));
                }
            }
            None => {
                if let Some(section) = document.get_mut(key.section()).and_then(toml_edit::Item::as_table_like_mut) {
                    section.remove(key.name());
                }
            }
        }
    }
    let text = document.to_string();
    Config::parse(&text, path)?;
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env_of(pairs: &'static [(&'static str, &'static str)]) -> impl Fn(&str) -> Option<String> {
        move |name| pairs.iter().find(|(key, _)| *key == name).map(|(_, value)| (*value).to_owned())
    }

    #[test]
    fn every_key_has_a_section_and_a_unique_path_and_variable() {
        let mut paths = std::collections::BTreeSet::new();
        let mut envs = std::collections::BTreeSet::new();
        for key in KEYS {
            assert!(key.path.split_once('.').is_some_and(|(s, n)| !s.is_empty() && !n.is_empty()), "{}", key.path);
            assert!(paths.insert(key.path), "duplicate {}", key.path);
            if let Some(env) = key.env {
                assert!(env.starts_with("MOLY_"), "{env}");
                assert!(envs.insert(env), "duplicate {env}");
            }
        }
    }

    #[test]
    fn the_environment_wins_over_the_file_and_an_empty_flag_does_not_count() {
        let config = Config::parse("[server]\nurl = \"https://file.example\"\n", None).unwrap();
        let set = env_of(&[("MOLY_SERVER_URL", "https://env.example")]);
        assert_eq!(config.text_in(&keys::SERVER_URL, &set).as_deref(), Some("https://env.example"));
        let empty = env_of(&[("MOLY_SERVER_URL", "")]);
        assert_eq!(config.text_in(&keys::SERVER_URL, &empty).as_deref(), Some(""));
        let hud = Config::parse("[display]
hide_hud = true
", None).unwrap();
        assert_eq!(hud.flag_in(&keys::HIDE_HUD, &env_of(&[("MOLY_HIDE_HUD", "")])), Ok(Some(true)));
        assert_eq!(Config::empty().text_in(&keys::SERVER_URL, &env_of(&[])), None);
    }

    #[test]
    fn unknown_keys_and_wrong_kinds_are_refused_by_name() {
        let unknown = Config::parse("[server]\nadress = \"x\"\n", None).unwrap_err();
        assert_eq!(unknown, ConfigError::Unknown { path: None, key: "server.adress".into() });
        let loose = Config::parse("url = \"https://x.example\"\n", None).unwrap_err();
        assert_eq!(loose, ConfigError::Unknown { path: None, key: "url".into() });
        let kind = Config::parse("[display]\nhide_hud = \"yes\"\n", None).unwrap_err();
        assert!(matches!(kind, ConfigError::Kind { key: "display.hide_hud", .. }));
        let address = Config::parse("[server]\nurl = \"server.example\"\n", None).unwrap_err();
        assert!(matches!(address, ConfigError::Kind { key: "server.url", expected: Kind::Url, .. }));
    }

    #[test]
    fn flags_read_one_or_zero_from_the_environment_and_booleans_from_the_file() {
        let config = Config::parse("[display]\nhide_hud = true\n", None).unwrap();
        assert_eq!(config.flag_in(&keys::HIDE_HUD, &env_of(&[])), Ok(Some(true)));
        assert_eq!(config.flag_in(&keys::HIDE_HUD, &env_of(&[("MOLY_HIDE_HUD", "0")])), Ok(Some(false)));
        assert!(config.flag_in(&keys::HIDE_HUD, &env_of(&[("MOLY_HIDE_HUD", "yes")])).is_err());
    }

    #[test]
    fn a_missing_required_key_names_the_section_and_the_variable() {
        let error = Config::empty().require_text(&keys::PLAYER_API).unwrap_err().to_string();
        assert!(error.contains("[player_data]") && error.contains("MOLY_PLAYER_API"), "{error}");
        let error = Config::empty().require_text(&keys::RESOURCE_BASE).unwrap_err().to_string();
        assert!(error.contains("[resources]") && !error.contains("MOLY_"), "{error}");
    }

    #[test]
    fn editing_keeps_comments_and_removes_keys() {
        let text = "# the launcher's choices\n[launcher]\n# keep this\njobs = 4\n";
        let edited = edit_text(
            text,
            &[
                (&keys::LAUNCHER_JOBS, Some(Value::Integer(8))),
                (&keys::RESOURCE_BASE, Some(Value::Text("https://cdn.example/".into()))),
            ],
            None,
        )
        .unwrap();
        assert!(edited.contains("# the launcher's choices") && edited.contains("# keep this"), "{edited}");
        let config = Config::parse(&edited, None).unwrap();
        assert_eq!(config.integer_in(&keys::LAUNCHER_JOBS, &env_of(&[])), Ok(Some(8)));
        let removed = edit_text(&edited, &[(&keys::LAUNCHER_JOBS, None)], None).unwrap();
        assert_eq!(Config::parse(&removed, None).unwrap().integer_in(&keys::LAUNCHER_JOBS, &env_of(&[])), Ok(None));
        assert!(edit_text(text, &[(&keys::SERVER_URL, Some(Value::Text("not an address".into())))], None).is_err());
    }
}
