//! The asset root's master identity and the paths it permits. A complete
//! producer pin selects an immutable commit of the region's repository on
//! the already admitted repository host; it cannot supply a new host or fall
//! back to moving mirrors. The repository the pin names is the one the
//! configuration gives the region (`masters.repository_<region>`); a pin that
//! names another, or a region without a configured name, is refused by name.

use bevy::asset::AssetPath;
use serde_json::Value;

use crate::remote::{self, Region, Remote};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct MasterSource {
    pub(super) region: Region,
    pub(super) pin: Option<MasterPin>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct MasterPin {
    pub(super) commit: String,
    /// The pin's `owning-repository/name`; its name must be the region's
    /// configured repository name.
    pub(super) repository: String,
    data_version: String,
    asset_version: String,
    asset_hash: String,
}

impl MasterPin {
    /// The pin names the repository `repository` (the configured name of its
    /// region) or no repository this runtime reads.
    fn names(&self, repository: &str) -> bool {
        self.repository
            .split_once('/')
            .is_some_and(|(_, name)| name == repository)
    }
}

impl MasterSource {
    pub(super) fn pinned(&self, region: Region) -> bool {
        self.pin.is_some() && self.region == region
    }

    pub(super) fn admits(&self, host: Remote, region: Region) -> bool {
        host.is_master() && (!self.pinned(region) || host == Remote::MasterRepository)
    }

    pub(super) fn table(
        &self,
        host: Remote,
        region: Region,
        table: &str,
    ) -> Option<AssetPath<'static>> {
        if !self.admits(host, region) {
            return None;
        }
        if region == self.region {
            if let Some(pin) = &self.pin {
                return Some(remote::master_table_at(region, &pin.commit, table));
            }
        }
        Some(remote::master_table(host, region, table))
    }

    pub(super) fn version(&self, host: Remote) -> Option<AssetPath<'static>> {
        if !self.admits(host, self.region) {
            return None;
        }
        Some(match &self.pin {
            Some(pin) => remote::master_version_at(self.region, &pin.commit),
            None => remote::master_version(host, self.region),
        })
    }

    /// A pinned document must agree with both the producer's master identity
    /// and its bundle identity before any master table may be used.
    /// `repository` is the region's configured repository name.
    pub(super) fn parse_version(
        &self,
        text: &str,
        repository: &str,
    ) -> Result<String, String> {
        let value: Value =
            serde_json::from_str(text).map_err(|error| format!("is not JSON ({error})"))?;
        let version = required(&value, "dataVersion", "upstream version")?;
        if let Some(pin) = &self.pin {
            if !pin.names(repository) {
                return Err(pin_repository_refusal(pin, self.region, repository));
            }
            for (field, expected) in [
                ("dataVersion", pin.data_version.as_str()),
                ("assetVersion", pin.asset_version.as_str()),
                ("assetHash", pin.asset_hash.as_str()),
            ] {
                if value[field].as_str() != Some(expected) {
                    return Err(format!(
                        "commit {} has {field} {:?}, expected {expected:?} from source.json",
                        pin.commit, value[field]
                    ));
                }
            }
        }
        Ok(version.to_owned())
    }
}

/// The refusal of a pin whose repository is not the region's configured one,
/// naming the repository it read and the key that names the expected one.
fn pin_repository_refusal(pin: &MasterPin, region: Region, repository: &str) -> String {
    format!(
        "commit {} of master.repository {:?} is not the repository {:?} named by {}",
        pin.commit,
        pin.repository,
        repository,
        region.repository_key()
    )
}

fn required<'a>(value: &'a Value, field: &str, owner: &str) -> Result<&'a str, String> {
    value[field]
        .as_str()
        .filter(|text| !text.is_empty())
        .ok_or_else(|| format!("{owner}.{field} is missing or not a nonempty string"))
}

/// A legacy root without master metadata keeps the moving regional sources.
/// Once a pin is declared, a partial or conflicting pin is refused. The pin's
/// repository name is checked against the configuration when its commit is
/// read ([`MasterSource::parse_version`]).
pub(super) fn parse_source(text: &str) -> Result<MasterSource, String> {
    let value: Value =
        serde_json::from_str(text).map_err(|error| format!("is not JSON ({error})"))?;
    if value["version"].as_u64() != Some(1) {
        return Err("has an unsupported or missing identity version".into());
    }
    let name = required(&value["source"], "region", "source")?;
    let region = Region::from_directory(name)
        .ok_or_else(|| format!("names the source region {name:?}, not a region directory"))?;
    let mut source = MasterSource { region, pin: None };
    let Some(master) = value.get("master").filter(|master| !master.is_null()) else {
        return Ok(source);
    };
    if required(master, "region", "master")? != name {
        return Err("master.region differs from source.region".into());
    }
    let repository = required(master, "repository", "master")?;
    let (owner, repo) = repository
        .split_once('/')
        .ok_or_else(|| "master.repository must be owner/name".to_owned())?;
    if owner.is_empty() || repo.is_empty() || repo.contains('/') {
        return Err("master.repository must be owner/name, with both parts non-empty".into());
    }
    let commit = required(master, "commit", "master")?;
    if commit.len() != 40
        || !commit
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err("master.commit must be a full lowercase 40-digit Git commit".into());
    }
    let data_version = required(master, "dataVersion", "master")?.to_owned();
    let asset_version = required(master, "assetVersion", "master")?.to_owned();
    let asset_hash = required(master, "assetHash", "master")?.to_owned();
    for (field, expected) in [("assetVersion", &asset_version), ("assetHash", &asset_hash)] {
        if required(&value["source"], field, "source")? != expected.as_str() {
            return Err(format!("master.{field} differs from source.{field}"));
        }
    }
    source.pin = Some(MasterPin {
        commit: commit.to_owned(),
        repository: repository.to_owned(),
        data_version,
        asset_version,
        asset_hash,
    });
    Ok(source)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// The configured repository name of each region.
    const JP: &str = "example-jp";
    const CN: &str = "example-cn";

    fn identity() -> Value {
        json!({
            "version": 1,
            "source": {"region": "jp", "assetVersion": "7.0.0.30", "assetHash": "assets"},
            "master": {
                "repository": "example-owner/example-jp",
                "region": "jp", "commit": "0123456789abcdef0123456789abcdef01234567",
                "dataVersion": "7.0.0.32", "assetVersion": "7.0.0.30", "assetHash": "assets"
            }
        })
    }

    fn parsed(value: &Value) -> Result<MasterSource, String> {
        parse_source(&value.to_string())
    }

    #[test]
    fn legacy_roots_keep_latest_repository_and_mirrors() {
        for region in ["jp", "cn"] {
            let source = parsed(&json!({"version": 1, "source": {"region": region}})).unwrap();
            assert_eq!(source.pin, None);
            for host in [
                Remote::MasterRepository,
                Remote::Master,
                Remote::MasterMirror,
            ] {
                assert_eq!(
                    source.table(host, Region::Jp, "configs"),
                    Some(remote::master_table(host, Region::Jp, "configs"))
                );
                assert_eq!(
                    source.version(host),
                    Some(remote::master_version(host, source.region))
                );
            }
            assert!(source.parse_version(r#"{"dataVersion":"8.0.0"}"#, JP).is_ok());
        }
    }

    #[test]
    fn a_pin_selects_commit_paths_and_excludes_moving_mirrors() {
        let source = parsed(&identity()).unwrap();
        let commit = source.pin.as_ref().unwrap().commit.as_str();
        assert_eq!(
            source.table(Remote::MasterRepository, Region::Jp, "configs"),
            Some(remote::master_table_at(Region::Jp, commit, "configs"))
        );
        assert_eq!(
            source.version(Remote::MasterRepository),
            Some(remote::master_version_at(Region::Jp, commit))
        );
        for host in [Remote::Master, Remote::MasterMirror, Remote::Storage] {
            assert_eq!(source.table(host, Region::Jp, "configs"), None);
            assert_eq!(source.version(host), None);
        }
        // The root's pin does not silently select another region's version.
        assert_eq!(
            source.table(Remote::Master, Region::Cn, "configs"),
            Some(remote::master_table(Remote::Master, Region::Cn, "configs"))
        );
    }

    #[test]
    fn partial_pins_are_not_treated_as_legacy_roots() {
        for field in [
            "repository",
            "region",
            "commit",
            "dataVersion",
            "assetVersion",
            "assetHash",
        ] {
            let mut value = identity();
            value["master"].as_object_mut().unwrap().remove(field);
            assert!(parsed(&value).is_err(), "missing {field}");
        }
        for field in ["assetVersion", "assetHash"] {
            let mut value = identity();
            value["source"].as_object_mut().unwrap().remove(field);
            assert!(parsed(&value).is_err(), "missing source.{field}");
        }
        let mut value = identity();
        value["master"] = json!({});
        assert!(parsed(&value).is_err());
    }

    #[test]
    fn regions_that_differ_or_repositories_that_are_not_owner_slash_name_are_refused() {
        for wrong in [
            "example-owner",
            "/example-jp",
            "example-owner/",
            "example-owner/example-jp/extra",
            "https://example.org/evil",
        ] {
            let mut value = identity();
            value["master"]["repository"] = json!(wrong);
            assert!(parsed(&value).is_err(), "{wrong}");
        }
        // The record names the region of its source.
        let mut value = identity();
        value["master"]["region"] = json!("cn");
        assert!(parsed(&value).is_err());
        // The record needs no provider: a foreign name is read like any.
        let mut value = identity();
        value["master"]["provider"] = json!("someone-else");
        assert!(parsed(&value).is_ok());
    }

    #[test]
    fn a_pin_names_the_configured_repository_of_its_region() {
        let source = parsed(&identity()).unwrap();
        let version =
            r#"{"dataVersion":"7.0.0.32","assetVersion":"7.0.0.30","assetHash":"assets"}"#;
        assert_eq!(source.parse_version(version, JP).unwrap(), "7.0.0.32");
        // Another region's configured name is not this pin's repository.
        let error = source.parse_version(version, CN).unwrap_err();
        assert!(
            error.contains("example-owner/example-jp") && error.contains("masters.repository_jp"),
            "{error}"
        );
        // The owner is the pin's own: only the repository name is configured.
        let mut value = identity();
        value["master"]["repository"] = json!("another-owner/example-jp");
        let source = parsed(&value).unwrap();
        assert!(source.parse_version(version, JP).is_ok());
    }

    #[test]
    fn only_immutable_plain_commits_are_accepted() {
        for commit in [
            "main",
            "abcd",
            "../main",
            "0123456789ABCDEF0123456789ABCDEF01234567",
            "0123456789abcdef0123456789abcdef0123456/",
        ] {
            let mut value = identity();
            value["master"]["commit"] = json!(commit);
            assert!(parsed(&value).is_err(), "{commit}");
        }
    }

    #[test]
    fn producer_and_bundle_identity_must_agree() {
        for field in ["assetVersion", "assetHash"] {
            let mut value = identity();
            value["master"][field] = json!("other");
            assert!(parsed(&value).unwrap_err().contains(field));
        }
    }

    #[test]
    fn upstream_version_must_match_every_pinned_field() {
        let source = parsed(&identity()).unwrap();
        let version =
            json!({"dataVersion":"7.0.0.32", "assetVersion":"7.0.0.30", "assetHash":"assets"});
        assert_eq!(
            source.parse_version(&version.to_string(), JP).unwrap(),
            "7.0.0.32"
        );
        for field in ["dataVersion", "assetVersion", "assetHash"] {
            let mut wrong = version.clone();
            wrong[field] = json!("other");
            assert!(
                source
                    .parse_version(&wrong.to_string(), JP)
                    .unwrap_err()
                    .contains(field)
            );
            wrong.as_object_mut().unwrap().remove(field);
            assert!(source.parse_version(&wrong.to_string(), JP).is_err());
        }
    }
}
