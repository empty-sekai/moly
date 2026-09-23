//! Player interaction qualification shared by buttons, world picks and request consumers.

use bevy::{ecs::system::SystemParam, prelude::*};
use moly_law::action_button::{
    character_box, update_player_box, CollisionBox2D, PLAYER_ADDITIONAL_HALF_EXTEND,
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

pub(crate) fn player_box(player: &Transform) -> CollisionBox2D {
    let mut bounds = CollisionBox2D::new([0.0, 0.0], PLAYER_ADDITIONAL_HALF_EXTEND, 0.0);
    update_player_box(
        &mut bounds,
        player.translation.to_array(),
        (player.rotation * Vec3::Z).to_array(),
        player.rotation.to_euler(EulerRot::YXZ).0.to_degrees(),
    );
    bounds
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
    settings_panel: Res<'w, crate::game_settings::SettingsPanel>,
    player_talk: Option<Res<'w, PlayerTalkSession>>,
    pair_talk: Option<Res<'w, ActiveTalk>>,
}

impl InteractionEligibility<'_, '_> {
    pub(crate) fn available(&self) -> bool {
        self.layers.on_field()
            && !self.settings_panel.blocks_world_input()
            && !self.edits.is_active()
            && !self.dialogs.blocks_field_input()
            && self.player_state.can_intercept
            && self.player_talk.is_none()
            && self.pair_talk.is_none()
    }

    /// The field accepts a button tap, without any reading of the player's
    /// own state. A conversation between NPCs does not close it; one with
    /// the player does.
    pub(crate) fn field_input_open(&self) -> bool {
        self.layers.on_field()
            && !self.settings_panel.blocks_world_input()
            && !self.edits.is_active()
            && !self.dialogs.blocks_field_input()
            && !self.player_in_talk()
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
    /// and Sketch. Of the other states Moly has Edit (the layout editor) and
    /// Talk (a conversation with the player); a harvest, a gimmick switch, a
    /// timeline, a shell dialog or a pushed layer stay in Normal or Harvest.
    pub(crate) fn collision_updates(&self) -> bool {
        !self.edits.is_active() && !self.player_in_talk()
    }

    /// ScreenLayerMysekaiHome, which holds the action buttons, is the current
    /// screen. The layout editor (SiteEditMode), a conversation with the
    /// player (MysekaiTalk) and every layer of the layer stack are entered by
    /// PushUIScreen, whose PushUIScreenCore exits the current screen first.
    /// Dialogs are not screens and leave it current.
    pub(crate) fn home_screen_current(&self) -> bool {
        self.layers.on_field() && !self.edits.is_active() && !self.player_in_talk()
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
            || !player_box(player).collides(&character_box(target.translation.to_array()))
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
