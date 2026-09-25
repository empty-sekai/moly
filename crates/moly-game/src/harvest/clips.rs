//! The source player's harvest clips as data: length, loop flag and
//! AnimationEvents (`MysekaiAnimationCommand`). The clips themselves are
//! never played on the SD body; their events and lengths drive the hit clock
//! and the waits.
//!
//! The client builds lower-case names (`..._l`, `..._o`); the export keeps
//! the asset's file name, which is the lower-case name, in `container`. The
//! join is by that file stem. How `PlayerAvatarView.PlayAnimation` treats
//! case was not read.

use std::collections::HashMap;

use bevy::asset::LoadState;
use bevy::prelude::*;
use moly_assets::json::JsonAsset;

const MANIFEST: &str = "moly://avatar/motion/mysekai__player_avatar.motion-manifest.json";

/// The four publishers of `MysekaiAnimationCommand` (all publish event 33).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ClipEvent {
    /// `PublishHarvestAction`: Hit, no state check.
    Hit,
    /// `PublishHarvestActionHit`: Hit only while the player's state is 7.
    HitInHarvestState,
    /// `PublishHarvestActionObject`: PostStartAction.
    PostStartAction,
    /// `PublishHarvestActionEffectOnly`: EffectOnly while the state is 7.
    EffectOnly,
}

#[derive(Clone, Debug)]
pub(crate) struct SourceClip {
    pub(crate) name: String,
    pub(crate) length: f32,
    pub(crate) looping: bool,
    pub(crate) events: Vec<(f32, ClipEvent)>,
}

#[derive(Resource)]
pub(crate) struct HarvestClips(pub(crate) HashMap<String, SourceClip>);

impl HarvestClips {
    pub(crate) fn get(&self, lower_name: &str) -> Option<&SourceClip> {
        self.0.get(lower_name)
    }
}

#[derive(Resource)]
pub(crate) struct HarvestClipsRequest(Handle<JsonAsset>);

pub(crate) fn load(mut commands: Commands, server: Res<AssetServer>) {
    commands.insert_resource(HarvestClipsRequest(
        server.load(bevy::asset::AssetPath::from(MANIFEST.to_owned())),
    ));
}

pub(crate) fn parse(
    mut commands: Commands,
    server: Res<AssetServer>,
    json: Res<Assets<JsonAsset>>,
    request: Option<Res<HarvestClipsRequest>>,
) {
    let Some(request) = request else {
        return;
    };
    match server.load_state(&request.0) {
        LoadState::Failed(error) => {
            warn!(
                "[harvest] input absent, the harvest action stays off: {MANIFEST} (the source swing clips' lengths and AnimationEvents): {error}"
            );
            commands.remove_resource::<HarvestClipsRequest>();
            return;
        }
        LoadState::Loaded => {}
        _ => return,
    }
    let Some(asset) = json.get(&request.0) else {
        return;
    };
    let value: serde_json::Value = serde_json::from_str(&asset.0)
        .unwrap_or_else(|error| panic!("{MANIFEST}: not JSON: {error}"));
    let mut clips = HashMap::new();
    let mut harvest = 0usize;
    for row in value["clips"]
        .as_array()
        .expect("motion manifest without clips")
    {
        let name = row["name"].as_str().expect("clip name").to_owned();
        let container = row["container"].as_str().expect("clip container");
        let stem = container
            .rsplit('/')
            .next()
            .and_then(|file| file.strip_suffix(".anim"))
            .unwrap_or_else(|| panic!("clip {name}: container is not an .anim path"))
            .to_owned();
        let mut events = Vec::new();
        for event in row["events"].as_array().into_iter().flatten() {
            let time = event["time"].as_f64().expect("event time") as f32;
            let kind = match event["functionName"].as_str().expect("event function") {
                "PublishHarvestAction" => ClipEvent::Hit,
                "PublishHarvestActionHit" => ClipEvent::HitInHarvestState,
                "PublishHarvestActionObject" => ClipEvent::PostStartAction,
                "PublishHarvestActionEffectOnly" => ClipEvent::EffectOnly,
                _ => continue,
            };
            events.push((time, kind));
        }
        if !events.is_empty() || row["harvest"].as_bool() == Some(true) {
            harvest += 1;
        }
        clips.insert(
            stem,
            SourceClip {
                name,
                length: row["durationSeconds"].as_f64().expect("clip duration") as f32,
                looping: row["loopTime"].as_bool().unwrap_or(false),
                events,
            },
        );
    }
    info!(
        "[harvest] source clip records: {} clips ({harvest} harvest family) from the motion manifest",
        clips.len()
    );
    commands.insert_resource(HarvestClips(clips));
    commands.remove_resource::<HarvestClipsRequest>();
}
