//! Nine-slice and tiling geometry for sprites drawn as CPU meshes.
//!
//! Adapted from the texture slicing of `bevy_sprite` (MIT OR Apache-2.0). The
//! clip renderer builds its own quad meshes, so it needs the slice rectangles
//! themselves rather than a shader-side slicing mode.

use bevy::log::{error, warn};
use bevy::math::{vec2, Rect, Vec2};
use bevy::sprite::{BorderRect, SliceScaleMode, TextureSlicer};

/// One texture rectangle and the area it is drawn into.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct TextureSlice {
    /// Texture area to draw.
    pub texture_rect: Rect,
    /// Draw size of the slice.
    pub draw_size: Vec2,
    /// Offset of the slice from the sprite centre.
    pub offset: Vec2,
}

impl TextureSlice {
    /// Splits the slice into tiles. The slice repeats along an axis when the
    /// ratio of its draw size to the texture rectangle exceeds `stretch_value`.
    #[must_use]
    pub fn tiled(self, stretch_value: f32, (tile_x, tile_y): (bool, bool)) -> Vec<Self> {
        if !tile_x && !tile_y {
            return vec![self];
        }
        let stretch_value = stretch_value.max(0.001);
        let rect_size = self.texture_rect.size();
        // Expected size of each tile; no tile is thinner than one pixel.
        let expected_size = Vec2::new(
            if tile_x {
                (rect_size.x * stretch_value).max(1.0)
            } else {
                self.draw_size.x
            },
            if tile_y {
                (rect_size.y * stretch_value).max(1.0)
            } else {
                self.draw_size.y
            },
        )
        .min(self.draw_size);

        let mut slices = Vec::new();
        let base_offset = Vec2::new(-self.draw_size.x / 2.0, self.draw_size.y / 2.0);
        let mut offset = base_offset;

        let mut remaining_columns = self.draw_size.y;
        while remaining_columns > 0.0 {
            let size_y = expected_size.y.min(remaining_columns);
            offset.x = base_offset.x;
            offset.y -= size_y / 2.0;
            let mut remaining_rows = self.draw_size.x;
            while remaining_rows > 0.0 {
                let size_x = expected_size.x.min(remaining_rows);
                offset.x += size_x / 2.0;
                let draw_size = Vec2::new(size_x, size_y);
                let delta = draw_size / expected_size;
                slices.push(Self {
                    texture_rect: Rect {
                        min: self.texture_rect.min,
                        max: self.texture_rect.min + self.texture_rect.size() * delta,
                    },
                    draw_size,
                    offset: self.offset + offset,
                });
                offset.x += size_x / 2.0;
                remaining_rows -= size_x;
            }
            offset.y -= size_y / 2.0;
            remaining_columns -= size_y;
        }
        if slices.len() > 1_000 {
            warn!(
                "A tiled texture generated {} slices; a larger stretch value reduces the cost",
                slices.len()
            );
        }
        slices
    }
}

/// The four corner slices in the order top-left, top-right, bottom-left,
/// bottom-right.
fn corner_slices(slicer: &TextureSlicer, base_rect: Rect, render_size: Vec2) -> [TextureSlice; 4] {
    let coef = render_size / base_rect.size();
    let BorderRect {
        min_inset: Vec2 { x: left, y: top },
        max_inset: Vec2 { x: right, y: bottom },
    } = slicer.border;
    let min_coef = coef.x.min(coef.y).min(slicer.max_corner_scale);
    [
        TextureSlice {
            texture_rect: Rect {
                min: base_rect.min,
                max: base_rect.min + vec2(left, top),
            },
            draw_size: vec2(left, top) * min_coef,
            offset: vec2(-render_size.x + left * min_coef, render_size.y - top * min_coef) / 2.0,
        },
        TextureSlice {
            texture_rect: Rect {
                min: vec2(base_rect.max.x - right, base_rect.min.y),
                max: vec2(base_rect.max.x, base_rect.min.y + top),
            },
            draw_size: vec2(right, top) * min_coef,
            offset: vec2(render_size.x - right * min_coef, render_size.y - top * min_coef) / 2.0,
        },
        TextureSlice {
            texture_rect: Rect {
                min: vec2(base_rect.min.x, base_rect.max.y - bottom),
                max: vec2(base_rect.min.x + left, base_rect.max.y),
            },
            draw_size: vec2(left, bottom) * min_coef,
            offset: vec2(-render_size.x + left * min_coef, -render_size.y + bottom * min_coef)
                / 2.0,
        },
        TextureSlice {
            texture_rect: Rect {
                min: vec2(base_rect.max.x - right, base_rect.max.y - bottom),
                max: base_rect.max,
            },
            draw_size: vec2(right, bottom) * min_coef,
            offset: vec2(render_size.x - right * min_coef, -render_size.y + bottom * min_coef)
                / 2.0,
        },
    ]
}

/// The left and right side slices.
fn horizontal_side_slices(
    slicer: &TextureSlicer,
    [tl_corner, tr_corner, bl_corner, br_corner]: &[TextureSlice; 4],
    base_rect: Rect,
    render_size: Vec2,
) -> [TextureSlice; 2] {
    [
        TextureSlice {
            texture_rect: Rect {
                min: base_rect.min + vec2(0.0, slicer.border.min_inset.y),
                max: vec2(
                    base_rect.min.x + slicer.border.min_inset.x,
                    base_rect.max.y - slicer.border.max_inset.y,
                ),
            },
            draw_size: vec2(
                tl_corner.draw_size.x,
                render_size.y - (tl_corner.draw_size.y + bl_corner.draw_size.y),
            ),
            offset: vec2(
                tl_corner.draw_size.x - render_size.x,
                bl_corner.draw_size.y - tl_corner.draw_size.y,
            ) / 2.0,
        },
        TextureSlice {
            texture_rect: Rect {
                min: vec2(
                    base_rect.max.x - slicer.border.max_inset.x,
                    base_rect.min.y + slicer.border.min_inset.y,
                ),
                max: base_rect.max - vec2(0.0, slicer.border.max_inset.y),
            },
            draw_size: vec2(
                tr_corner.draw_size.x,
                render_size.y - (tr_corner.draw_size.y + br_corner.draw_size.y),
            ),
            offset: vec2(
                render_size.x - tr_corner.draw_size.x,
                br_corner.draw_size.y - tr_corner.draw_size.y,
            ) / 2.0,
        },
    ]
}

/// The top and bottom side slices.
fn vertical_side_slices(
    slicer: &TextureSlicer,
    [tl_corner, tr_corner, bl_corner, br_corner]: &[TextureSlice; 4],
    base_rect: Rect,
    render_size: Vec2,
) -> [TextureSlice; 2] {
    [
        TextureSlice {
            texture_rect: Rect {
                min: base_rect.min + vec2(slicer.border.min_inset.x, 0.0),
                max: vec2(
                    base_rect.max.x - slicer.border.max_inset.x,
                    base_rect.min.y + slicer.border.min_inset.y,
                ),
            },
            draw_size: vec2(
                render_size.x - (tl_corner.draw_size.x + tr_corner.draw_size.x),
                tl_corner.draw_size.y,
            ),
            offset: vec2(
                tl_corner.draw_size.x - tr_corner.draw_size.x,
                render_size.y - tl_corner.draw_size.y,
            ) / 2.0,
        },
        TextureSlice {
            texture_rect: Rect {
                min: vec2(
                    base_rect.min.x + slicer.border.min_inset.x,
                    base_rect.max.y - slicer.border.max_inset.y,
                ),
                max: base_rect.max - vec2(slicer.border.max_inset.x, 0.0),
            },
            draw_size: vec2(
                render_size.x - (bl_corner.draw_size.x + br_corner.draw_size.x),
                bl_corner.draw_size.y,
            ),
            offset: vec2(
                bl_corner.draw_size.x - br_corner.draw_size.x,
                bl_corner.draw_size.y - render_size.y,
            ) / 2.0,
        },
    ]
}

/// Slices `rect` into at least nine sections drawn over `render_size`; centre
/// and sides in tile mode add more.
#[must_use]
pub(super) fn compute_slices(
    slicer: &TextureSlicer,
    rect: Rect,
    render_size: Option<Vec2>,
) -> Vec<TextureSlice> {
    let render_size = render_size.unwrap_or_else(|| rect.size());
    if (slicer.border.min_inset + slicer.border.max_inset)
        .cmpge(rect.size())
        .any()
    {
        error!("TextureSlicer::border has out of bounds values. No slicing will be applied");
        return vec![TextureSlice {
            texture_rect: rect,
            draw_size: render_size,
            offset: Vec2::ZERO,
        }];
    }
    let mut slices = Vec::with_capacity(9);
    let corners = corner_slices(slicer, rect, render_size);
    let vertical_sides = vertical_side_slices(slicer, &corners, rect, render_size);
    let horizontal_sides = horizontal_side_slices(slicer, &corners, rect, render_size);
    let center = TextureSlice {
        texture_rect: Rect {
            min: rect.min + slicer.border.min_inset,
            max: rect.max - slicer.border.max_inset,
        },
        draw_size: vec2(
            render_size.x - (corners[0].draw_size.x + corners[1].draw_size.x),
            render_size.y - (corners[0].draw_size.y + corners[2].draw_size.y),
        ),
        offset: vec2(vertical_sides[0].offset.x, horizontal_sides[0].offset.y),
    };

    slices.extend(corners);
    match slicer.center_scale_mode {
        SliceScaleMode::Stretch => slices.push(center),
        SliceScaleMode::Tile { stretch_value } => {
            slices.extend(center.tiled(stretch_value, (true, true)));
        }
    }
    match slicer.sides_scale_mode {
        SliceScaleMode::Stretch => {
            slices.extend(horizontal_sides);
            slices.extend(vertical_sides);
        }
        SliceScaleMode::Tile { stretch_value } => {
            slices.extend(
                horizontal_sides
                    .into_iter()
                    .flat_map(|s| s.tiled(stretch_value, (false, true))),
            );
            slices.extend(
                vertical_sides
                    .into_iter()
                    .flat_map(|s| s.tiled(stretch_value, (true, false))),
            );
        }
    }
    slices
}
