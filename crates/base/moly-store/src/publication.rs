//! The publication document a CDN serves at `game/manifest.json`.
//!
//! It names the resource snapshots the CDN holds. Paths in it are absolute
//! below the publication `prefix`; [`Snapshot::store_base`] turns one into an
//! address below the CDN base the document was read from.

use serde::Deserialize;

/// The publication document's address below a CDN base.
pub const PUBLICATION_PATH: &str = "game/manifest.json";

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Publication {
    pub schema_version: u32,
    pub prefix: String,
    pub snapshots: Vec<Snapshot>,
}

/// One region's resources at one game version.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub id: String,
    pub region: String,
    pub version: String,
    /// The asset store, absolute below the publication prefix.
    pub assets: String,
    #[serde(default)]
    pub packs: bool,
    #[serde(default)]
    pub asset_catalog: Option<String>,
}

impl Publication {
    pub fn parse(bytes: &[u8]) -> Result<Self, String> {
        let publication: Publication = serde_json::from_slice(bytes)
            .map_err(|error| format!("publication document: {error}"))?;
        if publication.schema_version != 1 {
            return Err(format!(
                "publication document schema {} is not supported",
                publication.schema_version
            ));
        }
        if !publication.prefix.starts_with('/') || !publication.prefix.ends_with('/') {
            return Err(format!("publication prefix {:?} is not a directory", publication.prefix));
        }
        Ok(publication)
    }

    /// The snapshots a local store can hold: packed, with a catalog.
    pub fn installable(&self) -> impl Iterator<Item = &Snapshot> {
        self.snapshots
            .iter()
            .filter(|snapshot| snapshot.packs && snapshot.asset_catalog.as_deref().is_some_and(crate::is_sha256))
    }
}

impl Snapshot {
    /// The asset store's address below `cdn`, which ends with `/`.
    pub fn store_base(&self, cdn: &str, prefix: &str) -> Result<String, String> {
        let relative = self
            .assets
            .strip_prefix(prefix)
            .filter(|rest| rest.ends_with('/') && !rest.split('/').any(|part| part == ".." || part == "."))
            .ok_or_else(|| format!("snapshot {} stores its assets outside the publication: {}", self.id, self.assets))?;
        Ok(format!("{cdn}{relative}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DOCUMENT: &str = r#"{
      "publisher": "test", "schemaVersion": 1, "prefix": "/root/",
      "release": {"id": "r"},
      "snapshots": [
        {"id": "jp-1.0.0-a", "region": "jp", "version": "1.0.0", "assets": "/root/asset-store/",
         "packs": true, "assetCatalog": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"},
        {"id": "cn-1.0.0-b", "region": "cn", "version": "1.0.0", "assets": "/root/snapshots/cn/assets/"}
      ]
    }"#;

    #[test]
    fn installable_snapshots_resolve_below_the_cdn() {
        let publication = Publication::parse(DOCUMENT.as_bytes()).unwrap();
        let installable: Vec<_> = publication.installable().collect();
        assert_eq!(installable.len(), 1);
        assert_eq!(
            installable[0].store_base("https://cdn.example/x/", &publication.prefix).unwrap(),
            "https://cdn.example/x/asset-store/"
        );
    }

    #[test]
    fn stores_outside_the_prefix_are_refused() {
        let mut snapshot = Publication::parse(DOCUMENT.as_bytes()).unwrap().snapshots[0].clone();
        snapshot.assets = "/other/asset-store/".into();
        assert!(snapshot.store_base("https://cdn.example/", "/root/").is_err());
        snapshot.assets = "/root/../asset-store/".into();
        assert!(snapshot.store_base("https://cdn.example/", "/root/").is_err());
    }
}
