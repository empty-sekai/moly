//! The layout grid while the layout editor is open: `ShowGrid` (event 15)
//! draws it, `HideGrid` (event 16) removes it.
//!
//! Source: the grid manager's subscriber takes the current site's grid
//! controller and calls `SetShowGrid(layout type)`; `ShowGrid(2)` (floor)
//! shows the floor grid controller, whose line and tile children the layout
//! editor built from the floor grid's bound (`GridData.GridBound`) under the
//! site view. `HideGrid` hides it. Both children sit at `(0, PositionY, 0)`
//! under the floor controller, which sits at `(0, PositionY, 0)` itself, so
//! the grid plane is at `2 * PositionY` above the site view.
//!
//! Lines (`FloorLineGridController.RefreshView`, `GridLine.CalculateMesh`):
//! `TileCount.x + 1` lines of constant x and `TileCount.z + 1` of constant z,
//! evenly spread between `Min` (the bound's minimum tile's minimum corner)
//! and `Max` (the maximum tile's maximum corner): line `i` of `n` at
//! `Min + (Max - Min) * clamp(i / (n - 1), 0, 1)`. Each is a quad of the
//! settings' width (`VerticleLineSize` for the constant-x lines,
//! `HorizontalLineSize` for the others) running the length of the grid; uv.x
//! is the position along the line (z for the constant-x lines, x for the
//! others). Every fourth line from the minimum edge is solid (submesh 0, the
//! `GridLine` material), the rest dotted (submesh 1, `DottedGridLine`).
//!
//! Tiles (`Grid2DFaceUber`): one quad over the bound, uv (0, 0) at the
//! minimum corner and (tile count x, tile count z) at the maximum, and an
//! RGBA32 state texture with one texel per tile (texel index x + width * z
//! from the bound's minimum). A texel packs the first state set of
//! Highlight, MotionConflictArea, MotionArea, MotionDisableArea,
//! FixtureExist and HidedFixture as n = the state's bit index + 1 in its red,
//! green and blue bits (4, 2, 1); `_TileSize` is set to the tile size.
//!
//! The fill on entry (`GridModel.SetFocus` for the floor): FixtureExist over
//! the bounding box of every site fixture placed in the floor layout; for
//! every fixture placed in exactly the floor layout its motion-area cells as
//! MotionDisableArea when any of them is off the grid or on another fixture
//! (`HasBoundBoxListOtherFixture`), else as MotionArea; then every tile that
//! has Highlight or FixtureExist together with MotionArea or
//! MotionDisableArea becomes MotionConflictArea alone. A selected fixture's
//! footprint is Highlight.
//!
//! Colours: the site environment's configuration names a colour profile
//! (`gridColorKey`); the profile table returns the first row with that key,
//! or its first row. `UpdateGridColor` writes each profile colour into its
//! material (`_Color`, `_Color1` and, for two-colour entries, `_Color2`), and
//! the tile face reads Highlight (`MovingFixtureTileSafe` while the fixture
//! can be placed, else `MovingFixtureTileAlert`), FixtureExist
//! (`PlacedFixtureTile`), HidedFixture, MotionArea and MotionDisableArea (their
//! two colours) and MotionConflictArea (the MotionDisableArea colour twice).
//! Until a profile is read the materials keep their serialized colours.
//!
//! Inputs: `layout-grid/grid.json` (the grid settings, the materials, their
//! shaders' pass state, the colour profiles) and the phenomenon index's
//! configuration for the committed phenomenon at the current environment
//! site. The programs are ported in `edit_grid_line.wgsl` and
//! `edit_grid_face.wgsl`. Each material's queue and pass state (cull, depth
//! test and write, blend, colour mask) are taken from the asset.
//!
//! The source works in its own grid frame; the product frame is its X mirror
//! (a product cell x is the source cell -x - 1, a product position x is the
//! source -x). Geometry and the texel layout are computed in the source
//! frame and mirrored per vertex, so uv, the stripe direction and the texel
//! index keep the source's values.
//!
//! Named gaps: the wall grid, the floating faces and the light poles are not
//! built; the unavailable zones (room levels only), the fixtures' extra used
//! bounds, the birthday cutscene areas and the hidden-fixture state are not
//! filled; with a fixture selected the fill keeps the entry rule for the
//! others and the selection's own motion area is not drawn; the line and the
//! tile renderers share queue 2025 and their order within it is not the
//! source's (the source sorts them with the engine's opaque criteria);
//! blending happens in the target's linear space, where the source (a gamma
//! space player) blends stored values.

use std::marker::PhantomData;

use bevy::asset::uuid::Uuid;
use bevy::asset::{AssetPath, LoadState, RenderAssetUsages};
use bevy::mesh::{Indices, MeshVertexBufferLayoutRef, PrimitiveTopology};
use bevy::pbr::{Material, MaterialPipeline, MaterialPipelineKey, MaterialPlugin};
use bevy::prelude::*;
use bevy::render::render_resource::{
    AsBindGroup, Extent3d, RenderPipelineDescriptor, ShaderType, SpecializedMeshPipelineError,
    TextureDimension, TextureFormat,
};
use bevy::shader::ShaderRef;
use moly_assets::json::JsonAsset;
use moly_assets::material_passes::SourceRenderState;
use moly_law::fixture::areas::motion_area_bounds;
use moly_law::fixture::position::layout_type;
use moly_law::fixture::GridPosition;
use serde_json::Value;

use super::assets::FixtureAreas;
use super::tile_rules::TileBox;
use super::{EditSession, FixtureEditSystems, PutStatus};
use crate::fixture::{EditableFixture, FixturePlacements};

const GRID: &str = "moly://layout-grid/grid.json";
const PHENOMENA: &str = "moly://phenomena/index.json";
/// `MysekaiConstants.TILE_SIZE`: the cell size and the face's `_TileSize`.
const TILE_SIZE: f32 = 0.25;
/// `GridLineMeshSettings` solid intervals (both directions).
const SOLID_INTERVAL: usize = 4;

const LINE_SHADER: Handle<Shader> = Handle::Uuid(
    Uuid::from_u128(0x5c1e_07a2_9b44_4d3e_8f61_2a7d_b0c9_e513),
    PhantomData,
);
const FACE_SHADER: Handle<Shader> = Handle::Uuid(
    Uuid::from_u128(0x0e8b_3f15_c6d2_47a9_b1f4_93e0_6a28_d7c1),
    PhantomData,
);

// `GridFillState` bits.
const HIGHLIGHT: u8 = 1;
const FIXTURE_EXIST: u8 = 2;
const HIDED_FIXTURE: u8 = 4;
const MOTION_AREA: u8 = 8;
const MOTION_DISABLE: u8 = 16;
const MOTION_CONFLICT: u8 = 32;
/// `Grid2DFaceUber.GridStateTable`: the packing's priority order.
const STATE_ORDER: [u8; 6] = [
    HIGHLIGHT,
    MOTION_CONFLICT,
    MOTION_AREA,
    MOTION_DISABLE,
    FIXTURE_EXIST,
    HIDED_FIXTURE,
];

/// A grid material's queue and pass state (the pipeline key).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct GridPassKey {
    queue: u32,
    state: SourceRenderState,
}

#[derive(Debug, Clone, Copy, ShaderType)]
struct LineUniform {
    /// `_MainTex_ST`.
    main_tex_st: Vec4,
    /// `_Color1` as stored.
    color1: Vec4,
    /// (`_LineRepeat`, `_LineFill`, 0, 0).
    line: Vec4,
}

/// `Mysekai/Grid/Line`.
#[derive(Asset, TypePath, Debug, Clone, AsBindGroup)]
#[bind_group_data(GridPassKey)]
pub(crate) struct GridLineMaterial {
    #[uniform(0)]
    value: LineUniform,
    key: GridPassKey,
}

impl From<&GridLineMaterial> for GridPassKey {
    fn from(material: &GridLineMaterial) -> Self {
        material.key
    }
}

#[derive(Debug, Clone, Copy, ShaderType)]
struct FaceUniform {
    /// (`_TileSize`, `_PatternRepeat`, `_FillRate`, 0).
    params: Vec4,
    highlight: Vec4,
    fixture_exist: Vec4,
    hided: Vec4,
    hided2: Vec4,
    motion_area: Vec4,
    motion_area2: Vec4,
    motion_disable: Vec4,
    motion_disable2: Vec4,
    conflict: Vec4,
    conflict2: Vec4,
}

/// `Mysekai/Grid/Face-Uber`.
#[derive(Asset, TypePath, Debug, Clone, AsBindGroup)]
#[bind_group_data(GridPassKey)]
pub(crate) struct GridFaceMaterial {
    #[uniform(0)]
    value: FaceUniform,
    /// The state texture (`_MainTex`), read with texel fetches.
    #[texture(1)]
    state: Handle<Image>,
    key: GridPassKey,
}

impl From<&GridFaceMaterial> for GridPassKey {
    fn from(material: &GridFaceMaterial) -> Self {
        material.key
    }
}

fn specialize_grid(descriptor: &mut RenderPipelineDescriptor, key: GridPassKey) {
    crate::material_order::set_queue(descriptor, key.queue);
    crate::source_render_state::apply(descriptor, key.state);
}

impl Material for GridLineMaterial {
    fn vertex_shader() -> ShaderRef {
        ShaderRef::Handle(LINE_SHADER.clone())
    }
    fn fragment_shader() -> ShaderRef {
        ShaderRef::Handle(LINE_SHADER.clone())
    }
    fn enable_prepass() -> bool {
        false
    }
    fn enable_shadows() -> bool {
        false
    }
    fn specialize(
        _pipeline: &MaterialPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        _layout: &MeshVertexBufferLayoutRef,
        key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        specialize_grid(descriptor, key.bind_group_data);
        Ok(())
    }
}

impl Material for GridFaceMaterial {
    fn vertex_shader() -> ShaderRef {
        ShaderRef::Handle(FACE_SHADER.clone())
    }
    fn fragment_shader() -> ShaderRef {
        ShaderRef::Handle(FACE_SHADER.clone())
    }
    fn enable_prepass() -> bool {
        false
    }
    fn enable_shadows() -> bool {
        false
    }
    fn specialize(
        _pipeline: &MaterialPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        _layout: &MeshVertexBufferLayoutRef,
        key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        specialize_grid(descriptor, key.bind_group_data);
        Ok(())
    }
}

/// One material of the grid settings' library, as read from the asset.
#[derive(Clone, Debug)]
struct MaterialDoc {
    name: String,
    shader: String,
    queue: u32,
    state: SourceRenderState,
    floats: Vec<(String, f32)>,
    colors: Vec<(String, Vec4)>,
    main_tex_st: Vec4,
}

impl MaterialDoc {
    fn float(&self, name: &str) -> Result<f32, String> {
        self.floats
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| *value)
            .ok_or_else(|| format!("material {} has no float {name}", self.name))
    }
    fn color(&self, name: &str) -> Result<Vec4, String> {
        self.colors
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| *value)
            .ok_or_else(|| format!("material {} has no colour {name}", self.name))
    }
}

/// One colour profile row (the fields the floor grid's line and tile parts
/// read).
#[derive(Clone, Debug)]
struct Profile {
    key: String,
    grid_line: Vec4,
    dotted_grid_line: Vec4,
    moving_safe: Vec4,
    moving_alert: Vec4,
    placed: Vec4,
    hided: (Vec4, Vec4),
    motion_area: (Vec4, Vec4),
    motion_disable: (Vec4, Vec4),
}

#[derive(Clone, Debug)]
struct GridDoc {
    vertical_line_size: f32,
    horizontal_line_size: f32,
    position_y: f32,
    show_line: bool,
    line: MaterialDoc,
    dotted: MaterialDoc,
    uber: MaterialDoc,
    profiles: Vec<Profile>,
}

fn number(value: &Value, what: &str) -> Result<f32, String> {
    value
        .as_f64()
        .map(|v| v as f32)
        .ok_or_else(|| format!("{what} is not a number"))
}

fn color(value: &Value, what: &str) -> Result<Vec4, String> {
    let channel = |name: &str| {
        value
            .get(name)
            .and_then(Value::as_f64)
            .map(|v| v as f32)
            .ok_or_else(|| format!("{what} has no {name}"))
    };
    Ok(Vec4::new(
        channel("r")?,
        channel("g")?,
        channel("b")?,
        channel("a")?,
    ))
}

/// A pass state entry: the material float it names, else the shader's value.
fn state_value(entry: &Value, material: &Value, what: &str) -> Result<f32, String> {
    let fixed = number(
        entry.get("value").unwrap_or(&Value::Null),
        &format!("{what} value"),
    )?;
    match entry.get("property").and_then(Value::as_str) {
        None => Ok(fixed),
        Some(name) => material
            .get("floats")
            .and_then(|floats| floats.get(name))
            .and_then(Value::as_f64)
            .map(|v| v as f32)
            .ok_or_else(|| format!("{what} names material float {name}, which is absent")),
    }
}

fn small(value: f32, max: u8, what: &str) -> Result<u8, String> {
    if value.fract() != 0.0 || value < 0.0 || value > f32::from(max) {
        return Err(format!(
            "{what} {value} is not a source enum value 0..={max}"
        ));
    }
    Ok(value as u8)
}

fn material_doc(doc: &Value, slot: &str) -> Result<MaterialDoc, String> {
    let material = doc
        .get("materials")
        .and_then(|m| m.get(slot))
        .filter(|m| !m.is_null())
        .ok_or_else(|| format!("material slot {slot} is empty"))?;
    let name = material
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("{slot}: no name"))?
        .to_owned();
    let shader = material
        .get("shader")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("{slot}: no shader"))?
        .to_owned();
    let queue = material
        .get("renderQueue")
        .and_then(Value::as_i64)
        .ok_or_else(|| format!("{slot}: no render queue"))?;
    let queue = u32::try_from(queue)
        .map_err(|_| format!("{slot}: render queue {queue} takes the shader's queue, not read"))?;
    let passes = doc
        .get("shaders")
        .and_then(|s| s.get(&shader))
        .and_then(|s| s.get("passes"))
        .and_then(Value::as_array)
        .ok_or_else(|| format!("{slot}: shader {shader} has no passes"))?;
    let [pass] = passes.as_slice() else {
        return Err(format!(
            "{slot}: shader {shader} has {} passes, one expected",
            passes.len()
        ));
    };
    let field = |name: &str| pass.get(name).unwrap_or(&Value::Null);
    let blend = field("blend");
    let blend_value = |name: &str| {
        state_value(
            blend.get(name).unwrap_or(&Value::Null),
            material,
            &format!("{shader} {name}"),
        )
    };
    let pass_value = |name: &str| state_value(field(name), material, &format!("{shader} {name}"));
    for name in ["offsetFactor", "offsetUnits", "alphaToMask"] {
        if pass_value(name)? != 0.0 {
            return Err(format!("{slot}: {shader} {name} is not 0; not ported"));
        }
    }
    let state = SourceRenderState {
        cull: small(pass_value("culling")?, 2, "culling")?,
        depth_test: small(pass_value("zTest")?, 8, "zTest")?,
        depth_write: pass_value("zWrite")? != 0.0,
        color_mask: small(blend_value("colMask")?, 15, "colMask")?,
        src_color: small(blend_value("srcBlend")?, 10, "srcBlend")?,
        dst_color: small(blend_value("destBlend")?, 10, "destBlend")?,
        src_alpha: small(blend_value("srcBlendAlpha")?, 10, "srcBlendAlpha")?,
        dst_alpha: small(blend_value("destBlendAlpha")?, 10, "destBlendAlpha")?,
        color_op: small(blend_value("blendOp")?, 4, "blendOp")?,
        alpha_op: small(blend_value("blendOpAlpha")?, 4, "blendOpAlpha")?,
    };
    let floats = material
        .get("floats")
        .and_then(Value::as_object)
        .ok_or_else(|| format!("{slot}: no floats"))?
        .iter()
        .map(|(key, value)| Ok((key.clone(), number(value, key)?)))
        .collect::<Result<Vec<_>, String>>()?;
    let colors = material
        .get("colors")
        .and_then(Value::as_object)
        .ok_or_else(|| format!("{slot}: no colours"))?
        .iter()
        .map(|(key, value)| Ok((key.clone(), color(value, key)?)))
        .collect::<Result<Vec<_>, String>>()?;
    let main_tex_st = match material.get("textures").and_then(|t| t.get("_MainTex")) {
        Some(tex) => {
            let pair = |name: &str| -> Result<(f32, f32), String> {
                let v = tex
                    .get(name)
                    .ok_or_else(|| format!("{slot}: _MainTex has no {name}"))?;
                Ok((
                    number(v.get("x").unwrap_or(&Value::Null), "x")?,
                    number(v.get("y").unwrap_or(&Value::Null), "y")?,
                ))
            };
            let (sx, sy) = pair("scale")?;
            let (ox, oy) = pair("offset")?;
            Vec4::new(sx, sy, ox, oy)
        }
        None => return Err(format!("{slot}: no _MainTex scale and offset")),
    };
    Ok(MaterialDoc {
        name,
        shader,
        queue,
        state,
        floats,
        colors,
        main_tex_st,
    })
}

fn profile(row: &Value) -> Result<Profile, String> {
    let key = row
        .get("key")
        .and_then(Value::as_str)
        .ok_or("a colour profile has no key")?
        .to_owned();
    let fields = row
        .get("fields")
        .ok_or_else(|| format!("profile {key}: no fields"))?;
    let one = |name: &str, which: &str| {
        color(
            fields
                .get(name)
                .and_then(|f| f.get(which))
                .unwrap_or(&Value::Null),
            &format!("profile {key} {name}.{which}"),
        )
    };
    Ok(Profile {
        grid_line: one("GridLine", "color")?,
        dotted_grid_line: one("DottedGridLine", "color")?,
        moving_safe: one("MovingFixtureTileSafe", "color")?,
        moving_alert: one("MovingFixtureTileAlert", "color")?,
        placed: one("PlacedFixtureTile", "color")?,
        hided: (
            one("HidedFixtureTile", "color")?,
            one("HidedFixtureTile", "color2")?,
        ),
        motion_area: (
            one("MotionAreaTileMaterial", "color")?,
            one("MotionAreaTileMaterial", "color2")?,
        ),
        motion_disable: (
            one("MotionDisableAreaTileMaterial", "color")?,
            one("MotionDisableAreaTileMaterial", "color2")?,
        ),
        key,
    })
}

fn parse_grid(text: &str) -> Result<GridDoc, String> {
    let doc: Value = serde_json::from_str(text).map_err(|e| format!("not JSON: {e}"))?;
    let settings = doc.get("settings").ok_or("no settings")?;
    let setting = |name: &str| {
        number(
            settings.get(name).unwrap_or(&Value::Null),
            &format!("setting {name}"),
        )
    };
    let profiles = doc
        .get("colorProfiles")
        .and_then(Value::as_array)
        .ok_or("no colour profiles")?
        .iter()
        .map(profile)
        .collect::<Result<Vec<_>, String>>()?;
    if profiles.is_empty() {
        return Err("the colour profile table is empty".into());
    }
    Ok(GridDoc {
        vertical_line_size: setting("VerticleLineSize")?,
        horizontal_line_size: setting("HorizontalLineSize")?,
        position_y: setting("PositionY")?,
        show_line: setting("IsShowLine")? != 0.0,
        line: material_doc(&doc, "GridLine")?,
        dotted: material_doc(&doc, "DottedGridLine")?,
        uber: material_doc(&doc, "UberMaterial")?,
        profiles,
    })
}

/// The environment whose configuration names the colour profile.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Environment {
    phenomenon: String,
    site: String,
}

enum ProfileState {
    /// No environment seen yet, or its configuration is not resolved.
    None,
    Loading {
        environment: Environment,
        path: String,
        config: Handle<JsonAsset>,
    },
    /// The profile row and the key the configuration named.
    Read {
        environment: Environment,
        index: usize,
        named: String,
    },
    Failed {
        environment: Environment,
        error: String,
    },
}

/// The spawned grid.
struct Shown {
    solid: Option<Entity>,
    dotted: Option<Entity>,
    face: Entity,
    face_material: Handle<GridFaceMaterial>,
    line_materials: Vec<(Handle<GridLineMaterial>, bool)>,
    tiles: TileBox,
    /// The fill and colours last written: session revision, profile
    /// generation.
    written: Option<(u64, u64)>,
}

#[derive(Resource)]
pub(crate) struct EditGrid {
    grid: Handle<JsonAsset>,
    index: Handle<JsonAsset>,
    doc: Option<Result<GridDoc, String>>,
    phenomena: Option<Result<Value, String>>,
    profile: ProfileState,
    /// Bumped whenever the profile changes.
    profile_generation: u64,
    /// `ShowGrid` mode while shown.
    requested: Option<u8>,
    shown: Option<Shown>,
    waiting_logged: bool,
}

fn insert_shaders(mut shaders: ResMut<Assets<Shader>>) {
    let _ = shaders.insert(
        LINE_SHADER.id(),
        Shader::from_wgsl(
            include_str!("../shaders/edit_grid_line.wgsl"),
            "moly_game/src/shaders/edit_grid_line.wgsl".to_owned(),
        ),
    );
    let _ = shaders.insert(
        FACE_SHADER.id(),
        Shader::from_wgsl(
            include_str!("../shaders/edit_grid_face.wgsl"),
            "moly_game/src/shaders/edit_grid_face.wgsl".to_owned(),
        ),
    );
}

fn load(mut commands: Commands, server: Res<AssetServer>) {
    commands.insert_resource(EditGrid {
        grid: server.load::<JsonAsset>(AssetPath::from(GRID.to_owned())),
        index: server.load::<JsonAsset>(AssetPath::from(PHENOMENA.to_owned())),
        doc: None,
        phenomena: None,
        profile: ProfileState::None,
        profile_generation: 0,
        requested: None,
        shown: None,
        waiting_logged: false,
    });
}

fn json_text(
    world: &World,
    handle: &Handle<JsonAsset>,
    path: &str,
) -> Option<Result<String, String>> {
    match world.resource::<AssetServer>().load_state(handle) {
        LoadState::Loaded => Some(
            world
                .resource::<Assets<JsonAsset>>()
                .get(handle)
                .map(|asset| asset.0.clone())
                .ok_or_else(|| format!("{path}: loaded but not in the assets")),
        ),
        LoadState::Failed(error) => Some(Err(format!("{path} failed to load: {error}"))),
        _ => None,
    }
}

fn read_docs(world: &mut World, grid: &mut EditGrid) {
    if grid.doc.is_none() {
        if let Some(text) = json_text(world, &grid.grid, GRID) {
            let parsed = text.and_then(|text| parse_grid(&text));
            match &parsed {
                Ok(doc) => info!(
                    "[edit-grid] {GRID} read: VerticleLineSize {} HorizontalLineSize {} PositionY {} IsShowLine {}; {} {} (queue {}, state {:?}) _LineRepeat {:?} _LineFill {:?} _MainTex_ST {:?}; {} {} (queue {}, state {:?}) _LineRepeat {:?} _LineFill {:?}; {} {} (queue {}, state {:?}) _PatternRepeat {:?} _FillRate {:?}; {} colour profiles {:?}",
                    doc.vertical_line_size,
                    doc.horizontal_line_size,
                    doc.position_y,
                    doc.show_line,
                    doc.line.name,
                    doc.line.shader,
                    doc.line.queue,
                    doc.line.state,
                    doc.line.float("_LineRepeat"),
                    doc.line.float("_LineFill"),
                    doc.line.main_tex_st,
                    doc.dotted.name,
                    doc.dotted.shader,
                    doc.dotted.queue,
                    doc.dotted.state,
                    doc.dotted.float("_LineRepeat"),
                    doc.dotted.float("_LineFill"),
                    doc.uber.name,
                    doc.uber.shader,
                    doc.uber.queue,
                    doc.uber.state,
                    doc.uber.float("_PatternRepeat"),
                    doc.uber.float("_FillRate"),
                    doc.profiles.len(),
                    doc.profiles.iter().map(|p| p.key.as_str()).collect::<Vec<_>>()
                ),
                Err(error) => error!("[edit-grid] {GRID}: {error}; the layout grid is not drawn"),
            }
            grid.doc = Some(parsed);
        }
    }
    if grid.phenomena.is_none() {
        if let Some(text) = json_text(world, &grid.index, PHENOMENA) {
            let parsed = text.and_then(|text| {
                serde_json::from_str::<Value>(&text)
                    .map_err(|e| format!("{PHENOMENA}: not JSON: {e}"))
            });
            if let Err(error) = &parsed {
                error!("[edit-grid] {error}; the grid colour profile is not read");
            }
            grid.phenomena = Some(parsed);
        }
    }
}

/// `RefreshGridColor(CurrentSiteEnvironmentConfig)`: the committed
/// phenomenon's configuration at the environment site (the site's override
/// when the index lists one, else the phenomenon's own).
fn track_profile(world: &mut World, grid: &mut EditGrid) {
    let Some(Ok(index)) = &grid.phenomena else {
        return;
    };
    let Some(Ok(doc)) = &grid.doc else {
        return;
    };
    let (Some(phenomenon), Some(site)) = (
        world
            .get_resource::<crate::weather::CommittedPhenomenon>()
            .map(|p| p.0.clone()),
        world
            .get_resource::<crate::site::SiteActive>()
            .map(|s| s.env_site.clone()),
    ) else {
        return;
    };
    let environment = Environment { phenomenon, site };
    let current = match &grid.profile {
        ProfileState::None => None,
        ProfileState::Loading { environment, .. }
        | ProfileState::Read { environment, .. }
        | ProfileState::Failed { environment, .. } => Some(environment.clone()),
    };
    if current.as_ref() != Some(&environment) {
        let entry = index
            .get("phenomena")
            .and_then(|p| p.get(&environment.phenomenon));
        let path = entry.and_then(|entry| {
            entry
                .get("overrides")
                .and_then(|o| o.get(&environment.site))
                .and_then(|o| o.get("config"))
                .or_else(|| entry.get("config"))
                .and_then(Value::as_str)
        });
        grid.profile = match path {
            Some(path) => {
                let path = format!("moly://phenomena/{path}");
                let config = world
                    .resource::<AssetServer>()
                    .load::<JsonAsset>(AssetPath::from(path.clone()));
                ProfileState::Loading {
                    environment,
                    path,
                    config,
                }
            }
            None => {
                let error = format!(
                    "{PHENOMENA} lists no configuration for {} at {}",
                    environment.phenomenon, environment.site
                );
                error!("[edit-grid] RefreshGridColor: {error}; the grid keeps its colours");
                ProfileState::Failed { environment, error }
            }
        };
    }
    let ProfileState::Loading {
        environment,
        path,
        config,
    } = &grid.profile
    else {
        return;
    };
    let Some(text) = json_text(world, config, path) else {
        return;
    };
    let (environment, path) = (environment.clone(), path.clone());
    let key = text.and_then(|text| {
        serde_json::from_str::<Value>(&text)
            .map_err(|e| format!("{path}: not JSON: {e}"))?
            .get("gridColorKey")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| format!("{path} has no gridColorKey"))
    });
    grid.profile = match key {
        Ok(named) => {
            // `GetGridColorProfile`: the first row with the key, else row 0.
            let found = doc.profiles.iter().position(|p| p.key == named);
            let index = found.unwrap_or(0);
            info!(
                "[edit-grid] RefreshGridColor: {}@{} configuration {path} gridColorKey {named:?} -> profile {:?}{}",
                environment.phenomenon,
                environment.site,
                doc.profiles[index].key,
                if found.is_none() { " (no row has the key: the table's first row)" } else { "" }
            );
            grid.profile_generation += 1;
            ProfileState::Read {
                environment,
                index,
                named,
            }
        }
        Err(error) => {
            // The source logs and leaves the grid colour unchanged.
            error!("[edit-grid] RefreshGridColor: {error}; the grid keeps its colours");
            ProfileState::Failed { environment, error }
        }
    };
}

/// The source cell x of a product cell x, and back (the X mirror).
fn source_cell_x(product_x: i32) -> i32 {
    -product_x - 1
}

/// Positions (product frame), uv and indices of one quad given in the source
/// frame, wound to face +Y after the mirror.
fn push_quad(
    positions: &mut Vec<[f32; 3]>,
    uvs: &mut Vec<[f32; 2]>,
    indices: &mut Vec<u32>,
    corners: [(f32, f32); 4],
    uv: [[f32; 2]; 4],
) {
    let base = positions.len() as u32;
    let p: Vec<Vec3> = corners
        .iter()
        .map(|&(x, z)| Vec3::new(-x, 0.0, z))
        .collect();
    for (point, uv) in p.iter().zip(uv) {
        positions.push(point.to_array());
        uvs.push(uv);
    }
    // The source's triangles (0, 3, 1), (1, 3, 2); reversed when the mirror
    // turned them to face down.
    let normal = (p[3] - p[0]).cross(p[1] - p[0]);
    let order: [u32; 6] = if normal.y > 0.0 {
        [0, 3, 1, 1, 3, 2]
    } else {
        [0, 1, 3, 1, 2, 3]
    };
    indices.extend(order.iter().map(|i| base + i));
}

fn mesh(positions: Vec<[f32; 3]>, uvs: Vec<[f32; 2]>, indices: Vec<u32>) -> Mesh {
    let normals = vec![[0.0, 1.0, 0.0]; positions.len()];
    Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
    .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uvs)
    .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
    .with_inserted_indices(Indices::U32(indices))
}

/// The grid plane's corners (source frame): the minimum tile's minimum
/// corner and the maximum tile's maximum corner.
fn corners(tiles: &TileBox) -> ((f32, f32), (f32, f32)) {
    (
        (
            tiles.min[0] as f32 * TILE_SIZE,
            tiles.min[2] as f32 * TILE_SIZE,
        ),
        (
            (tiles.max[0] + 1) as f32 * TILE_SIZE,
            (tiles.max[2] + 1) as f32 * TILE_SIZE,
        ),
    )
}

struct LineMeshes {
    solid: Mesh,
    dotted: Mesh,
    vertical: usize,
    horizontal: usize,
    solid_lines: usize,
    dotted_lines: usize,
}

/// `GridLine.CalculateMesh` for the floor.
fn line_meshes(tiles: &TileBox, vertical_width: f32, horizontal_width: f32) -> LineMeshes {
    let ((min_x, min_z), (max_x, max_z)) = corners(tiles);
    let mut solid = (Vec::new(), Vec::new(), Vec::new());
    let mut dotted = (Vec::new(), Vec::new(), Vec::new());
    let (mut solid_lines, mut dotted_lines) = (0, 0);
    let vertical = (tiles.count_x + 1) as usize;
    let horizontal = (tiles.count_z + 1) as usize;
    let at = |i: usize, n: usize| (i as f32 / (n - 1) as f32).clamp(0.0, 1.0);
    for i in 0..vertical {
        let u = min_x + (max_x - min_x) * at(i, vertical);
        let (a, b) = (u - vertical_width / 2.0, u + vertical_width / 2.0);
        let target = if i % SOLID_INTERVAL == 0 {
            solid_lines += 1;
            &mut solid
        } else {
            dotted_lines += 1;
            &mut dotted
        };
        push_quad(
            &mut target.0,
            &mut target.1,
            &mut target.2,
            [(a, min_z), (b, min_z), (b, max_z), (a, max_z)],
            [[min_z, 0.0], [min_z, 0.0], [max_z, 0.0], [max_z, 0.0]],
        );
    }
    for j in 0..horizontal {
        let v = min_z + (max_z - min_z) * at(j, horizontal);
        let (a, b) = (v - horizontal_width / 2.0, v + horizontal_width / 2.0);
        let target = if j % SOLID_INTERVAL == 0 {
            solid_lines += 1;
            &mut solid
        } else {
            dotted_lines += 1;
            &mut dotted
        };
        push_quad(
            &mut target.0,
            &mut target.1,
            &mut target.2,
            [(min_x, a), (max_x, a), (max_x, b), (min_x, b)],
            [[min_x, 0.0], [max_x, 0.0], [max_x, 0.0], [min_x, 0.0]],
        );
    }
    LineMeshes {
        solid: mesh(solid.0, solid.1, solid.2),
        dotted: mesh(dotted.0, dotted.1, dotted.2),
        vertical,
        horizontal,
        solid_lines,
        dotted_lines,
    }
}

/// `Grid2DFaceUber.GetOrCreateMesh` for the floor.
fn face_mesh(tiles: &TileBox) -> Mesh {
    let ((min_x, min_z), (max_x, max_z)) = corners(tiles);
    let (cx, cz) = table_size(tiles);
    let (cx, cz) = (cx as f32, cz as f32);
    let (mut positions, mut uvs, mut indices) = (Vec::new(), Vec::new(), Vec::new());
    push_quad(
        &mut positions,
        &mut uvs,
        &mut indices,
        [
            (min_x, min_z),
            (max_x, min_z),
            (max_x, max_z),
            (min_x, max_z),
        ],
        [[0.0, 0.0], [cx, 0.0], [cx, cz], [0.0, cz]],
    );
    mesh(positions, uvs, indices)
}

/// The bound's tile counts (the state table's width and height).
fn table_size(tiles: &TileBox) -> (usize, usize) {
    (
        (tiles.max[0] - tiles.min[0] + 1) as usize,
        (tiles.max[2] - tiles.min[2] + 1) as usize,
    )
}

/// The fill of the floor grid's tile table and its packed texels.
struct Fill {
    states: Vec<u8>,
    texels: Vec<u8>,
    /// Packed n (0..=6) counts.
    counts: [usize; 7],
    motion_fixtures: usize,
    disabled_fixtures: usize,
    /// Fixture views in the floor layout, and their footprints' cell count
    /// (before clipping and overlap), for the FixtureExist count.
    floor_views: usize,
    footprint_cells: usize,
}

fn footprint_cells(item: &EditableFixture) -> Option<(GridPosition, GridPosition)> {
    item.footprint().ok()
}

/// `GridModel.SetFocus` for the floor (and the selection's highlight).
fn fill(
    tiles: &TileBox,
    rows: &[EditableFixture],
    selected: Option<&EditableFixture>,
    areas: &FixtureAreas,
) -> Fill {
    let (width, height) = table_size(tiles);
    let mut states = vec![0u8; width * height];
    // `Grid2DTileTable.AddState`: cells outside the table are skipped.
    let mut add = |sx: i32, sz: i32, flag: u8| {
        let (i, j) = (sx - tiles.min[0], sz - tiles.min[2]);
        if i >= 0 && j >= 0 && (i as usize) < width && (j as usize) < height {
            states[i as usize + width * j as usize] |= flag;
        }
    };
    let selected_uid = selected.map(|item| item.uid.as_str());
    // The fixtures' views: the selection is where it is being edited.
    let mut views: Vec<&EditableFixture> = rows
        .iter()
        .filter(|row| Some(row.uid.as_str()) != selected_uid)
        .collect();
    if let Some(item) = selected {
        if rows.iter().any(|row| row.uid == item.uid) {
            views.push(item);
        }
    }
    let (mut floor_views, mut footprint_total) = (0, 0);
    for view in &views {
        if view.layout & layout_type::FLOOR == 0 {
            continue;
        }
        floor_views += 1;
        if let Some((min, max)) = footprint_cells(view) {
            footprint_total += (i32::from(max.x) - i32::from(min.x) + 1) as usize
                * (i32::from(max.z) - i32::from(min.z) + 1) as usize;
            for x in i32::from(min.x)..=i32::from(max.x) {
                for z in i32::from(min.z)..=i32::from(max.z) {
                    add(source_cell_x(x), z, FIXTURE_EXIST);
                }
            }
        }
    }
    // The model's floor tiles (the decided draft), for the motion check.
    let occupied: Vec<(&str, GridPosition, GridPosition)> = rows
        .iter()
        .filter(|row| row.layout == layout_type::FLOOR)
        .filter_map(|row| footprint_cells(row).map(|(a, b)| (row.uid.as_str(), a, b)))
        .collect();
    let (mut motion_fixtures, mut disabled_fixtures) = (0, 0);
    for row in rows {
        if row.layout != layout_type::FLOOR || Some(row.uid.as_str()) == selected_uid {
            continue;
        }
        let Some(meta) = areas.motion.get(&row.package) else {
            continue;
        };
        let Ok((source_center, source_direction, _)) =
            moly_assets::player_data::mirror_fixture_layout(
                row.center,
                row.grid_size,
                row.direction,
                row.layout,
            )
        else {
            continue;
        };
        let bounds = motion_area_bounds(
            meta.rows,
            meta.cols,
            |r, c| meta.cell(r, c),
            source_center,
            row.grid_size,
            source_direction,
        );
        if bounds.is_empty() {
            continue;
        }
        motion_fixtures += 1;
        // `HasBoundBoxOtherFixture` per bound: every cell a tile and empty or
        // this fixture's.
        let clear = |cell: GridPosition| {
            let (sx, y, z) = (i32::from(cell.x), i32::from(cell.y), i32::from(cell.z));
            let tile = (tiles.min[0]..=tiles.max[0]).contains(&sx)
                && (tiles.min[1]..=tiles.max[1]).contains(&y)
                && (tiles.min[2]..=tiles.max[2]).contains(&z);
            let px = source_cell_x(sx);
            tile && !occupied.iter().any(|(uid, a, b)| {
                *uid != row.uid
                    && (i32::from(a.x)..=i32::from(b.x)).contains(&px)
                    && (i32::from(a.y)..=i32::from(b.y)).contains(&y)
                    && (i32::from(a.z)..=i32::from(b.z)).contains(&z)
            })
        };
        let blocked = bounds.iter().any(|bound| {
            (i32::from(bound.min.x)..=i32::from(bound.max.x)).any(|x| {
                (i32::from(bound.min.z)..=i32::from(bound.max.z))
                    .any(|z| !clear(GridPosition::new(x as i8, bound.min.y, z as i8)))
            })
        });
        let flag = if blocked {
            disabled_fixtures += 1;
            MOTION_DISABLE
        } else {
            MOTION_AREA
        };
        for bound in &bounds {
            for x in i32::from(bound.min.x)..=i32::from(bound.max.x) {
                for z in i32::from(bound.min.z)..=i32::from(bound.max.z) {
                    add(x, z, flag);
                }
            }
        }
    }
    if let Some(item) = selected.filter(|item| item.layout == layout_type::FLOOR) {
        if let Some((min, max)) = footprint_cells(item) {
            for x in i32::from(min.x)..=i32::from(max.x) {
                for z in i32::from(min.z)..=i32::from(max.z) {
                    add(source_cell_x(x), z, HIGHLIGHT);
                }
            }
        }
    }
    // `CalculateMotionConflictArea`.
    for state in &mut states {
        if *state & (HIGHLIGHT | FIXTURE_EXIST) != 0 && *state & (MOTION_AREA | MOTION_DISABLE) != 0
        {
            *state = MOTION_CONFLICT;
        }
    }
    let mut counts = [0usize; 7];
    let mut texels: Vec<u8> = Vec::with_capacity(states.len() * 4);
    for state in &states {
        // `PackGridFillState`: n = the first set state's bit index + 1.
        let n = STATE_ORDER
            .iter()
            .find(|flag| *state & **flag != 0)
            .map_or(0u8, |flag| flag.trailing_zeros() as u8 + 1);
        counts[n as usize] += 1;
        let bit = |b: u8| -> u8 {
            if n & b != 0 {
                255
            } else {
                0
            }
        };
        texels.extend([bit(4), bit(2), bit(1), 255]);
    }
    Fill {
        states,
        texels,
        counts,
        motion_fixtures,
        disabled_fixtures,
        floor_views,
        footprint_cells: footprint_total,
    }
}

fn state_image(tiles: &TileBox, texels: Vec<u8>) -> Image {
    let (width, height) = table_size(tiles);
    Image::new(
        Extent3d {
            width: width as u32,
            height: height as u32,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        texels,
        TextureFormat::Rgba8Unorm,
        RenderAssetUsages::default(),
    )
}

/// The colours the materials hold: the profile's, or the serialized ones.
struct Colours {
    line: Vec4,
    dotted: Vec4,
    face: FaceUniform,
    source: String,
}

fn colours(
    doc: &GridDoc,
    profile: &Result<Profile, String>,
    can_place: bool,
) -> Result<Colours, String> {
    let params = Vec4::new(
        TILE_SIZE,
        doc.uber.float("_PatternRepeat")?,
        doc.uber.float("_FillRate")?,
        0.0,
    );
    Ok(match profile {
        Ok(p) => Colours {
            line: p.grid_line,
            dotted: p.dotted_grid_line,
            face: FaceUniform {
                params,
                highlight: if can_place {
                    p.moving_safe
                } else {
                    p.moving_alert
                },
                fixture_exist: p.placed,
                hided: p.hided.0,
                hided2: p.hided.1,
                motion_area: p.motion_area.0,
                motion_area2: p.motion_area.1,
                motion_disable: p.motion_disable.0,
                motion_disable2: p.motion_disable.1,
                conflict: p.motion_disable.0,
                conflict2: p.motion_disable.0,
            },
            source: format!("profile {}", p.key),
        },
        Err(why) => {
            let u = &doc.uber;
            Colours {
                line: doc.line.color("_Color1")?,
                dotted: doc.dotted.color("_Color1")?,
                face: FaceUniform {
                    params,
                    highlight: u.color("_HighlightColor")?,
                    fixture_exist: u.color("_FixtureExistColor")?,
                    hided: u.color("_HidedFixtureColor")?,
                    hided2: u.color("_HidedFixtureColor2")?,
                    motion_area: u.color("_MotionAreaColor")?,
                    motion_area2: u.color("_MotionAreaColor2")?,
                    motion_disable: u.color("_MotionDisableAreaColor")?,
                    motion_disable2: u.color("_MotionDisableAreaColor2")?,
                    conflict: u.color("_MotionConflictArea")?,
                    conflict2: u.color("_MotionConflictArea2")?,
                },
                source: format!("the materials' serialized colours ({why})"),
            }
        }
    })
}

fn line_uniform(material: &MaterialDoc, color1: Vec4) -> Result<LineUniform, String> {
    Ok(LineUniform {
        main_tex_st: material.main_tex_st,
        color1,
        line: Vec4::new(
            material.float("_LineRepeat")?,
            material.float("_LineFill")?,
            0.0,
            0.0,
        ),
    })
}

/// `SetShowGrid(2)`: build the floor grid under the site.
fn spawn(world: &mut World, grid: &mut EditGrid) {
    let Some(mode) = grid.requested else {
        return;
    };
    if grid.shown.is_some() {
        return;
    }
    let waiting = |grid: &mut EditGrid, what: String| {
        if !grid.waiting_logged {
            grid.waiting_logged = true;
            warn!("[edit-grid] ShowGrid({mode}): {what}; the grid is built once it is there");
        }
    };
    let doc = match grid.doc.clone() {
        None => return waiting(grid, format!("{GRID} is still loading")),
        Some(Err(_)) => return,
        Some(Ok(doc)) => doc,
    };
    if mode != 2 {
        warn!("[edit-grid] ShowGrid({mode}): only the floor grid (2) is built; not drawn");
        grid.shown = None;
        return;
    }
    let Some(floor) = world
        .get_resource::<FixturePlacements>()
        .and_then(FixturePlacements::floor_grid)
    else {
        return waiting(grid, "the site's floor grid size is not known yet".into());
    };
    let Some(session) = world.get_resource::<EditSession>() else {
        return waiting(grid, "the edit session is not published yet".into());
    };
    let rows = session.rows.clone();
    let selected = session.selected.as_ref().map(|s| s.item.clone());
    let revision = session.revision;
    let tiles = TileBox::of(floor, layout_type::FLOOR);
    let origin = super::put_effect::site_origin(world);
    let offset = Vec3::new(0.0, 2.0 * doc.position_y, 0.0);
    let transform = Transform::from_translation(origin + offset);
    let profile = profile_of(grid, &doc);
    let can_place = can_place(world);
    let colours = match colours(&doc, &profile, can_place) {
        Ok(colours) => colours,
        Err(error) => {
            error!("[edit-grid] ShowGrid({mode}): {error}; not drawn");
            grid.doc = Some(Err(error));
            return;
        }
    };
    let (Ok(line_value), Ok(dotted_value)) = (
        line_uniform(&doc.line, colours.line),
        line_uniform(&doc.dotted, colours.dotted),
    ) else {
        error!("[edit-grid] ShowGrid({mode}): a line material lacks _LineRepeat or _LineFill; not drawn");
        return;
    };
    let areas = world.get_resource::<FixtureAreas>();
    let empty = FixtureAreas::default();
    let areas_loaded = areas.is_some_and(|a| a.loaded);
    let filled = fill(&tiles, &rows, selected.as_ref(), areas.unwrap_or(&empty));
    let lines = line_meshes(&tiles, doc.vertical_line_size, doc.horizontal_line_size);
    let (width, height) = table_size(&tiles);
    let ((min_x, min_z), (max_x, max_z)) = corners(&tiles);

    let image = world
        .resource_mut::<Assets<Image>>()
        .add(state_image(&tiles, filled.texels.clone()));
    let face_material = world
        .resource_mut::<Assets<GridFaceMaterial>>()
        .add(GridFaceMaterial {
            value: colours.face,
            state: image,
            key: GridPassKey {
                queue: doc.uber.queue,
                state: doc.uber.state,
            },
        });
    let face_handle = world.resource_mut::<Assets<Mesh>>().add(face_mesh(&tiles));
    let face = world
        .spawn((
            Name::new("EditGrid tile face"),
            EditGridPart,
            Mesh3d(face_handle),
            MeshMaterial3d(face_material.clone()),
            transform,
            Visibility::Visible,
        ))
        .id();
    let mut line_materials = Vec::new();
    let (mut solid, mut dotted) = (None, None);
    if doc.show_line {
        for (mesh, value, material, is_dotted) in [
            (lines.solid, line_value, &doc.line, false),
            (lines.dotted, dotted_value, &doc.dotted, true),
        ] {
            let handle = world
                .resource_mut::<Assets<GridLineMaterial>>()
                .add(GridLineMaterial {
                    value,
                    key: GridPassKey {
                        queue: material.queue,
                        state: material.state,
                    },
                });
            let mesh = world.resource_mut::<Assets<Mesh>>().add(mesh);
            let entity = world
                .spawn((
                    Name::new(if is_dotted {
                        "EditGrid dotted lines"
                    } else {
                        "EditGrid solid lines"
                    }),
                    EditGridPart,
                    Mesh3d(mesh),
                    MeshMaterial3d(handle.clone()),
                    transform,
                    Visibility::Visible,
                ))
                .id();
            line_materials.push((handle, is_dotted));
            if is_dotted {
                dotted = Some(entity);
            } else {
                solid = Some(entity);
            }
        }
    }
    info!(
        "[edit-grid] ShowGrid({mode}) (SetShowGrid floor): floor grid {}x{}x{} (level {}, layout {}); tile box x {}..={} z {}..={} (source frame, product cells x {}..={}); {} tiles = {}x{} of {TILE_SIZE} m; plane x {:.3}..{:.3} z {:.3}..{:.3} (product x {:.3}..{:.3}) at y {:.3} above the site origin {:.3}",
        floor.width, floor.height, floor.depth, floor.level, floor.layout_id,
        tiles.min[0], tiles.max[0], tiles.min[2], tiles.max[2],
        source_cell_x(tiles.max[0]), source_cell_x(tiles.min[0]),
        width * height, width, height,
        min_x, max_x, min_z, max_z, -max_x, -min_x,
        offset.y, origin
    );
    info!(
        "[edit-grid] lines: {} constant-x (width {}) + {} constant-z (width {}); {} solid (every {SOLID_INTERVAL}th, {}) and {} dotted ({}); IsShowLine {}; entities solid {:?} dotted {:?}",
        lines.vertical, doc.vertical_line_size, lines.horizontal, doc.horizontal_line_size,
        lines.solid_lines, doc.line.name, lines.dotted_lines, doc.dotted.name,
        doc.show_line, solid, dotted
    );
    info!(
        "[edit-grid] tile face: quad uv (0,0)..({width},{height}), state texture {width}x{height} RGBA8, {} ({}), entity {face:?}",
        doc.uber.name, doc.uber.shader
    );
    info!(
        "[edit-grid] colours from {}: GridLine _Color1 {:?}, DottedGridLine _Color1 {:?}; face _TileSize {} _PatternRepeat {} _FillRate {}, highlight {:?} ({}), fixture exist {:?}, hided {:?}/{:?}, motion area {:?}/{:?}, motion disable {:?}/{:?}, conflict {:?}/{:?}",
        colours.source, colours.line, colours.dotted,
        colours.face.params.x, colours.face.params.y, colours.face.params.z,
        colours.face.highlight, if can_place { "safe" } else { "alert" },
        colours.face.fixture_exist, colours.face.hided, colours.face.hided2,
        colours.face.motion_area, colours.face.motion_area2,
        colours.face.motion_disable, colours.face.motion_disable2,
        colours.face.conflict, colours.face.conflict2
    );
    log_fill(revision, &filled, areas_loaded);
    grid.shown = Some(Shown {
        solid,
        dotted,
        face,
        face_material,
        line_materials,
        tiles,
        written: Some((revision, grid.profile_generation)),
    });
    grid.waiting_logged = false;
}

fn log_fill(revision: u64, filled: &Fill, areas_loaded: bool) {
    let flagged = |flag: u8| filled.states.iter().filter(|s| **s & flag != 0).count();
    info!(
        "[edit-grid] fill (session revision {revision}): packed n counts [empty {}, highlight {}, fixture exist {}, hided {}, motion area {}, motion disable {}, conflict {}]; flags fixture exist {} motion area {} motion disable {} conflict {}; {} floor fixtures whose footprints cover {} cells; {} fixtures with motion areas ({} disabled){}",
        filled.counts[0], filled.counts[1], filled.counts[2], filled.counts[3],
        filled.counts[4], filled.counts[5], filled.counts[6],
        flagged(FIXTURE_EXIST), flagged(MOTION_AREA), flagged(MOTION_DISABLE), flagged(MOTION_CONFLICT),
        filled.floor_views, filled.footprint_cells,
        filled.motion_fixtures, filled.disabled_fixtures,
        if areas_loaded { "" } else { "; the fixture area table is not loaded (no motion areas)" }
    );
}

/// The profile the materials take, or why they keep their serialized
/// colours.
fn profile_of(grid: &EditGrid, doc: &GridDoc) -> Result<Profile, String> {
    match &grid.profile {
        ProfileState::Read {
            index,
            named,
            environment,
        } => doc.profiles.get(*index).cloned().ok_or_else(|| {
            format!(
                "{}@{} names {named:?}, row {index} is not in the table",
                environment.phenomenon, environment.site
            )
        }),
        ProfileState::Failed { error, .. } => Err(error.clone()),
        ProfileState::Loading { path, .. } => Err(format!("{path} is still loading")),
        ProfileState::None => Err("no site environment committed yet".into()),
    }
}

/// `CanPlaceFixture`: true on entry, else the selection's put check
/// (`SiteLayoutUtility.CanPutFloor`, the one `SetFocus` is given).
fn can_place(world: &World) -> bool {
    let Some(session) = world.get_resource::<EditSession>() else {
        return true;
    };
    session
        .selected
        .as_ref()
        .is_none_or(|selection| super::put_status(world, session, selection) == PutStatus::Ok)
}

/// Rewrite the fill and the colours when the session or the profile changed.
fn refresh(world: &mut World, grid: &mut EditGrid) {
    let Some(Ok(doc)) = grid.doc.clone() else {
        return;
    };
    let generation = grid.profile_generation;
    let profile = profile_of(grid, &doc);
    let Some(shown) = grid.shown.as_mut() else {
        return;
    };
    let Some(session) = world.get_resource::<EditSession>() else {
        return;
    };
    let revision = session.revision;
    if shown.written == Some((revision, generation)) {
        return;
    }
    let rows = session.rows.clone();
    let selected = session.selected.as_ref().map(|s| s.item.clone());
    if world
        .get_resource::<FixturePlacements>()
        .and_then(FixturePlacements::floor_grid)
        .is_none()
    {
        return;
    }
    let profile_changed = shown.written.is_none_or(|(_, g)| g != generation);
    shown.written = Some((revision, generation));
    let can_place = can_place(world);
    let Ok(colours) = colours(&doc, &profile, can_place) else {
        return;
    };
    let empty = FixtureAreas::default();
    let areas = world.get_resource::<FixtureAreas>();
    let areas_loaded = areas.is_some_and(|a| a.loaded);
    let filled = fill(
        &shown.tiles,
        &rows,
        selected.as_ref(),
        areas.unwrap_or(&empty),
    );
    let image = world
        .resource_mut::<Assets<Image>>()
        .add(state_image(&shown.tiles, filled.texels.clone()));
    if let Some(material) = world
        .resource_mut::<Assets<GridFaceMaterial>>()
        .get_mut(&shown.face_material)
    {
        material.state = image;
        material.value = colours.face;
    }
    for (handle, dotted) in &shown.line_materials {
        if let Some(material) = world
            .resource_mut::<Assets<GridLineMaterial>>()
            .get_mut(handle)
        {
            material.value.color1 = if *dotted {
                colours.dotted
            } else {
                colours.line
            };
        }
    }
    if profile_changed {
        info!(
            "[edit-grid] colours now from {}: GridLine _Color1 {:?}, DottedGridLine _Color1 {:?}, fixture exist {:?}",
            colours.source, colours.line, colours.dotted, colours.face.fixture_exist
        );
    }
    log_fill(revision, &filled, areas_loaded);
}

fn sync(world: &mut World) {
    let Some(mut grid) = world.remove_resource::<EditGrid>() else {
        return;
    };
    read_docs(world, &mut grid);
    track_profile(world, &mut grid);
    spawn(world, &mut grid);
    refresh(world, &mut grid);
    world.insert_resource(grid);
}

/// A grid entity (line or tile face).
#[derive(Component)]
pub(crate) struct EditGridPart;

/// `ShowGrid` (event 15) received.
pub(super) fn show(world: &mut World, mode: u8) {
    let Some(mut grid) = world.get_resource_mut::<EditGrid>() else {
        warn!("[edit-grid] ShowGrid({mode}): the grid host is not installed; not drawn");
        return;
    };
    // Built by the host after this frame's edit commands, once the session
    // it reads is back in the world.
    grid.requested = Some(mode);
    grid.waiting_logged = false;
}

/// `HideGrid` (event 16) received: `SetHide` on the grid controller.
pub(super) fn hide(world: &mut World) {
    let Some(mut grid) = world.remove_resource::<EditGrid>() else {
        return;
    };
    if grid.requested.is_none() && grid.shown.is_none() {
        world.insert_resource(grid);
        return;
    }
    grid.requested = None;
    let mut removed = 0;
    if let Some(shown) = grid.shown.take() {
        for entity in [shown.solid, shown.dotted, Some(shown.face)]
            .into_iter()
            .flatten()
        {
            if world.despawn(entity) {
                removed += 1;
            }
        }
    }
    world.insert_resource(grid);
    let mut parts = world.query_filtered::<Entity, With<EditGridPart>>();
    let left = parts.iter(world).count();
    info!("[edit-grid] HideGrid: {removed} grid entities despawned; {left} remain");
}

/// Instrument (`MOLY_EDIT_AUTOPLAY`): the grid entity count whenever it
/// changes, after the frame's commands.
fn sample(parts: Query<Entity, With<EditGridPart>>, mut last: Local<Option<usize>>) {
    if std::env::var("MOLY_EDIT_AUTOPLAY").is_err() {
        return;
    }
    let count = parts.iter().count();
    if *last != Some(count) {
        *last = Some(count);
        info!("[edit-grid] sample: {count} grid entities exist");
    }
}

pub(super) fn install(app: &mut App) {
    app.add_plugins(MaterialPlugin::<GridLineMaterial>::default())
        .add_plugins(MaterialPlugin::<GridFaceMaterial>::default())
        .add_systems(Startup, (insert_shaders, load))
        .add_systems(Update, sync.after(FixtureEditSystems::View))
        .add_systems(PostUpdate, sample);
}
