//! RectMask2D clip geometry and source ScrollMask padding.
//!
//! UGUI's nearest active RectMask2D owns the Graphic, supplies one compound
//! ancestor rectangle, and supplies its own softness (not a product of masks).
//! Rectangular clipping does not implement the separate sprite/stencil Mask.

use super::{UiComponent, UiPrefab, UiRect};
use bevy::math::{Mat4, Vec2};
use serde::Deserialize;
use serde_json::Value;
use std::{collections::HashMap, sync::Arc};

#[derive(Debug, Clone, Deserialize)]
pub struct UiClipMaterial {
    pub shader: String,
    pub softness: Option<[f32; 2]>,
    pub scale: Option<[f32; 2]>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UiClipRect {
    pub min: Vec2,
    pub max: Vec2,
    /// Only the nearest clipper's CanvasRenderer softness. Some source shader
    /// families use material-owned softness or ignore softness altogether.
    pub softness: Vec2,
    pub valid: bool,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct UiClipping {
    pub rect: Option<UiClipRect>,
    raycasts: Arc<[RaycastMask]>,
}

impl UiClipping {
    pub fn contains(&self, point: Vec2) -> bool {
        self.raycasts.iter().all(|mask| mask.contains(point))
    }
}

#[derive(Debug, Clone, PartialEq)]
struct RaycastMask {
    local_from_canvas: Mat4,
    min: Vec2,
    max: Vec2,
    valid: bool,
}

impl RaycastMask {
    fn new(rect: &UiRect, padding: [f32; 4]) -> Self {
        let valid = rect.world.determinant().abs() >= f32::EPSILON;
        let min = -rect.size * rect.pivot + Vec2::new(padding[0], padding[1]);
        let max = rect.size * (Vec2::ONE - rect.pivot) - Vec2::new(padding[2], padding[3]);
        Self {
            local_from_canvas: if valid {
                rect.world.inverse()
            } else {
                Mat4::IDENTITY
            },
            min,
            max,
            valid: valid && max.cmpgt(min).all(),
        }
    }

    fn contains(&self, point: Vec2) -> bool {
        if !self.valid {
            return false;
        }
        let local = self
            .local_from_canvas
            .transform_point3(point.extend(0.))
            .truncate();
        local.cmpge(self.min).all() && local.cmple(self.max).all()
    }
}

/// Run after the shared auto-layout pass has resolved final RectTransforms.
/// Rectangular drawing and raycast filtering deliberately have separate data:
/// raycasts use each ancestor's transformed padded rectangle, not feather alpha.
pub fn apply(prefab: &UiPrefab, rects: &mut [UiRect]) -> Result<(), String> {
    if rects.len() != prefab.nodes.len() {
        return Err("UI clip geometry length differs from prefab".into());
    }
    let runtime_padding = scroll_mask_padding(prefab, rects)?;
    let mut descendants: Vec<UiClipping> = Vec::with_capacity(rects.len());
    let mut compounds: Vec<Option<UiClipRect>> = Vec::with_capacity(rects.len());
    for (index, node) in prefab.nodes.iter().enumerate() {
        let mut inherited = prefab.indices.parents[index]
            .map(|parent| descendants[parent].clone())
            .unwrap_or_default();
        let mut compound = prefab.indices.parents[index].and_then(|parent| compounds[parent]);
        // GetRectMaskForClippable / Graphic.Raycast stop at a sorting Canvas.
        if node.components.iter().any(|component| {
            component.class == "UnityEngine.Canvas"
                && boolean(&component.fields, "m_OverrideSorting", false)
        }) {
            inherited = UiClipping::default();
            compound = None;
        }
        let drawing = inherited.rect;
        if rects[index].active {
            for component in &node.components {
                let rectangular = component.class == "UnityEngine.UI.RectMask2D";
                let stencil = component.class == "UnityEngine.UI.Mask";
                if !(rectangular || stencil) {
                    continue;
                }
                let driven = rectangular.then(|| runtime_padding.get(&index)).flatten();
                if !component.enabled && driven.is_none() {
                    continue;
                }
                let padding = if rectangular {
                    driven
                        .copied()
                        .map(Ok)
                        .unwrap_or_else(|| vector4(&component.fields, "m_Padding"))?
                } else {
                    [0.; 4]
                };
                let mut raycasts = inherited.raycasts.to_vec();
                raycasts.push(RaycastMask::new(&rects[index], padding));
                inherited.raycasts = Arc::from(raycasts);
                if rectangular {
                    let softness = vector2(&component.fields, "m_Softness")?;
                    if softness.min_element() < 0. {
                        return Err(format!("negative RectMask2D softness: {}", node.path));
                    }
                    let (corner0, corner2) = corners(&rects[index]);
                    let mut min = corner0 + Vec2::new(padding[0], padding[1]);
                    let mut max = corner2 - Vec2::new(padding[2], padding[3]);
                    let mut valid = true;
                    if let Some(parent) = compound {
                        min = min.max(parent.min);
                        max = max.min(parent.max);
                        valid &= parent.valid;
                    }
                    valid &= max.cmpgt(min).all();
                    // Child clippers recompute the raw ancestor intersection;
                    // an ancestor's own viewport-cull decision is not inherited.
                    compound = Some(UiClipRect {
                        min,
                        max,
                        softness,
                        valid,
                    });
                    // Source PerformClipping's rootCanvasRect is this mask's
                    // world corners in root-canvas coordinates, not the screen.
                    valid &= max.cmpgt(corner0.min(corner2)).all()
                        && corner0.max(corner2).cmpgt(min).all();
                    inherited.rect = Some(UiClipRect {
                        min,
                        max,
                        softness,
                        valid,
                    });
                }
            }
        }
        // A RectMask2D does not register its own same-GameObject Graphic as a
        // clippable, but that node's ICanvasRaycastFilter still participates.
        rects[index].clipping = UiClipping {
            rect: drawing,
            raycasts: inherited.raycasts.clone(),
        };
        descendants.push(inherited);
        compounds.push(compound);
    }
    Ok(())
}

fn corners(rect: &UiRect) -> (Vec2, Vec2) {
    let local_min = -rect.size * rect.pivot;
    (
        rect.world.transform_point3(local_min.extend(0.)).truncate(),
        rect.world
            .transform_point3((local_min + rect.size).extend(0.))
            .truncate(),
    )
}

/// CN CustomScrollRect.SetMask + ScrollMask.SetMask{Vertical,Horizontal}.
/// ScreenManager.WorldToScreenPoint divides screen pixels by Canvas.scaleFactor,
/// so the source threshold and these resolved coordinates are both canvas units.
fn scroll_mask_padding(
    prefab: &UiPrefab,
    rects: &[UiRect],
) -> Result<HashMap<usize, [f32; 4]>, String> {
    let mut result = HashMap::new();
    for (index, node) in prefab.nodes.iter().enumerate() {
        if !rects[index].active {
            continue;
        }
        for scroll in node
            .components
            .iter()
            .filter(|c| c.enabled && c.class == "Sekai.UI.CustomScrollRect")
        {
            let Some(mask_id) = pointer_id(&scroll.fields, "mask")? else {
                continue;
            };
            let scroll_mask_node = prefab.find(&format!("@{mask_id}"))?;
            let scroll_mask = prefab.nodes[scroll_mask_node]
                .components
                .iter()
                .find(|c| c.path_id == mask_id && c.class == "Sekai.UI.ScrollMask")
                .ok_or_else(|| format!("CustomScrollRect mask {mask_id} is not ScrollMask"))?;
            let mask_id =
                pointer_id(&scroll_mask.fields, "mask")?.ok_or("ScrollMask has no RectMask2D")?;
            let mask_index = prefab.find(&format!("@{mask_id}"))?;
            let mask = prefab.nodes[mask_index]
                .components
                .iter()
                .find(|c| c.path_id == mask_id && c.class == "UnityEngine.UI.RectMask2D")
                .ok_or_else(|| format!("ScrollMask target {mask_id} is not RectMask2D"))?;
            let softness = vector2(&mask.fields, "m_Softness")?;
            let threshold = scalar(&scroll_mask.fields, "maskThreshold")?;
            if threshold <= 0. {
                return Err("ScrollMask threshold must be positive".into());
            }
            let viewport =
                pointer_id(&scroll.fields, "m_Viewport")?.ok_or("ScrollMask viewport missing")?;
            let content =
                pointer_id(&scroll.fields, "m_Content")?.ok_or("ScrollMask content missing")?;
            let viewport = &rects[prefab.find(&format!("@{viewport}"))?];
            let content = &rects[prefab.find(&format!("@{content}"))?];
            let vertical = boolean(&scroll.fields, "m_Vertical", false);
            let horizontal = boolean(&scroll.fields, "m_Horizontal", false);
            let axis = if vertical { 1 } else { 0 };
            let scrollable = viewport.size[axis] < content.size[axis];
            // SetMaskEnabled(true) also applies to an authored-disabled mask.
            let mut padding = vector4(&mask.fields, "m_Padding")?;
            if vertical || horizontal {
                let half = softness[axis] * 0.5;
                let (near, far) = if scrollable {
                    let (v0, v2) = corners(viewport);
                    let (c0, c2) = corners(content);
                    (
                        (c0[axis] - v0[axis]).abs().min(threshold),
                        (v2[axis] - c2[axis]).abs().min(threshold),
                    )
                } else {
                    (0., 0.)
                };
                padding = [0.; 4];
                padding[axis] = half * (near / threshold) - half;
                padding[axis + 2] = half * (far / threshold) - half;
            }
            if result.insert(mask_index, padding).is_some() {
                return Err("multiple active CustomScrollRects drive one ScrollMask".into());
            }
        }
    }
    Ok(result)
}

pub fn maskable(component: &UiComponent) -> bool {
    boolean(&component.fields, "m_Maskable", true)
}

fn boolean(fields: &Value, name: &str, default: bool) -> bool {
    fields
        .get(name)
        .and_then(|v| v.as_bool().or_else(|| v.as_i64().map(|n| n != 0)))
        .unwrap_or(default)
}

fn scalar(fields: &Value, name: &str) -> Result<f32, String> {
    let value = fields
        .get(name)
        .and_then(Value::as_f64)
        .ok_or_else(|| format!("missing clip field {name}"))? as f32;
    value
        .is_finite()
        .then_some(value)
        .ok_or_else(|| format!("non-finite clip field {name}"))
}

fn vector2(fields: &Value, name: &str) -> Result<Vec2, String> {
    let values = fields
        .get(name)
        .and_then(Value::as_array)
        .filter(|v| v.len() == 2)
        .ok_or_else(|| format!("missing clip Vector2 {name}"))?;
    let x = values[0].as_f64().ok_or("clip Vector2 x")? as f32;
    let y = values[1].as_f64().ok_or("clip Vector2 y")? as f32;
    let result = Vec2::new(x, y);
    result
        .is_finite()
        .then_some(result)
        .ok_or_else(|| format!("non-finite clip Vector2 {name}"))
}

fn vector4(fields: &Value, name: &str) -> Result<[f32; 4], String> {
    let values = fields
        .get(name)
        .and_then(Value::as_array)
        .filter(|v| v.len() == 4)
        .ok_or_else(|| format!("missing clip Vector4 {name}"))?;
    let mut result = [0.; 4];
    for (out, value) in result.iter_mut().zip(values) {
        *out = value.as_f64().ok_or("clip Vector4 component")? as f32;
        if !out.is_finite() {
            return Err(format!("non-finite clip Vector4 {name}"));
        }
    }
    Ok(result)
}

fn pointer_id(fields: &Value, name: &str) -> Result<Option<i64>, String> {
    let Some(value) = fields.get(name) else {
        return Ok(None);
    };
    let (file, id) = if let Some(pair) = value.as_array().filter(|p| p.len() == 2) {
        (pair[0].as_i64(), pair[1].as_i64())
    } else {
        (value["m_FileID"].as_i64(), value["m_PathID"].as_i64())
    };
    let (file, id) = (
        file.ok_or_else(|| format!("invalid clip PPtr {name}"))?,
        id.ok_or_else(|| format!("invalid clip PPtr {name}"))?,
    );
    if id == 0 {
        return Ok(None);
    }
    if file != 0 {
        return Err(format!("external clip PPtr {name}: {file}:{id}"));
    }
    Ok(Some(id))
}
