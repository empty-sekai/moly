//! Acceptance harness, talk tables: the product's talk-table loader and its
//! general-talk lottery on server talk lists, and the talk -> pre-action ->
//! tweet join the product resolves. See the parent harness module for the
//! protocol. The tables are read from `MOLY_ASSET_ROOT` through the same
//! document builder the product's loader uses.

use serde_json::{json, Value};

use super::FixtureActivityTables;
use crate::npc_harness::{case, emit};
use crate::npc_talk_lottery::{lottery_general_talk_id, Draws, LotteryScene, NpcView};
use crate::server_panel::TalkWithReadHistory;
use moly_law::talk::select::{GeneralPick, LotteryWeights};

fn tables_of(root: &str) -> FixtureActivityTables {
    let documents: Vec<Value> = super::PATHS
        .iter()
        .map(|path| {
            let file = path.trim_start_matches("moly://");
            let text = std::fs::read_to_string(format!("{root}/{file}"))
                .unwrap_or_else(|err| panic!("{root}/{file}: {err}"));
            serde_json::from_str(&text).unwrap_or_else(|err| panic!("{file}: {err}"))
        })
        .collect();
    FixtureActivityTables::from_documents(&documents).expect("activity tables")
}

/// One general lottery with the engine draw fixed to `index`.
fn general(scene: &LotteryScene<'_>, seeker: &NpcView, index: usize) -> Result<GeneralPick, String> {
    let mut int = |_: usize| index;
    let mut float = |total: f32| total;
    let mut pick = |_: usize| Some(0usize);
    let mut draws = Draws { engine_int: &mut int, engine_float: &mut float, sequence_pick: &mut pick, record: Vec::new() };
    lottery_general_talk_id(scene, seeker, &mut draws, "harness").map_err(|halt| format!("{halt:?}"))
}

/// The general pool the product draws from: pool[i] for every index.
fn pool_of(scene: &LotteryScene<'_>, seeker: &NpcView) -> Value {
    match general(scene, seeker, 0) {
        Ok(GeneralPick::Drawn { pool_len, .. }) => {
            let ids: Vec<i32> = (0..pool_len)
                .map(|i| match general(scene, seeker, i) {
                    Ok(GeneralPick::Drawn { talk_id, .. }) => talk_id,
                    other => panic!("pool changed between draws: {other:?}"),
                })
                .collect();
            json!({"pool": ids})
        }
        Ok(GeneralPick::Previous { talk_id }) => json!({"previous": talk_id}),
        Err(halt) => json!({"halt": halt}),
    }
}

#[test]
#[ignore = "requires MOLY_NPC_HARNESS_DIR and MOLY_ASSET_ROOT"]
fn a04_general_pool() {
    let input = case("a04");
    let root = std::env::var("MOLY_ASSET_ROOT").expect("MOLY_ASSET_ROOT");
    let tables = tables_of(&root);
    // Site id -> site type as the case states it (from the site master).
    let site_types: Vec<(i32, i32)> = input["site_types"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| (p[0].as_i64().unwrap() as i32, p[1].as_i64().unwrap() as i32))
        .collect();
    let site_type_of_site = move |site: i32| site_types.iter().find(|(id, _)| *id == site).map(|(_, t)| *t);
    let mut out = serde_json::Map::new();
    for (state, rows) in input["lists"].as_object().unwrap() {
        let list: Vec<TalkWithReadHistory> = rows
            .as_array()
            .unwrap()
            .iter()
            .map(|r| TalkWithReadHistory { talk_id: r[0].as_i64().unwrap() as i32, is_read: r[1].as_bool().unwrap() })
            .collect();
        let mut results = Vec::new();
        for query in input["queries"].as_array().unwrap() {
            let scene = LotteryScene {
                tables: &tables,
                talk_list: &list,
                phenomena_id: query["phenomena"].as_i64().unwrap() as i32,
                site_type_of_site: &site_type_of_site,
                npcs: &[],
                fixtures: &[],
                weights: LotteryWeights { talk1: 40.0, talk2: 20.0, talk3: 20.0, talk4: 20.0 },
                fixture_gates: None,
            };
            let seeker = NpcView {
                unit: query["unit"].as_u64().unwrap() as u32,
                site_type: query["site_type"].as_i64().map(|t| t as i32),
                previous_talk_id: query["previous"].as_i64().unwrap() as i32,
                talk_type: None,
                objective: None,
                state: 0,
                since_initialized: 0.0,
            };
            results.push(pool_of(&scene, &seeker));
        }
        out.insert(state.clone(), Value::Array(results));
    }
    emit("a04", &Value::Object(out));
}

#[test]
#[ignore = "requires MOLY_NPC_HARNESS_DIR and MOLY_ASSET_ROOT"]
fn a08_general_tweet_join() {
    let root = std::env::var("MOLY_ASSET_ROOT").expect("MOLY_ASSET_ROOT");
    let tables = tables_of(&root);
    let tweets_text = std::fs::read_to_string(format!("{root}/tweets.json")).expect("tweets.json");
    let (tweets, _) = crate::npc_tweet::parse_tweets(&tweets_text);
    let input = case("a08");
    let mut rows = Vec::new();
    for talk in input["talks"].as_array().unwrap() {
        let talk = talk.as_i64().unwrap() as i32;
        let pre = tables.pre_action_of(talk);
        let tweet_id = pre.map(|p| p.tweet_id);
        let row = tweet_id.and_then(|id| tweets.iter().find(|t| t.id == id));
        rows.push(json!({
            "talk": talk,
            "pre_action": pre.map(|p| p.id),
            "tweet_id": tweet_id,
            "timeline_group": pre.and_then(|p| p.timeline_group_id),
            "row": row.map(|t| json!({"text": t.text, "motion": t.motion_name, "emoticon": t.emoticon_name, "eye": t.eye_name, "mouth": t.mouth_name})),
        }));
    }
    emit("a08", &Value::Array(rows));
}
