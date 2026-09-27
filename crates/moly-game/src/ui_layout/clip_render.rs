//! RectMask2D and stencil Mask consumer for the existing prefab renderer's
//! images and glyphs. No atlas is created: clipped text uses the same
//! BalloonArt image pages.

use super::NodeCache;
use bevy::{
    asset::{RenderAssetUsages, uuid::Uuid},
    camera::visibility::RenderLayers,
    mesh::{Indices, PrimitiveTopology},
    prelude::*,
    render::render_resource::{AsBindGroup, ShaderType},
    shader::ShaderRef,
    sprite::TextureSlice,
    sprite_render::{AlphaMode2d, Material2d, Material2dPlugin},
};
use moly_assets::ui_layout::{
    UiComponent, UiRect,
    clipping::{UiClipRect, maskable},
};
use std::marker::PhantomData;
use super::stencil_mask::MAX_STENCIL_QUADS;

const SHADER: Handle<Shader> = Handle::Uuid(
    Uuid::from_u128(0x7569_7265_6374_636c_6970_0000_0000_0001),
    PhantomData,
);

#[derive(Debug, Clone, Copy, ShaderType)]
struct ClipUniform {
    color: Vec4,
    rect: Vec4,
    // xy: softness; zw: source projection's half-pixel term in canvas units.
    softness_pixel: Vec4,
    // texture, hard-clipping, FillColor's alpha-only texture read, stencil
    // test (the masking graphic's quad count).
    flags: UVec4,
    // x: the masking graphic's vertex alpha; y: 1 when it samples its texture.
    stencil_alpha: Vec4,
    // The masking graphic's quads in canvas units (min xy, max xy).
    stencil_quads: [Vec4; MAX_STENCIL_QUADS],
    // Per quad, canvas point to the masking graphic's uv: xy scale, zw offset.
    stencil_uvs: [Vec4; MAX_STENCIL_QUADS],
    // The masking graphic's own RectMask2D rectangle (min xy, max xy).
    stencil_rect: Vec4,
    // xy: that rectangle's softness term (the vertex stage's mask.zw for the
    // masking graphic's draw); z: 0 no rectangle, 1 soft, 2 hard.
    stencil_clip: Vec4,
}

#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
pub(crate) struct UiClipMaterial {
    #[uniform(0)]
    value: ClipUniform,
    #[texture(1)]
    #[sampler(2)]
    texture: Option<Handle<Image>>,
    #[texture(3)]
    #[sampler(4)]
    stencil_texture: Option<Handle<Image>>,
}

impl Material2d for UiClipMaterial {
    fn vertex_shader() -> ShaderRef {
        ShaderRef::Handle(SHADER.clone())
    }
    fn fragment_shader() -> ShaderRef {
        ShaderRef::Handle(SHADER.clone())
    }
    fn alpha_mode(&self) -> AlphaMode2d {
        AlphaMode2d::Blend
    }
}

impl UiClipMaterial {
    pub(super) fn set_color(&mut self, color: Color) {
        self.value.color = linear(color);
    }
}

pub(super) fn install(app: &mut App) {
    app.add_plugins(Material2dPlugin::<UiClipMaterial>::default());
    app.world_mut()
        .resource_mut::<Assets<Shader>>()
        .insert(
            SHADER.id(),
            Shader::from_wgsl(include_str!("clip.wgsl"), "moly_game/ui_layout/clip.wgsl"),
        )
        .expect("UI clip shader installation");
}

#[derive(Clone)]
pub(super) struct Clipped {
    rect: UiClipRect,
    softness: Vec2,
    pixel_scale: Vec2,
    hard: bool,
    fill_color: bool,
    stencil: Option<Stencil>,
}

/// The stencil a maskable Graphic is tested against: the masking graphic of
/// its enclosing `Mask` (one level). The masking graphic writes the stencil
/// where its own fragment survives the alpha clip `StencilMaterial` turns on
/// for it (`color.a - 0.001`, color = texture sample x vertex colour, times
/// its own RectMask2D factor when it has one), inside the quad it draws; a
/// Graphic under it draws only there.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Stencil {
    /// The quads the masking graphic draws: the canvas rectangle (min xy,
    /// max xy) and the uv map of a canvas point `p` (`p * xy + zw`). Empty
    /// when it draws nothing (a Filled amount below 0.001), so nothing under
    /// it is drawn.
    pub(super) quads: Vec<(Vec4, Vec4)>,
    /// The masking graphic's vertex alpha (its Color32 alpha times the
    /// inherited group alpha).
    pub(super) vertex_alpha: f32,
    /// The masking graphic's texture; `None` when it has no sprite (the white
    /// texture, alpha 1).
    pub(super) texture: Option<Handle<Image>>,
    /// The masking graphic's own RectMask2D clip; `None` outside every
    /// RectMask2D (or when it is not maskable).
    pub(super) clip: Option<StencilClip>,
}

/// The RectMask2D clip of a masking graphic. `MaskableGraphic.UpdateClipParent`
/// registers every maskable active Graphic with its RectMask2D, the masking
/// graphic included, so its stencil-writing draw carries the clip rectangle
/// (UI/Default `UNITY_UI_CLIP_RECT`: alpha times the softness factor) before
/// the alpha clip decides whether the stencil is written.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct StencilClip {
    rect: Vec4,
    softness: Vec2,
    pixel_scale: Vec2,
    hard: bool,
}

/// What a masking graphic's RectMask2D does to the stencil it writes.
pub(super) enum MaskingClip {
    /// Outside every RectMask2D, or not maskable (`UpdateClipParent` gives a
    /// non-maskable Graphic no clip parent).
    None,
    /// `MaskableGraphic.Cull`: an invalid compound rectangle, or one that
    /// does not overlap the masking graphic, culls its CanvasRenderer, so it
    /// writes no stencil and nothing under it is drawn.
    Culled,
    Rect(StencilClip),
}

pub(super) fn masking_clip(graphic: &UiComponent, rect: &UiRect) -> MaskingClip {
    match for_component(graphic, rect) {
        None => MaskingClip::None,
        Some(clip) if !clip.rect.valid => MaskingClip::Culled,
        Some(clip) => MaskingClip::Rect(StencilClip {
            rect: Vec4::new(
                clip.rect.min.x,
                clip.rect.min.y,
                clip.rect.max.x,
                clip.rect.max.y,
            ),
            softness: clip.softness,
            pixel_scale: clip.pixel_scale,
            hard: clip.hard,
        }),
    }
}

/// A rectangle test that never fails, for a stencil-tested Graphic outside
/// every RectMask2D (the vertex stage clamps the rectangle to 2e10).
const UNBOUNDED: f32 = 1e10;

pub(super) fn for_component(component: &UiComponent, rect: &UiRect) -> Option<Clipped> {
    if !maskable(component) {
        return None;
    }
    let mut clip = rect.clipping.rect?;
    // MaskableGraphic.Cull compares the compound rect with the min/max of all
    // four Graphic corners. It does not hide the GameObject or its children.
    let local_min = -rect.size * rect.pivot;
    let mut lower = Vec2::splat(f32::INFINITY);
    let mut upper = Vec2::splat(f32::NEG_INFINITY);
    for corner in [
        local_min,
        local_min + Vec2::new(rect.size.x, 0.),
        local_min + rect.size,
        local_min + Vec2::new(0., rect.size.y),
    ] {
        let point = rect.world.transform_point3(corner.extend(0.)).truncate();
        lower = lower.min(point);
        upper = upper.max(point);
    }
    clip.valid &= clip.max.cmpgt(lower).all() && upper.cmpgt(clip.min).all();
    Some(profile(component, clip))
}

/// A Graphic's clip under a stencil Mask as well as its RectMask2D. A Graphic
/// that is not maskable has stencil value 0 and is not tested
/// (`MaskableGraphic.GetModifiedMaterial`). Outside every RectMask2D the
/// rectangle test is a hard, unbounded one, so only the stencil decides.
pub(super) fn with_stencil(component: &UiComponent, rect: &UiRect, stencil: Option<&Stencil>) -> Option<Clipped> {
    let clip = for_component(component, rect);
    let Some(stencil) = stencil.filter(|_| maskable(component)) else {
        return clip;
    };
    let mut clip = clip.unwrap_or_else(|| {
        let unbounded = UiClipRect {
            min: Vec2::splat(-UNBOUNDED),
            max: Vec2::splat(UNBOUNDED),
            softness: Vec2::ZERO,
            valid: true,
        };
        let mut clip = profile(component, unbounded);
        clip.hard = true;
        clip
    });
    clip.rect.valid &= !stencil.quads.is_empty();
    clip.stencil = Some(stencil.clone());
    Some(clip)
}

/// The shader of the Graphic's material: its exported clip material's, or
/// UI/Default for an Image on the default material.
pub(super) fn shader(component: &UiComponent) -> Option<&str> {
    let default_image = component.fields.get("m_fontSize").is_none()
        && component.fields["m_Material"]
            .as_array()
            .is_some_and(|p| p.len() == 2 && p[1].as_i64() == Some(0));
    component
        .clip_material
        .as_ref()
        .map(|m| m.shader.as_str())
        .or(default_image.then_some("UI/Default"))
}

/// The clip parameters of the Graphic's material family for a rectangle.
fn profile(component: &UiComponent, clip: UiClipRect) -> Clipped {
    let shader = shader(component).unwrap_or_else(|| {
        panic!(
            "UI clip material source missing: {} @{}",
            component.class, component.path_id
        )
    });
    let mut result = Clipped {
        rect: clip,
        softness: clip.softness,
        pixel_scale: Vec2::ONE,
        hard: false,
        fill_color: false,
        stencil: None,
    };
    match shader {
        "UI/Default" | "Sekai/UI/UIGradient" => {}
        "Sekai/UI/FillColor" => {
            result.hard = true;
            result.fill_color = true;
        }
        "TextMeshPro/Distance Field" | "TextMeshPro/Mobile/Distance Field" => {
            let profile = component
                .clip_material
                .as_ref()
                .expect("TMP clip material profile");
            result.softness = Vec2::from_array(profile.softness.expect("TMP _MaskSoftness source"));
            result.pixel_scale = Vec2::from_array(profile.scale.expect("TMP _ScaleXY source"));
            assert!(
                result.softness.is_finite()
                    && result.pixel_scale.is_finite()
                    && result.pixel_scale.min_element() > 0.,
                "invalid TMP clip parameters"
            );
        }
        other => panic!(
            "UI clip shader not implemented: {other} @{}",
            component.path_id
        ),
    }
    result
}

pub(super) struct Draw<'a, 'w, 's> {
    pub commands: &'a mut Commands<'w, 's>,
    pub images: &'a Assets<Image>,
    pub meshes: &'a mut Assets<Mesh>,
    pub materials: &'a mut Assets<UiClipMaterial>,
    pub pixel_size: Vec2,
    pub layer: usize,
}

impl Draw<'_, '_, '_> {
    pub fn mesh(
        &mut self,
        parent: Entity,
        node: &mut NodeCache,
        mesh: Mesh,
        mut transform: Transform,
        color: Color,
        texture: Option<Handle<Image>>,
        clip: Clipped,
        alpha: f32,
    ) {
        if !clip.rect.valid {
            return;
        }
        let stencil = clip.stencil.as_ref();
        let mut stencil_quads = [Vec4::ZERO; MAX_STENCIL_QUADS];
        let mut stencil_uvs = [Vec4::ZERO; MAX_STENCIL_QUADS];
        for (i, (quad, uv)) in stencil.map_or(&[][..], |s| &s.quads[..]).iter().enumerate() {
            stencil_quads[i] = *quad;
            stencil_uvs[i] = *uv;
        }
        // The masking graphic's draw has the same canvas and projection as
        // this one, so its UI/Default vertex term uses the same pixel size.
        let (stencil_rect, stencil_clip) = match stencil.and_then(|s| s.clip) {
            None => (Vec4::ZERO, Vec4::ZERO),
            Some(c) => {
                let pixel = self.pixel_size / c.pixel_scale;
                let term = Vec2::splat(0.25) / (Vec2::splat(0.25) * c.softness + pixel.abs());
                (
                    c.rect,
                    Vec4::new(term.x, term.y, if c.hard { 2. } else { 1. }, 0.),
                )
            }
        };
        let material = self.materials.add(UiClipMaterial {
            value: ClipUniform {
                color: linear(color.with_alpha(color.alpha() * alpha)),
                rect: Vec4::new(
                    clip.rect.min.x,
                    clip.rect.min.y,
                    clip.rect.max.x,
                    clip.rect.max.y,
                ),
                softness_pixel: Vec4::new(
                    clip.softness.x,
                    clip.softness.y,
                    self.pixel_size.x / clip.pixel_scale.x,
                    self.pixel_size.y / clip.pixel_scale.y,
                ),
                flags: UVec4::new(
                    u32::from(texture.is_some()),
                    u32::from(clip.hard),
                    u32::from(clip.fill_color),
                    stencil.map_or(0, |s| s.quads.len() as u32),
                ),
                stencil_alpha: stencil.map_or(Vec4::ZERO, |s| {
                    Vec4::new(s.vertex_alpha, f32::from(u8::from(s.texture.is_some())), 0., 0.)
                }),
                stencil_quads,
                stencil_uvs,
                stencil_rect,
                stencil_clip,
            },
            texture,
            stencil_texture: stencil.and_then(|s| s.texture.clone()),
        });
        // All mesh vertices live in this prefab's existing canvas coordinates.
        // The view's normal parent transform/camera still positions the draw.
        // Transparent2d sorts by the entity transform, not baked vertex depth.
        // Keep the existing prefab painter depth on that transform.
        let depth = transform.translation.z;
        transform.translation.z = 0.;
        let mesh = self.meshes.add(mesh.transformed_by(transform));
        node.meshes.push(mesh.clone());
        node.clip_materials.push(material.clone());
        node.clip_material_colors.push(color);
        let entity = self
            .commands
            .spawn((
                Mesh2d(mesh),
                MeshMaterial2d(material),
                Transform::from_xyz(0., 0., depth),
                RenderLayers::layer(self.layer),
            ))
            .id();
        self.commands.entity(parent).add_child(entity);
    }

    pub fn sprite(
        &mut self,
        parent: Entity,
        node: &mut NodeCache,
        sprite: &Sprite,
        transform: Transform,
        clip: Clipped,
        alpha: f32,
    ) {
        if !clip.rect.valid {
            return;
        }
        assert!(
            sprite.texture_atlas.is_none(),
            "prefab sprites use exact crop rects, not Bevy atlas indices"
        );
        let size = self
            .images
            .get(&sprite.image)
            .expect("UI clip texture loaded")
            .size()
            .as_vec2();
        let rect = sprite
            .rect
            .unwrap_or_else(|| Rect::from_corners(Vec2::ZERO, size));
        let draw_size = sprite.custom_size.unwrap_or_else(|| rect.size());
        let slices = match &sprite.image_mode {
            SpriteImageMode::Auto => vec![TextureSlice {
                texture_rect: rect,
                draw_size,
                offset: Vec2::ZERO,
            }],
            SpriteImageMode::Sliced(slicer) => slicer.compute_slices(rect, Some(draw_size)),
            SpriteImageMode::Tiled {
                tile_x,
                tile_y,
                stretch_value,
            } => TextureSlice {
                texture_rect: rect,
                draw_size,
                offset: Vec2::ZERO,
            }
            .tiled(*stretch_value, (*tile_x, *tile_y)),
            SpriteImageMode::Scale(_) => {
                panic!("prefab clip SpriteScalingMode requires its existing UV layout")
            }
        };
        let mut mesh = Quads::default();
        for slice in slices {
            mesh.add(
                slice.draw_size,
                slice.texture_rect,
                size,
                Transform::from_translation(slice.offset.extend(0.)),
                Color::WHITE,
                sprite.flip_x,
                sprite.flip_y,
            );
        }
        self.mesh(
            parent,
            node,
            mesh.finish(),
            transform,
            sprite.color,
            Some(sprite.image.clone()),
            clip,
            alpha,
        );
    }

    pub fn glyphs(
        &mut self,
        parent: Entity,
        node: &mut NodeCache,
        glyphs: Vec<Glyph>,
        clip: Clipped,
        alpha: f32,
    ) {
        if !clip.rect.valid {
            return;
        }
        let mut page: Option<Handle<Image>> = None;
        let mut quads = Quads::default();
        let mut depth = 0.;
        for mut glyph in glyphs {
            if page.as_ref().is_some_and(|image| {
                image != &glyph.image || depth != glyph.transform.translation.z
            }) {
                self.mesh(
                    parent,
                    node,
                    std::mem::take(&mut quads).finish(),
                    Transform::from_xyz(0., 0., depth),
                    Color::WHITE,
                    page.take(),
                    clip.clone(),
                    alpha,
                );
            }
            page = Some(glyph.image.clone());
            depth = glyph.transform.translation.z;
            glyph.transform.translation.z = 0.;
            let image_size = self
                .images
                .get(&glyph.image)
                .expect("UI glyph atlas page loaded")
                .size()
                .as_vec2();
            quads.add(
                glyph.size,
                glyph.rect,
                image_size,
                glyph.transform,
                glyph.color,
                false,
                false,
            );
        }
        if page.is_some() {
            self.mesh(
                parent,
                node,
                quads.finish(),
                Transform::from_xyz(0., 0., depth),
                Color::WHITE,
                page,
                clip,
                alpha,
            );
        }
    }
}

pub(super) struct Glyph {
    pub image: Handle<Image>,
    pub rect: Rect,
    pub size: Vec2,
    pub transform: Transform,
    pub color: Color,
}

/// Contiguous texture runs are batched without changing glyph paint order.
/// Existing atlas pages and measured TMP pens are reused, never re-baked.
#[derive(Default)]
struct Quads {
    positions: Vec<[f32; 3]>,
    uv: Vec<[f32; 2]>,
    colors: Vec<[f32; 4]>,
    indices: Vec<u32>,
}

impl Quads {
    fn add(
        &mut self,
        size: Vec2,
        rect: Rect,
        image_size: Vec2,
        transform: Transform,
        color: Color,
        flip_x: bool,
        flip_y: bool,
    ) {
        let base = self.positions.len() as u32;
        let half = size * 0.5;
        let lo = rect.min / image_size;
        let hi = rect.max / image_size;
        let x = if flip_x { [hi.x, lo.x] } else { [lo.x, hi.x] };
        let y = if flip_y { [lo.y, hi.y] } else { [hi.y, lo.y] };
        let matrix = transform.to_matrix();
        for point in [
            Vec2::new(-half.x, -half.y),
            Vec2::new(half.x, -half.y),
            Vec2::new(half.x, half.y),
            Vec2::new(-half.x, half.y),
        ] {
            self.positions
                .push(matrix.transform_point3(point.extend(0.)).to_array());
            self.colors.push(linear(color).to_array());
        }
        self.uv
            .extend([[x[0], y[0]], [x[1], y[0]], [x[1], y[1]], [x[0], y[1]]]);
        self.indices
            .extend([base, base + 1, base + 2, base, base + 2, base + 3]);
    }

    fn finish(self) -> Mesh {
        let normals = vec![[0., 0., 1.]; self.positions.len()];
        Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::RENDER_WORLD,
        )
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, self.positions)
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, self.uv)
        .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, self.colors)
        .with_inserted_indices(Indices::U32(self.indices))
    }
}

fn linear(color: Color) -> Vec4 {
    let c = color.to_linear();
    Vec4::new(c.red, c.green, c.blue, c.alpha)
}
