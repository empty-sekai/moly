//! Runtime instances of extracted prefab subtrees.
//!
//! Instances keep the template's geometry and component data. This module does
//! not add layout controllers or interpret a custom ListView. Its host sets the
//! driven RectTransforms using the real controller's layout operation.

use super::{UiComponent, UiPrefab, UiPrefabSource};
use serde_json::Value;
use std::collections::{HashMap, HashSet};

/// Stable identity bindings for one instantiated subtree. Node indices are not
/// bindings: a later sibling insertion may move them in the painter's order.
#[derive(Debug, Clone)]
pub struct UiInstance {
    pub root_transform_id: i64,
    pub identities: HashMap<i64, i64>,
}

impl UiInstance {
    /// Resolve an original GameObject, Transform or component path ID.
    pub fn identity(&self, source_id: i64) -> Result<i64, String> {
        self.identities
            .get(&source_id)
            .copied()
            .ok_or_else(|| format!("UI identity {source_id} is outside this instance"))
    }

    /// An unambiguous selector accepted by UiPrefab::find and UiPrefabView.
    pub fn selector(&self, source_id: i64) -> Result<String, String> {
        self.identity(source_id).map(|id| format!("@{id}"))
    }

    pub fn root_selector(&self) -> String {
        format!("@{}", self.root_transform_id)
    }
}

impl UiPrefab {
    /// Clone a complete template subtree under an existing RectTransform.
    ///
    /// `parent` and `source_root` accept the same suffix / @identity selectors
    /// as `find`. `instance_name` is one unique path segment under `parent`.
    /// The root takes that runtime name; all descendant names are unchanged.
    ///
    /// The clone is the last child of `parent`, before the parent's following
    /// siblings. Node storage is rebuilt in depth-first hierarchy order while
    /// retaining sibling order, so the renderer and layout engine agree.
    /// Existing object IDs never change; returned bindings remain valid after
    /// subsequent insertions. No partial mutation is published on error.
    ///
    /// `pointerFields`, emitted by the prefab producer from actual PPtr types,
    /// is required for non-null component fields. Integer pairs alone are not
    /// pointers. Only fileId=0 pointers whose target belongs to the cloned
    /// subtree are remapped; external, null and non-cloned references retain
    /// their original representation and value. Sprite/texture metadata is not
    /// a cloned Unity object and is retained verbatim.
    pub fn instantiate_subtree(
        &mut self,
        parent: &str,
        template: &UiPrefab,
        source_root: &str,
        instance_name: &str,
    ) -> Result<UiInstance, String> {
        self.instantiate_children(parent, template, source_root, [instance_name])
            .map(|mut instances| instances.remove(0))
    }

    /// Batch form for list population. Reserves IDs and rebuilds the destination
    /// indices once, not once per cell. Subsequent selection/scroll updates
    /// should modify the owning view's overrides, not reinstantiate the list.
    pub fn instantiate_children<'a>(
        &mut self,
        parent: &str,
        template: &UiPrefab,
        source_root: &str,
        instance_names: impl IntoIterator<Item = &'a str>,
    ) -> Result<Vec<UiInstance>, String> {
        let names: Vec<_> = instance_names.into_iter().collect();
        if names.is_empty() {
            return Ok(Vec::new());
        }
        let parent_index = self.find(parent)?;
        let source_index = template.find(source_root)?;
        let parent_id = self.nodes[parent_index].transform_id;
        let mut occupied: HashSet<_> = self.nodes.iter().map(|node| node.path.clone()).collect();
        let mut instance_paths = Vec::with_capacity(names.len());
        for &name in &names {
            if name.is_empty() || name.contains('/') {
                return Err("UI instance name must be one non-empty path segment".into());
            }
            let path = format!("{}/{}", self.nodes[parent_index].path, name);
            if !occupied.insert(path.clone()) {
                return Err(format!("UI instance path already exists: {path}"));
            }
            instance_paths.push(path);
        }
        let source_order = template.subtree_order(source_index);
        let mut reserved = HashSet::new();
        reserve_document_ids(self, &mut reserved);
        reserve_document_ids(template, &mut reserved);
        let mut originals = Vec::new();
        let mut original_set = HashSet::new();
        for &index in &source_order {
            let node = &template.nodes[index];
            for original in [node.game_object_id, node.transform_id]
                .into_iter()
                .chain(node.components.iter().map(|component| component.path_id))
            {
                if original == 0 || !original_set.insert(original) {
                    return Err(format!(
                        "UI subtree has null or duplicate object identity {original}: {}",
                        node.path
                    ));
                }
                originals.push(original);
            }
        }

        let mut next_id = -1_i64;
        let mut nodes = Vec::with_capacity(source_order.len() * names.len());
        let mut instances = Vec::with_capacity(names.len());
        let source_path = &template.nodes[source_index].path;
        for (name, instance_path) in names.into_iter().zip(instance_paths) {
            let mut identities = HashMap::with_capacity(originals.len());
            for &original in &originals {
                while reserved.contains(&next_id) {
                    next_id = next_id
                        .checked_sub(1)
                        .ok_or("UI runtime identity space exhausted")?;
                }
                identities.insert(original, next_id);
                reserved.insert(next_id);
            }
            for &index in &source_order {
                let source = &template.nodes[index];
                let mut node = source.clone();
                node.game_object_id = identities[&source.game_object_id];
                node.transform_id = identities[&source.transform_id];
                if index == source_index {
                    node.parent_transform_id = parent_id;
                    node.path = instance_path.clone();
                    node.name = name.to_owned();
                } else {
                    node.parent_transform_id = *identities
                        .get(&source.parent_transform_id)
                        .ok_or_else(|| format!("UI clone parent missing: {}", source.path))?;
                    let suffix = source
                        .path
                        .strip_prefix(source_path)
                        .filter(|suffix| suffix.starts_with('/'))
                        .ok_or_else(|| {
                            format!("UI clone path is not below its root: {}", source.path)
                        })?;
                    node.path = format!("{instance_path}{suffix}");
                }
                for component in &mut node.components {
                    remap_component(component, &identities)?;
                    component.path_id = identities[&component.path_id];
                }
                nodes.push(node);
            }
            instances.push(UiInstance {
                root_transform_id: identities[&template.nodes[source_index].transform_id],
                identities,
            });
        }

        let order = self.subtree_order(0);
        if order.len() != self.nodes.len() {
            return Err("runtime UI destination must have a single hierarchy root".into());
        }
        let parent_position = order
            .iter()
            .position(|&i| i == parent_index)
            .ok_or("UI destination parent is outside its root")?;
        let insert_at = parent_position + self.subtree_order(parent_index).len();
        let mut combined = Vec::with_capacity(self.nodes.len() + nodes.len());
        combined.extend(order[..insert_at].iter().map(|&i| self.nodes[i].clone()));
        combined.extend(nodes);
        combined.extend(order[insert_at..].iter().map(|&i| self.nodes[i].clone()));
        let updated = UiPrefab::try_from(UiPrefabSource {
            version: self.version,
            prefab: self.prefab.clone(),
            nodes: combined,
        })?;
        *self = updated;
        Ok(instances)
    }

    /// Transform hierarchy traversal, not a path-prefix guess. Child ordering
    /// is the serialized ordering already used by auto_layout's child lists.
    fn subtree_order(&self, root: usize) -> Vec<usize> {
        let mut children = vec![Vec::new(); self.nodes.len()];
        for (child, parent) in self.indices.parents.iter().enumerate() {
            if let Some(parent) = parent {
                children[*parent].push(child);
            }
        }
        let mut order = Vec::new();
        let mut pending = vec![root];
        while let Some(index) = pending.pop() {
            order.push(index);
            pending.extend(children[index].iter().rev().copied());
        }
        order
    }
}

fn remap_component(
    component: &mut UiComponent,
    identities: &HashMap<i64, i64>,
) -> Result<(), String> {
    let Some(paths) = component.pointer_fields.as_ref() else {
        // A preserved opaque/partial component has no fields to reinterpret.
        // This is not a claim that its missing behaviour has been implemented.
        if component.fields.is_null() {
            return Ok(());
        }
        return Err(format!(
            "runtime UI {} @{} needs producer pointerFields metadata",
            component.class, component.path_id
        ));
    };
    let mut seen = HashSet::new();
    for path in paths {
        if !seen.insert(path) {
            return Err(format!("duplicate UI pointerFields entry {path}"));
        }
        let pointer = component.fields.pointer_mut(path).ok_or_else(|| {
            format!(
                "UI {} @{} pointerFields path missing: {path}",
                component.class, component.path_id
            )
        })?;
        let (file_id, path_id) = read_pointer(pointer).ok_or_else(|| {
            format!(
                "UI {} @{} pointerFields value is not a PPtr: {path}",
                component.class, component.path_id
            )
        })?;
        if file_id != 0 || path_id == 0 {
            continue;
        }
        if let Some(&id) = identities.get(&path_id) {
            match pointer {
                Value::Array(pair) => pair[1] = Value::from(id),
                Value::Object(object) => {
                    // read_pointer accepted only this exact typed key pair.
                    object.insert("m_PathID".into(), Value::from(id));
                }
                _ => unreachable!("read_pointer accepted a non-PPtr"),
            }
        }
    }
    Ok(())
}

fn read_pointer(value: &Value) -> Option<(i64, i64)> {
    match value {
        Value::Array(pair) if pair.len() == 2 => Some((pair[0].as_i64()?, pair[1].as_i64()?)),
        Value::Object(object) => Some((
            object.get("m_FileID")?.as_i64()?,
            object.get("m_PathID")?.as_i64()?,
        )),
        _ => None,
    }
}

fn reserve_document_ids(document: &UiPrefab, ids: &mut HashSet<i64>) {
    ids.insert(0);
    for node in &document.nodes {
        ids.extend([
            node.game_object_id,
            node.transform_id,
            node.parent_transform_id,
        ]);
        for component in &node.components {
            ids.insert(component.path_id);
            if let Some(paths) = &component.pointer_fields {
                for path in paths {
                    if let Some((_, path_id)) =
                        component.fields.pointer(path).and_then(read_pointer)
                    {
                        // Also avoid non-cloned same-file targets, so an old
                        // unresolved pointer cannot accidentally select a clone.
                        ids.insert(path_id);
                    }
                }
            }
        }
    }
}
