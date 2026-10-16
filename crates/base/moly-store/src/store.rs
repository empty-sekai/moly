//! The asset store layout, as far as copying it needs.
//!
//! `catalogs/<sha256>.json` lists packages; each package manifest
//! `packages/<sha256>.json` lists blobs `blobs/<xx>/<sha256>.bin`. Every file is
//! named by the SHA-256 of its bytes. The game's reader checks the documents in
//! full when it opens them; this module reads only the addresses, sizes and
//! digests.

use serde::Deserialize;
use std::collections::BTreeMap;

#[derive(Debug, Deserialize)]
pub struct Catalog {
    pub schema: String,
    #[serde(default)]
    pub region: Option<String>,
    #[serde(default)]
    pub version: Option<String>,
    pub packages: Vec<Package>,
}

#[derive(Debug, Deserialize)]
pub struct Package {
    pub id: String,
    pub kind: String,
    pub download_bytes: u64,
    pub manifest: String,
}

#[derive(Debug, Deserialize)]
pub struct Manifest {
    pub schema: String,
    pub entries: Vec<Entry>,
}

#[derive(Debug, Deserialize)]
pub struct Entry {
    pub blob: String,
    pub blob_sha256: String,
    pub blob_bytes: u64,
}

/// The catalog document's key.
pub fn catalog_key(id: &str) -> String {
    format!("catalogs/{id}.json")
}

/// A store key is a relative path of plain parts below `catalogs/`,
/// `packages/` or `blobs/`.
pub fn check_key(key: &str) -> Result<(), String> {
    let mut parts = key.split('/');
    let top = parts.next().unwrap_or("");
    let ok = matches!(top, "catalogs" | "packages" | "blobs")
        && key.split('/').skip(1).all(|part| {
            !part.is_empty()
                && part != "."
                && part != ".."
                && part.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
        })
        && key.split('/').count() >= 2;
    ok.then_some(()).ok_or_else(|| format!("store key {key:?} is not a plain store path"))
}

/// The SHA-256 a manifest key names (`packages/<sha256>.json`).
pub fn manifest_digest(key: &str) -> Result<&str, String> {
    key.strip_prefix("packages/")
        .and_then(|rest| rest.strip_suffix(".json"))
        .filter(|digest| crate::is_sha256(digest))
        .ok_or_else(|| format!("package manifest {key:?} is not named by its SHA-256"))
}

impl Catalog {
    pub fn parse(bytes: &[u8]) -> Result<Self, String> {
        let catalog: Catalog =
            serde_json::from_slice(bytes).map_err(|error| format!("catalog: {error}"))?;
        if catalog.schema != "moly-asset-packs/2" {
            return Err(format!("catalog schema {:?} is not supported", catalog.schema));
        }
        for package in &catalog.packages {
            check_key(&package.manifest)?;
            manifest_digest(&package.manifest)?;
        }
        Ok(catalog)
    }

    /// Download bytes per package kind.
    pub fn bytes_by_kind(&self) -> BTreeMap<String, u64> {
        let mut out = BTreeMap::new();
        for package in &self.packages {
            *out.entry(package.kind.clone()).or_default() += package.download_bytes;
        }
        out
    }
}

impl Manifest {
    pub fn parse(bytes: &[u8]) -> Result<Self, String> {
        let manifest: Manifest =
            serde_json::from_slice(bytes).map_err(|error| format!("package manifest: {error}"))?;
        if manifest.schema != "moly-asset-manifest/2" {
            return Err(format!("package manifest schema {:?} is not supported", manifest.schema));
        }
        for entry in &manifest.entries {
            if !crate::is_sha256(&entry.blob_sha256) {
                return Err(format!("blob {} has no SHA-256", entry.blob));
            }
            let expected = format!("{}/{}.bin", &entry.blob_sha256[..2], entry.blob_sha256);
            if entry.blob != expected {
                return Err(format!("blob {} is not stored under its digest", entry.blob));
            }
        }
        Ok(manifest)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_stay_inside_the_store() {
        assert!(check_key("blobs/ab/ab.bin").is_ok());
        assert!(check_key(&catalog_key(&"a".repeat(64))).is_ok());
        for bad in ["blobs/../x", "other/x", "blobs", "blobs//x", "blobs/a b", "/blobs/x", "blobs/x\\y"] {
            assert!(check_key(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn manifests_are_named_by_their_digest() {
        let digest = "b".repeat(64);
        assert_eq!(manifest_digest(&format!("packages/{digest}.json")).unwrap(), digest);
        assert!(manifest_digest("packages/short.json").is_err());
    }

    #[test]
    fn blobs_must_sit_under_their_digest() {
        let digest = "c".repeat(64);
        let good = format!(
            r#"{{"schema":"moly-asset-manifest/2","entries":[{{"blob":"cc/{digest}.bin","blob_sha256":"{digest}","blob_bytes":1}}]}}"#
        );
        assert!(Manifest::parse(good.as_bytes()).is_ok());
        let moved = good.replace("cc/", "dd/");
        assert!(Manifest::parse(moved.as_bytes()).is_err());
    }
}
