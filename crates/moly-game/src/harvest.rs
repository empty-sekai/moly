//! Harvesting on a harvest site: placement on arrival, the harvest action
//! loop, hit resolution, drops, pickup and the server log loop.
//!
//! Flow (each piece names the source method it follows, in its module):
//! - `catalog`: the extracted package index joined with the masters, the
//!   `HarvestServerMock` panel and the login fetch of the user data.
//! - `arrival`: `HarvestSiteController.Initialize` on every harvest-site
//!   arrival: the user harvest map, one fixture per frame, snap, the yaw and
//!   scale draws, the status branch, the unclaimed drops.
//! - `action`: target selection by proximity, the harvest button (the
//!   `ScreenLayerMysekaiHarvest` action button, plus the F key as a named
//!   stand-in), `PlayHarvestAction` with its cool-down and animation waits,
//!   the hit clock from the source swing clips' AnimationEvents, stamina and
//!   durability, the player's source clip and the tool in hand.
//! - `damage`: `HarvestObjectPresenter.OnDamage`: `UpdateHp`, the multi /
//!   single dispatch, SEs, the punch, `HandleResourceDrop`, the per-kind
//!   disappearance (the tree fall and the top's dither fade).
//! - `drops`: `CreateDropItem`: SE, pacing, scatter, the drop hop.
//! - `pickup`: drops approach the moving player and are collected.
//! - `queue`: the 1.0 s harvest log loop, request merging, the mock replies
//!   and the event 30 / 31 refresh.
//!
//! - `effects`: the harvest effects (101-143) from the effect table.
//!
//! - `prop_animator`: the Animator of the barrel, the toolbox and the
//!   treasure boxes, run from the controller in the package document.
//! - `airplane`: the paper airplane that delivers the transported treasure
//!   box (`TreasureBoxListMock` / `TreasureBoxSpawnMock` in the mock panel).
//! - `tone`: the HarvestTone camera (state 14) and the tone view's SE.
//! - `learn`: learning today's phenomenon on arrival (GameState 6, camera
//!   state 17, `ReleaseApiMock`).
//!
//! Named gaps: harvest objects do not carve the walk field (the source's
//! NavMeshObstacle, also the ones the driftage and treasure views switch);
//! the particle systems the driftage, toolbox and treasure views play and
//! stop are not drawn; drop models keep their glb materials.

pub(crate) mod action;
mod airplane;
mod arrival;
pub(crate) mod catalog;
mod clips;
mod damage;
mod drops;
mod effects;
pub(crate) mod law;
mod pickup;
mod prop_animator;
mod queue;
pub(crate) mod server_mock;
mod learn;
mod tone;
mod tool_model;
mod ui;

use std::collections::HashMap;

use bevy::ecs::observer::On;
use bevy::gltf::Gltf;
use bevy::prelude::*;
use bevy::scene::SceneInstanceReady;

use crate::site::GroundMeshes;
use catalog::HarvestCatalog;
use server_mock::UserDrop;

pub(crate) use action::HarvestAutoMoveHeld;
pub(crate) use arrival::snap_from;
pub(crate) use drops::HarvestDropAnimation;
pub(crate) use pickup::move_towards;
pub(crate) use learn::LearnSiteEnvironmentActive;
pub(crate) use arrival::HarvestViewNodes;

/// `UserMysekaiSiteHarvestFixtureStatus.harvested`.
pub(crate) const STATUS_HARVESTED: i32 = server_mock::FIXTURE_HARVESTED;

/// ResourceType values of the drop families (the drop chain's switch keys).
pub(crate) const RT_MATERIAL: i32 = 2;
pub(crate) const RT_MYSEKAI_FIXTURE: i32 = 39;
pub(crate) const RT_MYSEKAI_BLUEPRINT: i32 = 40;
pub(crate) const RT_MYSEKAI_MATERIAL: i32 = 41;
pub(crate) const RT_MYSEKAI_ITEM: i32 = 42;
pub(crate) const RT_MYSEKAI_TOOL: i32 = 43;
pub(crate) const RT_MYSEKAI_MUSIC_RECORD: i32 = 44;

/// The fixture family's drop model is constant (GetDropItemPrefab's 39 arm
/// only checks that the fixture exists); blueprints and items share one
/// model, records another.
const GLASSBALL_DROP_PACKAGE: &str =
    "mysekai__site__field__object__mdl_site_glassball_common_glassballdrop01";
const BLUEPRINT_DROP_PACKAGE: &str =
    "mysekai__site__field__object__mdl_site_blueprint_common_blueprintdrop01";
const RECORD_DROP_PACKAGE: &str =
    "mysekai__site__field__object__mdl_site_record_common_recorddrop01";

/// The closed set of view classes (`view[].class`).
pub(crate) const VIEW_CLASSES: [&str; 9] = [
    "MysekaiAreaStoneView",
    "MysekaiAreaTreeView",
    "MysekaiAreaPlantView",
    "MysekaiAreaJunkView",
    "MysekaiAreaToneView",
    "MysekaiAreaToolBoxView",
    "MysekaiAreaTreasureBoxView",
    "MysekaiAreadDriftageView",
    "MysekaiBirthdayPlantView",
];

/// The two arms of `OnDamage` (the interface the view class implements).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionInterface {
    /// `IMultiActionObject`: stone and tree.
    Multi,
    /// `ISingleActionObject`: the other seven classes.
    Single,
}

pub(crate) fn interface_of(view_class: &str) -> Option<ActionInterface> {
    match view_class {
        "MysekaiAreaStoneView" | "MysekaiAreaTreeView" => Some(ActionInterface::Multi),
        "MysekaiAreadDriftageView"
        | "MysekaiAreaJunkView"
        | "MysekaiAreaPlantView"
        | "MysekaiAreaToneView"
        | "MysekaiAreaToolBoxView"
        | "MysekaiAreaTreasureBoxView"
        | "MysekaiBirthdayPlantView" => Some(ActionInterface::Single),
        _ => None,
    }
}

/// `HarvestBaseView.SetupScaleRandom` has two callers: the stone and the tree
/// views.
pub(crate) fn scales_randomly(view_class: &str) -> bool {
    matches!(view_class, "MysekaiAreaStoneView" | "MysekaiAreaTreeView")
}

/// The SEs of one view class, by the view methods that play them.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct KindCues {
    /// `OnPlayerActionStart` (the swing start).
    pub(crate) swing_start: Option<&'static str>,
    /// `PlayHitSE` (multi) or `PlaySE` (single) at each hit.
    pub(crate) hit: Option<&'static str>,
    /// `PlayLastAttackSE` (multi, the last attack).
    pub(crate) last: Option<&'static str>,
    /// `PlayRareObjectBreakSE` (multi, rare, the last attack).
    pub(crate) rare_break: Option<&'static str>,
}

pub(crate) fn kind_cues(view_class: &str, is_rare: bool) -> KindCues {
    let rare = |cue| if is_rare { Some(cue) } else { None };
    match view_class {
        "MysekaiAreaTreeView" => KindCues {
            hit: Some("se_axe1"),
            last: Some("se_fallen_tree"),
            rare_break: rare("se_break_rare"),
            ..default()
        },
        "MysekaiAreaStoneView" => KindCues {
            hit: Some("se_pickaxe1"),
            last: Some("se_break_rock"),
            rare_break: rare("se_break_rare"),
            ..default()
        },
        "MysekaiAreaPlantView" => KindCues {
            hit: Some(if is_rare {
                "se_pick_plant_rare"
            } else {
                "se_pick_plant"
            }),
            ..default()
        },
        "MysekaiAreaJunkView" => KindCues {
            swing_start: Some("se_rustle"),
            hit: Some("se_pick_item"),
            ..default()
        },
        "MysekaiAreadDriftageView" => KindCues {
            swing_start: Some("se_broken_barrel"),
            hit: Some("se_pick_item"),
            ..default()
        },
        "MysekaiAreaToolBoxView" => KindCues {
            swing_start: Some("se_tresure_open"),
            ..default()
        },
        "MysekaiAreaTreasureBoxView" => KindCues {
            swing_start: Some("se_spawn_tresure_open"),
            ..default()
        },
        "MysekaiBirthdayPlantView" => KindCues {
            swing_start: Some("se_pick_birthday_plant"),
            ..default()
        },
        // Tone plays "se_" + its bundle name as an environment SE (later lane).
        _ => KindCues::default(),
    }
}

pub(crate) fn kind_word(kind: i32) -> &'static str {
    match kind {
        0 => "wood",
        1 => "mineral",
        2 => "plant",
        3 => "treasure_box_transport",
        4 => "treasure_box_fixed",
        5 => "other",
        6 => "tone",
        7 => "toolbox",
        8 => "driftage",
        9 => "birthday_plant",
        _ => "unknown",
    }
}

/// One placed harvest object's root.
#[derive(Component)]
pub struct HarvestRoot;

/// The model of one placed object (`HarvestObjectModel` and the view fields
/// the flow reads).
#[derive(Component)]
pub struct HarvestObject {
    pub(crate) uid: u64,
    pub(crate) site_id: u32,
    pub package: String,
    pub leaf: String,
    pub(crate) class: &'static str,
    /// The view's serialized fixture type.
    pub fixture_type: i32,
    pub fixture_id: i32,
    pub position_x: i32,
    pub position_z: i32,
    pub hp: i32,
    pub prev_hp: i32,
    pub status: i32,
    pub is_last_attack: bool,
    pub last_attack_stamina: i32,
    pub is_rare: bool,
    /// View radius times the random scale.
    pub radius: f32,
    pub(crate) scale: f32,
    /// Registered with the collision manager (removed by
    /// `RemoveCollisionObject` and for loaded-harvested rows).
    pub(crate) collision: bool,
    pub collision_type: i32,
    pub interface: ActionInterface,
    pub(crate) cues: KindCues,
    pub(crate) assetbundle: String,
    pub(crate) pending_drops: Vec<PendingDrop>,
    pub(crate) hits_taken: usize,
}

impl HarvestObject {
    /// `HarvestObjectModel.UpdateHp`, body for body (see `law::HpModel`,
    /// which carries the same rule for the value checks).
    pub(crate) fn update_hp(&mut self, damage: i32) -> i32 {
        let mut model = law::HpModel {
            hp: self.hp,
            prev_hp: self.prev_hp,
            harvested: self.status == STATUS_HARVESTED,
            is_last_attack: self.is_last_attack,
            last_attack_stamina: self.last_attack_stamina,
        };
        let returned = model.update_hp(damage);
        self.hp = model.hp;
        self.prev_hp = model.prev_hp;
        self.is_last_attack = model.is_last_attack;
        if model.harvested {
            self.status = STATUS_HARVESTED;
        }
        returned
    }
}

/// A drop row as the object model holds it (resolved prefab and rarity).
#[derive(Clone, Debug)]
pub(crate) struct PendingDrop {
    pub(crate) row: UserDrop,
    pub(crate) prefab: DropPrefab,
    /// `GetDropRarityType`.
    pub(crate) rarity: i32,
    /// The material's type (the scatter arms); -1 outside materials.
    pub(crate) material_type: i32,
}

#[derive(Clone, Debug)]
pub(crate) enum DropPrefab {
    Model { package: String },
    Unresolved(&'static str),
}

/// `GetDropRarityType`: the mask {2, 39, 40, 42, 44} (the word
/// 0x158000000004 tested in the binary) gives 2; a mysekai material its own
/// rarity; anything else logs and gives 0.
pub(crate) fn drop_rarity_type(resource_type: i32, material_rarity: Option<i32>) -> i32 {
    const MASK: u64 = 0x1580_0000_0004;
    if (0..64).contains(&resource_type) && (1u64 << resource_type) & MASK != 0 {
        return 2;
    }
    match resource_type {
        RT_MYSEKAI_MATERIAL => material_rarity.unwrap_or_else(|| {
            warn!("[harvest-drop] a material drop names no material row; rarity 0");
            0
        }),
        _ => 0,
    }
}

/// `GetDropMinRange / GetDropMaxRange / GetDropHeight`: (min, max, height).
pub(crate) fn drop_scatter_params(resource_type: i32, material_type: i32) -> (f32, f32, f32) {
    match resource_type {
        RT_MATERIAL
        | RT_MYSEKAI_FIXTURE
        | RT_MYSEKAI_BLUEPRINT
        | RT_MYSEKAI_ITEM
        | RT_MYSEKAI_MUSIC_RECORD => (0.6, 1.0, 0.0),
        RT_MYSEKAI_MATERIAL => match material_type {
            0 | 1 => (0.7, 1.2, 0.2),
            6 => (0.0, 0.0, 1.0),
            _ => (0.6, 1.0, 0.0),
        },
        _ => (0.6, 1.0, 0.0),
    }
}

/// `GetDropItemPrefab` for one row.
pub(crate) fn drop_model_package(catalog: &HarvestCatalog, drop: &UserDrop) -> Option<String> {
    match drop_prefab(catalog, drop) {
        DropPrefab::Model { package } => Some(package),
        DropPrefab::Unresolved(_) => None,
    }
}

fn constant_prefab(ids: &std::collections::HashSet<i64>, id: i64, package: &str) -> DropPrefab {
    if ids.contains(&id) {
        DropPrefab::Model {
            package: package.to_owned(),
        }
    } else {
        DropPrefab::Unresolved(
            "the drop resource is not in its master (GetDropItemPrefab returns null)",
        )
    }
}

fn drop_prefab(catalog: &HarvestCatalog, drop: &UserDrop) -> DropPrefab {
    match drop.resource_type {
        RT_MYSEKAI_FIXTURE => constant_prefab(
            &catalog.fixture_ids,
            drop.resource_id,
            GLASSBALL_DROP_PACKAGE,
        ),
        RT_MYSEKAI_BLUEPRINT => constant_prefab(
            &catalog.blueprint_ids,
            drop.resource_id,
            BLUEPRINT_DROP_PACKAGE,
        ),
        RT_MYSEKAI_ITEM => {
            constant_prefab(&catalog.item_ids, drop.resource_id, BLUEPRINT_DROP_PACKAGE)
        }
        RT_MYSEKAI_MUSIC_RECORD => constant_prefab(
            &catalog.music_record_ids,
            drop.resource_id,
            RECORD_DROP_PACKAGE,
        ),
        RT_MYSEKAI_MATERIAL => match catalog.materials.get(&drop.resource_id) {
            Some(material) => DropPrefab::Model {
                package: material.package.clone(),
            },
            None => DropPrefab::Unresolved("the material has no drop model row"),
        },
        RT_MYSEKAI_TOOL => {
            DropPrefab::Unresolved("tool family: the source refuses it (LogError, null)")
        }
        RT_MATERIAL => DropPrefab::Unresolved(
            "plain material family: its model split is not supplied to this consumer",
        ),
        _ => DropPrefab::Unresolved("resource type outside the drop families"),
    }
}

pub(crate) fn pending_drop(catalog: &HarvestCatalog, drop: &UserDrop) -> PendingDrop {
    let material = (drop.resource_type == RT_MYSEKAI_MATERIAL)
        .then(|| catalog.materials.get(&drop.resource_id))
        .flatten();
    PendingDrop {
        row: drop.clone(),
        prefab: drop_prefab(catalog, drop),
        rarity: drop_rarity_type(drop.resource_type, material.map(|m| m.rarity)),
        material_type: material.map_or(-1, |m| m.material_type),
    }
}

/// One drop batch (`CreateDropItem`), consumed by `drops::spawn`.
pub(crate) struct DropBatch {
    /// The fixture entity whose pending list loses each spawned row; none for
    /// the unclaimed drops of an arrival.
    pub(crate) origin: Option<Entity>,
    pub(crate) position_x: i32,
    pub(crate) position_z: i32,
    pub(crate) fixture_type: i32,
    pub(crate) remaining: Vec<PendingDrop>,
    /// The batch size reached `HarvestDropDelayItemCount`: one item a frame.
    pub(crate) delay: bool,
}

#[derive(Resource, Default)]
pub(crate) struct HarvestDropBatches(pub(crate) Vec<DropBatch>);

/// A drop on the ground (`MysekaiAreaDropItemView` and its model).
#[derive(Component)]
pub struct HarvestDropItem {
    pub uid: u64,
    /// 0 while scattering; 1.0 once landed.
    pub radius: f32,
    pub rarity: i32,
    pub resource_type: i32,
    pub resource_id: i64,
    pub(crate) site_id: u32,
    pub(crate) row: UserDrop,
    pub(crate) material_type: i32,
    /// `_acceleration` of the approach.
    pub(crate) acceleration: f32,
    /// `_isAttractToPlayer` (constructor default true).
    pub(crate) attracting: bool,
}

/// One hit request (the shape of `OnDamage`'s arguments).
#[derive(Debug, Clone, Copy)]
pub struct HarvestHit {
    pub target: Entity,
    pub damage: i32,
    pub tool_level: i32,
    pub is_boost: bool,
    /// The tool the swing used (`None`: bare hands).
    pub tool: Option<i64>,
}

/// Hits the action loop raises this frame; `damage::on_damage` drains them.
#[derive(Resource, Default)]
pub struct HarvestHits(pub Vec<HarvestHit>);

/// What `OnDamage` returned per hit (the action loop's bookkeeping reads it).
#[derive(Debug, Clone, Copy)]
pub(crate) struct HitResult {
    pub(crate) target: Entity,
    pub(crate) damage: i32,
    pub(crate) used: i32,
    pub(crate) is_last_attack: bool,
    pub(crate) tool: Option<i64>,
}

#[derive(Resource, Default)]
pub(crate) struct HarvestHitResults(pub(crate) Vec<HitResult>);

/// An `EffectManager.Emit` the flow asks for; `effects` draws it.
#[derive(Debug, Clone, Copy)]
pub(crate) struct EffectHook {
    pub(crate) kind: u16,
    pub(crate) position: Vec3,
    pub(crate) rotation: Quat,
}

impl EffectHook {
    /// `Emit(type, position, key)`: the prefab's own rotation.
    pub(crate) fn at(kind: u16, position: Vec3) -> Self {
        Self {
            kind,
            position,
            rotation: Quat::IDENTITY,
        }
    }
}

#[derive(Resource, Default)]
pub(crate) struct HarvestEffectHooks {
    pub(crate) pending: Vec<EffectHook>,
    pub(crate) total: usize,
}

/// Counters for the periodic status line (control flow only).
#[derive(Resource, Default)]
pub struct HarvestStats {
    pub hits: usize,
    pub multi_hits: usize,
    pub multi_final: usize,
    pub single_hits: usize,
    pub idle_returns: usize,
    pub drop_batches: usize,
    pub drop_items: usize,
    pub drop_refused: usize,
    pub collected: usize,
}

#[derive(Resource, Default)]
pub(crate) struct HarvestGltfs {
    pub(crate) by_key: HashMap<String, Handle<Gltf>>,
}

/// Package documents (scene dispatch here, materials in `harvest_material`).
#[derive(Resource, Default)]
pub(crate) struct HarvestDocs(pub(crate) HashMap<String, Handle<moly_assets::json::JsonAsset>>);

/// Every placed scene of the arrival has expanded (the material swap and the
/// stay particles wait for it).
#[derive(Resource)]
pub struct HarvestScenesReady;

#[derive(Resource, Default)]
pub(crate) struct HarvestSpawnedCount(pub(crate) usize);

#[derive(Resource, Default)]
struct HarvestScenesReadyCount(usize);

/// Ground vertices of the current site (heights of placements and drops).
#[derive(Resource, Default)]
pub(crate) struct HarvestGroundVerts(pub(crate) Option<Vec<Vec3>>);

/// Drop uid counter.
#[derive(Resource, Default)]
pub(crate) struct HarvestDropSeq(pub(crate) u64);

/// A seeded generator standing in for Unity's `Random` (splitmix64).
pub(crate) struct Rng(pub(crate) u64);

impl Rng {
    pub(crate) fn next_f32(&mut self) -> f32 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^= z >> 31;
        (z >> 40) as f32 / 16_777_216.0
    }
}

pub(crate) fn ground_world_verts(
    ground: &GroundMeshes,
    meshes: &Assets<Mesh>,
    parts: &Query<(&Mesh3d, &GlobalTransform)>,
) -> Vec<Vec3> {
    let mut verts = Vec::new();
    for (mesh3d, global) in parts {
        if !ground.0.contains(&mesh3d.0) {
            continue;
        }
        let Some(mesh) = meshes.get(&mesh3d.0) else {
            continue;
        };
        let Some(positions) = mesh
            .attribute(Mesh::ATTRIBUTE_POSITION)
            .and_then(|values| values.as_float3())
        else {
            continue;
        };
        verts.extend(
            positions
                .iter()
                .map(|p| global.transform_point(Vec3::from(*p))),
        );
    }
    verts
}

/// Highest ground vertex within 2 m, or `fallback` without one (a stand-in
/// for the source's downward raycast on the ground collider).
pub(crate) fn surface_y(verts: &[Vec3], x: f32, z: f32, fallback: f32) -> f32 {
    const RADIUS: f32 = 2.0;
    verts
        .iter()
        .filter(|v| {
            let dx = v.x - x;
            let dz = v.z - z;
            dx * dx + dz * dz <= RADIUS * RADIUS
        })
        .map(|v| v.y)
        .fold(fallback, f32::max)
}

/// The scene of the document's one `.prefab` root (the export writes the FBX
/// and the prefab as two scenes of the same name; the default scene is not
/// always the prefab).
pub(crate) fn prefab_scene_index(document: &str, leaf: &str) -> usize {
    let value: serde_json::Value = serde_json::from_str(document)
        .unwrap_or_else(|err| panic!("harvest document is not JSON ({leaf}): {err}"));
    let roots = value["roots"]
        .as_array()
        .unwrap_or_else(|| panic!("harvest document has no roots ({leaf})"));
    let scenes: Vec<usize> = roots
        .iter()
        .filter(|root| {
            root["assets"].as_array().is_some_and(|assets| {
                assets
                    .iter()
                    .any(|a| a.as_str().is_some_and(|s| s.ends_with(".prefab")))
            })
        })
        .map(|root| {
            root["scene"]
                .as_u64()
                .unwrap_or_else(|| panic!("prefab root without scene ({leaf})"))
                as usize
        })
        .collect();
    match scenes.as_slice() {
        [scene] => *scene,
        [] => panic!("harvest document has no prefab root ({leaf})"),
        many => panic!("harvest document has {} prefab roots ({leaf})", many.len()),
    }
}

/// Global observer: count the arrival's expanded scenes; all expanded opens
/// the material swap and the stay particles.
fn on_scene_ready(
    trigger: On<SceneInstanceReady>,
    roots: Query<&HarvestRoot>,
    arrival: Res<arrival::HarvestArrival>,
    spawned: Res<HarvestSpawnedCount>,
    mut count: ResMut<HarvestScenesReadyCount>,
    mut commands: Commands,
) {
    if roots.get(trigger.event().entity).is_err() {
        return;
    }
    count.0 += 1;
    if !arrival.in_progress() && count.0 == spawned.0 {
        info!(
            "[harvest] every placed scene expanded: {}/{}",
            count.0, spawned.0
        );
        commands.insert_resource(HarvestScenesReady);
    }
}

/// Queued by the site transition: the harvest objects, drops, the action,
/// the target and the button go with the site; the next arrival rebuilds.
pub(crate) fn clear_for_site_change(world: &mut World) {
    let roots: Vec<Entity> = world
        .query_filtered::<Entity, Or<(
            With<HarvestRoot>,
            With<HarvestDropItem>,
            With<effects::HarvestEffectRoot>,
        )>>()
        .iter(world)
        .collect();
    let count = roots.len();
    for entity in roots {
        if let Ok(entity) = world.get_entity_mut(entity) {
            entity.despawn();
        }
    }
    action::cancel_for_site_change(world);
    effects::clear_for_site_change(world);
    world.remove_resource::<HarvestScenesReady>();
    world.remove_resource::<crate::harvest_material::HarvestMaterialsSwapped>();
    world.resource_mut::<HarvestScenesReadyCount>().0 = 0;
    world.resource_mut::<HarvestSpawnedCount>().0 = 0;
    world.resource_mut::<HarvestDropBatches>().0.clear();
    world.resource_mut::<damage::HarvestStartHides>().0.clear();
    world.resource_mut::<damage::HarvestEffectOnly>().0.clear();
    world.resource_mut::<damage::HarvestTurnRequests>().0.clear();
    world
        .resource_mut::<prop_animator::PropAnimatorCalls>()
        .0
        .clear();
    world.resource_mut::<HarvestGroundVerts>().0 = None;
    world.resource_mut::<arrival::HarvestArrival>().clear();
    if count > 0 {
        info!("[harvest] site change: {count} harvest objects and drops removed with the site");
    }
}

/// Periodic status line (counters prove control flow only).
fn report(
    stats: Res<HarvestStats>,
    objects: Query<&HarvestObject>,
    drops: Query<&HarvestDropItem>,
    effects: Res<HarvestEffectHooks>,
    model: Option<Res<action::HarvestPlayerModel>>,
) {
    let total = objects.iter().count();
    if total == 0 && stats.hits == 0 {
        return;
    }
    let harvested = objects
        .iter()
        .filter(|object| object.status == STATUS_HARVESTED)
        .count();
    info!(
        "[harvest] objects {total} (harvested {harvested}) · hits {} (multi {} / last {} / single {} / idle {}) · drop batches {} items {} refused {} on ground {} collected {} · effect hooks {} · stamina {:?}",
        stats.hits,
        stats.multi_hits,
        stats.multi_final,
        stats.single_hits,
        stats.idle_returns,
        stats.drop_batches,
        stats.drop_items,
        stats.drop_refused,
        drops.iter().count(),
        stats.collected,
        effects.total,
        model.as_ref().map(|model| model.stamina),
    );
}

/// Harvest plugin. Order inside a frame: inputs and catalog, arrival, the
/// button and the action loop (it raises hits), `OnDamage`, the hit
/// bookkeeping, drops, tweens, pickup, the log queue.
pub struct HarvestPlugin;

/// The action loop's system set (the schedule orders player-state writers and
/// the input chain around it).
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct HarvestActionSet;

impl Plugin for HarvestPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<HarvestHits>()
            .init_resource::<HarvestHitResults>()
            .init_resource::<HarvestStats>()
            .init_resource::<HarvestSpawnedCount>()
            .init_resource::<HarvestScenesReadyCount>()
            .init_resource::<HarvestDropBatches>()
            .init_resource::<HarvestDropSeq>()
            .init_resource::<HarvestGroundVerts>()
            .init_resource::<HarvestGltfs>()
            .init_resource::<HarvestDocs>()
            .init_resource::<HarvestEffectHooks>()
            .init_resource::<arrival::HarvestArrival>()
            .init_resource::<action::HarvestAction>()
            .init_resource::<action::HarvestTargeting>()
            .init_resource::<action::HarvestCameraShakes>()
            .init_resource::<action::HarvestAutoplay>()
            .init_resource::<ui::HarvestButton>()
            .init_resource::<tool_model::HarvestToolModels>()
            .init_resource::<tool_model::ToolModelRequests>()
            .init_resource::<queue::HarvestLogQueue>()
            .init_resource::<effects::HarvestEffects>()
            .init_resource::<prop_animator::PropAnimatorCalls>()
            .init_resource::<damage::HarvestStartHides>()
            .init_resource::<damage::HarvestEffectOnly>()
            .init_resource::<damage::HarvestTurnRequests>()
            .init_resource::<airplane::PaperAirplanes>()
            .init_resource::<tone::HarvestToneCamera>()
            .init_resource::<learn::LearnEnvironment>()
            .add_systems(
                Startup,
                (catalog::load, clips::load, tool_model::load, effects::load),
            )
            .add_observer(on_scene_ready)
            .add_systems(
                Update,
                (
                    catalog::build,
                    clips::parse,
                    tool_model::parse,
                    arrival::place,
                    arrival::bind_views,
                    prop_animator::bind,
                    airplane::advance,
                    learn::advance,
                )
                    .chain(),
            )
            .add_systems(
                Update,
                (
                    ui::spawn_when_ready,
                    ui::read_input,
                    action::autoplay_press,
                    action::update_targets,
                    (action::advance, action::hold_for_auto_move).chain(),
                    damage::on_effect_only,
                    damage::on_damage,
                    action::after_hits,
                    drops::spawn,
                    damage::advance_punches,
                    damage::advance_after_forms,
                    damage::advance_start_hides,
                    damage::advance_turns,
                    prop_animator::advance,
                    drops::advance_animations,
                    pickup::collect_on_leave,
                    pickup::advance,
                    tool_model::apply,
                    ui::place,
                    queue::advance,
                )
                    .chain()
                    .in_set(HarvestActionSet)
                    .after(arrival::bind_views),
            )
            .add_systems(
                Update,
                (
                    report.run_if(bevy::time::common_conditions::on_timer(
                        std::time::Duration::from_secs(2),
                    )),
                    effects::advance.after(HarvestActionSet),
                ),
            )
            .add_systems(
                PostUpdate,
                (
                    action::advance_camera_shake.before(crate::camera::follow_avatar),
                    tone::advance_tone_camera.after(crate::camera::follow_avatar),
                    learn::advance_learn_camera.after(crate::camera::follow_avatar),
                ),
            );
    }
}
