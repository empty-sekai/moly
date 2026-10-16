//! Pure calculations for serialized linear/grid layout groups and fitters.
//!
//! Geometry is returned through the same RectTransform overrides used for drawing
//! and hit testing. Text and other external layout elements are measured by the
//! caller. The calculation follows the engine's layout rebuild loop: every
//! enabled layout component marks its layout root when it is enabled; each
//! frame rebuilds the marked roots (calculate horizontal, control horizontal,
//! calculate vertical, control vertical); writes go through the RectTransform
//! setters, whose rect changes send the dimension-change message that marks
//! roots again, for the same frame or, for a root layout group, the next one.
//! The result is the state once no root is left to rebuild.

use super::{RectTransform, UiComponent, UiPrefab};
use bevy::math::Vec2;
use serde_json::Value;
use std::collections::HashMap;

mod grid;
use grid::Grid;

/// Frames the rebuild loop may take before the layout counts as unsettled.
const MAX_FRAMES: usize = 64;

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
/// `measure(node, axis, rect_size)` supplies external layout properties when a
/// layout controller asks for them. Axis 0 is horizontal, 1 is vertical. For a
/// text element the size is its rect as of its last margin update (the text
/// ignores rect changes below 0.0001 in both axes), otherwise the current rect.
/// `None` means no external layout element.
///
/// Explicit overrides are the starting geometry, and `visible` the activity the
/// hierarchy is enabled with; layout controllers can overwrite their driven
/// properties, just as they overwrite serialized geometry. Inactive hierarchies
/// are not driven. Other component families remain uninterpreted.
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
    work.run(&mut measure)?;
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
    // RectOffset.horizontal / vertical: an integer sum, converted where used.
    fn padding_sum(self, axis: usize) -> f32 {
        self.padding[axis * 2].wrapping_add(self.padding[axis * 2 + 1]) as f32
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

    // LayoutGroup.GetStartOffset: the padding joins the required space
    // before the surplus is taken from the group's own size.
    fn start_offset(self, axis: usize, size: f32, required_without_padding: f32) -> f32 {
        let required = required_without_padding + self.padding_sum(axis);
        let surplus = size - required;
        self.leading(axis) + surplus * self.alignment(axis)
    }
}

/// The layout-relevant components of one node. `group_component` counts a
/// layout group in any state (the engine's component lookups for layout
/// groups do not filter by enabled state); every other entry is an enabled
/// component, and is only parsed for nodes active in the hierarchy.
#[derive(Default)]
struct Plan {
    group: Option<Group>,
    grid: Option<Grid>,
    fit: Option<[i32; 2]>,
    elements: Vec<Element>,
    ignore: bool,
    group_component: bool,
    /// An enabled Image or text component: an external ILayoutElement.
    external: bool,
    /// An enabled text component, whose rect-change handling keeps its own
    /// margin rect.
    text: bool,
    /// Enabled scroll rects and aspect ratio fitters: they take part in the
    /// rebuild walk (ScrollRect as a layout group and element, both as
    /// controllers) but their own layout is not interpreted.
    uninterpreted_group: bool,
    uninterpreted_controller: bool,
}

impl Plan {
    fn enabled_group(&self) -> bool {
        self.group.is_some() || self.grid.is_some() || self.uninterpreted_group
    }

    fn enabled_controller(&self) -> bool {
        self.fit.is_some() || self.enabled_group() || self.uninterpreted_controller
    }

    fn enabled_element(&self) -> bool {
        !self.elements.is_empty() || self.enabled_group() || self.external
    }
}

/// The priority of one winning property, not of an entire element tuple
/// (LayoutUtility.GetLayoutProperty).
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
        if priority < self.priority || value < 0.0 {
            return;
        }
        if priority > self.priority {
            self.value = value;
            self.priority = priority;
        } else if value > self.value {
            self.value = value;
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
    /// The solved rect size of every node.
    sizes: Vec<Vec2>,
    /// Layout group totals from each group's last calculation.
    totals: Vec<[Sizes; 2]>,
    /// A text element's rect size and pivot as of its last margin update.
    text_rects: Vec<Option<(Vec2, [f32; 2])>>,
    external: HashMap<(usize, usize, [u32; 2]), Option<LayoutMetrics>>,
    /// Layout roots marked for the rebuild, in marking order.
    queue: Vec<usize>,
    /// Root layout groups whose rect changed during a rebuild: marked when
    /// the next frame starts.
    delayed: Vec<usize>,
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
                Plan {
                    group_component: has_group_component(&node.components),
                    ..Plan::default()
                }
            });
        }
        let mut work = Self {
            prefab,
            canvas,
            rects,
            parents,
            children,
            active,
            plans,
            sizes: vec![Vec2::ZERO; count],
            totals: vec![[Sizes::default(); 2]; count],
            text_rects: vec![None; count],
            external: HashMap::new(),
            queue: Vec::new(),
            delayed: Vec::new(),
            changed: vec![false; count],
        };
        // Parents precede their children, so one pass solves every rect.
        for i in 0..count {
            let size = work.solved_size(i);
            if !size.is_finite() {
                return Err(format!(
                    "non-finite auto-layout size: {}",
                    prefab.nodes[i].path
                ));
            }
            work.sizes[i] = size;
        }
        Ok(work)
    }

    fn parent_size(&self, i: usize) -> Vec2 {
        self.parents[i]
            .map(|p| self.sizes[p])
            .unwrap_or(self.canvas)
    }

    fn solved_size(&self, i: usize) -> Vec2 {
        let parent_pivot = self.parents[i]
            .map(|p| Vec2::from_array(self.rects[p].pivot))
            .unwrap_or(Vec2::splat(0.5));
        super::rect_size(self.parent_size(i), parent_pivot, &self.rects[i])
    }

    // ------------------------------------------------------------ the rebuild loop

    fn run<F>(&mut self, measure: &mut F) -> Result<(), String>
    where
        F: FnMut(usize, usize, Vec2) -> Option<LayoutMetrics>,
    {
        self.enable();
        for _ in 0..MAX_FRAMES {
            // A delayed root group marks itself when the next frame starts.
            for root in std::mem::take(&mut self.delayed) {
                self.mark(root);
            }
            if self.queue.is_empty() {
                return Ok(());
            }
            self.rebuild_queue(measure)?;
        }
        Err(format!(
            "layout rebuilds still pending after {MAX_FRAMES} frames"
        ))
    }

    /// OnEnable of the enabled layout components, in hierarchy order: the
    /// groups, fitters, layout elements and graphics mark their layout root;
    /// a text element first takes its margins from its rect.
    fn enable(&mut self) {
        for i in 0..self.prefab.nodes.len() {
            if !self.active[i] {
                continue;
            }
            if self.plans[i].text {
                self.text_rects[i] = Some((self.sizes[i], self.rects[i].pivot));
            }
            if self.plans[i].enabled_controller() || self.plans[i].enabled_element() {
                self.mark(i);
            }
        }
    }

    /// LayoutRebuilder.MarkLayoutForRebuild: the root is the topmost node
    /// reached through parents that carry an active and enabled layout group;
    /// a node that is its own root needs an active and enabled controller. A
    /// root already in the queue is not queued again.
    fn mark(&mut self, rect: usize) {
        let mut root = rect;
        let mut parent = self.parents[rect];
        while let Some(p) = parent {
            if !(self.active[p] && self.plans[p].enabled_group()) {
                break;
            }
            root = p;
            parent = self.parents[p];
        }
        if root == rect && !(self.active[rect] && self.plans[rect].enabled_controller()) {
            return;
        }
        if !self.queue.contains(&root) {
            self.queue.push(root);
        }
    }

    fn depth(&self, mut i: usize) -> usize {
        let mut depth = 0;
        while let Some(p) = self.parents[i] {
            depth += 1;
            i = p;
        }
        depth
    }

    /// CanvasUpdateRegistry's layout update: the queue sorted by depth, then
    /// every entry rebuilt, including roots marked while it runs; roots
    /// marked again once rebuilt stay dropped until the queue is cleared.
    fn rebuild_queue<F>(&mut self, measure: &mut F) -> Result<(), String>
    where
        F: FnMut(usize, usize, Vec2) -> Option<LayoutMetrics>,
    {
        let mut queue = std::mem::take(&mut self.queue);
        queue.sort_by_key(|&root| self.depth(root));
        self.queue = queue;
        let mut j = 0;
        while j < self.queue.len() {
            let root = self.queue[j];
            // LayoutRebuilder.Rebuild.
            for axis in 0..2 {
                self.calculate(root, axis, measure)?;
                self.control(root, axis, measure)?;
            }
            j += 1;
        }
        self.queue.clear();
        Ok(())
    }

    /// PerformLayoutCalculation: children first, through nodes with an
    /// enabled layout element or with a layout group in any state.
    fn calculate<F>(&mut self, i: usize, axis: usize, measure: &mut F) -> Result<(), String>
    where
        F: FnMut(usize, usize, Vec2) -> Option<LayoutMetrics>,
    {
        let elements = self.active[i] && self.plans[i].enabled_element();
        if !elements && !self.plans[i].group_component {
            return Ok(());
        }
        for k in 0..self.children[i].len() {
            self.calculate(self.children[i][k], axis, measure)?;
        }
        if !elements {
            return Ok(());
        }
        let total = if let Some(group) = self.plans[i].group {
            Some(self.group_sizes(i, axis, group, measure)?)
        } else {
            self.plans[i]
                .grid
                .map(|grid| grid.metrics(axis, self.layout_children(i).len(), self.sizes[i]))
        };
        if let Some(total) = total {
            if !total.is_finite() {
                return Err(format!(
                    "non-finite layout group totals: {}",
                    self.prefab.nodes[i].path
                ));
            }
            self.totals[i][axis] = total;
        }
        Ok(())
    }

    /// PerformLayoutControl: parents first, self-controllers before the
    /// groups, and only through nodes with an enabled controller.
    fn control<F>(&mut self, i: usize, axis: usize, measure: &mut F) -> Result<(), String>
    where
        F: FnMut(usize, usize, Vec2) -> Option<LayoutMetrics>,
    {
        if !(self.active[i] && self.plans[i].enabled_controller()) {
            return Ok(());
        }
        if let Some(fit) = self.plans[i].fit {
            // ContentSizeFitter.HandleSelfFittingAlongAxis, then
            // RectTransform.SetSizeWithCurrentAnchors through the setter.
            if fit[axis] != 0 {
                let values = self.layout_sizes(i, axis, measure)?;
                let size = if fit[axis] == 1 {
                    values.min
                } else {
                    values.preferred
                };
                let parent = self.parent_size(i)[axis];
                let r = &self.rects[i];
                let mut size_delta = r.size_delta;
                size_delta[axis] = size - parent * (r.anchors_max[axis] - r.anchors_min[axis]);
                self.write(i, Field::SizeDelta, size_delta)?;
            }
        }
        if let Some(group) = self.plans[i].group {
            self.control_group(i, axis, group, measure)?;
        }
        if let Some(grid) = self.plans[i].grid {
            let children = self.layout_children(i);
            if axis == 0 {
                // Grid's horizontal pass sets the anchors and sizes, no positions.
                for child in children {
                    self.write(child, Field::AnchorMin, [0., 1.])?;
                    self.write(child, Field::AnchorMax, [0., 1.])?;
                    self.write(child, Field::SizeDelta, grid.cell.to_array())?;
                }
            } else {
                let positions = grid.positions(children.len(), self.sizes[i]);
                for (child, position) in children.into_iter().zip(positions) {
                    // The vertical pass places both axes with the sized
                    // placement, so each child is set to the cell size again.
                    self.set_child(child, 0, position.x, Some(grid.cell.x), 1.)?;
                    self.set_child(child, 1, position.y, Some(grid.cell.y), 1.)?;
                }
            }
        }
        for k in 0..self.children[i].len() {
            self.control(self.children[i][k], axis, measure)?;
        }
        Ok(())
    }

    // ------------------------------------------------------------ writes and rect changes

    /// A RectTransform property written through the engine's setter: stored
    /// only beyond its 10-ulp comparison, then the node's rect solved again;
    /// every node whose rect changed (children first) gets the dimension
    /// message.
    fn write(&mut self, i: usize, field: Field, value: [f32; 2]) -> Result<(), String> {
        let r = &mut self.rects[i];
        let slot = match field {
            Field::AnchorMin => &mut r.anchors_min,
            Field::AnchorMax => &mut r.anchors_max,
            Field::AnchoredPosition => &mut r.anchored_position,
            Field::SizeDelta => &mut r.size_delta,
        };
        if !store(slot, value) {
            return Ok(());
        }
        let valid = validate_rect(&self.rects[i]);
        valid.map_err(|e| format!("{}: {e}", self.prefab.nodes[i].path))?;
        self.changed[i] = true;
        let mut messages = Vec::new();
        self.solve_rects(i, &mut messages)?;
        for node in messages {
            self.dimensions_changed(node);
        }
        Ok(())
    }

    /// The native rect update: a node whose rect is unchanged stops the walk;
    /// a changed one updates its children first, then is reported.
    fn solve_rects(&mut self, i: usize, messages: &mut Vec<usize>) -> Result<(), String> {
        let size = self.solved_size(i);
        if !size.is_finite() {
            return Err(format!(
                "non-finite auto-layout size: {}",
                self.prefab.nodes[i].path
            ));
        }
        if size.x == self.sizes[i].x && size.y == self.sizes[i].y {
            return Ok(());
        }
        self.sizes[i] = size;
        for k in 0..self.children[i].len() {
            self.solve_rects(self.children[i][k], messages)?;
        }
        messages.push(i);
        Ok(())
    }

    /// OnRectTransformDimensionsChange on the node's components, during a
    /// rebuild: a fitter or scroll rect marks its root; a root layout group (no layout group
    /// in any state on its parent) marks itself when the next frame starts; a
    /// graphic only dirties its vertices; a text element ignores changes
    /// below 0.0001 in rect size and pivot, else updates its margins and marks
    /// its root.
    fn dimensions_changed(&mut self, i: usize) {
        if !self.active[i] {
            return;
        }
        // A fitter, and a scroll rect (not otherwise interpreted), mark
        // their root.
        if self.plans[i].fit.is_some() || self.plans[i].uninterpreted_group {
            self.mark(i);
        }
        let group = self.plans[i].group.is_some() || self.plans[i].grid.is_some();
        let root_group = self.parents[i].is_none_or(|p| !self.plans[p].group_component);
        if group && root_group {
            self.delayed.push(i);
        }
        if self.plans[i].text {
            let (size, pivot) = (self.sizes[i], self.rects[i].pivot);
            let (last, last_pivot) = self.text_rects[i].expect("enabled text has a margin rect");
            let small = |a: f32, b: f32| (a - b).abs() < 0.0001;
            if small(size.x, last.x)
                && small(size.y, last.y)
                && small(pivot[0], last_pivot[0])
                && small(pivot[1], last_pivot[1])
            {
                return;
            }
            self.text_rects[i] = Some((size, pivot));
            self.mark(i);
        }
    }

    // ------------------------------------------------------------ LayoutUtility

    fn layout_children(&self, i: usize) -> Vec<usize> {
        self.children[i]
            .iter()
            .copied()
            .filter(|child| self.active[*child] && !self.plans[*child].ignore)
            .collect()
    }

    /// LayoutUtility.GetMinSize / GetPreferredSize / GetFlexibleSize of a
    /// node, over its enabled layout elements at the time of the query.
    fn layout_sizes<F>(&mut self, i: usize, axis: usize, measure: &mut F) -> Result<Sizes, String>
    where
        F: FnMut(usize, usize, Vec2) -> Option<LayoutMetrics>,
    {
        let mut min = Winner::empty();
        let mut preferred = Winner::empty();
        let mut flexible = Winner::empty();
        if self.active[i] {
            if self.plans[i].group.is_some() || self.plans[i].grid.is_some() {
                let total = self.totals[i][axis];
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
            if self.plans[i].external {
                let size = self.text_rects[i].map_or(self.sizes[i], |(size, _)| size);
                let key = (i, axis, [size.x.to_bits(), size.y.to_bits()]);
                let values = match self.external.get(&key) {
                    Some(values) => *values,
                    None => {
                        let values = measure(i, axis, size);
                        self.external.insert(key, values);
                        values
                    }
                };
                if let Some(values) = values {
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
            }
        }
        Ok(Sizes {
            min: min.value,
            // Mathf.Max(min, preferred).
            preferred: if min.value > preferred.value {
                min.value
            } else {
                preferred.value
            },
            flexible: flexible.value,
        })
    }

    /// HorizontalOrVerticalLayoutGroup.GetChildSizes.
    fn child_sizes<F>(
        &mut self,
        child: usize,
        axis: usize,
        group: Group,
        measure: &mut F,
    ) -> Result<Sizes, String>
    where
        F: FnMut(usize, usize, Vec2) -> Option<LayoutMetrics>,
    {
        let mut values = if group.control[axis] {
            self.layout_sizes(child, axis, measure)?
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
        Ok(values)
    }

    fn scale_factor(&self, child: usize, axis: usize, group: Group) -> f32 {
        if group.scale[axis] {
            self.rects[child].local_scale[axis]
        } else {
            1.0
        }
    }

    /// HorizontalOrVerticalLayoutGroup.CalcAlongAxis.
    fn group_sizes<F>(
        &mut self,
        i: usize,
        axis: usize,
        group: Group,
        measure: &mut F,
    ) -> Result<Sizes, String>
    where
        F: FnMut(usize, usize, Vec2) -> Option<LayoutMetrics>,
    {
        let padding = group.padding_sum(axis);
        let mut total = Sizes {
            min: padding,
            preferred: padding,
            flexible: 0.0,
        };
        let children = self.layout_children(i);
        for &child in &children {
            let values = self.child_sizes(child, axis, group, measure)?;
            let (mut min, mut preferred, mut flexible) =
                (values.min, values.preferred, values.flexible);
            if group.scale[axis] {
                let scale = self.rects[child].local_scale[axis];
                min *= scale;
                preferred *= scale;
                flexible *= scale;
            }
            if group.cross_axis(axis) {
                // Mathf.Max(child + padding, total).
                let (min, preferred) = (min + padding, preferred + padding);
                if min > total.min {
                    total.min = min;
                }
                if preferred > total.preferred {
                    total.preferred = preferred;
                }
                if flexible > total.flexible {
                    total.flexible = flexible;
                }
            } else {
                total.min += min + group.spacing;
                total.preferred += preferred + group.spacing;
                total.flexible += flexible;
            }
        }
        if !group.cross_axis(axis) && !children.is_empty() {
            total.min -= group.spacing;
            total.preferred -= group.spacing;
        }
        // Mathf.Max(totalMin, totalPreferred).
        if total.min > total.preferred {
            total.preferred = total.min;
        }
        Ok(total)
    }

    /// HorizontalOrVerticalLayoutGroup.SetChildrenAlongAxis.
    fn control_group<F>(
        &mut self,
        i: usize,
        axis: usize,
        group: Group,
        measure: &mut F,
    ) -> Result<(), String>
    where
        F: FnMut(usize, usize, Vec2) -> Option<LayoutMetrics>,
    {
        let size = self.sizes[i][axis];
        let alignment = group.alignment(axis);
        let mut children = self.layout_children(i);
        if group.reverse {
            children.reverse();
        }
        if group.cross_axis(axis) {
            let inner = size - group.padding_sum(axis);
            for child in children {
                let values = self.child_sizes(child, axis, group, measure)?;
                let scale = self.scale_factor(child, axis, group);
                let maximum = if values.flexible > 0.0 {
                    size
                } else {
                    values.preferred
                };
                let required = clamp(inner, values.min, maximum);
                let start = group.start_offset(axis, size, required * scale);
                if group.control[axis] {
                    self.set_child(child, axis, start, Some(required), scale)?;
                } else {
                    let offset = (required - self.rects[child].size_delta[axis]) * alignment;
                    self.set_child(child, axis, start + offset, None, scale)?;
                }
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
                let values = self.child_sizes(child, axis, group, measure)?;
                let scale = self.scale_factor(child, axis, group);
                let child_size = values.min
                    + (values.preferred - values.min) * ratio
                    + values.flexible * flexible_multiplier;
                if group.control[axis] {
                    self.set_child(child, axis, position, Some(child_size), scale)?;
                } else {
                    let offset = (child_size - self.rects[child].size_delta[axis]) * alignment;
                    self.set_child(child, axis, position + offset, None, scale)?;
                }
                position += child_size * scale + group.spacing;
            }
        }
        Ok(())
    }

    /// LayoutGroup.SetChildAlongAxisWithScale, both variants: the anchors
    /// pinned to the top-left, the size (sized variant) and the position
    /// written through the setters. The sized variant places the child by the
    /// size it was given, which the sizeDelta setter may have kept at its old
    /// value; the unsized variant reads the child's current sizeDelta.
    fn set_child(
        &mut self,
        child: usize,
        axis: usize,
        position: f32,
        size: Option<f32>,
        scale: f32,
    ) -> Result<(), String> {
        self.write(child, Field::AnchorMin, [0.0, 1.0])?;
        self.write(child, Field::AnchorMax, [0.0, 1.0])?;
        let placed = match size {
            Some(size) => {
                let mut size_delta = self.rects[child].size_delta;
                size_delta[axis] = size;
                self.write(child, Field::SizeDelta, size_delta)?;
                size
            }
            None => self.rects[child].size_delta[axis],
        };
        let r = &self.rects[child];
        let mut anchored_position = r.anchored_position;
        anchored_position[axis] = if axis == 0 {
            position + placed * r.pivot[axis] * scale
        } else {
            -position - placed * (1.0 - r.pivot[axis]) * scale
        };
        self.write(child, Field::AnchoredPosition, anchored_position)
    }
}

#[derive(Clone, Copy)]
enum Field {
    AnchorMin,
    AnchorMax,
    AnchoredPosition,
    SizeDelta,
}

/// A Vector2 RectTransform property write as the engine's setter makes it:
/// the new value is kept only when a component differs from the current one
/// by more than 10 units in the last place (components of different sign
/// compare with `==`, so +0 and -0 are the same value). Returns whether the
/// value was stored.
fn store(field: &mut [f32; 2], value: [f32; 2]) -> bool {
    if within_ten_ulps(field[0], value[0]) && within_ten_ulps(field[1], value[1]) {
        return false;
    }
    *field = value;
    true
}

fn within_ten_ulps(a: f32, b: f32) -> bool {
    let (x, y) = (a.to_bits() as i32, b.to_bits() as i32);
    if (x ^ y) < 0 {
        return a == b;
    }
    let ordered = |v: i32| if v < 0 { i32::MIN.wrapping_sub(v) } else { v };
    ordered(x).wrapping_sub(ordered(y)).wrapping_abs() <= 10
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

fn short_class(component: &UiComponent) -> &str {
    component
        .class
        .rsplit('.')
        .next()
        .unwrap_or(&component.class)
}

fn is_scroll_rect(class: &str) -> bool {
    class.ends_with("ScrollRect")
}

/// A layout group component in any enabled state.
fn has_group_component(components: &[UiComponent]) -> bool {
    components.iter().any(|component| {
        let class = short_class(component);
        matches!(
            class,
            "HorizontalLayoutGroup" | "VerticalLayoutGroup" | "GridLayoutGroup"
        ) || is_scroll_rect(class)
    })
}

fn parse_plan(components: &[UiComponent]) -> Result<Plan, String> {
    let mut plan = Plan {
        group_component: has_group_component(components),
        ..Plan::default()
    };
    let mut has_ignorer = false;
    let mut all_ignore = true;
    for component in components {
        let class = short_class(component);
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
        } else if component.enabled && is_scroll_rect(class) {
            plan.uninterpreted_group = true;
            plan.uninterpreted_controller = true;
        } else if component.enabled && class == "AspectRatioFitter" {
            plan.uninterpreted_controller = true;
        } else if component.enabled && fields.get("m_fontSize").is_some() {
            // The external elements the host measures: text, then Image.
            plan.external = true;
            plan.text = true;
        } else if component.enabled && fields.get("m_Type").is_some() {
            plan.external = true;
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
