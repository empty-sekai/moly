//! Player interaction qualification shared by buttons, world picks and request consumers.

use bevy::{ecs::system::SystemParam, prelude::*};
use moly_law::action_button::{
    character_box, update_player_box, CollisionBox2D, PLAYER_ADDITIONAL_HALF_EXTEND,
};

use crate::{
    menu_shell::ShellDialogState,
    npc::{CharacterUnitId, NpcAction, NpcActions},
    player::PlayerControlled,
    player_state::PlayerAvatarStates,
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
        let Ok((_, target, id, actions, visibility, inherited, _)) = self.targets.get(entity) else {
            return false;
        };
        if id.0 != unit
            || player_hold.is_some()
            || !actions.ready()
            || !self.site.as_ref().is_some_and(|site| site.site_type == actions.site_type)
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
        if !self.allows(entity, unit) { return Err("target is not in the active interaction stack"); }
        let (player_entity, player, _) = self.players.single().map_err(|_| "player is not ready")?;
        let (_, _, _, actions, _, _, toon) = self.targets.get(entity).map_err(|_| "NPC left the scene")?;
        let hips = self.bones.get(toon.ok_or("NPC skeleton is not ready")?.hips_entity())
            .map_err(|_| "NPC hips transform is not ready")?.translation();
        let staying = |position: Vec3| self.targets.iter().any(|(_, _, _, _, _, _, toon)| {
            toon.and_then(|toon| self.bones.get(toon.hips_entity()).ok())
                .is_some_and(|hips| hips.translation().distance(position) < 0.25)
        });
        let position = if player.translation.distance(hips) >= 0.6 && !staying(player.translation) {
            player.translation
        } else {
            let face = self.face.as_deref().ok_or("navigation sample is not ready")?;
            // SiteActive.position is the source world's offset. This host
            // currently rebases one active site to its scene-root origin;
            // actors and navigation use that same runtime frame. All shell,
            // module and navigation roots share the active site's frame.
            let site_y = self.site_roots.iter().next()
                .ok_or("active site coordinate frame is not ready")?.translation().y;
            let mut candidates = Vec::with_capacity(8);
            for x in -1..=1 { for z in -1..=1 {
                if x != 0 || z != 0 {
                    candidates.push(Vec3::new(hips.x + x as f32 * 0.6, site_y, hips.z + z as f32 * 0.6));
                }
            }}
            candidates.sort_by(|a, b| a.distance_squared(player.translation).total_cmp(&b.distance_squared(player.translation)));
            candidates.into_iter().filter_map(|position| face.sample(position.to_array(), 0.05).map(Vec3::from))
                .find(|position| !staying(*position)).ok_or("no safe player position among the eight source candidates")?
        };
        if actions.current == NpcAction::ChangeSite {
            return actions.is_tweeting.then_some(position).ok_or("NPC is changing site without tweeting");
        }
        if actions.current == NpcAction::Talk || !actions.enable_talk { return Err("NPC talk is disabled"); }
        if actions.talk_owner.is_some_and(|owner| owner != player_entity) { return Err("NPC is occupied by another player"); }
        Ok(position)
    }
}
