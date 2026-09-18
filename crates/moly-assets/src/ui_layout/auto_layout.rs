//! Pure calculations for serialized linear/grid layout groups and fitters.
//!
//! Geometry is returned through the same RectTransform overrides used for drawing
//! and hit testing. Text and other external layout elements are measured by the
//! caller: horizontal calculation/control completes before vertical measurement.

use super::{RectTransform, UiComponent, UiPrefab};
use bevy::math::Vec2;
use serde_json::Value;
use std::collections::HashMap;

mod grid;
use grid::Grid;

/// One external ILayoutElement's properties for one axis.
///
/// Negative properties do not participate. Priority is evaluated independently
/// for min, preferred and flexible, alongside serialized LayoutElement values.
/// The callback represents one provider. If a node has several external providers,
/// a host may premerge them only when the winners share a priority; otherwise it
/// needs a richer provider interface, not an invented common priority.
#[derive(Debug, Clone, Copy)]
pub struct LayoutMetrics {
    pub min: f32,
    pub preferred: f32,
    pub flexible: f32,
    pub priority: i32,
}

/// Calculate the supported controllers in a prefab without changing the prefab.
///
/// `measure(node, axis, resolved_size)` supplies external layout properties.
/// Axis 0 is horizontal, 1 is vertical. During vertical measurement the supplied
/// width already includes horizontal groups and fitters. Horizontal preferred
/// width should be the element's horizontal requirement, not a height measured
/// using an obsolete wrapping width. `None` means no external layout element.
///
/// Explicit overrides are the starting geometry; layout controllers can overwrite
/// their driven properties, just as they overwrite serialized geometry. Inactive
/// hierarchies are not driven. Other component families remain uninterpreted.
/// Returned overrides include all input overrides plus the driven transforms.
pub fn compute_overrides<F>(
    prefab: &UiPrefab,
    canvas: Vec2,
    visible: &HashMap<usize, bool>,
    rect_overrides: &HashMap<usize, RectTransform>,
    mut measure: F,
) -> Result<HashMap<usize, RectTransform>, String>
where
    F: FnMut(usize, usize, Vec2) -> Option<LayoutMetrics>,
{
    let mut work = Work::new(prefab, canvas, visible, rect_overrides)?;
    // LayoutRebuilder ordering: measure H, control H, measure V, control V.
    for axis in 0..2 {
        work.refresh_sizes()?;
        work.measure_axis(axis, &mut measure)?;
        work.control_axis(axis)?;
    }
    let mut output = rect_overrides.clone();
    for (i, changed) in work.changed.iter().copied().enumerate() {
        if changed {
            output.insert(i, work.rects[i].clone());
        }
    }
    Ok(output)
}

#[derive(Clone, Copy, Default)]
struct Sizes {
    min: f32,
    preferred: f32,
    flexible: f32,
}

impl Sizes {
    fn is_finite(self) -> bool {
        self.min.is_finite() && self.preferred.is_finite() && self.flexible.is_finite()
    }
}

#[derive(Clone, Copy)]
struct Element {
    axes: [Sizes; 2],
    priority: i32,
}

#[derive(Clone, Copy)]
struct Group {
    vertical: bool,
    // left, right, top, bottom -- RectOffset stores integers, not float bits.
    padding: [i32; 4],
    spacing: f32,
    alignment: i32,
    control: [bool; 2],
    scale: [bool; 2],
    expand: [bool; 2],
    reverse: bool,
}

impl Group {
    fn padding_sum(self, axis: usize) -> f32 {
        self.padding[axis * 2] as f32 + self.padding[axis * 2 + 1] as f32
    }

    fn leading(self, axis: usize) -> f32 {
        self.padding[axis * 2] as f32
    }

    fn alignment(self, axis: usize) -> f32 {
        let cell = if axis == 0 {
            self.alignment % 3
        } else {
            self.alignment / 3
        };
        cell as f32 * 0.5
    }

    fn cross_axis(self, axis: usize) -> bool {
        self.vertical ^ (axis == 1)
    }

    fn start_offset(self, axis: usize, parent_size: f32, required: f32) -> f32 {
        self.leading(axis)
            + (parent_size - self.padding_sum(axis) - required) * self.alignment(axis)
    }
}

#[derive(Default)]
struct Plan {
    group: Option<Group>,
    grid: Option<Grid>,
    fit: Option<[i32; 2]>,
    elements: Vec<Element>,
    ignore: bool,
}

/// The priority of one winning property, not of an entire element tuple.
struct Winner {
    value: f32,
    priority: i32,
}

impl Winner {
    fn empty() -> Self {
        Self {
            value: 0.0,
            priority: i32::MIN,
        }
    }

    fn offer(&mut self, value: f32, priority: i32) {
        if value < 0.0 || priority < self.priority {
            return;
        }
        if priority > self.priority {
            self.value = value;
            self.priority = priority;
        } else {
            self.value = self.value.max(value);
        }
    }
}

struct Work<'a> {
    prefab: &'a UiPrefab,
    canvas: Vec2,
    rects: Vec<RectTransform>,
    parents: Vec<Option<usize>>,
    children: Vec<Vec<usize>>,
    active: Vec<bool>,
    plans: Vec<Plan>,
    sizes: Vec<Vec2>,
    metrics: Vec<[Sizes; 2]>,
    totals: Vec<[Sizes; 2]>,
    preorder: Vec<usize>,
    postorder: Vec<usize>,
    changed: Vec<bool>,
}

impl<'a> Work<'a> {
    fn new(
        prefab: &'a UiPrefab,
        canvas: Vec2,
        visible: &HashMap<usize, bool>,
        overrides: &HashMap<usize, RectTransform>,
    ) -> Result<Self, String> {
        if !canvas.is_finite() {
            return Err("non-finite auto-layout canvas".into());
        }
        let count = prefab.nodes.len();
        if visible.keys().chain(overrides.keys()).any(|i| *i >= count) {
            return Err("auto-layout override index is outside the prefab".into());
        }
        let mut ids: HashMap<i64, usize> = HashMap::new();
        let mut parents: Vec<Option<usize>> = Vec::with_capacity(count);
        let mut children: Vec<Vec<usize>> = vec![Vec::new(); count];
        let mut active: Vec<bool> = Vec::with_capacity(count);
        let mut rects = Vec::with_capacity(count);
        let mut plans = Vec::with_capacity(count);
        for (i, node) in prefab.nodes.iter().enumerate() {
            let parent = ids.get(&node.parent_transform_id).copied();
            if i > 0 && parent.is_none() {
                return Err(format!(
                    "auto-layout parent missing or out of order: {}",
                    node.path
                ));
            }
            if ids.insert(node.transform_id, i).is_some() {
                return Err(format!("duplicate auto-layout transform: {}", node.path));
            }
            if let Some(parent) = parent {
                children[parent].push(i);
            }
            let is_active = parent.map(|p| active[p]).unwrap_or(true)
                && visible.get(&i).copied().unwrap_or(node.active);
            let rect = overrides.get(&i).unwrap_or(&node.rect).clone();
            validate_rect(&rect).map_err(|e| format!("{}: {e}", node.path))?;
            parents.push(parent);
            active.push(is_active);
            rects.push(rect);
            plans.push(if is_active {
                parse_plan(&node.components).map_err(|e| format!("{}: {e}", node.path))?
            } else {
                Plan::default()
            });
        }
        let mut preorder = Vec::with_capacity(count);
        let mut postorder = Vec::with_capacity(count);
        let mut stack = Vec::new();
        for i in (0..count).rev() {
            if parents[i].is_none() {
                stack.push((i, false));
            }
        }
        while let Some((i, visited)) = stack.pop() {
            if visited {
                postorder.push(i);
            } else {
                preorder.push(i);
                stack.push((i, true));
                for &child in children[i].iter().rev() {
                    stack.push((child, false));
                }
            }
        }
        Ok(Self {
            prefab,
            canvas,
            rects,
            parents,
            children,
            active,
            plans,
            sizes: vec![Vec2::ZERO; count],
            metrics: vec![[Sizes::default(); 2]; count],
            totals: vec![[Sizes::default(); 2]; count],
            preorder,
            postorder,
            changed: vec![false; count],
        })
    }

    fn parent_size(&self, i: usize) -> Vec2 {
        self.parents[i]
            .map(|p| self.sizes[p])
            .unwrap_or(self.canvas)
    }

    fn refresh_size(&mut self, i: usize) -> Result<(), String> {
        let r = &self.rects[i];
        let size = self.parent_size(i)
            * (Vec2::from_array(r.anchors_max) - Vec2::from_array(r.anchors_min))
            + Vec2::from_array(r.size_delta);
        if !size.is_finite() {
            return Err(format!(
                "non-finite auto-layout size: {}",
                self.prefab.nodes[i].path
            ));
        }
        self.sizes[i] = size;
        Ok(())
    }

    fn refresh_sizes(&mut self) -> Result<(), String> {
        for step in 0..self.preorder.len() {
            self.refresh_size(self.preorder[step])?;
        }
        Ok(())
    }

    fn layout_children(&self, i: usize) -> Vec<usize> {
        self.children[i]
            .iter()
            .copied()
            .filter(|child| self.active[*child] && !self.plans[*child].ignore)
            .collect()
    }

    fn child_sizes(&self, child: usize, axis: usize, group: Group) -> Sizes {
        let mut values = if group.control[axis] {
            self.metrics[child][axis]
        } else {
            let size = self.rects[child].size_delta[axis];
            Sizes {
                min: size,
                preferred: size,
                flexible: 0.0,
            }
        };
        if group.expand[axis] {
            values.flexible = values.flexible.max(1.0);
        }
        values
    }

    fn scale_factor(&self, child: usize, axis: usize, group: Group) -> f32 {
        if group.scale[axis] {
            self.rects[child].local_scale[axis]
        } else {
            1.0
        }
    }

    fn group_sizes(&self, i: usize, axis: usize, group: Group) -> Sizes {
        let padding = group.padding_sum(axis);
        let mut total = Sizes {
            min: padding,
            preferred: padding,
            flexible: 0.0,
        };
        let children = self.layout_children(i);
        for &child in &children {
            let values = self.child_sizes(child, axis, group);
            let scale = self.scale_factor(child, axis, group);
            if group.cross_axis(axis) {
                total.min = total.min.max(values.min * scale + padding);
                total.preferred = total.preferred.max(values.preferred * scale + padding);
                total.flexible = total.flexible.max(values.flexible * scale);
            } else {
                total.min += values.min * scale + group.spacing;
                total.preferred += values.preferred * scale + group.spacing;
                total.flexible += values.flexible * scale;
            }
        }
        if !group.cross_axis(axis) && !children.is_empty() {
            total.min -= group.spacing;
            total.preferred -= group.spacing;
        }
        total.preferred = total.preferred.max(total.min);
        total
    }

    fn measure_axis<F>(&mut self, axis: usize, measure: &mut F) -> Result<(), String>
    where
        F: FnMut(usize, usize, Vec2) -> Option<LayoutMetrics>,
    {
        for step in 0..self.postorder.len() {
            let i = self.postorder[step];
            if !self.active[i] {
                continue;
            }
            let mut min = Winner::empty();
            let mut preferred = Winner::empty();
            let mut flexible = Winner::empty();
            let group_total = self.plans[i]
                .group
                .map(|group| self.group_sizes(i, axis, group))
                .or_else(|| {
                    self.plans[i].grid.map(|grid| {
                        grid.metrics(axis, self.layout_children(i).len(), self.sizes[i])
                    })
                });
            if let Some(total) = group_total {
                if !total.is_finite() {
                    return Err(format!(
                        "non-finite layout group totals: {}",
                        self.prefab.nodes[i].path
                    ));
                }
                self.totals[i][axis] = total;
                min.offer(total.min, 0);
                preferred.offer(total.preferred, 0);
                flexible.offer(total.flexible, 0);
            }
            for element in &self.plans[i].elements {
                let values = element.axes[axis];
                min.offer(values.min, element.priority);
                preferred.offer(values.preferred, element.priority);
                flexible.offer(values.flexible, element.priority);
            }
            // ILayoutElement getters are needed only by a controlling parent or
            // a fitter. Do not demand expensive/content-dependent metrics from
            // ignored or sizeDelta-controlled leaves which no layout reads.
            let measured_by_parent = self.parents[i].is_some_and(|parent| {
                !self.plans[i].ignore
                    && self.plans[parent]
                        .group
                        .is_some_and(|group| group.control[axis])
            });
            let measured_by_self = self.plans[i].fit.is_some_and(|fit| fit[axis] != 0);
            let external = if measured_by_parent || measured_by_self {
                measure(i, axis, self.sizes[i])
            } else {
                None
            };
            if let Some(values) = external {
                if !values.min.is_finite()
                    || !values.preferred.is_finite()
                    || !values.flexible.is_finite()
                {
                    return Err(format!(
                        "non-finite external layout metrics: {}",
                        self.prefab.nodes[i].path
                    ));
                }
                min.offer(values.min, values.priority);
                preferred.offer(values.preferred, values.priority);
                flexible.offer(values.flexible, values.priority);
            }
            self.metrics[i][axis] = Sizes {
                min: min.value,
                preferred: preferred.value.max(min.value),
                flexible: flexible.value,
            };
        }
        Ok(())
    }

    fn control_axis(&mut self, axis: usize) -> Result<(), String> {
        for step in 0..self.preorder.len() {
            let i = self.preorder[step];
            self.refresh_size(i)?;
            if !self.active[i] {
                continue;
            }
            // ILayoutSelfController runs before a group on the same transform.
            if let Some(fit) = self.plans[i].fit {
                if fit[axis] != 0 {
                    let values = self.metrics[i][axis];
                    let size = if fit[axis] == 1 {
                        values.min
                    } else {
                        values.preferred
                    };
                    let parent = self.parent_size(i)[axis];
                    let r = &mut self.rects[i];
                    r.size_delta[axis] =
                        size - parent * (r.anchors_max[axis] - r.anchors_min[axis]);
                    self.changed[i] = true;
                    self.refresh_size(i)?;
                }
            }
            if let Some(group) = self.plans[i].group {
                self.control_group(i, axis, group)?;
            }
            if let Some(grid) = self.plans[i].grid {
                let children = self.layout_children(i);
                if axis == 0 {
                    // Grid's horizontal pass sets both sizes, but no positions.
                    // Vertical text measurement therefore sees the cell width.
                    for child in children {
                        let rect = &mut self.rects[child];
                        rect.anchors_min = [0., 1.];
                        rect.anchors_max = [0., 1.];
                        rect.size_delta = grid.cell.to_array();
                        self.changed[child] = true;
                    }
                } else {
                    let positions = grid.positions(children.len(), self.sizes[i]);
                    for (child, position) in children.into_iter().zip(positions) {
                        // The vertical pass positions both axes without undoing
                        // sizes set by the horizontal layout/self-controller pass.
                        self.set_child(child, 0, position.x, None, 1.)?;
                        self.set_child(child, 1, position.y, None, 1.)?;
                    }
                }
            }
            validate_rect(&self.rects[i])
                .map_err(|e| format!("{}: {e}", self.prefab.nodes[i].path))?;
        }
        Ok(())
    }

    fn control_group(&mut self, i: usize, axis: usize, group: Group) -> Result<(), String> {
        let size = self.sizes[i][axis];
        let alignment = group.alignment(axis);
        let mut children = self.layout_children(i);
        if group.reverse {
            children.reverse();
        }
        if group.cross_axis(axis) {
            let inner = size - group.padding_sum(axis);
            for child in children {
                let values = self.child_sizes(child, axis, group);
                let scale = self.scale_factor(child, axis, group);
                let maximum = if values.flexible > 0.0 {
                    size
                } else {
                    values.preferred
                };
                let required = clamp(inner, values.min, maximum);
                let mut position = group.start_offset(axis, size, required * scale);
                if !group.control[axis] {
                    position += (required - self.rects[child].size_delta[axis]) * alignment;
                }
                self.set_child(
                    child,
                    axis,
                    position,
                    group.control[axis].then_some(required),
                    scale,
                )?;
            }
        } else {
            let total = self.totals[i][axis];
            let surplus = size - total.preferred;
            let mut position = group.leading(axis);
            let mut flexible_multiplier = 0.0;
            if surplus > 0.0 {
                if total.flexible == 0.0 {
                    position =
                        group.start_offset(axis, size, total.preferred - group.padding_sum(axis));
                } else if total.flexible > 0.0 {
                    flexible_multiplier = surplus / total.flexible;
                }
            }
            let ratio = if total.min == total.preferred {
                0.0
            } else {
                clamp((size - total.min) / (total.preferred - total.min), 0.0, 1.0)
            };
            for child in children {
                let values = self.child_sizes(child, axis, group);
                let scale = self.scale_factor(child, axis, group);
                let child_size = values.min
                    + (values.preferred - values.min) * ratio
                    + values.flexible * flexible_multiplier;
                let offset = if group.control[axis] {
                    0.0
                } else {
                    (child_size - self.rects[child].size_delta[axis]) * alignment
                };
                self.set_child(
                    child,
                    axis,
                    position + offset,
                    group.control[axis].then_some(child_size),
                    scale,
                )?;
                position += child_size * scale + group.spacing;
            }
        }
        Ok(())
    }

    fn set_child(
        &mut self,
        child: usize,
        axis: usize,
        position: f32,
        size: Option<f32>,
        scale: f32,
    ) -> Result<(), String> {
        let r = &mut self.rects[child];
        // Both SetChildAlongAxis variants pin both anchor axes to the top-left.
        r.anchors_min = [0.0, 1.0];
        r.anchors_max = [0.0, 1.0];
        if let Some(size) = size {
            r.size_delta[axis] = size;
        }
        r.anchored_position[axis] = if axis == 0 {
            position + r.size_delta[axis] * r.pivot[axis] * scale
        } else {
            -position - r.size_delta[axis] * (1.0 - r.pivot[axis]) * scale
        };
        validate_rect(r).map_err(|e| format!("{}: {e}", self.prefab.nodes[child].path))?;
        self.changed[child] = true;
        Ok(())
    }
}

// Mathf.Clamp's branch order also covers authored inverted ranges.
fn clamp(value: f32, minimum: f32, maximum: f32) -> f32 {
    if value < minimum {
        minimum
    } else if value > maximum {
        maximum
    } else {
        value
    }
}

fn parse_plan(components: &[UiComponent]) -> Result<Plan, String> {
    let mut plan = Plan::default();
    let mut has_ignorer = false;
    let mut all_ignore = true;
    for component in components {
        let class = component
            .class
            .rsplit('.')
            .next()
            .unwrap_or(&component.class);
        let fields = &component.fields;
        if class == "LayoutElement" {
            // LayoutGroup consults ILayoutIgnorer even when its Behaviour is
            // disabled. Its metric properties are filtered separately below.
            has_ignorer = true;
            all_ignore &= boolean(fields, "m_IgnoreLayout")?;
            if component.enabled {
                plan.elements.push(Element {
                    axes: [
                        Sizes {
                            min: scalar(fields, "m_MinWidth")?,
                            preferred: scalar(fields, "m_PreferredWidth")?,
                            flexible: scalar(fields, "m_FlexibleWidth")?,
                        },
                        Sizes {
                            min: scalar(fields, "m_MinHeight")?,
                            preferred: scalar(fields, "m_PreferredHeight")?,
                            flexible: scalar(fields, "m_FlexibleHeight")?,
                        },
                    ],
                    priority: integer(fields, "m_LayoutPriority")?,
                });
            }
        } else if component.enabled
            && (class == "HorizontalLayoutGroup" || class == "VerticalLayoutGroup")
        {
            if plan.group.is_some() || plan.grid.is_some() {
                return Err("multiple enabled layout groups on one node".into());
            }
            let alignment = integer(fields, "m_ChildAlignment")?;
            if !(0..=8).contains(&alignment) {
                return Err("invalid layout child alignment".into());
            }
            plan.group = Some(Group {
                vertical: class == "VerticalLayoutGroup",
                padding: padding(fields)?,
                spacing: scalar(fields, "m_Spacing")?,
                alignment,
                control: [
                    boolean(fields, "m_ChildControlWidth")?,
                    boolean(fields, "m_ChildControlHeight")?,
                ],
                scale: [
                    boolean(fields, "m_ChildScaleWidth")?,
                    boolean(fields, "m_ChildScaleHeight")?,
                ],
                expand: [
                    boolean(fields, "m_ChildForceExpandWidth")?,
                    boolean(fields, "m_ChildForceExpandHeight")?,
                ],
                reverse: boolean(fields, "m_ReverseArrangement")?,
            });
        } else if component.enabled && class == "GridLayoutGroup" {
            if plan.group.is_some() || plan.grid.is_some() {
                return Err("multiple enabled layout groups on one node".into());
            }
            plan.grid = Some(Grid::parse(fields)?);
        } else if component.enabled && class == "ContentSizeFitter" {
            if plan.fit.is_some() {
                return Err("multiple enabled size fitters on one node".into());
            }
            let fit = [
                integer(fields, "m_HorizontalFit")?,
                integer(fields, "m_VerticalFit")?,
            ];
            if fit.iter().any(|v| !(0..=2).contains(v)) {
                return Err("invalid ContentSizeFitter mode".into());
            }
            plan.fit = Some(fit);
        }
    }
    plan.ignore = has_ignorer && all_ignore;
    Ok(plan)
}

fn scalar(fields: &Value, name: &str) -> Result<f32, String> {
    let value = fields
        .get(name)
        .and_then(Value::as_f64)
        .ok_or_else(|| format!("missing numeric layout field {name}"))? as f32;
    if !value.is_finite() {
        return Err(format!("non-finite layout field {name}"));
    }
    Ok(value)
}

fn integer(fields: &Value, name: &str) -> Result<i32, String> {
    fields
        .get(name)
        .and_then(Value::as_i64)
        .and_then(|v| i32::try_from(v).ok())
        .ok_or_else(|| format!("layout field {name} must be an integer"))
}

fn boolean(fields: &Value, name: &str) -> Result<bool, String> {
    match fields.get(name) {
        Some(Value::Bool(value)) => Ok(*value),
        Some(value) if value.as_i64() == Some(0) => Ok(false),
        Some(value) if value.as_i64() == Some(1) => Ok(true),
        _ => Err(format!(
            "layout field {name} must be boolean or integer 0/1"
        )),
    }
}

fn padding(fields: &Value) -> Result<[i32; 4], String> {
    let value = fields
        .get("m_Padding")
        .ok_or("missing layout RectOffset m_Padding")?;
    // Explicit integer forms only. In particular, an old float Vec4 is not
    // reinterpreted as bits or silently rounded to zero.
    if let Some(values) = value.as_array() {
        if values.len() != 4 {
            return Err("layout RectOffset requires four integers".into());
        }
        let mut result = [0; 4];
        for (i, value) in values.iter().enumerate() {
            result[i] = value
                .as_i64()
                .and_then(|v| i32::try_from(v).ok())
                .ok_or_else(|| format!("layout RectOffset item {i} must be an integer"))?;
        }
        return Ok(result);
    }
    let names = if value.get("left").is_some() {
        ["left", "right", "top", "bottom"]
    } else if value.get("m_Left").is_some() {
        ["m_Left", "m_Right", "m_Top", "m_Bottom"]
    } else {
        ["x", "y", "z", "w"]
    };
    Ok([
        integer(value, names[0])?,
        integer(value, names[1])?,
        integer(value, names[2])?,
        integer(value, names[3])?,
    ])
}

fn validate_rect(rect: &RectTransform) -> Result<(), String> {
    if rect
        .anchors_min
        .iter()
        .chain(rect.anchors_max.iter())
        .chain(rect.anchored_position.iter())
        .chain(rect.size_delta.iter())
        .chain(rect.pivot.iter())
        .chain(rect.local_scale.iter())
        .chain(rect.local_rotation.iter())
        .any(|v| !v.is_finite())
    {
        return Err("non-finite auto-layout RectTransform".into());
    }
    Ok(())
}
