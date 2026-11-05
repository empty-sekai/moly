//! Main-light shadow depth map: the producer side of the single-map route (no
//! cascades, no atlas).
//!
//! One light-facing orthographic depth map (one matrix for the whole map, no
//! cascades, no atlas); a node built inside the Core3d graph draws every caster
//! directly before the main pass. The consumers are the site materials and
//! the room shell material (the same lookup in `shaders/site_material.wesl`
//! and `shaders/room_shell.wesl`; group 3 bindings 10/11/12 for the site, 4/5/6
//! for the room). Node and draw pattern: the render graph node style of
//! weather.rs plus the manual MeshAllocator draw of bevy_pbr volumetric_fog
//! (checked one by one against the 0.20 registry).
//!
//! # Settings
//!
//! Everything that shapes the map or the lookup is a runtime setting
//! (`shadow_settings.rs`): whether there is a map, its side in texels, the
//! frame (a sphere around the camera's frustum slice, or around every caster of
//! the site), the filter, the kernel radii, the sun's angular diameter, the
//! strength and the caster bias multipliers. The settings reach this module as
//! the render world's [`ShadowSettingsGlobals`], extracted on the frame they
//! change. A change of the resolution re-creates the depth maps during that
//! extraction (`apply_shadow_resolution`); on the same frame the main world
//! marks every material that binds the maps changed
//! ([`mark_materials_on_resolution`]), so their bind groups, prepared after
//! extraction, hold the new views.
//!
//! # Caster set
//!
//! Every entity in the main world that has [`Mesh3d`] and is hierarchy-enabled,
//! except: the sky shell (a huge camera-following sphere that is a full circle
//! of occluders seen from the light; it is kept in the list and marked, so the
//! light frame is derived from every caster while it has not settled, and the
//! prepare leaves it out of a camera frame), emote draw sites (world-space
//! billboards rebuilt every frame whose vertex attributes are world
//! coordinates and carry no projection semantics), and rain particles
//! ([`NoShadowCast`]: particles are not in the main light shadow map).
//!
//! Skinned meshes take a second route. A skinned mesh casts only when it is a
//! character renderer: an NPC mesh with the character material or a mesh of
//! the player's body with the avatar material. It is drawn with the joint
//! matrices of the current frame (each joint's world transform times its
//! inverse bind pose, the same palette the colour pass skins with), so the
//! shadow follows the playing animation. The rules are the source's:
//! - the player's body renderer comes from the avatar prefab, whose
//!   serialized cast-shadows mode is On, and nothing at runtime changes it;
//! - every NPC renderer is set On each frame, except Off while the NPC's
//!   dither alpha is exactly 0 (read here from the material the dither writes);
//! - the NPC accessory program declares no ShadowCaster pass, so its meshes
//!   carry [`NoShadowCast`];
//! - the character ShadowCaster programs cull nothing (their cull state is
//!   bound to a property no character material or global sets, which leaves it
//!   Off), move each vertex by the pipeline's caster bias along the light and
//!   along the normal, clamp to the near plane, and are drawn under the
//!   pipeline's global raster bias of the shadow slice (see
//!   `SHADOW_SLICE_RASTER_BIAS`).
//!
//! Other skinned meshes (skinned fixtures) stay out and are counted.
//!
//! The rigid casters carry the same source bias: every site program's
//! ShadowCaster pass (field object, tree, ground, water, fixture, object,
//! fence, canvas) runs the same vertex bias along the light and along the
//! normal, with the same near-plane clamp, under the same raster bias of the
//! shadow slice. A rigid mesh without a normal stream gets the bias along
//! the light only, and is counted in the account line.
//!
//! # Light frame
//!
//! The light view looks at the frame's centre from the light's direction
//! (the same axes whatever the fit: right = up × towards the light). With
//! [`ShadowFit::Site`] the frame is the bounding box of every caster's
//! world-space corners and the orthographic box its light-space extent
//! (square, centred). With [`ShadowFit::Camera`] the frame is the bounding
//! sphere of the camera's frustum slice, from its near plane to the settings'
//! distance, of the main 3D camera (the camera the site globals follow); the
//! orthographic box is the sphere's square, and its centre is snapped to whole
//! texels in the light's plane, so camera motion moves the sampled grid by
//! whole texels only. The depth range is, in both cases, the light-space
//! extent of every caster along the light plus `DEPTH_PAD` at both ends:
//! casters outside the view still cast into it. The sphere is never smaller
//! than `MIN_FRAME_RADIUS`.
//!
//! From one frame's frame the extraction derives the planes a draw is tested
//! against (module comment on `LightPlane`): a draw inside them cannot reach
//! the map, so it keeps its static-map place and pipeline. A draw outside them
//! is queued again, and a [`ShadowFit::Camera`] frame also leaves out a draw
//! that lies wholly behind the camera or beyond the fade.
//!
//! # Numeric values
//!
//! Sites run on the site pipeline asset among the engine resources (the one
//! installed when the game switches into a site, not the global default
//! pipeline asset, which disables main-light rendering entirely). The values
//! read from the engine's serialized settings: the pipeline asset's depth and
//! normal bias 1.0/1.0, both scaled here by texel × soft-shadow kernel 2.5
//! (vertex side, along the light and along the normal), and the shadow slice's
//! global raster bias 1 / 2.5 (`SHADOW_SLICE_RASTER_BIAS`). The caster bias is
//! taken in texels, so a finer map biases less in metres. The source's own
//! map (1024², shadow distance 25 m, margin 0.1) and the frame it fitted are
//! not reproduced: this module draws a map sized by the settings and framed as
//! the settings ask, and the distance fade follows the settings too.
//!
//! # Shadow account
//!
//! Four failure states look the same on screen (no shadow and no wrong
//! shadow): the pass did not run, it ran but drew nothing, the map holds no
//! content, or the map has content but the projection does not line up. One
//! account line separates them: the node only receives `&World`, so counting
//! goes through the interior mutability of `ShadowAccount`. The depth map is
//! read back periodically (on the native side; the wasm build has no blocking
//! poll and only keeps the counts), and coverage, caster origin alignment and
//! single-texel comparison hits are logged side by side so every value can be
//! recomputed during acceptance.
//!
//! # In-frame sequence
//!
//! Extract pulls the casters (with precomputed bounds for main-world meshes)
//! → PrepareResources re-creates the maps when the resolution changed, resolves
//! the light's orthographic frame, writes three uniform blocks and specializes
//! pipelines by vertex layout → the node runs the depth pass between
//! EndPrepasses and StartMainPass (a frame whose pipelines are not compiled yet,
//! whose caster set is empty, or where the shadows are switched off is skipped
//! entirely).
//!
//! # Static map
//!
//! A rigid caster whose mesh, world matrix and winding stayed the same for
//! `STATIC_AFTER_FRAMES` extractions is baked: drawn once into a second
//! depth map of the same size and state, which every frame's pass copies
//! texel for texel before it draws the other rigid casters and the skinned
//! ones. The map is drawn again whenever its content would differ: a baked
//! caster changed, left the list or had its mesh modified, the light frame
//! block changed, the resolution changed (the maps are new), or another caster
//! became stable. The depth test keeps the nearest depth per texel either way,
//! so the frame's map holds the same values as one pass over every caster.

use std::collections::HashMap;
use std::marker::PhantomData;
use std::sync::Mutex;

use bevy::asset::uuid::Uuid;
use bevy::core_pipeline::{Core3d, Core3dSystems};
use bevy::mesh::skinning::{SkinnedMesh, SkinnedMeshInverseBindposes};
use bevy::mesh::{Mesh, MeshVertexBufferLayoutRef};
use bevy::prelude::*;
use bevy::render::mesh::allocator::MeshAllocator;
use bevy::render::mesh::{RenderMesh, RenderMeshBufferInfo};
use bevy::render::render_asset::{ExtractedAssets, RenderAssets};
use bevy::render::render_resource::binding_types::{uniform_buffer, uniform_buffer_sized};
use bevy::render::render_resource::*;
use bevy::render::renderer::{RenderContext, RenderDevice, RenderQueue};
use bevy::render::{Extract, ExtractSchedule, Render, RenderApp, RenderStartup, RenderSystems};
use bevy::shape::Aabb3d;

use crate::character_material::CharacterMaterial;
use crate::env::SiteEnv;
use crate::fixture_material::WallLayoutShadowCasterOff;
use crate::markers::EmoteDraw;
use crate::render::gpu::{Bound, SharedBindGroupCache};
use crate::shadow_settings::{
    ShadowFilter, ShadowFit, ShadowSettings, ShadowSettingsGlobals, ShadowSettingsPlugin,
};
use crate::site_material::SiteMaterial;
use crate::sky::SkyDome;
use moly_assets::material_passes::SourceMaterialPasses;
use moly_assets::scene_state::{SourceRenderer, SourceShadowCastingMode};

/// The depth pass's vertex program, inserted by [`load`] under this id (the
/// repository has no default asset source directory, so the handle is made
/// here, as weather.rs does it).
const SHADOW_DEPTH_SHADER: Handle<Shader> = Handle::Uuid(
    Uuid::from_u128(0x6271_62d3_a2e2_4f19_b6c6_9f3e_51c8_0d44),
    PhantomData,
);

/// The program that copies the static map into the frame's map.
const SHADOW_COPY_SHADER: Handle<Shader> = Handle::Uuid(
    Uuid::from_u128(0x6271_62d3_a2e2_4f19_b6c6_9f3e_51c8_0d45),
    PhantomData,
);

/// Extractions a rigid caster must stay unchanged before it is baked into
/// the static map (module comment, "Static map").
const STATIC_AFTER_FRAMES: u32 = 30;

/// Written prepares of a settings run before the readiness line is reported.
const READY_FRAMES: u64 = 2;

/// Extractions that pass under a set of settings before the rigid casters a
/// camera frame never shows are left out of the list (a camera report and its
/// slice take a few frames to describe the view, so the first ones leave
/// every caster in; see [`uses_camera_frame`]).
const CASTER_HOLD_FRAMES: u32 = 120;

/// The smallest radius of the camera frame, in metres (the settings' 30 m
/// default sits well above it). With [`ShadowFit::Camera`] the map is as fine
/// as the view it covers: its texel is `2 × radius / resolution` wide (about
/// 1.9 cm at 2560² and a 38.5 m sphere, the default 30 m distance at 60°). With
/// [`ShadowFit::Site`] a texel is the site's width over the resolution.
const MIN_FRAME_RADIUS: f32 = 1.0;

/// How far behind the near plane the camera frame keeps its volume (world
/// units): between the camera and the near plane nothing is framed. The
/// source's light carries a shadow near plane of 0.2, which serves its own
/// cascade fit; the same figure pads the frustum slice here.
const CAMERA_FRAME_BEHIND: f32 = 0.2;

/// Depth safety padding of the light's orthographic frame (world units): the
/// depth range extends by this amount at both ends so casters at the edge are
/// not clipped.
const DEPTH_PAD: f32 = 1.0;

/// The distance fade: from 90% of the settings' distance the shadow fades out
/// and is gone at the distance itself.
const FADE_START: f32 = 0.9;

/// The shadow parameters the consumer block's second slot carries (the
/// `_MainLightShadowParams` shape): the main light intensity (1) and the soft
/// shadow flag. Neither is a lane the lookup reads; they carry the fixed
/// values of the site's main light so a line can be recomputed.
const SHADOW_MAIN_LIGHT_INTENSITY: f32 = 1.0;
const SHADOW_SOFT: f32 = 1.0;

/// The shadow account: the node's first read-back waits until the 60th
/// rendered frame (scene assembly and pipeline compilation have settled).
const PROBE_FIRST_FRAME: u64 = 60;
/// The browser build's interval between accounts: 600 frames (about 10 s).
#[cfg(target_arch = "wasm32")]
const PROBE_INTERVAL: u64 = 600;

/// The frames between accounts. The native depth-map read-back waits for the
/// GPU on the render thread, so it runs only for a diagnosis run that sets
/// `dev.shadow_probe_interval` (`MOLY_SHADOW_PROBE_INTERVAL`) (frames, at
/// least 1); read once. The browser build only logs a counter line and keeps
/// the default interval.
fn probe_interval() -> Option<u64> {
    #[cfg(target_arch = "wasm32")]
    {
        Some(PROBE_INTERVAL)
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        static INTERVAL: std::sync::OnceLock<Option<u64>> = std::sync::OnceLock::new();
        *INTERVAL.get_or_init(|| {
            moly_config::text(&moly_config::keys::DEV_SHADOW_PROBE_INTERVAL)
                .and_then(|value| value.parse::<u64>().ok())
                .filter(|value| *value > 0)
        })
    }
}

/// The threshold at which a read-back texel counts as holding a depth (the
/// cleared map is 1.0, and rasterised depth is only smaller). Used by the
/// native read-back only (the browser build logs counts, not the map).
#[cfg(not(target_arch = "wasm32"))]
const PROBE_CLEAR_EPS: f32 = 0.9999;

/// The byte size of the depth pass's frame block: two 4×4 matrices (the clip
/// matrix and the sampling matrix) and two vec4 (the caster bias and the
/// direction towards the light).
const FRAME_BYTES: usize = 160;

/// The site pipeline asset's shadow depth bias and normal bias (both 1.0).
/// The main light's caster bias takes them when the light defers to the
/// pipeline settings, which is the light data's default; the site light's own
/// light data is not read here.
const PIPELINE_SHADOW_DEPTH_BIAS: f32 = 1.0;
const PIPELINE_SHADOW_NORMAL_BIAS: f32 = 1.0;

/// The soft-shadow kernel radius the pipeline scales both caster biases by
/// when soft shadows are supported and the light is soft: 2.5 for the medium
/// quality, which is also the value when the light defers to the pipeline.
const SOFT_SHADOW_KERNEL_RADIUS: f32 = 2.5;

/// The raster bias the pipeline sets globally around the shadow slice's draw
/// (units 1.0, slope 2.5). No ShadowCaster pass of the site or character
/// programs declares an offset of its own, so this is every caster's whole
/// raster bias.
const SHADOW_SLICE_RASTER_BIAS: DepthBiasState = DepthBiasState {
    constant: 1,
    slope_scale: 2.5,
    clamp: 0.0,
};

/// Joints one palette window holds (the engine's skinning limit), and the
/// window's byte size: the dynamic uniform binding of the skinned entry point.
pub const MAX_JOINTS: usize = 256;
pub const PALETTE_WINDOW: u64 = (MAX_JOINTS * 64) as u64;

/// Palettes start at this alignment (the dynamic-offset alignment that every
/// device supports).
pub const PALETTE_ALIGN: usize = 256;

/// The consumer block's byte size: the matrix (64), then five vec4s: the map
/// size, the shadow parameters and the three filter vectors of
/// [`ShadowConsumer`]. The `SiteShadow` struct of `site_material.wesl` and
/// `room_shell.wesl` declares the same fields in the same order.
const CONSUMER_BYTES: usize = 64 + 5 * 16;

/// The object matrix pool's capacity; casters beyond it are dropped and
/// warned about once (a site holds far fewer, so this is only a guard).
const OBJECT_CAPACITY: usize = 8192;

/// The frame's depth map exceeds a mesh's vertex budget on the smallest GPUs
/// only at the top of [`crate::shadow_settings::RESOLUTION_MAX`]; the maps are
/// square and at most 8192 texels on a side.
///
/// The marker for entities that are not in the main light depth map.
#[derive(Component)]
pub struct NoShadowCast;

/// The camera fit's frustum slice of one render camera, written by
/// [`fit_camera_frame`] in the render world's `Prepare` and read by
/// [`prepare_shadow_draws`] in the same frame's `PrepareResources`: the
/// frustum corners at the near plane and at the settings' distance
/// ([`ShadowFit::Camera`]). A camera whose pose and projection did not change
/// since the last frame keeps its corners.
#[derive(Component, Clone, Copy)]
struct ShadowCameraSlice {
    near: [Vec3; 4],
    far: [Vec3; 4],
}

/// The depth pass's vertex block: its slot order is the contract with
/// `ShadowFrame` in `shaders/shadow_depth.wgsl`, and both change together.
/// Matrices are laid out column by column (the memory order of a WGSL
/// mat4x4).
#[derive(ShaderType)]
struct ShadowFrame {
    /// The depth pass's clip matrix (orthographic, z from near 0 to far 1).
    light_view_proj: [Vec4; 4],
    /// The sampling matrix: xy is the map's uv (y flipped), z the depth
    /// compared.
    world_to_shadow: [Vec4; 4],
    /// The caster bias in world units: x along the light, y along the normal
    /// (both negative, as the pipeline computes them).
    shadow_bias: Vec4,
    /// xyz: the direction towards the light.
    light_direction: Vec4,
}

fn mat4_cols(matrix: Mat4) -> [Vec4; 4] {
    let cols = matrix.to_cols_array();
    [
        Vec4::from_array(cols[0..4].try_into().unwrap()),
        Vec4::from_array(cols[4..8].try_into().unwrap()),
        Vec4::from_array(cols[8..12].try_into().unwrap()),
        Vec4::from_array(cols[12..16].try_into().unwrap()),
    ]
}

fn mat4_bytes(matrix: Mat4, bytes: &mut Vec<u8>) {
    for component in matrix.to_cols_array() {
        bytes.extend_from_slice(&component.to_le_bytes());
    }
}

/// The consumer block's Rust form: its slot order is the contract with
/// `SiteShadow` in `shaders/site_material.wesl` and `shaders/room_shell.wesl`,
/// and both change together. The flat order is the matrix's four columns, the
/// size, the parameters, then the filter constants.
struct ShadowConsumer {
    world_to_shadow: Mat4,
    /// (width, height, 1/width, 1/height).
    size: [f32; 4],
    /// `_MainLightShadowParams` shape: .x main light intensity, .y soft
    /// shadows. An intensity of 0 reads as a fully lit receiver, which is how
    /// a frame without a map is written.
    params: [f32; 4],
    /// `(filter, fade start m, fade end m, strength)`; the filter is 0 hard,
    /// 1 tent, 2 Pcf, 3 Pcss.
    filtering: Vec4,
    /// `(Pcf radius uv, Pcss light size uv per blocker metre, largest radius
    /// uv, 0)`.
    kernel: Vec4,
    /// `(texel uv, 0, 0, 0)`.
    depth: Vec4,
}

impl ShadowConsumer {
    fn bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(CONSUMER_BYTES);
        mat4_bytes(self.world_to_shadow, &mut bytes);
        for slot in [self.size, self.params] {
            for component in slot {
                bytes.extend_from_slice(&component.to_le_bytes());
            }
        }
        for slot in [self.filtering, self.kernel, self.depth] {
            for component in slot.to_array() {
                bytes.extend_from_slice(&component.to_le_bytes());
            }
        }
        debug_assert_eq!(bytes.len(), CONSUMER_BYTES);
        bytes
    }

    /// The block of a frame with no usable light frame: no matrix and no
    /// size, intensity 0, so the lookup reads a depth outside [0, 1) and the
    /// receiver is fully lit.
    fn unlit() -> Self {
        Self {
            world_to_shadow: Mat4::ZERO,
            size: [0.0; 4],
            params: [0.0; 4],
            filtering: Vec4::ZERO,
            kernel: Vec4::ZERO,
            depth: Vec4::ZERO,
        }
    }
}

/// The render world's shared shadow resources: built once, then only the
/// buffers are written each frame. The depth maps are re-created when the
/// settings' resolution changes (see `rebuild_shadow_maps`).
#[derive(Resource)]
pub struct ShadowMapGpu {
    /// The side of the depth maps, in texels (the settings' resolution the
    /// maps of `depth_texture`, `static_view` and the read-back were created
    /// for).
    size: u32,
    /// The depth map itself: the source of the read-back (the account measures
    /// coverage from it).
    pub depth_texture: Texture,
    /// The depth map's view: the depth pass's attachment and the site and
    /// room materials' sampling source.
    pub depth_view: TextureView,
    /// The comparison sampler: a bilinear compare, each tap compares the four
    /// texels around its point and interpolates by the fraction between them;
    /// `LessEqual` = a stored depth at or behind the fragment is lit (the
    /// map's z grows with distance).
    pub cmp_sampler: Sampler,
    /// How many prepares the depth maps of `depth_texture` have served.
    maps_frames: u32,
    /// The consumer block's buffer: one for every material, bound at group 3
    /// binding 10 (the room shell's binding 4).
    pub consumer_buffer: Buffer,
    /// The depth pass's frame block.
    frame_buffer: Buffer,
    /// The per-caster world matrix pool: a dynamic-offset binding with the
    /// slot size of `object_stride`.
    object_buffer: Buffer,
    /// The depth pipelines' group 0 layout (descriptors are cached by the
    /// `PipelineCache`).
    depth_layout: BindGroupLayoutDescriptor,
    /// The skinned entry point's group 1 layout: one palette window, bound at
    /// a dynamic offset.
    skin_layout: BindGroupLayoutDescriptor,
    /// The static map (module comment, "Static map"): same size and format as
    /// the frame's map, drawn as an attachment and read by the copy.
    static_view: TextureView,
    /// The copy's layout (the static map as a depth texture and a nearest,
    /// non-comparison sampler), its sampler and its pipeline.
    copy_layout: BindGroupLayoutDescriptor,
    copy_sampler: Sampler,
    copy_pipeline: CachedRenderPipelineId,
    /// The dynamic-offset slot stride, aligned by the device, at least 64
    /// bytes.
    object_stride: u32,
    /// One matrix's binding size (for the dynamic-offset check).
    object_binding_size: u64,
}

impl ShadowMapGpu {
    /// Whether a camera frame that never shows a caster may leave it out.
    /// Nothing is left out until [`CASTER_HOLD_FRAMES`] prepares after the
    /// maps were rebuilt: the camera frame of the first frames may describe a
    /// view that is still being set up.
    fn frame_has_settled(&self) -> bool {
        self.maps_frames >= CASTER_HOLD_FRAMES
    }
}

/// One depth draw: a mesh handle, a world matrix and the pipeline specialised
/// for that mesh's layout.
#[derive(Clone, Copy)]
struct ShadowDraw {
    mesh: AssetId<Mesh>,
    world: Mat4,
    /// The world box of the mesh's bind-pose box, as the light frame and the
    /// account measure it (the eight corners).
    corners: [Vec3; 8],
    pipeline: CachedRenderPipelineId,
    two_sided: bool,
    /// The sky shell: drawn into the map only while the frame is derived
    /// from every caster (a camera frame around the view would have it cover
    /// the whole map).
    sky_shell: bool,
    /// Consecutive extractions this draw stayed the same.
    stable: u32,
    /// The draw is in the static map, not drawn per frame.
    baked: bool,
}

/// One skinned depth draw: a character renderer and the byte offset of its
/// joint palette (this frame's) in the palette buffer.
struct SkinnedShadowDraw {
    mesh: AssetId<Mesh>,
    palette_offset: u32,
    pipeline: CachedRenderPipelineId,
    /// Whether the renderer is a mesh of the player's body.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    player: bool,
    /// World bounds of the skinned vertices: the mesh's bind-pose box carried
    /// by every joint matrix of its palette (a superset of every blend). Read
    /// by the read-back's player window only.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    bounds: (Vec3, Vec3),
}

/// Skinned-caster tallies of one extraction, for the account line.
#[derive(Default, Clone, Copy)]
struct SkinnedTally {
    player: usize,
    npc: usize,
    /// NPC renderers set Off because the NPC's dither alpha is 0.
    npc_off_by_dither: usize,
    /// Skinned meshes that are no character renderer (not casting).
    other_skinned: usize,
    palettes: usize,
    joints: usize,
}

/// The light-space planes of one frame's frame: a rigid caster inside them
/// cannot reach the map, so its place and pipeline are kept; a caster outside
/// them is queued again (and a camera frame also leaves out the casters that
/// are wholly behind it). Evaluation works in light space, where a box is an
/// interval per axis.
#[derive(Clone, Copy, PartialEq)]
struct LightPlane {
    /// The direction towards the light (the light view's axes follow it).
    to_light: Vec3,
    /// The cross section of the map, along the light's right and up axes.
    cross: Spans,
    /// The depth range of the map, along the light: [min, max] of the
    /// distance towards the light.
    depth: Span,
}

impl Default for LightPlane {
    /// No plane holds anything: every caster is queued again.
    fn default() -> Self {
        Self {
            to_light: Vec3::Y,
            cross: Spans::NONE,
            depth: Span::NONE,
        }
    }
}

impl LightPlane {
    /// A plane along `to_light` that holds nothing.
    fn none(to_light: Vec3) -> Self {
        Self {
            to_light,
            ..Self::default()
        }
    }

    /// The light view of a world point.
    fn light_space(&self, point: Vec3) -> LightPoint {
        let (right, up, towards) = light_axes(self.to_light);
        LightPoint {
            across: Vec2::new(point.dot(right), point.dot(up)),
            depth: point.dot(towards),
        }
    }

    /// The light-space box of a box given its world corners.
    fn boxed(&self, corners: &[Vec3; 8]) -> Box3 {
        let mut boxed = Box3::NONE;
        for corner in corners {
            boxed.grow(self.light_space(*corner));
        }
        boxed
    }

    /// Whether the box is inside the cross section and the depth range: then
    /// no part of it leaves the map.
    fn holds(&self, corners: &[Vec3; 8]) -> bool {
        let boxed = self.boxed(corners);
        boxed.is_set()
            && boxed.across.min.cmpge(self.cross.min).all()
            && boxed.across.max.cmple(self.cross.max).all()
            && boxed.depth.min >= self.depth.min
            && boxed.depth.max <= self.depth.max
    }

    /// Whether the box lies wholly beyond one side of the map: beyond the
    /// cross section across, or behind the nearest depth (a receiver inside
    /// the map cannot stand behind it, so neither it nor its shadow reaches
    /// the map).
    fn outside(&self, corners: &[Vec3; 8]) -> bool {
        let boxed = self.boxed(corners);
        if !boxed.is_set() {
            return false;
        }
        boxed.across.max.cmplt(self.cross.min).any()
            || boxed.across.min.cmpgt(self.cross.max).any()
            || boxed.depth.max < self.depth.min
    }
}

/// A point in the light view.
struct LightPoint {
    /// Light x and y.
    across: Vec2,
    /// The distance along the direction towards the light.
    depth: f32,
}

/// A box in the light view.
#[derive(Clone, Copy, Default)]
struct Box3 {
    across: Spans,
    depth: Span,
}

impl Box3 {
    const NONE: Self = Self {
        across: Spans::NONE,
        depth: Span::NONE,
    };

    /// Whether the box holds a point.
    fn is_set(&self) -> bool {
        self.across.is_set() && self.depth.is_set()
    }

    /// The box grown to hold `point`.
    fn grow(&mut self, point: LightPoint) {
        self.across.grow(point.across);
        self.depth.grow(point.depth);
    }
}

/// An interval that starts empty: the first value it takes sets it.
#[derive(Clone, Copy, Default, PartialEq)]
struct Span {
    min: f32,
    max: f32,
}

impl Span {
    const NONE: Self = Self {
        min: f32::INFINITY,
        max: f32::NEG_INFINITY,
    };

    /// Whether the interval holds a value.
    fn is_set(&self) -> bool {
        self.min <= self.max
    }

    /// The interval grown to hold `value`.
    fn grow(&mut self, value: f32) {
        if !value.is_finite() {
            return;
        }
        self.min = self.min.min(value);
        self.max = self.max.max(value);
    }
}

/// An interval in the light's x and y, starting empty.
#[derive(Clone, Copy, Default, PartialEq)]
struct Spans {
    min: Vec2,
    max: Vec2,
}

impl Spans {
    const NONE: Self = Self {
        min: Vec2::splat(f32::INFINITY),
        max: Vec2::splat(f32::NEG_INFINITY),
    };

    /// Whether both axes hold a value.
    fn is_set(&self) -> bool {
        self.min.cmple(self.max).all()
    }

    /// The interval grown to hold `point`.
    fn grow(&mut self, point: Vec2) {
        if !point.is_finite() {
            return;
        }
        self.min = self.min.min(point);
        self.max = self.max.max(point);
    }

    /// The interval grown to hold `other`.
    fn merge(&mut self, other: Self) {
        if other.is_set() {
            self.grow(other.min);
            self.grow(other.max);
        }
    }
}

/// The axes of a light view looking along `to_light`: right, up and the
/// direction towards the light. The up axis turns to Z for a light close to
/// straight above so that the axes do not degenerate.
fn light_axes(to_light: Vec3) -> (Vec3, Vec3, Vec3) {
    let up = if to_light.y.abs() > 0.99 { Vec3::Z } else { Vec3::Y };
    let right = up.cross(to_light).normalize();
    let up = to_light.cross(right);
    (right, up, to_light)
}

/// The light-space box of a box given at two opposite corners.
fn light_box(to_light: Vec3, low: Vec3, high: Vec3) -> Box3 {
    let plane = LightPlane::none(to_light);
    let mut corners = [Vec3::ZERO; 8];
    let mut index = 0;
    for sx in [low.x, high.x] {
        for sy in [low.y, high.y] {
            for sz in [low.z, high.z] {
                corners[index] = Vec3::new(sx, sy, sz);
                index += 1;
            }
        }
    }
    plane.boxed(&corners)
}

/// The light view of one frame's frame (module comment, "Light frame").
struct LightFrame {
    /// The axes of the light view (see [`light_axes`]).
    axes: (Vec3, Vec3, Vec3),
    /// Where the light view looks: the frame's centre.
    center: Vec3,
    /// The half side of the orthographic box (the light's x and y).
    half: f32,
    /// The near and far distances of the orthographic box along the light.
    near: f32,
    far: f32,
    /// The planes an extraction tests its casters against.
    plane: LightPlane,
}

/// How much one extraction saw and how many of the rigid casters the light
/// frame keeps.
#[derive(Default, Clone, Copy)]
struct ShadowCasterTally {
    seen: usize,
    kept: usize,
    /// Of the kept, the ones that are the sky shell.
    sky_shells: usize,
}

/// 抽取与解算结果：本帧的 caster 名单与光源框。
#[derive(Resource, Default)]
struct ShadowDrawList {
    draws: Vec<ShadowDraw>,
    /// The planes of the light frame the last prepare wrote: a caster inside
    /// them cannot reach the map and keeps its place and pipeline.
    plane: LightPlane,
    /// The settings the last prepare drew with.
    settings: Option<ShadowSettings>,
    /// How many extractions have drawn under those settings (the casters that
    /// never reach a camera frame are left out once the frame has settled,
    /// [`CASTER_HOLD_FRAMES`] later).
    planes_from: u32,
    casters: ShadowCasterTally,
    /// Rigid casters outside a camera frame's planes, wholly behind it or
    /// beyond the fade, left out of the list.
    left_out: usize,
    /// The skinned character casters of this extraction.
    skinned: Vec<SkinnedShadowDraw>,
    /// The world box of every skinned caster, merged: the depth range takes
    /// it in, so a character does not move the cross section.
    skinned_extent: Option<(Vec3, Vec3)>,
    /// Their joint palettes, each at a [`PALETTE_ALIGN`]-aligned offset.
    palette_bytes: Vec<u8>,
    skinned_tally: SkinnedTally,
    /// Whether this extraction's draw list differs from the previous one's in
    /// any mesh, world matrix bit, winding flag or count (the object pool and
    /// the static map follow it).
    changed: bool,

    /// The static map no longer holds exactly the baked draws as they are
    /// now; set by every change, turned into a bake request by
    /// [`plan_static_bake`].
    static_dirty: bool,
    /// The bake this frame's node run must draw, by generation; it stays set
    /// until a node run answers it.
    static_request: Option<u64>,
    static_generation: u64,
    /// Baked draws in the list.
    baked_count: usize,
    /// Total casters truncated by [`OBJECT_CAPACITY`] (the account reports it
    /// once).
    overflow: usize,
}

/// What the node's last bake of the static map drew: the generation it
/// answered, the draws it recorded and the baked draws it could not record
/// (mesh or buffer slices missing), which [`plan_static_bake`] turns back
/// into per-frame draws.
#[derive(Resource, Default)]
struct StaticBake(Mutex<BakeInner>);

#[derive(Default)]
struct BakeInner {
    answered: Option<u64>,
    recorded: usize,
    skipped: Vec<usize>,
}

/// The world-space bounds corners of each draw in [`ShadowDrawList`], at the
/// same index as the draw.
#[derive(Default)]
struct CasterCorners(Vec<[Vec3; 8]>);

/// 阴影账目（模块注释「阴影账目」）：渲染图节点只拿 `&World`，计数走
/// 内部可变性。锁的临界段都是几条赋值，渲染应用单线程，无重入。
#[derive(Resource, Default)]
struct ShadowAccount(Mutex<AccountInner>);

#[derive(Default)]
struct AccountInner {
    /// Prepares that resolved a light frame (the parameters have an
    /// intensity).
    frames_prepared: u64,
    /// Node runs that recorded at least one draw.
    frames_drawn: u64,
    /// Whole frames skipped because a pipeline was not compiled yet.
    frames_skipped_not_ready: u64,
    /// Node runs that recorded nothing.
    frames_empty_pass: u64,
    /// The latest prepare's caster list, and how many of them have no layout
    /// that can be drawn (INVALID).
    casters: usize,
    casters_invalid: usize,
    /// The latest node run's recorded draws.
    draws_recorded: usize,
    /// Of those, the draws in the static map (as of its last bake) and the
    /// rigid draws recorded in the frame's own pass; bakes of the static map.
    static_recorded: usize,
    per_frame_rigid: usize,
    static_bakes: u64,
    /// Since the settings last changed: the prepares run, and the ones whose
    /// light frame block and consumer block were written.
    prepares_since: u64,
    frames_written: u64,
    /// The latest prepare's skinned casters, those whose layout has no
    /// position/normal/joint streams, and the skinned draws last recorded.
    skinned_casters: usize,
    skinned_invalid: usize,
    skinned_recorded: usize,
    skinned_tally: SkinnedTally,
    /// The latest frame's light box geometry and consumer matrix.
    light_box: Option<(f32, f32, f32)>,
    /// The latest caster bias written: texel size, depth bias and normal
    /// bias, in metres.
    caster_bias: Option<(f32, f32, f32)>,
    /// The settings the latest prepare drew with.
    settings: Option<ShadowSettings>,
    /// The rigid casters the last extraction saw, kept and left out, and
    /// whether a sky shell was among them.
    casters_seen: usize,
    casters_kept: usize,
    casters_left_out: usize,
    sky_shell_in_frame: bool,
    /// Total casters truncated by the object capacity.
    casters_overflow: usize,
    /// The depth range, the sun's penumbra per metre and the map's side of
    /// the latest frame.
    depth_range: f32,
    penumbra_per_metre: f32,
    resolution: u32,
    /// The latest prepare's rigid casters whose mesh has no normal stream.
    rigid_without_normals: usize,
    world_to_shadow: Option<Mat4>,
    /// The depth-map read-back buffer (reused; native diagnosis only, the
    /// browser build has none). The maps of another side replace it with the
    /// previous read-back.
    #[cfg(not(target_arch = "wasm32"))]
    probe_buffer: Option<Buffer>,
    /// The read-back buffer's size, in bytes (0: none).
    #[cfg(not(target_arch = "wasm32"))]
    probe_bytes: u64,
    /// A copy is recorded and the node's next head reads it back.
    probe_pending: bool,
    /// The frame order of the next read-back (in frames drawn).
    next_probe_frame: u64,
    /// The previous read-back's texels and player window (native diagnosis).
    #[cfg(not(target_arch = "wasm32"))]
    previous_map: Option<Vec<u8>>,
    #[cfg(not(target_arch = "wasm32"))]
    previous_window: Option<(usize, usize, usize, usize)>,
}

/// The joint palette buffer of the skinned casters: replaced by a larger one
/// when a frame's palettes do not fit, so the node's bind group is looked up
/// by this buffer's id.
#[derive(Resource)]
struct ShadowPaletteGpu {
    buffer: Buffer,
    capacity: u64,
}

/// 管线特化键：同一顶点布局 + 图元拓扑共用一条深度管线。
#[derive(Clone, PartialEq, Eq, Hash)]
struct DepthPipelineKey {
    layout: MeshVertexBufferLayoutRef,
    topology: PrimitiveTopology,
    two_sided: bool,
    /// The skinned character entry point instead of the rigid one.
    skinned: bool,
    /// The mesh has a normal stream (the rigid entry point's normal bias).
    normals: bool,
}

fn depth_pipeline_key(mesh: &RenderMesh, two_sided: bool, skinned: bool) -> DepthPipelineKey {
    DepthPipelineKey {
        layout: mesh.layout.clone(),
        topology: mesh.primitive_topology(),
        two_sided,
        skinned,
        normals: mesh.layout.0.contains(Mesh::ATTRIBUTE_NORMAL),
    }
}

/// 已入队的深度管线：键 → 缓存 id（id 在编译完成前取不到真管线）。
#[derive(Resource, Default)]
struct DepthPipelines {
    queued: HashMap<DepthPipelineKey, CachedRenderPipelineId>,
}

/// Extract：抽本帧 caster（含预计算包围盒）。The rigid casters are the meshes
/// without a skin, except emote draws, rain and whatever is marked
/// [`NoShadowCast`] (the query) or fails the renderer, visibility and
/// source-pass tests below (module comment, "Caster set"). The sky shell is
/// taken too and marked: the light frame is derived from every caster while it
/// has not settled (the draw keeps its static-map place when nothing but the
/// sky shell changes), and the prepare leaves it out of a camera frame, which
/// would otherwise have it cover the whole map.
///
/// A caster in the same place as the preceding extraction's, whose corners
/// the last frame's planes still hold, keeps its draw (pipeline, static-map
/// place, stability); every other one is queued again. A draw outside the
/// planes of a camera frame is left out once the frame has settled
/// ([`CASTER_HOLD_FRAMES`]).
fn extract_shadow_casters(
    mut list: ResMut<ShadowDrawList>,
    gpu: Option<Res<ShadowMapGpu>>,
    casters: Extract<
        Query<
            (
                &Mesh3d,
                &GlobalTransform,
                &InheritedVisibility,
                Option<&ChildOf>,
                Option<&SourceRenderer>,
                Option<&SourceMaterialPasses>,
                Has<SkyDome>,
            ),
            (
                Without<EmoteDraw>,
                Without<SkinnedMesh>,
                Without<NoShadowCast>,
                Without<moly_assets::scene_state::SourceInactive>,
                Without<bevy::light::NotShadowCaster>,
                Without<WallLayoutShadowCasterOff>,
            ),
        >,
    >,
    parent_visibility: Extract<Query<&InheritedVisibility>>,
    meshes: Extract<Res<Assets<Mesh>>>,
    mut local_bounds: Local<HashMap<AssetId<Mesh>, Aabb3d>>,
    mut corners_of: Local<CasterCorners>,
    mut overflow_warned: Local<bool>,
) {
    let list = &mut *list;
    let stored = &mut corners_of.0;
    let previous = list.draws.len();
    // The list is built here and written back below: the corners' place and
    // the draws' place go together.
    stored.clear();
    // Bounds of meshes that left the asset store are dropped; a draw of such
    // a mesh is recomputed from the bounds its first sighting now caches.
    let mut dropped: Vec<AssetId<Mesh>> = Vec::new();
    local_bounds.retain(|id, _| {
        let keep = meshes.contains(*id);
        if !keep {
            dropped.push(*id);
        }
        keep
    });
    let mut plane = list.plane;
    let to_light = Vec3::from_array(SiteEnv::neutral().globals.light_vector);
    if let Some(direction) = to_light.try_normalize() {
        plane.to_light = direction;
    }
    // The camera frame of the frames just after the maps were rebuilt may
    // describe a view still being set up, so nothing is left out until it has
    // settled.
    let leaves_out_hidden = gpu.is_some_and(|gpu| gpu.frame_has_settled())
        && list
            .settings
            .is_some_and(|settings| list.planes_from >= CASTER_HOLD_FRAMES && uses_camera_frame(settings));

    let mut changed = false;
    // A baked draw that changes or leaves the list makes the static map
    // stale.
    let mut static_dirty = false;
    let mut draws: Vec<ShadowDraw> = Vec::with_capacity(list.draws.capacity().min(OBJECT_CAPACITY));
    let mut seen = 0usize;
    let mut sky_shells = 0usize;
    let mut left_out = 0usize;
    for (mesh, transform, visibility, parent, renderer, passes, sky_shell) in &casters {
        if renderer.is_some_and(|renderer| !renderer.casts_shadows()) {
            continue;
        }
        // ShadowsOnly hides the primitive in colour, not the owning object's
        // hierarchy. Honour that object's inherited visibility and source
        // activity, without reviving disabled renderers or hidden ancestors.
        let visible_to_light = if renderer.is_some_and(SourceRenderer::shadows_only) {
            parent
                .and_then(|parent| parent_visibility.get(parent.parent()).ok())
                .is_some_and(|visibility| visibility.get())
        } else {
            visibility.get()
        };
        if !visible_to_light {
            continue;
        }
        // Colour-pass visibility is not permission to invent a ShadowCaster
        // pass. For example, ground decorations can draw alpha-cut contours
        // in colour while declaring no shadow pass at all. Use source tags,
        // not a shader-name exclusion list. Absent metadata remains a separate
        // extraction/consumer gap; do not interpret it as a known empty list.
        if passes.is_some_and(|passes| !passes.has_shadow_caster()) {
            continue;
        }
        seen += 1;
        if draws.len() >= OBJECT_CAPACITY {
            continue;
        }
        let id = mesh.0.id();
        // ShadowsOnly draws its casters two-sided (there is no front to face
        // the light), as the source's cast mode does.
        let two_sided = renderer.is_some_and(|renderer| {
            matches!(
                renderer.shadow_casting(),
                Some(SourceShadowCastingMode::TwoSided | SourceShadowCastingMode::ShadowsOnly)
            )
        });
        let world = transform.to_matrix();
        let slot = draws.len();
        let before = list.draws.get(slot).filter(|_| slot < stored.len());
        let reused = before.is_some_and(|draw| {
            draw.mesh == id
                && draw.world.to_cols_array().map(f32::to_bits) == world.to_cols_array().map(f32::to_bits)
                && !dropped.contains(&id)
        });
        let corners = if reused {
            stored[slot]
        } else {
            let local = local_bounds.entry(id).or_insert_with(|| {
                meshes
                    .get(&mesh.0)
                    .and_then(|mesh| mesh.final_aabb)
                    .unwrap_or(Aabb3d::new(Vec3A::ZERO, Vec3A::ZERO))
            });
            let mut corners = [Vec3::ZERO; 8];
            let mut index = 0;
            for sx in [local.min.x, local.max.x] {
                for sy in [local.min.y, local.max.y] {
                    for sz in [local.min.z, local.max.z] {
                        corners[index] = world.transform_point3(Vec3::new(sx, sy, sz));
                        index += 1;
                    }
                }
            }
            corners
        };
        // The future planes: the last frame's moved by the direction change
        // and already covering this frame's corners (which the new ones,
        // once derived, hold as well).
        if reused && plane.holds(&corners) {
            changed |= list.draws[slot].two_sided != two_sided || list.draws[slot].sky_shell != sky_shell;
            let mut draw = list.draws[slot];
            if draw.two_sided != two_sided {
                static_dirty |= draw.baked;
                draw.two_sided = two_sided;
                draw.pipeline = CachedRenderPipelineId::INVALID;
                draw.baked = false;
                draw.stable = 0;
            } else {
                draw.stable = draw.stable.saturating_add(1);
            }
            // A camera frame never bakes the sky shell: it is drawn into the
            // frame's own pass.
            draw.sky_shell = sky_shell;
            sky_shells += sky_shell as usize;
            draws.push(draw);
            stored.push(corners);
            continue;
        }
        // A new box, or one outside the planes: left out when the view can
        // never show it, otherwise taken again (the static map must not keep
        // drawing a box that moved).
        if !sky_shell && leaves_out_hidden && plane.outside(&corners) {
            left_out += 1;
            continue;
        }
        changed = true;
        static_dirty |= before.is_some_and(|draw| draw.baked);
        sky_shells += sky_shell as usize;
        draws.push(ShadowDraw {
            mesh: id,
            world,
            corners,
            pipeline: CachedRenderPipelineId::INVALID,
            two_sided,
            sky_shell,
            stable: 0,
            baked: false,
        });
        stored.push(corners);
    }
    let overflow = seen.saturating_sub(OBJECT_CAPACITY);
    if overflow > 0 && !*overflow_warned {
        *overflow_warned = true;
        warn!("main-light shadow: {overflow} casters beyond the capacity of {OBJECT_CAPACITY} cast no shadow (warned once)");
    }
    changed |= draws.len() != previous;
    // A draw that left the list after the len above (the list shrank) or in
    // its middle makes the static map stale.
    static_dirty |= list.draws[draws.len().min(list.draws.len())..]
        .iter()
        .any(|draw| draw.baked);
    list.draws = draws;
    list.casters = ShadowCasterTally {
        seen,
        kept: list.draws.len(),
        sky_shells,
    };
    list.left_out = left_out;
    list.overflow = overflow;
    list.planes_from = list.planes_from.saturating_add(1);
    list.changed = changed;
    list.static_dirty |= static_dirty;
}

/// Whether the frame follows the camera ([`ShadowFit::Camera`]); the render
/// world's copy of the settings comes with the first extraction, so the
/// default stands in before it.
fn uses_camera_frame(settings: ShadowSettings) -> bool {
    settings.enabled && settings.fit == ShadowFit::Camera
}

/// Extract, after [`extract_shadow_casters`]: the skinned character casters
/// (module comment, "Caster set") and their joint palettes of this frame.
///
/// The light boxes stay the rigid casters' (a skinned draw's box is
/// conservative, so the cross section would move whenever a character
/// animates); only the depth range takes the skinned casters in, as the box
/// of all of them, so a character beyond the rigid depth range still casts. A
/// vertex nearer than the near plane is clamped to it by the skinned entry
/// point.
///
/// Renderers that share one skin (the same joints and inverse bind poses)
/// share one palette. A palette is the joint's global transform times its
/// inverse bind pose, joint by joint, exactly as the colour pass builds it.
fn extract_skinned_casters(
    mut list: ResMut<ShadowDrawList>,
    skinned: Extract<
        Query<
            (
                &Mesh3d,
                &SkinnedMesh,
                &InheritedVisibility,
                Option<&MeshMaterial3d<CharacterMaterial>>,
                Option<&crate::markers::PlayerBodyRenderer>,
                Option<&ChildOf>,
            ),
            (
                Without<NoShadowCast>,
                Without<moly_assets::scene_state::SourceInactive>,
                Without<bevy::light::NotShadowCaster>,
            ),
        >,
    >,
    joints: Extract<Query<&GlobalTransform>>,
    names: Extract<Query<&Name>>,
    bindposes: Extract<Res<Assets<SkinnedMeshInverseBindposes>>>,
    character_materials: Extract<Res<Assets<CharacterMaterial>>>,
    meshes: Extract<Res<Assets<Mesh>>>,
    mut local_bounds: Local<HashMap<AssetId<Mesh>, Aabb3d>>,
    mut too_many_warned: Local<bool>,
    mut others_logged: Local<bool>,
) {
    let list = &mut *list;
    let mut other_names: Vec<String> = Vec::new();
    local_bounds.retain(|id, _| meshes.contains(*id));
    list.skinned.clear();
    list.palette_bytes.clear();
    list.skinned_extent = None;
    let mut tally = SkinnedTally::default();
    // Skin -> (palette byte offset, index into `palette_matrices`).
    let mut palettes: HashMap<(AssetId<SkinnedMeshInverseBindposes>, Vec<Entity>), (u32, usize)> =
        HashMap::new();
    let mut palette_matrices: Vec<Vec<Mat4>> = Vec::new();
    for (mesh, skin, visibility, character, body, parent) in &skinned {
        // The player's body: the avatar's dressed skin, or its own model.
        let player = body.is_some();
        if !player && character.is_none() {
            tally.other_skinned += 1;
            if !*others_logged && other_names.len() < 12 {
                let name = parent
                    .and_then(|parent| names.get(parent.parent()).ok())
                    .map(|name| name.as_str().to_owned())
                    .unwrap_or_else(|| "?".to_owned());
                other_names.push(name);
            }
            continue;
        }
        if !visibility.get() {
            continue;
        }
        if let Some(character) = character {
            // The NPC's renderers are Off exactly while its dither alpha is 0.
            if character_materials
                .get(&character.0)
                .is_some_and(|material| material.params.dither_alpha == 0.0)
            {
                tally.npc_off_by_dither += 1;
                continue;
            }
        }
        let key = (skin.inverse_bindposes.id(), skin.joints.clone());
        let (palette_offset, palette_index) = match palettes.get(&key) {
            Some(found) => *found,
            None => {
                let matrices = match joint_palette(skin, &joints, &bindposes) {
                    Ok(matrices) => matrices,
                    Err(PaletteRefusal::TooManyJoints(count)) => {
                        if !*too_many_warned {
                            *too_many_warned = true;
                            warn!(
                                "main-light shadow: a skinned mesh has {count} joints, beyond the palette's {MAX_JOINTS}; it casts no shadow (warned once)"
                            );
                        }
                        continue;
                    }
                    Err(PaletteRefusal::Unavailable) => continue,
                };
                let offset = push_palette(&mut list.palette_bytes, &matrices);
                tally.palettes += 1;
                tally.joints += matrices.len();
                let entry = (offset, palette_matrices.len());
                palette_matrices.push(matrices);
                palettes.insert(key, entry);
                entry
            }
        };
        let id = mesh.0.id();
        let local = *local_bounds.entry(id).or_insert_with(|| {
            meshes
                .get(&mesh.0)
                .and_then(|mesh| mesh.final_aabb)
                .unwrap_or(Aabb3d::new(Vec3A::ZERO, Vec3A::ZERO))
        });
        // Every skinned vertex is a convex blend of its joints' images of the
        // bind-pose vertex, so the box of every joint's image of the bind-pose
        // box holds it.
        let mut min = Vec3::INFINITY;
        let mut max = Vec3::NEG_INFINITY;
        for matrix in &palette_matrices[palette_index] {
            for sx in [local.min.x, local.max.x] {
                for sy in [local.min.y, local.max.y] {
                    for sz in [local.min.z, local.max.z] {
                        let corner = matrix.transform_point3(Vec3::new(sx, sy, sz));
                        min = min.min(corner);
                        max = max.max(corner);
                    }
                }
            }
        }
        if player {
            tally.player += 1;
        } else {
            tally.npc += 1;
        }
        list.skinned_extent = Some(match list.skinned_extent {
            Some((low, high)) => (low.min(min), high.max(max)),
            None => (min, max),
        });
        list.skinned.push(SkinnedShadowDraw {
            mesh: id,
            palette_offset,
            pipeline: CachedRenderPipelineId::INVALID,
            player,
            bounds: (min, max),
        });
    }
    list.skinned_tally = tally;
    if !*others_logged && !other_names.is_empty() {
        *others_logged = true;
        info!(
            "shadow casters: {} skinned meshes are no character renderer and cast no shadow (first seen, by parent node: {})",
            tally.other_skinned,
            other_names.join(", ")
        );
    }
}

/// Why a skin has no joint palette this frame.
pub enum PaletteRefusal {
    /// More joints than a palette window holds.
    TooManyJoints(usize),
    /// The inverse bind poses are not loaded, a joint entity is gone, or the
    /// skin names more joints than it has inverse bind poses.
    Unavailable,
}

/// A skin's joint matrices of this frame, joint by joint: the joint's world
/// transform times its inverse bind pose, as the colour pass builds them, so
/// the blended matrix maps a bind-pose vertex straight to world space.
pub fn joint_palette(
    skin: &SkinnedMesh,
    joints: &Query<&GlobalTransform>,
    bindposes: &Assets<SkinnedMeshInverseBindposes>,
) -> Result<Vec<Mat4>, PaletteRefusal> {
    let Some(inverse) = bindposes.get(&skin.inverse_bindposes) else {
        return Err(PaletteRefusal::Unavailable);
    };
    if skin.joints.len() > MAX_JOINTS {
        return Err(PaletteRefusal::TooManyJoints(skin.joints.len()));
    }
    let mut matrices = Vec::with_capacity(skin.joints.len());
    for (joint, inverse_bindpose) in skin.joints.iter().zip(inverse.iter()) {
        let Ok(global) = joints.get(*joint) else {
            break;
        };
        matrices.push(Mat4::from(global.affine()) * *inverse_bindpose);
    }
    if matrices.is_empty() || matrices.len() != skin.joints.len() {
        return Err(PaletteRefusal::Unavailable);
    }
    Ok(matrices)
}

/// Appends a palette at the next [`PALETTE_ALIGN`]-aligned offset of `bytes`
/// and returns that offset, the dynamic offset of its window.
pub fn push_palette(bytes: &mut Vec<u8>, matrices: &[Mat4]) -> u32 {
    let offset = bytes.len().next_multiple_of(PALETTE_ALIGN);
    bytes.resize(offset, 0);
    for matrix in matrices {
        mat4_bytes(*matrix, bytes);
    }
    offset as u32
}

/// The light view's orthographic matrices: the depth pass's clip matrix and
/// the consumer's sampling matrix (xy folded into [0, 1] and y flipped: a
/// WebGPU framebuffer's origin is at the top left while NDC y points up, so
/// texture v = 0 is NDC y = +1; z is the compared depth in [0, 1], near 0
/// and far 1, which is exactly `orthographic`'s z convention). The frame's
/// geometry travels with the matrices (the light-space width and the depth
/// range, in metres), and the account derives texel size and depth gaps from
/// it.
struct LightMatrices {
    clip: Mat4,
    consumer: Mat4,
    width: f32,
    depth: f32,
    /// The unit direction towards the light.
    to_light: Vec3,
}

/// The shortest depth range a frame is given, in metres (a slice whose
/// casters all sit at one distance).
const MIN_DEPTH_RANGE: f32 = 2.0;

impl LightFrame {
    /// The clip and sampling matrices of the frame (the light axes, its
    /// centre and the depth range above).
    fn matrices(&self) -> Option<LightMatrices> {
        let (right, up, towards) = self.axes;
        if self.far.partial_cmp(&self.near) != Some(std::cmp::Ordering::Greater) || !self.half.is_finite() || self.half <= 0.0 {
            return None;
        }
        // The light view: the axes again, put on the frame's centre. The view
        // looks away from the light (its z axis points towards it), so the
        // distance in front of it is the negated depth towards the light, the
        // range `near..far` spans.
        let view = Mat4::from_cols(
            right.extend(0.0),
            up.extend(0.0),
            towards.extend(0.0),
            self.center.extend(1.0),
        )
        .inverse();
        let proj = glam::camera::rh::proj::directx::orthographic(
            -self.half,
            self.half,
            -self.half,
            self.half,
            self.near,
            self.far,
        );
        let light_view_proj = proj * view;
        // uv 折算：u = 0.5x + 0.5，v = −0.5y + 0.5（y 翻转见上），z 直通。
        let to_uv = Mat4::from_cols_array(&[
            0.5, 0.0, 0.0, 0.0, //
            0.0, -0.5, 0.0, 0.0, //
            0.0, 0.0, 1.0, 0.0, //
            0.5, 0.5, 0.0, 1.0,
        ]);
        Some(LightMatrices {
            clip: light_view_proj,
            consumer: to_uv * light_view_proj,
            width: 2.0 * self.half,
            depth: self.far - self.near,
            to_light: towards,
        })
    }
}

/// Builds the light frame of one extraction (module comment, "Light frame").
/// `casters` holds the world corners of every caster the frame keeps and
/// `skinned` the world box of every skinned caster; the depth range spans all
/// of them. The frame itself is the camera's slice (`slice`, the camera fit)
/// or the casters' box (the site fit).
fn light_frame(
    to_light: Vec3,
    settings: &ShadowSettings,
    resolution: u32,
    casters: &[[Vec3; 8]],
    slice: Option<&ShadowCameraSlice>,
    skinned: Option<(Vec3, Vec3)>,
) -> Option<LightFrame> {
    let axes = light_axes(to_light);
    let (right, up, towards) = axes;
    let plane = LightPlane::none(towards);
    let mut cross = Spans::NONE;
    let mut depth = Span::NONE;
    for corners in casters {
        let boxed = plane.boxed(corners);
        if boxed.is_set() {
            cross.merge(boxed.across);
            depth.grow(boxed.depth.min);
            depth.grow(boxed.depth.max);
        }
    }
    if let Some((low, high)) = skinned {
        let boxed = light_box(towards, low, high);
        if boxed.is_set() {
            cross.merge(boxed.across);
            depth.grow(boxed.depth.min);
            depth.grow(boxed.depth.max);
        }
    }
    if !cross.is_set() || !depth.is_set() {
        return None;
    }
    let (center_x, center_y, half) = match settings.fit {
        ShadowFit::Camera => {
            // The square around the slice's sphere, its centre on whole
            // texels of that square so that camera motion moves the grid by
            // whole texels only.
            let slice = slice?;
            let sphere = SliceSphere::around(slice);
            let half = sphere.radius.max(MIN_FRAME_RADIUS);
            let texel = 2.0 * half / resolution.max(1) as f32;
            let snap = |value: f32| (value / texel).round() * texel;
            (
                snap(sphere.center.dot(right)),
                snap(sphere.center.dot(up)),
                half,
            )
        }
        ShadowFit::Site => (
            (cross.min.x + cross.max.x) * 0.5,
            (cross.min.y + cross.max.y) * 0.5,
            ((cross.max.x - cross.min.x).max(cross.max.y - cross.min.y) * 0.5).max(MIN_FRAME_RADIUS),
        ),
    };
    let center = right * center_x + up * center_y;
    let near = -(depth.max + DEPTH_PAD);
    let far = (-(depth.min - DEPTH_PAD)).max(near + MIN_DEPTH_RANGE);
    let plane = LightPlane {
        to_light: towards,
        cross: Spans {
            min: Vec2::new(center_x - half, center_y - half),
            max: Vec2::new(center_x + half, center_y + half),
        },
        depth: Span {
            min: -far,
            max: -near,
        },
    };
    Some(LightFrame {
        axes,
        center,
        half,
        near,
        far,
        plane,
    })
}

/// The sphere around a camera slice's eight corners.
#[derive(Clone, Copy)]
struct SliceSphere {
    center: Vec3,
    radius: f32,
}

impl SliceSphere {
    /// The sphere around a slice of frustum corners (four at the near plane,
    /// four at the far end of the slice).
    fn around(slice: &ShadowCameraSlice) -> Self {
        let mut center = Vec3::ZERO;
        for corner in slice.near.iter().chain(slice.far.iter()) {
            center += *corner;
        }
        center /= 8.0;
        let radius = slice
            .near
            .iter()
            .chain(slice.far.iter())
            .map(|corner| corner.distance(center))
            .fold(0.0, f32::max);
        Self { center, radius }
    }
}

/// The slice of a camera as the next frame would find it again: the camera
/// and the settings it was taken with (the camera fit's frame).
#[derive(Component, Clone, Copy, PartialEq)]
struct ShadowCameraKey {
    camera: Entity,
    pose: [u32; 16],
    near: u32,
    far: u32,
    aspect: u32,
    fov: u32,
}

/// The camera fit's slice of one render camera (module comment on
/// [`ShadowCameraSlice`]): the frustum's corners at the near plane and at the
/// settings' distance, of the camera whose pose and projection the site
/// globals follow. Runs in the render world's `Prepare`, before
/// `PrepareResources`. A camera, projection and distance that did not change
/// since the last frame keep the slice they had.
fn fit_camera_frame(
    mut commands: Commands,
    settings: Option<Res<ShadowSettingsGlobals>>,
    env: Option<Res<SiteEnv>>,
    key: Query<&ShadowCameraKey>,
    cameras: Query<(Entity, &GlobalTransform, &Projection), With<Camera3d>>,
) {
    let Some(settings) = settings else {
        return;
    };
    if settings.settings.fit != ShadowFit::Camera {
        return;
    }
    // The main 3D camera is the one the site globals follow: the camera whose
    // position the environment holds.
    let view = env.as_deref().map(|env| env.globals.camera_position);
    let Some((entity, global, projection)) = cameras.iter().find(|(_, global, _)| {
        view.is_some_and(|view| {
            let at = global.translation();
            (at.x - view[0]).abs() < 1e-3 && (at.y - view[1]).abs() < 1e-3 && (at.z - view[2]).abs() < 1e-3
        })
    }) else {
        return;
    };
    let mut key_now = ShadowCameraKey {
        camera: entity,
        pose: [0; 16],
        near: 0.0f32.to_bits(),
        far: settings.settings.distance.to_bits(),
        aspect: 0,
        fov: 0,
    };
    for (slot, value) in key_now.pose.iter_mut().zip(global.to_matrix().to_cols_array()) {
        *slot = value.to_bits();
    }
    match projection {
        Projection::Perspective(perspective) => {
            key_now.near = perspective.near.to_bits();
            key_now.aspect = perspective.aspect_ratio.to_bits();
            key_now.fov = perspective.fov.to_bits();
        }
        Projection::Orthographic(orthographic) => key_now.near = orthographic.near.to_bits(),
        Projection::Custom(_) => return,
    }
    if key.get(entity).is_ok_and(|previous| *previous == key_now) {
        return;
    }
    let Some(slice) = camera_slice(global, projection, settings.settings.distance) else {
        return;
    };
    commands.entity(entity).insert((slice, key_now));
}

/// The corners of the frustum at the camera's near plane and at `distance` in
/// front of it, or `None` for a projection without a frustum to slice.
fn camera_slice(global: &GlobalTransform, projection: &Projection, distance: f32) -> Option<ShadowCameraSlice> {
    let affine = global.affine();
    let forward = affine.transform_vector3(Vec3::NEG_Z);
    let corners = |distance: f32| -> Option<[Vec3; 4]> {
        let mut corners = [Vec3::ZERO; 4];
        for (index, (x, y)) in [(-1.0f32, 1.0f32), (1.0, 1.0), (-1.0, -1.0), (1.0, -1.0)]
            .into_iter()
            .enumerate()
        {
            corners[index] = match projection {
                Projection::Perspective(perspective) => {
                    let half_height = (perspective.fov * 0.5).tan();
                    let half_width = half_height * perspective.aspect_ratio;
                    affine.transform_point3(Vec3::new(x * half_width, y * half_height, 1.0) * distance)
                }
                Projection::Orthographic(orthographic) => {
                    let half_height = orthographic.area.height() * 0.5;
                    let half_width = orthographic.area.width() * 0.5;
                    global.translation()
                        + forward * distance
                        + affine.transform_vector3(Vec3::new(x * half_width, y * half_height, 0.0))
                }
                Projection::Custom(_) => return None,
            };
        }
        Some(corners)
    };
    let near = match projection {
        Projection::Perspective(perspective) => perspective.near,
        Projection::Orthographic(orthographic) => orthographic.near,
        Projection::Custom(_) => return None,
    };
    let near = corners(near.max(CAMERA_FRAME_BEHIND))?;
    let far = corners(distance.max(CAMERA_FRAME_BEHIND))?;
    Some(ShadowCameraSlice { near, far })
}

/// What [`prepare_shadow_draws`] remembers between frames.
#[derive(Default)]
struct PrepareMemory {
    /// A first set of settings was applied.
    seen_settings: bool,
    /// The one-time readiness line was logged.
    readiness_logged: bool,
    /// Prepares run so far (the account is logged at a few of them).
    prepare_frames: u64,
    /// What the buffers last received.
    uploads: ShadowUploads,
    /// The side of the maps the uploads were written for.
    maps_size: u32,
}

/// What the three shadow buffers last received from `prepare_shadow_draws`.
#[derive(Default)]
struct ShadowUploads {
    frame: Option<Vec<u8>>,
    consumer: Option<Vec<u8>>,
    /// The settings the consumer block was built from (its distances and the
    /// light size are in metres, so unchanged bytes alone do not say the
    /// block still holds).
    consumer_settings: Option<ShadowSettings>,
    /// The object pool holds the current draw list's matrices.
    objects: bool,
}

impl ShadowUploads {
    /// The maps were rebuilt: nothing the buffers hold was written by this
    /// list or this frame block any more.
    fn forget(&mut self) {
        *self = Self::default();
    }
}

/// Queues the depth pipeline of one key, or INVALID when the mesh layout lacks
/// a stream the entry point reads (counted, and skipped by the node).
fn queue_depth_pipeline(
    pipeline_cache: &PipelineCache,
    gpu: &ShadowMapGpu,
    key: &DepthPipelineKey,
) -> CachedRenderPipelineId {
    if key.skinned {
        let Ok(vertex) = key.layout.0.get_layout(&[
            Mesh::ATTRIBUTE_POSITION.at_shader_location(0),
            Mesh::ATTRIBUTE_NORMAL.at_shader_location(1),
            Mesh::ATTRIBUTE_JOINT_INDEX.at_shader_location(2),
            Mesh::ATTRIBUTE_JOINT_WEIGHT.at_shader_location(3),
        ]) else {
            return CachedRenderPipelineId::INVALID;
        };
        return pipeline_cache.queue_render_pipeline(RenderPipelineDescriptor {
            label: Some("site_shadow_depth_skinned_pipeline".into()),
            layout: vec![gpu.depth_layout.clone(), gpu.skin_layout.clone()],
            vertex: VertexState {
                constants: vec![],
                shader: SHADOW_DEPTH_SHADER.clone(),
                shader_defs: vec![],
                entry_point: Some("shadow_depth_skinned_vertex".into()),
                buffers: vec![vertex],
            },
            fragment: None,
            primitive: PrimitiveState {
                topology: key.topology,
                front_face: FrontFace::Ccw,
                // The character ShadowCaster passes cull nothing.
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(DepthStencilState {
                format: TextureFormat::Depth32Float,
                depth_write_enabled: Some(true),
                depth_compare: Some(CompareFunction::Less),
                stencil: StencilState::default(),
                bias: SHADOW_SLICE_RASTER_BIAS,
            }),
            multisample: Default::default(),
            ..Default::default()
        });
    }
    // 位置属性的偏移由网格布局自身给出；缺位置的网格按 INVALID
    // 记账，节点侧跳过。With a normal stream the rigid entry point runs the
    // whole source bias; without one, the bias along the light only.
    let (attributes, entry_point) = if key.normals {
        (
            vec![
                Mesh::ATTRIBUTE_POSITION.at_shader_location(0),
                Mesh::ATTRIBUTE_NORMAL.at_shader_location(1),
            ],
            "shadow_depth_vertex",
        )
    } else {
        (
            vec![Mesh::ATTRIBUTE_POSITION.at_shader_location(0)],
            "shadow_depth_vertex_without_normal",
        )
    };
    match key.layout.0.get_layout(&attributes) {
        Ok(vertex) => pipeline_cache.queue_render_pipeline(RenderPipelineDescriptor {
            label: Some("site_shadow_depth_pipeline".into()),
            layout: vec![gpu.depth_layout.clone()],
            vertex: VertexState {
                constants: vec![],
                shader: SHADOW_DEPTH_SHADER.clone(),
                shader_defs: vec![],
                entry_point: Some(entry_point.into()),
                buffers: vec![vertex],
            },
            fragment: None,
            primitive: PrimitiveState {
                topology: key.topology,
                front_face: FrontFace::Ccw,
                cull_mode: if key.two_sided { None } else { Some(Face::Back) },
                ..Default::default()
            },
            depth_stencil: Some(DepthStencilState {
                format: TextureFormat::Depth32Float,
                depth_write_enabled: Some(true),
                // normal-Z：保留离光最近（深度最小）的面。
                depth_compare: Some(CompareFunction::Less),
                stencil: StencilState::default(),
                bias: SHADOW_SLICE_RASTER_BIAS,
            }),
            multisample: Default::default(),
            ..Default::default()
        }),
        Err(_) => CachedRenderPipelineId::INVALID,
    }
}

/// PrepareResources：重建深度图（边长变了时）、特化管线、解光源正交框、写
/// 三块 uniform。
///
/// 挂 PrepareResources 而非 Prepare：与引擎的网格资产 prepare（Prepare 集）
/// 错开一拍，本帧的 `RenderAssets<RenderMesh>` 与 MeshAllocator 切片已定。
///
/// A change of the settings (including a resolution change, whose new maps
/// [`apply_shadow_resolution`] created during extraction) restarts the draw
/// list's planes and the buffers' upload memory.
///
/// A buffer whose bytes came out as the last frame's run's is skipped: the
/// light frame block by content (frames without a frame write zeros), the
/// object pool when the draw list did not change, and the consumer block both
/// by content and whenever the settings changed (its distances and light size
/// are in metres).
fn prepare_shadow_draws(
    mut draws: ResMut<ShadowDrawList>,
    mut pipelines: ResMut<DepthPipelines>,
    pipeline_cache: Res<PipelineCache>,
    render_device: Res<RenderDevice>,
    render_queue: Res<RenderQueue>,
    meshes: Res<RenderAssets<RenderMesh>>,
    extracted: Option<Res<ExtractedAssets<RenderMesh>>>,
    env: Option<Res<SiteEnv>>,
    settings: Option<Res<ShadowSettingsGlobals>>,
    mut gpu: ResMut<ShadowMapGpu>,
    mut palette: ResMut<ShadowPaletteGpu>,
    account: Res<ShadowAccount>,
    slices: Query<&ShadowCameraSlice>,
    mut memory: Local<PrepareMemory>,
) {
    let PrepareMemory {
        seen_settings,
        readiness_logged,
        prepare_frames,
        uploads,
        maps_size,
    } = &mut *memory;
    let settings = settings.map(|globals| globals.settings).unwrap_or_default();
    // The settings changed: the planes and the extractions' hold count start
    // again.
    let changed = !*seen_settings || draws.settings != Some(settings);
    if *maps_size != gpu.size {
        // New maps: nothing the buffers hold was written for them.
        uploads.forget();
        *maps_size = gpu.size;
    }
    if changed {
        draws.planes_from = 0;
        {
            let mut frame = account.0.lock().unwrap();
            frame.prepares_since = 0;
            frame.frames_written = 0;
        }
        draws.settings = Some(settings);
        *seen_settings = true;
    }
    if !settings.enabled {
        // No depth pass: no draw stays on the GPU, and a draw that comes back
        // is queued again on that frame.
        for item in &mut draws.draws {
            item.pipeline = CachedRenderPipelineId::INVALID;
            item.baked = false;
        }
        for item in &mut draws.skinned {
            item.pipeline = CachedRenderPipelineId::INVALID;
        }
        draws.baked_count = 0;
        draws.plane = LightPlane::default();
        // The receivers read intensity 0 whichever map is bound: the map of
        // the last frame with shadows is not updated any more.
        let consumer = ShadowConsumer::unlit();
        let consumer_bytes = consumer.bytes();
        if uploads.consumer.as_deref() != Some(consumer_bytes.as_slice()) {
            render_queue.write_buffer(&gpu.consumer_buffer, 0, &consumer_bytes);
            uploads.consumer = Some(consumer_bytes);
            uploads.consumer_settings = None;
        }
        return;
    }

    // 矩阵池按 draw 序全量写（缺管线的 draw 也占位，动态 offset 才对得上）。
    // The dynamic binding reads at index * object_stride, not index * 64.
    // Pad every matrix, including unavailable meshes, to that same slot size.
    // Otherwise each draw after the first consumes another object's transform
    // (or unwritten/stale bytes) and casts a displaced or malformed silhouette.
    // The pool is written once, never reallocated, and keeps what was last
    // written; it is rewritten only when the draw list changed.
    let stride = gpu.object_stride as usize;
    let write_objects = draws.changed || !uploads.objects;
    let mut object_bytes = Vec::new();
    if write_objects {
        object_bytes.reserve(draws.draws.len() * stride);
        for item in &draws.draws {
            let next_slot = object_bytes.len() + stride;
            mat4_bytes(item.world, &mut object_bytes);
            object_bytes.resize(next_slot, 0);
        }
    }
    // A rigid draw keeps its pipeline across frames while its mesh is not
    // prepared again (a mesh prepared again may have another vertex layout);
    // extraction clears it when the draw's mesh or sidedness changes.
    let touched = extracted
        .as_deref()
        .filter(|extracted| !extracted.added.is_empty() || !extracted.modified.is_empty());
    for item in draws.draws.iter_mut() {
        let prepared_again = touched.is_some_and(|extracted| {
            extracted.added.contains(&item.mesh) || extracted.modified.contains(&item.mesh)
        });
        if item.pipeline != CachedRenderPipelineId::INVALID && !prepared_again {
            continue;
        }
        let Some(render_mesh) = meshes.get(item.mesh) else {
            item.pipeline = CachedRenderPipelineId::INVALID;
            continue;
        };
        let key = depth_pipeline_key(render_mesh, item.two_sided, false);
        item.pipeline = *pipelines
            .queued
            .entry(key)
            .or_insert_with_key(|key| queue_depth_pipeline(&pipeline_cache, &gpu, key));
    }
    // The skinned character casters: their own entry point and state (module
    // comment, "Caster set"); a mesh without normals or joint streams is
    // counted as a layout that cannot be drawn.
    for item in draws.skinned.iter_mut() {
        item.pipeline = match meshes.get(item.mesh) {
            Some(render_mesh) => *pipelines
                .queued
                .entry(depth_pipeline_key(render_mesh, true, true))
                .or_insert_with_key(|key| queue_depth_pipeline(&pipeline_cache, &gpu, key)),
            None => CachedRenderPipelineId::INVALID,
        };
    }
    // This frame's joint palettes; a larger buffer replaces the old one when
    // they do not fit (the window read at the last offset must fit too).
    if !draws.palette_bytes.is_empty() {
        let needed = draws.palette_bytes.len() as u64 + PALETTE_WINDOW;
        if needed > palette.capacity {
            let capacity = needed.next_power_of_two();
            palette.buffer = render_device.create_buffer(&BufferDescriptor {
                label: Some("site_shadow_palettes"),
                size: capacity,
                usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            palette.capacity = capacity;
        }
        render_queue.write_buffer(&palette.buffer, 0, &draws.palette_bytes);
    }

    // The light frame (module comment, "Light frame"). Without a light
    // direction or a caster there is no frame: the consumer reads intensity 0
    // and every receiver is fully lit, whichever map is bound. The sky shell
    // reaches the light only while the frame is derived from every caster (a
    // camera frame would have it drawn across the whole map).
    let camera_frame = settings.fit == ShadowFit::Camera;
    let slice = camera_frame.then(|| slices.single().ok().copied()).flatten();
    let to_light = env
        .as_deref()
        .and_then(|env| Vec3::from_array(env.globals.light_vector).try_normalize());
    let mut corners: Vec<[Vec3; 8]> = Vec::with_capacity(draws.draws.len());
    corners.extend(
        draws
            .draws
            .iter()
            .filter(|draw| !camera_frame || !draw.sky_shell)
            .map(|draw| draw.corners),
    );
    let frame = to_light
        .filter(|_| !corners.is_empty())
        .and_then(|to_light| {
            light_frame(
                to_light,
                &settings,
                gpu.size,
                &corners,
                slice.as_ref(),
                draws.skinned_extent,
            )
        });
    let light = frame.as_ref().and_then(LightFrame::matrices);
    let (frame_block, consumer) = match &light {
        Some(matrices) => {
            // The pipeline's caster bias: both biases in shadow-map texels of
            // world size (the orthographic width over the resolution, as the
            // pipeline derives it from the projection), negated, scaled by the
            // soft-shadow kernel radius and by the settings' multipliers.
            let texel = matrices.width / gpu.size as f32;
            let depth_bias =
                -PIPELINE_SHADOW_DEPTH_BIAS * texel * SOFT_SHADOW_KERNEL_RADIUS * settings.depth_bias;
            let normal_bias =
                -PIPELINE_SHADOW_NORMAL_BIAS * texel * SOFT_SHADOW_KERNEL_RADIUS * settings.normal_bias;
            let to_light = matrices.to_light;
            (
                ShadowFrame {
                    light_view_proj: mat4_cols(matrices.clip),
                    world_to_shadow: mat4_cols(matrices.consumer),
                    shadow_bias: Vec4::new(depth_bias, normal_bias, 0.0, 0.0),
                    light_direction: to_light.extend(0.0),
                },
                consumer_block(
                    matrices.consumer,
                    matrices.width,
                    gpu.size,
                    &settings,
                    matrices.depth,
                ),
            )
        }
        None => (
            ShadowFrame {
                light_view_proj: [Vec4::ZERO; 4],
                world_to_shadow: [Vec4::ZERO; 4],
                shadow_bias: Vec4::ZERO,
                light_direction: Vec4::ZERO,
            },
            ShadowConsumer::unlit(),
        ),
    };
    // The planes the next extraction tests a draw against. A camera frame
    // that moved makes the static map stale: its baked draws were placed under
    // the old frame (a draw set that only lost a draw to the frame still has
    // its baked draws in the list, and a bake covers them).
    draws.plane = match &frame {
        Some(frame) => {
            if frame.plane != draws.plane {
                draws.static_dirty = true;
            }
            frame.plane
        }
        None => LightPlane::none(to_light.unwrap_or(Vec3::Y)),
    };

    // 阴影账目：prepare 侧记帧数与名单（节点侧记执行与读回，模块注释
    // 「阴影账目」）。The numbers are this run's, which skips nothing, so the
    // account can tell a skipped buffer from a written one.
    let invalid_now = draws
        .draws
        .iter()
        .filter(|item| item.pipeline == CachedRenderPipelineId::INVALID)
        .count();
    // The static map was drawn with the previous frame block (its bias
    // included); a new block makes it stale.
    let frame_written = uploads.frame.as_deref() != Some(frame_bytes_of(&frame_block).as_slice());
    let consumer_bytes = consumer.bytes();
    let consumer_written = uploads.consumer_settings != Some(settings)
        || uploads.consumer.as_deref() != Some(consumer_bytes.as_slice());
    {
        let mut inner = account.0.lock().unwrap();
        inner.frames_prepared += light.is_some() as u64;
        inner.casters = draws.draws.len();
        inner.casters_invalid = invalid_now;
        inner.skinned_casters = draws.skinned.len();
        inner.skinned_invalid = draws
            .skinned
            .iter()
            .filter(|item| item.pipeline == CachedRenderPipelineId::INVALID)
            .count();
        inner.skinned_tally = draws.skinned_tally;
        inner.settings = Some(settings);
        inner.casters_seen = draws.casters.seen;
        inner.casters_kept = draws.casters.kept;
        inner.casters_left_out = draws.left_out;
        inner.sky_shell_in_frame = draws.casters.sky_shells > 0;
        inner.casters_overflow = draws.overflow;
        inner.resolution = gpu.size;
        inner.prepares_since += 1;
        inner.frames_written += (frame_written && consumer_written) as u64;
        if let Some(matrices) = &light {
            inner.light_box = Some((matrices.width, matrices.width, matrices.depth));
            inner.world_to_shadow = Some(matrices.consumer);
            inner.caster_bias = Some((
                matrices.width / gpu.size as f32,
                frame_block.shadow_bias.x,
                frame_block.shadow_bias.y,
            ));
            inner.depth_range = matrices.depth;
            inner.penumbra_per_metre = settings.penumbra_per_metre();
        }
        inner.rigid_without_normals = draws
            .draws
            .iter()
            .filter(|item| {
                meshes
                    .get(item.mesh)
                    .is_some_and(|mesh| !mesh.layout.0.contains(Mesh::ATTRIBUTE_NORMAL))
            })
            .count();
    }
    let frame_bytes = frame_bytes_of(&frame_block);
    if frame_written {
        render_queue.write_buffer(&gpu.frame_buffer, 0, &frame_bytes);
        uploads.frame = Some(frame_bytes);
        draws.static_dirty = true;
    }
    if consumer_written {
        render_queue.write_buffer(&gpu.consumer_buffer, 0, &consumer_bytes);
        uploads.consumer = Some(consumer_bytes);
        uploads.consumer_settings = Some(settings);
    }
    if write_objects {
        if !object_bytes.is_empty() {
            render_queue.write_buffer(&gpu.object_buffer, 0, &object_bytes);
        }
        uploads.objects = true;
    }
    // 深度图生成行（一次）：出现即代表光源框解出、矩阵与逐实体矩阵池已
    // 入 GPU，节点将在主 pass 前成图；消费面是站点材质的 binding 10/11/12。
    let written = account.0.lock().unwrap().frames_written;
    if !*readiness_logged && light.is_some() && written >= READY_FRAMES {
        *readiness_logged = true;
        info!(
            "main-light shadow map ready: {}², {} (filter {:?}, strength {}, Pcf radius {} m, sun {}°, caster bias x{} / x{}); rigid casters {} of {} seen ({} left out, sky shell {}), skinned {}; texel {} cm, depth range {} m; consumer block at group 3 binding 10, depth at 11, comparison sampler at 12",
            gpu.size,
            match settings.fit {
                ShadowFit::Camera => "camera fit",
                ShadowFit::Site => "site fit",
            },
            settings.filter,
            settings.strength,
            settings.pcf_radius,
            settings.light_angle,
            settings.depth_bias,
            settings.normal_bias,
            draws.draws.len(),
            draws.casters.seen,
            draws.left_out,
            draws.casters.sky_shells,
            draws.skinned.len(),
            light.as_ref().map_or(0.0, |matrices| matrices.width / gpu.size as f32) * 100.0,
            light.as_ref().map_or(0.0, |matrices| matrices.depth),
        );
    }
    gpu.maps_frames = gpu.maps_frames.saturating_add(1);
    // 节点死锁检测（账目兜底）：pass 一次都没跑过而 prepare 已进行很久，
    // 账目行从 prepare 侧落一次计数，避免「图就绪」行与实际 pass 之间的
    // 缺口无人喊。
    *prepare_frames += 1;
    if [60u64, 600, 1800].contains(&*prepare_frames) {
        let drawn = account.0.lock().unwrap().frames_drawn;
        if drawn == 0 {
            info!(
                "main-light shadow account (prepare's view, frame {}): the pass has still not run (frames_drawn 0) — the node does not run or the whole frame is skipped in silence, and the depth map stays empty; {} casters (no layout that can be drawn: {invalid_now})",
                *prepare_frames,
                draws.draws.len(),
            );
        }
    }
}

/// The depth pass's frame block as bytes (the WGSL `ShadowFrame`'s memory
/// order).
fn frame_bytes_of(frame: &ShadowFrame) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(FRAME_BYTES);
    for matrix in [frame.light_view_proj, frame.world_to_shadow] {
        for column in matrix {
            for component in column.to_array() {
                bytes.extend_from_slice(&component.to_le_bytes());
            }
        }
    }
    for slot in [frame.shadow_bias, frame.light_direction] {
        for component in slot.to_array() {
            bytes.extend_from_slice(&component.to_le_bytes());
        }
    }
    debug_assert_eq!(bytes.len(), FRAME_BYTES);
    bytes
}

/// The consumer block of one light frame: the matrix, the map size, the
/// shadow parameters and the filter constants of `settings`. `depth` is the
/// map's depth range in metres.
fn consumer_block(
    consumer: Mat4,
    width: f32,
    resolution: u32,
    settings: &ShadowSettings,
    depth: f32,
) -> ShadowConsumer {
    // The fade reads the receiver's distance from the camera: the shadow is
    // full up to 90% of the settings' distance and gone at the distance
    // itself.
    let fade_start = FADE_START * settings.distance;
    let fade_end = settings.distance;
    let code = match settings.filter {
        ShadowFilter::Hard => 0.0,
        ShadowFilter::Tent => 1.0,
        ShadowFilter::Pcf => 2.0,
        ShadowFilter::Pcss => 3.0,
    };
    let filtering = Vec4::new(code, fade_start, fade_end, settings.strength);
    // The map has a constant depth range: a metre along the light is `1 /
    // depth` of the map's z, and a metre across the frame is `1 / width` of
    // its uv, so a kernel radius travels as two factors (the Pcss light size
    // is `tan(angle)` times depth over width).
    let kernel = Vec4::new(
        settings.pcf_radius / width,
        settings.penumbra_per_metre() * depth / width,
        settings.max_radius_texels / resolution as f32,
        0.0,
    );
    let depth_range = Vec4::new(1.0 / resolution as f32, 0.0, 0.0, 0.0);
    ShadowConsumer {
        world_to_shadow: consumer,
        size: [
            resolution as f32,
            resolution as f32,
            1.0 / resolution as f32,
            1.0 / resolution as f32,
        ],
        params: [SHADOW_MAIN_LIGHT_INTENSITY, SHADOW_SOFT, 0.0, 0.0],
        filtering,
        kernel,
        depth: depth_range,
    }
}

/// PrepareResources, after [`prepare_shadow_draws`]: which rigid draws the
/// static map holds and whether this frame's node run draws it again
/// (module comment, "Static map").
///
/// The last bake's answer comes first: an answered request is cleared, and
/// the baked draws that bake could not record are drawn per frame from now
/// on (the map lacks them). Then a draw whose mesh was added or modified this
/// frame, or whose mesh is no longer on the GPU, starts over as a per-frame
/// draw; and every draw stable for [`STATIC_AFTER_FRAMES`] extractions whose
/// mesh and buffer slices are present is baked. Any of these, a change seen
/// by the extraction or a new light frame block requests a bake; an
/// unanswered request is requested again.
fn plan_static_bake(
    mut list: ResMut<ShadowDrawList>,
    meshes: Res<RenderAssets<RenderMesh>>,
    allocator: Res<MeshAllocator>,
    extracted: Option<Res<ExtractedAssets<RenderMesh>>>,
    bake: Res<StaticBake>,
) {
    let list = &mut *list;
    {
        let mut answer = bake.0.lock().unwrap();
        if list.static_request.is_some() && answer.answered == list.static_request {
            list.static_request = None;
            for index in answer.skipped.drain(..) {
                if let Some(draw) = list.draws.get_mut(index) {
                    draw.baked = false;
                    draw.stable = 0;
                }
            }
        }
    }
    let touched = extracted
        .as_deref()
        .filter(|extracted| !extracted.added.is_empty() || !extracted.modified.is_empty());
    for draw in &mut list.draws {
        let mesh_changed = touched.is_some_and(|extracted| {
            extracted.added.contains(&draw.mesh) || extracted.modified.contains(&draw.mesh)
        });
        if mesh_changed || (draw.baked && meshes.get(draw.mesh).is_none()) {
            list.static_dirty |= draw.baked;
            draw.baked = false;
            draw.stable = 0;
        }
    }
    for draw in &mut list.draws {
        // The sky shell is never left to the frame's map alone: a camera
        // frame draws it per frame.
        if draw.baked
            || draw.sky_shell
            || draw.stable < STATIC_AFTER_FRAMES
            || draw.pipeline == CachedRenderPipelineId::INVALID
        {
            continue;
        }
        let Some(render_mesh) = meshes.get(draw.mesh) else {
            continue;
        };
        let slices = allocator.mesh_vertex_slice(&draw.mesh).is_some()
            && match render_mesh.buffer_info {
                RenderMeshBufferInfo::Indexed { .. } => allocator.mesh_index_slice(&draw.mesh).is_some(),
                RenderMeshBufferInfo::NonIndexed => true,
            };
        if slices {
            draw.baked = true;
            list.static_dirty = true;
        }
    }
    list.baked_count = list.draws.iter().filter(|draw| draw.baked).count();
    // Every bake answers a request for the list as it is: a skipped draw
    // became a per-frame draw above, and a draw that changed or left since
    // the last bake set `static_dirty`.
    if list.static_dirty || list.static_request.is_some() {
        list.static_generation += 1;
        list.static_request = Some(list.static_generation);
        list.static_dirty = false;
    }
}

/// Records one rigid draw at its object slot into a depth pass, skipping
/// sets that would bind the bound pipeline or buffers again; evaluates to
/// false when the draw's pipeline, mesh or buffer slices are not available.
macro_rules! record_rigid {
    ($pass:expr, $bound:expr, $resolve:expr, $item:expr, $index:expr,
     $bind_group:expr, $stride:expr, $meshes:expr, $allocator:expr) => {{
        let item: &ShadowDraw = $item;
        let bound: &mut BoundState = $bound;
        match ($resolve(item.pipeline), $meshes.get(item.mesh), $allocator.mesh_vertex_slice(&item.mesh)) {
            (Some(pipeline), Some(render_mesh), Some(vertex_slice)) => {
                let index_slice = match &render_mesh.buffer_info {
                    RenderMeshBufferInfo::Indexed { index_format, count } => {
                        $allocator.mesh_index_slice(&item.mesh).map(|slice| Some((slice, *index_format, *count)))
                    }
                    RenderMeshBufferInfo::NonIndexed => Some(None),
                };
                match index_slice {
                    Some(index_slice) => {
                        if bound.pipeline != Some(item.pipeline) {
                            $pass.set_pipeline(pipeline);
                            bound.pipeline = Some(item.pipeline);
                        }
                        $pass.set_bind_group(0, $bind_group, &[($index as u32) * $stride]);
                        if bound.vertex != Some(vertex_slice.buffer.id()) {
                            $pass.set_vertex_buffer(0, *vertex_slice.buffer.slice(..));
                            bound.vertex = Some(vertex_slice.buffer.id());
                        }
                        match index_slice {
                            Some((index_slice, index_format, count)) => {
                                if bound.index != Some((index_slice.buffer.id(), index_format)) {
                                    $pass.set_index_buffer(*index_slice.buffer.slice(..), index_format);
                                    bound.index = Some((index_slice.buffer.id(), index_format));
                                }
                                $pass.draw_indexed(
                                    index_slice.range.start..(index_slice.range.start + count),
                                    vertex_slice.range.start as i32,
                                    0..1,
                                );
                            }
                            None => $pass.draw(vertex_slice.range.clone(), 0..1),
                        }
                        true
                    }
                    None => false,
                }
            }
            _ => false,
        }
    }};
}

/// What a depth pass has bound (see [`record_rigid`]).
#[derive(Default)]
struct BoundState {
    pipeline: Option<CachedRenderPipelineId>,
    vertex: Option<BufferId>,
    index: Option<(BufferId, IndexFormat)>,
}

/// 深度 pass：一次成图，跨视图共享（站点场景只有一台 3D 相机）。
///
/// Its one bind group binds the frame and object buffers, which are replaced
/// only when they grow, so it is cached by those buffer ids.
fn shadow_depth_pass(
    world: &World,
    bind_groups: Local<SharedBindGroupCache>,
    mut render_context: RenderContext,
) {
    // 上一帧的深度图读回先处理（native 诊断；wasm 无阻塞 poll 不做，
    // 模块注释「阴影账目」）：拷贝已随上一帧提交，poll 等完成即 map。
    #[cfg(not(target_arch = "wasm32"))]
    finish_pending_probe(world, &render_context);

    let draws = world.resource::<ShadowDrawList>();
    // With the shadows off nothing is drawn: the static map (and its copy)
    // stays as it is, and no pass runs.
    if draws.draws.is_empty()
        || world
            .get_resource::<ShadowSettingsGlobals>()
            .is_some_and(|settings| !settings.settings.enabled)
    {
        return;
    }
    let gpu = world.resource::<ShadowMapGpu>();
    let pipeline_cache = world.resource::<PipelineCache>();
    let meshes = world.resource::<RenderAssets<RenderMesh>>();
    let allocator = world.resource::<MeshAllocator>();

    // 汇总用到的管线：任一没编完就整帧跳过（weather 同款——首几帧空过）。
    let mut ready: Vec<(CachedRenderPipelineId, &RenderPipeline)> = Vec::new();
    for item in &draws.draws {
        if item.pipeline == CachedRenderPipelineId::INVALID
            || ready.iter().any(|(id, _)| *id == item.pipeline)
        {
            continue;
        }
        let Some(pipeline) = pipeline_cache.get_render_pipeline(item.pipeline) else {
            world
                .resource::<ShadowAccount>()
                .0
                .lock()
                .unwrap()
                .frames_skipped_not_ready += 1;
            return;
        };
        ready.push((item.pipeline, pipeline));
    }
    let resolve = |id: CachedRenderPipelineId| -> Option<&RenderPipeline> {
        ready
            .iter()
            .find(|(candidate, _)| *candidate == id)
            .map(|(_, pipeline)| *pipeline)
    };

    // 帧块 + 矩阵池一个 bind group；逐 draw 用动态 offset 换窗。
    let bind_group = {
        let frame = world.resource::<bevy::diagnostic::FrameCount>().0;
        let mut groups = bind_groups.lock();
        groups.get(
            render_context.render_device(),
            "site_shadow_depth_bind_group",
            &pipeline_cache.get_bind_group_layout(&gpu.depth_layout),
            &[
                (0, Bound::Buffer(&gpu.frame_buffer, 0, None)),
                (
                    1,
                    Bound::Buffer(
                        &gpu.object_buffer,
                        0,
                        Some(std::num::NonZeroU64::new(gpu.object_binding_size).unwrap()),
                    ),
                ),
            ],
            frame,
        )
    };
    // The palette window of the skinned casters, bound at each draw's
    // palette offset.
    let skin_group = (!draws.skinned.is_empty()).then(|| {
        let palette = world.resource::<ShadowPaletteGpu>();
        let frame = world.resource::<bevy::diagnostic::FrameCount>().0;
        bind_groups.lock().get(
            render_context.render_device(),
            "site_shadow_skin_bind_group",
            &pipeline_cache.get_bind_group_layout(&gpu.skin_layout),
            &[(
                0,
                Bound::Buffer(&palette.buffer, 0, std::num::NonZeroU64::new(PALETTE_WINDOW)),
            )],
            frame,
        )
    });

    // The static map (module comment, "Static map"): drawn again when a
    // bake is requested that no run has answered yet, and copied into the
    // frame's map when it holds draws. Until the copy pipeline exists,
    // every rigid draw goes to the frame's map.
    let copy_pipeline = pipeline_cache.get_render_pipeline(gpu.copy_pipeline);
    let mut answer = world.resource::<StaticBake>().0.lock().unwrap();
    let bake_now = copy_pipeline.is_some()
        && draws.static_request.is_some()
        && answer.answered != draws.static_request;
    let use_static = copy_pipeline.is_some() && draws.baked_count > 0;
    if bake_now {
        let mut pass = render_context
            .command_encoder()
            .begin_render_pass(&RenderPassDescriptor {
                label: Some("site_shadow_static_pass"),
                color_attachments: &[],
                depth_stencil_attachment: Some(RenderPassDepthStencilAttachment {
                    view: &gpu.static_view,
                    depth_ops: Some(Operations {
                        load: LoadOp::Clear(1.0),
                        store: StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
        let mut bound = BoundState::default();
        let mut recorded = 0;
        let mut skipped = Vec::new();
        for (index, item) in draws.draws.iter().enumerate() {
            if !item.baked {
                continue;
            }
            if record_rigid!(pass, &mut bound, resolve, item, index, &bind_group,
                gpu.object_stride, meshes, allocator)
            {
                recorded += 1;
            } else {
                skipped.push(index);
            }
        }
        drop(pass);
        answer.answered = draws.static_request;
        answer.recorded = recorded;
        answer.skipped = skipped;
        world.resource::<ShadowAccount>().0.lock().unwrap().static_bakes += 1;
    }
    let static_recorded = if use_static { answer.recorded } else { 0 };
    drop(answer);
    let copy_group = use_static.then(|| {
        let frame = world.resource::<bevy::diagnostic::FrameCount>().0;
        bind_groups.lock().get(
            render_context.render_device(),
            "site_shadow_static_copy_bind_group",
            &pipeline_cache.get_bind_group_layout(&gpu.copy_layout),
            &[
                (0, Bound::View(&gpu.static_view)),
                (1, Bound::Sampler(&gpu.copy_sampler)),
            ],
            frame,
        )
    });

    let mut pass = render_context
        .command_encoder()
        .begin_render_pass(&RenderPassDescriptor {
            label: Some("site_shadow_depth_pass"),
            color_attachments: &[],
            depth_stencil_attachment: Some(RenderPassDepthStencilAttachment {
                view: &gpu.depth_view,
                depth_ops: Some(Operations {
                    load: LoadOp::Clear(1.0),
                    store: StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });

    // Pipeline, vertex buffer and index buffer stay bound across draws of
    // one pass; a set that would bind the same object again is skipped, so
    // every draw still runs with exactly the state it set before.
    let mut bound = BoundState::default();
    if let (Some(copy_pipeline), Some(copy_group)) = (copy_pipeline, &copy_group) {
        pass.set_pipeline(copy_pipeline);
        bound.pipeline = Some(gpu.copy_pipeline);
        pass.set_bind_group(0, copy_group, &[]);
        pass.draw(0..3, 0..1);
    }
    let mut per_frame: usize = 0;
    for (index, item) in draws.draws.iter().enumerate() {
        if use_static && item.baked {
            continue;
        }
        if record_rigid!(pass, &mut bound, resolve, item, index, &bind_group,
            gpu.object_stride, meshes, allocator)
        {
            per_frame += 1;
        }
    }
    let drawn = static_recorded + per_frame;

    // The skinned character casters, after the rigid ones (the depth test
    // makes the order irrelevant). A skinned pipeline still compiling
    // leaves only its own draws out, not the frame.
    let mut skinned_drawn: usize = 0;
    if let Some(skin_group) = &skin_group {
        for item in &draws.skinned {
            if item.pipeline == CachedRenderPipelineId::INVALID {
                continue;
            }
            let (Some(pipeline), Some(render_mesh)) = (
                pipeline_cache.get_render_pipeline(item.pipeline),
                meshes.get(item.mesh),
            ) else {
                continue;
            };
            let Some(vertex_slice) = allocator.mesh_vertex_slice(&item.mesh) else {
                continue;
            };
            if bound.pipeline != Some(item.pipeline) {
                pass.set_pipeline(pipeline);
                bound.pipeline = Some(item.pipeline);
            }
            // The rigid object window is not read by the skinned entry
            // point; offset 0 is a valid window of the pool.
            pass.set_bind_group(0, &bind_group, &[0]);
            pass.set_bind_group(1, skin_group, &[item.palette_offset]);
            if bound.vertex != Some(vertex_slice.buffer.id()) {
                pass.set_vertex_buffer(0, *vertex_slice.buffer.slice(..));
                bound.vertex = Some(vertex_slice.buffer.id());
            }
            match &render_mesh.buffer_info {
                RenderMeshBufferInfo::Indexed { index_format, count } => {
                    let Some(index_slice) = allocator.mesh_index_slice(&item.mesh) else {
                        continue;
                    };
                    if bound.index != Some((index_slice.buffer.id(), *index_format)) {
                        pass.set_index_buffer(*index_slice.buffer.slice(..), *index_format);
                        bound.index = Some((index_slice.buffer.id(), *index_format));
                    }
                    pass.draw_indexed(
                        index_slice.range.start..(index_slice.range.start + *count),
                        vertex_slice.range.start as i32,
                        0..1,
                    );
                }
                RenderMeshBufferInfo::NonIndexed => {
                    pass.draw(vertex_slice.range, 0..1);
                }
            }
            skinned_drawn += 1;
        }
    }
    drop(pass);

    // 阴影账目（节点侧）：帧数与实录 draw 数；到档的帧顺手录一次深度
    // 图拷贝，下一帧节点头读回落账（native；wasm 只落计数行）。
    let probe_due = {
        let mut inner = world.resource::<ShadowAccount>().0.lock().unwrap();
        inner.frames_drawn += 1;
        inner.draws_recorded = drawn;
        inner.static_recorded = static_recorded;
        inner.per_frame_rigid = per_frame;
        inner.skinned_recorded = skinned_drawn;
        if drawn == 0 {
            inner.frames_empty_pass += 1;
        }
        frames_drawn_due(&mut inner)
    };
    if probe_due && drawn > 0 {
        #[cfg(not(target_arch = "wasm32"))]
        record_depth_probe(world, &mut render_context);
        #[cfg(target_arch = "wasm32")]
        log_counter_line(world);
    }
}

/// 读回档期判定与顺延（锁内调用）：首个读回在第 [`PROBE_FIRST_FRAME`]
/// 个成图帧，此后每 [`PROBE_INTERVAL`] 帧一次。0 是未初始化档期的哨兵。
fn frames_drawn_due(inner: &mut AccountInner) -> bool {
    let Some(interval) = probe_interval() else {
        return false;
    };
    if inner.probe_pending {
        return false;
    }
    if inner.next_probe_frame == 0 {
        inner.next_probe_frame = PROBE_FIRST_FRAME;
    }
    if inner.frames_drawn >= inner.next_probe_frame {
        inner.next_probe_frame = inner.frames_drawn + interval;
        true
    } else {
        false
    }
}

/// 录一次深度图读回（native 账目）：深度 pass 之后在同一 encoder 上把
/// 整张图拷进复用 buffer，标记 pending，下一帧节点头 [`finish_pending_probe`]
/// 读回落账。
#[cfg(not(target_arch = "wasm32"))]
fn record_depth_probe(world: &World, render_context: &mut RenderContext) {
    let gpu = world.resource::<ShadowMapGpu>();
    let size = gpu.size;
    let bytes = (size as u64) * (size as u64) * 4;
    let mut inner = world.resource::<ShadowAccount>().0.lock().unwrap();
    // The maps of another side fit another buffer.
    if inner.probe_bytes != bytes {
        inner.probe_buffer = None;
        inner.previous_map = None;
        inner.previous_window = None;
        inner.probe_bytes = bytes;
    }
    let buffer = inner
        .probe_buffer
        .get_or_insert_with(|| {
            render_context.render_device().create_buffer(&BufferDescriptor {
                label: Some("site_shadow_probe_readback"),
                size: bytes,
                usage: BufferUsages::MAP_READ | BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        })
        .clone();
    render_context
        .command_encoder()
        .copy_texture_to_buffer(
            gpu.depth_texture.as_image_copy(),
            TexelCopyBufferInfo {
                buffer: &buffer,
                layout: TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(size * 4),
                    rows_per_image: None,
                },
            },
            Extent3d {
                width: size,
                height: size,
                depth_or_array_layers: 1,
            },
        );
    inner.probe_pending = true;
}

/// 读回落账（native 账目）：map + 阻塞 poll 等拷贝完成，随后把
/// 「pass 跑没跑、图里有没有内容、内容对不对得上 caster、单 texel 比较
/// 命中分布」四件事并排落一行。四态在画面上同形（模块注释「阴影账目」），
/// 行里的数逐一区分它们：
/// - pass 没跑：`drawn` 帧数 0 / `draw` 实录 0；
/// - 图里没内容：`非清` texel 数 0；
/// - 投影对不上：`出框` 原点数高或 `邻域有图` 低（矩阵/框错位）；
/// - 比较全亮：`lit vs 影` 里影一路为 0（偏置或 z 折叠错向）。
#[cfg(not(target_arch = "wasm32"))]
fn finish_pending_probe(world: &World, render_context: &RenderContext) {
    let mut inner = world.resource::<ShadowAccount>().0.lock().unwrap();
    if !inner.probe_pending {
        return;
    }
    inner.probe_pending = false;
    let Some(buffer) = inner.probe_buffer.clone() else {
        return;
    };
    let (sender, receiver) = std::sync::mpsc::channel();
    buffer.slice(..).map_async(MapMode::Read, move |result| {
        let _ = sender.send(result);
    });
    if render_context
        .render_device()
        .poll(PollType::wait_indefinitely())
        .is_err()
    {
        warn!("shadow account: the depth map read-back's poll failed; this read-back is dropped");
        return;
    }
    if !matches!(receiver.recv(), Ok(Ok(()))) {
        warn!("shadow account: the depth map read-back's map failed; this read-back is dropped");
        return;
    }

    // 深度统计：非清 texel 数（clear 是 1.0，光栅只会更小）与最小深度。
    let Ok(data) = buffer.slice(..).get_mapped_range() else {
        warn!("shadow account: the depth map read-back has no mapped range; this read-back is dropped");
        return;
    };
    // A read-back recorded before a resolution change holds the old map: its
    // bytes no longer match the map's side, and it is dropped.
    if data.len() as u64 != inner.probe_bytes {
        warn!(
            "shadow account: the read-back holds {} bytes but the map has {}; this read-back is dropped",
            data.len(),
            inner.probe_bytes
        );
        return;
    }
    let size = ((inner.probe_bytes / 4) as f64).sqrt() as usize;
    let side = size as i64;
    let texel = |x: usize, y: usize| -> f32 {
        let i = (y * size + x) * 4;
        f32::from_le_bytes(data[i..i + 4].try_into().unwrap())
    };
    let mut nonclear = 0usize;
    let mut min_depth = 1.0f32;
    for chunk in data.as_chunks::<4>().0 {
        let d = f32::from_le_bytes(*chunk);
        if d < PROBE_CLEAR_EPS {
            nonclear += 1;
            min_depth = min_depth.min(d);
        }
    }

    // caster 原点对齐：把每个 caster 的世界原点过消费矩阵（和站点材质同
    // 一条路），看图里有没有它、单 texel 比较哪边亮。名单是本帧的，矩阵
    // 是成图那一帧的——站点场景静态，跨帧差可忽略。
    let (mut origins, mut out_of_range, mut near_nonclear) = (0usize, 0usize, 0usize);
    let (mut lit, mut shadowed, mut max_gap) = (0usize, 0usize, 0.0f32);
    if let Some(world_to_shadow) = inner.world_to_shadow {
        let draws = world.resource::<ShadowDrawList>();
        for item in &draws.draws {
            let clip = world_to_shadow.transform_point3(item.world.w_axis.truncate());
            origins += 1;
            if !(0.0..1.0).contains(&clip.x)
                || !(0.0..1.0).contains(&clip.y)
                || !(0.0..1.0).contains(&clip.z)
            {
                out_of_range += 1;
            }
            let tx = ((clip.x * side as f32) as i64).clamp(0, side - 1) as usize;
            let ty = ((clip.y * side as f32) as i64).clamp(0, side - 1) as usize;
            let mut hit = false;
            for dy in -1i64..=1 {
                for dx in -1i64..=1 {
                    let (x, y) = (tx as i64 + dx, ty as i64 + dy);
                    if x >= 0 && y >= 0 && x < side && y < side {
                        hit |= texel(x as usize, y as usize) < PROBE_CLEAR_EPS;
                    }
                }
            }
            near_nonclear += hit as usize;
            // 单 texel 比较，方向与消费口一致：存储深度 ≥ 原点深度 → 点亮。
            let stored = texel(tx, ty);
            if clip.z <= stored {
                lit += 1;
            } else {
                shadowed += 1;
            }
            max_gap = max_gap.max((stored - clip.z).abs());
        }
    }
    let (box_w, _box_h, box_d) = inner.light_box.unwrap_or((0.0, 0.0, 0.0));
    let caster_bias = inner.caster_bias.unwrap_or((0.0, 0.0, 0.0));
    let total_texels = (inner.probe_bytes / 4) as f32;

    // The player's caster window: the texel rectangle the player's skinned
    // bounds project to. Its texels are compared with the previous read-back
    // over the union of this and the previous window, and the rest of the map
    // is compared too, so a change inside the window with the player's bounds
    // centre in place is the animation moving the caster.
    let draws = world.resource::<ShadowDrawList>();
    let player_bounds = draws
        .skinned
        .iter()
        .filter(|item| item.player)
        .map(|item| item.bounds)
        .reduce(|a, b| (a.0.min(b.0), a.1.max(b.1)));
    let window = player_bounds
        .zip(inner.world_to_shadow)
        .map(|((low, high), to_shadow)| {
            let (mut x0, mut y0, mut x1, mut y1) = (size, size, 0usize, 0usize);
            for sx in [low.x, high.x] {
                for sy in [low.y, high.y] {
                    for sz in [low.z, high.z] {
                        let uv = to_shadow.transform_point3(Vec3::new(sx, sy, sz));
                        let tx = ((uv.x * size as f32) as i64).clamp(0, side - 1) as usize;
                        let ty = ((uv.y * size as f32) as i64).clamp(0, side - 1) as usize;
                        x0 = x0.min(tx);
                        y0 = y0.min(ty);
                        x1 = x1.max(tx);
                        y1 = y1.max(ty);
                    }
                }
            }
            (x0, y0, x1, y1)
        });
    let mut window_nonclear = 0usize;
    let mut window_min = 1.0f32;
    if let Some((x0, y0, x1, y1)) = window {
        for y in y0..=y1 {
            for x in x0..=x1 {
                let d = texel(x, y);
                if d < PROBE_CLEAR_EPS {
                    window_nonclear += 1;
                    window_min = window_min.min(d);
                }
            }
        }
    }
    let union = match (window, inner.previous_window) {
        (Some(a), Some(b)) => Some((a.0.min(b.0), a.1.min(b.1), a.2.max(b.2), a.3.max(b.3))),
        (a, b) => a.or(b),
    };
    let (mut changed_in, mut changed_out) = (None, None);
    if let Some(previous) = inner.previous_map.as_deref() {
        let (mut inside, mut outside) = (0usize, 0usize);
        for (index, (now, before)) in data
            .as_chunks::<4>()
            .0
            .iter()
            .zip(previous.as_chunks::<4>().0)
            .enumerate()
        {
            if now != before {
                let (x, y) = (index % size, index / size);
                let within =
                    union.is_some_and(|(x0, y0, x1, y1)| (x0..=x1).contains(&x) && (y0..=y1).contains(&y));
                if within {
                    inside += 1;
                } else {
                    outside += 1;
                }
            }
        }
        changed_in = Some(inside);
        changed_out = Some(outside);
    }
    let mut copy = inner.previous_map.take().unwrap_or_default();
    copy.clear();
    copy.extend_from_slice(&data);
    inner.previous_map = Some(copy);
    inner.previous_window = window;
    drop(data);
    buffer.unmap();
    let tally = inner.skinned_tally;
    let (centre, extent) = player_bounds
        .map(|(low, high)| ((low + high) * 0.5, high - low))
        .unwrap_or((Vec3::ZERO, Vec3::ZERO));
    info!(
        "shadow account, skinned casters: {} drawn {} (player {}, NPC {}; NPC Off at dither alpha 0: {}; other skinned meshes not casting: {}; layout without normal/joint streams {}); palettes {}, joints {} (joint world transform x inverse bind pose, this frame); rigid casters {}, total casters {}; player window {:?}: nonclear {} texels, min depth {:.4}, changed since last read-back {:?} (rest of map {:?}); player bounds centre ({:.3}, {:.3}, {:.3}) size ({:.3}, {:.3}, {:.3}); caster bias (all casters) from pipeline 1.0/1.0 x texel x 2.5: texel {:.6} m, depth {:.6} m, normal {:.6} m; raster bias 1 / 2.5; rigid casters without normals {}; static map: {} rigid draws as of its last bake, {} rigid draws per frame, {} bakes",
        inner.skinned_casters,
        inner.skinned_recorded,
        tally.player,
        tally.npc,
        tally.npc_off_by_dither,
        tally.other_skinned,
        inner.skinned_invalid,
        tally.palettes,
        tally.joints,
        inner.casters,
        inner.casters + inner.skinned_casters,
        window,
        window_nonclear,
        window_min,
        changed_in,
        changed_out,
        centre.x,
        centre.y,
        centre.z,
        extent.x,
        extent.y,
        extent.z,
        caster_bias.0,
        caster_bias.1,
        caster_bias.2,
        inner.rigid_without_normals,
        inner.static_recorded,
        inner.per_frame_rigid,
        inner.static_bakes,
    );
    info!(
        "shadow account: settings {}; frames prepared {} / drawn {} (skipped, not compiled {}; empty {}); casters {} (no layout that can be drawn {}, draws recorded {}); light box {box_w:.1} x {box_w:.1} m deep {box_d:.1} m ({}², texel {:.2} cm); nonclear texels {} ({:.2}%), min depth {min_depth:.4}; caster origins: projected {origins}, out of the frame {out_of_range}, with content near them {near_nonclear}; single texel comparison lit {lit} vs shadowed {shadowed}, largest depth gap {max_gap:.4}",
        inner
            .settings
            .map(|settings| format!("{settings:?}"))
            .unwrap_or_else(|| "none yet".to_owned()),
        inner.frames_prepared,
        inner.frames_drawn,
        inner.frames_skipped_not_ready,
        inner.frames_empty_pass,
        inner.casters,
        inner.casters_invalid,
        inner.draws_recorded,
        size,
        box_w * 100.0 / size.max(1) as f32,
        nonclear,
        nonclear as f32 * 100.0 / total_texels.max(1.0),
    );
}

/// wasm 侧账目行（无阻塞读回）：只落计数，四态里能区分「跑没跑 / 画没
/// 画」，图内容与对齐在 native 侧验。
#[cfg(target_arch = "wasm32")]
fn log_counter_line(world: &World) {
    let inner = world.resource::<ShadowAccount>().0.lock().unwrap();
    info!(
        "shadow account (browser build, counts): frames prepared {} / drawn {} (skipped, not compiled {}; empty {}); casters {} (no layout that can be drawn {}, draws recorded {}); skinned casters {} drawn {} (player {}, NPC {}, NPC Off at dither 0 {}, other skinned {}, layout invalid {}; palettes {}, joints {}); static map: {} rigid draws as of its last bake, {} rigid draws per frame, {} bakes",
        inner.frames_prepared,
        inner.frames_drawn,
        inner.frames_skipped_not_ready,
        inner.frames_empty_pass,
        inner.casters,
        inner.casters_invalid,
        inner.draws_recorded,
        inner.skinned_casters,
        inner.skinned_recorded,
        inner.skinned_tally.player,
        inner.skinned_tally.npc,
        inner.skinned_tally.npc_off_by_dither,
        inner.skinned_tally.other_skinned,
        inner.skinned_invalid,
        inner.skinned_tally.palettes,
        inner.skinned_tally.joints,
        inner.static_recorded,
        inner.per_frame_rigid,
        inner.static_bakes,
    );
}

/// `ExtractSchedule`, after the settings are copied: re-creates the depth maps
/// when the settings' resolution differs from the maps' side. It runs during
/// extraction so that the material bind groups prepared later on the same
/// frame (marked changed by [`mark_materials_on_resolution`]) bind the new
/// views.
fn apply_shadow_resolution(
    settings: Res<ShadowSettingsGlobals>,
    render_device: Res<RenderDevice>,
    pipeline_cache: Res<PipelineCache>,
    gpu: Option<ResMut<ShadowMapGpu>>,
    account: Res<ShadowAccount>,
) {
    let Some(mut gpu) = gpu else {
        return;
    };
    let size = settings.settings.resolution;
    if gpu.size != size {
        rebuild_shadow_maps(&render_device, &pipeline_cache, &mut gpu, &account, size);
    }
}

/// Main world: marks every material of type `M` changed on the frame the
/// shadow resolution changes. Each such material's bind group holds the depth
/// map's view, which `apply_shadow_resolution` replaces during that frame's
/// extraction; a material marked changed is extracted and prepared again on
/// the same frame, after the new maps exist. The first frame only records the
/// resolution: the maps were created at it.
pub fn mark_materials_on_resolution<M: Asset>(
    settings: Res<ShadowSettings>,
    mut last: Local<Option<u32>>,
    materials: Option<ResMut<Assets<M>>>,
) {
    let resolution = settings.resolution;
    let previous = last.replace(resolution);
    if previous.is_none_or(|previous| previous == resolution) {
        return;
    }
    let Some(mut materials) = materials else {
        return;
    };
    let ids: Vec<AssetId<M>> = materials.ids().collect();
    for id in ids {
        // The asset is marked modified when the borrow is taken out of its
        // guard; holding the guard alone marks nothing.
        if let Some(material) = materials.get_mut(id) {
            material.into_inner();
        }
    }
}

/// The maps, views and samplers of one resolution.
struct ShadowMaps {
    size: u32,
    depth_texture: Texture,
    depth_view: TextureView,
    static_view: TextureView,
    cmp_sampler: Sampler,
}

/// Creates the depth maps of `size` and their samplers.
fn create_maps(render_device: &RenderDevice, size: u32) -> ShadowMaps {
    let texture = render_device.create_texture(&TextureDescriptor {
        label: Some("site_shadow_depth_texture"),
        size: Extent3d {
            width: size,
            height: size,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: TextureDimension::D2,
        format: TextureFormat::Depth32Float,
        usage: TextureUsages::RENDER_ATTACHMENT
            | TextureUsages::TEXTURE_BINDING
            | TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let depth_view = texture.create_view(&TextureViewDescriptor {
        dimension: Some(TextureViewDimension::D2),
        ..Default::default()
    });
    let static_view = render_device
        .create_texture(&TextureDescriptor {
            label: Some("site_shadow_static_depth_texture"),
            size: Extent3d {
                width: size,
                height: size,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: TextureFormat::Depth32Float,
            usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        })
        .create_view(&TextureViewDescriptor {
            dimension: Some(TextureViewDimension::D2),
            ..Default::default()
        });
    // The comparison sampler: a bilinear compare (each tap compares the four
    // texels around its point and interpolates), `LessEqual` = a stored depth
    // at or behind the fragment is lit (the map's z grows with distance).
    let cmp_sampler = render_device.create_sampler(&SamplerDescriptor {
        address_mode_u: AddressMode::ClampToEdge,
        address_mode_v: AddressMode::ClampToEdge,
        mag_filter: FilterMode::Linear,
        min_filter: FilterMode::Linear,
        mipmap_filter: MipmapFilterMode::Nearest,
        compare: Some(CompareFunction::LessEqual),
        ..Default::default()
    });
    ShadowMaps {
        size,
        depth_texture: texture,
        depth_view,
        static_view,
        cmp_sampler,
    }
}

/// The static map's copy: its bind group layout, sampler and pipeline. Every
/// texel is written with the stored depth (compare Always, no raster bias:
/// the depth comes from the program, not the rasteriser).
fn create_copy(
    render_device: &RenderDevice,
    pipeline_cache: &PipelineCache,
) -> (BindGroupLayoutDescriptor, Sampler, CachedRenderPipelineId) {
    // The copy reads the static map as an unfilterable float texture through a
    // nearest, non-comparison sampler (shaders/shadow_depth_copy.wgsl).
    let copy_layout = BindGroupLayoutDescriptor::new(
        "site_shadow_static_copy_layout",
        &[
            BindGroupLayoutEntry {
                binding: 0,
                visibility: ShaderStages::FRAGMENT,
                ty: BindingType::Texture {
                    sample_type: TextureSampleType::Float { filterable: false },
                    view_dimension: TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            BindGroupLayoutEntry {
                binding: 1,
                visibility: ShaderStages::FRAGMENT,
                ty: BindingType::Sampler(SamplerBindingType::NonFiltering),
                count: None,
            },
        ],
    );
    let copy_sampler = crate::weather_depth::raw_depth_sampler(render_device);
    let copy_pipeline = pipeline_cache.queue_render_pipeline(RenderPipelineDescriptor {
        label: Some("site_shadow_static_copy_pipeline".into()),
        layout: vec![copy_layout.clone()],
        vertex: VertexState {
            shader: SHADOW_COPY_SHADER.clone(),
            entry_point: Some("vertex".into()),
            ..Default::default()
        },
        fragment: Some(FragmentState {
            shader: SHADOW_COPY_SHADER.clone(),
            entry_point: Some("fragment".into()),
            targets: vec![],
            ..Default::default()
        }),
        primitive: PrimitiveState {
            cull_mode: None,
            ..Default::default()
        },
        depth_stencil: Some(DepthStencilState {
            format: TextureFormat::Depth32Float,
            depth_write_enabled: Some(true),
            depth_compare: Some(CompareFunction::Always),
            stencil: StencilState::default(),
            bias: DepthBiasState::default(),
        }),
        multisample: MultisampleState::default(),
        ..Default::default()
    });
    (copy_layout, copy_sampler, copy_pipeline)
}

/// Replaces the depth maps with maps of `size`: the frame's and the static
/// map's textures and views, their samplers, and the read-back sized by the
/// old side. The buffers and layouts are not sized by the map and stay.
///
/// Every material bind group that binds the maps holds the old depth view;
/// those materials are marked changed in the main world on the same frame
/// ([`mark_materials_on_resolution`]) and prepared again after this runs.
fn rebuild_shadow_maps(
    render_device: &RenderDevice,
    pipeline_cache: &PipelineCache,
    gpu: &mut ShadowMapGpu,
    account: &ShadowAccount,
    size: u32,
) {
    let maps = create_maps(render_device, size);
    let (copy_layout, copy_sampler, copy_pipeline) = create_copy(render_device, pipeline_cache);
    gpu.size = maps.size;
    gpu.depth_texture = maps.depth_texture;
    gpu.depth_view = maps.depth_view;
    gpu.static_view = maps.static_view;
    gpu.cmp_sampler = maps.cmp_sampler;
    gpu.copy_layout = copy_layout;
    gpu.copy_sampler = copy_sampler;
    gpu.copy_pipeline = copy_pipeline;
    gpu.maps_frames = 0;
    let mut inner = account.0.lock().unwrap();
    // A read-back of the old side is dropped with it.
    inner.probe_pending = false;
    #[cfg(not(target_arch = "wasm32"))]
    {
        inner.probe_buffer = None;
        inner.previous_map = None;
        inner.previous_window = None;
        inner.probe_bytes = 0;
    }
    inner.next_probe_frame = 0;
}

/// RenderStartup: the depth maps, the comparison sampler, the three buffers
/// and the group 0 layout.
fn init_shadow_resources(
    mut commands: Commands,
    render_device: Res<RenderDevice>,
    pipeline_cache: Res<PipelineCache>,
    settings: Option<Res<ShadowSettingsGlobals>>,
) {
    // The maps' side is the settings' resolution; without them yet, the
    // default, which the first prepare applies.
    let size = settings
        .map(|settings| settings.settings.resolution)
        .unwrap_or_else(|| ShadowSettings::default().resolution);
    let maps = create_maps(&render_device, size);
    let (copy_layout, copy_sampler, copy_pipeline) = create_copy(&render_device, &pipeline_cache);
    let consumer_buffer = render_device.create_buffer(&BufferDescriptor {
        label: Some("site_shadow_consumer"),
        size: CONSUMER_BYTES as u64,
        usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let frame_buffer = render_device.create_buffer(&BufferDescriptor {
        label: Some("site_shadow_frame"),
        size: FRAME_BYTES as u64,
        usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    // 动态 offset 槽距按设备对齐（通常 256）。
    let align = render_device
        .limits()
        .min_uniform_buffer_offset_alignment
        .max(64);
    let object_buffer = render_device.create_buffer(&BufferDescriptor {
        label: Some("site_shadow_objects"),
        size: (align as u64) * (OBJECT_CAPACITY as u64),
        usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let depth_layout = BindGroupLayoutDescriptor::new(
        "site_shadow_depth_layout",
        &BindGroupLayoutEntries::with_indices(
            ShaderStages::VERTEX,
            (
                (0, uniform_buffer::<ShadowFrame>(false)),
                (1, uniform_buffer::<[Vec4; 4]>(true)),
            ),
        ),
    );
    let skin_layout = BindGroupLayoutDescriptor::new(
        "site_shadow_skin_layout",
        &BindGroupLayoutEntries::with_indices(
            ShaderStages::VERTEX,
            ((
                0,
                uniform_buffer_sized(true, std::num::NonZeroU64::new(PALETTE_WINDOW)),
            ),),
        ),
    );
    commands.insert_resource(ShadowPaletteGpu {
        buffer: render_device.create_buffer(&BufferDescriptor {
            label: Some("site_shadow_palettes"),
            size: PALETTE_WINDOW,
            usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        }),
        capacity: PALETTE_WINDOW,
    });
    commands.insert_resource(ShadowMapGpu {
        size: maps.size,
        depth_texture: maps.depth_texture,
        depth_view: maps.depth_view,
        cmp_sampler: maps.cmp_sampler,
        maps_frames: 0,
        consumer_buffer,
        frame_buffer,
        object_buffer,
        object_stride: align,
        object_binding_size: std::mem::size_of::<[Vec4; 4]>() as u64,
        depth_layout,
        skin_layout,
        static_view: maps.static_view,
        copy_layout,
        copy_sampler,
        copy_pipeline,
    });
}

/// Startup: the two programs of the shadow map enter the shader table (the
/// same shape as weather.rs; the repository has no default asset source
/// directory to load them from). The consumers' programs (the site material's
/// in site_material.rs, the room shell's in room_shell.rs) are inserted the
/// same way by the crates that hold their paths.
fn load(mut shaders: ResMut<Assets<Shader>>) {
    let _ = shaders.insert(
        SHADOW_DEPTH_SHADER.id(),
        Shader::from_wgsl(
            include_str!("shaders/shadow_depth.wgsl"),
            "moly_render/src/shaders/shadow_depth.wgsl".to_owned(),
        ),
    );
    let _ = shaders.insert(
        SHADOW_COPY_SHADER.id(),
        Shader::from_wgsl(
            include_str!("shaders/shadow_depth_copy.wgsl"),
            "moly_render/src/shaders/shadow_depth_copy.wgsl".to_owned(),
        ),
    );
}

/// 主光阴影深度图插件：消费面（站点材质）在 native 与 wasm 都在，两侧都装。
pub struct ShadowmapPlugin;

impl Plugin for ShadowmapPlugin {
    fn build(&self, app: &mut App) {
        crate::render::gpu::install_bind_group_caches(app);
        app.add_plugins(ShadowSettingsPlugin)
            .add_systems(Startup, load)
            .add_systems(Last, mark_materials_on_resolution::<SiteMaterial>);
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
            return;
        };
        render_app
            .init_resource::<ShadowDrawList>()
            .init_resource::<DepthPipelines>()
            .init_resource::<ShadowAccount>()
            .init_resource::<StaticBake>()
            .add_systems(RenderStartup, init_shadow_resources)
            .add_systems(Render, fit_camera_frame.in_set(RenderSystems::Prepare).before(prepare_shadow_draws))
            .add_systems(
                Render,
                (
                    prepare_shadow_draws,
                    plan_static_bake.after(prepare_shadow_draws),
                )
                    .in_set(RenderSystems::PrepareResources),
            )
            .add_systems(
                ExtractSchedule,
                (extract_shadow_casters, extract_skinned_casters).chain(),
            )
            .add_systems(
                ExtractSchedule,
                apply_shadow_resolution.after(crate::shadow_settings::extract_shadow_settings),
            )
            .add_systems(
                Core3d,
                shadow_depth_pass
                    .after(Core3dSystems::Prepass)
                    .before(Core3dSystems::MainPass),
            );
    }
}
