//! `HarvestObjectPresenter.OnDamage` and what a hit shows.
//!
//! Per hit: `UpdateHp`; then by the view's interface.
//! - Multi (tree, stone): `PlayMultiActionDamageEffect(toolLevel, boost,
//!   isLast)` = the hit effect by tool level, the boost effect, `PlayHitSE`,
//!   `DamageAnimation` (the punch), and on the last attack `PlayLastAttackSE`;
//!   a rare last attack also `PlayRareObjectBreakSE`; `HandleResourceDrop`;
//!   on the last attack `ChangeAfterObject` and `RemoveCollisionObject`.
//! - Single (the other kinds): `PlayDamageEffect`, the boost effect,
//!   `PlaySE`, the punch, `HandleResourceDrop`, `ChangeAfterObject`,
//!   `RemoveCollisionObject`.
//!
//! The punch is `DOPunchPosition((0.01, 0, 0), 0.7, 6, 1)` on every kind,
//! never killed first (two overlapping punches run together). In the product
//! frame the source x axis is reflected, so the punch goes along -x.
//!
//! The EffectOnly clip event plays the effect part alone
//! (`PlayDamageEffectForIndefinite`: the multi views' effect, boost effect,
//! hit SE and punch without the last-attack SEs; the single views'
//! `PlayDamageEffect`, boost effect and SE), with no damage and no drop.

use bevy::prelude::*;

use bevy::diagnostic::FrameCount;
use bevy::ecs::system::SystemParam;
use moly_assets::source_navigation::SourceObjectIdentity;

use super::law::{dither_variant_on, fade_step, punch_points, FadeStep, PointTween, SegmentEase};
use super::prop_animator::{PropAnimatorCalls, PropCall};
use super::{
    ActionInterface, DropBatch, EffectHook, HarvestDropBatches, HarvestEffectHooks,
    HarvestHitResults, HarvestHits, HarvestObject, HarvestStats, HarvestViewNodes, HitResult,
    PendingDrop, STATUS_HARVESTED,
};
use crate::audio::{SeClass, SeRequest, SeRequests};
use crate::player::PlayerControlled;
use crate::site_material::SiteMaterial;
use crate::site_move::timeline::Delay;

/// `DamageAnimation`'s punch in the product frame (source (0.01, 0, 0)).
const PUNCH: Vec3 = Vec3::new(-0.01, 0.0, 0.0);
const PUNCH_DURATION: f32 = 0.7;
const PUNCH_VIBRATO: i32 = 6;
const PUNCH_ELASTICITY: f32 = 1.0;

/// The tree fall: the top rotates to local Euler (90, 0, 0) over 2.0 s with
/// InQuart; `PlayFadeAsync` waits 2.0 s (`UniTask.Delay`), emits 132 at the
/// delete position and runs `PlayFadeAnimation(material, 0.5)` on the top's
/// own material: the dither value falls from 1 to 0 over 0.5 s, then the top
/// turns off.
const FALL_DURATION: f32 = 2.0;
const FADE_DELAY: f32 = 2.0;
const FADE_DURATION: f32 = 0.5;
/// Driftage and toolbox disappear after `UniTask.Delay(1.0 s)`.
const DELAYED_HIDE: f32 = 1.0;

/// EffectOnly requests of this frame (`PlayDamageEffectForIndefinite`).
#[derive(Resource, Default)]
pub(crate) struct HarvestEffectOnly(pub(crate) Vec<EffectOnly>);

pub(crate) struct EffectOnly {
    pub(crate) target: Entity,
    pub(crate) tool_level: i32,
    pub(crate) is_boost: bool,
}

/// Object turns to start (`DORotate((0, yaw, 0), 0.2, Fast)`, the default
/// ease OutQuad): the treasure box's PostStartAction. Yaw in the product
/// frame.
#[derive(Resource, Default)]
pub(crate) struct HarvestTurnRequests(pub(crate) Vec<(Entity, f32)>);

/// The treasure box's turn duration.
const TURN_DURATION: f32 = 0.2;

/// The toolbox's `OnPlayerActionStart` also waits `Delay(1.0 s)` and then
/// turns its box off (and stops its cut particle): the earlier of this and
/// its `ChangeAfterObject` delay hides it.
#[derive(Resource, Default)]
pub(crate) struct HarvestStartHides(pub(crate) Vec<(Entity, Delay)>);

/// Running punches of one object, in creation order.
#[derive(Component, Default)]
pub(crate) struct HarvestPunches(Vec<PointTween>);

/// A disappearance in progress (`ChangeAfterObject`).
#[derive(Component)]
pub(crate) enum HarvestAfterForm {
    TreeFall {
        elapsed: f32,
        from: Option<Quat>,
        /// `PlayFadeAsync`'s `Delay(2.0 s)`, created with the fall.
        delay: Delay,
        /// `PlayFadeAnimation`'s time once the delay is due.
        fade: Option<f32>,
        /// The top's own material copies (`renderer.material`), made when
        /// the fade starts.
        materials: Vec<Handle<SiteMaterial>>,
    },
    DelayedHide {
        delay: Delay,
    },
    /// The tone's ChangeAfterObject: the listen clip's length, then its SE
    /// stops (the field effect stopped at the hit).
    ToneStop {
        delay: Delay,
    },
}

pub(crate) fn push_se(se: &mut SeRequests, cue: &str, source: &'static str) {
    se.0.push(SeRequest {
        owner: None,
        cue: cue.to_owned(),
        class: SeClass::Ingame,
        source,
    });
}

/// Hit effect by tool level (`PlayDamageNormalEffect`): tree 121 / 122 / 123
/// (levels 3 and 4) / 124; stone 111 / 112 / 113 / 114.
fn multi_hit_effect(class: &str, tool_level: i32) -> Option<u16> {
    let base = match class {
        "MysekaiAreaTreeView" => 120,
        "MysekaiAreaStoneView" => 110,
        _ => return None,
    };
    Some(match tool_level {
        1 => base + 1,
        2 => base + 2,
        3 | 4 => base + 3,
        5 => base + 4,
        _ => return None,
    })
}

/// Update: drain the hit queue through `OnDamage`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn on_damage(
    mut commands: Commands,
    mut hits: ResMut<HarvestHits>,
    mut results: ResMut<HarvestHitResults>,
    mut objects: Query<(
        &mut HarvestObject,
        &mut Visibility,
        &Transform,
        Option<&HarvestViewNodes>,
    )>,
    mut punches: Query<&mut HarvestPunches>,
    mut node_visibility: Query<&mut Visibility, Without<HarvestObject>>,
    players: Query<&Transform, (With<PlayerControlled>, Without<HarvestObject>)>,
    mut batches: ResMut<HarvestDropBatches>,
    mut stats: ResMut<HarvestStats>,
    mut se: ResMut<SeRequests>,
    mut effects: ResMut<HarvestEffectHooks>,
    configs: Option<Res<crate::client_config::ClientConfigs>>,
    frames: Res<FrameCount>,
    mut animator_calls: ResMut<PropAnimatorCalls>,
    clips: Option<Res<super::clips::HarvestClips>>,
) {
    let Some(configs) = configs else {
        return;
    };
    let drop_delay_count =
        configs.int(crate::client_config::KEY_HARVEST_DROP_DELAY_ITEM_COUNT) as usize;
    let player = players.single().ok().map(|transform| transform.translation);
    for hit in std::mem::take(&mut hits.0) {
        let Ok((mut object, mut visibility, transform, nodes)) = objects.get_mut(hit.target) else {
            warn!("[harvest] a hit names no harvest object: {:?}", hit.target);
            continue;
        };
        let already = object.status == STATUS_HARVESTED;
        object.hits_taken += 1;
        let returned = object.update_hp(hit.damage);
        stats.hits += 1;
        if already {
            stats.idle_returns += 1;
            info!(
                "[harvest] hit on {}#{} already harvested: UpdateHp returned {returned}, nothing shown",
                object.leaf, object.fixture_id
            );
            results.0.push(HitResult {
                target: hit.target,
                damage: hit.damage,
                used: returned,
                is_last_attack: object.is_last_attack,
                tool: hit.tool,
            });
            continue;
        }
        let last = object.is_last_attack;
        let position = transform.translation;
        let (cues, hooks) = damage_effect(
            hit.target,
            &object,
            position,
            player,
            hit.tool_level,
            hit.is_boost,
            last,
            &mut effects,
            &mut se,
            &mut animator_calls,
            "harvest-hit",
        );
        // DamageAnimation on every kind.
        push_punch(&mut commands, &mut punches, hit.target);
        // HandleResourceDrop: the last attack takes every remaining row
        // (row hp >= hp after); other hits the rows with hp < row hp <= prev.
        assert!(
            object.fixture_type < 10,
            "fixture type {} is outside the drop gate",
            object.fixture_type
        );
        let (hp, prev_hp) = (object.hp, object.prev_hp);
        let admitted: Vec<PendingDrop> = object
            .pending_drops
            .iter()
            .filter(|drop| {
                if last {
                    drop.row.hp >= hp
                } else {
                    hp < drop.row.hp && drop.row.hp <= prev_hp
                }
            })
            .cloned()
            .collect();
        let mut tail = String::new();
        if !admitted.is_empty() {
            let count = admitted.len();
            let delay = drop_delay_count <= count;
            super::drops::play_drop_item_se(object.fixture_type, &admitted, &mut se);
            batches.0.push(DropBatch {
                origin: Some(hit.target),
                position_x: object.position_x,
                position_z: object.position_z,
                fixture_type: object.fixture_type,
                remaining: admitted,
                delay,
            });
            stats.drop_batches += 1;
            tail = format!(
                " -> drop batch of {count} (thresholds {:?}; {} with HarvestDropDelayItemCount {drop_delay_count})",
                batches
                    .0
                    .last()
                    .expect("pushed")
                    .remaining
                    .iter()
                    .map(|d| d.row.hp)
                    .collect::<Vec<_>>(),
                if delay { "one a frame" } else { "same frame" }
            );
        }
        let disappears = object.interface == ActionInterface::Single || last;
        if disappears {
            match object.interface {
                ActionInterface::Multi => stats.multi_final += 1,
                ActionInterface::Single => stats.single_hits += 1,
            }
            change_after_object(
                &mut commands,
                hit.target,
                &object,
                position,
                &mut visibility,
                nodes,
                &mut node_visibility,
                &mut effects,
                u64::from(frames.0),
                // GetHarvestActionTime(null, tone): the listen clip's length.
                clips
                    .as_deref()
                    .and_then(|clips| clips.get("mov_u000_site_listen01_o"))
                    .map_or(0.0, |clip| clip.length),
            );
            object.collision = false;
            tail.push_str(" -> ChangeAfterObject + RemoveCollisionObject");
        } else {
            stats.multi_hits += 1;
        }
        info!(
            "[harvest] OnDamage {}#{} hit {}: damage {} hp {} -> {} returned {} last {} ({:?}) SE {:?} effect hooks {:?}{tail}",
            object.leaf,
            object.fixture_id,
            object.hits_taken,
            hit.damage,
            prev_hp,
            hp,
            returned,
            last,
            object.interface,
            cues,
            hooks,
        );
        results.0.push(HitResult {
            target: hit.target,
            damage: hit.damage,
            used: returned,
            is_last_attack: last,
            tool: hit.tool,
        });
    }
}

fn hide(
    entity: Option<Entity>,
    root_visibility: &mut Visibility,
    nodes: &mut Query<&mut Visibility, Without<HarvestObject>>,
) {
    match entity.and_then(|entity| nodes.get_mut(entity).ok()) {
        Some(mut visibility) => *visibility = Visibility::Hidden,
        None => *root_visibility = Visibility::Hidden,
    }
}

#[allow(clippy::too_many_arguments)]
fn change_after_object(
    commands: &mut Commands,
    root: Entity,
    object: &HarvestObject,
    position: Vec3,
    root_visibility: &mut Visibility,
    nodes: Option<&HarvestViewNodes>,
    node_visibility: &mut Query<&mut Visibility, Without<HarvestObject>>,
    effects: &mut HarvestEffectHooks,
    frame: u64,
    listen_length: f32,
) {
    let object_node = nodes.and_then(|nodes| nodes.object);
    match object.class {
        "MysekaiAreaTreeView" => {
            let (Some(after), Some(nodes)) = (nodes.and_then(|n| n.after), nodes) else {
                warn!(
                    "[harvest] {}#{}: tree after mesh unresolved; the whole tree hides",
                    object.leaf, object.fixture_id
                );
                *root_visibility = Visibility::Hidden;
                return;
            };
            // FallDownAnimation: before off, after on, under on.
            hide(object_node, root_visibility, node_visibility);
            if let Ok(mut visibility) = node_visibility.get_mut(after) {
                *visibility = Visibility::Inherited;
            }
            if let Some(mut visibility) = nodes
                .under
                .and_then(|under| node_visibility.get_mut(under).ok())
            {
                *visibility = Visibility::Inherited;
            }
            commands.entity(root).insert(HarvestAfterForm::TreeFall {
                elapsed: 0.0,
                from: None,
                delay: Delay::new(FADE_DELAY, frame),
                fade: None,
                materials: Vec::new(),
            });
        }
        "MysekaiAreaStoneView" => {
            hide(object_node, root_visibility, node_visibility);
            effects.pending.push(EffectHook::at(131, position));
        }
        "MysekaiAreadDriftageView" | "MysekaiAreaToolBoxView" => {
            // The driftage view first disables `_navmeshObstacle`.
            if object.class == "MysekaiAreadDriftageView" {
                commands.queue(move |world: &mut World| {
                    super::obstacles::switch(world, root, "_navmeshObstacle", false, "ChangeAfterObject");
                });
            }
            commands.entity(root).insert(HarvestAfterForm::DelayedHide {
                delay: Delay::new(DELAYED_HIDE, frame),
            });
        }
        // The treasure box's ChangeAfterObject turns its lid obstacle on,
        // waits 2.0 s and stops the cut particle: the opened box stays.
        // PlayDamageEffect stopped the field effect (its node hides here);
        // ChangeAfterObject waits the listen clip's length, then stops the
        // SE and fades the BGM back (the fade is not ported).
        "MysekaiAreaToneView" => {
            hide(object_node, root_visibility, node_visibility);
            commands.entity(root).insert(HarvestAfterForm::ToneStop {
                delay: Delay::new(listen_length, frame),
            });
        }
        "MysekaiAreaTreasureBoxView" => {
            commands.queue(move |world: &mut World| {
                super::obstacles::switch(world, root, "_treasureBoxLidNavMeshObstacle", true, "ChangeAfterObject");
            });
            info!(
                "[harvest] {}#{} ChangeAfterObject: the opened box stays (lid obstacle enabled; cut particle not modelled)",
                object.leaf, object.fixture_id
            );
        }
        _ => hide(object_node, root_visibility, node_visibility),
    }
}

/// Update: advance every running punch in creation order; the last written
/// value stands, as with two DOTween punches on one transform.
pub(crate) fn advance_punches(
    time: Res<Time>,
    mut commands: Commands,
    mut query: Query<(Entity, &mut HarvestPunches, &mut Transform)>,
) {
    let dt = time.delta_secs();
    for (entity, mut punches, mut transform) in &mut query {
        let mut current = transform.translation;
        for tween in &mut punches.0 {
            current = tween.advance(current, dt);
        }
        transform.translation = current;
        punches.0.retain(|tween| !tween.done());
        if punches.0.is_empty() {
            commands.entity(entity).remove::<HarvestPunches>();
        }
    }
}

/// The top's material handles: the site material on the after node and on
/// its own mesh primitives (the node's children without a source identity).
#[derive(SystemParam)]
pub(crate) struct TopMaterials<'w, 's> {
    materials: ResMut<'w, Assets<SiteMaterial>>,
    handles: Query<'w, 's, &'static MeshMaterial3d<SiteMaterial>>,
    children: Query<'w, 's, &'static Children>,
    identities: Query<'w, 's, (), With<SourceObjectIdentity>>,
}

impl TopMaterials<'_, '_> {
    fn primitives(&self, node: Entity) -> Vec<Entity> {
        let mut out = vec![node];
        if let Ok(kids) = self.children.get(node) {
            out.extend(kids.iter().filter(|kid| self.identities.get(*kid).is_err()));
        }
        out
    }

    /// `renderer.material`: one copy of each primitive's material, so only
    /// this tree's top changes.
    fn instance(&mut self, commands: &mut Commands, node: Entity) -> Vec<Handle<SiteMaterial>> {
        let mut copies = Vec::new();
        for entity in self.primitives(node) {
            let Ok(handle) = self.handles.get(entity) else {
                continue;
            };
            let Some(material) = self.materials.get(&handle.0).cloned() else {
                continue;
            };
            let copy = self.materials.add(material);
            commands.entity(entity).insert(MeshMaterial3d(copy.clone()));
            copies.push(copy);
        }
        copies
    }

    /// `SetDitherAlpha(value)` on the copies; returns what the first copy
    /// now holds (its pipeline key's dither flag and `_DitherAlpha`).
    fn set_dither(&mut self, copies: &[Handle<SiteMaterial>], value: f32) -> Option<(bool, f32)> {
        for handle in copies {
            if let Some(material) = self.materials.get_mut(handle) {
                material.key.tree_dither = dither_variant_on(value);
                material.params.dither_alpha = value;
            }
        }
        copies
            .first()
            .and_then(|handle| self.materials.get(handle))
            .map(|material| (material.key.tree_dither, material.params.dither_alpha))
    }
}

/// Update: the tree fall and the delayed disappearances.
#[allow(clippy::type_complexity, clippy::too_many_arguments)]
pub(crate) fn advance_after_forms(
    time: Res<Time>,
    frames: Res<FrameCount>,
    mut commands: Commands,
    mut forms: Query<(
        Entity,
        &mut HarvestAfterForm,
        &HarvestObject,
        &HarvestViewNodes,
        &mut Visibility,
    )>,
    mut nodes: Query<(&mut Transform, &mut Visibility, &GlobalTransform), Without<HarvestObject>>,
    mut effects: ResMut<HarvestEffectHooks>,
    mut tops: TopMaterials,
) {
    let dt = time.delta_secs();
    let frame = u64::from(frames.0);
    for (entity, mut form, object, view, mut root_visibility) in &mut forms {
        match &mut *form {
            HarvestAfterForm::TreeFall {
                elapsed,
                from,
                delay,
                fade,
                materials,
            } => {
                let Some(after) = view.after else {
                    commands.entity(entity).remove::<HarvestAfterForm>();
                    continue;
                };
                *elapsed += dt;
                if let Ok((mut transform, _, _)) = nodes.get_mut(after) {
                    let start = *from.get_or_insert(transform.rotation);
                    let t = (*elapsed / FALL_DURATION).clamp(0.0, 1.0);
                    let eased = t * t * t * t;
                    transform.rotation =
                        start.slerp(Quat::from_rotation_x(std::f32::consts::FRAC_PI_2), eased);
                }
                if fade.is_none() && delay.tick(frame, dt) {
                    let position = view
                        .delete_at
                        .and_then(|node| nodes.get(node).ok())
                        .map(|(_, _, global)| global.translation())
                        .unwrap_or_default();
                    effects.pending.push(EffectHook::at(132, position));
                    *materials = tops.instance(&mut commands, after);
                    *fade = Some(0.0);
                    if materials.is_empty() {
                        warn!(
                            "[harvest] {}#{} fall: delay done at {:.3} s, delete effect 132 at {position:.2}; the top has no site material to dither (material swap not applied), it hides when the fade ends",
                            object.leaf, object.fixture_id, elapsed
                        );
                    } else {
                        info!(
                            "[harvest] {}#{} fall: delay done at {:.3} s, delete effect 132 at {position:.2}; PlayFadeAnimation on {} copied top materials",
                            object.leaf,
                            object.fixture_id,
                            elapsed,
                            materials.len()
                        );
                    }
                }
                let Some(fade_time) = fade.as_mut() else {
                    continue;
                };
                let before = *fade_time;
                match fade_step(fade_time, FADE_DURATION, dt) {
                    FadeStep::Set(value) => {
                        let held = tops.set_dither(materials, value);
                        info!(
                            "[harvest-fade] {}#{} SetDitherAlpha({value:.4}) at fade time {before:.4}: the top material holds (tree_dither, _DitherAlpha) = {held:?}",
                            object.leaf, object.fixture_id
                        );
                    }
                    FadeStep::Finish => {
                        tops.set_dither(materials, 0.0);
                        if let Ok((_, mut visibility, _)) = nodes.get_mut(after) {
                            *visibility = Visibility::Hidden;
                        }
                        info!(
                            "[harvest] {}#{} fall done at {:.3} s (fade time {before:.4}): SetDitherAlpha(0), top off, the under part stays until the next load",
                            object.leaf, object.fixture_id, elapsed
                        );
                        commands.entity(entity).remove::<HarvestAfterForm>();
                    }
                }
            }
            HarvestAfterForm::ToneStop { delay } => {
                if delay.tick(frame, dt) {
                    let owner = entity;
                    commands.queue(move |world: &mut World| {
                        crate::audio::dispose_scoped_se(world, owner);
                    });
                    info!(
                        "[harvest-tone] {}#{} ChangeAfterObject: the listen time passed; the tone SE stopped (the BGM fade back is not ported)",
                        object.leaf, object.fixture_id
                    );
                    commands.entity(entity).remove::<HarvestAfterForm>();
                }
            }
            HarvestAfterForm::DelayedHide { delay } => {
                if delay.tick(frame, dt) {
                    match view.object.and_then(|node| nodes.get_mut(node).ok()) {
                        Some((_, mut visibility, _)) => *visibility = Visibility::Hidden,
                        None => *root_visibility = Visibility::Hidden,
                    }
                    info!(
                        "[harvest] {}#{} off after the 1.0 s delay",
                        object.leaf, object.fixture_id
                    );
                    commands.entity(entity).remove::<HarvestAfterForm>();
                }
            }
        }
    }
}

/// The effect part of `PlayMultiActionDamageEffect(toolLevel, boost, last)`
/// and `PlaySingleActionDamageEffect(boost)`: the effect hooks, the boost
/// effect, the SE cues (the last-attack SEs only with `last`), and the
/// treasure box's `open`. The punch is the caller's. Returns the cues and
/// the hooks for the caller's line.
#[allow(clippy::too_many_arguments)]
fn damage_effect(
    target: Entity,
    object: &HarvestObject,
    position: Vec3,
    player: Option<Vec3>,
    tool_level: i32,
    is_boost: bool,
    last: bool,
    effects: &mut HarvestEffectHooks,
    se: &mut SeRequests,
    animator_calls: &mut PropAnimatorCalls,
    label: &'static str,
) -> (Vec<&'static str>, Vec<u16>) {
    // The multi effects stand 0.2 m back toward the player and 0.35 m up.
    let effect_at = match player {
        Some(player) => {
            let dir = Vec2::new(position.x - player.x, position.z - player.z).normalize_or_zero();
            position + Vec3::new(-0.2 * dir.x, 0.35, -0.2 * dir.y)
        }
        None => position + Vec3::Y * 0.35,
    };
    // ... and turn to face the player (`LookRotation(-0.2 * dir)`; in the
    // product frame the yaw of that direction).
    let facing = match player {
        Some(player) => {
            let back = Vec2::new(player.x - position.x, player.z - position.z);
            if back.length_squared() > 0.0 {
                Quat::from_rotation_y(back.x.atan2(back.y))
            } else {
                Quat::IDENTITY
            }
        }
        None => Quat::IDENTITY,
    };
    let mut cues: Vec<&'static str> = Vec::new();
    let mut hooks: Vec<u16> = Vec::new();
    match object.interface {
        ActionInterface::Multi => {
            if let Some(kind) = multi_hit_effect(object.class, tool_level) {
                hooks.push(kind);
            }
            if is_boost {
                hooks.push(if tool_level == 5 { 143 } else { 141 });
            }
            cues.extend(object.cues.hit);
            if last {
                cues.extend(object.cues.last);
                cues.extend(object.cues.rare_break);
            }
        }
        ActionInterface::Single => {
            match object.class {
                "MysekaiAreaPlantView" => hooks.push(101),
                "MysekaiAreadDriftageView" => hooks.push(133),
                // PlayDamageEffect: the cut particle plays (not drawn) and
                // the Animator opens.
                "MysekaiAreaTreasureBoxView" => animator_calls
                    .0
                    .push((target, PropCall::SetBool("open", true))),
                _ => {}
            }
            if is_boost
                && matches!(
                    object.class,
                    "MysekaiAreaJunkView" | "MysekaiAreadDriftageView" | "MysekaiBirthdayPlantView"
                )
            {
                hooks.push(141);
            }
            cues.extend(object.cues.hit);
        }
    }
    // The single-action views emit at their own position.
    for kind in &hooks {
        effects.pending.push(match object.interface {
            ActionInterface::Multi => EffectHook {
                kind: *kind,
                position: effect_at,
                rotation: facing,
            },
            ActionInterface::Single => EffectHook::at(*kind, position),
        });
    }
    for cue in &cues {
        push_se(se, cue, label);
    }
    (cues, hooks)
}

/// `DOPunchPosition((0.01, 0, 0), 0.7, 6, 1)` added to the object's running
/// punches.
fn push_punch(commands: &mut Commands, punches: &mut Query<&mut HarvestPunches>, target: Entity) {
    let tween = PointTween::new(
        punch_points(PUNCH, PUNCH_DURATION, PUNCH_VIBRATO, PUNCH_ELASTICITY),
        SegmentEase::OutQuad,
    );
    match punches.get_mut(target) {
        Ok(mut list) => list.0.push(tween),
        Err(_) => {
            commands.entity(target).insert(HarvestPunches(vec![tween]));
        }
    }
}

/// Update, after the action: the EffectOnly clip events
/// (`PlayDamageEffectForIndefinite(toolLevel, boost)`, `isLastAttack` false).
#[allow(clippy::too_many_arguments)]
pub(crate) fn on_effect_only(
    mut commands: Commands,
    mut requests: ResMut<HarvestEffectOnly>,
    objects: Query<(&HarvestObject, &Transform)>,
    mut punches: Query<&mut HarvestPunches>,
    players: Query<&Transform, (With<PlayerControlled>, Without<HarvestObject>)>,
    mut se: ResMut<SeRequests>,
    mut effects: ResMut<HarvestEffectHooks>,
    mut animator_calls: ResMut<PropAnimatorCalls>,
) {
    let player = players.single().ok().map(|transform| transform.translation);
    for request in std::mem::take(&mut requests.0) {
        let Ok((object, transform)) = objects.get(request.target) else {
            continue;
        };
        let (cues, hooks) = damage_effect(
            request.target,
            object,
            transform.translation,
            player,
            request.tool_level,
            request.is_boost,
            false,
            &mut effects,
            &mut se,
            &mut animator_calls,
            "harvest-effect-only",
        );
        let punched = matches!(object.interface, ActionInterface::Multi);
        if punched {
            push_punch(&mut commands, &mut punches, request.target);
        }
        info!(
            "[harvest] {}#{} PlayDamageEffectForIndefinite: effect hooks {:?}, SE {:?}{}",
            object.leaf,
            object.fixture_id,
            hooks,
            cues,
            if punched { ", punch" } else { "" }
        );
    }
}

/// Update: object turns (`DORotate` on the yaw, `RotateMode.Fast`: the
/// change wrapped to the shortest way; default ease OutQuad; the frame that
/// starts a tween counts).
pub(crate) fn advance_turns(
    time: Res<Time>,
    mut requests: ResMut<HarvestTurnRequests>,
    mut running: Local<Vec<(Entity, f32, f32, crate::site_move::timeline::TweenClock)>>,
    mut objects: Query<(&HarvestObject, &mut Transform)>,
) {
    let dt = time.delta_secs();
    for (entity, to) in std::mem::take(&mut requests.0) {
        if let Ok((_, transform)) = objects.get(entity) {
            let from = transform.rotation.to_euler(EulerRot::YXZ).0;
            running.push((
                entity,
                from,
                to,
                crate::site_move::timeline::TweenClock::new(TURN_DURATION),
            ));
        }
    }
    running.retain_mut(|(entity, from, to, clock)| {
        let Ok((object, mut transform)) = objects.get_mut(*entity) else {
            return false;
        };
        let t = crate::camera::out_quad(clock.advance(dt));
        transform.rotation = Quat::from_rotation_y(super::law::fast_yaw(*from, *to, t));
        if clock.done() {
            info!(
                "[harvest] {}#{} turn done: yaw {:.2} deg (read back; target {:.2} deg, from {:.2} deg)",
                object.leaf,
                object.fixture_id,
                transform.rotation.to_euler(EulerRot::YXZ).0.to_degrees(),
                to.to_degrees(),
                from.to_degrees()
            );
            return false;
        }
        true
    });
}

/// Update: the toolbox's swing-start hide.
pub(crate) fn advance_start_hides(
    time: Res<Time>,
    frames: Res<FrameCount>,
    mut hides: ResMut<HarvestStartHides>,
    mut objects: Query<(&HarvestObject, Option<&HarvestViewNodes>, &mut Visibility)>,
    mut nodes: Query<&mut Visibility, Without<HarvestObject>>,
) {
    let dt = time.delta_secs();
    let frame = u64::from(frames.0);
    hides.0.retain_mut(|(entity, delay)| {
        if !delay.tick(frame, dt) {
            return true;
        }
        if let Ok((object, view, mut root_visibility)) = objects.get_mut(*entity) {
            hide(
                view.and_then(|v| v.object),
                &mut root_visibility,
                &mut nodes,
            );
            info!(
                "[harvest] {}#{} off 1.0 s after the swing start (OnPlayerActionStart's delay)",
                object.leaf, object.fixture_id
            );
        }
        false
    });
}
