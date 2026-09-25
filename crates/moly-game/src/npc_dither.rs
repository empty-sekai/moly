//! Presenter call 5: `UpdateNpcDither`, the NPC's dither alpha.
//!
//! Source shape of the call: without a field camera, a graphics config or a
//! camera object nothing is written; in the cut-scene game state the alpha is
//! 1.0; outside the Mysekai scene nothing is written; otherwise the view's
//! `UpdateDitherAlpha(camera, config)`. This host runs the NPC-distance branch
//! of that update (the first NPC in the avatar list whose hips are within the
//! transparent distance, measured to its root); the camera and player
//! branches are named gaps of this file. Transforms are read as the frame's
//! update sees them: the last propagated pose.
//!
//! The NPC branch is off (alpha 1.0) while this NPC is talking or plays a
//! fixture action (states FixtureAction, FixtureActionIdle and the two
//! multi-character fixture states with their idles; the idles of the latter
//! do not exist in this host). Photo mode does not exist in this host.
//! Also not in this host: the shadow-casting switch at alpha 0, the
//! obstacle-avoidance write of a match, the dither keyword and its
//! disable property, and the skip of an approximately equal alpha.

use bevy::prelude::*;

use crate::character_material::{CharacterMaterial, ToonMaterials};
use crate::npc::CharacterUnitId;

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
) {
    let Some(registry) = registry.as_deref() else {
        return;
    };
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
        let alpha = moly_law::objective::overlap::npc_dither_alpha(
            talking.is_some(),
            false,
            fixture_action,
            &others,
        );
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
