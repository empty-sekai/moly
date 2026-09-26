//! Character silhouette: the player avatar's stencil-shadow pass.
//!
//! # The source rule
//!
//! The avatar programs (body and tool) carry a stencil-shadow pass that the
//! renderer's stencil-shadow feature draws after the transparents, for the
//! opaque queue range and every layer, with no depth or stencil override. Its
//! state: stencil reference 1, comparison Equal, pass and depth-fail
//! DecrementSaturate, fail Keep; depth test Greater with depth write; cull
//! Back; blend SrcAlpha / OneMinusSrcAlpha for colour and alpha. Its program
//! outputs one global colour, the graphics configuration's stencil-shadow
//! colour (read here from `graphics-config/stencil-shadow.json`, never typed
//! in). The NPC programs have no such pass.
//!
//! So the silhouette appears exactly where the stencil holds 1 when the pass
//! runs and the avatar lies behind what the depth buffer holds. The value 1
//! is not the character-silhouette stencil id. Field objects write stencil 2
//! (their settings row names the character-silhouette id, which the stencil
//! id table maps to 2); the only writer of 1 is the door-wall clip, the
//! colourless stencil mesh of a room door (its row names the door-wall-clip
//! id, which the table maps to 1). Outdoors no draw writes 1, so outdoors
//! the pass draws nothing, behind a house or a field object alike.
//!
//! # How the product reproduces it
//!
//! The product's main pass has no stencil. This module replays the depth and
//! stencil of the stencil-relevant draws, in queue order, into a
//! depth-stencil target of its own, each with its source state: the row of
//! its shader attribute in the graphics configuration (queue, stencil id
//! through the table, comparison, operations, depth test and depth write),
//! or, for the character programs, the pass state they declare. Then, at the
//! point the source feature runs (after the transparents), it draws the
//! avatar's meshes with the stencil test Equal 1 and the pass's operations
//! against that stencil, the depth test Greater against the camera's own
//! depth, and the pass's blend computed in the encoded values of the
//! source's gamma colour space. The replay runs only while a room door's
//! stencil mesh is shown; with no writer of 1 the stencil holds no 1, and
//! the pass is known to draw nothing.
//!
//! Draws replayed: the room door's parts (by the attribute the door binds on
//! them, [`SourceStencilAttribute`]), the room shell (floor, walls, entrance),
//! the fixtures with the fixture material, the player avatar and the NPCs.
//!
//! # Cases this cannot match (named)
//!
//! - Draws of other programs are not replayed and are counted in the account
//!   line (with the names of their nodes, once per room visit): canvas,
//!   transparent-block and tree fixtures, window stencil masks, the room's
//!   plain background (its stencil test is Equal 0 with Keep and never
//!   changes a 1), effects and particles. A window mask in front of a door
//!   would turn door pixels from 1 to 2.
//! - Alpha-clipped and dithered fragments are replayed whole: the source
//!   discards them before its stencil write. An NPC whose dither alpha is 0
//!   with its dither gate on discards every fragment and is left out.
//! - The NPC stencil reference is the character's own id (4 i + 4); the
//!   replay writes 4 for all of them. Every such id is at least 4, never 1.
//! - The source's silhouette writes the avatar's depth into the camera depth;
//!   here the camera depth is only read.
//! - Face morphs are not applied to the replayed meshes.
//! - Opaque draws of one queue are ordered front to back by the distance of
//!   their origins, transparent ones back to front; the source sorts by its
//!   own criteria within a queue.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::marker::PhantomData;
use std::num::NonZeroU64;
use std::sync::Mutex;

use bevy::asset::uuid::Uuid;
use bevy::core_pipeline::core_3d::graph::{Core3d, Node3d};
use bevy::diagnostic::FrameCount;
use bevy::ecs::query::QueryItem;
use bevy::mesh::skinning::{SkinnedMesh, SkinnedMeshInverseBindposes};
use bevy::mesh::{Mesh, MeshVertexBufferLayoutRef};
use bevy::prelude::*;
use bevy::render::camera::ExtractedCamera;
use bevy::render::mesh::allocator::MeshAllocator;
use bevy::render::mesh::{RenderMesh, RenderMeshBufferInfo};
use bevy::render::render_asset::RenderAssets;
use bevy::render::render_graph::{
    NodeRunError, RenderGraphContext, RenderGraphExt, RenderLabel, RenderSubGraph, ViewNode,
    ViewNodeRunner,
};
use bevy::render::render_resource::binding_types::uniform_buffer_sized;
use bevy::render::render_resource::*;
use bevy::render::renderer::{RenderContext, RenderDevice, RenderQueue};
use bevy::render::texture::{CachedTexture, TextureCache};
use bevy::render::view::{ExtractedView, ViewDepthTexture, ViewTarget};
use bevy::render::{Extract, ExtractSchedule, Render, RenderApp, RenderStartup, RenderSystems};
use moly_assets::json::JsonAsset;
use moly_assets::scene_state::SourceInactive;
use serde_json::Value;

use crate::avatar_material::AvatarMaterial;
use crate::character_material::CharacterMaterial;
use crate::fixture_material::FixtureMaterial;
use crate::render::gpu::{Bound, SharedBindGroupCache};
use crate::room_shell::{RoomShellKey, RoomShellMaterial, ShellProgram};
use crate::shadowmap::{joint_palette, push_palette, NoShadowCast, PALETTE_WINDOW};

const SILHOUETTE_SHADER: Handle<Shader> = Handle::Uuid(
    Uuid::from_u128(0x6368_6172_5f73_696c_686f_7565_7474_6501),
    PhantomData,
);

/// The graphics configuration's stencil-shadow document (colour and
/// per-attribute material rendering settings).
const CONFIG_PATH: &str = "moly://graphics-config/stencil-shadow.json";

const DEPTH_STENCIL_FORMAT: TextureFormat = TextureFormat::Depth24PlusStencil8;

/// One slot of the view and world-matrix pools: the dynamic-offset alignment
/// every device supports.
const SLOT: usize = 256;
const MATRIX_BYTES: u64 = 64;

/// The build's stencil id table, indexed by a settings row's stencil id
/// reference: Zero, CharacterSilhouette, DoorWallClip, GroundClip,
/// WindowWallClip, Wall.
const STENCIL_ID_TABLE: [u32; 6] = [0, 2, 1, 3, 2, 0];

/// Shader attribute values (the settings rows' `shaderAttribute`).
mod attribute {
    pub const GROUND_OPAQUE: u8 = 1;
    pub const WALL: u8 = 3;
    pub const DOOR_WALL_CLIP: u8 = 4;
    pub const FIXTURE_OBJECT_OPAQUE: u8 = 5;
    pub const CHARACTER: u8 = 13;
    pub const DOOR_OUTSIDE: u8 = 14;
    /// The ground lookup's result for its mode property 1.
    pub const GROUND_OTHER: u8 = 16;
    pub const RUG_OPAQUE: u8 = 28;
    pub const RUG_ALPHA_BLENDED: u8 = 29;
}

/// Opaque queues end here; above it a queue is transparent.
const OPAQUE_QUEUE_END: u32 = 2500;

/// A draw's source stencil state.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct StencilRule {
    reference: u32,
    compare: CompareFunction,
    pass: StencilOperation,
    fail: StencilOperation,
    depth_fail: StencilOperation,
}

/// A replayed draw's depth and stencil state, in the product's reversed depth.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct ReplayState {
    /// `None`: the stencil test is disabled (comparison 0).
    stencil: Option<StencilRule>,
    depth_compare: CompareFunction,
    depth_write: bool,
}

const fn always(reference: u32, pass: StencilOperation) -> Option<StencilRule> {
    Some(StencilRule {
        reference,
        compare: CompareFunction::Always,
        pass,
        fail: StencilOperation::Keep,
        depth_fail: StencilOperation::Keep,
    })
}

/// The avatar programs' colour pass: reference 0, Always, Replace; depth
/// LessEqual with depth write (reversed here).
const AVATAR_PASS: ReplayState = ReplayState {
    stencil: always(0, StencilOperation::Replace),
    depth_compare: CompareFunction::GreaterEqual,
    depth_write: true,
};
/// The NPC program's colour pass: the character's stencil id, Always,
/// Replace. The id is 4 i + 4 for character index i; 4 stands for all.
const NPC_PASS: ReplayState = ReplayState {
    stencil: always(4, StencilOperation::Replace),
    ..AVATAR_PASS
};
/// The NPC accessory program's colour pass: reference 0, Always, Zero.
const NPC_ACCESSORY_PASS: ReplayState = ReplayState {
    stencil: always(0, StencilOperation::Zero),
    ..AVATAR_PASS
};
/// The avatar programs' stencil-shadow pass: reference 1, Equal; pass and
/// depth fail DecrementSaturate, fail Keep. Its depth test (Greater) runs in
/// the fragment against the camera depth, so the pipeline's depth test
/// always passes and the depth-fail operation is never reached.
const SILHOUETTE_REFERENCE: u32 = 1;
const SILHOUETTE_STENCIL: StencilFaceState = StencilFaceState {
    compare: CompareFunction::Equal,
    fail_op: StencilOperation::Keep,
    depth_fail_op: StencilOperation::DecrementClamp,
    pass_op: StencilOperation::DecrementClamp,
};

/// Unity's CompareFunction as a stencil comparison (reference against the
/// buffer value, the order wgpu compares in).
fn stencil_compare(value: u64) -> Result<CompareFunction, String> {
    Ok(match value {
        1 => CompareFunction::Never,
        2 => CompareFunction::Less,
        3 => CompareFunction::Equal,
        4 => CompareFunction::LessEqual,
        5 => CompareFunction::Greater,
        6 => CompareFunction::NotEqual,
        7 => CompareFunction::GreaterEqual,
        8 => CompareFunction::Always,
        other => return Err(format!("stencil comparison {other}")),
    })
}

/// Unity's CompareFunction as a depth test in the product's reversed depth
/// (nearer is larger), so every order comparison turns around.
fn reversed_depth_compare(value: u64) -> Result<CompareFunction, String> {
    Ok(match value {
        1 => CompareFunction::Never,
        2 => CompareFunction::Greater,
        3 => CompareFunction::Equal,
        4 => CompareFunction::GreaterEqual,
        5 => CompareFunction::Less,
        6 => CompareFunction::NotEqual,
        7 => CompareFunction::LessEqual,
        8 => CompareFunction::Always,
        other => return Err(format!("depth test {other}")),
    })
}

/// Unity's StencilOp.
fn stencil_op(value: u64) -> Result<StencilOperation, String> {
    Ok(match value {
        0 => StencilOperation::Keep,
        1 => StencilOperation::Zero,
        2 => StencilOperation::Replace,
        3 => StencilOperation::IncrementClamp,
        4 => StencilOperation::DecrementClamp,
        5 => StencilOperation::Invert,
        6 => StencilOperation::IncrementWrap,
        7 => StencilOperation::DecrementWrap,
        other => return Err(format!("stencil operation {other}")),
    })
}

/// One attribute's settings row: its queue and the state the material
/// builder gives its material.
#[derive(Clone, Copy, Debug)]
struct AttributeRule {
    queue: u32,
    state: ReplayState,
}

/// The graphics configuration's stencil-shadow document, parsed.
#[derive(Resource, Clone, Debug)]
pub(crate) struct SilhouetteRules {
    colour: [f32; 4],
    attributes: HashMap<u8, AttributeRule>,
    /// Rows this replay cannot state, with the reason.
    refused: Vec<(u8, String)>,
}

fn parse_rules(text: &str) -> Result<SilhouetteRules, String> {
    let doc: Value = serde_json::from_str(text).map_err(|error| error.to_string())?;
    let rgba = doc["stencilShadowColor"]["rgba"]
        .as_array()
        .ok_or("no stencilShadowColor.rgba")?;
    let colour: Vec<f32> = rgba
        .iter()
        .map(|v| v.as_f64().map(|v| v as f32))
        .collect::<Option<_>>()
        .ok_or("a colour component is not a number")?;
    let colour: [f32; 4] = colour
        .try_into()
        .map_err(|_| "the colour does not have four components".to_owned())?;
    let rows = doc["materialRenderingSettings"]
        .as_array()
        .ok_or("no materialRenderingSettings")?;
    let mut attributes = HashMap::new();
    let mut refused = Vec::new();
    for row in rows {
        let attribute = row["shaderAttribute"]
            .as_u64()
            .and_then(|a| u8::try_from(a).ok())
            .ok_or("a row without a shader attribute")?;
        match parse_row(row) {
            Ok(rule) => {
                if attributes.insert(attribute, rule).is_some() {
                    return Err(format!("attribute {attribute} has two rows"));
                }
            }
            Err(reason) => refused.push((attribute, reason)),
        }
    }
    Ok(SilhouetteRules {
        colour,
        attributes,
        refused,
    })
}

fn parse_row(row: &Value) -> Result<AttributeRule, String> {
    let queue = row["renderQueue"].as_u64().ok_or("no render queue")? as u32;
    let passes = row["passes"].as_array().ok_or("no passes")?;
    let [pass] = passes.as_slice() else {
        return Err(format!("{} pass rows", passes.len()));
    };
    let int = |key: &str| pass[key].as_u64().ok_or_else(|| format!("no {key}"));
    let flag = |key: &str| pass[key].as_bool().ok_or_else(|| format!("no {key}"));
    // The builder writes the depth test and depth write only when their
    // switches are on; off, the material's own values stand, which the
    // replay does not carry.
    if !flag("isActiveZTest")? || !flag("isActiveZWrite")? {
        return Err("depth state left to the material".into());
    }
    let comparison = int("stencilComp")?;
    let stencil = if comparison == 0 {
        None
    } else {
        let id = int("stencilIdReference")? as usize;
        let reference = *STENCIL_ID_TABLE
            .get(id)
            .ok_or_else(|| format!("stencil id reference {id} is outside the table"))?;
        Some(StencilRule {
            reference,
            compare: stencil_compare(comparison)?,
            pass: stencil_op(int("stencilPassOp")?)?,
            fail: stencil_op(int("stencilFailOp")?)?,
            depth_fail: stencil_op(int("stencilZFailOp")?)?,
        })
    };
    Ok(AttributeRule {
        queue,
        state: ReplayState {
            stencil,
            depth_compare: reversed_depth_compare(int("zTest")?)?,
            depth_write: flag("zWrite")?,
        },
    })
}

/// The document's load; parsed once it arrives.
#[derive(Resource)]
struct RulesSource {
    handle: Handle<JsonAsset>,
    settled: bool,
}

fn load_rules(mut commands: Commands, server: Res<AssetServer>) {
    commands.insert_resource(RulesSource {
        handle: server.load(CONFIG_PATH),
        settled: false,
    });
}

/// Without the document there is no colour and no settings to replay: the
/// pass is refused, loudly, once.
fn settle_rules(
    mut commands: Commands,
    source: Option<ResMut<RulesSource>>,
    server: Res<AssetServer>,
    documents: Res<Assets<JsonAsset>>,
) {
    let Some(mut source) = source else {
        return;
    };
    if source.settled {
        return;
    }
    if server.load_state(&source.handle).is_failed() {
        source.settled = true;
        warn!("character silhouette refused: {CONFIG_PATH} did not load; the stencil-shadow pass draws nothing");
        return;
    }
    let Some(document) = documents.get(&source.handle) else {
        return;
    };
    source.settled = true;
    match parse_rules(&document.0) {
        Ok(rules) => {
            info!(
                "character silhouette: colour {:?} and {} attribute rows from the graphics configuration ({} rows not replayable: {:?})",
                rules.colour,
                rules.attributes.len(),
                rules.refused.len(),
                rules.refused
            );
            commands.insert_resource(rules);
        }
        Err(reason) => warn!(
            "character silhouette refused: {CONFIG_PATH} does not parse ({reason}); the stencil-shadow pass draws nothing"
        ),
    }
}

/// The source shader attribute of a draw whose program the product does not
/// run: the room door's parts, bound by the door from its sidecar materials.
#[derive(Component, Clone, Copy, Debug)]
pub(crate) struct SourceStencilAttribute(pub u8);

/// The object program's usage switch, its cells with a fixed attribute.
/// Usages 0 and 14 go to the ground lookup, 4 and 13 to the basic-fixture
/// lookup and 8 to the field-object lookup, each of which reads further
/// properties; those are not decided by the usage.
fn object_usage_attribute(usage: u32) -> Option<u8> {
    Some(match usage {
        1 => 2,
        2 => 3,
        3 => 4,
        5 => 7,
        6 => 8,
        7 => 9,
        9 => 12,
        10 => 14,
        11 => 18,
        12 => 17,
        15 => 22,
        _ => return None,
    })
}

/// The shader attribute of a door sidecar material: the object program by
/// its usage, the basic-fixture program by its usage and blend mode (a
/// property the material lacks reads 0, as the source lookup reads it).
/// Other programs (the switched-off shadow mesh) have none here.
pub(crate) fn door_part_attribute(material: &Value) -> Option<u8> {
    let floats = &material["floats"];
    let whole = |key: &str| -> Option<f64> {
        floats[key]
            .as_f64()
            .filter(|v| v.fract() == 0.0 && *v >= 0.0)
    };
    match material["shader"]["name"].as_str()? {
        "Mysekai/Object" => object_usage_attribute(whole("_ObjectShaderUsage")? as u32),
        "Mysekai/Fixture/Basic" => {
            let usage = whole("_FixtureShaderUsage").unwrap_or(0.0) as i32;
            let blend = whole("_FixtureObjectBlendMode").unwrap_or(0.0) as i32;
            moly_law::fixture::family::basic_fixture_shader_attribute(usage, blend)
                .ok()
                .and_then(|a| u8::try_from(a).ok())
        }
        _ => None,
    }
}

/// The room shell's attribute. Its programs are the floor program and the
/// object program with usage 2 (wall), 10 (door outside) or 14 (the ground
/// lookup). A material's queue is its attribute's queue plus its render
/// priority, which is 0 on every room module material, so the queue names
/// the attribute among these candidates; any other queue is not replayed.
fn shell_attribute(rules: &SilhouetteRules, key: &RoomShellKey) -> Option<u8> {
    let candidates: &[u8] = match key.program {
        ShellProgram::Floor => &[attribute::GROUND_OPAQUE],
        ShellProgram::Object => &[
            attribute::WALL,
            attribute::DOOR_OUTSIDE,
            attribute::GROUND_OPAQUE,
            attribute::GROUND_OTHER,
        ],
    };
    candidates.iter().copied().find(|a| {
        rules
            .attributes
            .get(a)
            .is_some_and(|rule| rule.queue == key.queue)
    })
}

/// A fixture material's attribute: the rug program by its blend, the fence
/// program's single attribute, and the basic program's two-way table.
fn fixture_attribute(material: &FixtureMaterial) -> Option<u8> {
    let key = &material.key;
    if let Some(blended) = key.rug {
        return Some(if blended {
            attribute::RUG_ALPHA_BLENDED
        } else {
            attribute::RUG_OPAQUE
        });
    }
    if key.fence {
        return Some(attribute::FIXTURE_OBJECT_OPAQUE);
    }
    moly_law::fixture::family::basic_fixture_shader_attribute(
        key.window_clip as i32,
        material.blend as i32,
    )
    .ok()
    .and_then(|a| u8::try_from(a).ok())
}

/// Where a draw's vertices go: one world matrix, or its skin's palette. The
/// value is the dynamic offset of its window.
#[derive(Clone, Copy, Debug)]
enum Space {
    Rigid(u32),
    Skinned(u32),
}

struct ReplayDraw {
    mesh: AssetId<Mesh>,
    space: Space,
    state: ReplayState,
    queue: u32,
    distance: f32,
    pipeline: CachedRenderPipelineId,
}

struct SilhouetteDraw {
    mesh: AssetId<Mesh>,
    space: Space,
    distance: f32,
    pipeline: CachedRenderPipelineId,
}

#[derive(Clone, Default, Debug)]
struct SilhouetteTally {
    door_masks: usize,
    /// Replayed draws by shader attribute (the character programs under
    /// the character attribute).
    by_attribute: BTreeMap<u8, usize>,
    avatar: usize,
    npc: usize,
    npc_accessory: usize,
    npc_discarded: usize,
    /// Door parts, shells or fixtures without a replayable attribute row.
    unstated: usize,
    /// Shown meshes of other programs, not replayed.
    not_replayed: usize,
    palettes_refused: usize,
}

/// This frame's replay: filled in the extract, pipelines chosen in prepare.
#[derive(Resource, Default)]
struct SilhouetteDrawList {
    active: bool,
    colour: [f32; 4],
    replay: Vec<ReplayDraw>,
    silhouette: Vec<SilhouetteDraw>,
    transform_bytes: Vec<u8>,
    palette_bytes: Vec<u8>,
    tally: SilhouetteTally,
}

fn push_matrix(bytes: &mut Vec<u8>, matrix: Mat4) -> u32 {
    let offset = bytes.len();
    for component in matrix.to_cols_array() {
        bytes.extend_from_slice(&component.to_le_bytes());
    }
    bytes.resize(offset + SLOT, 0);
    offset as u32
}

type PaletteKey = (AssetId<SkinnedMeshInverseBindposes>, Vec<Entity>);

/// A draw's space: its world matrix, or its skin's palette of this frame
/// (skins that share joints and bind poses share one palette).
fn draw_space(
    transform: &GlobalTransform,
    skin: Option<&SkinnedMesh>,
    joints: &Query<&GlobalTransform>,
    bindposes: &Assets<SkinnedMeshInverseBindposes>,
    palettes: &mut HashMap<PaletteKey, u32>,
    list: &mut SilhouetteDrawList,
) -> Option<Space> {
    let Some(skin) = skin else {
        let offset = push_matrix(&mut list.transform_bytes, Mat4::from(transform.affine()));
        return Some(Space::Rigid(offset));
    };
    let key = (skin.inverse_bindposes.id(), skin.joints.clone());
    if let Some(offset) = palettes.get(&key) {
        return Some(Space::Skinned(*offset));
    }
    match joint_palette(skin, joints, bindposes) {
        Ok(matrices) => {
            let offset = push_palette(&mut list.palette_bytes, &matrices);
            palettes.insert(key, offset);
            Some(Space::Skinned(offset))
        }
        Err(_) => {
            list.tally.palettes_refused += 1;
            None
        }
    }
}

/// The replay's input of this frame. Nothing is replayed while no room
/// door's stencil mesh is shown: without a writer of 1 the stencil-shadow
/// pass has no pixel to draw.
#[allow(clippy::type_complexity, clippy::too_many_arguments)]
fn extract_silhouette_draws(
    mut list: ResMut<SilhouetteDrawList>,
    rules: Extract<Option<Res<SilhouetteRules>>>,
    doors: Extract<
        Query<
            (
                Entity,
                &Mesh3d,
                &SourceStencilAttribute,
                &GlobalTransform,
                Option<&SkinnedMesh>,
                &InheritedVisibility,
                Option<&ChildOf>,
            ),
            Without<SourceInactive>,
        >,
    >,
    shells: Extract<
        Query<
            (
                Entity,
                &Mesh3d,
                &MeshMaterial3d<RoomShellMaterial>,
                &GlobalTransform,
                &InheritedVisibility,
            ),
            Without<SourceInactive>,
        >,
    >,
    fixtures: Extract<
        Query<
            (
                Entity,
                &Mesh3d,
                &MeshMaterial3d<FixtureMaterial>,
                &GlobalTransform,
                Option<&SkinnedMesh>,
                &InheritedVisibility,
            ),
            Without<SourceInactive>,
        >,
    >,
    characters: Extract<
        Query<
            (
                Entity,
                &Mesh3d,
                &GlobalTransform,
                Option<&SkinnedMesh>,
                Has<MeshMaterial3d<AvatarMaterial>>,
                Option<&MeshMaterial3d<CharacterMaterial>>,
                Has<NoShadowCast>,
                &InheritedVisibility,
            ),
            (
                Or<(
                    With<MeshMaterial3d<AvatarMaterial>>,
                    With<MeshMaterial3d<CharacterMaterial>>,
                )>,
                Without<SourceInactive>,
            ),
        >,
    >,
    others: Extract<Query<(Entity, &InheritedVisibility, Option<&ChildOf>), With<Mesh3d>>>,
    lookups: Extract<(
        Query<&GlobalTransform>,
        Query<&InheritedVisibility>,
        Query<&Name>,
        Query<&GlobalTransform, With<Camera3d>>,
    )>,
    assets: Extract<(
        Res<Assets<SkinnedMeshInverseBindposes>>,
        Res<Assets<RoomShellMaterial>>,
        Res<Assets<FixtureMaterial>>,
        Res<Assets<CharacterMaterial>>,
    )>,
    mut named: Local<bool>,
) {
    let list = &mut *list;
    list.active = false;
    list.replay.clear();
    list.silhouette.clear();
    list.transform_bytes.clear();
    list.palette_bytes.clear();
    list.tally = SilhouetteTally::default();
    let Some(rules) = rules.as_ref().map(|rules| &**rules) else {
        return;
    };
    let (joints, visibility, names, cameras) = &*lookups;
    let (bindposes, shell_materials, fixture_materials, character_materials) = &*assets;
    // A door part hidden for writing no colour is shown when its parent is.
    let part_shown = |shown: &InheritedVisibility, parent: Option<&ChildOf>| {
        shown.get()
            || parent.is_some_and(|parent| {
                visibility
                    .get(parent.parent())
                    .is_ok_and(|shown| shown.get())
            })
    };
    let door_masks = doors
        .iter()
        .filter(|(_, _, attribute, _, _, shown, parent)| {
            attribute.0 == attribute::DOOR_WALL_CLIP && part_shown(*shown, *parent)
        })
        .count();
    if door_masks == 0 {
        if *named {
            info!("character silhouette: replay off (no door stencil mesh shown)");
        }
        *named = false;
        return;
    }
    list.active = true;
    list.colour = rules.colour;
    list.tally.door_masks = door_masks;
    let eye = cameras
        .iter()
        .next()
        .map(|camera| camera.translation())
        .unwrap_or(Vec3::ZERO);
    let mut palettes: HashMap<PaletteKey, u32> = HashMap::new();
    let mut replayed: HashSet<Entity> = HashSet::new();

    // (entity, mesh, transform, skin, attribute for the tally, state, queue)
    let mut push = |list: &mut SilhouetteDrawList,
                    entity: Entity,
                    mesh: &Mesh3d,
                    transform: &GlobalTransform,
                    skin: Option<&SkinnedMesh>,
                    tally_attribute: u8,
                    state: ReplayState,
                    queue: u32|
     -> Option<Space> {
        let space = draw_space(transform, skin, joints, bindposes, &mut palettes, list)?;
        replayed.insert(entity);
        *list.tally.by_attribute.entry(tally_attribute).or_default() += 1;
        list.replay.push(ReplayDraw {
            mesh: mesh.0.id(),
            space,
            state,
            queue,
            distance: transform.translation().distance(eye),
            pipeline: CachedRenderPipelineId::INVALID,
        });
        Some(space)
    };

    for (entity, mesh, attribute, transform, skin, shown, parent) in doors.iter() {
        if !part_shown(shown, parent) {
            continue;
        }
        let Some(rule) = rules.attributes.get(&attribute.0) else {
            list.tally.unstated += 1;
            continue;
        };
        push(
            list,
            entity,
            mesh,
            transform,
            skin,
            attribute.0,
            rule.state,
            rule.queue,
        );
    }
    for (entity, mesh, material, transform, shown) in shells.iter() {
        if !shown.get() {
            continue;
        }
        let Some(material) = shell_materials.get(&material.0) else {
            continue;
        };
        let Some((attribute, rule)) = shell_attribute(rules, &material.key)
            .and_then(|a| rules.attributes.get(&a).map(|rule| (a, rule)))
        else {
            list.tally.unstated += 1;
            continue;
        };
        push(
            list,
            entity,
            mesh,
            transform,
            None,
            attribute,
            rule.state,
            material.key.queue,
        );
    }
    for (entity, mesh, material, transform, skin, shown) in fixtures.iter() {
        if !shown.get() {
            continue;
        }
        let Some(material) = fixture_materials.get(&material.0) else {
            continue;
        };
        let Some((attribute, rule)) = fixture_attribute(material)
            .and_then(|a| rules.attributes.get(&a).map(|rule| (a, rule)))
        else {
            list.tally.unstated += 1;
            continue;
        };
        let queue = material.key.render_queue;
        push(
            list, entity, mesh, transform, skin, attribute, rule.state, queue,
        );
    }
    // The character programs keep their own pass state; the queue is the
    // character attribute's.
    if let Some(character_queue) = rules.attributes.get(&attribute::CHARACTER).map(|r| r.queue) {
        for (entity, mesh, transform, skin, avatar, npc, accessory, shown) in characters.iter() {
            if !shown.get() {
                continue;
            }
            let state = if avatar {
                AVATAR_PASS
            } else {
                let dither = npc
                    .and_then(|material| character_materials.get(&material.0))
                    .map(|material| (material.params.use_dither, material.params.dither_alpha));
                // With its dither gate on, a dither alpha of 0 is below every
                // threshold of the dither table: every fragment is discarded.
                if dither.is_some_and(|(gate, alpha)| gate > 0.0 && alpha == 0.0) {
                    list.tally.npc_discarded += 1;
                    continue;
                }
                if accessory {
                    list.tally.npc_accessory += 1;
                    NPC_ACCESSORY_PASS
                } else {
                    list.tally.npc += 1;
                    NPC_PASS
                }
            };
            let Some(space) = push(
                list,
                entity,
                mesh,
                transform,
                skin,
                attribute::CHARACTER,
                state,
                character_queue,
            ) else {
                continue;
            };
            if avatar {
                list.tally.avatar += 1;
                list.silhouette.push(SilhouetteDraw {
                    mesh: mesh.0.id(),
                    space,
                    distance: transform.translation().distance(eye),
                    pipeline: CachedRenderPipelineId::INVALID,
                });
            }
        }
    }
    // Opaque queues front to back, transparent ones back to front.
    list.replay.sort_by(|a, b| {
        a.queue.cmp(&b.queue).then_with(|| {
            if a.queue <= OPAQUE_QUEUE_END {
                a.distance.total_cmp(&b.distance)
            } else {
                b.distance.total_cmp(&a.distance)
            }
        })
    });
    list.silhouette
        .sort_by(|a, b| a.distance.total_cmp(&b.distance));

    let mut not_replayed: BTreeMap<String, usize> = BTreeMap::new();
    for (entity, shown, parent) in others.iter() {
        if !shown.get() || replayed.contains(&entity) {
            continue;
        }
        list.tally.not_replayed += 1;
        if !*named {
            let name = parent
                .and_then(|parent| names.get(parent.parent()).ok())
                .map(|name| name.as_str().to_owned())
                .unwrap_or_else(|| "?".to_owned());
            *not_replayed.entry(name).or_default() += 1;
        }
    }
    if !*named {
        *named = true;
        let shown: Vec<String> = not_replayed
            .iter()
            .take(16)
            .map(|(name, count)| format!("{name} x{count}"))
            .collect();
        info!(
            "character silhouette: replay on ({door_masks} door stencil meshes shown); {} shown meshes of other programs are not replayed ({} parent nodes, first: {})",
            list.tally.not_replayed,
            not_replayed.len(),
            shown.join(", ")
        );
    }
}

/// A buffer rewritten from offset 0 each frame, replaced by a larger one
/// when the bytes plus the window read at their last offset do not fit.
struct GrowBuffer {
    label: &'static str,
    buffer: Buffer,
    capacity: u64,
}

impl GrowBuffer {
    fn new(device: &RenderDevice, label: &'static str, capacity: u64) -> Self {
        Self {
            label,
            buffer: Self::create(device, label, capacity),
            capacity,
        }
    }

    fn create(device: &RenderDevice, label: &'static str, size: u64) -> Buffer {
        device.create_buffer(&BufferDescriptor {
            label: Some(label),
            size,
            usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }

    fn write(&mut self, device: &RenderDevice, queue: &RenderQueue, bytes: &[u8], window: u64) {
        if bytes.is_empty() {
            return;
        }
        let needed = bytes.len() as u64 + window;
        if needed > self.capacity {
            self.capacity = needed.next_power_of_two();
            self.buffer = Self::create(device, self.label, self.capacity);
        }
        queue.write_buffer(&self.buffer, 0, bytes);
    }
}

#[derive(Resource)]
struct SilhouetteGpu {
    view_layout: BindGroupLayoutDescriptor,
    rigid_layout: BindGroupLayoutDescriptor,
    skin_layout: BindGroupLayoutDescriptor,
    composite_layout: BindGroupLayoutDescriptor,
    views: GrowBuffer,
    transforms: GrowBuffer,
    palettes: GrowBuffer,
    colour: Buffer,
}

fn init_gpu(mut commands: Commands, device: Res<RenderDevice>) {
    let window = |size: u64| uniform_buffer_sized(true, NonZeroU64::new(size));
    let view_layout = BindGroupLayoutDescriptor::new(
        "character_silhouette_view_layout",
        &BindGroupLayoutEntries::with_indices(ShaderStages::VERTEX, ((0, window(MATRIX_BYTES)),)),
    );
    let rigid_layout = BindGroupLayoutDescriptor::new(
        "character_silhouette_rigid_layout",
        &BindGroupLayoutEntries::with_indices(ShaderStages::VERTEX, ((0, window(MATRIX_BYTES)),)),
    );
    let skin_layout = BindGroupLayoutDescriptor::new(
        "character_silhouette_skin_layout",
        &BindGroupLayoutEntries::with_indices(ShaderStages::VERTEX, ((0, window(PALETTE_WINDOW)),)),
    );
    // The scene colour and the camera depth, both read raw with textureLoad:
    // a depth texture bound as unfilterable float reads on every backend.
    let raw = |binding: u32| BindGroupLayoutEntry {
        binding,
        visibility: ShaderStages::FRAGMENT,
        ty: BindingType::Texture {
            sample_type: TextureSampleType::Float { filterable: false },
            view_dimension: TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    };
    let composite_layout = BindGroupLayoutDescriptor::new(
        "character_silhouette_composite_layout",
        &[
            raw(0),
            raw(1),
            BindGroupLayoutEntry {
                binding: 2,
                visibility: ShaderStages::FRAGMENT,
                ty: BindingType::Buffer {
                    ty: BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: NonZeroU64::new(16),
                },
                count: None,
            },
        ],
    );
    let colour = device.create_buffer(&BufferDescriptor {
        label: Some("character_silhouette_colour"),
        size: 16,
        usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    commands.insert_resource(SilhouetteGpu {
        view_layout,
        rigid_layout,
        skin_layout,
        composite_layout,
        views: GrowBuffer::new(&device, "character_silhouette_views", SLOT as u64),
        transforms: GrowBuffer::new(&device, "character_silhouette_transforms", SLOT as u64),
        palettes: GrowBuffer::new(&device, "character_silhouette_palettes", PALETTE_WINDOW),
        colour,
    });
}

/// The pass a pipeline draws.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum PassKey {
    Replay(ReplayState),
    Silhouette(TextureFormat),
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct PipelineKey {
    layout: MeshVertexBufferLayoutRef,
    topology: PrimitiveTopology,
    skinned: bool,
    pass: PassKey,
}

#[derive(Resource, Default)]
struct SilhouettePipelines(HashMap<PipelineKey, CachedRenderPipelineId>);

fn stencil_state(rule: Option<StencilRule>) -> StencilState {
    let Some(rule) = rule else {
        return StencilState::default();
    };
    let face = StencilFaceState {
        compare: rule.compare,
        fail_op: rule.fail,
        depth_fail_op: rule.depth_fail,
        pass_op: rule.pass,
    };
    StencilState {
        front: face,
        back: face,
        read_mask: 0xff,
        write_mask: 0xff,
    }
}

/// Every replayed program and the stencil-shadow pass cull back faces (the
/// room and door materials carry cull Back, the character passes declare
/// it, the fixture pipeline keeps the engine's back-face culling).
fn queue_pipeline(
    cache: &PipelineCache,
    gpu: &SilhouetteGpu,
    key: &PipelineKey,
) -> CachedRenderPipelineId {
    let attributes = if key.skinned {
        vec![
            Mesh::ATTRIBUTE_POSITION.at_shader_location(0),
            Mesh::ATTRIBUTE_JOINT_INDEX.at_shader_location(1),
            Mesh::ATTRIBUTE_JOINT_WEIGHT.at_shader_location(2),
        ]
    } else {
        vec![Mesh::ATTRIBUTE_POSITION.at_shader_location(0)]
    };
    let Ok(vertex_layout) = key.layout.0.get_layout(&attributes) else {
        return CachedRenderPipelineId::INVALID;
    };
    let shader_defs = if key.skinned {
        vec!["SKINNED".into()]
    } else {
        Vec::new()
    };
    let space = if key.skinned {
        gpu.skin_layout.clone()
    } else {
        gpu.rigid_layout.clone()
    };
    let (label, layout, fragment, depth_stencil) = match key.pass {
        PassKey::Replay(state) => (
            "character_silhouette_replay_pipeline",
            vec![gpu.view_layout.clone(), space],
            None,
            DepthStencilState {
                format: DEPTH_STENCIL_FORMAT,
                depth_write_enabled: state.depth_write,
                depth_compare: state.depth_compare,
                stencil: stencil_state(state.stencil),
                bias: DepthBiasState::default(),
            },
        ),
        PassKey::Silhouette(format) => (
            "character_silhouette_pipeline",
            vec![gpu.view_layout.clone(), space, gpu.composite_layout.clone()],
            Some(FragmentState {
                shader: SILHOUETTE_SHADER.clone(),
                shader_defs: shader_defs.clone(),
                entry_point: Some("silhouette_fragment".into()),
                targets: vec![Some(ColorTargetState {
                    format,
                    blend: None,
                    write_mask: ColorWrites::ALL,
                })],
            }),
            DepthStencilState {
                format: DEPTH_STENCIL_FORMAT,
                depth_write_enabled: false,
                depth_compare: CompareFunction::Always,
                stencil: StencilState {
                    front: SILHOUETTE_STENCIL,
                    back: SILHOUETTE_STENCIL,
                    read_mask: 0xff,
                    write_mask: 0xff,
                },
                bias: DepthBiasState::default(),
            },
        ),
    };
    cache.queue_render_pipeline(RenderPipelineDescriptor {
        label: Some(label.into()),
        layout,
        vertex: VertexState {
            shader: SILHOUETTE_SHADER.clone(),
            shader_defs,
            entry_point: Some("vertex".into()),
            buffers: vec![vertex_layout],
        },
        fragment,
        primitive: PrimitiveState {
            topology: key.topology,
            front_face: FrontFace::Ccw,
            cull_mode: Some(Face::Back),
            ..Default::default()
        },
        depth_stencil: Some(depth_stencil),
        multisample: MultisampleState::default(),
        ..Default::default()
    })
}

/// A view's targets for the pass: the replay's depth-stencil and a copy of
/// the scene colour under the pass.
#[derive(Component)]
struct SilhouetteTarget {
    depth_stencil: CachedTexture,
    scratch: CachedTexture,
    view_offset: u32,
}

/// PrepareBindGroups: the views' targets (the view target and depth texture
/// of this frame exist by then), the pipelines and the uploads.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn prepare_silhouette(
    mut commands: Commands,
    mut list: ResMut<SilhouetteDrawList>,
    mut pipelines: ResMut<SilhouettePipelines>,
    pipeline_cache: Res<PipelineCache>,
    device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
    meshes: Res<RenderAssets<RenderMesh>>,
    mut gpu: ResMut<SilhouetteGpu>,
    mut textures: ResMut<TextureCache>,
    account: Res<SilhouetteAccount>,
    views: Query<(
        Entity,
        &ExtractedCamera,
        &ExtractedView,
        &ViewTarget,
        &ViewDepthTexture,
        Has<SilhouetteTarget>,
    )>,
    mut refused_logged: Local<bool>,
) {
    let list = &mut *list;
    let mut view_bytes = Vec::new();
    let mut format = None;
    for (entity, camera, view, target, depth, had) in &views {
        let compatible = target.main_texture().sample_count() == 1
            && depth.texture.sample_count() == 1
            && depth.texture.format() == TextureFormat::Depth32Float
            && depth
                .texture
                .usage()
                .contains(TextureUsages::TEXTURE_BINDING);
        if !list.active || camera.render_graph != Core3d.intern() || !compatible {
            if list.active && camera.render_graph == Core3d.intern() && !*refused_logged {
                *refused_logged = true;
                warn!(
                    "character silhouette refused on a view: it needs single-sample colour and a single-sample Depth32Float depth readable as a texture"
                );
            }
            if had {
                commands.entity(entity).remove::<SilhouetteTarget>();
            }
            continue;
        }
        // The colour pass's clip-from-world matrix, derived the engine's way.
        let clip_from_world = view
            .clip_from_world
            .unwrap_or_else(|| view.clip_from_view * view.world_from_view.to_matrix().inverse());
        let view_offset = push_matrix(&mut view_bytes, clip_from_world);
        let size = target.main_texture().size();
        let depth_stencil = textures.get(
            &device,
            TextureDescriptor {
                label: Some("character_silhouette_depth_stencil"),
                size,
                mip_level_count: 1,
                sample_count: 1,
                dimension: TextureDimension::D2,
                format: DEPTH_STENCIL_FORMAT,
                usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::COPY_SRC,
                view_formats: &[],
            },
        );
        let scratch = textures.get(
            &device,
            TextureDescriptor {
                label: Some("character_silhouette_scene_copy"),
                size,
                mip_level_count: 1,
                sample_count: 1,
                dimension: TextureDimension::D2,
                format: target.main_texture_format(),
                usage: TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST,
                view_formats: &[],
            },
        );
        format.get_or_insert(target.main_texture_format());
        commands.entity(entity).insert(SilhouetteTarget {
            depth_stencil,
            scratch,
            view_offset,
        });
    }
    if !list.active {
        return;
    }
    let gpu = &mut *gpu;
    gpu.views.write(&device, &queue, &view_bytes, 0);
    gpu.transforms
        .write(&device, &queue, &list.transform_bytes, 0);
    gpu.palettes
        .write(&device, &queue, &list.palette_bytes, PALETTE_WINDOW);
    let colour: Vec<u8> = list.colour.iter().flat_map(|c| c.to_le_bytes()).collect();
    queue.write_buffer(&gpu.colour, 0, &colour);

    let gpu: &SilhouetteGpu = gpu;
    let mut pipeline_for = |mesh: AssetId<Mesh>, skinned: bool, pass: PassKey| {
        let Some(render_mesh) = meshes.get(mesh) else {
            return CachedRenderPipelineId::INVALID;
        };
        let key = PipelineKey {
            layout: render_mesh.layout.clone(),
            topology: render_mesh.primitive_topology(),
            skinned,
            pass,
        };
        *pipelines
            .0
            .entry(key)
            .or_insert_with_key(|key| queue_pipeline(&pipeline_cache, gpu, key))
    };
    for draw in &mut list.replay {
        let skinned = matches!(draw.space, Space::Skinned(_));
        draw.pipeline = pipeline_for(draw.mesh, skinned, PassKey::Replay(draw.state));
    }
    if let Some(format) = format {
        for draw in &mut list.silhouette {
            let skinned = matches!(draw.space, Space::Skinned(_));
            draw.pipeline = pipeline_for(draw.mesh, skinned, PassKey::Silhouette(format));
        }
    }
    let mut inner = account.0.lock().unwrap();
    inner.tally = list.tally.clone();
    inner.replay_draws = list.replay.len();
    inner.replay_invalid = list
        .replay
        .iter()
        .filter(|draw| draw.pipeline == CachedRenderPipelineId::INVALID)
        .count();
    inner.silhouette_draws = list.silhouette.len();
    inner.colour = list.colour;
}

/// The first read-back comes on this replayed frame, then one every
/// `MOLY_SILHOUETTE_PROBE_INTERVAL` replayed frames (default below).
const PROBE_FIRST_FRAME: u64 = 10;
const PROBE_INTERVAL: u64 = 300;

fn probe_interval() -> u64 {
    static INTERVAL: std::sync::OnceLock<u64> = std::sync::OnceLock::new();
    *INTERVAL.get_or_init(|| {
        std::env::var("MOLY_SILHOUETTE_PROBE_INTERVAL")
            .ok()
            .and_then(|raw| raw.trim().parse::<u64>().ok())
            .filter(|frames| *frames > 0)
            .unwrap_or(PROBE_INTERVAL)
    })
}

/// A stencil read-back recorded this frame, read on the next.
#[cfg(not(target_arch = "wasm32"))]
struct PendingProbe {
    width: u32,
    height: u32,
    row: u32,
    frame: u64,
    tally: SilhouetteTally,
    replay_draws: usize,
    replay_invalid: usize,
    replay_recorded: usize,
    silhouette_draws: usize,
    silhouette_recorded: usize,
}

#[derive(Default)]
struct AccountInner {
    tally: SilhouetteTally,
    replay_draws: usize,
    replay_invalid: usize,
    silhouette_draws: usize,
    colour: [f32; 4],
    frames_drawn: u64,
    frames_not_ready: u64,
    silhouette_not_ready: u64,
    next_probe: u64,
    /// The stencil before and after the silhouette, and their byte size.
    #[cfg(not(target_arch = "wasm32"))]
    buffers: Option<(Buffer, Buffer, u64)>,
    #[cfg(not(target_arch = "wasm32"))]
    probe: Option<PendingProbe>,
}

/// The pass's account: prepare writes the list's counts, the node its frames
/// and read-backs. The node has shared access only.
#[derive(Resource, Default)]
struct SilhouetteAccount(Mutex<AccountInner>);

#[derive(Debug, Hash, PartialEq, Eq, Clone, RenderLabel)]
struct CharacterSilhouetteLabel;

struct CharacterSilhouetteNode {
    bind_groups: SharedBindGroupCache,
}

impl FromWorld for CharacterSilhouetteNode {
    fn from_world(world: &mut World) -> Self {
        Self {
            bind_groups: SharedBindGroupCache::from_world(world),
        }
    }
}

/// Binds a mesh's vertex (and index) buffer and draws it once; false when
/// the mesh is not on the GPU.
macro_rules! draw_mesh {
    ($pass:expr, $mesh:expr, $meshes:expr, $allocator:expr) => {{
        let mesh: &AssetId<Mesh> = $mesh;
        match ($meshes.get(*mesh), $allocator.mesh_vertex_slice(mesh)) {
            (Some(render_mesh), Some(vertex)) => match &render_mesh.buffer_info {
                RenderMeshBufferInfo::Indexed {
                    index_format,
                    count,
                } => match $allocator.mesh_index_slice(mesh) {
                    Some(index) => {
                        $pass.set_vertex_buffer(0, *vertex.buffer.slice(..));
                        $pass.set_index_buffer(*index.buffer.slice(..), *index_format);
                        $pass.draw_indexed(
                            index.range.start..(index.range.start + *count),
                            vertex.range.start as i32,
                            0..1,
                        );
                        true
                    }
                    None => false,
                },
                RenderMeshBufferInfo::NonIndexed => {
                    $pass.set_vertex_buffer(0, *vertex.buffer.slice(..));
                    $pass.draw(vertex.range.clone(), 0..1);
                    true
                }
            },
            _ => false,
        }
    }};
}

impl ViewNode for CharacterSilhouetteNode {
    type ViewQuery = (
        &'static ViewTarget,
        &'static ViewDepthTexture,
        Option<&'static SilhouetteTarget>,
    );

    fn run(
        &self,
        _graph: &mut RenderGraphContext,
        render_context: &mut RenderContext,
        (view_target, depth, target): QueryItem<Self::ViewQuery>,
        world: &World,
    ) -> Result<(), NodeRunError> {
        #[cfg(not(target_arch = "wasm32"))]
        finish_pending_probe(world, render_context);
        let Some(target) = target else {
            return Ok(());
        };
        let list = world.resource::<SilhouetteDrawList>();
        if !list.active {
            return Ok(());
        }
        let account = world.resource::<SilhouetteAccount>();
        let cache = world.resource::<PipelineCache>();
        // Every replay pipeline must be ready: a draw left out would change
        // the stencil the silhouette tests. A draw without a pipeline (its
        // mesh lacks a stream) is counted in prepare.
        let mut replay_pipelines = Vec::with_capacity(list.replay.len());
        for draw in &list.replay {
            if draw.pipeline == CachedRenderPipelineId::INVALID {
                replay_pipelines.push(None);
                continue;
            }
            let Some(pipeline) = cache.get_render_pipeline(draw.pipeline) else {
                account.0.lock().unwrap().frames_not_ready += 1;
                return Ok(());
            };
            replay_pipelines.push(Some(pipeline));
        }
        let mut silhouette_pipelines = Vec::with_capacity(list.silhouette.len());
        for draw in &list.silhouette {
            if draw.pipeline == CachedRenderPipelineId::INVALID {
                silhouette_pipelines.push(None);
                continue;
            }
            match cache.get_render_pipeline(draw.pipeline) {
                Some(pipeline) => silhouette_pipelines.push(Some(pipeline)),
                None => {
                    account.0.lock().unwrap().silhouette_not_ready += 1;
                    silhouette_pipelines.clear();
                    break;
                }
            }
        }
        let run_silhouette = silhouette_pipelines.iter().any(Option::is_some);

        let gpu = world.resource::<SilhouetteGpu>();
        let meshes = world.resource::<RenderAssets<RenderMesh>>();
        let allocator = world.resource::<MeshAllocator>();
        let frame = world.resource::<FrameCount>().0;
        let (view_group, rigid_group, skin_group, composite_group) = {
            let device = render_context.render_device();
            let mut groups = self.bind_groups.lock();
            let view_group = groups.get(
                device,
                "character_silhouette_view",
                &cache.get_bind_group_layout(&gpu.view_layout),
                &[(
                    0,
                    Bound::Buffer(&gpu.views.buffer, 0, NonZeroU64::new(MATRIX_BYTES)),
                )],
                frame,
            );
            let rigid_group = groups.get(
                device,
                "character_silhouette_rigid",
                &cache.get_bind_group_layout(&gpu.rigid_layout),
                &[(
                    0,
                    Bound::Buffer(&gpu.transforms.buffer, 0, NonZeroU64::new(MATRIX_BYTES)),
                )],
                frame,
            );
            let skin_group = groups.get(
                device,
                "character_silhouette_skin",
                &cache.get_bind_group_layout(&gpu.skin_layout),
                &[(
                    0,
                    Bound::Buffer(&gpu.palettes.buffer, 0, NonZeroU64::new(PALETTE_WINDOW)),
                )],
                frame,
            );
            let composite_group = groups.get(
                device,
                "character_silhouette_composite",
                &cache.get_bind_group_layout(&gpu.composite_layout),
                &[
                    (0, Bound::View(&target.scratch.default_view)),
                    (1, Bound::View(depth.view())),
                    (2, Bound::whole(&gpu.colour)),
                ],
                frame,
            );
            (view_group, rigid_group, skin_group, composite_group)
        };

        // The replay: depth and stencil of the stencil-relevant draws in
        // queue order, from a cleared target (far depth, stencil 0).
        let mut replayed = 0usize;
        {
            let mut pass =
                render_context
                    .command_encoder()
                    .begin_render_pass(&RenderPassDescriptor {
                        label: Some("character_silhouette_replay"),
                        color_attachments: &[],
                        depth_stencil_attachment: Some(RenderPassDepthStencilAttachment {
                            view: &target.depth_stencil.default_view,
                            depth_ops: Some(Operations {
                                load: LoadOp::Clear(0.0),
                                store: StoreOp::Store,
                            }),
                            stencil_ops: Some(Operations {
                                load: LoadOp::Clear(0),
                                store: StoreOp::Store,
                            }),
                        }),
                        timestamp_writes: None,
                        occlusion_query_set: None,
                    });
            for (draw, pipeline) in list.replay.iter().zip(&replay_pipelines) {
                let Some(pipeline) = pipeline else {
                    continue;
                };
                pass.set_pipeline(pipeline);
                pass.set_stencil_reference(draw.state.stencil.map_or(0, |rule| rule.reference));
                pass.set_bind_group(0, &view_group, &[target.view_offset]);
                match draw.space {
                    Space::Rigid(offset) => pass.set_bind_group(1, &rigid_group, &[offset]),
                    Space::Skinned(offset) => pass.set_bind_group(1, &skin_group, &[offset]),
                }
                if draw_mesh!(pass, &draw.mesh, meshes, allocator) {
                    replayed += 1;
                }
            }
        }

        let probe_due = {
            let mut inner = account.0.lock().unwrap();
            inner.frames_drawn += 1;
            if inner.next_probe == 0 {
                inner.next_probe = PROBE_FIRST_FRAME;
            }
            let due = inner.frames_drawn >= inner.next_probe;
            if due {
                inner.next_probe = inner.frames_drawn + probe_interval();
            }
            due
        };
        #[cfg(not(target_arch = "wasm32"))]
        let probe = probe_due.then(|| begin_probe(world, render_context, target));
        #[cfg(target_arch = "wasm32")]
        if probe_due {
            log_counter_line(world);
        }

        // The stencil-shadow pass: the scene colour under it is copied first,
        // since the pass blends with it in encoded values.
        let mut silhouette_drawn = 0usize;
        if run_silhouette {
            let size = target.scratch.texture.size();
            render_context.command_encoder().copy_texture_to_texture(
                view_target.main_texture().as_image_copy(),
                target.scratch.texture.as_image_copy(),
                size,
            );
            let mut pass =
                render_context
                    .command_encoder()
                    .begin_render_pass(&RenderPassDescriptor {
                        label: Some("character_silhouette"),
                        color_attachments: &[Some(view_target.get_color_attachment())],
                        depth_stencil_attachment: Some(RenderPassDepthStencilAttachment {
                            view: &target.depth_stencil.default_view,
                            depth_ops: Some(Operations {
                                load: LoadOp::Load,
                                store: StoreOp::Store,
                            }),
                            stencil_ops: Some(Operations {
                                load: LoadOp::Load,
                                store: StoreOp::Store,
                            }),
                        }),
                        timestamp_writes: None,
                        occlusion_query_set: None,
                    });
            pass.set_stencil_reference(SILHOUETTE_REFERENCE);
            for (draw, pipeline) in list.silhouette.iter().zip(&silhouette_pipelines) {
                let Some(pipeline) = pipeline else {
                    continue;
                };
                pass.set_pipeline(pipeline);
                pass.set_bind_group(0, &view_group, &[target.view_offset]);
                match draw.space {
                    Space::Rigid(offset) => pass.set_bind_group(1, &rigid_group, &[offset]),
                    Space::Skinned(offset) => pass.set_bind_group(1, &skin_group, &[offset]),
                }
                pass.set_bind_group(2, &composite_group, &[]);
                if draw_mesh!(pass, &draw.mesh, meshes, allocator) {
                    silhouette_drawn += 1;
                }
            }
        }

        #[cfg(not(target_arch = "wasm32"))]
        if let Some(Some((after, width, height, row))) = probe {
            copy_stencil(
                render_context,
                &target.depth_stencil.texture,
                &after,
                width,
                height,
                row,
            );
            let mut inner = account.0.lock().unwrap();
            let pending = PendingProbe {
                width,
                height,
                row,
                frame: inner.frames_drawn,
                tally: inner.tally.clone(),
                replay_draws: inner.replay_draws,
                replay_invalid: inner.replay_invalid,
                replay_recorded: replayed,
                silhouette_draws: inner.silhouette_draws,
                silhouette_recorded: silhouette_drawn,
            };
            inner.probe = Some(pending);
        }
        #[cfg(target_arch = "wasm32")]
        let _ = (replayed, silhouette_drawn);
        Ok(())
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn copy_stencil(
    render_context: &mut RenderContext,
    texture: &Texture,
    buffer: &Buffer,
    width: u32,
    height: u32,
    row: u32,
) {
    render_context.command_encoder().copy_texture_to_buffer(
        TexelCopyTextureInfo {
            texture,
            mip_level: 0,
            origin: Origin3d::ZERO,
            aspect: TextureAspect::StencilOnly,
        },
        TexelCopyBufferInfo {
            buffer,
            layout: TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(row),
                rows_per_image: None,
            },
        },
        Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
}

/// Copies the replayed stencil (before the silhouette) and returns the
/// buffer for the copy after it; `None` while a read-back is still pending.
#[cfg(not(target_arch = "wasm32"))]
fn begin_probe(
    world: &World,
    render_context: &mut RenderContext,
    target: &SilhouetteTarget,
) -> Option<(Buffer, u32, u32, u32)> {
    let size = target.depth_stencil.texture.size();
    let (width, height) = (size.width, size.height);
    // Buffer copies take rows in multiples of 256 bytes.
    let row = width.next_multiple_of(256);
    let bytes = row as u64 * height as u64;
    let (before, after) = {
        let mut inner = world.resource::<SilhouetteAccount>().0.lock().unwrap();
        if inner.probe.is_some() {
            return None;
        }
        if inner
            .buffers
            .as_ref()
            .is_none_or(|(_, _, size)| *size != bytes)
        {
            let device = render_context.render_device();
            let buffer = |label: &'static str| {
                device.create_buffer(&BufferDescriptor {
                    label: Some(label),
                    size: bytes,
                    usage: BufferUsages::MAP_READ | BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                })
            };
            inner.buffers = Some((
                buffer("character_silhouette_stencil_before"),
                buffer("character_silhouette_stencil_after"),
                bytes,
            ));
        }
        let (before, after, _) = inner.buffers.clone().unwrap();
        (before, after)
    };
    copy_stencil(
        render_context,
        &target.depth_stencil.texture,
        &before,
        width,
        height,
        row,
    );
    Some((after, width, height, row))
}

/// Reads last frame's two stencil copies and writes the account line: the
/// stencil values the silhouette pass tested, and the pixels it drew (a 1
/// its pass decremented to 0).
#[cfg(not(target_arch = "wasm32"))]
fn finish_pending_probe(world: &World, render_context: &RenderContext) {
    let account = world.resource::<SilhouetteAccount>();
    let mut inner = account.0.lock().unwrap();
    let Some(probe) = inner.probe.take() else {
        return;
    };
    let Some((before, after, _)) = inner.buffers.clone() else {
        return;
    };
    let (sender, receiver) = std::sync::mpsc::channel();
    for (index, buffer) in [&before, &after].into_iter().enumerate() {
        let sender = sender.clone();
        buffer.slice(..).map_async(MapMode::Read, move |result| {
            let _ = sender.send((index, result.is_ok()));
        });
    }
    drop(sender);
    if render_context
        .render_device()
        .poll(PollType::wait_indefinitely())
        .is_err()
    {
        warn!("character silhouette account: the stencil read-back poll failed; this read-back is void");
        return;
    }
    let mapped: Vec<(usize, bool)> = receiver.iter().collect();
    if mapped.len() != 2 || mapped.iter().any(|(_, ok)| !ok) {
        for (index, ok) in &mapped {
            if *ok {
                [&before, &after][*index].unmap();
            }
        }
        warn!(
            "character silhouette account: a stencil read-back did not map; this read-back is void"
        );
        return;
    }
    let (mut histogram, mut left, mut drawn) = ([0usize; 256], 0usize, 0usize);
    {
        let a = before.slice(..).get_mapped_range();
        let b = after.slice(..).get_mapped_range();
        for y in 0..probe.height as usize {
            let start = y * probe.row as usize;
            for x in start..start + probe.width as usize {
                histogram[a[x] as usize] += 1;
                left += (b[x] == 1) as usize;
                drawn += (a[x] == 1 && b[x] == 0) as usize;
            }
        }
    }
    before.unmap();
    after.unmap();
    let tally = &probe.tally;
    info!(
        "character silhouette account (replayed frame {}): door stencil meshes {}; replay draws recorded {} of {} (by shader attribute {:?}; pipelines missing {}), avatar meshes {}, NPC meshes {} and accessory meshes {} ({} left out, every fragment discarded by the dither), attribute rows missing {}, shown meshes of other programs not replayed {}, palettes refused {}; frames skipped with pipelines compiling: replay {}, silhouette {}; stencil at the pass over {}x{} pixels: 0: {}, 1: {}, 2: {}, 3: {}, 4 and above: {}; silhouette draws recorded {} of {}, pixels drawn {} (a 1 decremented to 0), 1 left after the pass {}; colour {:?} from the graphics configuration",
        probe.frame,
        tally.door_masks,
        probe.replay_recorded,
        probe.replay_draws,
        tally.by_attribute,
        probe.replay_invalid,
        tally.avatar,
        tally.npc,
        tally.npc_accessory,
        tally.npc_discarded,
        tally.unstated,
        tally.not_replayed,
        tally.palettes_refused,
        inner.frames_not_ready,
        inner.silhouette_not_ready,
        probe.width,
        probe.height,
        histogram[0],
        histogram[1],
        histogram[2],
        histogram[3],
        histogram[4..].iter().sum::<usize>(),
        probe.silhouette_recorded,
        probe.silhouette_draws,
        drawn,
        left,
        inner.colour,
    );
}

/// The wasm build has no blocking read-back: its account line has the
/// counts only.
#[cfg(target_arch = "wasm32")]
fn log_counter_line(world: &World) {
    let inner = world.resource::<SilhouetteAccount>().0.lock().unwrap();
    info!(
        "character silhouette account (counts): replayed frames {}, door stencil meshes {}, replay draws {} (pipelines missing {}), silhouette draws {}; frames skipped with pipelines compiling: replay {}, silhouette {}",
        inner.frames_drawn,
        inner.tally.door_masks,
        inner.replay_draws,
        inner.replay_invalid,
        inner.silhouette_draws,
        inner.frames_not_ready,
        inner.silhouette_not_ready,
    );
}

fn load_shader(mut shaders: ResMut<Assets<Shader>>) {
    let _ = shaders.insert(
        SILHOUETTE_SHADER.id(),
        Shader::from_wgsl(
            include_str!("shaders/character_silhouette.wgsl"),
            "moly_game/src/shaders/character_silhouette.wgsl".to_owned(),
        ),
    );
}

/// The stencil-shadow pass of the player avatar, after the transparents
/// (where the source's feature runs) and before the main pass ends.
pub struct CharacterSilhouettePlugin;

impl Plugin for CharacterSilhouettePlugin {
    fn build(&self, app: &mut App) {
        crate::render::gpu::install_bind_group_caches(app);
        app.add_systems(Startup, (load_shader, load_rules))
            .add_systems(Update, settle_rules);
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
            return;
        };
        render_app
            .init_resource::<SilhouetteDrawList>()
            .init_resource::<SilhouettePipelines>()
            .init_resource::<SilhouetteAccount>()
            .add_systems(RenderStartup, init_gpu)
            .add_systems(ExtractSchedule, extract_silhouette_draws)
            .add_systems(
                Render,
                prepare_silhouette.in_set(RenderSystems::PrepareBindGroups),
            )
            .add_render_graph_node::<ViewNodeRunner<CharacterSilhouetteNode>>(
                Core3d,
                CharacterSilhouetteLabel,
            )
            .add_render_graph_edges(
                Core3d,
                (
                    Node3d::MainTransparentPass,
                    CharacterSilhouetteLabel,
                    Node3d::EndMainPass,
                ),
            );
    }
}
