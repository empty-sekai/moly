//! The tweet master and the greeting tables, as the NPC states read them.
//!
//! The tweet state resolves its row by the AI model's tweet id, and the
//! greeting state picks its greeting from the greeting, owner and condition
//! tables. Both come from the two extracted files the balloon renders from
//! (`tweets.json` for the tweet rows and the after-edit pools,
//! `tweet-tables.json` for the greeting chain). A load failure panics at the
//! asset boundary; a row without a required key panics with its id only (the
//! text never goes into a message).

use bevy::asset::{AssetPath, LoadState};
use bevy::prelude::*;
use moly_assets::json::JsonAsset;
use moly_law::tweet::{GreetingConditionRow, GreetingRow, TweetRow, WithoutRelatedTalkRow};

/// The load request for both files; removed once both are parsed.
#[derive(Resource)]
pub(crate) struct TweetTableHandles {
    tweets: Handle<JsonAsset>,
    tables: Handle<JsonAsset>,
}

/// The parsed tables.
#[derive(Resource)]
pub(crate) struct TweetTables {
    /// Every tweet row, in ascending id order (the master's key order).
    tweets: Vec<TweetRow>,
    /// The owner rows that give each greeting (and each site-entry and
    /// after-edit tweet) its character and tweet id.
    pub(crate) wrt: Vec<WithoutRelatedTalkRow>,
    /// The greeting rows, in master order.
    pub(crate) greetings: Vec<GreetingRow>,
    /// The greeting condition rows.
    pub(crate) conditions: Vec<GreetingConditionRow>,
    /// The after-edit pools as extracted: character unit -> tweet ids (read
    /// by the balloon's after-edit reaction through its seam).
    #[allow(dead_code, reason = "read by the balloon through its seam")]
    pub(crate) after_edit_pools: Vec<(i32, Vec<i32>)>,
}

impl TweetTables {
    /// The tweet row of `id` (the master lookup by id).
    pub(crate) fn tweet(&self, id: i32) -> Option<&TweetRow> {
        self.tweets
            .binary_search_by_key(&id, |row| row.id)
            .ok()
            .map(|index| &self.tweets[index])
    }

    /// Every row (read by the balloon through its seam).
    #[allow(dead_code, reason = "read by the balloon through its seam")]
    pub(crate) fn tweets(&self) -> &[TweetRow] {
        &self.tweets
    }
}

fn tweet_tables_path() -> AssetPath<'static> {
    AssetPath::from("moly://tweet-tables.json".to_owned())
}

/// Startup: request both files.
pub(crate) fn load(mut commands: Commands, server: Res<AssetServer>) {
    commands.insert_resource(TweetTableHandles {
        tweets: server.load::<JsonAsset>(moly_assets::tweet_master()),
        tables: server.load::<JsonAsset>(tweet_tables_path()),
    });
}

/// Update: parse once both files are in.
pub(crate) fn parse(
    mut commands: Commands,
    server: Res<AssetServer>,
    jsons: Res<Assets<JsonAsset>>,
    handles: Option<Res<TweetTableHandles>>,
) {
    let Some(handles) = handles else {
        return;
    };
    for (name, handle) in [("tweet master", &handles.tweets), ("tweet tables", &handles.tables)] {
        if let LoadState::Failed(err) = server.load_state(handle) {
            panic!("{name} failed to load: {err:?}");
        }
    }
    let (Some(tweets_json), Some(tables_json)) =
        (jsons.get(&handles.tweets), jsons.get(&handles.tables))
    else {
        return;
    };
    let (tweets, after_edit_pools) = parse_tweets(&tweets_json.0);
    let (wrt, greetings, conditions) = parse_greeting_tables(&tables_json.0);
    info!(
        "[npc-tweet] tables ready: tweets {} / owner rows {} / greetings {} / conditions {} / after-edit pools {}",
        tweets.len(),
        wrt.len(),
        greetings.len(),
        conditions.len(),
        after_edit_pools.len()
    );
    commands.insert_resource(TweetTables {
        tweets,
        wrt,
        greetings,
        conditions,
        after_edit_pools,
    });
    commands.remove_resource::<TweetTableHandles>();
}

fn optional_string(row: &serde_json::Value, key: &str, id: i32) -> Option<String> {
    match row.get(key) {
        None | Some(serde_json::Value::Null) => None,
        Some(value) => Some(
            value
                .as_str()
                .unwrap_or_else(|| panic!("tweet {id}: {key} is not a string"))
                .to_owned(),
        ),
    }
}

fn required_string(row: &serde_json::Value, key: &str, id: i32) -> String {
    row.get(key)
        .and_then(|value| value.as_str())
        .unwrap_or_else(|| panic!("tweet {id} has no {key}"))
        .to_owned()
}

/// `tweets.json`: the tweet rows (sorted by id) and the after-edit pools.
pub(crate) fn parse_tweets(text: &str) -> (Vec<TweetRow>, Vec<(i32, Vec<i32>)>) {
    let value: serde_json::Value =
        serde_json::from_str(text).unwrap_or_else(|err| panic!("tweet master is not JSON: {err}"));
    let rows = value
        .get("tweets")
        .and_then(|value| value.as_object())
        .unwrap_or_else(|| panic!("tweet master has no tweets object"));
    let mut tweets: Vec<TweetRow> = rows
        .iter()
        .map(|(key, row)| {
            let id: i32 = key
                .parse()
                .unwrap_or_else(|_| panic!("tweet key is not a numeric id: {key}"));
            TweetRow {
                id,
                motion_name: optional_string(row, "motion", id),
                emoticon_name: optional_string(row, "emoticon", id),
                eye_name: required_string(row, "eye", id),
                mouth_name: required_string(row, "mouth", id),
                text: required_string(row, "text", id),
            }
        })
        .collect();
    tweets.sort_by_key(|row| row.id);
    let pools = value
        .get("afterEditPools")
        .and_then(|value| value.as_object())
        .unwrap_or_else(|| panic!("tweet master has no afterEditPools object"))
        .iter()
        .map(|(key, ids)| {
            let unit: i32 = key
                .parse()
                .unwrap_or_else(|_| panic!("after-edit pool key is not a unit id: {key}"));
            let ids = ids
                .as_array()
                .unwrap_or_else(|| panic!("after-edit pool {unit} is not an id array"))
                .iter()
                .map(|id| {
                    id.as_i64()
                        .map(|id| id as i32)
                        .unwrap_or_else(|| panic!("after-edit pool {unit} holds a non-integer id"))
                })
                .collect();
            (unit, ids)
        })
        .collect();
    (tweets, pools)
}

/// `tweet-tables.json`: the owner, greeting and condition tables. A null
/// value2 reads as 0 (no upper bound for the visit-count type; the phenomena
/// type does not read it).
pub(crate) fn parse_greeting_tables(
    text: &str,
) -> (
    Vec<WithoutRelatedTalkRow>,
    Vec<GreetingRow>,
    Vec<GreetingConditionRow>,
) {
    let value: serde_json::Value =
        serde_json::from_str(text).unwrap_or_else(|err| panic!("tweet tables are not JSON: {err}"));
    let int_of = |row: &serde_json::Value, key: &str| -> i32 {
        row.get(key)
            .and_then(|value| value.as_i64())
            .unwrap_or_else(|| panic!("tweet table row has no {key}")) as i32
    };
    let array = |key: &str| -> &Vec<serde_json::Value> {
        value
            .get(key)
            .and_then(|value| value.as_array())
            .unwrap_or_else(|| panic!("tweet tables have no {key} array"))
    };
    let wrt = array("withoutRelatedTalks")
        .iter()
        .map(|row| WithoutRelatedTalkRow {
            id: int_of(row, "id"),
            game_character_unit_id: int_of(row, "gameCharacterUnitId"),
            tweet_id: int_of(row, "tweetId"),
        })
        .collect();
    let greetings = array("greetings")
        .iter()
        .map(|row| GreetingRow {
            id: int_of(row, "id"),
            without_related_talk_id: int_of(row, "withoutRelatedTalkId"),
            greeting_condition_id: int_of(row, "greetingConditionId"),
        })
        .collect();
    let conditions = array("greetingConditions")
        .iter()
        .map(|row| {
            let condition_type = row
                .get("conditionType")
                .and_then(|value| value.as_str())
                .unwrap_or_else(|| panic!("condition row has no conditionType"))
                .to_owned();
            let value2 = row
                .get("value2")
                .and_then(|value| match value {
                    serde_json::Value::Null => Some(0),
                    value => value.as_i64(),
                })
                .unwrap_or_else(|| panic!("condition row has no value2")) as i32;
            GreetingConditionRow {
                id: int_of(row, "id"),
                condition_type,
                value1: int_of(row, "value1"),
                value2,
            }
        })
        .collect();
    (wrt, greetings, conditions)
}
