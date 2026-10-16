//! Image fill geometry, before the prefab's world transform is applied.
//! The same normalized cut drives position and UV so that filling reveals the
//! image, rather than stretching it. This does not implement a soft-mask pass.

use bevy::{
    asset::RenderAssetUsages,
    mesh::{Indices, PrimitiveTopology},
    prelude::*,
};

#[derive(Clone, Copy)]
pub(super) enum FillMethod {
    Horizontal,
    Vertical,
    Radial90,
    Radial180,
    Radial360,
}

impl FillMethod {
    pub(super) fn from_serialized(value: i64) -> Self {
        match value {
            0 => Self::Horizontal,
            1 => Self::Vertical,
            2 => Self::Radial90,
            3 => Self::Radial180,
            4 => Self::Radial360,
            _ => panic!("unsupported Image fill method {value}"),
        }
    }
}

pub(super) fn mesh(
    size: Vec2,
    method: FillMethod,
    amount: f32,
    origin: usize,
    clockwise: bool,
) -> Option<Mesh> {
    assert!(amount.is_finite(), "non-finite Image fill amount");
    let radial = !matches!(method, FillMethod::Horizontal | FillMethod::Vertical);
    assert!(
        origin < if radial { 4 } else { 2 },
        "invalid Image fill origin {origin}"
    );
    let amount = amount.clamp(0., 1.);
    if amount < 0.001 {
        return None;
    }
    let mut quads = Vec::with_capacity(4);
    if amount >= 1. {
        quads.push(quad(0., 0., 1., 1.));
    } else {
        match method {
            FillMethod::Horizontal => quads.push(if origin == 1 {
                quad(1. - amount, 0., 1., 1.)
            } else {
                quad(0., 0., amount, 1.)
            }),
            FillMethod::Vertical => quads.push(if origin == 1 {
                quad(0., 1. - amount, 1., 1.)
            } else {
                quad(0., 0., 1., amount)
            }),
            FillMethod::Radial90 => {
                push_cut(&mut quads, quad(0., 0., 1., 1.), amount, clockwise, origin);
            }
            FillMethod::Radial180 => {
                let even = usize::from(origin > 1);
                for side in 0..2 {
                    let bounds = if origin == 0 || origin == 2 {
                        if side == even {
                            quad(0., 0., 0.5, 1.)
                        } else {
                            quad(0.5, 0., 1., 1.)
                        }
                    } else if side == even {
                        quad(0., 0.5, 1., 1.)
                    } else {
                        quad(0., 0., 1., 0.5)
                    };
                    let fill = amount * 2. - if clockwise { side } else { 1 - side } as f32;
                    push_cut(
                        &mut quads,
                        bounds,
                        fill.clamp(0., 1.),
                        clockwise,
                        (side + origin + 3) % 4,
                    );
                }
            }
            FillMethod::Radial360 => {
                for corner in 0..4 {
                    let x = if corner < 2 { 0. } else { 0.5 };
                    let y = if corner == 0 || corner == 3 { 0. } else { 0.5 };
                    let order = (corner + origin) % 4;
                    let fill = amount * 4. - if clockwise { order } else { 3 - order } as f32;
                    push_cut(
                        &mut quads,
                        quad(x, y, x + 0.5, y + 0.5),
                        fill.clamp(0., 1.),
                        clockwise,
                        (corner + 2) % 4,
                    );
                }
            }
        }
    }
    if quads.is_empty() {
        return None;
    }
    let mut positions = Vec::with_capacity(quads.len() * 4);
    let mut uv = Vec::with_capacity(quads.len() * 4);
    let mut indices = Vec::with_capacity(quads.len() * 6);
    for points in quads {
        let start = positions.len() as u32;
        for p in points {
            let position = (p - Vec2::splat(0.5)) * size;
            positions.push([position.x, position.y, 0.]);
            // Extracted standalone sprite images use a top-left UV origin.
            uv.push([p.x, 1. - p.y]);
        }
        // Quad order is bottom-left, top-left, top-right, bottom-right.
        indices.extend_from_slice(&[start, start + 2, start + 1, start, start + 3, start + 2]);
    }
    let normals = vec![[0., 0., 1.]; positions.len()];
    Some(
        Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::RENDER_WORLD,
        )
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uv)
        .with_inserted_indices(Indices::U32(indices)),
    )
}

fn quad(x0: f32, y0: f32, x1: f32, y1: f32) -> [Vec2; 4] {
    [
        Vec2::new(x0, y0),
        Vec2::new(x0, y1),
        Vec2::new(x1, y1),
        Vec2::new(x1, y0),
    ]
}

fn push_cut(
    quads: &mut Vec<[Vec2; 4]>,
    mut points: [Vec2; 4],
    fill: f32,
    clockwise: bool,
    corner: usize,
) {
    if fill < 0.001 {
        return;
    }
    let invert = clockwise ^ (corner & 1 != 0);
    if invert || fill <= 0.999 {
        let angle = if invert { 1. - fill } else { fill } * std::f32::consts::FRAC_PI_2;
        let (mut sin, mut cos) = angle.sin_cos();
        let [i0, i1, i2, i3] = [corner, (corner + 1) % 4, (corner + 2) % 4, (corner + 3) % 4];
        let lerp = |a: f32, b: f32, t: f32| a + (b - a) * t.clamp(0., 1.);
        if corner & 1 != 0 {
            if sin > cos {
                cos /= sin;
                sin = 1.;
                if invert {
                    points[i1].x = lerp(points[i0].x, points[i2].x, cos);
                    points[i2].x = points[i1].x;
                }
            } else if cos > sin {
                sin /= cos;
                cos = 1.;
                if !invert {
                    points[i2].y = lerp(points[i0].y, points[i2].y, sin);
                    points[i3].y = points[i2].y;
                }
            } else {
                cos = 1.;
                sin = 1.;
            }
            if !invert {
                points[i3].x = lerp(points[i0].x, points[i2].x, cos);
            } else {
                points[i1].y = lerp(points[i0].y, points[i2].y, sin);
            }
        } else {
            if cos > sin {
                sin /= cos;
                cos = 1.;
                if !invert {
                    points[i1].y = lerp(points[i0].y, points[i2].y, sin);
                    points[i2].y = points[i1].y;
                }
            } else if sin > cos {
                cos /= sin;
                sin = 1.;
                if invert {
                    points[i2].x = lerp(points[i0].x, points[i2].x, cos);
                    points[i3].x = points[i2].x;
                }
            } else {
                cos = 1.;
                sin = 1.;
            }
            if invert {
                points[i3].y = lerp(points[i0].y, points[i2].y, sin);
            } else {
                points[i1].x = lerp(points[i0].x, points[i2].x, cos);
            }
        }
    }
    quads.push(points);
}
