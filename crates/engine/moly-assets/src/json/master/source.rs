//! The asset root's master identity and the paths it permits. A complete
//! producer pin selects an immutable commit on the already admitted regional
//! repository; it cannot supply a new host or fall back to moving mirrors.

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
    data_version: String,
    asset_version: String,
    asset_hash: String,
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
    pub(super) fn parse_version(&self, text: &str) -> Result<String, String> {
        let value: Value =
            serde_json::from_str(text).map_err(|error| format!("is not JSON ({error})"))?;
        let version = required(&value, "dataVersion", "upstream version")?;
        if let Some(pin) = &self.pin {
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

fn required<'a>(value: &'a Value, field: &str, owner: &str) -> Result<&'a str, String> {
    value[field]
        .as_str()
        .filter(|text| !text.is_empty())
        .ok_or_else(|| format!("{owner}.{field} is missing or not a nonempty string"))
}

/// A legacy root without master metadata keeps the moving regional sources.
/// Once a pin is declared, a partial, conflicting or foreign pin is refused.
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
    if master
        .get("provider")
        .is_some_and(|provider| provider.as_str() != Some("haruki"))
    {
        return Err("master.provider is not the admitted haruki repository provider".into());
    }
    if required(master, "region", "master")? != name {
        return Err("master.region differs from source.region".into());
    }
    let repository = required(master, "repository", "master")?;
    if repository != format!("Team-Haruki/{}", region.repository()) {
        return Err(format!(
            "master.repository {repository:?} is not the admitted {name} repository"
        ));
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

    fn identity() -> Value {
        json!({
            "version": 1,
            "source": {"region": "jp", "assetVersion": "7.0.0.30", "assetHash": "assets"},
            "master": {
                "provider": "haruki", "repository": "Team-Haruki/haruki-sekai-master",
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
            assert!(source.parse_version(r#"{"dataVersion":"8.0.0"}"#).is_ok());
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
    fn foreign_repositories_regions_and_provider_are_refused() {
        for (field, wrong) in [
            ("repository", "someone/haruki-sekai-master"),
            ("repository", "Team-Haruki/haruki-sekai-sc-master"),
            ("repository", "https://example.org/evil"),
            ("provider", "sekai-world"),
            ("region", "cn"),
        ] {
            let mut value = identity();
            value["master"][field] = json!(wrong);
            assert!(parsed(&value).is_err(), "{field}={wrong}");
        }
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
            source.parse_version(&version.to_string()).unwrap(),
            "7.0.0.32"
        );
        for field in ["dataVersion", "assetVersion", "assetHash"] {
            let mut wrong = version.clone();
            wrong[field] = json!("other");
            assert!(
                source
                    .parse_version(&wrong.to_string())
                    .unwrap_err()
                    .contains(field)
            );
            wrong.as_object_mut().unwrap().remove(field);
            assert!(source.parse_version(&wrong.to_string()).is_err());
        }
    }
}
