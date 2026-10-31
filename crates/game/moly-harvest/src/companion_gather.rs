//! A companion's gather: the companion swings at a wood or mineral harvest
//! object, and each of its hits is a harvest hit of the player's own.
//!
//! The companion's side ([`moly_features::companion::CompanionCommand::Gather`])
//! walks beside the object and swings; this side chooses what it swings and
//! what each hit does:
//!
//! - the object, by UID or the nearest one left of a kind that no other
//!   companion is gathering ([`prepare`]);
//! - the tool, the one the player's harvest would take up for that kind
//!   (`SetAppropriateToolData`, read without storing the choice), and its
//!   Loop clip (`mov_u000_site_ax|pickax<level>_l`);
//! - the swings, as many as the object's hit points take at the tool's
//!   attack power, and the last attack after them;
//! - each hit, a [`HarvestHit`] with the tool, so the damage, its effects and
//!   sound, the break, the drops, the stamina, the durability and the server
//!   log are the player's harvest's own;
//! - the tool's model in the companion's right hand while it swings.

use std::collections::HashMap;

use bevy::gltf::Gltf;
use bevy::prelude::*;

use super::action::HarvestPlayerModel;
use super::catalog::HarvestCatalog;
use super::law::{harvest_clip_name, ToolClock, ToolState};
use super::tool_model::{spawn_in_hand, HarvestToolModels};
use super::{kind_word, HarvestHit, HarvestHits, HarvestObject, STATUS_HARVESTED};
use moly_features::companion::{hold_tool, Companion, CompanionCommand, GatherCue, HeldTool};
use moly_features::player_avatar::AvatarDriver;

/// The harvest object a gather names.
#[derive(Clone, Debug, PartialEq)]
pub enum GatherTarget {
    /// The object of this UID.
    Uid(u64),
    /// The nearest object left to the companion, of the kind word (`wood`
    /// or `mineral`) when one is given.
    Nearest { kind: Option<String> },
}

/// The gathers in flight, by companion unit.
#[derive(Resource, Default)]
pub struct CompanionGathers(HashMap<u32, Claim>);

struct Claim {
    target: Entity,
    uid: u64,
    tool: i64,
    assetbundle: String,
    damage: i32,
    level: i32,
    /// The swing has started and wants the tool in the hand.
    swinging: bool,
    /// The tool model is in the hand.
    holding: bool,
}

/// The gather the companion of `unit`, standing at `from`, makes of
/// `target`: the command to write, and a line naming the object, the tool
/// and the swings. Refused with the reason when the object is gone or
/// harvested, is not wood or mineral, or the player has no tool for it.
pub fn prepare(world: &mut World, unit: u32, from: Vec3, target: &GatherTarget) -> Result<(CompanionCommand, String), String> {
    let claimed: Vec<Entity> = world
        .get_resource::<CompanionGathers>()
        .map(|gathers| {
            gathers
                .0
                .iter()
                .filter(|(other, _)| **other != unit)
                .map(|(_, claim)| claim.target)
                .collect()
        })
        .unwrap_or_default();
    let mut objects = world.query::<(Entity, &HarvestObject, &GlobalTransform)>();
    let rows: Vec<(Entity, u64, Vec3, f32, bool, i32, i32)> = objects
        .iter(world)
        .map(|(entity, object, at)| {
            (
                entity,
                object.uid,
                at.translation(),
                object.radius,
                object.status == STATUS_HARVESTED,
                object.fixture_type,
                object.hp,
            )
        })
        .collect();
    if rows.is_empty() {
        return Err("this site has no gatherables".into());
    }
    let row = match target {
        GatherTarget::Uid(uid) => {
            let row = rows
                .into_iter()
                .find(|row| row.1 == *uid)
                .ok_or_else(|| format!("no gatherable with UID {uid} on this site"))?;
            if row.4 {
                return Err(format!("gatherable {uid} is already harvested"));
            }
            row
        }
        GatherTarget::Nearest { kind } => rows
            .into_iter()
            .filter(|row| !row.4 && matches!(row.5, 0 | 1) && !claimed.contains(&row.0))
            .filter(|row| kind.as_deref().is_none_or(|kind| kind_word(row.5) == kind))
            .min_by(|a, b| a.2.distance_squared(from).total_cmp(&b.2.distance_squared(from)))
            .ok_or_else(|| match kind {
                Some(kind) => format!("no {kind} gatherable left to gather on this site"),
                None => "no wood or mineral gatherable left to gather on this site".to_owned(),
            })?,
    };
    let (entity, uid, at, radius, _, fixture_type, hp) = row;
    if !matches!(fixture_type, 0 | 1) {
        return Err(format!(
            "gatherable {uid} is {}: a companion gathers wood and mineral only",
            kind_word(fixture_type)
        ));
    }
    let catalog = world
        .get_resource::<HarvestCatalog>()
        .ok_or("the harvest catalog is not loaded")?;
    let model = world
        .get_resource::<HarvestPlayerModel>()
        .ok_or("the player's harvest model is not loaded")?;
    let tool = model.appropriate_tool(catalog, fixture_type).ok_or_else(|| {
        format!("the player has no tool left for {}", kind_word(fixture_type))
    })?;
    let def = catalog
        .tool(tool)
        .ok_or_else(|| format!("tool {tool} is not in the harvest catalog"))?;
    let clock = ToolClock {
        tool_type: def.tool_type,
        level: def.level,
        cool_time: def.cool_time,
    };
    let clip = harvest_clip_name(fixture_type, Some(clock), ToolState::Loop)
        .ok_or_else(|| format!("no harvest clip for {}", kind_word(fixture_type)))?;
    let damage = def.attack_power.max(1);
    // The hit that takes the hit points to zero leaves the object standing;
    // the next one is its last attack (`UpdateHp`).
    let swings = (hp.max(0) as u32).div_ceil(damage as u32) + 1;
    let claim = Claim {
        target: entity,
        uid,
        tool,
        assetbundle: def.assetbundle.clone(),
        damage,
        level: def.level,
        swinging: false,
        holding: false,
    };
    let note = format!(
        "{} {uid} at ({:.2}, {:.2}, {:.2}), hp {hp}: {swings} swings of {clip} with tool {tool}",
        kind_word(fixture_type),
        at.x,
        at.y,
        at.z
    );
    world.get_resource_or_init::<CompanionGathers>().0.insert(unit, claim);
    Ok((
        CompanionCommand::Gather {
            unit,
            at,
            reach: radius,
            clip,
            swings,
            tag: uid,
        },
        note,
    ))
}

/// Update, before the harvest action loop: each companion hit becomes a hit
/// of the player's harvest on the object, while it has hit points left.
pub fn hits(
    mut cues: MessageReader<GatherCue>,
    mut gathers: ResMut<CompanionGathers>,
    mut harvest: ResMut<HarvestHits>,
    model: Option<Res<HarvestPlayerModel>>,
    objects: Query<&HarvestObject>,
) {
    for cue in cues.read() {
        match cue {
            GatherCue::Swinging { unit, tag } => {
                if let Some(claim) = gathers.0.get_mut(unit).filter(|claim| claim.uid == *tag) {
                    claim.swinging = true;
                }
            }
            GatherCue::Hit { unit, tag, hit, of, .. } => {
                let Some(claim) = gathers.0.get(unit).filter(|claim| claim.uid == *tag) else {
                    continue;
                };
                let Ok(object) = objects.get(claim.target) else {
                    warn!("[harvest] companion {unit} hit {hit}/{of}: gatherable {tag} left the site");
                    continue;
                };
                if object.status == STATUS_HARVESTED {
                    continue;
                }
                harvest.0.push(HarvestHit {
                    target: claim.target,
                    damage: claim.damage,
                    tool_level: claim.level,
                    is_boost: model.as_deref().is_some_and(|model| model.stamina.has_boost_or_enhance()),
                    tool: Some(claim.tool),
                });
                info!(
                    "[harvest] companion {unit} hit {hit}/{of} on {}#{} (hp {}) with tool {}",
                    object.leaf, object.fixture_id, object.hp, claim.tool
                );
            }
            GatherCue::Ended { unit, tag } => {
                if gathers.0.get(unit).is_some_and(|claim| claim.uid == *tag) {
                    gathers.0.remove(unit);
                }
            }
        }
    }
}

/// Update: a swinging companion takes its gather's tool model in its right
/// hand; the companion's side puts it away with the gather.
pub fn tools(
    mut commands: Commands,
    server: Res<AssetServer>,
    gltfs: Res<Assets<Gltf>>,
    mut models: ResMut<HarvestToolModels>,
    mut gathers: ResMut<CompanionGathers>,
    companions: Query<(Entity, &Companion, &AvatarDriver, Has<HeldTool>)>,
    names: Query<(Entity, &Name)>,
    parents: Query<&ChildOf>,
) {
    for (entity, companion, driver, held) in &companions {
        let Some(claim) = gathers.0.get_mut(&companion.unit) else {
            continue;
        };
        if !claim.swinging || claim.holding || held {
            continue;
        }
        match spawn_in_hand(
            &mut commands,
            &server,
            &gltfs,
            &mut models,
            claim.tool,
            &claim.assetbundle,
            driver,
            &names,
            &parents,
        ) {
            Ok(Some(tool)) => {
                hold_tool(&mut commands, entity, tool);
                claim.holding = true;
                info!(
                    "[harvest] companion {}: tool model {} in the right hand",
                    companion.unit, claim.assetbundle
                );
            }
            Ok(None) => {}
            Err(reason) => {
                claim.holding = true;
                warn!("[harvest] companion {}: no tool in the hand: {reason}", companion.unit);
            }
        }
    }
}
