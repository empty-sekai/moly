//! Presenter call 5: `UpdateNpcDither`, the NPC's dither alpha.
//!
//! Source shape of the call: without a field camera, a graphics config or a
//! camera object nothing is written; in the cut-scene game state the alpha is
//! 1.0; outside the Mysekai scene nothing is written; otherwise the view's
//! `UpdateDitherAlpha(camera, config)`. This host runs its NPC-distance
//! branch (the first NPC in the avatar list whose hips are within the
//! transparent distance, measured to its root) and its player branch (the
//! local player within 1.15 of the NPC's hips while the NPC is not talking:
//! with the player visible, the distance from the NPC's root to the player
//! remapped over 0.45), and takes the smaller of the two. Transforms are read
//! as the frame's update sees them: the last propagated pose.
//!
//! The camera branch (`GetDitherAlpha`) remaps the NPC root's screen depth
//! (the field camera's view-space depth of the root, what `WorldToScreenPoint`
//! gives as z) from the camera's near clip plus the graphics config's NPC
//! dither start offset over its fall-off (`graphics-config/npc-dither.json`;
//! the photo-mode pair is never taken: no photo mode here). It is the player
//! branch's value when the player is not visible, and it lowers the result
//! when the player is visible and the NPC is not talking. Without the
//! document or a field camera it is left out, and the first frame that needs
//! it says so once. Multiplayer's other players (the second near-player
//! kind) do not exist in this host.
//!
//! The NPC branch is off (alpha 1.0) while this NPC is talking or plays a
//! fixture action (states FixtureAction, FixtureActionIdle and the two
//! multi-character fixture states with their idles; the idles of the latter
//! do not exist in this host). Photo mode does not exist in this host.
//! Also not in this host: the shadow-casting switch at alpha 0, the
//! obstacle-avoidance write of a match, the dither keyword and its
//! disable property, and the skip of an approximately equal alpha.

use bevy::prelude::*;

use moly_assets::json::JsonAsset;

use crate::character_material::{CharacterMaterial, ToonMaterials};
use crate::npc::CharacterUnitId;

/// The graphics configuration's NPC dither document.
const DITHER_CONFIG_PATH: &str = "moly://graphics-config/npc-dither.json";

/// The camera branch's start offset and fall-off (the non-photo pair).
#[derive(Debug, Clone, Copy)]
pub(crate) struct DitherDistances {
    start_offset: f32,
    fall_off: f32,
}

/// The document's two non-photo floats, from their serialized bit words.
fn parse_distances(text: &str) -> Result<DitherDistances, String> {
    let document: serde_json::Value =
        serde_json::from_str(text).map_err(|error| format!("does not parse: {error}"))?;
    let word = |field: &str| -> Result<f32, String> {
        let hex = document["bits"][field]
            .as_str()
            .ok_or_else(|| format!("no bits for {field}"))?;
        u32::from_str_radix(hex, 16)
            .map(f32::from_bits)
            .map_err(|error| format!("{field}: {error}"))
    };
    Ok(DitherDistances {
        start_offset: word("_npcDitherStartDistanceOffset")?,
        fall_off: word("_npcDitherDistanceFallOff")?,
    })
}

/// The document's load, kept by the dither system.
#[derive(Default)]
pub(crate) struct DitherConfigLoad {
    handle: Option<Handle<JsonAsset>>,
    settled: Option<Result<DitherDistances, String>>,
}

impl DitherConfigLoad {
    fn poll(
        &mut self,
        server: &AssetServer,
        documents: &Assets<JsonAsset>,
    ) -> Option<DitherDistances> {
        if let Some(settled) = &self.settled {
            return settled.as_ref().ok().copied();
        }
        let handle = self
            .handle
            .get_or_insert_with(|| server.load(DITHER_CONFIG_PATH))
            .clone();
        let settled = if server.load_state(&handle).is_failed() {
            Err(format!("{DITHER_CONFIG_PATH} did not load"))
        } else if let Some(document) = documents.get(&handle) {
            parse_distances(&document.0)
        } else {
            return None;
        };
        match &settled {
            Ok(distances) => info!(
                "[npc-dither] camera branch: start offset {} (bits {:08x}), fall-off {} (bits {:08x}) from the graphics configuration",
                distances.start_offset,
                distances.start_offset.to_bits(),
                distances.fall_off,
                distances.fall_off.to_bits()
            ),
            Err(reason) => warn!("[npc-dither] camera branch refused: {reason}"),
        }
        let distances = settled.as_ref().ok().copied();
        self.settled = Some(settled);
        distances
    }
}

/// Presenter call 5.
#[allow(clippy::type_complexity)]
pub(crate) fn update_dither(
    mut materials: ResMut<Assets<CharacterMaterial>>,
    registry: Option<Res<crate::npc::Registry>>,
    npcs: Query<
        (
            Entity,
            &CharacterUnitId,
            &ToonMaterials,
            Option<&crate::talk::TalkHold>,
            Option<&crate::npc::NpcActions>,
        ),
        Without<crate::player::PlayerControlled>,
    >,
    globals: Query<&GlobalTransform>,
    gate: crate::npc_presenter::PresenterGate,
    players: Query<(Entity, &InheritedVisibility), With<crate::player::PlayerControlled>>,
    cameras: Query<(&Camera, &GlobalTransform, &Projection), With<Camera3d>>,
    server: Res<AssetServer>,
    documents: Res<Assets<JsonAsset>>,
    mut config: Local<DitherConfigLoad>,
    mut camera_gap_reported: Local<bool>,
    mut camera_lowered: Local<std::collections::HashSet<u32>>,
) {
    let Some(registry) = registry.as_deref() else {
        return;
    };
    let distances = config.poll(&server, &documents);
    // The field camera: its world-to-view transform and near clip plane.
    let field_camera = cameras
        .iter()
        .find(|(camera, _, _)| camera.is_active)
        .and_then(|(_, transform, projection)| match projection {
            Projection::Perspective(p) => Some((transform.affine().inverse(), p.near)),
            _ => None,
        });
    // The avatar list's first match per unit.
    let mut positions: std::collections::HashMap<u32, Option<(Vec3, Vec3)>> =
        std::collections::HashMap::new();
    for (entity, unit, toon, _, _) in &npcs {
        positions.entry(unit.0).or_insert_with(|| {
            Some((
                globals.get(toon.hips_entity()).ok()?.translation(),
                globals.get(entity).ok()?.translation(),
            ))
        });
    }
    // The local player: its position and whether it is shown (TPS mode).
    let player = players.iter().next().and_then(|(entity, visible)| {
        Some((globals.get(entity).ok()?.translation(), visible.get()))
    });
    for (entity, unit, toon, talking, actions) in &npcs {
        if !gate.runs(entity) {
            continue;
        }
        let (Ok(hips), Ok(root)) = (globals.get(toon.hips_entity()), globals.get(entity)) else {
            continue;
        };
        let others: Vec<_> = registry
            .character_unit_ids
            .iter()
            .filter(|id| **id != unit.0)
            .filter_map(|id| positions.get(id).copied().flatten())
            .map(
                |(other_hips, other_root)| moly_law::objective::overlap::DitherNeighbor {
                    hips_distance: other_hips.distance(hips.translation()),
                    root_distance: other_root.distance(root.translation()),
                },
            )
            .collect();
        use crate::npc::NpcAction;
        let fixture_action = actions.is_some_and(|actions| {
            matches!(
                actions.current,
                NpcAction::FixtureAction
                    | NpcAction::FixtureActionIdle
                    | NpcAction::MainSomeFixtureAction
                    | NpcAction::SomeFixtureAction
            )
        });
        use moly_law::objective::overlap as law;
        let npc_alpha = law::npc_dither_alpha(talking.is_some(), false, fixture_action, &others);
        // IsValidPlayerDistanceTransparent: not talking (no photo mode here),
        // the player within the near distance of the hips.
        let near_player = player.filter(|(position, _)| {
            talking.is_none() && law::player_near((*position - hips.translation()).to_array())
        });
        let tps = player.is_some_and(|(_, visible)| visible);
        let camera_needed =
            near_player.is_some_and(|(_, shown)| !shown) || (tps && talking.is_none());
        // GetDitherAlpha: the root's screen depth (view-space depth in front
        // of the camera) against near + start offset over the fall-off.
        let camera_alpha = match (distances, field_camera) {
            (Some(distances), Some((to_view, near))) => {
                let depth = -to_view.transform_point3(root.translation()).z;
                Some(law::camera_dither_alpha(
                    depth,
                    near,
                    distances.start_offset,
                    distances.fall_off,
                ))
            }
            _ => None,
        };
        if camera_needed && camera_alpha.is_none() && !*camera_gap_reported {
            *camera_gap_reported = true;
            warn!(
                "[npc-dither] unit={}: the camera branch is not applied ({})",
                unit.0,
                if distances.is_none() {
                    "no NPC dither distances"
                } else {
                    "no field camera"
                }
            );
        }
        let alpha = law::update_dither_alpha(
            npc_alpha,
            near_player.map(|(position, _)| (position - root.translation()).to_array()),
            tps,
            talking.is_none(),
            camera_alpha,
        );
        if camera_needed {
            if let Some(camera_alpha) = camera_alpha.filter(|a| *a < 1.0) {
                if camera_lowered.insert(unit.0) {
                    info!(
                        "[npc-dither] unit={}: camera branch alpha {camera_alpha:.3} (result {alpha:.3}), first time below 1",
                        unit.0
                    );
                }
            }
        }
        let use_dither = if moly_law::objective::overlap::use_dither(alpha) {
            1.0_f32
        } else {
            0.0
        };
        for handle in toon.slot_handles() {
            let changed = materials.get(handle).is_some_and(|material| {
                material.params.dither_alpha.to_bits() != alpha.to_bits()
                    || material.params.use_dither.to_bits() != use_dither.to_bits()
            });
            if !changed {
                continue;
            }
            if let Some(material) = materials.get_mut_untracked(handle) {
                material.params.dither_alpha = alpha;
                material.params.use_dither = use_dither;
            }
        }
    }
}
