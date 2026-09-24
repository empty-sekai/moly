//! `Graphic` default quad and `Image` Simple / Sliced mesh generation.
//!
//! The output is the vertex helper stream in the order the engine fills it:
//! each `AddVert` appends one vertex (position, Color32 colour, uv0) and each
//! `AddTriangle` appends three indices. Quads are added lower-left,
//! upper-left, upper-right, lower-right with triangles (0,1,2) and (2,3,0).
//!
//! Sprite UV and padding values come from the native sprite accessors
//! (`Sprites.DataUtility`). Their arithmetic is transcribed from the engine's
//! arm64 player: the outer UV is the texture rect times the texel size scaled
//! by the render data's downscale multiplier; the inner UV insets it by the
//! border minus the tight-packing offset; the padding is the offset on the
//! left/bottom and the remainder of the sprite rect on the right/top. The
//! texel size itself is taken as one over the texture dimension.

use super::unity_math;

/// A `Rect` (x, y = min corner; width, height).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl Rect {
    /// A rect transform's local rect: min corner at `-pivot * size`.
    pub fn from_size_pivot(size: [f32; 2], pivot: [f32; 2]) -> Self {
        Self { x: -pivot[0] * size[0], y: -pivot[1] * size[1], width: size[0], height: size[1] }
    }

    fn size(&self, axis: usize) -> f32 {
        if axis == 0 {
            self.width
        } else {
            self.height
        }
    }
}

/// One vertex as the UI vertex helper stores it (normal and tangent are the
/// helper's defaults and do not vary in these meshes).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UiVertex {
    pub position: [f32; 3],
    pub color: [u8; 4],
    pub uv0: [f32; 2],
}

/// The vertex helper's vertex and index streams.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct VertexStream {
    pub vertices: Vec<UiVertex>,
    pub indices: Vec<u32>,
}

impl VertexStream {
    fn add_vert(&mut self, position: [f32; 3], color: [u8; 4], uv0: [f32; 2]) {
        self.vertices.push(UiVertex { position, color, uv0 });
    }

    fn add_triangle(&mut self, a: u32, b: u32, c: u32) {
        self.indices.extend_from_slice(&[a, b, c]);
    }

    /// `Image.AddQuad` (the Vector2 posMin/posMax, uvMin/uvMax overload).
    fn add_quad(
        &mut self,
        pos_min: [f32; 2],
        pos_max: [f32; 2],
        color: [u8; 4],
        uv_min: [f32; 2],
        uv_max: [f32; 2],
    ) {
        let start = self.vertices.len() as u32;
        self.add_vert([pos_min[0], pos_min[1], 0.0], color, [uv_min[0], uv_min[1]]);
        self.add_vert([pos_min[0], pos_max[1], 0.0], color, [uv_min[0], uv_max[1]]);
        self.add_vert([pos_max[0], pos_max[1], 0.0], color, [uv_max[0], uv_max[1]]);
        self.add_vert([pos_max[0], pos_min[1], 0.0], color, [uv_max[0], uv_min[1]]);
        self.add_triangle(start, start + 1, start + 2);
        self.add_triangle(start + 2, start + 3, start);
    }
}

/// The sprite fields the Image code and the native sprite accessors read.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpriteData {
    /// `Sprite.rect` width and height, in pixels.
    pub rect_size: [f32; 2],
    /// `Sprite.border` (left, bottom, right, top), in pixels.
    pub border: [f32; 4],
    /// `Sprite.pixelsPerUnit`.
    pub pixels_per_unit: f32,
    /// The texture rect inside the (atlas) texture: x, y, width, height.
    pub texture_rect: [f32; 4],
    /// The tight-packing offset of the texture rect inside the sprite rect.
    pub texture_rect_offset: [f32; 2],
    /// Render data downscale multiplier.
    pub downscale_multiplier: f32,
    /// Texture width and height; None when the sprite has no texture.
    pub texture_size: Option<[f32; 2]>,
}

impl SpriteData {
    fn texel_scale(&self) -> Option<[f32; 2]> {
        self.texture_size.map(|size| {
            [
                (1.0 / size[0]) * self.downscale_multiplier,
                (1.0 / size[1]) * self.downscale_multiplier,
            ]
        })
    }

    /// `Sprites.DataUtility.GetPadding`.
    pub fn padding(&self) -> [f32; 4] {
        let off = self.texture_rect_offset;
        let tr = self.texture_rect;
        [
            off[0],
            off[1],
            self.rect_size[0] - (off[0] + tr[2]),
            self.rect_size[1] - (off[1] + tr[3]),
        ]
    }

    /// `Sprites.DataUtility.GetOuterUV`.
    pub fn outer_uv(&self) -> [f32; 4] {
        let Some(t) = self.texel_scale() else {
            return [0.0, 0.0, 1.0, 1.0];
        };
        let tr = self.texture_rect;
        [tr[0] * t[0], t[1] * tr[1], t[0] * (tr[0] + tr[2]), t[1] * (tr[1] + tr[3])]
    }

    /// `Sprites.DataUtility.GetInnerUV`.
    pub fn inner_uv(&self) -> [f32; 4] {
        let Some(t) = self.texel_scale() else {
            return [0.0, 0.0, 1.0, 1.0];
        };
        let tr = self.texture_rect;
        let off = self.texture_rect_offset;
        let b = self.border;
        let right_top_padding = [
            self.rect_size[0] - (off[0] + tr[2]),
            self.rect_size[1] - (off[1] + tr[3]),
        ];
        [
            t[0] * ((tr[0] + b[0]) - off[0]),
            t[1] * ((tr[1] + b[1]) - off[1]),
            t[0] * ((tr[0] + tr[2]) - (b[2] - right_top_padding[0])),
            t[1] * ((tr[1] + tr[3]) - (b[3] - right_top_padding[1])),
        ]
    }

    /// `Image.hasBorder`: the border's squared magnitude is positive.
    pub fn has_border(&self) -> bool {
        let b = self.border;
        b[0] * b[0] + b[1] * b[1] + b[2] * b[2] + b[3] * b[3] > 0.0
    }
}

/// `Image.pixelsPerUnit`: sprite pixels per unit (100 without a sprite)
/// over the canvas reference pixels per unit (100 without a canvas).
pub fn pixels_per_unit(sprite_pixels_per_unit: Option<f32>, reference_pixels_per_unit: Option<f32>) -> f32 {
    sprite_pixels_per_unit.unwrap_or(100.0) / reference_pixels_per_unit.unwrap_or(100.0)
}

/// `Image.multipliedPixelsPerUnit`.
pub fn multiplied_pixels_per_unit(pixels_per_unit: f32, multiplier: f32) -> f32 {
    pixels_per_unit * multiplier
}

/// `Graphic.GetPixelAdjustedRect` when the effective canvas is not pixel
/// perfect (or is world space, or has scale factor 0): the rect itself. The
/// pixel-perfect branch is native rounding that is not ported, so it yields
/// None instead of a guessed rect.
pub fn pixel_adjusted_rect(rect: Rect, pixel_perfect: bool) -> Option<Rect> {
    (!pixel_perfect).then_some(rect)
}

/// `Graphic.OnPopulateMesh(VertexHelper)`: the rect as one quad with UVs
/// (0,0), (0,1), (1,1), (1,0).
pub fn graphic_quad(rect: Rect, color: [f32; 4]) -> VertexStream {
    let v = [rect.x, rect.y, rect.x + rect.width, rect.y + rect.height];
    let color32 = unity_math::color32(color);
    let mut vh = VertexStream::default();
    vh.add_vert([v[0], v[1], 0.0], color32, [0.0, 0.0]);
    vh.add_vert([v[0], v[3], 0.0], color32, [0.0, 1.0]);
    vh.add_vert([v[2], v[3], 0.0], color32, [1.0, 1.0]);
    vh.add_vert([v[2], v[1], 0.0], color32, [1.0, 0.0]);
    vh.add_triangle(0, 1, 2);
    vh.add_triangle(2, 3, 0);
    vh
}

/// `Image.PreserveSpriteAspectRatio`.
pub fn preserve_sprite_aspect_ratio(rect: &mut Rect, sprite_size: [f32; 2], pivot: [f32; 2]) {
    let sprite_ratio = sprite_size[0] / sprite_size[1];
    let rect_ratio = rect.width / rect.height;
    if sprite_ratio > rect_ratio {
        let old_height = rect.height;
        rect.height = rect.width * (1.0 / sprite_ratio);
        rect.y += (old_height - rect.height) * pivot[1];
    } else {
        let old_width = rect.width;
        rect.width = rect.height * sprite_ratio;
        rect.x += (old_width - rect.width) * pivot[0];
    }
}

/// `Image.GetDrawingDimensions`: left, bottom, right, top.
pub fn drawing_dimensions(rect: Rect, pivot: [f32; 2], sprite: &SpriteData, preserve_aspect: bool) -> [f32; 4] {
    let padding = sprite.padding();
    let size = sprite.rect_size;
    let mut r = rect;
    let sprite_w = unity_math::round_to_int(size[0]) as f32;
    let sprite_h = unity_math::round_to_int(size[1]) as f32;
    let v = [
        padding[0] / sprite_w,
        padding[1] / sprite_h,
        (sprite_w - padding[2]) / sprite_w,
        (sprite_h - padding[3]) / sprite_h,
    ];
    if preserve_aspect && size[0] * size[0] + size[1] * size[1] > 0.0 {
        preserve_sprite_aspect_ratio(&mut r, size, pivot);
    }
    [
        r.x + r.width * v[0],
        r.y + r.height * v[1],
        r.x + r.width * v[2],
        r.y + r.height * v[3],
    ]
}

/// `Image.GenerateSimpleSprite`.
pub fn simple(rect: Rect, pivot: [f32; 2], sprite: &SpriteData, preserve_aspect: bool, color: [f32; 4]) -> VertexStream {
    let v = drawing_dimensions(rect, pivot, sprite, preserve_aspect);
    let uv = sprite.outer_uv();
    let color32 = unity_math::color32(color);
    let mut vh = VertexStream::default();
    vh.add_vert([v[0], v[1], 0.0], color32, [uv[0], uv[1]]);
    vh.add_vert([v[0], v[3], 0.0], color32, [uv[0], uv[3]]);
    vh.add_vert([v[2], v[3], 0.0], color32, [uv[2], uv[3]]);
    vh.add_vert([v[2], v[1], 0.0], color32, [uv[2], uv[1]]);
    vh.add_triangle(0, 1, 2);
    vh.add_triangle(2, 3, 0);
    vh
}

/// `Image.GetAdjustedBorders`.
pub fn adjusted_borders(border: [f32; 4], adjusted_rect: Rect, original_rect: Rect) -> [f32; 4] {
    let mut border = border;
    for axis in 0..=1 {
        if original_rect.size(axis) != 0.0 {
            let ratio = adjusted_rect.size(axis) / original_rect.size(axis);
            border[axis] *= ratio;
            border[axis + 2] *= ratio;
        }
        let combined = border[axis] + border[axis + 2];
        if adjusted_rect.size(axis) < combined && combined != 0.0 {
            let ratio = adjusted_rect.size(axis) / combined;
            border[axis] *= ratio;
            border[axis + 2] *= ratio;
        }
    }
    border
}

/// `Image.GenerateSlicedSprite`. `rect` is the rect transform's rect and
/// `pixel_adjusted` the pixel-adjusted rect the Image draws into.
pub fn sliced(
    rect: Rect,
    pixel_adjusted: Rect,
    pivot: [f32; 2],
    sprite: &SpriteData,
    multiplied_pixels_per_unit: f32,
    fill_center: bool,
    color: [f32; 4],
) -> VertexStream {
    if !sprite.has_border() {
        return simple(pixel_adjusted, pivot, sprite, false, color);
    }
    let outer = sprite.outer_uv();
    let inner = sprite.inner_uv();
    let padding = sprite.padding();
    let border = sprite.border;
    let r = pixel_adjusted;
    let ppu = multiplied_pixels_per_unit;
    let adjusted = adjusted_borders(
        [border[0] / ppu, border[1] / ppu, border[2] / ppu, border[3] / ppu],
        r,
        rect,
    );
    let padding = [padding[0] / ppu, padding[1] / ppu, padding[2] / ppu, padding[3] / ppu];
    let mut vert = [
        [padding[0], padding[1]],
        [adjusted[0], adjusted[1]],
        [r.width - adjusted[2], r.height - adjusted[3]],
        [r.width - padding[2], r.height - padding[3]],
    ];
    for v in &mut vert {
        v[0] += r.x;
        v[1] += r.y;
    }
    let uv = [
        [outer[0], outer[1]],
        [inner[0], inner[1]],
        [inner[2], inner[3]],
        [outer[2], outer[3]],
    ];
    let color32 = unity_math::color32(color);
    let mut vh = VertexStream::default();
    for x in 0..3 {
        let x2 = x + 1;
        for y in 0..3 {
            if !fill_center && x == 1 && y == 1 {
                continue;
            }
            let y2 = y + 1;
            if vert[x2][0] - vert[x][0] <= 0.0 || vert[y2][1] - vert[y][1] <= 0.0 {
                continue;
            }
            vh.add_quad(
                [vert[x][0], vert[y][1]],
                [vert[x2][0], vert[y2][1]],
                color32,
                [uv[x][0], uv[y][1]],
                [uv[x2][0], uv[y2][1]],
            );
        }
    }
    vh
}

/// `Image.Type` in serialized order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageType {
    Simple,
    Sliced,
    Tiled,
    Filled,
}

impl ImageType {
    pub fn from_serialized(value: i64) -> Option<Self> {
        match value {
            0 => Some(Self::Simple),
            1 => Some(Self::Sliced),
            2 => Some(Self::Tiled),
            3 => Some(Self::Filled),
            _ => None,
        }
    }
}

/// Everything `Image.OnPopulateMesh` reads for the Simple and Sliced paths.
#[derive(Debug, Clone, Copy)]
pub struct ImageInput<'a> {
    pub image_type: ImageType,
    pub sprite: Option<&'a SpriteData>,
    /// `rectTransform.rect`.
    pub rect: Rect,
    /// `GetPixelAdjustedRect()`.
    pub pixel_adjusted_rect: Rect,
    pub pivot: [f32; 2],
    /// `Graphic.color`.
    pub color: [f32; 4],
    pub preserve_aspect: bool,
    pub fill_center: bool,
    pub use_sprite_mesh: bool,
    pub pixels_per_unit_multiplier: f32,
    /// The canvas reference pixels per unit (None: no canvas, the Image's 100).
    pub reference_pixels_per_unit: Option<f32>,
}

/// Result of `Image.OnPopulateMesh` for the paths this module ports.
#[derive(Debug, Clone, PartialEq)]
pub enum Populated {
    Mesh(VertexStream),
    /// A branch of `OnPopulateMesh` that this module does not port, by name.
    Unported(&'static str),
}

/// `Image.OnPopulateMesh(VertexHelper)`.
pub fn populate(input: &ImageInput) -> Populated {
    let Some(sprite) = input.sprite else {
        return Populated::Mesh(graphic_quad(input.pixel_adjusted_rect, input.color));
    };
    match input.image_type {
        ImageType::Simple if input.use_sprite_mesh => Populated::Unported("Image.GenerateSprite"),
        ImageType::Simple => Populated::Mesh(simple(
            input.pixel_adjusted_rect,
            input.pivot,
            sprite,
            input.preserve_aspect,
            input.color,
        )),
        ImageType::Sliced => {
            let ppu = pixels_per_unit(Some(sprite.pixels_per_unit), input.reference_pixels_per_unit);
            Populated::Mesh(sliced(
                input.rect,
                input.pixel_adjusted_rect,
                input.pivot,
                sprite,
                multiplied_pixels_per_unit(ppu, input.pixels_per_unit_multiplier),
                input.fill_center,
                input.color,
            ))
        }
        ImageType::Tiled => Populated::Unported("Image.GenerateTiledSprite"),
        ImageType::Filled => Populated::Unported("Image.GenerateFilledSprite"),
    }
}
