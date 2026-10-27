//! The bulk hammer uses the existing Fixture/Basic shader and source skinning.
//! Its PackageExtract geometry already flips UV V; ordinary fixture geometry
//! does not. Keep that adaptation local to this owned attachment.
use std::collections::HashMap;

use bevy::{
    asset::LoadState,
    gltf::GltfMaterialName,
    prelude::*,
    world_serialization::{WorldInstance, WorldInstanceSpawner},
};
use moly_assets::{
    json::JsonAsset,
    material_passes::{SourceMaterialPasses, SourceRenderState},
    sidecar::{parse_site_sidecar, SiteMaterialSource},
};

use crate::fixture_material::{resolve_site_basic, FixtureMaterial};
use crate::player_avatar::item_timeline::{BulkToolMaterialState, PlayerStepItem};

const BASIC: &str = "Mysekai/Fixture/Basic";

fn resolve(
    source: &SiteMaterialSource,
    texture: Handle<Image>,
) -> Result<(FixtureMaterial, SourceMaterialPasses), String> {
    if source.shader != BASIC {
        return Err(format!(
            "{} uses {}, expected {BASIC}",
            source.name, source.shader
        ));
    }
    let passes = source
        .passes
        .clone()
        .ok_or("source pass metadata missing")?;
    let opaque = SourceRenderState {
        cull: 2,
        depth_test: 4,
        depth_write: true,
        color_mask: 15,
        src_color: 1,
        dst_color: 0,
        src_alpha: 1,
        dst_alpha: 0,
        color_op: 0,
        alpha_op: 0,
    };
    if !passes.has_light_mode("MysekaiObject") || passes.color_state() != Some(opaque) {
        return Err("bulk hammer has unsupported source Base render state".into());
    }
    // This attachment has one ordinary colour texture. Keep other authored
    // branches explicit instead of treating their absent setup as success.
    if source.textures.len() != 1
        || source.textures[0].0 != "_MainTex"
        || source.floats.get("_FixtureShaderUsage") != Some(&0.0)
        || source.floats.get("_FixtureObjectBlendMode") != Some(&0.0)
    {
        return Err("bulk hammer is not the supported opaque MainTex-only Basic branch".into());
    }
    let mut basic = resolve_site_basic(&source.slot, texture)?;
    if basic.has_mask
        || basic.colour_picker.is_some()
        || basic.material.crystal.is_some()
        || basic.material.multi_uv.is_some()
    {
        return Err("bulk hammer needs additional source material bindings".into());
    }
    // The glTF loader retained the texture's authored sampler and decoded its
    // sRGB format. FixtureMaterial already performs that format-domain bridge.
    basic.material.site_sampler = false;
    basic.material.params.uv_v_flip = 0.0;
    // Unity offsets precede the exporter's V reflection.
    basic.material.params.main_tex_offset[1] = -basic.material.params.main_tex_offset[1];
    Ok((basic.material, passes))
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub fn swap(
    mut commands: Commands,
    step: Res<PlayerStepItem>,
    server: Res<AssetServer>,
    json: Res<Assets<JsonAsset>>,
    roots: Query<(&BulkToolMaterialState, &WorldInstance)>,
    instances: Res<WorldInstanceSpawner>,
    children: Query<&Children>,
    parts: Query<(Entity, &MeshMaterial3d<StandardMaterial>, &GltfMaterialName)>,
    standard: Res<Assets<StandardMaterial>>,
    mut materials: ResMut<Assets<FixtureMaterial>>,
) {
    let Some((root, document)) = step.bulk_tool_model() else {
        return;
    };
    let Ok((BulkToolMaterialState::Pending, instance)) = roots.get(root) else {
        return;
    };
    if !instances.instance_is_ready(**instance) {
        return;
    }
    let refusal = |commands: &mut Commands, reason: String| {
        error!("[bulk-harvest] hammer source material refused: {reason}");
        commands
            .entity(root)
            .insert((BulkToolMaterialState::Refused(reason), Visibility::Hidden));
    };
    if let LoadState::Failed(error) = server.load_state(&document) {
        refusal(&mut commands, format!("document failed to load: {error}"));
        return;
    }
    let Some(document) = json.get(&document) else {
        return;
    };
    let mut descendants = Vec::new();
    let mut stack = vec![root];
    while let Some(entity) = stack.pop() {
        if let Ok(kids) = children.get(entity) {
            stack.extend(kids.iter());
        }
        if let Ok(part) = parts.get(entity) {
            descendants.push(part);
        }
    }
    if descendants.is_empty() {
        return;
    }
    let sidecar = match parse_site_sidecar(document.0.as_bytes()) {
        Ok(sidecar) => sidecar,
        Err(error) => {
            refusal(&mut commands, error.to_string());
            return;
        }
    };
    let mut plan = HashMap::new();
    for (_, original, name) in &descendants {
        if plan.contains_key(&original.0) {
            continue;
        }
        let sources: Vec<_> = sidecar
            .materials
            .iter()
            .filter(|row| row.name == name.0)
            .collect();
        let [source] = sources.as_slice() else {
            refusal(
                &mut commands,
                format!("material {} has {} source rows", name.0, sources.len()),
            );
            return;
        };
        let Some(original_material) = standard.get(&original.0) else {
            return;
        };
        let Some(texture) = original_material.base_color_texture.clone() else {
            refusal(
                &mut commands,
                format!("material {} has no glTF main texture", name.0),
            );
            return;
        };
        match server.load_state(&texture) {
            LoadState::Failed(error) => {
                refusal(
                    &mut commands,
                    format!("material {} main texture: {error}", name.0),
                );
                return;
            }
            LoadState::Loaded => {}
            _ => return,
        }
        match resolve(source, texture) {
            Ok(resolved) => {
                plan.insert(original.0.clone(), resolved);
            }
            Err(reason) => {
                refusal(&mut commands, reason);
                return;
            }
        }
    }
    let material_count = plan.len();
    let ready: HashMap<_, _> = plan
        .into_iter()
        .map(|(source, (material, passes))| (source, (materials.add(material), passes)))
        .collect();
    for (entity, original, _) in &descendants {
        let (material, passes) = &ready[&original.0];
        commands
            .entity(*entity)
            .remove::<MeshMaterial3d<StandardMaterial>>()
            .insert((MeshMaterial3d(material.clone()), passes.clone()));
    }
    commands
        .entity(root)
        .insert((BulkToolMaterialState::Ready, Visibility::Inherited));
    info!("[bulk-harvest] hammer: {} owned primitives / {material_count} materials bound to {BASIC}; source pass state, skinning and exported UV convention retained", descendants.len());
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn source() -> SiteMaterialSource {
        let scalar = |value| json!({"name":"<noninit>","val":value});
        let row = json!({"name":"synthetic-basic","shader":{"name":BASIC,"shaderPasses":[{
            "name":"Base","lightMode":"MysekaiObject","renderState":{
                "culling":scalar(2),"zTest":scalar(4),"zWrite":scalar(1),"blend":{
                    "colMask":scalar(15),"srcBlend":scalar(1),"destBlend":scalar(0),
                    "srcBlendAlpha":scalar(1),"destBlendAlpha":scalar(0),
                    "blendOp":scalar(0),"blendOpAlpha":scalar(0)}}}]},
            "keywords":["_DISABLE_DITHER","_RECEIVE_SHADOWS_OFF"],
            "textures":{"_MainTex":"main.png"},"floats":{
                "_FixtureShaderUsage":0,"_FixtureObjectBlendMode":0,
                "_MainTexOffsetX":0.2,"_MainTexOffsetY":0.3,"_DitherAlpha":1,
                "_UsePhenomenaLighting":1,"_OverrideShadingParameter":0,
                "_LocalShadingIntensity":1,"_LocalEdgeThreshold":0.65,
                "_LocalEdgeSmoothness":0.04,"_RenderPriority":0}});
        parse_site_sidecar(
            serde_json::to_string(&json!({"materials":[row],"textures":["main.png"]}))
                .unwrap()
                .as_bytes(),
        )
        .unwrap()
        .materials
        .remove(0)
    }

    #[test]
    fn bulk_uses_fixture_shading_with_exported_uvs_and_authored_sampler() {
        let source = source();
        let (material, passes) = resolve(&source, Handle::default()).unwrap();
        assert_eq!(material.key.render_queue, 2065);
        assert!(!material.key.alpha_clip && !material.key.dither && !material.site_sampler);
        assert_eq!(material.params.uv_v_flip, 0.0);
        assert_eq!(material.params.main_tex_offset, [0.2, -0.3]);
        assert_eq!(material.params.use_phenomena_lighting, 1.0);
        assert!(passes.has_light_mode("MysekaiObject"));
    }

    #[test]
    fn bulk_refuses_avatar_shader_and_unbound_additional_material_inputs() {
        let mut row = source();
        row.slot.shader = "Mysekai/Avatar-Tool".into();
        assert!(resolve(&row, Handle::default()).is_err());
        row = source();
        row.slot.textures.push(("_EmissionMaskTex".into(), 1));
        assert!(resolve(&row, Handle::default()).is_err());
        row = source();
        row.passes = None;
        assert!(resolve(&row, Handle::default()).is_err());
    }
}
