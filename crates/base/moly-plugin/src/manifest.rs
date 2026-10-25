//! `plugin.toml`: what a plugin is and what it needs.
//!
//! ```toml
//! [plugin]
//! id = "moly.companion"          # lowercase ASCII letters, digits, '.', '-', '_'; starts with a letter
//! name = "AI player"
//! version = "0.1.0"              # semver
//! moly = ">=0.0.5"               # semver requirement on the running moly version
//! kind = "luau"                  # "luau" or "native"
//! entry = "main.luau"            # required for luau, refused for native; a relative path inside the plugin directory
//! description = "…"              # optional
//!
//! [depends]                      # required plugins: id = semver requirement
//! "moly.mcp" = ">=0.1"
//!
//! [after]                        # optional plugins: loaded first when enabled, otherwise ignored
//! "example.weather" = "*"
//!
//! [conflicts]                    # plugins that must not be enabled together with this one
//! "example.other" = "*"
//!
//! [[provides]]                   # seams this plugin provides
//! seam = "companion.brain"
//! target = "unit:5"              # optional: the one target of a per-target seam
//!
//! [config]                       # default settings; any TOML table
//! unit = 5
//! ```
//!
//! Unknown keys are refused, so a misspelt field is an error and not a
//! silent default.

use std::{collections::BTreeMap, fmt, path::Path};

use semver::{Version, VersionReq};
use serde::Deserialize;

/// A plugin id: lowercase ASCII letters, digits, `.`, `-` and `_`, starting
/// with a letter, at most 64 bytes.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PluginId(String);

impl PluginId {
    pub fn new(id: &str) -> Result<Self, ManifestError> {
        let starts_with_letter = id.as_bytes().first().is_some_and(u8::is_ascii_lowercase);
        let well_formed = id.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'.' | b'-' | b'_')
        });
        if id.len() <= 64 && starts_with_letter && well_formed {
            Ok(Self(id.to_owned()))
        } else {
            Err(ManifestError::BadId(id.to_owned()))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for PluginId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PluginKind {
    /// A Luau script run in the sandbox.
    Luau,
    /// Built into the game binary; the manifest only declares it.
    Native,
}

/// One `[[provides]]` entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Provides {
    pub seam: String,
    /// The target of a per-target seam (for example `unit:5`); `None` for
    /// the seam as a whole.
    pub target: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Manifest {
    pub id: PluginId,
    pub name: String,
    pub version: Version,
    pub moly: VersionReq,
    pub kind: PluginKind,
    /// Relative path of the entry script (Luau only).
    pub entry: Option<String>,
    pub description: Option<String>,
    pub depends: BTreeMap<PluginId, VersionReq>,
    pub after: BTreeMap<PluginId, VersionReq>,
    pub conflicts: BTreeMap<PluginId, VersionReq>,
    pub provides: Vec<Provides>,
    /// Default settings; the profile's settings for this plugin are merged
    /// over it key by key at the top level.
    pub config: toml::Table,
}

impl Manifest {
    /// Parse the text of a `plugin.toml`.
    pub fn parse(text: &str) -> Result<Self, ManifestError> {
        let file: ManifestFile =
            toml::from_str(text).map_err(|err| ManifestError::Syntax(err.to_string()))?;
        let section = file.plugin;
        let id = PluginId::new(&section.id)?;
        let version = Version::parse(&section.version).map_err(|err| ManifestError::BadVersion {
            field: "version".to_owned(),
            value: section.version.clone(),
            error: err.to_string(),
        })?;
        let moly = VersionReq::parse(&section.moly).map_err(|err| ManifestError::BadVersion {
            field: "moly".to_owned(),
            value: section.moly.clone(),
            error: err.to_string(),
        })?;
        let kind = match section.kind.as_str() {
            "luau" => PluginKind::Luau,
            "native" => PluginKind::Native,
            other => {
                return Err(ManifestError::Syntax(format!(
                    "plugin.kind must be \"luau\" or \"native\", found {other:?}"
                )))
            }
        };
        let entry = check_entry(kind, section.entry)?;
        let depends = parse_requirements("depends", file.depends)?;
        let after = parse_requirements("after", file.after)?;
        let conflicts = parse_requirements("conflicts", file.conflicts)?;
        check_relations(
            &id,
            [
                ("depends", &depends),
                ("after", &after),
                ("conflicts", &conflicts),
            ],
        )?;
        let provides: Vec<Provides> = file
            .provides
            .into_iter()
            .map(|provided| Provides {
                seam: provided.seam,
                target: provided.target,
            })
            .collect();
        Ok(Self {
            id,
            name: section.name,
            version,
            moly,
            kind,
            entry,
            description: section.description,
            depends,
            after,
            conflicts,
            provides,
            config: file.config,
        })
    }

    /// Read `<dir>/plugin.toml`.
    pub fn read(dir: &Path) -> Result<Self, ManifestError> {
        let path = dir.join("plugin.toml");
        let text = std::fs::read_to_string(&path).map_err(|err| ManifestError::Io {
            path: path.display().to_string(),
            error: err.to_string(),
        })?;
        Self::parse(&text)
    }
}

#[derive(Debug, PartialEq)]
pub enum ManifestError {
    Io { path: String, error: String },
    /// TOML syntax or an unknown, missing or mistyped key; the message
    /// names the key.
    Syntax(String),
    BadId(String),
    BadVersion { field: String, value: String, error: String },
    /// `entry` missing for a Luau plugin, present for a native one, absolute,
    /// or leaving the plugin directory (`..`).
    BadEntry(String),
    /// A plugin listed in `depends`, `after` or `conflicts` of itself, or in
    /// two of those tables.
    SelfOrRepeated(String),
}

impl fmt::Display for ManifestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ManifestError::Io { path, error } => {
                write!(f, "Could not read {path}: {error}.")
            }
            ManifestError::Syntax(message) => {
                write!(f, "The plugin manifest or profile is not valid: {message}")
            }
            ManifestError::BadId(id) => write!(
                f,
                "Plugin id {id:?} is not valid: it must be at most 64 bytes, start with a lowercase ASCII letter, and use only lowercase ASCII letters, digits, '.', '-' and '_'."
            ),
            ManifestError::BadVersion { field, value, error } => write!(
                f,
                "Field `{field}` has the value {value:?}, which is not a valid semver version or requirement: {error}."
            ),
            ManifestError::BadEntry(message) => {
                write!(f, "The plugin entry is not valid: {message}.")
            }
            ManifestError::SelfOrRepeated(message) => {
                write!(f, "The plugin relations are not valid: {message}.")
            }
        }
    }
}

impl std::error::Error for ManifestError {}

/// The on-disk shape of `plugin.toml`. Unknown keys are refused at every level.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ManifestFile {
    plugin: PluginSection,
    #[serde(default)]
    depends: BTreeMap<String, String>,
    #[serde(default)]
    after: BTreeMap<String, String>,
    #[serde(default)]
    conflicts: BTreeMap<String, String>,
    #[serde(default)]
    provides: Vec<ProvidesSection>,
    #[serde(default)]
    config: toml::Table,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PluginSection {
    id: String,
    name: String,
    version: String,
    moly: String,
    kind: String,
    #[serde(default)]
    entry: Option<String>,
    #[serde(default)]
    description: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ProvidesSection {
    seam: String,
    #[serde(default)]
    target: Option<String>,
}

/// `entry` is required for a Luau plugin and refused for a native one.
fn check_entry(kind: PluginKind, entry: Option<String>) -> Result<Option<String>, ManifestError> {
    match (kind, entry) {
        (PluginKind::Native, None) => Ok(None),
        (PluginKind::Native, Some(_)) => Err(ManifestError::BadEntry(
            "a native plugin must not set entry".to_owned(),
        )),
        (PluginKind::Luau, None) => Err(ManifestError::BadEntry(
            "a luau plugin must set entry to its script path".to_owned(),
        )),
        (PluginKind::Luau, Some(entry)) => {
            check_entry_path(&entry)?;
            Ok(Some(entry))
        }
    }
}

/// The entry must be a relative path that stays inside the plugin directory.
fn check_entry_path(entry: &str) -> Result<(), ManifestError> {
    let refuse = |reason: &str| ManifestError::BadEntry(format!("entry {entry:?} {reason}"));
    let bytes = entry.as_bytes();
    if entry.is_empty() {
        return Err(refuse("must name a file"));
    }
    if entry.starts_with('/') || entry.starts_with('\\') {
        return Err(refuse("must be relative to the plugin directory"));
    }
    if bytes.len() >= 2 && bytes[1] == b':' && bytes[0].is_ascii_alphabetic() {
        return Err(refuse("must not start with a drive letter"));
    }
    if entry.split(|c: char| c == '/' || c == '\\').any(|part| part == "..") {
        return Err(refuse("must not contain a `..` component"));
    }
    Ok(())
}

/// Parse one of the `depends`, `after` or `conflicts` tables.
fn parse_requirements(
    table: &str,
    entries: BTreeMap<String, String>,
) -> Result<BTreeMap<PluginId, VersionReq>, ManifestError> {
    let mut parsed = BTreeMap::new();
    for (name, text) in entries {
        let field = format!("{table}.{name}");
        let id = PluginId::new(&name)?;
        let requirement = VersionReq::parse(&text).map_err(|err| ManifestError::BadVersion {
            field,
            value: text,
            error: err.to_string(),
        })?;
        parsed.insert(id, requirement);
    }
    Ok(parsed)
}

/// A plugin may not list itself, and may not list another plugin in more than one table.
fn check_relations(
    id: &PluginId,
    tables: [(&str, &BTreeMap<PluginId, VersionReq>); 3],
) -> Result<(), ManifestError> {
    let mut listed: BTreeMap<&PluginId, &str> = BTreeMap::new();
    for (table, entries) in tables {
        for other in entries.keys() {
            if other == id {
                return Err(ManifestError::SelfOrRepeated(format!(
                    "plugin {id} lists itself in {table}"
                )));
            }
            if let Some(first) = listed.insert(other, table) {
                return Err(ManifestError::SelfOrRepeated(format!(
                    "plugin {other} is listed in both {first} and {table}"
                )));
            }
        }
    }
    Ok(())
}
