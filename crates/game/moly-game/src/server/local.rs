//! The local document: the client-local lists of the application's local
//! settings that the web save keeps (`ApplicationLocalSettings.MysekaiTopics`
//! and `.UnlockedMasterSiteLevelIds`), schemaVersion 1.
//!
//! The source keeps the topics as a map from the topic enum's name to its
//! value; the document keeps the same map. It is client state, not server
//! state: the panel never edits it.

use serde_json::{json, Map, Value};

pub(crate) const SCHEMA_VERSION: u64 = 1;

/// `MysekaiTopicEnum`, name and value.
pub(crate) const TOPICS: [(&str, i32); 4] = [
    ("StaminaRefresh", 0),
    ("SiteExpansion", 1),
    ("HarvestSiteMapReleased", 2),
    ("HarvestSiteDialogReleased", 3),
];

/// `MysekaiTopicEnum.StaminaRefresh`.
pub(crate) const TOPIC_STAMINA_REFRESH: i32 = 0;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct LocalDocument {
    /// Topic values, in the order they were added.
    pub(crate) topics: Vec<i32>,
    pub(crate) unlocked_master_site_level_ids: Vec<i32>,
}

fn topic_name(value: i32) -> Option<&'static str> {
    TOPICS
        .iter()
        .find(|(_, v)| *v == value)
        .map(|(name, _)| *name)
}

impl LocalDocument {
    pub(crate) fn parse(text: &str) -> Result<Self, String> {
        let value: Value = serde_json::from_str(text)
            .map_err(|error| format!("the local document is not JSON: {error}"))?;
        let doc = value
            .as_object()
            .ok_or("the local document is not an object")?;
        let version = doc
            .get("schemaVersion")
            .and_then(Value::as_u64)
            .ok_or("the local document has no integer schemaVersion")?;
        if version != SCHEMA_VERSION {
            return Err(format!(
                "local document schemaVersion {version} is not supported (this engine reads {SCHEMA_VERSION})"
            ));
        }
        if let Some(key) = doc.keys().find(|key| {
            ![
                "schemaVersion",
                "MysekaiTopics",
                "UnlockedMasterSiteLevelIds",
            ]
            .contains(&key.as_str())
        }) {
            return Err(format!("the local document has an unknown field {key}"));
        }
        let topics_map = doc
            .get("MysekaiTopics")
            .and_then(Value::as_object)
            .ok_or("the local document's MysekaiTopics is not an object")?;
        let mut topics = Vec::new();
        for (name, value) in topics_map {
            let value = value
                .as_i64()
                .and_then(|value| i32::try_from(value).ok())
                .ok_or_else(|| format!("MysekaiTopics.{name} is not a topic value"))?;
            if topic_name(value) != Some(name.as_str()) {
                return Err(format!(
                    "MysekaiTopics.{name} = {value} is not a MysekaiTopicEnum name and value"
                ));
            }
            topics.push(value);
        }
        let ids = doc
            .get("UnlockedMasterSiteLevelIds")
            .and_then(Value::as_array)
            .ok_or("the local document's UnlockedMasterSiteLevelIds is not an array")?
            .iter()
            .map(|id| {
                id.as_i64()
                    .and_then(|id| i32::try_from(id).ok())
                    .ok_or_else(|| {
                        format!("UnlockedMasterSiteLevelIds holds {id}, not a site-level id")
                    })
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            topics,
            unlocked_master_site_level_ids: ids,
        })
    }

    pub(crate) fn to_text(&self) -> String {
        let mut topics = Map::new();
        for value in &self.topics {
            if let Some(name) = topic_name(*value) {
                topics.insert(name.to_owned(), json!(value));
            }
        }
        json!({
            "schemaVersion": SCHEMA_VERSION,
            "MysekaiTopics": topics,
            "UnlockedMasterSiteLevelIds": self.unlocked_master_site_level_ids,
        })
        .to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let doc = LocalDocument {
            topics: vec![1, 0],
            unlocked_master_site_level_ids: vec![3, 7],
        };
        let back = LocalDocument::parse(&doc.to_text()).unwrap();
        assert_eq!(back.unlocked_master_site_level_ids, vec![3, 7]);
        let mut topics = back.topics.clone();
        topics.sort_unstable();
        assert_eq!(topics, vec![0, 1]);
    }

    #[test]
    fn a_mismatched_topic_is_refused() {
        let error = LocalDocument::parse(
            r#"{"schemaVersion":1,"MysekaiTopics":{"SiteExpansion":0},"UnlockedMasterSiteLevelIds":[]}"#,
        )
        .unwrap_err();
        assert!(error.contains("SiteExpansion = 0"), "{error}");
    }

    #[test]
    fn a_newer_schema_is_refused() {
        let error = LocalDocument::parse(r#"{"schemaVersion":2}"#).unwrap_err();
        assert!(
            error.contains("schemaVersion 2 is not supported"),
            "{error}"
        );
    }
}
