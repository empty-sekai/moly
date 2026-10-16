//! `moly.state`: the facts a client most often needs in one read. Entities
//! are given as the protocol's entity numbers, so `world.get_components`
//! and the other `world.*` methods take them as they are.

use bevy::diagnostic::FrameCount;
use bevy::prelude::*;
use serde_json::{json, Value};

use crate::camera::{FieldCameraModel, FieldCameraState};
use crate::npc::{CharacterUnitId, NpcActions};
use crate::npc_objective::ObjectiveMind;
use crate::player::PlayerControlled;

pub(super) fn snapshot(world: &mut World) -> Value {
    json!({
        "frame": world.get_resource::<FrameCount>().map(|frame| frame.0),
        "time": time(world),
        "server_clock_ms": crate::server::server_now_ms(),
        "entry_done": crate::entry::entry_settled(world.get_resource::<crate::entry::EntrySequence>()),
        "site_moving": world.contains_resource::<crate::site_move::SiteMoveActive>(),
        "talk_active": world.contains_resource::<crate::talk::ActiveTalk>()
            || world.contains_resource::<crate::player_talk::PlayerTalkSession>(),
        "site": site(world),
        "weather": weather(world),
        "player": player(world),
        "characters": characters(world),
        "screens": world.get_resource::<crate::ui_layers::ScreenManager>().map(crate::ui_layers::ScreenManager::summary),
        "camera": camera(world),
    })
}

fn vec3(value: Vec3) -> Value {
    json!([value.x, value.y, value.z])
}

pub(super) fn time(world: &World) -> Value {
    let virtual_time = world.get_resource::<Time<Virtual>>();
    json!({
        "elapsed": virtual_time.map(Time::elapsed_secs_f64),
        "scale": virtual_time.map(Time::relative_speed),
        "paused": virtual_time.map(Time::is_paused),
        "real_elapsed": world.get_resource::<Time<Real>>().map(Time::elapsed_secs_f64),
    })
}

pub(super) fn site(world: &World) -> Value {
    let Some(site) = world.get_resource::<crate::site::SiteActive>() else {
        return Value::Null;
    };
    json!({
        "site_id": site.site_id,
        "level": site.level,
        "site_type": site.site_type,
        "category": site.category,
        "scene": site.scene,
        "indoor": site.is_indoor(),
    })
}

pub(super) fn weather(world: &World) -> Value {
    let Some(current) = world.get_resource::<crate::weather::CurrentPhenomenonId>() else {
        return Value::Null;
    };
    let name = world
        .get_resource::<crate::weather::PhenomenonCatalogue>()
        .and_then(|catalogue| catalogue.0.iter().find(|option| option.id == current.0))
        .map(|option| option.name.clone());
    json!({ "phenomenon": current.0, "name": name })
}

fn player(world: &mut World) -> Value {
    let state = world
        .get_resource::<crate::player_state::PlayerAvatarStates>()
        .map(|states| format!("{:?}", states.current));
    let mut players = world.query_filtered::<(Entity, &GlobalTransform), With<PlayerControlled>>();
    match players.single(world) {
        Ok((entity, transform)) => json!({
            "entity": entity.to_bits(),
            "position": vec3(transform.translation()),
            "state": state,
        }),
        Err(_) => json!({ "entity": null, "state": state }),
    }
}

pub(super) fn characters(world: &mut World) -> Value {
    let mut roster = world.query_filtered::<(
        Entity,
        &CharacterUnitId,
        &GlobalTransform,
        Option<&NpcActions>,
        &ObjectiveMind,
        Has<crate::npc::residency::Away>,
    ), (Without<PlayerControlled>, Without<crate::cutscene::CutSceneAvatar>)>();
    let mut characters: Vec<(u32, Value)> = roster
        .iter(world)
        .map(|(entity, unit, transform, actions, mind, away)| {
            (
                unit.0,
                json!({
                    "unit": unit.0,
                    "entity": entity.to_bits(),
                    "position": vec3(transform.translation()),
                    "away": away,
                    "action": actions.map(|actions| format!("{:?}", actions.current)),
                    "tweeting": actions.map(|actions| actions.is_tweeting),
                    "talking": actions.map(|actions| actions.talk_owner.is_some()),
                    "objective": mind.current.map(|objective| format!("{objective:?}")),
                    "executing": mind.executing,
                }),
            )
        })
        .collect();
    characters.sort_by_key(|(unit, _)| *unit);
    Value::Array(characters.into_iter().map(|(_, character)| character).collect())
}

fn camera(world: &World) -> Value {
    let state = world.get_resource::<FieldCameraState>().map(|state| format!("{:?}", state.0));
    let Some(model) = world.get_resource::<FieldCameraModel>() else {
        return json!({ "state": state });
    };
    json!({
        "state": state,
        "look_at": vec3(model.look_at),
        "distance": model.distance,
        "yaw": model.yaw,
        "pitch": model.pitch,
        "fov": model.fov,
    })
}
