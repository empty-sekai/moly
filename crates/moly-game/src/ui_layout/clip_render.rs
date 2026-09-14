//! RectMask2D consumer for the existing prefab renderer's images and glyphs.
//! No atlas is created: clipped text uses the same BalloonArt image pages.

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
    // texture, hard-clipping, FillColor's alpha-only texture read, reserved.
    flags: UVec4,
}

#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
pub(crate) struct UiClipMaterial {
    #[uniform(0)]
    value: ClipUniform,
    #[texture(1)]
    #[sampler(2)]
    texture: Option<Handle<Image>>,
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

#[derive(Clone, Copy)]
pub(super) struct Clipped {
    rect: UiClipRect,
    softness: Vec2,
    pixel_scale: Vec2,
    hard: bool,
    fill_color: bool,
}

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
    let default_image = component.fields.get("m_fontSize").is_none()
        && component.fields["m_Material"]
            .as_array()
            .is_some_and(|p| p.len() == 2 && p[1].as_i64() == Some(0));
    let shader = component
        .clip_material
        .as_ref()
        .map(|m| m.shader.as_str())
        .or(default_image.then_some("UI/Default"))
        .unwrap_or_else(|| {
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
    Some(result)
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
                    0,
                ),
            },
            texture,
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
                    clip,
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
