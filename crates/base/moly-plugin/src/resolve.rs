//! Startup resolution of the enabled plugin set.
//!
//! [`resolve`] either returns the load order and the seam table, or refuses
//! with **every** problem it found, each naming the plugins involved. The
//! game does not start with a partial set.
//!
//! Rules:
//! 1. Every enabled id must have a manifest among `available`
//!    ([`ResolveError::NotInstalled`]); an id found in two directories is
//!    [`ResolveError::DuplicateId`].
//! 2. Every enabled plugin's `moly` requirement must match `moly_version`.
//! 3. Every `depends` entry must be enabled and its version must match the
//!    requirement. `after` entries that are not enabled are ignored; enabled
//!    ones must match their requirement.
//! 4. No two enabled plugins may conflict (a `conflicts` entry on either side
//!    whose requirement matches the other's version).
//! 5. Every `[[provides]]` seam must be declared ([`crate::seam::declared`]);
//!    a `target` on a seam that is not per-target is refused.
//! 6. An exclusive seam (or seam@target) with two or more plugin providers is
//!    refused unless the profile's `[seams]` names one of them; a profile
//!    choice naming a plugin that does not provide it, or is not enabled, is
//!    refused too.
//! 7. Load order: dependencies and enabled `after` entries before the
//!    plugin; among plugins with no order between them, by id. A cycle is
//!    refused with the plugins on it.

use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
    path::PathBuf,
};

use semver::{Version, VersionReq};

use crate::{
    manifest::{Manifest, PluginId},
    profile::Profile,
    seam::{declared, ExclusiveSeam, Provider, SeamKind, SeamTable, SEAMS},
};

/// One installed plugin: its directory and manifest.
#[derive(Clone, Debug)]
pub struct Installed {
    pub dir: PathBuf,
    pub manifest: Manifest,
}

#[derive(Clone, Debug)]
pub struct Resolved {
    /// Enabled plugins in load order.
    pub order: Vec<Installed>,
    pub seams: SeamTable,
    /// Each enabled plugin's settings: manifest `[config]` with the
    /// profile's table merged over it at the top level.
    pub config: BTreeMap<PluginId, toml::Table>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ResolveError {
    NotInstalled { plugin: PluginId },
    DuplicateId { plugin: PluginId, dirs: Vec<PathBuf> },
    MolyVersion { plugin: PluginId, requires: VersionReq, running: Version },
    MissingDependency { plugin: PluginId, dependency: PluginId, requires: VersionReq },
    DependencyVersion { plugin: PluginId, dependency: PluginId, requires: VersionReq, found: Version },
    Conflict { plugin: PluginId, other: PluginId },
    UnknownSeam { plugin: PluginId, seam: String },
    TargetNotAllowed { plugin: PluginId, seam: String, target: String },
    /// `seam` is `name` or `name@target`.
    SeamContested { seam: String, providers: Vec<PluginId> },
    BadSeamChoice { seam: String, plugin: PluginId },
    Cycle { plugins: Vec<PluginId> },
}

impl fmt::Display for ResolveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ResolveError::NotInstalled { plugin } => write!(
                f,
                "Plugin {plugin} is enabled in the profile, but no installed plugin has that id."
            ),
            ResolveError::DuplicateId { plugin, dirs } => write!(
                f,
                "Plugin {plugin} is installed in more than one directory: {}.",
                join_paths(dirs)
            ),
            ResolveError::MolyVersion {
                plugin,
                requires,
                running,
            } => write!(
                f,
                "Plugin {plugin} requires moly {requires}, but the running moly version is {running}."
            ),
            ResolveError::MissingDependency {
                plugin,
                dependency,
                requires,
            } => write!(
                f,
                "Plugin {plugin} depends on {dependency} {requires}, which is not enabled."
            ),
            ResolveError::DependencyVersion {
                plugin,
                dependency,
                requires,
                found,
            } => write!(
                f,
                "Plugin {plugin} requires {dependency} {requires}, but the enabled {dependency} is version {found}."
            ),
            ResolveError::Conflict { plugin, other } => write!(
                f,
                "Plugins {plugin} and {other} conflict, so they cannot be enabled together."
            ),
            ResolveError::UnknownSeam { plugin, seam } => write!(
                f,
                "Plugin {plugin} names the seam {seam}, which the game does not declare."
            ),
            ResolveError::TargetNotAllowed {
                plugin,
                seam,
                target,
            } => write!(
                f,
                "Plugin {plugin} names the target {target} for seam {seam}, which does not take targets."
            ),
            ResolveError::SeamContested { seam, providers } => write!(
                f,
                "Seam {seam} has several enabled providers ({}); choose one in the profile's [seams] table.",
                join_ids(providers)
            ),
            ResolveError::BadSeamChoice { seam, plugin } => write!(
                f,
                "The profile's [seams] table names {plugin} for {seam}, which is refused: only an enabled provider of an exclusive seam or target can be chosen."
            ),
            ResolveError::Cycle { plugins } => write!(
                f,
                "These plugins depend on each other in a cycle and cannot be ordered: {}.",
                join_ids(plugins)
            ),
        }
    }
}

fn join_ids(ids: &[PluginId]) -> String {
    ids.iter()
        .map(|id| id.to_string())
        .collect::<Vec<_>>()
        .join(", ")
}

fn join_paths(dirs: &[PathBuf]) -> String {
    dirs.iter()
        .map(|dir| dir.display().to_string())
        .collect::<Vec<_>>()
        .join(", ")
}

/// Resolve the profile's enabled set against the installed plugins.
pub fn resolve(
    available: &[Installed],
    profile: &Profile,
    moly_version: &Version,
) -> Result<Resolved, Vec<ResolveError>> {
    let mut errors = Vec::new();

    // Rule 1: every enabled plugin is installed in exactly one directory.
    let mut installed: BTreeMap<PluginId, Vec<&Installed>> = BTreeMap::new();
    for item in available {
        installed
            .entry(item.manifest.id.clone())
            .or_default()
            .push(item);
    }
    let enabled: BTreeSet<PluginId> = profile.enabled().cloned().collect();
    let mut units: BTreeMap<PluginId, &Installed> = BTreeMap::new();
    for id in &enabled {
        match installed.get(id) {
            None => errors.push(ResolveError::NotInstalled { plugin: id.clone() }),
            Some(found) if found.len() == 1 => {
                units.insert(id.clone(), found[0]);
            }
            Some(found) => errors.push(ResolveError::DuplicateId {
                plugin: id.clone(),
                dirs: found.iter().map(|item| item.dir.clone()).collect(),
            }),
        }
    }

    // Rule 2: the running moly version satisfies each enabled plugin's requirement.
    for (id, unit) in &units {
        if !unit.manifest.moly.matches(moly_version) {
            errors.push(ResolveError::MolyVersion {
                plugin: id.clone(),
                requires: unit.manifest.moly.clone(),
                running: moly_version.clone(),
            });
        }
    }

    // Rule 3: dependencies must be enabled and match; enabled `after` entries must match.
    // A dependency that is enabled but not usable is already reported by rule 1.
    for (id, unit) in &units {
        for (dependency, requires) in &unit.manifest.depends {
            if !enabled.contains(dependency) {
                errors.push(ResolveError::MissingDependency {
                    plugin: id.clone(),
                    dependency: dependency.clone(),
                    requires: requires.clone(),
                });
            } else if let Some(dep) = units.get(dependency) {
                if !requires.matches(&dep.manifest.version) {
                    errors.push(ResolveError::DependencyVersion {
                        plugin: id.clone(),
                        dependency: dependency.clone(),
                        requires: requires.clone(),
                        found: dep.manifest.version.clone(),
                    });
                }
            }
        }
        for (after, requires) in &unit.manifest.after {
            if let Some(dep) = units.get(after) {
                if !requires.matches(&dep.manifest.version) {
                    errors.push(ResolveError::DependencyVersion {
                        plugin: id.clone(),
                        dependency: after.clone(),
                        requires: requires.clone(),
                        found: dep.manifest.version.clone(),
                    });
                }
            }
        }
    }

    // Rule 4: a conflicts entry whose requirement matches the other enabled plugin's
    // version. Each unordered pair is reported once.
    let mut clashes: BTreeSet<(PluginId, PluginId)> = BTreeSet::new();
    for (id, unit) in &units {
        for (other, requires) in &unit.manifest.conflicts {
            if let Some(other_unit) = units.get(other) {
                if requires.matches(&other_unit.manifest.version) {
                    let pair = if id <= other {
                        (id.clone(), other.clone())
                    } else {
                        (other.clone(), id.clone())
                    };
                    clashes.insert(pair);
                }
            }
        }
    }
    for (plugin, other) in clashes {
        errors.push(ResolveError::Conflict { plugin, other });
    }

    // Rule 5: every provided seam is declared, and only a per-target seam takes a target.
    // The valid exclusive providers feed rule 6.
    let mut exclusive_whole: BTreeMap<&'static str, BTreeSet<PluginId>> = BTreeMap::new();
    let mut exclusive_targeted: BTreeMap<(&'static str, String), BTreeSet<PluginId>> =
        BTreeMap::new();
    for (id, unit) in &units {
        for provided in &unit.manifest.provides {
            let Some(decl) = declared(&provided.seam) else {
                errors.push(ResolveError::UnknownSeam {
                    plugin: id.clone(),
                    seam: provided.seam.clone(),
                });
                continue;
            };
            match &provided.target {
                Some(target) if !decl.per_target => {
                    errors.push(ResolveError::TargetNotAllowed {
                        plugin: id.clone(),
                        seam: provided.seam.clone(),
                        target: target.clone(),
                    });
                }
                Some(target) => {
                    if decl.kind == SeamKind::Exclusive {
                        exclusive_targeted
                            .entry((decl.name, target.clone()))
                            .or_default()
                            .insert(id.clone());
                    }
                }
                None => {
                    if decl.kind == SeamKind::Exclusive {
                        exclusive_whole
                            .entry(decl.name)
                            .or_default()
                            .insert(id.clone());
                    }
                }
            }
        }
    }

    // Rule 7: load order. A plugin follows its enabled dependencies and enabled `after`
    // entries. Among plugins with no order between them, the smaller id goes first.
    let mut waiting_on: BTreeMap<PluginId, usize> = BTreeMap::new();
    let mut unlocks: BTreeMap<PluginId, Vec<PluginId>> = BTreeMap::new();
    for (id, unit) in &units {
        waiting_on.entry(id.clone()).or_insert(0);
        for prerequisite in unit.manifest.depends.keys().chain(unit.manifest.after.keys()) {
            if units.contains_key(prerequisite) {
                *waiting_on.entry(id.clone()).or_insert(0) += 1;
                unlocks
                    .entry(prerequisite.clone())
                    .or_default()
                    .push(id.clone());
            }
        }
    }
    let mut ready: BTreeSet<PluginId> = waiting_on
        .iter()
        .filter(|(_, waiting)| **waiting == 0)
        .map(|(id, _)| id.clone())
        .collect();
    let mut order_ids: Vec<PluginId> = Vec::new();
    while let Some(id) = ready.pop_first() {
        for next in unlocks.get(&id).into_iter().flatten() {
            if let Some(waiting) = waiting_on.get_mut(next) {
                *waiting -= 1;
                if *waiting == 0 {
                    ready.insert(next.clone());
                }
            }
        }
        order_ids.push(id);
    }
    if order_ids.len() < units.len() {
        // A plugin is on a cycle when walking forward from it, through unplaced plugins,
        // leads back to it. Plugins that only wait on a cycle are not named.
        let placed: BTreeSet<PluginId> = order_ids.iter().cloned().collect();
        let mut on_cycle: Vec<PluginId> = Vec::new();
        for start in units.keys().filter(|id| !placed.contains(*id)) {
            let mut stack: Vec<&PluginId> = unlocks.get(start).into_iter().flatten().collect();
            let mut seen: BTreeSet<&PluginId> = BTreeSet::new();
            let mut found = false;
            while let Some(node) = stack.pop() {
                if node == start {
                    found = true;
                    break;
                }
                if placed.contains(node) || !seen.insert(node) {
                    continue;
                }
                stack.extend(unlocks.get(node).into_iter().flatten());
            }
            if found {
                on_cycle.push(start.clone());
            }
        }
        errors.push(ResolveError::Cycle { plugins: on_cycle });
    }

    // Rule 6: the profile's choices are checked first, then each exclusive seam gets its
    // provider.
    let mut chosen: BTreeMap<String, PluginId> = BTreeMap::new();
    for (key, choice) in &profile.seams {
        let (name, target) = match key.split_once('@') {
            Some((name, target)) => (name, Some(target)),
            None => (key.as_str(), None),
        };
        let Some(decl) = declared(name) else {
            errors.push(ResolveError::UnknownSeam {
                plugin: choice.clone(),
                seam: key.clone(),
            });
            continue;
        };
        if let Some(target) = target {
            if !decl.per_target {
                errors.push(ResolveError::TargetNotAllowed {
                    plugin: choice.clone(),
                    seam: name.to_owned(),
                    target: target.to_owned(),
                });
                continue;
            }
        }
        let providers = match target {
            Some(target) => exclusive_targeted.get(&(decl.name, target.to_owned())),
            None => exclusive_whole.get(decl.name),
        };
        let names_provider = providers.is_some_and(|set| set.contains(choice));
        if decl.kind == SeamKind::Exclusive && names_provider {
            chosen.insert(key.clone(), choice.clone());
        } else {
            errors.push(ResolveError::BadSeamChoice {
                seam: key.clone(),
                plugin: choice.clone(),
            });
        }
    }

    let mut exclusive: BTreeMap<&'static str, ExclusiveSeam> = BTreeMap::new();
    for decl in SEAMS.iter().filter(|decl| decl.kind == SeamKind::Exclusive) {
        let whole_providers = exclusive_whole
            .get(decl.name)
            .cloned()
            .unwrap_or_default();
        let targets: Vec<(String, BTreeSet<PluginId>)> = exclusive_targeted
            .iter()
            .filter(|((name, _), _)| *name == decl.name)
            .map(|((_, target), providers)| (target.clone(), providers.clone()))
            .collect();
        if whole_providers.is_empty() && targets.is_empty() {
            continue;
        }
        let mut seam = ExclusiveSeam {
            whole: Provider::Game,
            targets: BTreeMap::new(),
        };
        if let Some(provider) = choose(decl.name, &whole_providers, &chosen, &mut errors) {
            seam.whole = provider;
        }
        for (target, providers) in targets {
            let key = format!("{}@{}", decl.name, target);
            if let Some(provider) = choose(&key, &providers, &chosen, &mut errors) {
                seam.targets.insert(target, provider);
            }
        }
        exclusive.insert(decl.name, seam);
    }

    // Multi seams list their plugin providers in load order.
    let mut multi: BTreeMap<&'static str, Vec<PluginId>> = BTreeMap::new();
    for id in &order_ids {
        for provided in &units[id].manifest.provides {
            if let Some(decl) = declared(&provided.seam) {
                if decl.kind == SeamKind::Multi && provided.target.is_none() {
                    let providers = multi.entry(decl.name).or_default();
                    if !providers.contains(id) {
                        providers.push(id.clone());
                    }
                }
            }
        }
    }

    // Each enabled plugin's settings: the manifest defaults, with the profile's top-level
    // keys replacing them.
    let mut config: BTreeMap<PluginId, toml::Table> = BTreeMap::new();
    for id in &order_ids {
        let mut settings = units[id].manifest.config.clone();
        if let Some(entry) = profile.plugins.get(id) {
            for (key, value) in entry.config.iter() {
                settings.insert(key.clone(), value.clone());
            }
        }
        config.insert(id.clone(), settings);
    }

    errors.sort_by(|a, b| a.to_string().cmp(&b.to_string()));
    if !errors.is_empty() {
        return Err(errors);
    }
    let order: Vec<Installed> = order_ids.iter().map(|id| units[id].clone()).collect();
    Ok(Resolved {
        order,
        seams: SeamTable { exclusive, multi },
        config,
    })
}

/// The provider of one exclusive seam or seam@target: the profile's choice when it names
/// one, else the only provider, else the game. Several providers with no choice are
/// recorded as a contested seam.
fn choose(
    key: &str,
    providers: &BTreeSet<PluginId>,
    chosen: &BTreeMap<String, PluginId>,
    errors: &mut Vec<ResolveError>,
) -> Option<Provider> {
    if let Some(choice) = chosen.get(key) {
        return Some(Provider::Plugin(choice.clone()));
    }
    match providers.len() {
        0 => Some(Provider::Game),
        1 => providers.iter().next().cloned().map(Provider::Plugin),
        _ => {
            errors.push(ResolveError::SeamContested {
                seam: key.to_owned(),
                providers: providers.iter().cloned().collect(),
            });
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A native plugin at version 0.1.0, with `tail` (its tables) appended after `[plugin]`.
    fn plugin(id: &str, tail: &str) -> Installed {
        let text = format!(
            "[plugin]\nid = \"{id}\"\nname = \"test\"\nversion = \"0.1.0\"\nmoly = \">=0.0.5\"\nkind = \"native\"\n{tail}"
        );
        Installed {
            dir: PathBuf::from(id),
            manifest: Manifest::parse(&text).expect("test manifest parses"),
        }
    }

    fn parse_profile(text: &str) -> Profile {
        Profile::parse(text).expect("test profile parses")
    }

    fn running() -> Version {
        Version::parse("0.0.5").expect("test version parses")
    }

    #[test]
    fn dependency_is_loaded_before_its_dependent() {
        let available = [
            plugin("moly.app", "[depends]\n\"moly.lib\" = \">=0.1\"\n"),
            plugin("moly.lib", ""),
        ];
        let profile = parse_profile(
            "[plugins.\"moly.app\"]\nenabled = true\n[plugins.\"moly.lib\"]\nenabled = true\n",
        );
        let resolved = resolve(&available, &profile, &running()).expect("resolves");
        let order: Vec<&str> = resolved
            .order
            .iter()
            .map(|item| item.manifest.id.as_str())
            .collect();
        assert_eq!(order, ["moly.lib", "moly.app"]);
    }

    #[test]
    fn missing_dependency_is_refused() {
        let available = [plugin("moly.app", "[depends]\n\"moly.lib\" = \"*\"\n")];
        let profile = parse_profile("[plugins.\"moly.app\"]\nenabled = true\n");
        let errors = resolve(&available, &profile, &running()).expect_err("refused");
        assert!(errors
            .iter()
            .any(|error| matches!(error, ResolveError::MissingDependency { .. })));
    }

    #[test]
    fn conflicting_plugins_are_refused() {
        let available = [
            plugin("moly.a", "[conflicts]\n\"moly.b\" = \"*\"\n"),
            plugin("moly.b", ""),
        ];
        let profile = parse_profile(
            "[plugins.\"moly.a\"]\nenabled = true\n[plugins.\"moly.b\"]\nenabled = true\n",
        );
        let errors = resolve(&available, &profile, &running()).expect_err("refused");
        assert!(errors
            .iter()
            .any(|error| matches!(error, ResolveError::Conflict { .. })));
    }

    #[test]
    fn contested_exclusive_seam_is_refused() {
        let available = [
            plugin("moly.a", "[[provides]]\nseam = \"companion.brain\"\n"),
            plugin("moly.b", "[[provides]]\nseam = \"companion.brain\"\n"),
        ];
        let profile = parse_profile(
            "[plugins.\"moly.a\"]\nenabled = true\n[plugins.\"moly.b\"]\nenabled = true\n",
        );
        let errors = resolve(&available, &profile, &running()).expect_err("refused");
        assert!(errors
            .iter()
            .any(|error| matches!(error, ResolveError::SeamContested { .. })));
    }

    #[test]
    fn profile_choice_resolves_contested_seam() {
        let available = [
            plugin("moly.a", "[[provides]]\nseam = \"companion.brain\"\n"),
            plugin("moly.b", "[[provides]]\nseam = \"companion.brain\"\n"),
        ];
        let profile = parse_profile(
            "[plugins.\"moly.a\"]\nenabled = true\n[plugins.\"moly.b\"]\nenabled = true\n\
             [seams]\n\"companion.brain\" = \"moly.b\"\n",
        );
        let resolved = resolve(&available, &profile, &running()).expect("resolves");
        let winner = PluginId::new("moly.b").expect("test id is valid");
        assert_eq!(
            resolved.seams.provider("companion.brain", None),
            Provider::Plugin(winner)
        );
    }

    #[test]
    fn undeclared_seam_is_refused() {
        let available = [plugin("moly.a", "[[provides]]\nseam = \"no.such.seam\"\n")];
        let profile = parse_profile("[plugins.\"moly.a\"]\nenabled = true\n");
        let errors = resolve(&available, &profile, &running()).expect_err("refused");
        assert!(errors
            .iter()
            .any(|error| matches!(error, ResolveError::UnknownSeam { .. })));
    }

    #[test]
    fn dependency_cycle_is_refused() {
        let available = [
            plugin("moly.a", "[depends]\n\"moly.b\" = \"*\"\n"),
            plugin("moly.b", "[depends]\n\"moly.a\" = \"*\"\n"),
        ];
        let profile = parse_profile(
            "[plugins.\"moly.a\"]\nenabled = true\n[plugins.\"moly.b\"]\nenabled = true\n",
        );
        let errors = resolve(&available, &profile, &running()).expect_err("refused");
        assert!(errors
            .iter()
            .any(|error| matches!(error, ResolveError::Cycle { .. })));
    }

    #[test]
    fn moly_version_mismatch_is_refused() {
        let text = "[plugin]\nid = \"moly.a\"\nname = \"test\"\nversion = \"0.1.0\"\nmoly = \">=9.0.0\"\nkind = \"native\"\n";
        let available = [Installed {
            dir: PathBuf::from("moly.a"),
            manifest: Manifest::parse(text).expect("test manifest parses"),
        }];
        let profile = parse_profile("[plugins.\"moly.a\"]\nenabled = true\n");
        let errors = resolve(&available, &profile, &running()).expect_err("refused");
        assert!(errors
            .iter()
            .any(|error| matches!(error, ResolveError::MolyVersion { .. })));
    }
}
