//! Reading a user's model file: the humanoid bone map it declares and the
//! copy of the file the engine's glTF loader reads.
//!
//! Three forms are read, in this order:
//! 1. VRM 1.0 (`extensions.VRMC_vrm.humanoid.humanBones.<bone>.node`);
//! 2. VRM 0.x (`extensions.VRM.humanoid.humanBones[] = {bone, node}`);
//! 3. plain glTF, where node names give the bones
//!    ([`super::humanoid::bone_from_node_name`]).
//!
//! The copy differs from the file in three places, all in the JSON chunk;
//! the binary chunk is carried over byte for byte:
//! - every node is renamed: a humanoid bone to `oc.<bone>`, any other node to
//!   `oc.n<index>`. Bones are then found by name without ambiguity, and no
//!   node of the model can be mistaken for a joint of the player avatar that
//!   other systems look up by name (`Head`, `Hips`, `Penlight_R`, ...);
//!   every mesh is renamed to `oc.m<index>` for the same reason (the loader
//!   names a single-primitive mesh's entity after its mesh);
//! - the VRM extensions are removed from `extensionsRequired`, where the
//!   loader would refuse them (they describe the avatar, not the geometry);
//! - a VRM 0.x material whose glTF base colour texture is missing gets the
//!   texture its VRM material properties name as `_MainTex`.
//!
//! Files that reference other files (external buffers or images) are
//! refused: the model is read from memory only.

use super::constraint::{self, ConstraintKind, NodeConstraint};
use super::humanoid;
use bevy::math::{Mat4, Quat, Vec3};
use bevy::prelude::warn;
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;

/// The largest model file read (VRM files with their textures are commonly
/// tens of mebibytes).
pub(crate) const MAX_MODEL_BYTES: usize = 128 * 1024 * 1024;

const GLB_MAGIC: u32 = 0x4654_6C67;
const CHUNK_JSON: u32 = 0x4E4F_534A;
const CHUNK_BIN: u32 = 0x004E_4942;

/// Where the humanoid map came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MapSource {
    Vrm1,
    Vrm0,
    NodeNames,
    /// A character body of the cast: bones from its node names, materials
    /// from its rig profile.
    Character,
}

impl MapSource {
    pub(crate) fn label(self) -> &'static str {
        match self {
            MapSource::Vrm1 => "VRM 1.0",
            MapSource::Vrm0 => "VRM 0.x",
            MapSource::NodeNames => "glTF (bones from node names)",
            MapSource::Character => "character body",
        }
    }
}

/// A model file ready for the loader.
#[derive(Debug)]
pub(crate) struct ModelFile {
    pub(crate) source: MapSource,
    /// Humanoid bone -> glTF node index.
    pub(crate) bones: BTreeMap<&'static str, usize>,
    /// The bytes the loader reads.
    pub(crate) bytes: Vec<u8>,
    /// `glb` or `gltf`: the extension that selects the loader.
    pub(crate) extension: &'static str,
    /// The original node names of the mapped bones, for the log.
    pub(crate) original_names: BTreeMap<&'static str, String>,
    /// The node constraints that can be applied, in evaluation order.
    pub(crate) constraints: Vec<NodeConstraint>,
}

/// The name a node carries in the copy.
pub(crate) fn node_name(bone: Option<&str>, index: usize) -> String {
    match bone {
        Some(bone) => format!("oc.{bone}"),
        None => format!("oc.n{index}"),
    }
}

/// Splits a GLB into its JSON and binary chunks.
fn split_glb(bytes: &[u8]) -> Result<(Value, Option<&[u8]>), String> {
    let word = |at: usize| -> Result<u32, String> {
        bytes
            .get(at..at + 4)
            .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
            .ok_or_else(|| "The GLB file is truncated".to_owned())
    };
    if word(4)? != 2 {
        return Err("Only glTF 2.0 binary files are supported".into());
    }
    let total = word(8)? as usize;
    if total > bytes.len() {
        return Err("The GLB file is shorter than its header says".into());
    }
    let mut at = 12;
    let mut json = None;
    let mut bin = None;
    while at + 8 <= total {
        let length = word(at)? as usize;
        let kind = word(at + 4)?;
        let start = at + 8;
        let end = start
            .checked_add(length)
            .filter(|end| *end <= total)
            .ok_or("A GLB chunk runs past the end of the file")?;
        match kind {
            CHUNK_JSON if json.is_none() => {
                let text = std::str::from_utf8(&bytes[start..end])
                    .map_err(|_| "The GLB JSON chunk is not UTF-8")?;
                json = Some(
                    serde_json::from_str::<Value>(text.trim_end_matches(['\0', ' ']))
                        .map_err(|error| format!("The GLB JSON chunk is not JSON: {error}"))?,
                );
            }
            CHUNK_BIN if bin.is_none() => bin = Some(&bytes[start..end]),
            _ => {}
        }
        at = end;
    }
    Ok((json.ok_or("The GLB file has no JSON chunk")?, bin))
}

/// Writes a GLB: the JSON chunk padded with spaces, the binary chunk with
/// zeros, both to four bytes.
fn write_glb(json: &Value, bin: Option<&[u8]>) -> Vec<u8> {
    let mut text = serde_json::to_vec(json).expect("a JSON value serializes");
    while text.len() % 4 != 0 {
        text.push(b' ');
    }
    let bin_padded = bin.map(|bin| bin.len().div_ceil(4) * 4);
    let total = 12 + 8 + text.len() + bin_padded.map_or(0, |len| 8 + len);
    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(&GLB_MAGIC.to_le_bytes());
    out.extend_from_slice(&2u32.to_le_bytes());
    out.extend_from_slice(&(total as u32).to_le_bytes());
    out.extend_from_slice(&(text.len() as u32).to_le_bytes());
    out.extend_from_slice(&CHUNK_JSON.to_le_bytes());
    out.extend_from_slice(&text);
    if let (Some(bin), Some(padded)) = (bin, bin_padded) {
        out.extend_from_slice(&(padded as u32).to_le_bytes());
        out.extend_from_slice(&CHUNK_BIN.to_le_bytes());
        out.extend_from_slice(bin);
        out.resize(total, 0);
    }
    out
}

fn node_index(value: &Value, nodes: usize, what: &str) -> Result<usize, String> {
    value
        .as_u64()
        .map(|index| index as usize)
        .filter(|index| *index < nodes)
        .ok_or_else(|| format!("{what} names no node of the file"))
}

/// Reads `N` numbers from a JSON array.
fn floats<const N: usize>(values: &[Value]) -> Option<[f32; N]> {
    let numbers: Vec<f32> = values
        .iter()
        .map(|value| value.as_f64().map(|number| number as f32))
        .collect::<Option<_>>()?;
    numbers.try_into().ok()
}

/// A node's rest rotation: its `rotation` quaternion, else the rotation part
/// of its `matrix`, else identity. A zero quaternion counts as identity.
fn node_rest(node: &Value) -> Quat {
    let rotation = if let Some(xyzw) = node["rotation"].as_array().and_then(|list| floats::<4>(list)) {
        Quat::from_array(xyzw)
    } else if let Some(cols) = node["matrix"].as_array().and_then(|list| floats::<16>(list)) {
        let (_, rotation, _) = Mat4::from_cols_array(&cols).to_scale_rotation_translation();
        rotation
    } else {
        return Quat::IDENTITY;
    };
    if rotation.length_squared() > 0.0 && rotation.is_finite() {
        rotation.normalize()
    } else {
        Quat::IDENTITY
    }
}

/// The unit axis a `rollAxis` names.
fn roll_axis(name: Option<&str>) -> Result<Vec3, String> {
    match name {
        Some("X") => Ok(Vec3::X),
        Some("Y") => Ok(Vec3::Y),
        Some("Z") => Ok(Vec3::Z),
        Some(other) => Err(format!("rollAxis \"{other}\" is not X, Y or Z")),
        None => Err("rollAxis is missing".into()),
    }
}

/// The unit axis an `aimAxis` names.
fn aim_axis(name: Option<&str>) -> Result<Vec3, String> {
    match name {
        Some("PositiveX") => Ok(Vec3::X),
        Some("NegativeX") => Ok(Vec3::NEG_X),
        Some("PositiveY") => Ok(Vec3::Y),
        Some("NegativeY") => Ok(Vec3::NEG_Y),
        Some("PositiveZ") => Ok(Vec3::Z),
        Some("NegativeZ") => Ok(Vec3::NEG_Z),
        Some(other) => Err(format!("aimAxis \"{other}\" is not a signed axis")),
        None => Err("aimAxis is missing".into()),
    }
}

/// One node's constraint, read from the `constraint` object that holds
/// exactly one of `roll`, `aim` and `rotation`. Nodes are named as the copy
/// names them ([`node_name`]).
fn parse_constraint(
    index: usize,
    node: &Value,
    constraint: &Map<String, Value>,
    list: &[Value],
    by_node: &BTreeMap<usize, &'static str>,
) -> Result<NodeConstraint, String> {
    let kinds: Vec<&str> = ["roll", "aim", "rotation"]
        .into_iter()
        .filter(|kind| constraint.contains_key(*kind))
        .collect();
    let [kind] = kinds.as_slice() else {
        return Err(format!(
            "it must have exactly one of roll, aim, rotation (it has {})",
            kinds.len()
        ));
    };
    let body = &constraint[*kind];
    let src = node_index(&body["source"], list.len(), &format!("the source of node {index}"))?;
    if src == index {
        return Err("its source is the node itself".into());
    }
    let weight = body["weight"].as_f64().unwrap_or(1.0) as f32;
    if !weight.is_finite() {
        return Err("its weight is not a finite number".into());
    }
    let kind = match *kind {
        "roll" => ConstraintKind::Roll {
            axis: roll_axis(body["rollAxis"].as_str())?,
        },
        "aim" => ConstraintKind::Aim {
            axis: aim_axis(body["aimAxis"].as_str())?,
        },
        _ => ConstraintKind::Rotation,
    };
    Ok(NodeConstraint {
        dst: index,
        src,
        dst_name: node_name(by_node.get(&index).copied(), index),
        src_name: node_name(by_node.get(&src).copied(), src),
        kind,
        weight,
        dst_rest: node_rest(node),
        src_rest: node_rest(&list[src]),
    })
}

/// Reads every `VRMC_node_constraint` of the file, in evaluation order. A
/// constraint that cannot be applied is left out with its node and reason;
/// constraints that depend on a skipped or cyclic constraint are left out too.
fn read_constraints(
    root: &Value,
    nodes: usize,
    by_node: &BTreeMap<usize, &'static str>,
) -> (Vec<NodeConstraint>, Vec<(usize, String)>) {
    let list = root["nodes"].as_array().map(Vec::as_slice).unwrap_or(&[]);
    let mut parent = vec![None; nodes];
    for (index, node) in list.iter().enumerate() {
        let children = node["children"].as_array().into_iter().flatten();
        for child in children.filter_map(Value::as_u64) {
            if let Some(slot) = parent.get_mut(child as usize) {
                *slot = Some(index);
            }
        }
    }
    let mut kept = Vec::new();
    let mut skipped = Vec::new();
    for (index, node) in list.iter().enumerate() {
        let Some(constraint) = node["extensions"]["VRMC_node_constraint"]["constraint"].as_object()
        else {
            continue;
        };
        match parse_constraint(index, node, constraint, list, by_node) {
            Ok(found) => kept.push(found),
            Err(reason) => skipped.push((index, reason)),
        }
    }
    let (ordered, blocked) = constraint::order(&kept, &parent);
    skipped.extend(blocked.into_iter().map(|dst| {
        (
            dst,
            "it is on a dependency cycle, or depends on one".to_owned(),
        )
    }));
    (ordered, skipped)
}

/// The humanoid map the file declares, or the one its node names give.
fn humanoid_map(
    root: &Value,
    nodes: usize,
) -> Result<(MapSource, BTreeMap<&'static str, usize>), String> {
    let extensions = &root["extensions"];
    let mut bones = BTreeMap::new();
    if let Some(human) = extensions["VRMC_vrm"]["humanoid"]["humanBones"].as_object() {
        for (name, entry) in human {
            let Some(bone) = humanoid::canonical(name, false) else {
                continue;
            };
            let node = node_index(&entry["node"], nodes, &format!("VRM bone {name}"))?;
            bones.insert(bone, node);
        }
        return Ok((MapSource::Vrm1, bones));
    }
    if let Some(human) = extensions["VRM"]["humanoid"]["humanBones"].as_array() {
        for entry in human {
            let Some(name) = entry["bone"].as_str() else {
                continue;
            };
            let Some(bone) = humanoid::canonical(name, true) else {
                continue;
            };
            let node = node_index(&entry["node"], nodes, &format!("VRM bone {name}"))?;
            bones.entry(bone).or_insert(node);
        }
        return Ok((MapSource::Vrm0, bones));
    }
    // Plain glTF: depth first from the default scene, the first node of a
    // bone's name wins.
    let node_list = root["nodes"].as_array().map(Vec::as_slice).unwrap_or(&[]);
    let scene = root["scene"].as_u64().unwrap_or(0) as usize;
    let mut stack: Vec<usize> = root["scenes"][scene]["nodes"]
        .as_array()
        .map(|roots| roots.iter().rev().filter_map(Value::as_u64).map(|n| n as usize).collect())
        .unwrap_or_default();
    let mut seen = vec![false; nodes];
    while let Some(index) = stack.pop() {
        if index >= nodes || std::mem::replace(&mut seen[index], true) {
            continue;
        }
        let node = &node_list[index];
        if let Some(bone) = node["name"].as_str().and_then(humanoid::bone_from_node_name) {
            bones.entry(bone).or_insert(index);
        }
        if let Some(children) = node["children"].as_array() {
            stack.extend(children.iter().rev().filter_map(Value::as_u64).map(|n| n as usize));
        }
    }
    Ok((MapSource::NodeNames, bones))
}

/// Reads a model file and prepares the loader's copy.
pub(crate) fn read_model(bytes: &[u8]) -> Result<ModelFile, String> {
    if bytes.len() > MAX_MODEL_BYTES {
        return Err(format!(
            "The model file exceeds the {} MiB limit",
            MAX_MODEL_BYTES >> 20
        ));
    }
    let glb = bytes.len() >= 4
        && u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) == GLB_MAGIC;
    let (mut root, bin) = if glb {
        split_glb(bytes)?
    } else {
        let text = std::str::from_utf8(bytes)
            .map_err(|_| "The file is neither a GLB/VRM file nor glTF JSON")?;
        let value = serde_json::from_str::<Value>(text.trim_start_matches('\u{feff}'))
            .map_err(|_| "The file is neither a GLB/VRM file nor glTF JSON")?;
        (value, None)
    };
    if !root.is_object() {
        return Err("The glTF root is not an object".into());
    }
    if root["asset"]["version"].as_str().is_none_or(|v| !v.starts_with('2')) {
        return Err("Only glTF 2.0 files are supported".into());
    }
    for family in ["buffers", "images"] {
        if let Some(items) = root[family].as_array() {
            for item in items {
                if let Some(uri) = item["uri"].as_str() {
                    if !uri.starts_with("data:") {
                        return Err(format!(
                            "The file references another file ({family}: {uri}); use a single .glb or .vrm file"
                        ));
                    }
                }
            }
        }
    }
    let nodes = root["nodes"].as_array().map_or(0, Vec::len);
    if nodes == 0 {
        return Err("The file has no nodes".into());
    }
    let (source, bones) = humanoid_map(&root, nodes)?;
    let missing: Vec<&str> = humanoid::REQUIRED_BONES
        .iter()
        .copied()
        .filter(|bone| !bones.contains_key(bone))
        .collect();
    if !missing.is_empty() {
        return Err(format!(
            "The {} humanoid map lacks required bones: {}",
            source.label(),
            missing.join(", ")
        ));
    }
    let mut seen_nodes = BTreeMap::new();
    for (bone, node) in &bones {
        if let Some(other) = seen_nodes.insert(*node, *bone) {
            return Err(format!("Bones {other} and {bone} name the same node {node}"));
        }
    }
    let by_node: BTreeMap<usize, &'static str> =
        bones.iter().map(|(bone, node)| (*node, *bone)).collect();
    let (constraints, skipped) = read_constraints(&root, nodes, &by_node);
    if !skipped.is_empty() {
        let list: Vec<String> = skipped
            .iter()
            .map(|(node, reason)| format!("node {node} ({reason})"))
            .collect();
        warn!("[oc] node constraints skipped: {}", list.join("; "));
    }
    let mut original_names = BTreeMap::new();
    if let Some(list) = root.get_mut("nodes").and_then(Value::as_array_mut) {
        for (index, node) in list.iter_mut().enumerate() {
            let Some(object) = node.as_object_mut() else {
                return Err(format!("Node {index} is not an object"));
            };
            let bone = by_node.get(&index).copied();
            if let Some(bone) = bone {
                original_names.insert(
                    bone,
                    object
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_owned(),
                );
            }
            object.insert("name".into(), Value::String(node_name(bone, index)));
        }
    }
    if let Some(list) = root.get_mut("meshes").and_then(Value::as_array_mut) {
        for (index, mesh) in list.iter_mut().enumerate() {
            if let Some(object) = mesh.as_object_mut() {
                object.insert("name".into(), Value::String(format!("oc.m{index}")));
            }
        }
    }
    // `get_mut`, not indexing: indexing a missing key would insert a null.
    if let Some(required) = root.get_mut("extensionsRequired").and_then(Value::as_array_mut) {
        required.retain(|name| name.as_str().is_none_or(|name| !name.starts_with("VRM")));
        if required.is_empty() {
            root.as_object_mut()
                .expect("checked above")
                .remove("extensionsRequired");
        }
    }
    if source == MapSource::Vrm0 {
        vrm0_base_textures(&mut root);
    }
    let (bytes, extension) = if glb {
        (write_glb(&root, bin), "glb")
    } else {
        (
            serde_json::to_vec(&root).expect("a JSON value serializes"),
            "gltf",
        )
    };
    Ok(ModelFile {
        source,
        bones,
        bytes,
        extension,
        original_names,
        constraints,
    })
}

/// VRM 0.x keeps its material settings in `materialProperties`, matched to
/// the glTF materials by name; its `_MainTex` is the base colour texture.
fn vrm0_base_textures(root: &mut Value) {
    let properties: Vec<(String, u64)> = root["extensions"]["VRM"]["materialProperties"]
        .as_array()
        .map(|list| {
            list.iter()
                .filter_map(|entry| {
                    Some((
                        entry["name"].as_str()?.to_owned(),
                        entry["textureProperties"]["_MainTex"].as_u64()?,
                    ))
                })
                .collect()
        })
        .unwrap_or_default();
    let textures = root["textures"].as_array().map_or(0, Vec::len) as u64;
    let Some(materials) = root.get_mut("materials").and_then(Value::as_array_mut) else {
        return;
    };
    for material in materials {
        let Some(name) = material["name"].as_str().map(str::to_owned) else {
            continue;
        };
        let Some((_, texture)) = properties.iter().find(|(n, _)| *n == name) else {
            continue;
        };
        if *texture >= textures {
            continue;
        }
        let Some(object) = material.as_object_mut() else {
            continue;
        };
        let pbr = object
            .entry("pbrMetallicRoughness")
            .or_insert_with(|| Value::Object(Map::new()));
        if let Some(pbr) = pbr.as_object_mut() {
            pbr.entry("baseColorTexture")
                .or_insert_with(|| json!({ "index": texture }));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A humanoid node list: one node per bone name, children as listed.
    fn rig(names: &[&str]) -> Value {
        let nodes: Vec<Value> = names
            .iter()
            .enumerate()
            .map(|(index, name)| {
                let mut node = json!({ "name": name });
                if index + 1 < names.len() {
                    node["children"] = json!([index + 1]);
                }
                node
            })
            .collect();
        json!({
            "asset": { "version": "2.0" },
            "scene": 0,
            "scenes": [{ "nodes": [0] }],
            "nodes": nodes,
        })
    }

    const REQUIRED_MIXAMO: [&str; 15] = [
        "Hips",
        "Spine",
        "Head",
        "LeftUpLeg",
        "LeftLeg",
        "LeftFoot",
        "RightUpLeg",
        "RightLeg",
        "RightFoot",
        "LeftArm",
        "LeftForeArm",
        "LeftHand",
        "RightArm",
        "RightForeArm",
        "RightHand",
    ];

    #[test]
    fn plain_gltf_maps_bones_by_name_and_renames_every_node() {
        let mut names = REQUIRED_MIXAMO.to_vec();
        names.push("Armature");
        let mut root = rig(&names);
        root["meshes"] = json!([{ "name": "Head", "primitives": [] }]);
        let file = read_model(&serde_json::to_vec(&root).unwrap()).unwrap();
        assert_eq!(file.source, MapSource::NodeNames);
        assert_eq!(file.extension, "gltf");
        assert_eq!(file.bones["leftUpperArm"], 9);
        let copy: Value = serde_json::from_slice(&file.bytes).unwrap();
        assert_eq!(copy["nodes"][9]["name"], "oc.leftUpperArm");
        assert_eq!(copy["nodes"][15]["name"], "oc.n15");
        assert_eq!(file.original_names["leftUpperArm"], "LeftArm");
        assert_eq!(copy["meshes"][0]["name"], "oc.m0");
    }

    #[test]
    fn vrm1_map_wins_over_names_and_vrm_extensions_leave_required() {
        let mut root = rig(&["a", "b", "c", "d", "e", "f", "g", "h", "i", "j", "k", "l", "m", "n", "o"]);
        let mut human = Map::new();
        for (index, bone) in humanoid::REQUIRED_BONES.iter().enumerate() {
            human.insert((*bone).to_owned(), json!({ "node": index }));
        }
        root["extensions"] = json!({ "VRMC_vrm": { "humanoid": { "humanBones": human } } });
        root["extensionsUsed"] = json!(["VRMC_vrm"]);
        root["extensionsRequired"] = json!(["VRMC_vrm", "KHR_texture_transform"]);
        let glb = write_glb(&root, Some(&[1u8, 2, 3][..]));
        let file = read_model(&glb).unwrap();
        assert_eq!(file.source, MapSource::Vrm1);
        assert_eq!(file.extension, "glb");
        let (copy, bin) = split_glb(&file.bytes).unwrap();
        assert_eq!(bin.unwrap()[..3], [1, 2, 3]);
        assert_eq!(copy["extensionsRequired"], json!(["KHR_texture_transform"]));
        assert_eq!(copy["nodes"][0]["name"], "oc.hips");
        assert_eq!(file.bytes.len() % 4, 0);
    }

    #[test]
    fn vrm0_array_map_reads_thumbs_and_main_textures() {
        let mut names = REQUIRED_MIXAMO.to_vec();
        names.push("Thumb");
        let mut root = rig(&names);
        let mut bones: Vec<Value> = humanoid::REQUIRED_BONES
            .iter()
            .enumerate()
            .map(|(index, bone)| json!({ "bone": bone, "node": index }))
            .collect();
        bones.push(json!({ "bone": "leftThumbProximal", "node": 11 }));
        root["extensions"] = json!({ "VRM": {
            "humanoid": { "humanBones": bones },
            "materialProperties": [{ "name": "skin", "textureProperties": { "_MainTex": 0 } }],
        } });
        root["materials"] = json!([{ "name": "skin" }]);
        root["textures"] = json!([{ "source": 0 }]);
        // Bone leftHand (index 11 in REQUIRED_BONES) and the thumb share a
        // node: refused.
        assert!(read_model(&serde_json::to_vec(&root).unwrap()).is_err());
        root["extensions"]["VRM"]["humanoid"]["humanBones"][15]["node"] = json!(15);
        let file = read_model(&serde_json::to_vec(&root).unwrap()).unwrap();
        assert_eq!(file.source, MapSource::Vrm0);
        assert_eq!(file.bones["leftThumbMetacarpal"], 15);
        let copy: Value = serde_json::from_slice(&file.bytes).unwrap();
        assert_eq!(
            copy["materials"][0]["pbrMetallicRoughness"]["baseColorTexture"]["index"],
            0
        );
    }

    #[test]
    fn missing_bones_and_external_files_are_refused() {
        let root = rig(&REQUIRED_MIXAMO[..14]);
        let error = read_model(&serde_json::to_vec(&root).unwrap()).unwrap_err();
        assert!(error.contains("rightHand"), "{error}");
        let mut root = rig(&REQUIRED_MIXAMO);
        root["buffers"] = json!([{ "uri": "model.bin", "byteLength": 4 }]);
        let error = read_model(&serde_json::to_vec(&root).unwrap()).unwrap_err();
        assert!(error.contains("model.bin"), "{error}");
        assert!(read_model(b"not a model").is_err());
    }

    #[test]
    fn node_constraints_are_read_and_invalid_ones_are_skipped() {
        use std::f32::consts::{FRAC_PI_2, FRAC_PI_4};
        let mut names = REQUIRED_MIXAMO.to_vec();
        names.extend(["Armature", "Accessory", "Broken"]);
        let mut root = rig(&names);
        // Node 16: a roll about Y at weight 0.5, with a rest rotation of a
        // quarter turn about Z (the quaternion carries the half angle).
        root["nodes"][16]["rotation"] = json!([0.0, 0.0, FRAC_PI_4.sin(), FRAC_PI_4.cos()]);
        root["nodes"][16]["extensions"] = json!({ "VRMC_node_constraint": { "constraint": {
            "roll": { "source": 9, "rollAxis": "Y", "weight": 0.5 }
        } } });
        // Node 17: two kinds at once, so it is skipped.
        root["nodes"][17]["extensions"] = json!({ "VRMC_node_constraint": { "constraint": {
            "roll": { "source": 9, "rollAxis": "Y" },
            "aim": { "source": 9, "aimAxis": "PositiveZ" }
        } } });
        let file = read_model(&serde_json::to_vec(&root).unwrap()).unwrap();
        assert_eq!(file.constraints.len(), 1);
        let found = &file.constraints[0];
        assert_eq!((found.dst, found.src), (16, 9));
        assert_eq!(found.dst_name, "oc.n16");
        assert_eq!(found.src_name, "oc.leftUpperArm");
        assert_eq!(found.weight, 0.5);
        assert_eq!(found.kind, ConstraintKind::Roll { axis: Vec3::Y });
        assert!(found.dst_rest.angle_between(Quat::from_rotation_z(FRAC_PI_2)) < 1e-5);
    }
}
