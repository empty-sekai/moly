//! Player interaction qualification shared by buttons, world picks and request consumers.

use bevy::{ecs::system::SystemParam, prelude::*};
use moly_law::action_button::{
    character_box, fixture_box, house_box, update_player_box, CollisionBox2D,
    HOUSE_BOX_DOOR_OFFSET, PLAYER_ADDITIONAL_HALF_EXTEND,
};

use crate::{
    menu_shell::ShellDialogState,
    npc::{CharacterUnitId, NpcAction, NpcActions},
    player::PlayerControlled,
    player_state::{PlayerActionState, PlayerAvatarStates},
    player_talk::PlayerTalkSession,
    talk::{ActiveTalk, TalkHold},
    ui_layers::UiLayerStack,
};

// The collision boxes are built and tested in the source frame. The runtime
// frame reflects the source's x axis, and the box test is not
// mirror-symmetric (each box projects three of its four corners), so testing
// reflected boxes answers differently near the edges, on the opposite side.

/// A runtime position in the source frame: x reflected back.
pub(crate) fn source_point(runtime: Vec3) -> [f32; 3] {
    [-runtime.x, runtime.y, runtime.z]
}

/// A placed fixture's box rotation, from its runtime rotation: the source
/// direction's quarter turns times 90 degrees, the angle
/// `FixtureView.ForceSetRotation` writes into the box. The runtime yaw is the
/// reflected source yaw, so the turns run the other way (Left and Right
/// swap).
pub(crate) fn source_box_rotation(rotation: Quat) -> f32 {
    let forward = rotation * Vec3::Z;
    let quarters = (forward.x.atan2(forward.z) / std::f32::consts::FRAC_PI_2).round() as i32;
    quarters.wrapping_neg().rem_euclid(4) as f32 * 90.0
}

/// The player's additional box (`ObjectCollisionManager
/// .UpdateAdditionalCollisionBox`), in the source frame: centre half a metre
/// ahead along the source forward, rotation minus the source yaw.
pub(crate) fn player_box(player: &Transform) -> CollisionBox2D {
    let mut bounds = CollisionBox2D::new([0.0, 0.0], PLAYER_ADDITIONAL_HALF_EXTEND, 0.0);
    let forward = player.rotation * Vec3::Z;
    update_player_box(
        &mut bounds,
        source_point(player.translation),
        [-forward.x, forward.y, forward.z],
        crate::site_move::camera::source_yaw(player.rotation),
    );
    bounds
}

/// An NPC's collision box, from its pose, in the source frame.
pub(crate) fn character_target_box(target: &Transform) -> CollisionBox2D {
    character_box(source_point(target.translation))
}

/// A placed fixture's collision box, from its root pose and its master grid,
/// in the source frame.
pub(crate) fn fixture_target_box(
    root: &Transform,
    grid_width: i32,
    grid_depth: i32,
    has_collider: bool,
) -> CollisionBox2D {
    fixture_box(
        source_point(root.translation),
        grid_width,
        grid_depth,
        source_box_rotation(root.rotation),
        has_collider,
    )
}

/// The house's runtime box centre (`HouseView.GetBoxCollider2DCenter`): the
/// outside door point plus the house's rotation times (0, 0, 1.2); the origin
/// when the house has no outside door point.
pub(crate) fn house_box_centre(root: &Transform, outside_door: Option<Vec3>) -> Vec3 {
    outside_door
        .map(|door| door + root.rotation * Vec3::new(0.0, 0.0, HOUSE_BOX_DOOR_OFFSET))
        .unwrap_or(Vec3::ZERO)
}

/// The house's collision box in the source frame: `HouseView`'s fixed box at
/// [`house_box_centre`], turned by the house's direction.
pub(crate) fn house_target_box(root: &Transform, outside_door: Option<Vec3>, has_collider: bool) -> CollisionBox2D {
    house_box(
        source_point(house_box_centre(root, outside_door)),
        source_box_rotation(root.rotation),
        has_collider,
    )
}

#[derive(SystemParam)]
pub(crate) struct InteractionEligibility<'w, 's> {
    players: Query<
        'w,
        's,
        (Entity, &'static Transform, Option<&'static TalkHold>),
        With<PlayerControlled>,
    >,
    targets: Query<
        'w,
        's,
        (
            Entity,
            &'static Transform,
            &'static CharacterUnitId,
            &'static NpcActions,
            Option<&'static Visibility>,
            Option<&'static InheritedVisibility>,
            Option<&'static crate::character_material::ToonMaterials>,
        ),
        Without<PlayerControlled>,
    >,
    bones: Query<'w, 's, &'static GlobalTransform>,
    site_roots: Query<'w, 's, &'static GlobalTransform, With<crate::site::SiteRoot>>,
    face: Option<Res<'w, crate::npc_objective::ObjectiveFace>>,
    site: Option<Res<'w, crate::site::SiteActive>>,
    layers: Res<'w, UiLayerStack>,
    dialogs: Res<'w, ShellDialogState>,
    player_state: Res<'w, PlayerAvatarStates>,
    edits: Res<'w, crate::fixture_edit::EditSessionActive>,
    overlays: crate::game_settings::OverlayPanels<'w>,
    player_talk: Option<Res<'w, PlayerTalkSession>>,
    pair_talk: Option<Res<'w, ActiveTalk>>,
    entry: Option<Res<'w, crate::entry::EntrySequence>>,
    delivery: Option<Res<'w, crate::delivery::DeliveryGameState>>,
    holds: crate::game_state::PerformanceHolds<'w>,
    screens: Option<Res<'w, crate::ui_layers::ScreenManager>>,
}

impl InteractionEligibility<'_, '_> {
    /// `ExistsDialog(Layer_Dialog)`: a dialog is shown over the field, and
    /// the dialog layer takes the taps the field's buttons would get (the
    /// buttons stay drawn under it).
    pub(crate) fn dialog_shown(&self) -> bool {
        self.screens
            .as_deref()
            .is_some_and(crate::ui_layers::ScreenManager::exists_dialog)
    }

    pub(crate) fn available(&self) -> bool {
        crate::entry::control_open(self.entry.as_deref())
            && self.layers.on_field()
            && !self.overlays.block_world_input()
            && !self.edits.is_active()
            && !self.dialogs.blocks_field_input()
            && self.player_state.can_intercept
            && self.player_talk.is_none()
            && self.pair_talk.is_none()
            && !self.holds.ui_blocked()
    }

    /// The field accepts a button tap, without any reading of the player's
    /// own state. A conversation between NPCs does not close it; one with
    /// the player does.
    pub(crate) fn field_input_open(&self) -> bool {
        crate::entry::control_open(self.entry.as_deref())
            && self.layers.on_field()
            && !self.overlays.block_world_input()
            && !self.edits.is_active()
            && !self.dialogs.blocks_field_input()
            && !self.player_in_talk()
            && !self.holds.ui_blocked()
    }

    /// The GameState Talk equivalent: a conversation that includes the player.
    pub(crate) fn player_in_talk(&self) -> bool {
        self.player_talk.is_some()
            || self
                .pair_talk
                .as_ref()
                .is_some_and(|talk| talk.includes_player())
    }

    /// ObjectCollisionManager.IsCanUpdate holds in GameState Normal, Harvest
    /// and Sketch. Of the other states Moly has Edit (the layout editor),
    /// Talk (a conversation with the player) and Delivery (set by a delivery
    /// press, until the next site move); a harvest, a gimmick switch, a
    /// timeline, a shell dialog or a pushed layer stay in Normal or Harvest.
    pub(crate) fn collision_updates(&self) -> bool {
        !self.edits.is_active()
            && !self.player_in_talk()
            && self.delivery.is_none()
            && !self.holds.collision_stopped()
    }

    /// ScreenLayerMysekaiHome, which holds the action buttons, is the current
    /// screen. The layout editor (SiteEditMode), a conversation with the
    /// player (MysekaiTalk) and every layer of the layer stack are entered by
    /// PushUIScreen, whose PushUIScreenCore exits the current screen first.
    /// Dialogs are not screens and leave it current.
    pub(crate) fn home_screen_current(&self) -> bool {
        // The home screen becomes current in the entry's OnFinishEnterAsync.
        crate::entry::hud_open(self.entry.as_deref())
            && self.layers.on_field()
            && !self.edits.is_active()
            && !self.player_in_talk()
    }

    /// The NPC is a registered collision object: its avatar is set up.
    pub(crate) fn talk_registered(&self, entity: Entity) -> bool {
        self.targets
            .get(entity)
            .is_ok_and(|(_, _, _, actions, _, _, _)| actions.ready())
    }

    /// IsActionButtonTypeAvailable for a Talk entry: CheckTargetSite (the
    /// NPC's site is the current site), then CanShowTalkActionButton (the
    /// tutorial gate, open outside a tutorial and Moly has no tutorial, and
    /// the NPC is in the NPC list). `None` when no site is active to compare
    /// with.
    pub(crate) fn talk_available(&self, entity: Entity) -> Option<bool> {
        let site = self.site.as_ref()?;
        Some(
            self.targets
                .get(entity)
                .is_ok_and(|(_, _, _, actions, _, _, _)| {
                    actions.ready() && site.site_type == actions.site_type
                }),
        )
    }

    /// AddShowButtonStack for a Talk entry after its lock check:
    /// IsActionButtonTypeAvailable, then IsCanActionNPC (the NPC's view is
    /// active and visible). Talk state, EnableTalk and the player's own state
    /// are read when the button is tapped, not here.
    pub(crate) fn talk_admitted(&self, entity: Entity) -> Option<bool> {
        if !self.talk_available(entity)? {
            return Some(false);
        }
        Some(
            self.targets
                .get(entity)
                .is_ok_and(|(_, _, _, _, visibility, inherited, _)| {
                    !visibility.is_some_and(|v| *v == Visibility::Hidden)
                        && !inherited.is_some_and(|visible| !visible.get())
                }),
        )
    }

    pub(crate) fn player_action(&self) -> PlayerActionState {
        self.player_state.current
    }

    pub(crate) fn player_entity(&self) -> Option<Entity> {
        self.players.single().ok().map(|(entity, _, _)| entity)
    }

    pub(crate) fn for_unit(&self, unit: u32) -> bool {
        self.targets
            .iter()
            .find(|(_, _, id, _, _, _, _)| id.0 == unit)
            .is_some_and(|(entity, _, _, _, _, _, _)| self.allows(entity, unit))
    }

    pub(crate) fn allows(&self, entity: Entity, unit: u32) -> bool {
        if !self.available() {
            return false;
        }
        let Ok((_, player, player_hold)) = self.players.single() else {
            return false;
        };
        let Ok((_, target, id, actions, visibility, inherited, _)) = self.targets.get(entity)
        else {
            return false;
        };
        if id.0 != unit
            || player_hold.is_some()
            || !actions.ready()
            || !self
                .site
                .as_ref()
                .is_some_and(|site| site.site_type == actions.site_type)
            || visibility.is_some_and(|v| *v == Visibility::Hidden)
            || inherited.is_some_and(|visible| !visible.get())
            || !player_box(player).collides(&character_target_box(target))
        {
            return false;
        }
        true
    }

    /// Click qualification is intentionally later than button appearance. The
    /// source first finds a safe player position, then evaluates talk ownership.
    pub(crate) fn click_position(&self, entity: Entity, unit: u32) -> Result<Vec3, &'static str> {
        if !self.allows(entity, unit) {
            return Err("target is not in the active interaction stack");
        }
        self.talk_position(entity)
    }

    /// Library selection is not proximity selection. It keeps the same actor,
    /// navigation, occupancy and ownership validation while deliberately not
    /// requiring the target to be the current action-button candidate.
    pub(crate) fn library_position(&self, entity: Entity) -> Result<Vec3, &'static str> {
        self.talk_position(entity)
    }

    fn talk_position(&self, entity: Entity) -> Result<Vec3, &'static str> {
        let (player_entity, player, _) =
            self.players.single().map_err(|_| "player is not ready")?;
        let (_, _, _, actions, _, _, toon) =
            self.targets.get(entity).map_err(|_| "NPC left the scene")?;
        let hips = self
            .bones
            .get(toon.ok_or("NPC skeleton is not ready")?.hips_entity())
            .map_err(|_| "NPC hips transform is not ready")?
            .translation();
        let staying = |position: Vec3| {
            self.targets.iter().any(|(_, _, _, _, _, _, toon)| {
                toon.and_then(|toon| self.bones.get(toon.hips_entity()).ok())
                    .is_some_and(|hips| hips.translation().distance(position) < 0.25)
            })
        };
        let position = if player.translation.distance(hips) >= 0.6 && !staying(player.translation) {
            player.translation
        } else {
            let face = self
                .face
                .as_deref()
                .ok_or("navigation sample is not ready")?;
            // SiteActive.position is the source world's offset. This host
            // currently rebases one active site to its scene-root origin;
            // actors and navigation use that same runtime frame. All shell,
            // module and navigation roots share the active site's frame.
            let site_y = self
                .site_roots
                .iter()
                .next()
                .ok_or("active site coordinate frame is not ready")?
                .translation()
                .y;
            let mut candidates = Vec::with_capacity(8);
            for x in -1..=1 {
                for z in -1..=1 {
                    if x != 0 || z != 0 {
                        candidates.push(Vec3::new(
                            hips.x + x as f32 * 0.6,
                            site_y,
                            hips.z + z as f32 * 0.6,
                        ));
                    }
                }
            }
            candidates.sort_by(|a, b| {
                a.distance_squared(player.translation)
                    .total_cmp(&b.distance_squared(player.translation))
            });
            candidates
                .into_iter()
                .filter_map(|position| face.sample(position.to_array(), 0.05).map(Vec3::from))
                .find(|position| !staying(*position))
                .ok_or("no safe player position among the eight source candidates")?
        };
        if actions.current == NpcAction::ChangeSite {
            return actions
                .is_tweeting
                .then_some(position)
                .ok_or("NPC is changing site without tweeting");
        }
        if actions.current == NpcAction::Talk || !actions.enable_talk {
            return Err("NPC talk is disabled");
        }
        if actions
            .talk_owner
            .is_some_and(|owner| owner != player_entity)
        {
            return Err("NPC is occupied by another player");
        }
        Ok(position)
    }
}

/// Box-test outcomes over a synthetic pose grid, for a value-by-value
/// comparison with an independent transcription of the source's test.
///
/// Every case is given in the source frame: a target box and the player at
/// each of 121 x 121 positions (-3.00 m to 3.00 m in 0.05 m steps, computed
/// in f32 as `(i - 60) as f32 * 0.05`) and 24 facings (Unity yaw 0 to 345
/// degrees in 15 degree steps). The poses are carried into the runtime frame
/// the way placements and the player are: x reflected, a fixture's Left and
/// Right directions swapped, and the player's rotation the reflected yaw.
/// Each case writes one `0`/`1` per pose to `<case>.txt`, facing outermost,
/// then x, then z.
#[cfg(test)]
mod pose_sample {
    use super::*;
    use moly_law::fixture::position::direction_yaw_degrees;
    use moly_law::fixture::Direction;

    const STEPS: i32 = 121;
    const TURNS: i32 = 24;

    fn runtime(source: [f32; 3]) -> Vec3 {
        Vec3::new(-source[0], source[1], source[2])
    }

    fn reflected(direction: Direction) -> Direction {
        match direction {
            Direction::Left => Direction::Right,
            Direction::Right => Direction::Left,
            other => other,
        }
    }

    fn fixture_root(centre: [f32; 3], direction: Direction) -> Transform {
        crate::fixture::source_transform(
            runtime(centre).to_array(),
            direction_yaw_degrees(reflected(direction)).to_radians(),
        )
    }

    fn outcomes(target: &CollisionBox2D) -> String {
        let mut out = String::with_capacity((STEPS * STEPS * TURNS) as usize);
        for k in 0..TURNS {
            let yaw = (k * 15) as f32;
            for i in 0..STEPS {
                for j in 0..STEPS {
                    let x = (i - 60) as f32 * 0.05;
                    let z = (j - 60) as f32 * 0.05;
                    let player = Transform::from_translation(runtime([x, 0.0, z]))
                        .with_rotation(Quat::from_rotation_y(-yaw.to_radians()));
                    out.push(if player_box(&player).collides(target) {
                        '1'
                    } else {
                        '0'
                    });
                }
            }
        }
        out
    }

    #[test]
    #[ignore = "writes the outcomes to the directory named by MOLY_ACTION_BOX_SAMPLE_DIR"]
    fn write_box_sample() {
        let dir = std::path::PathBuf::from(
            std::env::var("MOLY_ACTION_BOX_SAMPLE_DIR")
                .expect("MOLY_ACTION_BOX_SAMPLE_DIR names the output directory"),
        );
        std::fs::create_dir_all(&dir).unwrap();
        // Record Transform query results, not nominal trigonometric angles:
        // a quaternion's f32 forward/euler round trip is part of the input
        // supplied by the host, before the source box equations run.
        let facings: Vec<_> = (0..TURNS).map(|k| {
            let yaw = (k * 15) as f32;
            let rotation = Quat::from_rotation_y(-yaw.to_radians());
            let forward = rotation * Vec3::Z;
            serde_json::json!({"nominalYaw":yaw,
                "forward":[-forward.x,forward.y,forward.z],
                "yaw":crate::site_move::camera::source_yaw(rotation)})
        }).collect();
        let mut house_offsets = serde_json::Map::new();
        for (name, direction) in [
            ("house_front", Direction::Front), ("house_right", Direction::Right),
            ("house_back", Direction::Back), ("house_null", Direction::Left),
            ("house_no_door", Direction::Left),
        ] {
            let root = fixture_root([2.0,0.0,2.0], direction);
            let offset = root.rotation * Vec3::new(0.0,0.0,1.2);
            house_offsets.insert(name.into(), serde_json::json!([-offset.x,offset.y,offset.z]));
        }
        std::fs::write(dir.join("transform-inputs.json"), serde_json::to_vec_pretty(
            &serde_json::json!({"schemaVersion":1,"facings":facings,"houseOffsets":house_offsets})
        ).unwrap()).unwrap();
        let fixtures = [
            (
                "fixture_4x4_front",
                [0.0, 0.0, 0.0],
                (4, 4),
                Direction::Front,
            ),
            (
                "fixture_4x6_right",
                [0.0, 0.0, 0.0],
                (4, 6),
                Direction::Right,
            ),
            (
                "fixture_4x6_front",
                [0.125, 0.0, -0.375],
                (4, 6),
                Direction::Front,
            ),
            (
                "fixture_4x6_left",
                [0.125, 0.0, -0.375],
                (4, 6),
                Direction::Left,
            ),
            (
                "fixture_4x6_back",
                [0.125, 0.0, -0.375],
                (4, 6),
                Direction::Back,
            ),
            (
                "fixture_3x5_right",
                [0.125, 0.0, -0.375],
                (3, 5),
                Direction::Right,
            ),
        ];
        for (name, centre, (width, depth), direction) in fixtures {
            let root = fixture_root(centre, direction);
            let target = fixture_target_box(&root, width, depth, true);
            std::fs::write(dir.join(format!("{name}.txt")), outcomes(&target)).unwrap();
        }
        let houses = [
            ("house_front", [0.4, 0.0, -1.1], Direction::Front),
            ("house_right", [0.4, 0.0, -1.1], Direction::Right),
            ("house_back", [-0.6, 0.0, 0.9], Direction::Back),
        ];
        for (name, door, direction) in houses {
            let root = fixture_root([2.0, 0.0, 2.0], direction);
            let target = house_target_box(&root, Some(runtime(door)), true);
            std::fs::write(dir.join(format!("{name}.txt")), outcomes(&target)).unwrap();
        }
        let root = fixture_root([0.125, 0.0, -0.375], Direction::Left);
        let target = fixture_target_box(&root, 3, 5, false);
        std::fs::write(dir.join("fixture_null.txt"), outcomes(&target)).unwrap();
        let target = house_target_box(&root, Some(runtime([0.4, 0.0, -1.1])), false);
        std::fs::write(dir.join("house_null.txt"), outcomes(&target)).unwrap();
        let target = house_target_box(&root, None, true);
        std::fs::write(dir.join("house_no_door.txt"), outcomes(&target)).unwrap();
        let npc = Transform::from_translation(runtime([0.3, 0.0, -0.2]));
        std::fs::write(
            dir.join("character.txt"),
            outcomes(&character_target_box(&npc)),
        )
        .unwrap();
    }
}
