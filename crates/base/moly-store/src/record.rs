//! What a resource directory holds: `moly-resources.json`, written by the
//! installer after every file of a catalog is in place and verified.

use serde::{Deserialize, Serialize};
use std::path::Path;

pub const RECORD_FILE: &str = "moly-resources.json";

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Record {
    pub schema: u32,
    pub snapshots: Vec<Installed>,
}

/// One complete catalog in the directory.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Installed {
    pub id: String,
    pub region: String,
    pub version: String,
    pub catalog: String,
    /// The CDN it was downloaded from.
    pub cdn: String,
    /// Seconds since the Unix epoch when the last install finished.
    pub installed_at: u64,
    pub bytes: u64,
}

impl Record {
    pub fn read(dir: &Path) -> Result<Record, String> {
        match std::fs::read(dir.join(RECORD_FILE)) {
            Ok(bytes) => {
                let record: Record = serde_json::from_slice(&bytes)
                    .map_err(|error| format!("{}: {error}", dir.join(RECORD_FILE).display()))?;
                if record.schema != 1 {
                    return Err(format!("{} has schema {}", RECORD_FILE, record.schema));
                }
                Ok(record)
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                Ok(Record { schema: 1, snapshots: Vec::new() })
            }
            Err(error) => Err(format!("{}: {error}", dir.join(RECORD_FILE).display())),
        }
    }

    pub fn write(&self, dir: &Path) -> Result<(), String> {
        let text = serde_json::to_string_pretty(self).map_err(|error| error.to_string())?;
        let path = dir.join(RECORD_FILE);
        let part = dir.join(format!("{RECORD_FILE}.part"));
        std::fs::write(&part, text + "\n")
            .and_then(|()| std::fs::rename(&part, &path))
            .map_err(|error| format!("{}: {error}", path.display()))
    }

    /// Replace the entry with the same catalog, or add it.
    pub fn upsert(&mut self, installed: Installed) {
        self.snapshots.retain(|old| old.catalog != installed.catalog && old.id != installed.id);
        self.snapshots.push(installed);
    }

    /// The entry the game uses by default: the one named by `preferred`
    /// (a snapshot id or a region), else the most recent install.
    pub fn select(&self, preferred: Option<&str>) -> Option<&Installed> {
        if let Some(preferred) = preferred {
            if let Some(found) = self
                .snapshots
                .iter()
                .filter(|item| item.id == preferred || item.region == preferred)
                .max_by_key(|item| item.installed_at)
            {
                return Some(found);
            }
        }
        self.snapshots.iter().max_by_key(|item| item.installed_at)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(id: &str, region: &str, at: u64) -> Installed {
        Installed {
            id: id.into(),
            region: region.into(),
            version: "1".into(),
            catalog: format!("{id:0>64}"),
            cdn: crate::DEFAULT_CDN.into(),
            installed_at: at,
            bytes: 1,
        }
    }

    #[test]
    fn selection_prefers_the_named_snapshot_then_the_newest() {
        let mut record = Record { schema: 1, snapshots: vec![] };
        record.upsert(item("a", "jp", 1));
        record.upsert(item("b", "cn", 2));
        assert_eq!(record.select(None).unwrap().id, "b");
        assert_eq!(record.select(Some("jp")).unwrap().id, "a");
        assert_eq!(record.select(Some("a")).unwrap().id, "a");
        assert_eq!(record.select(Some("kr")).unwrap().id, "b");
        record.upsert(item("a", "jp", 3));
        assert_eq!(record.snapshots.len(), 2);
        assert_eq!(record.select(None).unwrap().id, "a");
    }
}
