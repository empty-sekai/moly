//! Non-Basic furniture surfaces, sharing source properties and GPU resources.
//! Canvas and transparent blocks have their own programs. Furniture trees use
//! the same Tree program as sites, with the fixture importer's UV/sampler frame.

use std::collections::HashMap;

use bevy::{
    asset::RenderAssetUsages,
    ecs::system::{lifetimeless::SRes, SystemParamItem},
    gltf::{GltfMaterialExtras, GltfMaterialName},
    pbr::{Material, MaterialPipeline, MaterialPipelineKey, MaterialPlugin},
    prelude::*,
    render::{render_asset::RenderAssets, render_resource::*, renderer::RenderDevice, texture::GpuImage},
    shader::ShaderRef,
};
use moly_assets::{
    material_passes::{SourceMaterialPasses, SourceRenderState},
    material_textures::SourceMaterialTextures,
    sidecar::parse_fixture_material,
};
use moly_law::material::{FloatLookup, MaterialSlot};

use crate::{
    env::SiteEnvGpuBuffer,
    fixture::{FixtureVisualRoot, FixtureVisualSceneReady, FixtureVisualReady, FixtureScenesReady},
    site_material::{resolve_tree_textures, SiteMaterial, SiteMaterialKey},
};
use super::{FixtureMaterialSet, WallLayoutShadowCasterOff};

const CANVAS: &str = "Mysekai/Fixture/Canvas";
const BLOCK: &str = "Mysekai/Fixture/TransparentBlock";
const TREE: &str = "Mysekai/Site/Tree";

pub(super) fn owns_shader(shader: &str) -> bool { matches!(shader, CANVAS | BLOCK | TREE) }

#[derive(Resource, Default)]
pub(super) struct FixtureSurfaceReadiness(pub bool);

/// Source BlockFixtureViewManager starts at one and clamps its global to [0,1].
/// This affects face opacity only: edges retain their authored outline alpha.
#[derive(Resource)]
pub struct TransparentBlockAppearance { pub face_opacity: f32 }

impl Default for TransparentBlockAppearance {
    fn default() -> Self { Self { face_opacity: 1.0 } }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum SurfaceKind { Canvas, TransparentBlock }

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct SurfaceKey {
    kind: SurfaceKind,
    state: SourceRenderState,
    dither: bool,
    queue: u32,
}

#[derive(Asset, TypePath, Clone)]
struct FixtureSurfaceMaterial {
    key: SurfaceKey,
    /// Colour1, Colour2, OutlineColor, (scale, edge size, light gate, opacity).
    uniforms: [[f32; 4]; 4],
    main_tex: Handle<Image>,
    overlay_tex: Handle<Image>,
}

impl AsBindGroup for FixtureSurfaceMaterial {
    type Data = SurfaceKey;
    type Param = (SRes<SiteEnvGpuBuffer>, SRes<RenderAssets<GpuImage>>);
    fn label() -> &'static str { "fixture_surface_material" }
    fn bind_group_data(&self) -> Self::Data { self.key }
    fn unprepared_bind_group(
        &self, _layout: &BindGroupLayout, _device: &RenderDevice,
        (env, images): &mut SystemParamItem<'_, '_, Self::Param>, _force: bool,
    ) -> Result<UnpreparedBindGroup, AsBindGroupError> {
        let main = images.get(&self.main_tex).ok_or(AsBindGroupError::RetryNextUpdate)?;
        let overlay = images.get(&self.overlay_tex).ok_or(AsBindGroupError::RetryNextUpdate)?;
        let bytes = self.uniforms.iter().flatten().flat_map(|v| v.to_le_bytes()).collect();
        Ok(UnpreparedBindGroup { bindings: BindingResources(vec![
            (0, OwnedBindingResource::Data(OwnedData(bytes))),
            (1, OwnedBindingResource::Buffer(env.buffer.clone())),
            (2, OwnedBindingResource::TextureView(TextureViewDimension::D2, main.texture_view.clone())),
            (3, OwnedBindingResource::Sampler(SamplerBindingType::Filtering, main.sampler.clone())),
            (4, OwnedBindingResource::TextureView(TextureViewDimension::D2, overlay.texture_view.clone())),
            (5, OwnedBindingResource::Sampler(SamplerBindingType::Filtering, overlay.sampler.clone())),
        ]) })
    }
    fn bind_group_layout_entries(_device: &RenderDevice, _force: bool) -> Vec<BindGroupLayoutEntry> {
        let uniform = |binding| BindGroupLayoutEntry { binding, visibility: ShaderStages::VERTEX_FRAGMENT,
            ty: BindingType::Buffer { ty: BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }, count: None };
        let texture = |binding| BindGroupLayoutEntry { binding, visibility: ShaderStages::FRAGMENT,
            ty: BindingType::Texture { sample_type: TextureSampleType::Float { filterable: true },
                view_dimension: TextureViewDimension::D2, multisampled: false }, count: None };
        let sampler = |binding| BindGroupLayoutEntry { binding, visibility: ShaderStages::FRAGMENT,
            ty: BindingType::Sampler(SamplerBindingType::Filtering), count: None };
        vec![uniform(0), uniform(1), texture(2), sampler(3), texture(4), sampler(5)]
    }
}

fn blending(state: SourceRenderState) -> bool {
    (state.src_color, state.dst_color, state.src_alpha, state.dst_alpha, state.color_op, state.alpha_op)
        != (1, 0, 1, 0, 0, 0)
}

fn factor(value: u8) -> BlendFactor {
    match value {
        0 => BlendFactor::Zero, 1 => BlendFactor::One, 2 => BlendFactor::Dst,
        3 => BlendFactor::Src, 4 => BlendFactor::OneMinusDst, 5 => BlendFactor::SrcAlpha,
        6 => BlendFactor::OneMinusSrc, 7 => BlendFactor::DstAlpha, 8 => BlendFactor::OneMinusDstAlpha,
        9 => BlendFactor::SrcAlphaSaturated, 10 => BlendFactor::OneMinusSrcAlpha,
        _ => unreachable!("source blend factor validated on import"),
    }
}

fn operation(value: u8) -> BlendOperation {
    match value {
        0 => BlendOperation::Add, 1 => BlendOperation::Subtract, 2 => BlendOperation::ReverseSubtract,
        3 => BlendOperation::Min, 4 => BlendOperation::Max,
        _ => unreachable!("source blend operation validated on import"),
    }
}

fn apply_state(descriptor: &mut RenderPipelineDescriptor, state: SourceRenderState) {
    descriptor.primitive.cull_mode = match state.cull { 0 => None, 1 => Some(Face::Front), _ => Some(Face::Back) };
    if let Some(depth) = descriptor.depth_stencil.as_mut() {
        depth.depth_write_enabled = state.depth_write;
        // Bevy's camera depth is reverse-Z. Equality and disabled/always tests
        // stay unchanged; ordered comparisons reverse, not the source values.
        depth.depth_compare = match state.depth_test {
            0 | 8 => CompareFunction::Always, 1 => CompareFunction::Never,
            2 => CompareFunction::Greater, 3 => CompareFunction::Equal,
            4 => CompareFunction::GreaterEqual, 5 => CompareFunction::Less,
            6 => CompareFunction::NotEqual, 7 => CompareFunction::LessEqual,
            _ => unreachable!("source depth comparison validated on import"),
        };
    }
    if let Some(target) = descriptor.fragment.as_mut().and_then(|f| f.targets.first_mut()).and_then(Option::as_mut) {
        target.blend = blending(state).then_some(BlendState {
            color: BlendComponent { src_factor: factor(state.src_color), dst_factor: factor(state.dst_color), operation: operation(state.color_op) },
            alpha: BlendComponent { src_factor: factor(state.src_alpha), dst_factor: factor(state.dst_alpha), operation: operation(state.alpha_op) },
        });
        // Unity ColorWriteMask is A/B/G/R, wgpu ColorWrites is R/G/B/A.
        target.write_mask = ColorWrites::empty();
        for (bit, channel) in [(8, ColorWrites::RED), (4, ColorWrites::GREEN), (2, ColorWrites::BLUE), (1, ColorWrites::ALPHA)] {
            if state.color_mask & bit != 0 { target.write_mask |= channel; }
        }
    }
}

impl Material for FixtureSurfaceMaterial {
    fn vertex_shader() -> ShaderRef { "embedded://moly_game/shaders/fixture_surface.wgsl".into() }
    fn fragment_shader() -> ShaderRef { Self::vertex_shader() }
    fn enable_prepass() -> bool { false }
    fn enable_shadows() -> bool { false }
    fn alpha_mode(&self) -> AlphaMode { if blending(self.key.state) { AlphaMode::Blend } else { AlphaMode::Opaque } }
    fn specialize(_pipeline: &MaterialPipeline, descriptor: &mut RenderPipelineDescriptor,
        _layout: &bevy::mesh::MeshVertexBufferLayoutRef, key: MaterialPipelineKey<Self>) -> Result<(), SpecializedMeshPipelineError> {
        let key = key.bind_group_data;
        crate::material_order::set_queue(descriptor, key.queue);
        apply_state(descriptor, key.state);
        if let Some(fragment) = descriptor.fragment.as_mut() {
            fragment.shader_defs.push(match key.kind { SurfaceKind::Canvas => "SURFACE_CANVAS", SurfaceKind::TransparentBlock => "SURFACE_BLOCK" }.into());
            if key.dither { fragment.shader_defs.push("SURFACE_DITHER".into()); }
        }
        Ok(())
    }
}

#[derive(Asset, TypePath, Clone)]
struct FixtureTreeMaterial { material: SiteMaterial, state: SourceRenderState }

impl AsBindGroup for FixtureTreeMaterial {
    type Data = (SiteMaterialKey, SourceRenderState);
    type Param = <SiteMaterial as AsBindGroup>::Param;
    fn label() -> &'static str { "fixture_tree_material" }
    fn bind_group_data(&self) -> Self::Data { (self.material.key, self.state) }
    fn bind_group_layout_entries(device: &RenderDevice, force: bool) -> Vec<BindGroupLayoutEntry> {
        SiteMaterial::bind_group_layout_entries(device, force)
    }
    fn unprepared_bind_group(&self, layout: &BindGroupLayout, device: &RenderDevice,
        param: &mut SystemParamItem<'_, '_, Self::Param>, force: bool) -> Result<UnpreparedBindGroup, AsBindGroupError> {
        let mut group = self.material.unprepared_bind_group(layout, device, param, force)?;
        // glTF already retained the authored samplers, including Clamp and
        // Mirror. Do not replace those with the site loader's Repeat sampler.
        for (binding, handle) in [(3, Some(&self.material.main_tex)), (9, self.material.leaf_mask_tex.as_ref())] {
            if let Some(handle) = handle {
                let image = param.2.get(handle).ok_or(AsBindGroupError::RetryNextUpdate)?;
                if let Some((_, resource)) = group.bindings.0.iter_mut().find(|(slot, _)| *slot == binding) {
                    *resource = OwnedBindingResource::Sampler(SamplerBindingType::Filtering, image.sampler.clone());
                }
            }
        }
        Ok(group)
    }
}

impl Material for FixtureTreeMaterial {
    fn vertex_shader() -> ShaderRef { SiteMaterial::vertex_shader() }
    fn fragment_shader() -> ShaderRef { SiteMaterial::fragment_shader() }
    fn enable_prepass() -> bool { false }
    fn enable_shadows() -> bool { false }
    fn specialize(pipeline: &MaterialPipeline, descriptor: &mut RenderPipelineDescriptor,
        layout: &bevy::mesh::MeshVertexBufferLayoutRef, key: MaterialPipelineKey<Self>) -> Result<(), SpecializedMeshPipelineError> {
        SiteMaterial::specialize(pipeline, descriptor, layout, MaterialPipelineKey {
            mesh_key: key.mesh_key, bind_group_data: key.bind_group_data.0,
        })?;
        descriptor.vertex.shader_defs.push("FIXTURE_SOURCE_UV".into());
        apply_state(descriptor, key.bind_group_data.1);
        Ok(())
    }
}

#[derive(Clone)]
enum SurfaceHandle { Flat(Handle<FixtureSurfaceMaterial>), Tree(Handle<FixtureTreeMaterial>) }

fn white_image(images: &mut Assets<Image>, white: &mut Option<Handle<Image>>) -> Handle<Image> {
    white.get_or_insert_with(|| images.add(Image::new_fill(
        Extent3d { width: 1, height: 1, depth_or_array_layers: 1 }, TextureDimension::D2,
        &[255, 255, 255, 255], TextureFormat::Rgba8UnormSrgb, RenderAssetUsages::default(),
    ))).clone()
}

fn number(slot: &MaterialSlot, property: &str) -> f32 {
    slot.get(property).unwrap_or_else(|| panic!("家具材质 {} 缺源属性 {property}", slot.name))
}

fn bind_surfaces(
    mut commands: Commands, ready: Option<Res<FixtureScenesReady>>,
    roots: Query<(Entity, &FixtureVisualRoot), (With<FixtureVisualSceneReady>, Without<FixtureVisualReady>)>, children: Query<&Children>,
    parts: Query<(&MeshMaterial3d<StandardMaterial>, &GltfMaterialName, &GltfMaterialExtras,
                 Option<&SourceMaterialTextures>, Option<&SourceMaterialPasses>)>,
    mut materials: ResMut<Assets<FixtureSurfaceMaterial>>, mut trees: ResMut<Assets<FixtureTreeMaterial>>,
    mut images: ResMut<Assets<Image>>, mut white: Local<Option<Handle<Image>>>,
    mut cache: Local<HashMap<AssetId<StandardMaterial>, SurfaceHandle>>,
    mut readiness: ResMut<FixtureSurfaceReadiness>, appearance: Res<TransparentBlockAppearance>,
    revision: Res<crate::fixture::FixtureLayoutRevision>,
    mut seen_revision: Local<u64>,
) {
    if *seen_revision != revision.0 {
        cache.clear();
        readiness.0 = false;
        *seen_revision = revision.0;
    }
    if ready.is_none() { readiness.0 = false; return; }
    for (root, placement) in &roots {
        let mut stack = vec![root];
        while let Some(entity) = stack.pop() {
            if let Ok(kids) = children.get(entity) { stack.extend(kids.iter()); }
            let Ok((source, name, extras, textures, passes)) = parts.get(entity) else { continue; };
            let value: serde_json::Value = serde_json::from_str(&extras.value).expect("fixture material JSON");
            let Some(shader) = value.get("shader").and_then(|v| v.as_str()) else { continue; };
            if !owns_shader(shader) { continue; }
            let handle = cache.entry(source.0.id()).or_insert_with(|| {
                let slot = parse_fixture_material(&name.0, &value).expect("fixture surface properties");
                let state = passes.and_then(SourceMaterialPasses::color_state)
                    .unwrap_or_else(|| panic!("家具材质 {} 缺提取后的 Base pass 渲染状态", name.0));
                let mut texture = |property: &str| -> Handle<Image> {
                    if let Some(handle) = textures.and_then(|v| v.0.get(property)) { return handle.clone(); }
                    let default = value.get("shaderTextureDefaults").and_then(|v| v.get(property)).and_then(|v| v.as_str());
                    match default {
                        Some("white") => white_image(&mut images, &mut white),
                        _ => panic!("家具材质 {} 的 {property} 无已解析纹理或已支持的源默认值 ({default:?})", name.0),
                    }
                };
                let result = if shader == TREE {
                    let main = texture("_MainTex");
                    let leaf = slot.keywords.iter().any(|k| k == "_USE_TREE_ANIMATION").then(|| texture("_LeafMaskTex"));
                    let material = resolve_tree_textures(&slot, main, leaf)
                        .unwrap_or_else(|reason| panic!("{reason}"));
                    SurfaceHandle::Tree(trees.add(FixtureTreeMaterial { material, state }))
                } else {
                    let mut uniforms = [[0.0; 4]; 4];
                    let (kind, main_tex, overlay_tex, dither, base_queue) = if shader == CANVAS {
                        uniforms[3] = [0.0, 0.0, number(&slot, "_UsePhenomenaLighting"), number(&slot, "_DitherAlpha")];
                        // Canvas Base has an _ENABLE_DITHER axis, not Basic's
                        // inverse _DISABLE_DITHER. _USE_ALPHA_CLIP is shadow-only.
                        let dither = slot.keywords.iter().any(|k| k == "_ENABLE_DITHER");
                        (SurfaceKind::Canvas, texture("_MainTex"), texture("_OverlayColorMap"), dither, 2065)
                    } else {
                        for (i, property) in ["_Color1", "_Color2", "_OutlineColor"].into_iter().enumerate() {
                            uniforms[i] = *slot.colors.get(property).unwrap_or_else(|| panic!("{} 缺 {property}", name.0));
                        }
                        uniforms[3] = [number(&slot, "_Scale"), number(&slot, "_EdgeSize"), 0.0, appearance.face_opacity.clamp(0.0, 1.0)];
                        let white = white_image(&mut images, &mut white);
                        (SurfaceKind::TransparentBlock, white.clone(), white, false, 3020)
                    };
                    let queue = (base_queue + slot.get("_RenderPriority").unwrap_or(0.0) as i32) as u32;
                    SurfaceHandle::Flat(materials.add(FixtureSurfaceMaterial {
                        key: SurfaceKey { kind, state, dither, queue }, uniforms, main_tex, overlay_tex,
                    }))
                };
                info!("家具材质接入：{} ({shader})，源 pass 状态 {state:?}", name.0);
                result
            }).clone();
            let mut target = commands.entity(entity);
            target.remove::<MeshMaterial3d<StandardMaterial>>();
            match handle {
                SurfaceHandle::Flat(material) => { target.insert(MeshMaterial3d(material)); }
                SurfaceHandle::Tree(material) => { target.insert(MeshMaterial3d(material)); }
            }
            if placement.is_wall_layout() { target.insert(WallLayoutShadowCasterOff); }
        }
    }
    readiness.0 = true;
}

fn update_block_opacity(appearance: Res<TransparentBlockAppearance>, mut materials: ResMut<Assets<FixtureSurfaceMaterial>>) {
    if !appearance.is_changed() { return; }
    let opacity = appearance.face_opacity.clamp(0.0, 1.0);
    for (_, material) in materials.iter_mut() {
        if material.key.kind == SurfaceKind::TransparentBlock { material.uniforms[3][3] = opacity; }
    }
}

pub(super) struct FixtureSurfacePlugin;

impl Plugin for FixtureSurfacePlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins((MaterialPlugin::<FixtureSurfaceMaterial>::default(), MaterialPlugin::<FixtureTreeMaterial>::default()))
            .init_resource::<FixtureSurfaceReadiness>()
            .init_resource::<TransparentBlockAppearance>()
            .add_systems(Update, (bind_surfaces, update_block_opacity).chain()
                .after(crate::fixture::FixtureLayoutSet).before(FixtureMaterialSet));
        bevy::asset::embedded_asset!(app, "shaders/fixture_surface.wgsl");
    }
}
