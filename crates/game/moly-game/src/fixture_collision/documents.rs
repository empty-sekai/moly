//! GLB v4 keeps collision geometry on one asset-only node. Scene roots reference
//! that node without instantiating or cloning its large extras on every fixture.
use super::{Document, Node, validate_contract};
use bevy::{
    asset::AssetId,
    gltf::{Gltf, GltfNode},
    prelude::*,
};
use serde::Deserialize;
use std::{collections::HashMap, sync::Arc};

pub(super) const METADATA_NODE: &str = "__moly_fixture_collision";

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Reference {
    schema_version: u32,
    node: usize,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct CollisionExtras {
    pub fixture_collision: Option<Document>,
    pub fixture_collision_ref: Option<Reference>,
    pub source_collision: Option<Node>,
}

pub(super) fn validate(document: &Document) -> Result<(), String> {
    validate_contract(document.schema_version, &document.coordinate_contract)?;
    if document.units != "source-unity-unit" {
        return Err("fixture collision unit contract unsupported".into());
    }
    if !document.gaps.is_empty() {
        return Err(format!(
            "fixture collision extraction has {} unresolved inputs",
            document.gaps.len()
        ));
    }
    Ok(())
}

pub(super) fn resolve(
    reference: &Reference,
    owner: &crate::fixture::FixtureSource,
    gltfs: &Assets<Gltf>,
    nodes: &Assets<GltfNode>,
    cache: &mut HashMap<AssetId<GltfNode>, Arc<Document>>,
) -> Result<Arc<Document>, String> {
    if reference.schema_version != 1 {
        return Err("fixture collision reference schema unsupported".into());
    }
    let gltf = gltfs
        .get(&owner.0)
        .ok_or("fixture collision source still loading")?;
    let handle = gltf
        .nodes
        .get(reference.node)
        .ok_or("fixture collision reference node out of range")?;
    if let Some(document) = cache.get(&handle.id()) {
        return Ok(Arc::clone(document));
    }
    let node = nodes
        .get(handle)
        .ok_or("fixture collision metadata node still loading")?;
    if node.index != reference.node
        || node.name != METADATA_NODE
        || node.mesh.is_some()
        || node.skin.is_some()
        || !node.children.is_empty()
        || node.transform != Transform::IDENTITY
    {
        return Err("fixture collision reference must name its isolated metadata node".into());
    }
    let extras = node
        .extras
        .as_ref()
        .ok_or("fixture collision metadata extras missing")?;
    let extras: CollisionExtras = serde_json::from_str(&extras.value)
        .map_err(|error| format!("fixture collision metadata: {error}"))?;
    if extras.fixture_collision_ref.is_some() || extras.source_collision.is_some() {
        return Err(
            "fixture collision metadata node must not reference another node or collider".into(),
        );
    }
    let document = extras
        .fixture_collision
        .ok_or("fixture collision referenced document missing")?;
    validate(&document)?;
    let document = Arc::new(document);
    cache.insert(handle.id(), Arc::clone(&document));
    Ok(document)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::gltf::GltfExtras;

    fn source(nodes: Vec<Handle<GltfNode>>) -> Gltf {
        Gltf {
            nodes,
            scenes: vec![],
            named_scenes: default(),
            meshes: vec![],
            named_meshes: default(),
            materials: vec![],
            named_materials: default(),
            named_nodes: default(),
            skins: vec![],
            named_skins: default(),
            default_scene: None,
            animations: vec![],
            named_animations: default(),
            source: None,
        }
    }

    #[test]
    fn referenced_document_is_shared_only_within_its_source_and_bake() {
        let mut nodes = Assets::<GltfNode>::default();
        let extras = serde_json::json!({"fixtureCollision":{
            "schemaVersion":1,"coordinateContract":moly_assets::coordinates::CONTRACT,
            "units":"source-unity-unit","geometry":[],"gaps":[]}})
        .to_string();
        let node = nodes.add(GltfNode {
            index: 0,
            name: METADATA_NODE.into(),
            children: vec![],
            mesh: None,
            skin: None,
            transform: Transform::IDENTITY,
            is_animation_root: false,
            extras: Some(GltfExtras { value: extras }),
        });
        let mut gltfs = Assets::<Gltf>::default();
        let owner = crate::fixture::FixtureSource(gltfs.add(source(vec![node.clone()])));
        let reference = Reference {
            schema_version: 1,
            node: 0,
        };
        let mut cache = HashMap::new();
        let first = resolve(&reference, &owner, &gltfs, &nodes, &mut cache).unwrap();
        let second = resolve(&reference, &owner, &gltfs, &nodes, &mut cache).unwrap();
        assert!(Arc::ptr_eq(&first, &second));
        assert_eq!(cache.len(), 1);
        let weak = Arc::downgrade(&first);
        drop(first);
        drop(second);
        drop(cache);
        assert!(
            weak.upgrade().is_none(),
            "no permanent parsed-geometry cache"
        );
        let empty = crate::fixture::FixtureSource(gltfs.add(source(vec![])));
        assert!(resolve(&reference, &empty, &gltfs, &nodes, &mut HashMap::new()).is_err());
        nodes.get_mut(&node).unwrap().transform.translation.x = 1.0;
        assert!(resolve(&reference, &owner, &gltfs, &nodes, &mut HashMap::new()).is_err());
    }

    #[test]
    fn references_reject_other_schemas_and_ambiguous_payloads() {
        assert!(serde_json::from_str::<Reference>(r#"{"schemaVersion":1,"node":-1}"#).is_err());
        assert!(
            serde_json::from_str::<Reference>(
                r#"{"schemaVersion":1,"node":0,"document":"elsewhere"}"#
            )
            .is_err()
        );
        let reference = Reference {
            schema_version: 2,
            node: 0,
        };
        let gltfs = Assets::<Gltf>::default();
        let owner = crate::fixture::FixtureSource(gltfs.reserve_handle());
        assert!(
            resolve(
                &reference,
                &owner,
                &gltfs,
                &Assets::default(),
                &mut HashMap::new()
            )
            .err()
            .unwrap()
            .contains("schema")
        );
    }
}
