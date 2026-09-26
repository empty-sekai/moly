//! UGUI `Mask` (the stencil mask) on the prefab renderer.
//!
//! Source (UGUI 1.0.0 of Unity 2022.3): `MaskUtilities.GetStencilDepth` counts
//! the ancestors carrying an enabled `Mask` whose Graphic is active, walking
//! up to and including the nearest Canvas that overrides sorting. A maskable
//! Graphic with a depth above zero draws with stencil test Equal against the
//! masks' bits (`MaskableGraphic.GetModifiedMaterial`). The masking graphic
//! itself writes the stencil (`Mask.GetModifiedMaterial`: Replace, Always at
//! depth one, colour write mask `All` only when `showMaskGraphic`), with the
//! alpha clip on (`StencilMaterial.Add` enables `UNITY_UI_ALPHACLIP` for a
//! writing operation; UI/Default then clips `color.a - 0.001`, colour =
//! texture sample times vertex colour).
//!
//! The 2D pass here has no stencil aspect (its depth target is Depth32Float),
//! so the stencil is not replayed as a buffer. A Graphic under a mask is drawn
//! with the masking graphic's coverage test in its own fragment instead: the
//! masking graphic's mesh is a set of axis-aligned quads, each with its uvs
//! mapped linearly over it (a Simple or Sliced Image through the Image rules,
//! a Horizontal or Vertical Filled Image through `GenerateFilledSprite`,
//! which cuts the quad and its uvs by the same amount); a fragment passes
//! where one of those quads covers it and the masking graphic's texture there
//! times its vertex alpha is not below 0.001. That is the stencil's content.
//!
//! Under a RectMask2D the masking graphic is a clippable like any other
//! (`MaskableGraphic.UpdateClipParent` does not look at `isMaskingGraphic`),
//! so its stencil-writing draw has the rectangle clip: UI/Default multiplies
//! the alpha by the RectMask2D softness factor before the alpha clip, and a
//! culled masking graphic (`MaskableGraphic.Cull`) draws nothing, so nothing
//! under it is drawn. The Graphics under the mask keep their own RectMask2D
//! clip as well; the two tests compose per fragment.
//!
//! Other masking graphics (radial fills, Tiled images, raw images, text, a
//! rotated quad, a shader other than UI/Default) and a mask inside another
//! mask are refused with one error each and leave their Graphics untested.

use super::{
    Override, UiLayouts,
    clip_render::{self, MaskingClip, Stencil},
    image_path, image_rule_mesh_data,
};
use bevy::prelude::*;
use moly_assets::ui_layout::{UiComponent, UiNode, UiPrefab, UiRect};
use serde_json::Value;
use std::collections::HashMap;

/// The most quads a masking graphic draws here (a Sliced image's nine).
pub(super) const MAX_STENCIL_QUADS: usize = 9;

/// The stencil state of one prefab view's nodes.
pub(super) struct Stencils {
    /// The stencil each node's Graphics are tested against.
    pub(super) tested: Vec<Option<Stencil>>,
    /// The masking graphic of an enabled mask that does not show it: its
    /// colour write mask is 0, so it is not drawn.
    pub(super) unshown: Vec<Option<i64>>,
}

/// The Graphic `Mask.graphic` names: the GameObject's Graphic.
fn graphic(node: &UiNode) -> Option<&UiComponent> {
    node.components.iter().find(|c| c.fields.get("m_Color").is_some() && c.fields.get("m_Material").is_some())
}

fn overrides_sorting(node: &UiNode) -> bool {
    node.components.iter().any(|c| {
        c.class == "UnityEngine.Canvas" && c.fields["m_OverrideSorting"].as_bool() == Some(true)
    })
}

pub(super) fn resolve(
    layouts: &UiLayouts,
    doc: &UiPrefab,
    rects: &[UiRect],
    overrides: &HashMap<usize, &Override>,
    alphas: &[f32],
) -> Stencils {
    let count = doc.nodes.len();
    let mut tested: Vec<Option<Stencil>> = Vec::with_capacity(count);
    // The stencil the node's descendants are tested against.
    let mut below: Vec<Option<Stencil>> = Vec::with_capacity(count);
    let mut unshown = vec![None; count];
    for (index, node) in doc.nodes.iter().enumerate() {
        // GetStencilDepth starts at the parent and stops after the nearest
        // Canvas overriding sorting, which FindRootSortOverrideCanvas finds
        // from the Graphic's own GameObject: a Graphic on that Canvas has
        // depth 0.
        let own = if overrides_sorting(node) {
            None
        } else {
            doc.parent(index).and_then(|parent| below[parent].clone())
        };
        let mut next = own.clone();
        let mask = node.components.iter().find(|c| c.class == "UnityEngine.UI.Mask");
        if let (Some(mask), Some(graphic)) = (mask, graphic(node)) {
            if rects[index].active && mask.enabled {
                if mask.fields["m_ShowMaskGraphic"].as_bool() == Some(false) {
                    unshown[index] = Some(graphic.path_id);
                }
                if graphic.enabled {
                    let shape = if own.is_some() {
                        Err("a mask inside another mask".to_owned())
                    } else {
                        shape(layouts, doc, index, graphic, &rects[index], overrides.get(&index).copied(), alphas[index])
                    };
                    match shape {
                        Ok(mut stencil) => {
                            match clip_render::masking_clip(graphic, &rects[index]) {
                                MaskingClip::None => {}
                                MaskingClip::Culled => stencil.quads.clear(),
                                MaskingClip::Rect(clip) => stencil.clip = Some(clip),
                            }
                            next = Some(stencil);
                        }
                        Err(reason) => layouts.note_stencil_refusal(doc, mask, &reason),
                    }
                }
            }
        }
        tested.push(own);
        below.push(next);
    }
    Stencils { tested, unshown }
}

/// One quad of the masking graphic's mesh: its canvas rectangle (min, max)
/// and the uv at those two corners.
struct MaskQuad {
    min: Vec2,
    max: Vec2,
    uv_min: Vec2,
    uv_max: Vec2,
}

/// The stencil a masking graphic writes.
fn shape(
    layouts: &UiLayouts,
    doc: &UiPrefab,
    index: usize,
    graphic: &UiComponent,
    rect: &UiRect,
    change: Option<&Override>,
    group_alpha: f32,
) -> Result<Stencil, String> {
    let f = &graphic.fields;
    let image_type = f.get("m_Type").and_then(Value::as_i64)
        .ok_or_else(|| format!("masking graphic {} is not an Image", graphic.class))?;
    // The written alpha below is UI/Default's fragment alpha.
    match clip_render::shader(graphic) {
        Some("UI/Default") => {}
        other => return Err(format!("a masking graphic with shader {other:?}")),
    }
    let world = rect.world;
    let (sx, sy) = (world.x_axis.x, world.y_axis.y);
    let skew = world.x_axis.y.abs().max(world.y_axis.x.abs());
    if sx == 0. || sy == 0. || skew > 1e-6 * sx.abs().min(sy.abs()) {
        return Err("a rotated masking quad".to_owned());
    }
    // Local (relative to the rect centre) quads with uvs in the exported
    // top-down image.
    let local: Vec<MaskQuad> = match image_type {
        0 | 1 => {
            let data = image_rule_mesh_data(layouts, doc, index, graphic, change, rect)
                .map_err(|reason| format!("the Image rules do not mesh the masking image ({reason:?})"))?;
            if data.positions.len() % 4 != 0 || data.indices.len() != data.positions.len() / 4 * 6 {
                return Err("the masking image's mesh is not made of quads".to_owned());
            }
            let mut quads = Vec::with_capacity(data.positions.len() / 4);
            for (p, uv) in data.positions.chunks(4).zip(data.uvs.chunks(4)) {
                // VertexHelper.AddQuad order: min, (min.x, max.y), max, (max.x, min.y).
                let corners_match = p[1][0] == p[0][0] && p[1][1] == p[2][1] && p[3][0] == p[2][0] && p[3][1] == p[0][1]
                    && uv[1][0] == uv[0][0] && uv[1][1] == uv[2][1] && uv[3][0] == uv[2][0] && uv[3][1] == uv[0][1];
                if !corners_match {
                    return Err("a masking image quad is not axis-aligned".to_owned());
                }
                quads.push(MaskQuad {
                    min: Vec2::new(p[0][0], p[0][1]), max: Vec2::new(p[2][0], p[2][1]),
                    uv_min: Vec2::from_array(uv[0]), uv_max: Vec2::from_array(uv[2]),
                });
            }
            quads
        }
        3 => {
            let method = f["m_FillMethod"].as_i64().expect("Image fill method");
            if method > 1 {
                return Err(format!("a Filled masking image with fill method {method}"));
            }
            if f["m_PreserveAspect"].as_bool() != Some(false) {
                return Err("a Filled masking image with preserveAspect".to_owned());
            }
            if let Some(sprite) = graphic.sprite.as_ref() {
                let at = |name: &str, i: usize| sprite[name][i].as_f64();
                if at("textureRectOffset", 0) != Some(0.) || at("textureRectOffset", 1) != Some(0.)
                    || at("textureRect", 2) != at("rect", 2) || at("textureRect", 3) != at("rect", 3) {
                    return Err("a Filled masking sprite with padding".to_owned());
                }
            }
            let origin = f["m_FillOrigin"].as_i64().expect("Image fill origin");
            let amount = change.and_then(|c| c.fill)
                .unwrap_or_else(|| f["m_FillAmount"].as_f64().expect("Image fill amount") as f32)
                .clamp(0., 1.);
            // GenerateFilledSprite: nothing below 0.001; otherwise the quad
            // and its uvs are cut from the origin side by the amount.
            if amount < 0.001 {
                Vec::new()
            } else {
                let (mut a, mut b) = (Vec2::ZERO, Vec2::ONE);
                match (method, origin) {
                    (0, 1) => a.x = 1. - amount,
                    (0, _) => b.x = amount,
                    (_, 1) => a.y = 1. - amount,
                    _ => b.y = amount,
                }
                let to_local = |n: Vec2| (n - Vec2::splat(0.5)) * rect.size;
                vec![MaskQuad {
                    min: to_local(a), max: to_local(b),
                    uv_min: Vec2::new(a.x, 1. - a.y), uv_max: Vec2::new(b.x, 1. - b.y),
                }]
            }
        }
        other => return Err(format!("a masking image of type {other}")),
    };
    if local.len() > MAX_STENCIL_QUADS {
        return Err(format!("a masking image of {} quads", local.len()));
    }
    let centre = rect.center();
    let mut quads = Vec::with_capacity(local.len());
    for quad in local {
        // Canvas corners of the two uv corners; the scale may mirror them.
        let c0 = centre + Vec2::new(sx, sy) * quad.min;
        let c1 = centre + Vec2::new(sx, sy) * quad.max;
        let span = c1 - c0;
        if span.x == 0. || span.y == 0. {
            continue;
        }
        let scale = (quad.uv_max - quad.uv_min) / span;
        let offset = quad.uv_min - c0 * scale;
        quads.push((
            Vec4::new(c0.x.min(c1.x), c0.y.min(c1.y), c0.x.max(c1.x), c0.y.max(c1.y)),
            Vec4::new(scale.x, scale.y, offset.x, offset.y),
        ));
    }
    // Graphic.color reaches the vertices as Color32.
    let alpha = match change.and_then(|c| c.graphic_color).filter(|(id, _)| *id == graphic.path_id) {
        Some((_, value)) => value[3],
        None => f["m_Color"][3].as_f64().expect("Image m_Color alpha") as f32,
    };
    let vertex_alpha = (alpha.clamp(0., 1.) * 255.).round() / 255. * group_alpha;
    let texture = image_path(layouts, graphic, change).map(|path| layouts.images[path].clone());
    Ok(Stencil { quads, vertex_alpha, texture, clip: None })
}
