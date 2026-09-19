//! Source-route icon receipts, not a phenomenon-name-to-artwork table.
//! The full provenance is checked once on index load. Only the immutable PNG
//! artifact descriptor is published to the browser, which verifies its bytes.
use crate::weather_timeline::{SourceObjectIdentity, SourceReference};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct WeatherIconArtifact {
    pub file: String,
    pub sha256: String,
    pub bytes: u64,
    pub width: u32,
    pub height: u32,
}
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WeatherIconReceipt {
    pub schema_version: u32,
    #[serde(flatten)]
    pub artifact: WeatherIconArtifact,
    pub decoding: IconDecoding,
    pub sources: Vec<IconSource>,
}
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IconDecoding {
    pub unity_py: String,
    pub pillow: String,
    pub format: String,
    pub compression_level: u32,
}
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IconSource {
    #[serde(flatten)]
    pub identity: SourceObjectIdentity,
    pub asset: String,
    pub asset_path: String,
    pub object_name: String,
    pub class: String,
    pub reference: SourceReference,
    pub references: Vec<IconBackingReference>,
    pub inputs: Vec<IconInput>,
}
#[derive(Debug, Clone, Deserialize)]
pub struct IconBackingReference {
    pub field: String,
    #[serde(flatten)]
    pub reference: SourceReference,
}
#[derive(Debug, Clone, Deserialize)]
pub struct IconInput {
    pub bundle: String,
    pub bytes: u64,
    pub sha256: String,
}
fn hash_valid(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn valid_key(key: &str) -> bool {
    !key.is_empty()
        && key != "."
        && key != ".."
        && !key
            .chars()
            .any(|c| c.is_control() || "/\\:%?#*<>|".contains(c))
}
impl WeatherIconArtifact {
    pub fn validate(&self, key: &str) -> Result<(), String> {
        if !valid_key(key)
            || self.file != format!("icons/{key}.png")
            || !hash_valid(&self.sha256)
            || !(33..=4 * 1024 * 1024).contains(&self.bytes)
            || self.width == 0
            || self.height == 0
            || self.width > 8192
            || self.height > 8192
        {
            return Err("invalid source icon artifact descriptor".into());
        }
        Ok(())
    }
    pub fn verify_png(&self, bytes: &[u8]) -> Result<(), String> {
        if bytes.len() as u64 != self.bytes || format!("{:x}", Sha256::digest(bytes)) != self.sha256
        {
            return Err("source icon PNG byte count or hash mismatch".into());
        }
        if bytes.len() < 33
            || bytes[..8] != [137, 80, 78, 71, 13, 10, 26, 10]
            || bytes[8..12] != [0, 0, 0, 13]
            || &bytes[12..16] != b"IHDR"
            || u32::from_be_bytes(bytes[16..20].try_into().unwrap()) != self.width
            || u32::from_be_bytes(bytes[20..24].try_into().unwrap()) != self.height
        {
            return Err("source icon PNG dimensions or header mismatch".into());
        }
        Ok(())
    }
}
impl WeatherIconReceipt {
    pub fn validate(&self, key: &str) -> Result<(), String> {
        self.artifact.validate(key)?;
        if self.schema_version != 2
            || self.sources.is_empty()
            || self.decoding.format != "PNG"
            || self.decoding.unity_py.is_empty()
            || self.decoding.pillow.is_empty()
            || self.decoding.compression_level > 9
        {
            return Err("unsupported or unproven source icon receipt".into());
        }
        for source in &self.sources {
            source.identity.validate()?;
            source.reference.validate()?;
            let path = source.asset_path.replace('\\', "/");
            let asset = path.rsplit('/').next().unwrap_or_default();
            let stem = asset.rsplit_once('.').map_or(asset, |(stem, _)| stem);
            if stem != key
                || asset != source.asset
                || source.reference.target.as_ref() != Some(&source.identity)
                || !matches!(source.class.as_str(), "Texture2D" | "Sprite")
            {
                return Err("source icon route, class and resolved object disagree".into());
            }
            let mut bundles = BTreeSet::new();
            for input in &source.inputs {
                if input.bundle.is_empty()
                    || input.bytes == 0
                    || !hash_valid(&input.sha256)
                    || !bundles.insert(input.bundle.as_str())
                {
                    return Err("missing or conflicting source icon input receipts".into());
                }
            }
            if !bundles.contains(source.identity.bundle.as_str())
                || !bundles.contains(source.reference.owner.bundle.as_str())
            {
                return Err("source icon owner or target input not receipted".into());
            }
            let mut fields = BTreeSet::new();
            for backing in &source.references {
                backing.reference.validate()?;
                if backing.field.is_empty()
                    || !fields.insert(backing.field.as_str())
                    || backing.reference.owner != source.identity
                    || backing
                        .reference
                        .target
                        .as_ref()
                        .is_some_and(|target| !bundles.contains(target.bundle.as_str()))
                {
                    return Err("invalid or unreceipted backing icon reference".into());
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn artifact() -> (WeatherIconArtifact, Vec<u8>) {
        let mut bytes = vec![
            137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, b'I', b'H', b'D', b'R',
        ];
        bytes.extend_from_slice(&152u32.to_be_bytes());
        bytes.extend_from_slice(&152u32.to_be_bytes());
        bytes.extend_from_slice(&[8, 6, 0, 0, 0, 0, 0, 0, 0]);
        (
            WeatherIconArtifact {
                file: "icons/key.png".into(),
                sha256: format!("{:x}", Sha256::digest(&bytes)),
                bytes: bytes.len() as u64,
                width: 152,
                height: 152,
            },
            bytes,
        )
    }
    #[test]
    fn artifact_must_match_hash_size_and_source_dimensions() {
        let (mut source, bytes) = artifact();
        source.validate("key").unwrap();
        source.verify_png(&bytes).unwrap();
        let mut damaged = bytes.clone();
        damaged[30] ^= 1;
        assert!(source.verify_png(&damaged).is_err());
        source.width = 153;
        assert!(source.verify_png(&bytes).is_err());
    }
    #[test]
    fn artwork_cannot_be_selected_through_a_label_or_unsafe_route() {
        let (mut source, _) = artifact();
        assert!(source.validate("other_label").is_err());
        for key in ["../key", "%2e%2e", "foo:bar", "", "a/b", "a\\b", "x?y"] {
            source.file = format!("icons/{key}.png");
            assert!(source.validate(key).is_err());
        }
    }
    #[test]
    #[ignore = "requires MOLY_WEATHER_SOURCE_ROOT with fresh producer output"]
    fn current_source_icon_routes_and_pngs_are_verified() {
        let root = std::path::PathBuf::from(std::env::var("MOLY_WEATHER_SOURCE_ROOT").unwrap());
        let index = crate::weather_index::PhenomenonIndex::from_bytes(
            &std::fs::read(root.join("index.json")).unwrap(),
        )
        .unwrap();
        assert!(!index.icon_sources.is_empty());
        for (key, source) in &index.icon_sources {
            source.validate(key).unwrap();
            source
                .artifact
                .verify_png(&std::fs::read(root.join(&source.artifact.file)).unwrap())
                .unwrap();
            let mut altered = source.clone();
            altered.sources[0].reference.pointer.path_id = "2".into();
            assert!(altered.validate(key).is_err());
            println!(
                "verified source icon {key}: {}x{}",
                source.artifact.width, source.artifact.height
            );
        }
    }
}
