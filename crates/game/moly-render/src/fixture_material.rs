//! Furniture Basic / Rug / Fence materials: the Bevy material of the
//! `Mysekai/Fixture/Basic` base program.
//!
//! Material values come from `GltfMaterialExtras` on the entities expanded
//! from a glb scene (the extractor writes the Unity properties into the glTF
//! material extras: `floats`, a table of floats; the top-level
//! `fixtureShaderUsage`; the `shader` name) and from `GltfMaterialName`; the
//! main texture handle is the glb material's own `baseColorTexture` (the
//! extractor writes `_MainTex` into it). The variant is encoded into the
//! pipeline specialization by `FixtureMaterialKey`; the WGSL is in
//! `shaders/fixture_material.wesl`.
//!
//! Refusal semantics: a failed family gate (closed set of shader names plus
//! the named URP/Lit refusal), a missing required float, an out-of-range
//! (usage, blend) table selection or one contradicting the blend mode, a glb
//! material without a main texture — each is refused by material name, and
//! the entity keeps its original material rather than a default-based
//! approximation.
//! Exception for the main texture: when the source slot is empty (the
//! extras carry no `_MainTex` key), the engine binds the shader's declared
//! default, and a `white` default is bound as a 1x1 white image. A key with a
//! null value is a texture bound in a dependency package that the extractor
//! did not write; that stays a named refusal.
//!
//! Crystal variant (`_ENABLE_CRYSTAL_FIXTURE`, together with the shader's
//! normal-map block): see `FIXTURE_CRYSTAL` / `FIXTURE_NORMAL_MAP` in the
//! WGSL. Its second colour target (the colour-picker emission, the
//! normal-driven emission and the centre damp) is drawn by the emission pass
//! from the same [`CrystalParams`]; a blocked crystal draws no second target.
//!
//! `_ENABLE_MULTI_UV_SCROLL` variant: the base colour is not `_MainTex`
//! but up to three `_LayerTex*` layers. `_LayerTex1` is always on;
//! `_UseLayerTex2/3` gate the others, and a gated-off layer's slot is
//! never read, so it binds the engine fallback. The ST comes from the
//! material's own `textureScaleOffset`.
//! Families inside the closed set but not Basic (ShadowMesh / Road / Object
//! and so on) are out of scope: they are counted by name and keep their
//! original material; that is not a refusal. Their pixel contribution is
//! always zero, so the exceptions are hidden rather than kept:
//! - a pass whose render state writes nothing (see [`StencilOnlyPass`]):
//!   this pass makes the mesh invisible in the game, and hiding it per the
//!   Unity render state avoids drawing it as a white surface.
//! - the ShadowMesh family (see [`ShadowMeshGated`]): its fragment gates on
//!   the material float `_Show` and discards everything below 0.5; `_Show`
//!   is 0.0 on every material of that family in live data (the extractor
//!   copies the floats as authored), and none of the 1,429
//!   AnimationClip float curves targets it, so nothing rewrites it at run
//!   time.
//!
//! Window-outside arm (`fixtureShaderUsage` = 1 → table selection 26/27):
//! the game clips it to the window opening with the stencil buffer (the
//! window package's own zero-write surface writes ref4 over the opening
//! first, and the appearance mesh draws only inside it at Equal ref4). This
//! pipeline has no stencil buffer, so the equivalent gate runs in the
//! fragment: the zero-write surface is a planar convex quad, the vertex
//! stage projects the four corners and the current vertex to NDC, the
//! fragment tests "point inside the convex quad" and discards outside the
//! window. At swap time the four corners are read from the zero-write
//! surface mesh of the same glb (both nodes have identity local
//! transforms, so the corners are the original vertex positions) and baked
//! into the last four slots of the material uniform.
//!
//! Variant switch panel: the top-level `validKeywords` list of the glb
//! extras is the only source of the game's runtime shader variant switches
//! (the game re-derives the keywords from integer properties when the
//! material is built; the `invalidKeywords` list is an editor leftover of
//! keywords once valid and now disabled, and is not read at run time — the
//! tool-generated `_SHOW_*` preview gates all live there and are never
//! consumed). This pipeline handles them in three groups:
//! - `_USE_ALPHA_CLIP` / `_DISABLE_DITHER` / `_ENABLE_MODULE_FRESNEL` /
//!   `_ENABLE_MODULE_REFLECTION`: the four implemented compile-time variants
//!   (alpha discard / Bayer dither block / fresnel additive colour /
//!   reflection additive colour), each toggled by the presence of its
//!   keyword (the reflection one has an additional data-side condition, see
//!   below);
//! - `_RECEIVE_SHADOWS_OFF`: byte-for-byte identical output in every
//!   compiled combination of the Basic program — no variant of that program
//!   samples the main-light shadow map, so the switch is the engine's "does
//!   not receive shadows" marker with no program-level effect; this pipeline
//!   has the same shape (no shadow-receiving pass) and only accounts for the
//!   switch without changing the image;
//! - every other keyword: accounted per material by name (swap log + count),
//!   never swallowed silently, see the accounting list.
//!
//! The texture branch has parallel additive arms, each with its own keyword,
//! usable alone or together. The rules and the bitwise comparison are in
//! `moly_law::fixture::fresnel`.
//!
//! fresnel (`_ENABLE_MODULE_FRESNEL`): the variant with the switch on adds
//! to rgb after the treasure shadow and before fog
//! exp2(log2(clamp(1−N·V, 0, 1)) × `_FresnelPower`)
//! × `_FresnelColor.rgb` × `_FresnelColor.a` — clamp first, then take the
//! logarithm (when N·V slightly exceeds 1, a negative number reaches log2
//! and yields NaN), and alpha is a factor, not decoration. Perspective (camera
//! position minus fragment position, normalized, with no epsilon clamp) and
//! orthographic (the third row of the
//! view matrix, when unity_OrthoParams.w is non-zero). ⚠ All 3 live materials
//! that carry it belong to vehicles (`*_ref` of bike/car/flyingcar) and
//! **none is in the placement table** — its effect is not visible in the
//! current scene; it is ready for vehicle placement.
//!
//! reflection (`_ENABLE_MODULE_REFLECTION`): right after the fresnel arm and
//! before fog, adds min(pow(1−N·V, `_ReflectionFresnelPower`), 1) × cube-map
//! sample × `_ReflectionIntensity`. Three points must be kept: this arm's
//! log2 **input is not clamped** (unlike the fresnel arm), only the upper
//! clamp min(·, 1) applies; all arms **share one 1−N·V** (in a variant
//! the clamped copy and reflection the unclamped original); the
//! cube-map term **folds to a constant** — the material binds no object to
//! that slot, so the engine substitutes its built-in default cube map, and
//! that map has the same colour on all six faces ⇒ the sampled value does not
//! depend on direction, mip or mip bias ⇒ the reflection vector is never
//! computed in this pipeline.
//! ⚠ **The fold holds only for materials whose slot is empty**: 19 live
//! materials have this branch on, 16 of them (the egg family, all in the
//! placement table) have an empty reference in the slot ⇒ folded form; the
//! 3 (`*_ref` of the vehicles) bind a real cube map ⇒ the fold is wrong for
//! them, so **the branch stays off and they are accounted by name** (one
//! named warning line at the end of the swap), and the path with a real
//! sample is not implemented; none of them is placed either. The check is
//! whether the `_ReflectionCubeMap` **key is present** in the texture
//! reference table the extractor writes into the extras, which agrees
//! entry by entry with the empty/non-empty object reference in the game.
//!
//! Unsupported list (consumers the Basic program has and this pipeline does
//! not):
//!
//! Decisions (present in the data, not wired into the program, and why):
//! - Vertex colour blending is not wired: neither the fragment nor the vertex
//!   program of the Basic family consumes COLOR0. The vertex COLOR_0
//!   attribute in the glb is a data surface the extractor provides uniformly
//!   for the whole family; "a live value exists in the data" does not mean
//!   "the program consumes it". Positive controls are other families, given
//!   as (attributes reaching COLOR0, float attributes in total): FieldObject
//!   12 of 14 and Tree 56 of 59
//!   reach COLOR0 semantics — those programs sit on vertex colour,
//!   Basic does not; wiring it into Basic would add a multiplication to the
//!   fragment that the game does not perform.
//! - `_NormalShadingIntensity` / `_ShadingNormalBlend` / bare `_Edge*` /
//!   `_ReceiveShadow`: in the compiled Basic program the translator gives
//!   them a "declared but unreferenced" renamed prefix (a dead-uniform
//!   marker, shipped with the program). They are accounted by dead key and
//!   never enter the pipeline.
//!
//! The second colour target (SV_Target1 of the Basic program = emission mask
//! × phenomena emission switch chain) is provided by a dedicated emission
//! pass ([`crate::fixture_emission`]): for type 1 the gate is bright==1, for
//! type 2 the gate is dark==1, otherwise 0; the alpha of both targets equals
//! the main texture alpha. Three material ints (`_BrightPhenomenaEmission` /
//! `_DarkPhenomenaEmission` / `_PhenomenaEmissionOverrideMode`) and the
//! `_EmissionMaskTex` texture are read at swap time: the gate ints enter
//! the [`FixtureEmission`] component (consumed by the shader), the mask mean
//! into [`EmissionAccount`] (the initial material account). The initial
//! override is 0; gimmick animation events can force it on or off on an
//! actual instance, and that dynamic value goes through the emission pass's
//! per-instance attribute, not inferred from the initial material table,
//! which has no writer. A material with a missing mask texture and non-zero
//! gate ints is treated as zero contribution with a named warning
//! (fail-closed: never a silently sampled blank mask).
//!
//! Unsupported list (consumers the Basic program has and this pipeline does
//! not):
//! - Wall layout turns ShadowCaster off at run time: in the game each
//!   material's ShadowCaster pass is not drawn while a wall layout is active;
//!   this pipeline has no shadow pass (prepass and shadow receiving are off,
//!   the same shape as the site family), and the marker component is
//!   `WallLayoutShadowCasterOff` (defined in `fixture.rs`).

#[path = "fixture_surfaces.rs"]
pub mod surfaces;

use std::collections::HashMap;

use bevy::asset::LoadState;
use bevy::ecs::system::lifetimeless::SRes;
use bevy::ecs::system::SystemParamItem;
use bevy::gltf::{GltfMaterialExtras, GltfMaterialName};
use bevy::image::Image;
use bevy::pbr::{Material, MaterialPipeline, MaterialPipelineKey, MaterialPlugin, StandardMaterial};
use bevy::prelude::*;
use bevy::reflect::TypePath;
use bevy::render::render_asset::RenderAssets;
use bevy::render::render_resource::*;
use bevy::render::renderer::RenderDevice;
use bevy::render::texture::GpuImage;
use bevy::shader::ShaderRef;
use moly_assets::material_textures::SourceMaterialTextures;
use moly_law::fixture::family::{basic_fixture_shader_attribute, check_family_shader};
use moly_law::fixture::mip::mip_levels;

use crate::env::{SiteEnv, SiteEnvGpuBuffer};
use crate::source_stencil::{source_attribute, SourceStencilAttribute};
use crate::render_inputs::{FixtureScenesReady, FixtureVisualRoot, FixtureVisualSceneReady};
use crate::render_inputs::FixtureVisualReady;
use crate::fixture_emission::FixtureEmission;

/// Family key handled here: the shader name, the "Basic" entry of the closed
/// family set. Other names in the closed set are out of scope (counted by name
/// and kept at swap time).
const BASIC_SHADER: &str = "Mysekai/Fixture/Basic";

/// Shader name of the ShadowMesh family: the whole family always contributes 0
/// pixels because of the material float `_Show` gate, see [`ShadowMeshGated`].
const SHADOWMESH_SHADER: &str = "Mysekai/Fixture/ShadowMesh";
const RUG_SHADER: &str = "Mysekai/Fixture/Rug";
const FENCE_SHADER: &str = "Mysekai/Fixture/Fence";

/// Slot count and byte size of the material uniform. The slot order is a
/// contract between this file and the `FixtureParams` struct in
/// `shaders/fixture_material.wesl`; change both together.
pub const PARAMS_SLOTS: usize = 12;
pub const PARAMS_BYTES: usize = PARAMS_SLOTS * 16;

/// The first seven slots hold one vec4 per Unity property; a slot the variant
/// does not consume (a dither gate that is off) is still written. The last
/// four slots are the window-outside gate corners (not Unity properties, but
/// appended to the tail of the same uniform, see the module note on the
/// window-outside arm).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FixtureParams {
    /// `(_MainTexOffsetX, _MainTexOffsetY)`: added to uv0 in the vertex stage.
    pub main_tex_offset: [f32; 2],
    /// Source UV scroll velocity; packed in the unused zw of the UV slot.
    pub uv_scroll: [f32; 2],
    /// `_DitherAlpha`. The game does not consume it when the dither variant is
    /// off; the data value is still written.
    pub dither_alpha: f32,
    pub use_phenomena_lighting: f32,
    pub override_shading_parameter: f32,
    pub local_shading_intensity: f32,
    pub local_edge_threshold: f32,
    pub local_edge_smoothness: f32,
    /// Fixture meshes retain source UVs; exported PNG rows begin at the top.
    /// This conversion applies to every material, including emissive masks.
    pub uv_v_flip: f32,
    /// Window-outside gate corners: the zero-write surface mesh vertices
    /// expressed in this mesh's local frame, already ordered as a planar
    /// convex ring. All zero for non-window materials (the consuming code is
    /// removed at compile time by the `FIXTURE_WINDOW_CLIP` variant, so the
    /// corners are never read).
    pub clip_corners: [[f32; 3]; 4],
}

impl FixtureParams {
    /// Flattens the values into bytes in the slot order above.
    pub fn bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(PARAMS_BYTES);
        let mut push = |slot: [f32; 4]| {
            for component in slot {
                bytes.extend_from_slice(&component.to_le_bytes());
            }
        };
        push([self.main_tex_offset[0], self.main_tex_offset[1], self.uv_scroll[0], self.uv_scroll[1]]);
        push([self.dither_alpha, 0.0, 0.0, 0.0]);
        push([self.use_phenomena_lighting, 0.0, 0.0, 0.0]);
        push([self.override_shading_parameter, 0.0, 0.0, 0.0]);
        push([self.local_shading_intensity, 0.0, 0.0, 0.0]);
        push([self.local_edge_threshold, 0.0, 0.0, 0.0]);
        push([self.local_edge_smoothness, 0.0, 0.0, 0.0]);
        push([self.uv_v_flip, 0.0, 0.0, 0.0]);
        for corner in &self.clip_corners {
            push([corner[0], corner[1], corner[2], 0.0]);
        }
        debug_assert_eq!(bytes.len(), PARAMS_BYTES);
        bytes
    }
}

/// Pipeline specialization key: the variant within the family. It enters the
/// pipeline cache as `AsBindGroup::Data`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FixtureMaterialKey {
    pub fence: bool,
    /// None = Basic; Some(blended) = Rug, with source queue 2007/2008.
    pub rug: Option<bool>,
    pub render_queue: u32,
    /// `_USE_ALPHA_CLIP`: a sampled alpha with alpha − 0.5 < 0 is discarded
    /// (the threshold is a compile-time constant).
    pub alpha_clip: bool,
    /// The inverse of `_DISABLE_DITHER`: the Bayer dither block is switched on
    /// or off at compile time in the shader. In live data every material has
    /// dithering off (`_DisableDither` = 1), and the block is kept for variants
    /// that do not disable it.
    pub dither: bool,
    /// Window-outside arm (usage=1 → table selection 26/27): the screen-space
    /// quad gate compiled in (`FIXTURE_WINDOW_CLIP`).
    pub window_clip: bool,
    /// `_ENABLE_MODULE_FRESNEL`: the fresnel additive-colour block
    /// (`FIXTURE_MODULE_FRESNEL`, after the treasure shadow and before fog). All
    /// 3 live materials carrying it are vehicles, none placed.
    pub fresnel: bool,
    /// `_ENABLE_MODULE_REFLECTION`: the reflection additive-colour block
    /// (`FIXTURE_MODULE_REFLECTION`, right after the fresnel arm and before
    /// fog). On only for materials whose **cube-map slot is empty** — there the
    /// cube-map sample folds to a constant (see
    /// [`moly_law::fixture::fresnel::UNBOUND_CUBE_SAMPLE`]). Materials that bind
    /// a real cube map (the 3 vehicles) cannot take this path: the branch stays
    /// off and the swap end warns by name.
    pub reflection: bool,
}

/// The material asset of the furniture Basic family.
#[derive(Debug, Clone, TypePath, Asset)]
pub struct FixtureMaterial {
    pub key: FixtureMaterialKey,
    pub params: FixtureParams,
    pub main_tex: Handle<Image>,
    /// Boolean form of `_FixtureObjectBlendMode`: the `AlphaMode::Blend`
    /// switch. The two-dimensional table selections (5/6/26/27) have already
    /// been checked against it during resolution.
    pub blend: bool,
    /// The fresnel branch parameters (`_FresnelPower` / `_FresnelColor`).
    /// Filled with 0 when the branch is off (the block is removed by
    /// specialization and the values are not read). **Not part of
    /// `params.bytes()`**: the order of those 12 slots is the prefix contract
    /// shared by the main pass and the emission pass (the emission object pool
    /// embeds params in a 64+192+16 flat layout, and growing the slots would
    /// shift the emission fields on that side) — the texture-branch parameters
    /// share the main pass's separate binding 4, see
    /// [`Self::shading_branch_bytes`].
    pub fresnel_power: f32,
    pub fresnel_color: [f32; 4],
    /// The reflection branch parameters (`_ReflectionFresnelPower` /
    /// `_ReflectionIntensity`). The cube-map term is not here — it folds to a
    /// constant in the shader (see [`FixtureMaterialKey::reflection`]).
    pub reflection_power: f32,
    pub reflection_intensity: f32,
    /// JP `_ENABLE_CRYSTAL_FIXTURE` variant (main pass only). Kept outside
    /// [`FixtureMaterialKey`]: that key is shared with the emission pass.
    pub crystal: Option<CrystalParams>,
    /// A Basic material on a site scene: its textures come from the site
    /// sidecar as plain images and are sampled like every site material, with
    /// the shared trilinear repeat sampler (the sidecar carries no sampler
    /// settings). Furniture keeps the sampler the glb authored.
    pub site_sampler: bool,
    /// JP `_ENABLE_MULTI_UV_SCROLL` variant (main pass only).
    pub multi_uv: Option<MultiUvParams>,
}

/// The multi-UV variant's layers: `_LayerTex1` always, `_LayerTex2/3` when
/// their int gates are on (a gated-off layer binds nothing and is never read).
#[derive(Debug, Clone, PartialEq)]
pub struct MultiUvParams {
    pub textures: [Option<Handle<Image>>; 3],
    /// `_LayerTex{1,2,3}_ST` (scale xy, offset zw).
    pub st: [[f32; 4]; 3],
    /// `_LayerTex{1,2,3}_UVScrollX/Y`.
    pub scroll: [[f32; 2]; 3],
    /// `_UseLayerTex2`, `_UseLayerTex3` (ints compared with 0.5 at run time).
    pub use_layer: [f32; 2],
    /// `_LayerTex{1,2,3}_ParallaxStrength`.
    pub parallax: [f32; 3],
}

/// The crystal variant's material values, read by the main pass only.
#[derive(Debug, Clone, PartialEq)]
pub struct CrystalParams {
    pub light_influence_scale: f32,
    /// `_UseNormalDrivenEmission` (an int compared with 0.5 at run time).
    pub use_normal_driven_emission: f32,
    pub normal_dark_to_color: f32,
    pub normal_bright_to_emission: f32,
    /// `_UseBrightFresnelAdd` (an int compared with 0.5 at run time).
    pub use_bright_fresnel_add: f32,
    pub bright_fresnel_add_intensity: f32,
    /// The normal-map block (`_UseNormalMap` > 0.5) with its texture.
    pub normal_map: Option<NormalMap>,
    /// Second target, normal-driven step: `_NormalDarkEmission` (the bright
    /// end is `normal_bright_to_emission`, shared with the main target).
    pub normal_dark_emission: f32,
    /// Second target, centre damp: `_UseCenterEmissionDamp` (an int compared
    /// with 0.5 at run time), `_CenterEmissionDamp`,
    /// `_CenterEmissionSuppressMin`, `_CenterEmissionSuppressMax`.
    pub use_centre_emission_damp: f32,
    pub centre_emission_damp: f32,
    pub centre_emission_suppress_min: f32,
    pub centre_emission_suppress_max: f32,
}

/// `_NormalMap` and the block's four values.
#[derive(Debug, Clone, PartialEq)]
pub struct NormalMap {
    pub texture: Handle<Image>,
    pub bump_scale: f32,
    pub exaggeration: f32,
    /// `_UseNormalParallaxShift` (an int compared with 0.5 at run time).
    pub use_parallax_shift: f32,
    pub parallax_shift: f32,
}

/// Pipeline key of the main pass: the shared key plus the two crystal
/// switches (compiled only into this pass).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FixturePipelineKey {
    pub base: FixtureMaterialKey,
    pub crystal: bool,
    pub normal_map: bool,
    pub multi_uv: bool,
}

impl FixtureMaterial {
    /// Serialization of the texture-branch parameters (binding 4, 3×vec4). The
    /// reason it is split from params is given in the field comments — that
    /// part is a byte contract across passes and cannot grow; this block is
    /// read only in the main pass fragment and can grow. The slot order is a
    /// contract with `FixtureShadingBranch` in the WGSL; change both together.
    fn shading_branch_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(192);
        let crystal = self.crystal.as_ref();
        let normal_map = crystal.and_then(|crystal| crystal.normal_map.as_ref());
        for component in [
            self.fresnel_power,
            0.0,
            0.0,
            0.0,
            self.fresnel_color[0],
            self.fresnel_color[1],
            self.fresnel_color[2],
            self.fresnel_color[3],
            self.reflection_power,
            self.reflection_intensity,
            0.0,
            0.0,
            // Crystal slots (read only by the crystal variant; zero otherwise).
            crystal.map_or(0.0, |c| c.light_influence_scale),
            crystal.map_or(0.0, |c| c.use_normal_driven_emission),
            crystal.map_or(0.0, |c| c.normal_dark_to_color),
            crystal.map_or(0.0, |c| c.normal_bright_to_emission),
            crystal.map_or(0.0, |c| c.use_bright_fresnel_add),
            crystal.map_or(0.0, |c| c.bright_fresnel_add_intensity),
            0.0,
            0.0,
            normal_map.map_or(0.0, |n| n.bump_scale),
            normal_map.map_or(0.0, |n| n.exaggeration),
            normal_map.map_or(0.0, |n| n.use_parallax_shift),
            normal_map.map_or(0.0, |n| n.parallax_shift),
        ] {
            bytes.extend_from_slice(&component.to_le_bytes());
        }
        // Multi-UV slots (zero unless the variant is on).
        let multi = self.multi_uv.as_ref();
        let st = |i: usize| multi.map_or([0.0; 4], |m| m.st[i]);
        let scroll = |i: usize| multi.map_or([0.0; 2], |m| m.scroll[i]);
        let use_layer = multi.map_or([0.0; 2], |m| m.use_layer);
        let parallax = multi.map_or([0.0; 3], |m| m.parallax);
        for component in st(0).into_iter().chain(st(1)).chain(st(2))
            .chain(scroll(0)).chain(scroll(1))
            .chain(scroll(2)).chain(use_layer)
            .chain(parallax).chain([0.0])
        {
            bytes.extend_from_slice(&component.to_le_bytes());
        }
        bytes
    }

    fn pipeline_key(&self) -> FixturePipelineKey {
        FixturePipelineKey {
            base: self.key,
            crystal: self.crystal.is_some(),
            normal_map: self.crystal.as_ref().is_some_and(|c| c.normal_map.is_some()),
            multi_uv: self.multi_uv.is_some(),
        }
    }
}

impl AsBindGroup for FixtureMaterial {
    type Data = FixturePipelineKey;
    type Param = (
        SRes<SiteEnvGpuBuffer>,
        SRes<RenderAssets<GpuImage>>,
        SRes<bevy::render::texture::FallbackImage>,
        SRes<crate::render::gpu::SharedSamplers>,
    );

    fn label() -> &'static str {
        "fixture_material"
    }

    fn build_bind_group(
        &self,
        _layout: &BindGroupLayout,
        render_device: &RenderDevice,
        (env_buffer, images, fallback, samplers): &mut SystemParamItem<'_, '_, Self::Param>,
        _force_no_bindless: bool,
        output: &mut BindGroupBuilder,
    ) -> Result<(), AsBindGroupError> {
        let main = images
            .get(&self.main_tex)
            .ok_or(AsBindGroupError::RetryNextUpdate)?;
        // The normal map is bound only for the normal-map variant; every
        // other variant binds the engine fallback, which no compiled program
        // reads (the layout is shared).
        let normal = match self.crystal.as_ref().and_then(|c| c.normal_map.as_ref()) {
            Some(normal_map) => images
                .get(&normal_map.texture)
                .ok_or(AsBindGroupError::RetryNextUpdate)?,
            None => &fallback.d2,
        };
        // The single main texture slot always holds a real texture — a material
        // without a main texture is refused while its entry is planned — so no
        // fallback binding is needed.
        let mut bindings = UnpreparedBindingResources(vec![
            // binding 0: the material's own parameter block.
            (0, crate::render::gpu::push_data(output, &self.params.bytes())),
            // binding 1: the global values, sharing one buffer with site
            // materials.
            (1, UnpreparedBindingResource::Buffer(env_buffer.buffer.clone())),
            (
                2,
                UnpreparedBindingResource::TextureView(
                    TextureViewDimension::D2,
                    main.texture_view.clone(),
                ),
            ),
            (3, UnpreparedBindingResource::Sampler(
                SamplerBindingType::Filtering,
                if self.site_sampler { samplers.repeat_linear.clone() } else { main.sampler.clone() },
            )),
            // binding 4: the texture-branch parameters (a variant with the
            // branches off does not read it but binds it anyway — the layout is
            // shared by every variant, so an extra binding is valid).
            (4, crate::render::gpu::push_data(output, &self.shading_branch_bytes())),
            (
                5,
                UnpreparedBindingResource::TextureView(
                    TextureViewDimension::D2,
                    normal.texture_view.clone(),
                ),
            ),
            (6, UnpreparedBindingResource::Sampler(SamplerBindingType::Filtering, normal.sampler.clone())),
        ]);
        // Bindings 7..12: the three multi-UV layers, fallback when absent.
        for (layer, slot) in [(0usize, 7u32), (1, 9), (2, 11)] {
            let image = match self.multi_uv.as_ref().and_then(|m| m.textures[layer].as_ref()) {
                Some(handle) => images.get(handle).ok_or(AsBindGroupError::RetryNextUpdate)?,
                None => &fallback.d2,
            };
            bindings.0.push((
                slot,
                UnpreparedBindingResource::TextureView(TextureViewDimension::D2, image.texture_view.clone()),
            ));
            bindings.0.push((
                slot + 1,
                UnpreparedBindingResource::Sampler(SamplerBindingType::Filtering, image.sampler.clone()),
            ));
        }
        output.binding_resources.0.extend(bindings.0);
        Ok(())
    }

    fn bind_group_data(&self) -> Self::Data {
        self.pipeline_key()
    }

    fn bind_group_layout_entries(
        _render_device: &RenderDevice,
        _force_no_bindless: bool,
    ) -> Vec<BindGroupLayoutEntry> {
        let uniform = |binding: u32| BindGroupLayoutEntry {
            binding,
            visibility: ShaderStages::VERTEX | ShaderStages::FRAGMENT,
            ty: BindingType::Buffer {
                ty: BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let texture = |binding: u32| BindGroupLayoutEntry {
            binding,
            visibility: ShaderStages::FRAGMENT,
            ty: BindingType::Texture {
                sample_type: TextureSampleType::Float { filterable: true },
                view_dimension: TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let sampler = |binding: u32| BindGroupLayoutEntry {
            binding,
            visibility: ShaderStages::FRAGMENT,
            ty: BindingType::Sampler(SamplerBindingType::Filtering),
            count: None,
        };
        // binding 4 is read only in the fragment (the additive blocks of the
        // texture branches).
        let fragment_uniform = |binding: u32| BindGroupLayoutEntry {
            binding,
            visibility: ShaderStages::FRAGMENT,
            ty: BindingType::Buffer {
                ty: BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        vec![
            uniform(0),
            uniform(1),
            texture(2),
            sampler(3),
            fragment_uniform(4),
            texture(5),
            sampler(6),
            texture(7),
            sampler(8),
            texture(9),
            sampler(10),
            texture(11),
            sampler(12),
        ]
    }
}

impl Material for FixtureMaterial {
    // The key prefix of an embedded source is the lib name (hyphens turned
    // into underscores), as noted for the site material.
    fn vertex_shader() -> ShaderRef {
        "embedded://moly_render/shaders/fixture_material.wesl".into()
    }

    fn fragment_shader() -> ShaderRef {
        "embedded://moly_render/shaders/fixture_material.wesl".into()
    }

    /// Blend mode: a material with `_FixtureObjectBlendMode` = 1 (AlphaBlended)
    /// uses the alpha-blend pipeline; 0 (Opaque) is drawn opaque.
    fn alpha_mode(&self) -> AlphaMode {
        // Rug blending belongs between ground and objects in the source's
        // opaque queue range; its actual blend state is installed in specialize.
        if self.key.rug.is_some() {
            AlphaMode::Opaque
        } else if self.blend {
            AlphaMode::Blend
        } else {
            AlphaMode::Opaque
        }
    }

    /// The furniture family stays out of the prepass: only the Base fragment is
    /// implemented here, and shadow consumers are out of scope.
    fn enable_prepass() -> bool {
        false
    }

    fn enable_shadows() -> bool {
        false
    }

    fn specialize(
        _pipeline: &MaterialPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        _layout: &bevy::mesh::MeshVertexBufferLayoutRef,
        key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        let pipeline_key = key.bind_group_data;
        let base = pipeline_key.base;
        let mut defs: Vec<&str> = Vec::new();
        if base.fence { defs.push("FIXTURE_FENCE"); }
        crate::material_order::set_queue(descriptor, base.render_queue);
        if let Some(blended) = base.rug {
            defs.push("FIXTURE_RUG");
            if let Some(depth) = descriptor.depth_stencil.as_mut() {
                depth.depth_write_enabled = Some(false);
                depth.depth_compare = Some(CompareFunction::Always);
            }
            if let Some(target) = descriptor.fragment.as_mut()
                .and_then(|f| f.targets.get_mut(0)).and_then(Option::as_mut) {
                target.blend = blended.then_some(BlendState::ALPHA_BLENDING);
            }
        }
        if base.alpha_clip {
            defs.push("FIXTURE_ALPHA_CLIP");
        }
        if base.dither {
            defs.push("FIXTURE_DITHER");
        }
        if base.window_clip {
            defs.push("FIXTURE_WINDOW_CLIP");
        }
        if base.fresnel {
            defs.push("FIXTURE_MODULE_FRESNEL");
        }
        if base.reflection {
            defs.push("FIXTURE_MODULE_REFLECTION");
        }
        // Vertex output for the view direction: both texture branches read the
        // same output and need it whenever either is on. **Derived from those
        // keys rather than a separate switch** — if it is missed, the shader
        // references an identifier that does not exist, which only surfaces as
        // a pipeline error at run time, and `cargo build` does not validate a
        // single wgsl byte.
        if base.fresnel || base.reflection {
            defs.push("FIXTURE_VIEW_DIR");
        }
        // The crystal variant is only resolved together with both texture
        // branches (every crystal program in the source carries them), so it
        // never needs FIXTURE_VIEW_DIR on its own.
        if pipeline_key.crystal {
            defs.push("FIXTURE_CRYSTAL");
        }
        if pipeline_key.normal_map {
            defs.push("FIXTURE_NORMAL_MAP");
        }
        if pipeline_key.multi_uv {
            defs.push("FIXTURE_MULTI_UV");
        }
        for def in defs {
            descriptor.vertex.shader_defs.push(def.into());
            if let Some(ref mut fragment) = descriptor.fragment {
                fragment.shader_defs.push(def.into());
            }
        }
        Ok(())
    }
}

// ---- Wall-layout ShadowCaster marker ----

/// Marker on mesh entities placed by a wall layout (LayoutType has any bit of
/// 0xF0 set). The game disables the ShadowCaster pass of each material while a
/// wall layout is active (`SetupRenderer`); the main-light shadow consumer
/// excludes wall furniture by this run-time state and leaves the visibility of
/// its colour pass alone.
#[derive(Component)]
pub struct WallLayoutShadowCasterOff;

/// Marker on mesh entities whose pass writes nothing: material `_ColorMask` = 0
/// (all colour channels off) and `_ZWrite` = 0 (depth not written). Such a pass
/// writes only the stencil buffer in the game, its contribution to colour and
/// depth is always zero, and hiding it in the main pass gives value-for-value
/// the same output; its stencil writes are reproduced by stencil replay from
/// the attached [`SourceStencilAttribute`]. The check uses the **CPU-side
/// render-state floats**, not the shader name: Unity configures the render
/// state from these floats and the shader program does not consume them;
/// deciding by name ("stencil materials should be hidden") or by texture
/// presence (a white-surface material has no texture) is the wrong gate — the
/// same lesson as the variant gate that reads only keywords.
#[derive(Component)]
pub struct StencilOnlyPass;

/// Marker on mesh entities of the ShadowMesh family: its fragment gates on the
/// material float `_Show` and discards everything below 0.5; in live data
/// `_Show` is 0.0 on every material of the family (the extractor copies the
/// authored floats as they are and never rewrites them), and no AnimationClip
/// float curve targets it — the pixel contribution is always 0 in the game.
/// This pipeline does not implement the family's shader; keeping the glb
/// default material would draw this projection mesh as a wrong solid surface,
/// while hiding it gives value-for-value the same output, not an
/// approximation. The shadow-mesh consumer can find these meshes again by this
/// marker.
#[derive(Component)]
pub struct ShadowMeshGated;

/// The render-state "writes nothing" signature: `_ColorMask` = 0 and
/// `_ZWrite` = 0. Blend factors do not reach the output when no channel can be
/// written, so they need no check.
fn is_stencil_only(extras: &serde_json::Value) -> bool {
    matches!(
        (extras_float(extras, "_ColorMask"), extras_float(extras, "_ZWrite")),
        (Some(0.0), Some(0.0))
    )
}

// ---- Resolution ----

/// The float table entry of the glb material extras.
fn extras_float(extras: &serde_json::Value, key: &str) -> Option<f64> {
    extras
        .get("floats")
        .and_then(|floats| floats.get(key))
        .and_then(|value| value.as_f64())
}

/// The colour table entry of the glb material extras (a 4-tuple). An array
/// whose length is not 4 is treated as a missing key (fail-closed: the caller
/// refuses by name, with no truncation and no padding).
fn extras_color(extras: &serde_json::Value, key: &str) -> Option<[f64; 4]> {
    let array = extras
        .get("colors")
        .and_then(|colors| colors.get(key))
        .and_then(|value| value.as_array())?;
    let mut color = [0.0; 4];
    for (slot, value) in color.iter_mut().zip(array) {
        *slot = value.as_f64()?;
    }
    Some(color)
}

/// The top-level usage entry of the glb material extras.
fn extras_usage(extras: &serde_json::Value) -> Option<f64> {
    extras
        .get("fixtureShaderUsage")
        .and_then(|value| value.as_f64())
}

/// The top-level `validKeywords` list of the glb material extras (the source of
/// the game's runtime shader variant switches, see the module note). None =
/// the list is missing: fail-closed, the caller refuses by name rather than
/// guessing an empty list.
fn extras_valid_keywords(extras: &serde_json::Value) -> Option<&Vec<serde_json::Value>> {
    extras
        .get("validKeywords")
        .and_then(|value| value.as_array())
}

/// A material whose glb has no base colour texture: Ok when the source slot
/// is empty and the shader declares the `white` default (Unity then binds its
/// built-in white texture). The extractor writes a `textures` key with a null
/// value for a texture bound in a dependency package it did not export, and
/// no key for an empty slot; the defaults come from the shader's property
/// declarations (`shaderTextureDefaults`).
fn empty_main_tex_default(extras: &serde_json::Value) -> Result<(), String> {
    let bound = extras
        .get("textures")
        .and_then(|textures| textures.as_object())
        .is_some_and(|textures| textures.contains_key("_MainTex"));
    if bound {
        return Err("_MainTex 绑定的贴图不在本包（依赖包贴图未导出）".into());
    }
    match extras
        .get("shaderTextureDefaults")
        .and_then(|defaults| defaults.get("_MainTex"))
        .and_then(|value| value.as_str())
    {
        Some("white") => Ok(()),
        other => Err(format!("_MainTex 槽为空且 shader 默认贴图 {other:?} 未建模")),
    }
}

/// What resolving one Basic furniture material yields: the GPU material, the
/// two-dimensional table selection (compared against the log), the emission
/// ints recorded in the log, the variant switch panel items compared against the
/// log, the emission ints that go into the [`FixtureEmission`] component as the
/// material-side value, and the glTF texture index of the mask texture (from
/// which the build stage creates the handle).
struct ResolvedBasic {
    force_emission: bool,
    material: FixtureMaterial,
    attribute: u32,
    emission: EmissionInts,
    /// Whether `_RECEIVE_SHADOWS_OFF` is present (no program-level effect, only
    /// accounted).
    receive_shadows_off: bool,
    /// Name of each keyword that appears beyond the handled switches (live data
    /// lists unimplemented texture branches, see the module note); any other
    /// name follows the same accounting, without guessing its meaning.
    unported_keywords: Vec<String>,
    /// The reflection keyword is on but the material **binds a real cube map**
    /// — the folded form is wrong for it and this pipeline has no path with a
    /// real sample ⇒ the branch stays off and the material is accounted by name
    /// (the keyword also stays in `unported_keywords`). In live data these are
    /// the 3 vehicle materials, none placed.
    reflection_needs_cubemap: bool,
    /// The crystal keyword is on but its branch cannot be drawn (named
    /// reason). The keyword then stays in `unported_keywords`.
    crystal_blocked: Option<String>,
    /// glTF texture index of `_EmissionMaskTex` in the extras texture table;
    /// None when the key is missing (irrelevant for materials whose gate ints
    /// are all zero; a non-zero gate is treated as zero contribution with a
    /// named warning).
    mask_index: Option<u32>,
    /// JP `_UseEmissionColorPicker` on: `_EmissionColor.rgb *
    /// _EmissionIntensity`, which replaces the mask texel in the second target.
    colour_picker: Option<[f32; 3]>,
}

/// The material ints that drive the switch chain of the second colour target
/// (stored as 0/1 floats). They go into [`FixtureEmission`] at swap time and
/// are compared by phenomena type in the emission pass fragment. A missing
/// float records None, never a silent 0.
/// The initial material flags are separate from the per-instance override
/// written later by fixture animation events.
#[derive(Debug, Clone, Copy)]
struct EmissionInts {
    bright: Option<f64>,
    dark: Option<f64>,
}

/// Phenomena emission gate value: implements the switch chain of the second
/// colour target (at phenomena type 1 bright is compared against 1, at type 2
/// dark against 1, any other type gives 0). The tier-switch account and its
/// log take the gate from this one function.
fn emission_gate(emission: &EmissionInts, emission_type: i32) -> Option<i32> {
    let (bright, dark) = (emission.bright?, emission.dark?);
    Some(match emission_type {
        1 => (bright == 1.0) as i32,
        2 => (dark == 1.0) as i32,
        _ => 0,
    })
}

/// The emission account gathered when a swap completes: the bright/dark arms
/// and the linear mask mean of each material. The weather tier-switch log
/// reads it with the current phenomena type to derive each material's
/// contribution row, so the post-switch emission account follows from the log
/// (the shader side has its own gate; both evaluate the same switch chain).
#[derive(Resource, Default)]
pub struct EmissionAccount {
    pub rows: Vec<EmissionRow>,
}

/// One account row. `mask_mean` is the per-channel mean of the mask texture
/// after decoding into the linear domain (alpha dropped); a missing mask (or
/// texels not yet available) records None.
pub struct EmissionRow {
    pub name: String,
    pub bright: Option<f64>,
    pub dark: Option<f64>,
    pub mask_mean: Option<[f32; 3]>,
}

/// The account of the fixtures a save kept, while the fixtures it appended
/// are swapped: their rows follow the kept ones.
#[derive(Resource)]
pub struct EmissionAccountKept(pub EmissionAccount);

impl EmissionAccount {
    /// Derives each material's contribution row by phenomena type (for the
    /// tier-switch log): gate × mask mean. A row whose gate (an int key) or
    /// mask is missing keeps None as is, never a silent 0.
    pub fn contribution_lines(&self, emission_type: i32) -> Vec<String> {
        self.rows
            .iter()
            .map(|row| {
                format!(
                    "{}：亮臂 {:?} 暗臂 {:?} 遮罩均值 {:?} ⇒ 类型 {} 门 {:?} 贡献 {:?}",
                    row.name,
                    row.bright,
                    row.dark,
                    row.mask_mean,
                    emission_type,
                    emission_gate(
                        &EmissionInts { bright: row.bright, dark: row.dark },
                        emission_type
                    ),
                    match (
                        emission_gate(
                            &EmissionInts { bright: row.bright, dark: row.dark },
                            emission_type
                        ),
                        row.mask_mean,
                    ) {
                        (Some(1), Some(mean)) => Some(mean),
                        (Some(0), _) => Some([0.0; 3]),
                        _ => None,
                    }
                )
            })
            .collect()
    }
}

/// Resolves one Basic furniture material: family gate → two-dimensional
/// selection table check → window-outside clip corners → variant key and
/// uniform slots. A failure returns a named error and the caller keeps the
/// original material.
///
/// `clip_quad` is the four corners of the zero-write surface under the glb
/// root the material belongs to (taken from the first root seen); it is
/// required only at usage=1 (the window-outside arm): a missing zero-write
/// surface or unusable corners is a named refusal, never a silent pass —
/// passing would draw the whole window-outside mesh into a scene that has no
/// wall.
fn resolve_basic(
    name: &str,
    extras: &serde_json::Value,
    main_tex: Handle<Image>,
    clip_quad: Option<&Result<[Vec3; 4], String>>,
    source_textures: Option<&SourceMaterialTextures>,
) -> Result<ResolvedBasic, String> {
    let source_texture = |key: &str| source_textures.and_then(|textures| textures.0.get(key)).cloned();
    let normal_map_texture = source_texture("_NormalMap");
    // Two-dimensional selection table: (usage, blend) → ShaderAttribute. Out of
    // range is refused; the selection's opaque/blended pair (5/26 opaque,
    // 6/27 blended) must agree with the blend mode — a mismatch means the data
    // contradicts itself and is refused loudly.
    let usage = extras_usage(extras)
        .ok_or_else(|| format!("家具材质 {name} 的 extras 缺 fixtureShaderUsage"))?;
    let blend_raw = extras_float(extras, "_FixtureObjectBlendMode")
        .ok_or_else(|| format!("家具材质 {name} 缺浮点属性 _FixtureObjectBlendMode"))?;
    let attribute = basic_fixture_shader_attribute(usage as i32, blend_raw as i32)
        .map_err(|err| format!("家具材质 {name}：{err}"))?;
    let blend = blend_raw >= 0.5;
    // Consistency check of the selection against the blend mode: in the table
    // 5/26 is the opaque slot and 6/27 the blended slot. The selection comes
    // from looking up (usage, blend); this only checks that the table output
    // agrees with the blend column — an error in either shows up loudly here.
    let expected = if blend { 6 } else { 5 };
    if attribute != expected && attribute != expected + 21 {
        return Err(format!(
            "家具材质 {name} 的二维选择落点 {attribute} 与 _FixtureObjectBlendMode \
             {blend_raw} 互相矛盾"
        ));
    }
    // Window-outside arm: usage=1 → selection 26/27; in the game it draws only
    // inside the opening at stencil Equal. The corners come from the zero-write
    // surface of the same glb; a missing or unusable quad is a named refusal and
    // the material keeps its original material — louder than drawing the whole
    // window-outside mesh silently.
    let window_clip = usage as i32 == 1;
    let clip_corners = if window_clip {
        match clip_quad {
            Some(Ok(quad)) => [
                quad[0].to_array(),
                quad[1].to_array(),
                quad[2].to_array(),
                quad[3].to_array(),
            ],
            Some(Err(reason)) => {
                return Err(format!(
                    "窗外观支路材质 {name} 的写零面四角不可用：{reason}"
                ));
            }
            None => {
                return Err(format!(
                    "窗外观支路材质 {name} 所在 glb 下没有写零面（模板写者缺失），\
                     无法建屏空间门"
                ));
            }
        }
    } else {
        [[0.0; 3]; 4]
    };
    // Variant switch panel: keyword switches; the game at run time honours only
    // valid keywords (re-derived from integer properties when the material is
    // built; the invalid list is an editor leftover and is not read). The older
    // way of reading these switches (the `_UseAlphaClip`/`_DisableDither`
    // floats) agrees item by item with the keyword side on every Basic
    // material — what changes is which source is read, not the value.
    let valid = extras_valid_keywords(extras)
        .ok_or_else(|| format!("家具材质 {name} 的 extras 缺 validKeywords 列"))?;
    let keyword_on = |keyword: &str| {
        valid
            .iter()
            .any(|item| item.as_str() == Some(keyword))
    };
    let alpha_clip = keyword_on("_USE_ALPHA_CLIP");
    // The keyword that disables dithering is `_DISABLE_DITHER`; the dither
    // variant is its inverse.
    let dither = !keyword_on("_DISABLE_DITHER");
    let receive_shadows_off = keyword_on("_RECEIVE_SHADOWS_OFF");
    let fresnel = keyword_on("_ENABLE_MODULE_FRESNEL");
    // Reflection arm: this pipeline's folded form applies only when the keyword
    // is on **and** the cube-map slot is empty. The slot state is read from the
    // texture reference table the extractor writes into the extras — **whether
    // the key is present** is the check: key absent = that texture slot of the
    // game material is an empty reference (the fold holds, the sample is a
    // constant); key present (value null, because glTF has no cube-map type,
    // so the extractor can only record "an object is referenced here") = the
    // material binds a real cube map ⇒ **the fold is wrong for it**, a real
    // sample is required and this pipeline has no such path ⇒ the branch stays
    // off and the material is accounted by name.
    // The check agrees entry by entry with the empty/non-empty object
    // reference in the game (16 empty / 3 non-empty) and is a separate read
    // path from it; `_MainTex` in the same table always has a real index, so
    // "key absent" really is absent, not a broken table.
    let reflection_keyword = keyword_on("_ENABLE_MODULE_REFLECTION");
    let cube_bound = extras
        .get("textures")
        .and_then(|textures| textures.as_object())
        .is_some_and(|textures| textures.contains_key("_ReflectionCubeMap"));
    let reflection = reflection_keyword && !cube_bound;
    let reflection_needs_cubemap = reflection_keyword && cube_bound;
    // JP `_ENABLE_CRYSTAL_FIXTURE`. Every crystal program of the source
    // carries the fresnel and reflection keywords, and the JP normal-map
    // block (`_UseNormalMap`, an int read at run time in every JP Basic
    // program) is used by the data only together with it; any other
    // combination is a program this port has not read, and is refused.
    let crystal_keyword = keyword_on("_ENABLE_CRYSTAL_FIXTURE");
    let use_normal_map = extras_float(extras, "_UseNormalMap").is_some_and(|value| value > 0.5);
    if use_normal_map && !crystal_keyword {
        return Err(format!(
            "furniture material {name}: _UseNormalMap is on while \
             _ENABLE_CRYSTAL_FIXTURE is off; that combination is not supported"
        ));
    }
    let (crystal, crystal_blocked) = if crystal_keyword {
        if !(fresnel && reflection_keyword) {
            return Err(format!(
                "家具材质 {name} 开着 _ENABLE_CRYSTAL_FIXTURE 却缺菲涅尔或反射 keyword：\
                 源程序表里没有这个组合"
            ));
        }
        let crystal_get = |key: &str| -> Result<f32, String> {
            extras_float(extras, key)
                .map(|value| value as f32)
                .ok_or_else(|| format!("家具材质 {name} 开着 crystal 却缺浮点属性 {key}"))
        };
        let normal_map = if use_normal_map {
            // Key present with a null value: the material binds a texture that
            // lives in a dependency package and was not exported into this
            // glb. Key absent: the slot is empty and the engine would bind the
            // shader's default, which this port does not model. Both keep the
            // crystal branch off, by name.
            let bound = extras
                .get("textures")
                .and_then(|textures| textures.as_object())
                .is_some_and(|textures| textures.contains_key("_NormalMap"));
            match (normal_map_texture, bound) {
                (Some(texture), _) => Ok(Some(NormalMap {
                    texture,
                    bump_scale: crystal_get("_BumpScale")?,
                    exaggeration: crystal_get("_NormalExaggeration")?,
                    use_parallax_shift: crystal_get("_UseNormalParallaxShift")?,
                    parallax_shift: crystal_get("_NormalParallaxShift")?,
                })),
                (None, true) => Err(format!(
                    "{name}: _NormalMap is bound in the source but its texture is not in this \
                     package (dependency texture not exported)"
                )),
                (None, false) => Err(format!(
                    "{name}: _UseNormalMap is on with an empty _NormalMap slot (engine default \
                     texture not modelled)"
                )),
            }
        } else {
            Ok(None)
        };
        match normal_map {
            Ok(normal_map) if reflection => (
                Some(CrystalParams {
                    light_influence_scale: crystal_get("_LightInfluenceScale")?,
                    use_normal_driven_emission: crystal_get("_UseNormalDrivenEmission")?,
                    normal_dark_to_color: crystal_get("_NormalDarkToColor")?,
                    normal_bright_to_emission: crystal_get("_NormalBrightToEmission")?,
                    use_bright_fresnel_add: crystal_get("_UseBrightFresnelAdd")?,
                    bright_fresnel_add_intensity: crystal_get("_BrightFresnelAddIntensity")?,
                    normal_map,
                    normal_dark_emission: crystal_get("_NormalDarkEmission")?,
                    use_centre_emission_damp: crystal_get("_UseCenterEmissionDamp")?,
                    centre_emission_damp: crystal_get("_CenterEmissionDamp")?,
                    centre_emission_suppress_min: crystal_get("_CenterEmissionSuppressMin")?,
                    centre_emission_suppress_max: crystal_get("_CenterEmissionSuppressMax")?,
                }),
                None,
            ),
            Ok(_) => (
                None,
                Some(format!("{name}: crystal needs the reflection branch, whose cube map is bound")),
            ),
            Err(reason) => (None, Some(reason)),
        }
    } else {
        (None, None)
    };
    // JP `_ENABLE_MULTI_UV_SCROLL`: its programs carry no fresnel, reflection,
    // clip or crystal keyword; a material combining them names a program this
    // port has not read.
    let multi_uv = if keyword_on("_ENABLE_MULTI_UV_SCROLL") {
        if fresnel || reflection_keyword || alpha_clip || crystal_keyword {
            return Err(format!(
                "家具材质 {name} 的 _ENABLE_MULTI_UV_SCROLL 与菲涅尔/反射/clip/crystal 同开：\
                 源程序表里没有这个组合"
            ));
        }
        let multi_get = |key: &str| -> Result<f32, String> {
            extras_float(extras, key)
                .map(|value| value as f32)
                .ok_or_else(|| format!("家具材质 {name} 开着 multi-UV 却缺浮点属性 {key}"))
        };
        let use_layer = [multi_get("_UseLayerTex2")?, multi_get("_UseLayerTex3")?];
        let mut textures: [Option<Handle<Image>>; 3] = [None, None, None];
        let mut st = [[0.0f32; 4]; 3];
        let mut scroll = [[0.0f32; 2]; 3];
        let mut parallax = [0.0f32; 3];
        for layer in 0..3 {
            let property = format!("_LayerTex{}", layer + 1);
            let used = layer == 0 || use_layer[layer - 1] > 0.5;
            if used {
                textures[layer] = Some(source_texture(&property).ok_or_else(|| {
                    format!("家具材质 {name} 的 {property} 未装载（槽为空或贴图不在本包）")
                })?);
            }
            let scale_offset = extras
                .get("textureScaleOffset")
                .and_then(|table| table.get(&property))
                .and_then(|value| value.as_array())
                .filter(|array| array.len() == 4)
                .ok_or_else(|| format!("家具材质 {name} 缺 {property} 的 ST"))?;
            for (slot, value) in st[layer].iter_mut().zip(scale_offset) {
                *slot = value.as_f64().ok_or_else(|| format!("家具材质 {name} 的 {property} ST 非数值"))? as f32;
            }
            scroll[layer] = [
                multi_get(&format!("{property}_UVScrollX"))?,
                multi_get(&format!("{property}_UVScrollY"))?,
            ];
            parallax[layer] = multi_get(&format!("{property}_ParallaxStrength"))?;
        }
        Some(MultiUvParams { textures, st, scroll, use_layer, parallax })
    } else {
        None
    };
    let unported_keywords: Vec<String> = valid
        .iter()
        .filter_map(|item| item.as_str())
        .filter(|keyword| {
            if *keyword == "_ENABLE_MODULE_REFLECTION" {
                // The 16 entries that take the folded form no longer count as
                // unimplemented; the few that bind a real cube map still do —
                // for them this branch really is not implemented.
                return !reflection;
            }
            if *keyword == "_ENABLE_CRYSTAL_FIXTURE" {
                return crystal.is_none();
            }
            if *keyword == "_ENABLE_MULTI_UV_SCROLL" {
                return multi_uv.is_none();
            }
            !matches!(
                *keyword,
                "_USE_ALPHA_CLIP"
                    | "_DISABLE_DITHER"
                    | "_RECEIVE_SHADOWS_OFF"
                    | "_ENABLE_MODULE_FRESNEL"
            )
        })
        .map(str::to_owned)
        .collect();
    let get = |key: &str| -> Result<f32, String> {
        let value = extras_float(extras, key)
            .ok_or_else(|| format!("家具材质 {name} 缺浮点属性 {key}"))?;
        Ok(value as f32)
    };
    let params = FixtureParams {
        uv_scroll: [0.0; 2],
        main_tex_offset: [
            get("_MainTexOffsetX")?,
            get("_MainTexOffsetY")?,
        ],
        dither_alpha: get("_DitherAlpha")?,
        use_phenomena_lighting: get("_UsePhenomenaLighting")?,
        override_shading_parameter: get("_OverrideShadingParameter")?,
        local_shading_intensity: get("_LocalShadingIntensity")?,
        local_edge_threshold: get("_LocalEdgeThreshold")?,
        local_edge_smoothness: get("_LocalEdgeSmoothness")?,
        uv_v_flip: 1.0,
        clip_corners,
    };
    // The fresnel parameters: required while the branch is on (either one
    // missing is a named refusal — the switch contradicts the data, and a
    // silent default would add colour the game does not add); filled with 0
    // while it is off (the block is removed by specialization and the values
    // are not read).
    let (fresnel_power, fresnel_color) = if fresnel {
        (
            get("_FresnelPower")?,
            extras_color(extras, "_FresnelColor")
                .map(|c| [c[0] as f32, c[1] as f32, c[2] as f32, c[3] as f32])
                .ok_or_else(|| {
                    format!("家具材质 {name} 开着 fresnel 却缺 _FresnelColor")
                })?,
        )
    } else {
        (0.0, [0.0; 4])
    };
    // The reflection parameters: required while the branch is on (either one
    // missing is a named refusal, the same shape as fresnel — a silent default
    // would add colour the game does not add); filled with 0 while it is off
    // (the block is removed by specialization and the values are not read).
    let (reflection_power, reflection_intensity) = if reflection {
        (
            get("_ReflectionFresnelPower")?,
            get("_ReflectionIntensity")?,
        )
    } else {
        (0.0, 0.0)
    };
    let emission = EmissionInts {
        bright: extras_float(extras, "_BrightPhenomenaEmission"),
        dark: extras_float(extras, "_DarkPhenomenaEmission"),
    };
    // glTF texture index of the mask texture: the extractor writes the material
    // texture references into the extras `textures` table (texture name → glTF
    // texture index). A missing key means the material has no mask.
    let mask_index = extras
        .get("textures")
        .and_then(|textures| textures.get("_EmissionMaskTex"))
        .and_then(|value| value.as_u64())
        .map(|value| value as u32);
    // JP `_UseEmissionColorPicker`, a runtime int in every JP Basic program:
    // on, the second target starts from `_EmissionColor.xyz *
    // _EmissionIntensity` and the mask is not sampled. The colour property is
    // passed as authored (gamma project, no conversion).
    let colour_picker = match extras_float(extras, "_UseEmissionColorPicker") {
        Some(value) if value > 0.5 => {
            let colour = extras_color(extras, "_EmissionColor")
                .ok_or_else(|| format!("家具材质 {name} 开着 _UseEmissionColorPicker 却缺 _EmissionColor"))?;
            let intensity = extras_float(extras, "_EmissionIntensity")
                .ok_or_else(|| format!("家具材质 {name} 开着 _UseEmissionColorPicker 却缺 _EmissionIntensity"))?;
            Some([
                colour[0] as f32 * intensity as f32,
                colour[1] as f32 * intensity as f32,
                colour[2] as f32 * intensity as f32,
            ])
        }
        _ => None,
    };
    let base_queue = match (window_clip, blend) {
        (true, true) => 2085, (true, false) => 2035,
        (false, true) => 3020, (false, false) => 2065,
    };
    let render_queue = u32::try_from(base_queue + extras_float(extras, "_RenderPriority").unwrap_or(0.0) as i32)
        .map_err(|_| format!("家具材质 {name} 的渲染队列为负"))?;
    Ok(ResolvedBasic {
        force_emission: false,
        material: FixtureMaterial {
            key: FixtureMaterialKey {
                fence: false,
                rug: None,
                render_queue,
                alpha_clip,
                dither,
                window_clip,
                fresnel,
                reflection,
            },
            params,
            main_tex,
            blend,
            fresnel_power,
            fresnel_color,
            reflection_power,
            reflection_intensity,
            crystal,
            site_sampler: false,
            multi_uv,
        },
        attribute,
        emission,
        receive_shadows_off,
        unported_keywords,
        reflection_needs_cubemap,
        crystal_blocked,
        mask_index,
        colour_picker,
    })
}

/// A `Mysekai/Fixture/Basic` material authored on a site scene: the site
/// sidecar slot instead of glb extras, the same resolution as the furniture
/// path. The slot is rewritten into the extras shape `resolve_basic` reads
/// (floats, colours, the valid keywords, the usage float, the non-empty
/// texture slots); the window-outside arm has no stencil writer on a site and
/// is refused there by the same rule. Site textures keep the site family's
/// texture handling (see [`FixtureMaterial::site_sampler`]).
pub struct SiteBasic {
    pub material: FixtureMaterial,
    /// `_EmissionMaskTex` is a non-empty slot of the material.
    pub has_mask: bool,
    /// JP `_UseEmissionColorPicker` on: the second target's colour.
    pub colour_picker: Option<[f32; 3]>,
    pub bright: f32,
    pub dark: f32,
}

pub fn resolve_site_basic(
    slot: &moly_law::material::MaterialSlot,
    main_tex: Handle<Image>,
) -> Result<SiteBasic, String> {
    let floats: serde_json::Map<String, serde_json::Value> = slot
        .floats
        .iter()
        .map(|(key, value)| (key.clone(), serde_json::json!(*value as f64)))
        .collect();
    let colors: serde_json::Map<String, serde_json::Value> = slot
        .colors
        .iter()
        .map(|(key, value)| (key.clone(), serde_json::json!((*value).map(|c| c as f64))))
        .collect();
    let textures: serde_json::Map<String, serde_json::Value> = slot
        .textures
        .iter()
        .map(|(key, index)| (key.clone(), serde_json::json!(*index)))
        .collect();
    let mut extras = serde_json::json!({
        "floats": floats,
        "colors": colors,
        "validKeywords": slot.keywords,
        "textures": textures,
    });
    if let Some(usage) = slot.floats.get("_FixtureShaderUsage") {
        extras["fixtureShaderUsage"] = serde_json::json!(*usage as f64);
    }
    let resolved = resolve_basic(&slot.name, &extras, main_tex, None, None)?;
    if !resolved.unported_keywords.is_empty() {
        return Err(format!(
            "furniture material {} on a site has unsupported keywords {:?}",
            slot.name, resolved.unported_keywords
        ));
    }
    let mut material = resolved.material;
    material.site_sampler = true;
    Ok(SiteBasic {
        material,
        has_mask: resolved.mask_index.is_some(),
        colour_picker: resolved.colour_picker,
        // Missing ints take the shader default 0, as on the furniture path.
        bright: resolved.emission.bright.unwrap_or(0.0) as f32,
        dark: resolved.emission.dark.unwrap_or(0.0) as f32,
    })
}

/// Fence Base shares the toon equation and UV offset with Basic, but has no
/// alpha clip, fragment normal normalization, treasure shadow, fog or emission.
/// All 16 Basic variants reduce to a single fragment program.
fn resolve_fence(name: &str, extras: &serde_json::Value, main_tex: Handle<Image>) -> Result<ResolvedBasic, String> {
    let get = |key: &str| extras_float(extras, key)
        .map(|v| v as f32).ok_or_else(|| format!("Fence 材质 {name} 缺属性 {key}"));
    for (key, expected) in [("_ZWrite", 1.0), ("_ZTest", 4.0), ("_Cull", 2.0),
        ("_ColorMask", 15.0), ("_SrcBlend", 1.0), ("_DstBlend", 0.0)] {
        if get(key)? != expected { return Err(format!("Fence 材质 {name} 的 {key} 不在已实现的源状态内")); }
    }
    let valid = extras_valid_keywords(extras)
        .ok_or_else(|| format!("Fence 材质 {name} 缺 validKeywords"))?;
    let keywords: Vec<&str> = valid.iter().filter_map(|v| v.as_str()).collect();
    let queue = u32::try_from(2065 + get("_RenderPriority")? as i32)
        .map_err(|_| format!("Fence 材质 {name} 队列为负"))?;
    Ok(ResolvedBasic {
        material: FixtureMaterial {
            key: FixtureMaterialKey { fence: true, rug: None, render_queue: queue,
                alpha_clip: false, dither: !keywords.contains(&"_DISABLE_DITHER"),
                window_clip: false, fresnel: false, reflection: false },
            params: FixtureParams {
                main_tex_offset: [get("_MainTexOffsetX")?, get("_MainTexOffsetY")?], uv_scroll: [0.0; 2],
                dither_alpha: get("_DitherAlpha")?, use_phenomena_lighting: get("_UsePhenomenaLighting")?,
                override_shading_parameter: get("_OverrideShadingParameter")?,
                local_shading_intensity: get("_LocalShadingIntensity")?,
                local_edge_threshold: get("_LocalEdgeThreshold")?, local_edge_smoothness: get("_LocalEdgeSmoothness")?,
                uv_v_flip: 1.0, clip_corners: [[0.0; 3]; 4],
            },
            main_tex, blend: false, fresnel_power: 0.0, fresnel_color: [0.0; 4], reflection_power: 0.0, reflection_intensity: 0.0,
            crystal: None,
            site_sampler: false,
            multi_uv: None,
        },
        // GetAttribute maps Fence directly to 5, without the Basic usage/blend table.
        attribute: 5, emission: EmissionInts { bright: Some(0.0), dark: Some(0.0) },
        force_emission: false, mask_index: None, colour_picker: None, receive_shadows_off: true,
        reflection_needs_cubemap: false, crystal_blocked: None,
        unported_keywords: keywords.iter().filter(|k| !["_DISABLE_DITHER", "_RECEIVE_SHADOWS_OFF",
            "_USE_ALPHA_CLIP", "_USE_MYSEKAI_FOG", "_USE_MYSEKAI_SITE_EXTENSION", "INSTANCING_ON"].contains(k))
            .map(|k| k.to_string()).collect(),
    })
}

/// Rug Base uses source UV scroll, direct phenomena tint and treasure shadows;
/// it has no Basic half-Lambert shade, dither, Fresnel or reflection modules.
fn resolve_rug(name: &str, extras: &serde_json::Value, main_tex: Handle<Image>) -> Result<ResolvedBasic, String> {
    let get = |key: &str| extras_float(extras, key)
        .map(|v| v as f32).ok_or_else(|| format!("Rug 材质 {name} 缺属性 {key}"));
    let blend_raw = get("_FixtureObjectBlendMode")?;
    if blend_raw != 0.0 && blend_raw != 1.0 { return Err(format!("Rug 材质 {name} 混合模式越界 {blend_raw}")); }
    let blend = blend_raw == 1.0;
    for (key, expected) in [("_ZWrite", 0.0), ("_ZTest", 8.0), ("_Cull", 2.0), ("_ColorMask", 15.0),
        ("_SrcBlend", if blend { 5.0 } else { 1.0 }), ("_DstBlend", if blend { 10.0 } else { 0.0 })] {
        if get(key)? != expected { return Err(format!("Rug 材质 {name} 的 {key} 不在已实现的源状态内")); }
    }
    let keywords: Vec<&str> = extras.get("validKeywords").and_then(|v| v.as_array())
        .into_iter().flatten().filter_map(|v| v.as_str()).collect();
    let unported_keywords = keywords.iter().filter(|k| !["_USE_ALPHA_CLIP", "_USE_MYSEKAI_FOG", "_RECEIVE_SHADOWS_OFF"].contains(k))
        .map(|k| k.to_string()).collect();
    let priority = get("_RenderPriority")? as i32;
    let queue = u32::try_from(if blend { 2008 + priority } else { 2007 + priority })
        .map_err(|_| format!("Rug 材质 {name} 队列为负"))?;
    Ok(ResolvedBasic {
        material: FixtureMaterial {
            key: FixtureMaterialKey { fence: false, rug: Some(blend), render_queue: queue,
                alpha_clip: keywords.contains(&"_USE_ALPHA_CLIP"), dither: false,
                window_clip: false, fresnel: false, reflection: false },
            params: FixtureParams { main_tex_offset: [0.0; 2], uv_scroll: [get("_UVScrollX")?, get("_UVScrollY")?],
                dither_alpha: 1.0, use_phenomena_lighting: get("_UsePhenomenaLighting")?,
                override_shading_parameter: 0.0, local_shading_intensity: 0.0,
                local_edge_threshold: 0.0, local_edge_smoothness: 0.0, uv_v_flip: 1.0, clip_corners: [[0.0; 3]; 4] },
            main_tex, blend, fresnel_power: 0.0, fresnel_color: [0.0; 4], reflection_power: 0.0, reflection_intensity: 0.0,
            crystal: None,
            site_sampler: false,
            multi_uv: None,
        },
        attribute: if blend { 29 } else { 28 },
        emission: EmissionInts { bright: extras_float(extras, "_BrightPhenomenaEmission"), dark: extras_float(extras, "_DarkPhenomenaEmission") },
        force_emission: get("_EnableManualEmission")? == 1.0 || get("_DebugEmission")? == 1.0,
        receive_shadows_off: get("_ReceiveShadow")? == 0.0,
        unported_keywords, reflection_needs_cubemap: false, crystal_blocked: None,
        mask_index: extras.get("textures").and_then(|v| v.get("_EmissionMaskTex")).and_then(|v| v.as_u64()).map(|v| v as u32),
        // The Rug shader declares no colour picker (its programs are unchanged in JP).
        colour_picker: None,
    })
}

/// Orders the four unique vertices of the zero-write surface into a planar
/// convex ring (by planar polar angle about the centroid, counter-clockwise
/// seen from the normal of the first edge). The ring order is a precondition
/// of the fragment's "point inside the convex quad" test; non-planar / 
/// degenerate / non-convex input is refused here by name — a shape the data
/// should not contain, never silently approximated.
fn order_clip_quad(points: [Vec3; 4]) -> Result<[Vec3; 4], String> {
    let normal = (points[1] - points[0]).cross(points[2] - points[0]);
    let scale = normal.length();
    if scale < 1e-9 {
        return Err(format!(
            "四点退化（前三点共线）[{:?} {:?} {:?} {:?}]",
            points[0], points[1], points[2], points[3]
        ));
    }
    for point in &points {
        if (point - points[0]).dot(normal).abs() > scale * 1e-4 {
            return Err(format!(
                "四点不共面 [{:?} {:?} {:?} {:?}]",
                points[0], points[1], points[2], points[3]
            ));
        }
    }
    let u = (points[1] - points[0]).normalize();
    let v = normal.normalize().cross(u);
    let centroid = (points[0] + points[1] + points[2] + points[3]) * 0.25;
    let angle = |point: Vec3| {
        let delta = point - centroid;
        // atan2(component along v, component along u): the angle is measured
        // from the u axis towards the v axis — counter-clockwise about +n̂ in
        // the right-handed frame (u, v, n̂). Swapping the two arguments mirrors
        // the ring order to clockwise and every convexity test then comes out
        // negative (every convex quad is refused).
        delta.dot(v).atan2(delta.dot(u))
    };
    let mut ring = points;
    ring.sort_by(|a, b| angle(*a).total_cmp(&angle(*b)));
    // Convexity: the cross product of three adjacent ring points along the face
    // normal keeps one sign (strictly one sign; a degenerate edge is refused).
    for i in 0..4 {
        let a = ring[i];
        let b = ring[(i + 1) % 4];
        let c = ring[(i + 2) % 4];
        if (b - a).cross(c - b).dot(normal) <= 0.0 {
            return Err(format!(
                "四点非凸 [{:?} {:?} {:?} {:?}]",
                points[0], points[1], points[2], points[3]
            ));
        }
    }
    Ok(ring)
}

// ---- Mip chain ----

/// Decode and encode of the sRGB EOTF (the same curve the GPU uses for sRGB
/// texels). The box filter that generates mips works in the domain of the
/// sampled output (linear), and alpha does not pass through gamma.
fn srgb_decode(c: f32) -> f32 {
    if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
}

fn srgb_encode(c: f32) -> f32 {
    if c <= 0.0031308 { c * 12.92 } else { 1.055 * c.powf(1.0 / 2.4) - 0.055 }
}

/// Decode table for 8-bit stored values: entry `v` is `srgb_decode(v / 255)`,
/// computed by the same function on the same f32 input. There are only 256
/// stored values, so a table lookup is bit-identical to computing per texel;
/// the accumulation order in filtering is unchanged, so the bytes of every
/// level of the chain are unchanged too. On the encoding side the input is a
/// mean, not a finite set, so it is still computed value by value.
fn srgb_decode_table() -> &'static [f32; 256] {
    static TABLE: std::sync::OnceLock<[f32; 256]> = std::sync::OnceLock::new();
    TABLE.get_or_init(|| std::array::from_fn(|value| srgb_decode(value as f32 / 255.0)))
}

/// Generates the full 2×2 box mip chain on the CPU: for RGBA8 main textures of
/// power-of-two shapes (non-square included — the level count follows the
/// longest edge, and a side that reaches 1 first stays 1), filtering happens in
/// the domain of the sampled output (sRGB formats are decoded, averaged, then
/// encoded), and the texels are packed tightly into one data buffer in the
/// LayerMajor dense mip order (the layout wgpu's
/// `create_texture_with_data` expects, which pads row alignment internally).
///
/// A shape that cannot be handled is returned by name (no silent
/// approximation): `AlreadyChained` (the ktx/dds path already has a chain),
/// `BadShape` (either side is 0 or not a power of two — the game keeps no
/// chain for such a shape), `BadFormat` (neither RGBA8 format), `NoData` (the
/// texels are gone — taken by render extraction, later than the frame `Loaded`
/// was observed), `BadLength` (the data length does not match the size). The
/// level rule and the corpus check that every serialized mip count matches
/// value for value are in `moly_law::fixture::mip`: the four live placement
/// shapes give 256²→9, 512²→10, 2048²→12 and 2048×1024→12 (all 45
/// material→texture rows match).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MipSkip {
    AlreadyChained,
    BadShape,
    BadFormat,
    NoData,
    BadLength,
    /// Used only by [`mask_storage_mean`]: the texture is not in an sRGB
    /// format, while the shader always encodes back to the stored domain ⇒ that
    /// step would encode a second time. See the guard note on that function.
    NotSrgb,
}

/// Read-only check before filling the chain: the refusal reasons and their order
/// are those of [`generate_mip_chain`], and the values the fill needs are also
/// returned. `Assets::get_mut` emits Modified unconditionally, and a texture hit
/// by Modified is fully re-extracted with a new GPU texture — a texture that
/// already has its chain, or cannot get one, should be screened out here first
/// through a shared borrow.
pub fn mip_chain_plan(image: &Image) -> Result<MipChainPlan, MipSkip> {
    let width = image.texture_descriptor.size.width;
    let height = image.texture_descriptor.size.height;
    if image.texture_descriptor.mip_level_count > 1 {
        return Err(MipSkip::AlreadyChained);
    }
    // Level rule: a non-zero power of two has the full chain, and the level
    // count follows the longest edge. A non-square shape is not a bad shape —
    // the serialized mip counts are given that way.
    let levels = mip_levels(width, height).ok_or(MipSkip::BadShape)?;
    let srgb = match image.texture_descriptor.format {
        TextureFormat::Rgba8UnormSrgb => true,
        TextureFormat::Rgba8Unorm => false,
        _ => return Err(MipSkip::BadFormat),
    };
    let data = image.data.as_ref().ok_or(MipSkip::NoData)?;
    let base_len = (width as usize).checked_mul(height as usize)
        .and_then(|pixels| pixels.checked_mul(4)).ok_or(MipSkip::BadLength)?;
    if data.len() != base_len {
        return Err(MipSkip::BadLength);
    }
    let mut chain_len = base_len;
    let (mut w, mut h) = (width as usize, height as usize);
    for _ in 1..levels {
        w = (w / 2).max(1);
        h = (h / 2).max(1);
        chain_len = chain_len.checked_add(w * h * 4).ok_or(MipSkip::BadLength)?;
    }
    Ok(MipChainPlan { levels, srgb, base_len, chain_len })
}

/// A chain checked by [`mip_chain_plan`]: the level count, whether the format
/// is sRGB, and the byte size of level 0 and of the whole chain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MipChainPlan {
    levels: u32,
    srgb: bool,
    base_len: usize,
    chain_len: usize,
}

pub fn generate_mip_chain(image: &mut Image) -> Result<u32, MipSkip> {
    let MipChainPlan { levels, srgb, base_len, chain_len } = mip_chain_plan(image)?;
    let width = image.texture_descriptor.size.width as usize;
    let height = image.texture_descriptor.size.height as usize;
    // Reuse the decoded base-level buffer. Reserve the complete
    // chain once, then read the previous level and write its disjoint tail.
    // No full-resolution clone or per-level scratch buffer is needed; the CPU
    // image remains available to every existing caller after generation.
    let chain = image.data.as_mut().expect("base data validated above");
    chain.reserve_exact(chain_len - base_len);
    chain.resize(chain_len, 0);
    let decode = srgb_decode_table();
    let (mut src_offset, mut dst_offset) = (0, base_len);
    let (mut w, mut h) = (width, height);
    while w > 1 || h > 1 {
        let cw = (w / 2).max(1);
        let ch = (h / 2).max(1);
        // Taps per axis: 2 (box 2×2) while the parent still has ≥2 texels on
        // that axis, falling back to 1 (a direct copy) once it reaches 1. For a
        // square the filter is always 2×2, the same filter as before.
        let taps_x = w / cw;
        let taps_y = h / ch;
        let mean_scale = 1.0 / (taps_x * taps_y) as f32;
        let dst_len = cw * ch * 4;
        let (previous, tail) = chain.split_at_mut(dst_offset);
        let src = &previous[src_offset..];
        let dst = &mut tail[..dst_len];
        for y in 0..ch {
            for x in 0..cw {
                let mut acc = [0f32; 4];
                for dy in 0..taps_y {
                    for dx in 0..taps_x {
                        let i = ((y * taps_y + dy) * w + (x * taps_x + dx)) * 4;
                        for c in 0..4 {
                            let linear = if srgb && c < 3 {
                                decode[src[i + c] as usize]
                            } else {
                                src[i + c] as f32 / 255.0
                            };
                            acc[c] += linear;
                        }
                    }
                }
                let o = (y * cw + x) * 4;
                for c in 0..4 {
                    let mean = acc[c] * mean_scale;
                    let channel = if srgb && c < 3 {
                        srgb_encode(mean)
                    } else {
                        mean
                    };
                    dst[o + c] = (channel * 255.0 + 0.5).min(255.0) as u8;
                }
            }
        }
        src_offset = dst_offset;
        dst_offset += dst_len;
        w = cw;
        h = ch;
    }
    image.texture_descriptor.mip_level_count = levels;
    Ok(levels)
}

#[cfg(test)]
mod mip_tests {
    use super::*;
    use bevy::asset::RenderAssetUsages;

    fn image(width: u32, height: u32, srgb: bool, data: Vec<u8>) -> Image {
        Image::new(Extent3d { width, height, depth_or_array_layers: 1 }, TextureDimension::D2,
            data, if srgb { TextureFormat::Rgba8UnormSrgb } else { TextureFormat::Rgba8Unorm },
            RenderAssetUsages::all())
    }

    // Frozen pre-optimization buffer algorithm: a byte-equivalence oracle, not
    // an independent claim about the original client's filtering behavior.
    fn copied_chain(data: &[u8], mut w: usize, mut h: usize, srgb: bool) -> Vec<u8> {
        let mut chain = data.to_vec();
        let mut src = data.to_vec();
        while w > 1 || h > 1 {
            let (cw, ch) = ((w / 2).max(1), (h / 2).max(1));
            let (taps_x, taps_y) = (w / cw, h / ch);
            let mean_scale = 1.0 / (taps_x * taps_y) as f32;
            let mut dst = vec![0u8; cw * ch * 4];
            for y in 0..ch {
                for x in 0..cw {
                    let mut acc = [0f32; 4];
                    for dy in 0..taps_y {
                        for dx in 0..taps_x {
                            let i = ((y * taps_y + dy) * w + (x * taps_x + dx)) * 4;
                            for c in 0..4 {
                                let channel = src[i + c] as f32 / 255.0;
                                acc[c] += if srgb && c < 3 { srgb_decode(channel) } else { channel };
                            }
                        }
                    }
                    let o = (y * cw + x) * 4;
                    for c in 0..4 {
                        let mean = acc[c] * mean_scale;
                        let channel = if srgb && c < 3 { srgb_encode(mean) } else { mean };
                        dst[o + c] = (channel * 255.0 + 0.5).min(255.0) as u8;
                    }
                }
            }
            chain.extend_from_slice(&dst);
            src = dst;
            w = cw; h = ch;
        }
        chain
    }

    #[test]
    fn in_place_chain_is_byte_identical_for_both_color_spaces_and_short_edges() {
        for (w, h) in [(1, 1), (1, 16), (16, 1), (2, 8), (8, 2), (8, 8), (16, 8)] {
            for srgb in [false, true] {
                let data: Vec<u8> = (0..w * h * 4).map(|i| ((i * 73 + i / 7 * 19) % 256) as u8).collect();
                let expected = copied_chain(&data, w as usize, h as usize, srgb);
                let mut image = image(w, h, srgb, data);
                assert_eq!(generate_mip_chain(&mut image), mip_levels(w, h).ok_or(MipSkip::BadShape));
                assert_eq!(image.data.as_ref().unwrap(), &expected, "{w}x{h}, srgb={srgb}");
                assert_eq!(image.asset_usage, RenderAssetUsages::all());
            }
        }
    }

    #[test]
    fn reserved_image_storage_is_reused_without_changing_alpha_or_the_base_level() {
        for (srgb, rgb) in [(false, 128), (true, 188)] {
            let data = [0, 0, 0, 0, 255, 255, 255, 255].repeat(2);
            let mut image = image(2, 2, srgb, data.clone());
            image.data.as_mut().unwrap().reserve_exact(4);
            let base_pointer = image.data.as_ref().unwrap().as_ptr();
            assert_eq!(generate_mip_chain(&mut image), Ok(2));
            let chain = image.data.as_ref().unwrap();
            assert_eq!(chain.as_ptr(), base_pointer);
            assert_eq!(&chain[..16], data.as_slice());
            assert_eq!(&chain[16..], &[rgb, rgb, rgb, 128]);
            let expected = chain.clone();
            assert_eq!(generate_mip_chain(&mut image), Err(MipSkip::AlreadyChained));
            assert_eq!(image.data.as_ref().unwrap(), &expected);
        }
    }

    #[test]
    fn rejected_mip_input_keeps_its_original_data_and_descriptor() {
        let mut image = image(2, 2, false, vec![37; 16]);
        image.data.as_mut().unwrap().pop();
        let expected = image.data.clone();
        assert_eq!(generate_mip_chain(&mut image), Err(MipSkip::BadLength));
        assert_eq!(image.data, expected);
        assert_eq!(image.texture_descriptor.mip_level_count, 1);
    }
}

/// Fills the chain and hands the texels to the render world: extraction in this
/// frame moves the whole chain instead of copying it, leaving only the
/// descriptor in the main world. A texture that cannot be chained, or already
/// is, is screened out by [`mip_chain_plan`] first — no mutable borrow, no
/// Modified event. A handle not in Assets is reported as `NoData`.
///
/// After the texels are handed over, only one reader in the main world wants
/// them again: later swap batches need the mask mean. The mean is computed and
/// recorded in the frame of the hand-over (while the data is still there), see
/// [`MaskMeans`].
///
/// The chained upload replaces the single-level texture the image already
/// has on the GPU; the replacement is recorded so that the old texture is
/// destroyed together with the image (see [`crate::gpu_image_release`]).
fn chain_and_release(
    images: &mut Assets<Image>,
    replaced: &mut MessageWriter<crate::gpu_image_release::ImageTextureReplaced>,
    handle: &Handle<Image>,
) -> Result<u32, MipSkip> {
    mip_chain_plan(images.get(handle).ok_or(MipSkip::NoData)?)?;
    let mut image = images.get_mut(handle).ok_or(MipSkip::NoData)?;
    let levels = generate_mip_chain(&mut image)?;
    moly_assets::residency::release_to_render_world(&mut image);
    replaced.write(crate::gpu_image_release::ImageTextureReplaced(handle.id()));
    Ok(levels)
}

/// The mask means already computed, recorded per texture asset. The mean reads
/// only the stored values of level 0, and level 0 never changes during an
/// asset's life (chain filling only appends after it), so after the texels are
/// handed to the render world a later swap batch gets the same number from this
/// record as from the texels. Dropped when the asset is removed.
#[derive(Default)]
struct MaskMeans(HashMap<AssetId<Image>, [f32; 3]>);

impl MaskMeans {
    fn read(&mut self, id: AssetId<Image>, image: &Image) -> Result<[f32; 3], MipSkip> {
        match mask_storage_mean(image) {
            Ok(mean) => {
                self.0.insert(id, mean);
                Ok(mean)
            }
            Err(MipSkip::NoData) => self.0.get(&id).copied().ok_or(MipSkip::NoData),
            Err(reason) => Err(reason),
        }
    }
}

/// Per-channel mean of level 0 of the mask texture, taken **in the stored
/// (gamma) domain** — the emission buffer holds stored-domain values
/// (`shaders/fixture_emission.wesl` encodes back to the stored domain after
/// sampling, and that buffer is `Rgba16Float`, which the hardware no longer
/// encodes), and the account mean is comparable only in the same domain. An
/// 8-bit texel already is the stored value, so nothing is **decoded** here.
/// Alpha is dropped.
///
/// ⚠ Decoding into the linear domain used to be done here, and that was the
/// right measure for an earlier buffer domain; whenever the buffer domain
/// changes, this measure must change with it, or the account reports numbers
/// in a domain the buffer no longer holds.
///
/// An unhandleable shape (neither RGBA8 format / texels missing / length
/// mismatch) is returned by name — the account row then records None, never a
/// silent 0. Mip chain generation does not change level 0, so reading after the
/// chain is filled gives the same result.
///
/// **`NotSrgb` is a fail-closed guard, not a shape refusal**: on the shader
/// side the encode back to the stored domain is **unconditional**, and that
/// step is an identity round trip only when the hardware really decoded first
/// (= the texture was loaded as sRGB). The mask is loaded through its glb
/// sub-asset label (bevy_gltf loads as sRGB a texture that no material
/// references with a linear usage), so the normal case is sRGB; if one ever
/// arrives in a linear format, the shader **encodes once more**, the whole
/// emission turns too bright, and that is a silent error. ⇒ report it here by
/// name instead of letting it drift silently.
fn mask_storage_mean(image: &Image) -> Result<[f32; 3], MipSkip> {
    match image.texture_descriptor.format {
        TextureFormat::Rgba8UnormSrgb => {}
        TextureFormat::Rgba8Unorm => return Err(MipSkip::NotSrgb),
        _ => return Err(MipSkip::BadFormat),
    }
    let data = image.data.as_ref().ok_or(MipSkip::NoData)?;
    let width = image.texture_descriptor.size.width;
    let height = image.texture_descriptor.size.height;
    // After mip generation the data holds the whole chain; the mean reads only
    // level 0 (the account row measures the overall mask brightness, and the
    // whole-image mean of level 0 is the basis of that derivation). Data shorter
    // than one level is genuinely missing data.
    let level0 = (width * height * 4) as usize;
    if data.len() < level0 {
        return Err(MipSkip::BadLength);
    }
    let mut acc = [0f32; 3];
    for i in (0..level0).step_by(4) {
        for c in 0..3 {
            acc[c] += data[i + c] as f32 / 255.0;
        }
    }
    let count = (width * height) as f32;
    Ok([acc[0] / count, acc[1] / count, acc[2] / count])
}

// ---- Swap ----

/// Completion marker of the material swap.
#[derive(Resource)]
pub struct FixtureMaterialsSwapped;

/// One entry of the swap plan: the resolved furniture material and its glb
/// material handle.
struct Planned {
    force_emission: bool,
    name: String,
    material: FixtureMaterial,
    /// glb material handle the entry was resolved from: the swap replaces by
    /// it.
    source: Handle<StandardMaterial>,
    /// Two-dimensional selection table result (5/6 ordinary furniture, 26/27
    /// window outside), compared in the sampling log to confirm the window
    /// family takes the right arm.
    attribute: u32,
    /// The emission ints (they enter the [`FixtureEmission`] component).
    emission: EmissionInts,
    /// Variant switch panel entry of the sampling log, see [`ResolvedBasic`].
    receive_shadows_off: bool,
    unported_keywords: Vec<String>,
    /// The reflection branch is on but the material binds a real cube map (the
    /// folded form is wrong for it, so the branch is off).
    reflection_needs_cubemap: bool,
    /// Crystal keyword on, branch off (named reason).
    crystal_blocked: Option<String>,
    /// JP `_UseEmissionColorPicker` on: the second target's colour.
    colour_picker: Option<[f32; 3]>,
    /// The main texture is the shader's declared default (`white`) because
    /// the material's `_MainTex` slot is empty.
    default_main_tex: bool,
    /// Emission mask handle: loaded at build time from the same glb by the
    /// texture index in the extras. None = the material has no mask (no
    /// contribution is lost when the gate ints are all zero; a non-zero gate is
    /// treated as zero contribution with a named warning, fail-closed).
    mask: Option<Handle<Image>>,
    /// The data-side scale/offset 4-tuple of `_MainTex` (the extras'
    /// `textureScaleOffset`), used only for comparison in the swap's sampling
    /// log — the Basic variant does not consume `_MainTex_ST` (it is culled at
    /// compile time), and the shader only adds the uv offset. A missing value is
    /// recorded as None, never silently taken as identity.
    main_tex_st: Option<[f64; 4]>,
}

/// The complete state of one swap.
struct SwapPlan {
    planned: Vec<Planned>,
    /// Handles of materials whose render state writes nothing, with their shader
    /// attribute: at execution the mesh entities with these handles are hidden
    /// and the attribute is attached for stencil replay to read.
    stencil_only: Vec<(Handle<StandardMaterial>, Option<u8>)>,
    /// Handles of ShadowMesh family materials: always 0 pixels in the game,
    /// hidden at execution.
    shadow_mesh: Vec<Handle<StandardMaterial>>,
    tally: SwapTally,
}

type PendingVisuals<'w, 's> = Query<'w, 's, (Entity, &'static FixtureVisualRoot),
    (With<FixtureVisualSceneReady>, Without<FixtureVisualReady>)>;

/// The counters: emitted together as `info!`/`warn!` when a swap completes,
/// computed at that moment to show that the swap really took place.
#[derive(Default)]
struct SwapTally {
    fence_materials: usize,
    fence_entities: usize,
    rug_materials: usize,
    rug_entities: usize,
    basic_materials: usize,
    basic_entities: usize,
    /// Sub-count of blended materials (`_FixtureObjectBlendMode` = 1).
    blended_materials: usize,
    /// Sub-count of window-outside arm materials (usage=1 → selection 26/27).
    window_clip_materials: usize,
    /// Number of entities marked with the wall-layout marker.
    wall_entities: usize,
    /// Named refusals: family gate / required float / two-dimensional
    /// selection / missing main texture / unusable window gate corners.
    refused: Vec<String>,
    /// Families inside the closed set but not Basic (out of scope), kept by
    /// name.
    unported: Vec<String>,
    /// Render state writes nothing (`ColorMask` 0 + `ZWrite` 0): hidden material
    /// names and entity count.
    stencil_hidden_names: Vec<String>,
    stencil_hidden_entities: usize,
    /// ShadowMesh family (`_Show` always 0): hidden material names and entity
    /// count.
    shadow_mesh_names: Vec<String>,
    shadow_mesh_entities: usize,
    /// Valid keywords with no branch in this pipeline: named per material
    /// (`material (keyword)`), never swallowed silently.
    unported_keyword_materials: Vec<String>,
    /// Sub-count of materials whose reflection branch is on (empty cube-map
    /// slot ⇒ the sample folds to a constant).
    reflection_materials: usize,
    /// Names of materials with the reflection branch on that bind a real cube
    /// map: the folded form is wrong for them and this pipeline keeps the branch
    /// off (the keyword also stays in the unimplemented account).
    reflection_needs_cubemap_names: Vec<String>,
    /// Materials drawing the crystal branch.
    crystal_materials: usize,
    /// Crystal keyword on, branch off: named reasons.
    crystal_blocked: Vec<String>,
    /// Materials whose empty `_MainTex` slot takes the shader default.
    default_main_tex_materials: usize,
}

/// Update system: once every furniture scene is expanded (`FixtureScenesReady`,
/// the fixture module's latch), build the plan, wait for all textures and swap
/// once. Before that it does nothing each frame.
fn switch_materials(
    mut commands: Commands,
    ready: Option<Res<FixtureScenesReady>>,
    swapped: Option<Res<FixtureMaterialsSwapped>>,
    server: Res<AssetServer>,
    env: Res<SiteEnv>,
    (roots, hidden_residents): (PendingVisuals, Query<(), With<crate::render_inputs::HiddenResidentVisual>>),
    children: Query<&Children>,
    parts: Query<(
        &MeshMaterial3d<StandardMaterial>,
        &GltfMaterialName,
        &GltfMaterialExtras,
    )>,
    mesh_parts: Query<&Mesh3d>,
    meshes: Res<Assets<Mesh>>,
    std_materials: Res<Assets<StandardMaterial>>,
    source_textures: Query<&SourceMaterialTextures>,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<FixtureMaterial>>,
    mut plan: Local<Option<SwapPlan>>,
    layout: (Res<surfaces::FixtureSurfaceReadiness>, Res<crate::render_inputs::FixtureLayoutRevision>, Local<u64>, Local<Vec<Entity>>, Local<MaskMeans>,
        MessageWriter<crate::gpu_image_release::ImageTextureReplaced>),
) {
    let (surfaces_ready, revision, mut seen_revision, mut seen_roots, mut mask_means, mut replaced) = layout;
    let mut pending_roots: Vec<_> = roots.iter().map(|(entity, _)| entity).collect();
    pending_roots.sort_unstable();
    if *seen_roots != pending_roots {
        *plan = None;
        *seen_roots = pending_roots;
    }
    if *seen_revision != revision.0 {
        *plan = None;
        *seen_revision = revision.0;
    }
    if swapped.is_some() && roots.is_empty() {
        return;
    }
    if ready.is_none() {
        return;
    }
    if !surfaces_ready.0 { return; }
    // The shader's declared `white` default for an empty `_MainTex` slot:
    // Unity binds its built-in white texture, (1, 1, 1, 1) in the stored
    // domain. Bevy's image plugin registers the same thing at the default
    // handle: a 1x1 image of all-255 bytes in the default sRGB format.
    let white = Handle::<Image>::default();
    let Some(mut state) = plan.take().or_else(|| {
        build_swap_plan(
            &roots,
            &children,
            &parts,
            &mesh_parts,
            &meshes,
            &std_materials,
            &source_textures,
            &white,
        )
    }) else {
        return;
    };

    // Wait until every texture is present: swapping before that would flash the
    // entities back to the default material. A failed load panics by name.
    // The shared default image is added directly to the assets, so the asset
    // server has no load state for it; it counts as loaded once it exists.
    let mut all_loaded = true;
    for item in &state.planned {
        let normal_map = item.material.crystal.as_ref()
            .and_then(|crystal| crystal.normal_map.as_ref())
            .map(|normal_map| &normal_map.texture);
        let layers = item.material.multi_uv.as_ref()
            .map(|multi| multi.textures.iter().flatten().collect::<Vec<_>>())
            .unwrap_or_default();
        for (texture, what) in [
            (Some(&item.material.main_tex), "贴图"),
            (item.mask.as_ref(), "自发光遮罩"),
            (normal_map, "normal map"),
        ].into_iter().chain(layers.into_iter().map(|layer| (Some(layer), "multi-UV layer"))) {
            let Some(texture) = texture else { continue };
            match server.get_load_state(texture.id()) {
                Some(LoadState::Failed(err)) => {
                    panic!("家具材质 {} 的{what}装载失败：{err:?}", item.name)
                }
                Some(LoadState::Loaded) => {}
                None if images.contains(texture.id()) => {}
                _ => all_loaded = false,
            }
        }
    }
    if !all_loaded {
        *plan = Some(state);
        return;
    }

    // Interlock: the shader always re-encodes the sampled normal map into the
    // stored domain, which is the identity round trip only for an sRGB-format
    // texture (how bevy_gltf loads an extras-only texture). A linear one would
    // be encoded once too often; its crystal branch is kept off by name instead
    // of drawing a bent normal.
    for item in &mut state.planned {
        let format = item.material.crystal.as_ref()
            .and_then(|crystal| crystal.normal_map.as_ref())
            .and_then(|normal_map| images.get(&normal_map.texture))
            .map(|image| image.texture_descriptor.format);
        if let Some(format) = format.filter(|format| *format != TextureFormat::Rgba8UnormSrgb) {
            item.material.crystal = None;
            item.crystal_blocked = Some(format!(
                "{}: _NormalMap loaded as {format:?}, not sRGB; the stored-domain re-encode would be wrong",
                item.name
            ));
            item.unported_keywords.push("_ENABLE_CRYSTAL_FIXTURE".into());
        }
    }

    // Filling the mip chain (a step of the same swap): for these power-of-two
    // textures the GPU generates a full chain and samples it trilinearly;
    // bevy's PNG load path always leaves mip_level_count = 1, so minified
    // sampling aliases without mips. This must run in the same frame `Loaded`
    // was observed: render extraction takes the `data` of the main world, and
    // one frame later the texels are gone.
    // Modifying the main-world `Image` emits Modified and the render side
    // rebuilds the GPU texture from the new descriptor and data. Masks are
    // handled in the same batch: the emission pass samples them with the global
    // mip bias too.
    let mut mipped: Vec<(&str, u32)> = Vec::new();
    // One texture shared by several materials: once the first material has
    // filled the chain, the later ones find the chain already there
    // (AlreadyChained) — not a skip, but counted separately as shared (a mask
    // sharing its handle with the main texture, or one mask shared by several
    // materials, falls into this count).
    let mut shared_chains = 0usize;
    let mut mip_skipped: Vec<(&str, MipSkip)> = Vec::new();
    for item in &state.planned {
        let name = item.name.as_str();
        // The shared 1x1 default has a single level already.
        if !item.default_main_tex {
            match chain_and_release(&mut images, &mut replaced, &item.material.main_tex) {
                Ok(levels) => mipped.push((name, levels)),
                Err(MipSkip::AlreadyChained) => shared_chains += 1,
                Err(reason) => mip_skipped.push((name, reason)),
            }
        }
        if let Some(mask) = &item.mask {
            match chain_and_release(&mut images, &mut replaced, mask) {
                Ok(_) => {}
                Err(MipSkip::AlreadyChained) => shared_chains += 1,
                Err(reason) => mip_skipped.push((name, reason)),
            }
        }
        // The normal map is sampled with the global mip bias like the main
        // texture, so it needs the same full chain.
        let normal_map = item.material.crystal.as_ref()
            .and_then(|c| c.normal_map.as_ref())
            .map(|normal_map| &normal_map.texture);
        let layers = item.material.multi_uv.iter().flat_map(|multi| multi.textures.iter().flatten());
        for texture in normal_map.into_iter().chain(layers) {
            match chain_and_release(&mut images, &mut replaced, texture) {
                Ok(_) => {}
                Err(MipSkip::AlreadyChained) => shared_chains += 1,
                Err(reason) => mip_skipped.push((name, reason)),
            }
        }
    }
    info!(
        "家具贴图 mip 链：补 {} 张（2×2 box，sRGB 在线性域滤波），逐张级数 \
         {mipped:?}（级数按最长边，与源序列化的 mip 计数一致，非方形不是坏形状）；\
         共享句柄复用已有链 {} 次；跳过 {} 张（具名原因）：{skipped:?}",
        mipped.len(),
        shared_chains,
        mip_skipped.len(),
        mipped = mipped,
        skipped = mip_skipped,
    );

    // Emission account: the mask mean is read before render extraction takes
    // the texels (in the same frame as the swap). Collected when the mask is at
    // hand (also when the gate ints are all zero — the shader compares the gate
    // by phenomena type, which turns non-zero on a type switch) or when a gate
    // int is non-zero; a material with neither contributes a constant 0 and is
    // not accounted. The weather tier-switch log reads this account to derive
    // each material's contribution row.
    let mut account = EmissionAccount::default();
    let mut mask_mean_skipped: Vec<(String, MipSkip)> = Vec::new();
    mask_means.0.retain(|id, _| images.contains(*id));
    for item in &state.planned {
        let armed = item.mask.is_some()
            || item.emission.bright == Some(1.0)
            || item.emission.dark == Some(1.0);
        if !armed {
            continue;
        }
        // A non-zero gate int with a missing mask: fail-closed zero contribution
        // (no silent blank mask), with a named warning — entities of such a
        // combination get no emission component.
        if let Some(reason) = &item.crystal_blocked {
            // The crystal program's second target runs the normal-driven step
            // and the centre damp on the emission colour; without the crystal
            // branch those steps have no inputs, so nothing is drawn.
            warn!(
                "Fixture material {}: second colour target not drawn, the crystal branch is off ({reason})",
                item.name
            );
        } else if item.mask.is_none() && item.colour_picker.is_none() {
            warn!(
                "家具材质 {} 的门 int 非零但 extras 缺 _EmissionMaskTex：\
                 自发光按零贡献处理（fail-closed，不取空白遮罩）",
                item.name
            );
        }
        let mask_mean = match &item.mask {
            Some(mask) => match images.get(mask) {
                Some(image) => match mask_means.read(mask.id(), image) {
                    Ok(mean) => Some(mean),
                    Err(reason) => {
                        mask_mean_skipped.push((item.name.clone(), reason));
                        None
                    }
                },
                None => {
                    mask_mean_skipped.push((item.name.clone(), MipSkip::NoData));
                    None
                }
            },
            None => None,
        };
        account.rows.push(EmissionRow {
            name: item.name.clone(),
            bright: item.emission.bright,
            dark: item.emission.dark,
            mask_mean,
        });
    }
    if !mask_mean_skipped.is_empty() {
        warn!(
            "自发光遮罩均值不可读 {} 条（账目行记 None，贡献推导不出）：{:?}",
            mask_mean_skipped.len(),
            mask_mean_skipped
        );
    }

    // Swap execution: map each mesh entity to a plan index by its glb material
    // handle and swap one by one. After ready the expanded entity set no longer
    // changes, so one pass over the hierarchy is enough.
    let mut by_handle: HashMap<Handle<StandardMaterial>, usize> = HashMap::new();
    for (index, item) in state.planned.iter().enumerate() {
        by_handle.insert(item.source.clone(), index);
    }
    let mut swapped_entities: HashMap<usize, usize> = HashMap::new();
    // Fence's source draw manager groups instances by mesh/material/texture.
    // Reusing its material handle preserves Bevy instancing across source parts.
    // Basic keeps per-entity materials for animated face/UV parameters.
    let mut fence_materials: HashMap<usize, Handle<FixtureMaterial>> = HashMap::new();
    for (root_entity, placement) in roots.iter() {
        let wall = placement.is_wall_layout();
        let mut stack = vec![root_entity];
        while let Some(entity) = stack.pop() {
            if let Ok(kids) = children.get(entity) {
                stack.extend(kids.iter().map(|child| child));
            }
            let Ok((material, _, _)) = parts.get(entity) else {
                // An entity without a glb material name/extras (not on the glb
                // material path): keep its original material and do not count it
                // here (it cannot be named).
                continue;
            };
            if let Some(index) = by_handle.get(&material.0).copied() {
                let item = &state.planned[index];
                let handle = if item.material.key.fence {
                    fence_materials.entry(index)
                        .or_insert_with(|| materials.add(item.material.clone())).clone()
                } else {
                    materials.add(item.material.clone())
                };
                commands
                    .entity(entity)
                    .remove::<MeshMaterial3d<StandardMaterial>>()
                    .insert(MeshMaterial3d(handle));
                // Emission eligibility: inserted when the mask is at hand (also
                // when the gate ints are all zero — the fragment compares the
                // gate by phenomena type, which is zero then and turns live on a
                // type switch). Materials without a mask are skipped (gate ints
                // that are non-zero with no mask already got a named warning in
                // the accounting step).
                // The colour picker replaces the mask texel, so a picker
                // material without a mask binds its main texture in the
                // unread mask slot. A blocked crystal draws no second target.
                let emission_mask = item.mask.clone()
                    .or_else(|| item.colour_picker.map(|_| item.material.main_tex.clone()))
                    .filter(|_| item.crystal_blocked.is_none());
                if let Some(mask) = emission_mask {
                    commands.entity(entity).insert(FixtureEmission {
                        mask,
                        main_tex: item.material.main_tex.clone(),
                        params: item.material.params,
                        key: item.material.key,
                        blend: item.material.blend,
                        // A missing key counts as 0: the shader's property
                        // default is 0.
                        bright: item.emission.bright.unwrap_or(0.0) as f32,
                        dark: item.emission.dark.unwrap_or(0.0) as f32,
                        force_emission: item.force_emission,
                        colour_picker: item.colour_picker,
                        crystal: item.material.crystal.clone(),
                    });
                }
                *swapped_entities.entry(index).or_default() += 1;
                if wall {
                    commands.entity(entity).insert(WallLayoutShadowCasterOff);
                    state.tally.wall_entities += 1;
                }
            } else if let Some((_, attribute)) =
                state.stencil_only.iter().find(|(handle, _)| handle == &material.0)
            {
                let mut entity = commands.entity(entity);
                entity.insert((StencilOnlyPass, Visibility::Hidden));
                if let Some(attribute) = *attribute {
                    entity.insert(SourceStencilAttribute(attribute));
                }
                state.tally.stencil_hidden_entities += 1;
            } else if state.shadow_mesh.iter().any(|handle| handle == &material.0) {
                commands
                    .entity(entity)
                    .insert((ShadowMeshGated, Visibility::Hidden));
                state.tally.shadow_mesh_entities += 1;
            }
        }
        // A visible glTF scene would first enter the render queues with the
        // importer's material. Publish the prepared hierarchy in the same
        // command batch as the material replacements and hidden source passes.
        let visibility = if hidden_residents.contains(root_entity) { Visibility::Hidden } else { Visibility::Inherited };
        commands.entity(root_entity).insert((visibility, FixtureVisualReady));
    }
    for (index, entities) in &swapped_entities {
        let item = &state.planned[*index];
        if item.material.key.fence {
            state.tally.fence_materials += 1;
            state.tally.fence_entities += entities;
        } else if item.material.key.rug.is_some() {
            state.tally.rug_materials += 1;
            state.tally.rug_entities += entities;
        } else {
            state.tally.basic_materials += 1;
            state.tally.basic_entities += entities;
        }
        if item.material.blend {
            state.tally.blended_materials += 1;
        }
        if item.material.key.window_clip {
            state.tally.window_clip_materials += 1;
        }
        if item.material.key.reflection {
            state.tally.reflection_materials += 1;
        }
        if item.reflection_needs_cubemap {
            state
                .tally
                .reflection_needs_cubemap_names
                .push(item.name.clone());
        }
        if item.material.crystal.is_some() {
            state.tally.crystal_materials += 1;
        }
        if let Some(reason) = &item.crystal_blocked {
            state.tally.crystal_blocked.push(reason.clone());
        }
        if item.default_main_tex {
            state.tally.default_main_tex_materials += 1;
        }
        if !item.unported_keywords.is_empty() {
            state.tally.unported_keyword_materials.push(format!(
                "{} ({})",
                item.name,
                item.unported_keywords.join(", ")
            ));
        }
    }

    info!(
        "家具材质换装：Basic {} 材质 {} 实体（混合 {} 材质，窗外观 {} 材质，反射折叠 {} 材质）；墙布局标记 {} 实体；\
         写零 pass 隐藏 {} 材质 {} 实体；ShadowMesh 族隐藏 {} 材质 {} 实体；\
         未实现 keyword 分支 {} 材质",
        state.tally.basic_materials,
        state.tally.basic_entities,
        state.tally.blended_materials,
        state.tally.window_clip_materials,
        state.tally.reflection_materials,
        state.tally.wall_entities,
        state.tally.stencil_hidden_names.len(),
        state.tally.stencil_hidden_entities,
        state.tally.shadow_mesh_names.len(),
        state.tally.shadow_mesh_entities,
        state.tally.unported_keyword_materials.len(),
    );
    if state.tally.fence_entities > 0 {
        info!("Fence 材质换装：{} 材质 {} 实体；源队列 2065 + priority；无 alpha clip / fog / treasure shadow / emission", state.tally.fence_materials, state.tally.fence_entities);
    }
    if state.tally.rug_entities > 0 {
        info!("Rug 材质换装：{} 材质 {} 实体；源队列 2007/2008 + priority；ZTest Always / ZWrite Off", state.tally.rug_materials, state.tally.rug_entities);
    }
    if !state.tally.stencil_hidden_names.is_empty() {
        info!(
            "写零 pass（ColorMask 0 + ZWrite 0，只写模板缓冲）：{:?}",
            state.tally.stencil_hidden_names
        );
    }
    if !state.tally.shadow_mesh_names.is_empty() {
        info!(
            "ShadowMesh 族（源片元以 _Show 为门且 live 数据恒 0，像素贡献为 0）：{:?}",
            state.tally.shadow_mesh_names
        );
    }
    // Per-parameter sampling log (acceptance check: the light colour,
    // threshold and ST values in use can be derived from the log).
    // The three threshold values are picked as local or global by
    // _OverrideShadingParameter; both sides are printed, because the choice
    // itself is also something to derive.
    for item in &state.planned {
        let p = &item.material.params;
        let use_local = p.override_shading_parameter > 0.5;
        let (threshold, smoothness, intensity) = if use_local {
            (
                p.local_edge_threshold,
                p.local_edge_smoothness,
                p.local_shading_intensity,
            )
        } else {
            (
                env.globals.edge_threshold,
                env.globals.edge_smoothness,
                1.0,
            )
        };
        // Emission account row: derive the gate and contribution from the
        // phenomena type of the swap frame (gate × mask mean; gate 0 contributes
        // [0,0,0], and a gate that is on with a missing or unreadable mask
        // records None).
        let emission_type = env.emission_type as i32;
        let gate_now = emission_gate(&item.emission, emission_type);
        let mask_mean = account
            .rows
            .iter()
            .find(|row| row.name == item.name)
            .and_then(|row| row.mask_mean);
        info!(
            "家具材质采样 {name}：现象光门 {gate} 阈值 {threshold:.4}（局部 {lt:.4}/全局 {gt:.4}）\
             平滑度 {smoothness:.4}（局部 {ls:.4}/全局 {gs:.4}）强度 {intensity:.2}（局部 {li:.2}/全局 1.00）\
             阴面色 {shade:?} 光色 {light:?} uv=uv0+({ox:.2},{oy:.2}) 数据侧 ST {st:?}（源变体不乘 ST）\
             变体 clip={clip} dither={dither}（ditherAlpha={da:.2}）rso={rso}（源程序级零效果）\
             fresnel={fresnel}（power={fp:.2} color={fc:?}；live 带它的材质全未摆放）\
             reflection={reflection}（power={rp:.2} intensity={ri:.4} 立方图采样折叠为常数 {cube:.7}\
             ⇒ 掠射角每通道加 {edge:.7}）\
             未实现={unported:?} blend={blend}\
             自发光 亮臂 {bright:?} 暗臂 {dark:?}（现象类型 {etype} 门 {egate:?}，遮罩均值 {emean:?} ⇒ 贡献 {contrib:?}）",
            name = item.name,
            gate = p.use_phenomena_lighting,
            threshold = threshold,
            lt = p.local_edge_threshold,
            gt = env.globals.edge_threshold,
            smoothness = smoothness,
            ls = p.local_edge_smoothness,
            gs = env.globals.edge_smoothness,
            intensity = intensity,
            li = p.local_shading_intensity,
            shade = env.globals.phenomena_shade_color,
            light = env.globals.phenomena_directional_light_color,
            ox = p.main_tex_offset[0],
            oy = p.main_tex_offset[1],
            st = item.main_tex_st,
            clip = item.material.key.alpha_clip,
            dither = item.material.key.dither,
            da = p.dither_alpha,
            rso = item.receive_shadows_off,
            fresnel = item.material.key.fresnel,
            fp = item.material.fresnel_power,
            fc = item.material.fresnel_color,
            reflection = item.material.key.reflection,
            rp = item.material.reflection_power,
            ri = item.material.reflection_intensity,
            cube = moly_law::fixture::fresnel::UNBOUND_CUBE_SAMPLE,
            // What this branch adds to each channel at grazing angles (N·V → 0,
            // fresnel = 1 after the upper clamp): the sample constant ×
            // intensity. Facing the camera the term tends to 0, so this is its
            // upper bound, and the strength of the rim highlight on screen.
            edge = moly_law::fixture::fresnel::UNBOUND_CUBE_SAMPLE
                * item.material.reflection_intensity,
            unported = item.unported_keywords,
            blend = item.material.blend,
            bright = item.emission.bright,
            dark = item.emission.dark,
            etype = emission_type,
            egate = gate_now,
            emean = mask_mean,
            contrib = match (gate_now, mask_mean) {
                (Some(1), Some(mean)) => Some(mean),
                (Some(0), _) => Some([0.0; 3]),
                _ => None,
            },
        );
        // Window-outside arm check: the selection (26/27 rather than 5/6) and the
        // gate corners — the log shows that the window family takes the right
        // arm.
        if item.material.key.window_clip {
            let corners = item.material.params.clip_corners;
            info!(
                "窗外观支路 {name}：二维表落点 attr {attribute}（26 不透明/27 混合，非 5/6），\
                 屏空间门四角（写零面顶点，本网格局部系，环序）[{a:?} {b:?} {c:?} {d:?}]",
                name = item.name,
                attribute = item.attribute,
                a = corners[0],
                b = corners[1],
                c = corners[2],
                d = corners[3],
            );
        }
    }
    // Live global values: the `SiteEnv` of the frame the swap completes (with
    // values the weather system has taken over).
    info!(
        "站点全局量实况（换装时）：光向 {:?} 光色 {:?} 阴面色 {:?} 全局阈值 {} 平滑度 {} \
         mip 偏置 {:?} 宝藏位 {:?}/{:?} 宝藏强度 {:?}",
        env.globals.light_vector,
        env.globals.phenomena_directional_light_color,
        env.globals.phenomena_shade_color,
        env.globals.edge_threshold,
        env.globals.edge_smoothness,
        env.globals.mip_bias,
        env.globals.treasure_positions[0],
        env.globals.treasure_positions[1],
        env.globals.treasure_shadow_intensity,
    );
    if !state.tally.refused.is_empty() {
        warn!(
            "家具具名拒绝 {} 条：{:?}",
            state.tally.refused.len(),
            state.tally.refused
        );
    }
    if !state.tally.unported.is_empty() {
        warn!(
            "闭集内非 Basic 族（本单范围外，保留原材质）{} 条：{:?}",
            state.tally.unported.len(),
            state.tally.unported
        );
    }
    if !state.tally.unported_keyword_materials.is_empty() {
        warn!(
            "未实现 keyword 分支（源变体开着、本管线没有的分支，逐材质具名）{} 条：{:?}",
            state.tally.unported_keyword_materials.len(),
            state.tally.unported_keyword_materials
        );
    }
    if !state.tally.reflection_needs_cubemap_names.is_empty() {
        warn!(
            "反射分支开着但材质绑了真立方图（未绑定时的常数折叠对它们是错的，\
             本管线没有真采样那条路 ⇒ 分支未开）{} 条：{:?}",
            state.tally.reflection_needs_cubemap_names.len(),
            state.tally.reflection_needs_cubemap_names
        );
    }
    info!(
        "Fixture crystal branch: {} materials drawn; empty _MainTex slots on the shader's white default: {} materials",
        state.tally.crystal_materials, state.tally.default_main_tex_materials,
    );
    if !state.tally.crystal_blocked.is_empty() {
        warn!(
            "Fixture crystal branch off for {} materials (named): {:?}",
            state.tally.crystal_blocked.len(),
            state.tally.crystal_blocked
        );
    }
    // Register the account: on a weather tier switch it reports emission
    // contribution per material name (the contribution rows are computed from
    // this account — the arms can be derived from the log).
    if swapped.is_none() {
        commands.queue(move |world: &mut World| {
            let mut account = account;
            if let Some(EmissionAccountKept(kept)) = world.remove_resource::<EmissionAccountKept>() {
                let appended = std::mem::replace(&mut account.rows, kept.rows);
                account.rows.extend(appended);
            }
            world.insert_resource(account);
        });
        commands.insert_resource(FixtureMaterialsSwapped);
    }
}

/// Builds the plan: walks the hierarchy down from each furniture root, collects
/// the first entity seen per glb material handle and resolves each one
/// (material name + extras + main texture handle). The wall-layout marker is
/// attached per entity at execution (the same handle may appear under wall and
/// non-wall placements). The window-outside gate corners are read from the
/// zero-write surface mesh of the same glb by the first root seen (repeated
/// placements of one handle share the same glb, so the corners are identical).
fn build_swap_plan(
    roots: &PendingVisuals,
    children: &Query<&Children>,
    parts: &Query<(
        &MeshMaterial3d<StandardMaterial>,
        &GltfMaterialName,
        &GltfMaterialExtras,
    )>,
    mesh_parts: &Query<&Mesh3d>,
    meshes: &Assets<Mesh>,
    std_materials: &Assets<StandardMaterial>,
    source_textures: &Query<&SourceMaterialTextures>,
    default_white: &Handle<Image>,
) -> Option<SwapPlan> {
    // First-seen handle → (material name, extras text, root). Entities with the
    // same handle share one resolution; the root is recorded to locate the
    // zero-write surface for the window-outside gate corners.
    let mut seen: HashMap<Handle<StandardMaterial>, (String, String, Entity, Entity)> = HashMap::new();
    // Handle → [(root, mesh entity)]: the mesh position of the zero-write
    // surface is read from here.
    let mut handle_entities: HashMap<Handle<StandardMaterial>, Vec<(Entity, Entity)>> =
        HashMap::new();
    let mut tally = SwapTally::default();
    let mut planned = Vec::new();
    let mut stencil_only = Vec::new();
    let mut shadow_mesh = Vec::new();
    for (root_entity, _) in roots.iter() {
        let mut stack = vec![root_entity];
        while let Some(entity) = stack.pop() {
            if let Ok(kids) = children.get(entity) {
                stack.extend(kids.iter().map(|child| child));
            }
            let Ok((material, name, extras)) = parts.get(entity) else {
                continue;
            };
            seen.entry(material.0.clone()).or_insert_with(|| {
                (name.0.clone(), extras.value.clone(), root_entity, entity)
            });
            handle_entities
                .entry(material.0.clone())
                .or_default()
                .push((root_entity, entity));
        }
    }
    // First pass: classify the extras handle by handle. Zero-write surfaces /
    // ShadowMesh / out of scope / refusals are recorded here; Basic is held
    // pending and resolved once the zero-write surface corners are collected.
    struct PendingBasic {
        name: String,
        extras: serde_json::Value,
        root: Entity,
        entity: Entity,
        source: Handle<StandardMaterial>,
    }
    let mut pending_basic: Vec<PendingBasic> = Vec::new();
    for (handle, (name, extras_json, root, entity)) in &seen {
        let extras: serde_json::Value = match serde_json::from_str(extras_json) {
            Ok(value) => value,
            Err(err) => {
                tally
                    .refused
                    .push(format!("家具材质 {name} 的 extras 不是合法 JSON：{err}"));
                continue;
            }
        };
        // Render state "writes nothing": checked before the family gate — the
        // render state is driven by material floats and is unrelated to the
        // shader name; a zero-write material of a refused family also draws no
        // pixel, and keeping its original material would only draw an invisible
        // mesh as a white surface.
        if is_stencil_only(&extras) {
            tally.stencil_hidden_names.push(name.clone());
            let attribute = extras
                .get("shader")
                .and_then(|value| value.as_str())
                .and_then(|shader| source_attribute(shader, &extras["floats"]));
            stencil_only.push((handle.clone(), attribute));
            continue;
        }
        // Family gate: the named refusal (URP/Lit) and unknown names are refused;
        // names inside the closed set but not Basic take the out-of-scope path.
        // ShadowMesh has its own group: always 0 pixels in the game, and keeping
        // the original material would draw a wrong solid surface, so it goes into
        // the hidden set.
        let shader = match extras.get("shader").and_then(|value| value.as_str()) {
            Some(shader) => shader,
            None => {
                tally
                    .refused
                    .push(format!("家具材质 {name} 的 extras 缺 shader 名"));
                continue;
            }
        };
        if let Err(reason) = check_family_shader(shader) {
            tally.refused.push(reason);
            continue;
        }
        if shader == SHADOWMESH_SHADER {
            tally.shadow_mesh_names.push(name.clone());
            shadow_mesh.push(handle.clone());
            continue;
        }
        // Road owns a separate runtime material and neighborhood path.
        if shader == "Mysekai/Fixture/Road" { continue; }
        if surfaces::owns_shader(shader) { continue; }
        if shader != BASIC_SHADER && shader != RUG_SHADER && shader != FENCE_SHADER {
            tally.unported.push(format!("{name} ({shader})"));
            continue;
        }
        pending_basic.push(PendingBasic {
            name: name.clone(),
            extras,
            root: *root,
            entity: *entity,
            source: handle.clone(),
        });
    }
    // Zero-write surface corners: collect the unique vertices of every mesh with
    // a zero-write material per root, validate the shape (exactly four points,
    // planar, convex) and order the ring. Any other point count, or several
    // zero-write surfaces that cannot be merged, is recorded as a named Err —
    // a usage=1 material is refused on that Err during resolution, never let
    // through silently.
    let mut stencil_points: HashMap<Entity, Vec<Vec3>> = HashMap::new();
    for (handle, _) in &stencil_only {
        for &(root, entity) in handle_entities.get(handle).into_iter().flatten() {
            let points = stencil_points.entry(root).or_default();
            let Ok(mesh_handle) = mesh_parts.get(entity) else {
                continue;
            };
            let Some(mesh) = meshes.get(&mesh_handle.0) else {
                continue;
            };
            let Some(positions) = mesh
                .attribute(Mesh::ATTRIBUTE_POSITION)
                .and_then(|values| values.as_float3())
            else {
                continue;
            };
            for position in positions {
                let vertex = Vec3::from_slice(position);
                if !points.contains(&vertex) {
                    points.push(vertex);
                }
            }
        }
    }
    let mut clip_quads: HashMap<Entity, Result<[Vec3; 4], String>> = HashMap::new();
    for (root, points) in stencil_points {
        if points.is_empty() {
            clip_quads.insert(root, Err("写零面网格没有可读的顶点属性".into()));
            continue;
        }
        if points.len() != 4 {
            clip_quads.insert(
                root,
                Err(format!(
                    "写零面的唯一顶点 {} 个（门是四边形判定，期望 4）",
                    points.len()
                )),
            );
            continue;
        }
        let quad = order_clip_quad([points[0], points[1], points[2], points[3]]);
        clip_quads.insert(root, quad);
    }
    // Second pass: resolve the Basic materials. The window-outside gate corners
    // are taken from the first root seen.
    for PendingBasic {
        name,
        extras,
        root,
        entity,
        source,
    } in pending_basic
    {
        // Main texture: the glb material's `baseColorTexture` (the extractor
        // writes `_MainTex` into it).
        // Without one, the extras say why: a `_MainTex` key with a null value
        // is a bound texture that is not in this package (refused by name);
        // no key is an empty slot, which the engine fills with the shader's
        // declared default. Only the `white` default is modelled.
        let (main_tex, default_main_tex) = match std_materials
            .get(&source)
            .and_then(|material| material.base_color_texture.clone())
        {
            Some(handle) => (handle, false),
            None => match empty_main_tex_default(&extras) {
                Ok(()) => (default_white.clone(), true),
                Err(reason) => {
                    tally.refused.push(format!("家具材质 {name}：{reason}"));
                    continue;
                }
            },
        };
        let entity_textures = source_textures.get(entity).ok();
        let resolved = if extras.get("shader").and_then(|v| v.as_str()) == Some(FENCE_SHADER) {
            resolve_fence(&name, &extras, main_tex.clone())
        } else if extras.get("shader").and_then(|v| v.as_str()) == Some(RUG_SHADER) {
            resolve_rug(&name, &extras, main_tex.clone())
        } else {
            resolve_basic(&name, &extras, main_tex.clone(), clip_quads.get(&root), entity_textures)
        };
        match resolved {
            Ok(resolved) => {
                // Import retains extras-only textures in the scene. Reuse that
                // handle: loading a dropped TextureN label here reloads the GLB
                // and respawns entities after their materials were installed.
                let mask = match resolved.mask_index {
                    Some(index) => {
                        match source_textures.get(entity).ok()
                            .and_then(|textures| textures.0.get("_EmissionMaskTex")).cloned() {
                            Some(handle) => Some(handle),
                            None => {
                                tally.refused.push(format!(
                                    "家具材质 {name} 的遮罩 Texture{index} 未由 glTF 装载器保留"
                                ));
                                continue;
                            }
                        }
                    }
                    None => source_textures.get(entity).ok()
                        .and_then(|textures| textures.0.get("_EmissionMaskTex")).cloned(),
                };
                planned.push(Planned {
                    force_emission: resolved.force_emission,
                    name: name.clone(),
                    material: resolved.material,
                    source,
                    attribute: resolved.attribute,
                    emission: resolved.emission,
                    receive_shadows_off: resolved.receive_shadows_off,
                    unported_keywords: resolved.unported_keywords,
                    reflection_needs_cubemap: resolved.reflection_needs_cubemap,
                    crystal_blocked: resolved.crystal_blocked,
                    colour_picker: resolved.colour_picker,
                    default_main_tex,
                    mask,
                    main_tex_st: extras
                        .get("textureScaleOffset")
                        .and_then(|st| st.get("_MainTex"))
                        .and_then(|value| value.as_array())
                        .and_then(|array| {
                            let mut st = [0.0; 4];
                            for (slot, value) in st.iter_mut().zip(array) {
                                *slot = value.as_f64()?;
                            }
                            Some(st)
                        }),
                });
            }
            Err(reason) => tally.refused.push(reason),
        }
    }
    Some(SwapPlan {
        planned,
        stencil_only,
        shadow_mesh,
        tally,
    })
}

/// The furniture material plugin: the material pipeline and the swap. The
/// global-value bridge (`SiteEnvGpuBuffer`) is installed by the site material
/// plugin and only consumed here — same buffer, same slot order; change both
/// together.
#[derive(SystemSet, Debug, Hash, PartialEq, Eq, Clone)]
pub struct FixtureMaterialSet;

pub struct FixtureMaterialPlugin;

impl Plugin for FixtureMaterialPlugin {
    fn build(&self, app: &mut App) {
        // The material plugin resolves the shader path while it is added.
        bevy::asset::embedded_asset!(app, "shaders/fixture_material.wesl");
        crate::gpu_image_release::install(app);
        crate::render::gpu::install_shared_samplers(app);
        crate::gpu_image_release::prepare_after_images::<FixtureMaterial>(app);
        app.add_plugins(MaterialPlugin::<FixtureMaterial>::default())
            .add_plugins(surfaces::FixtureSurfacePlugin)
            .add_systems(Update, switch_materials.in_set(FixtureMaterialSet)
                .after(crate::render_inputs::FixtureLayoutSet).after(crate::fixture_colors::prepare)
                .run_if(crate::fixture_colors::ready));
    }
}
