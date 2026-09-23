//! Bind authored room skins to the shared structural room module.
//!
//! Module materials are slots, not furniture artwork. This surface pass binds
//! the authored skin textures without treating module vertex colours as paint.
//! Bindings and colour-free draw meshes are instance-owned; original glTF
//! assets and other rooms are never mutated.
use crate::site::{GroundEpoch, SiteAssets, SiteScenesReady, SiteSelection};
use bevy::{
    asset::{AssetId, LoadState},
    gltf::Gltf,
    math::Affine2,
    mesh::MeshVertexBufferLayoutRef,
    pbr::{ExtendedMaterial, MaterialExtension, MaterialExtensionKey, MaterialExtensionPipeline},
    prelude::*,
    render::render_resource::{
        AsBindGroup, RenderPipelineDescriptor, SpecializedMeshPipelineError,
    },
};
use moly_assets::json::JsonAsset;
use std::collections::HashMap;

// Keep StandardMaterial's surface shader, but retain the authored render queue.
// In particular Room/Floor (2005) must draw before Rug (2007/2008), whose source
// pass deliberately uses Always depth comparison without depth writes.
type RoomMaterial = ExtendedMaterial<StandardMaterial, RoomSurfaceOrder>;

#[derive(Asset, AsBindGroup, TypePath, Debug, Clone)]
#[bind_group_data(RoomSurfaceOrderKey)]
struct RoomSurfaceOrder {
    queue: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct RoomSurfaceOrderKey(u32);

impl From<&RoomSurfaceOrder> for RoomSurfaceOrderKey {
    fn from(value: &RoomSurfaceOrder) -> Self {
        Self(value.queue)
    }
}

impl MaterialExtension for RoomSurfaceOrder {
    fn specialize(
        _pipeline: &MaterialExtensionPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        _layout: &MeshVertexBufferLayoutRef,
        key: MaterialExtensionKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        crate::material_order::set_queue(descriptor, key.bind_group_data.0);
        Ok(())
    }
}

fn surface_queue(document: &serde_json::Value, wall: bool) -> u32 {
    let token = if wall { "_wall_" } else { "_floor_" };
    let mut queues = document["materials"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|row| {
            row["name"]
                .as_str()
                .is_some_and(|name| name.contains(token))
        })
        .filter_map(|row| {
            row["renderQueue"]
                .as_u64()
                .and_then(|v| u32::try_from(v).ok())
        });
    let first = queues.next();
    // Some generated room skins provide only surfaceBindings, not a separate
    // material row. These are the source room wall/floor shader default queues.
    first
        .filter(|first| queues.all(|queue| queue == *first))
        .unwrap_or(if wall { 2060 } else { 2005 })
}

#[derive(Resource, Clone, Debug, PartialEq, Eq)]
pub(crate) struct RoomAppearance {
    pub(crate) wall: String,
    pub(crate) floor: String,
}
impl Default for RoomAppearance {
    fn default() -> Self {
        Self {
            wall: "mis0001".into(),
            floor: "mis0001".into(),
        }
    }
}
#[derive(Resource, Default)]
pub(crate) struct RoomAppearanceState {
    pub(crate) ready: bool,
    pub(crate) phase: String,
    pub(crate) error: Option<String>,
    key: Option<(u64, String, String)>,
    sources: Vec<Handle<JsonAsset>>,
    images: Vec<Handle<Image>>,
    replacements: HashMap<AssetId<StandardMaterial>, Handle<RoomMaterial>>,
    render_meshes: HashMap<AssetId<Mesh>, Handle<Mesh>>,
}
#[derive(Component)]
struct AppliedAppearance {
    original: AssetId<StandardMaterial>,
}

fn safe_skin(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}
fn surface_material(
    document: &serde_json::Value,
    wall: bool,
) -> Result<(&str, [f32; 4], [f32; 4], bevy::pbr::UvChannel), String> {
    let materials = document
        .get("materials")
        .and_then(|v| v.as_array())
        .ok_or("房间外观缺少材质索引")?;
    let token = if wall { "_wall_" } else { "_floor_" };
    let candidates: Vec<_> = materials
        .iter()
        .filter(|row| {
            row.get("name")
                .and_then(|v| v.as_str())
                .is_some_and(|n| n.contains(token))
        })
        .collect();
    let channel = if wall { "wall" } else { "floor" };
    let bindings = document
        .get("surfaceBindings")
        .and_then(|v| v.get(channel))
        .and_then(|v| v.as_array());
    // The preview uses the original first colour. A generated binding is the
    // source controller's texture-name/color/UV join; _MainTex may be null.
    let generated = bindings.map(|rows| {
        rows.iter()
            .filter(|row| row["colorId"].as_u64() == Some(1))
            .collect::<Vec<_>>()
    });
    let (texture, uv) = if let Some(rows) = &generated {
        let [row] = rows.as_slice() else {
            return Err("这款墙面或地板没有唯一的原始颜色贴图".into());
        };
        let texture = row["texture"].as_str().ok_or("房间外观缺少原始贴图引用")?;
        let uv = match row["texCoord"].as_u64() {
            Some(0) => bevy::pbr::UvChannel::Uv0,
            Some(1) => bevy::pbr::UvChannel::Uv1,
            _ => return Err("这款房间外观的纹理坐标暂不支持".into()),
        };
        (texture, uv)
    } else {
        let textured: Vec<_> = candidates
            .iter()
            .filter(|row| {
                row.pointer("/textures/_MainTex")
                    .and_then(|v| v.as_str())
                    .is_some()
            })
            .collect();
        let [row] = textured.as_slice() else {
            return Err("房间外观没有唯一的原始墙面或地板材质".into());
        };
        (
            row.pointer("/textures/_MainTex")
                .and_then(|v| v.as_str())
                .unwrap(),
            bevy::pbr::UvChannel::Uv0,
        )
    };
    if !texture.starts_with("textures/")
        || texture.contains("..")
        || texture.contains('\\')
        || texture.contains(':')
    {
        return Err("房间纹理引用不是安全的相对路径".into());
    }
    let material = if candidates.len() == 1 {
        Some(candidates[0])
    } else {
        None
    };
    let array = |pointer: &str, fallback: [f32; 4]| -> [f32; 4] {
        material
            .and_then(|row| row.pointer(pointer))
            .and_then(|v| v.as_array())
            .filter(|v| v.len() == 4)
            .map(|values| {
                std::array::from_fn(|i| {
                    values[i]
                        .as_f64()
                        .filter(|v| v.is_finite())
                        .unwrap_or(fallback[i] as f64) as f32
                })
            })
            .unwrap_or(fallback)
    };
    Ok((
        texture,
        array("/colors/_Color", [1.; 4]),
        array("/textureScaleOffset/_MainTex", [1., 1., 0., 0.]),
        uv,
    ))
}

fn apply(
    selection: Res<SiteSelection>,
    epoch: Option<Res<GroundEpoch>>,
    scenes: Option<Res<SiteScenesReady>>,
    site: Option<Res<SiteAssets>>,
    appearance: Res<RoomAppearance>,
    mut state: ResMut<RoomAppearanceState>,
    server: Res<AssetServer>,
    json: Res<Assets<JsonAsset>>,
    gltfs: Res<Assets<Gltf>>,
    mut materials: ResMut<Assets<RoomMaterial>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut commands: Commands,
    mut walk_sources: Option<ResMut<crate::site::WalkFaceMeshes>>,
    mut ground_sources: Option<ResMut<crate::site::GroundMeshes>>,
    mut drawn: Query<
        (
            Entity,
            Option<&MeshMaterial3d<StandardMaterial>>,
            Option<&MeshMaterial3d<RoomMaterial>>,
            &mut Mesh3d,
            Option<&AppliedAppearance>,
        ),
        Or<(
            With<MeshMaterial3d<StandardMaterial>>,
            With<MeshMaterial3d<RoomMaterial>>,
        )>,
    >,
) {
    if !selection.is_room() {
        *state = RoomAppearanceState::default();
        return;
    }
    let (Some(epoch), Some(_), Some(site)) = (epoch, scenes, site) else {
        state.phase = "site scenes".into();
        return;
    };
    let Some(module_handle) = site.module.as_ref() else {
        state.phase = "module handle".into();
        return;
    };
    let Some(module) = gltfs.get(module_handle) else {
        state.phase = "module asset".into();
        return;
    };
    let key = (epoch.0, appearance.wall.clone(), appearance.floor.clone());
    if state.key.as_ref() != Some(&key) {
        *state = RoomAppearanceState {
            key: Some(key),
            ..default()
        };
        if !safe_skin(&appearance.wall) || !safe_skin(&appearance.floor) {
            state.error = Some("房间外观名称无效".into());
            return;
        }
        state.sources = [&appearance.wall, &appearance.floor]
            .into_iter()
            .map(|skin| server.load::<JsonAsset>(format!("moly://site/skins/{skin}/{skin}.json")))
            .collect();
    }
    if state.error.is_some() {
        return;
    }
    if state.replacements.is_empty() {
        state.phase = format!(
            "source documents: {:?}",
            state
                .sources
                .iter()
                .map(|h| server.load_state(h))
                .collect::<Vec<_>>()
        );
        for handle in &state.sources {
            if matches!(server.load_state(handle), LoadState::Failed(_)) {
                state.error = Some("房间外观的原始资源缺失，请重新提取该外观".into());
                return;
            }
        }
        let Some(source) = state
            .sources
            .iter()
            .map(|h| json.get(h))
            .collect::<Option<Vec<_>>>()
        else {
            return;
        };
        let documents = match source
            .iter()
            .map(|a| serde_json::from_str::<serde_json::Value>(&a.0))
            .collect::<Result<Vec<_>, _>>()
        {
            Ok(value) => value,
            Err(_) => {
                state.error = Some("房间外观数据无法解析".into());
                return;
            }
        };
        for (index, (slot, skin)) in [
            ("mat_wall_main", &appearance.wall),
            ("mat_floor_main", &appearance.floor),
        ]
        .into_iter()
        .enumerate()
        {
            let Some(original) = module.named_materials.get(slot) else {
                state.error = Some(format!("房间模块缺少 {slot} 材质槽"));
                return;
            };
            let (path, color, st, uv) = match surface_material(&documents[index], index == 0) {
                Ok(value) => value,
                Err(reason) => {
                    state.error = Some(reason);
                    return;
                }
            };
            let image = server.load::<Image>(format!("moly://site/skins/{skin}/{path}"));
            let handle = materials.add(RoomMaterial {
                base: StandardMaterial {
                    base_color: Color::linear_rgba(color[0], color[1], color[2], color[3]),
                    base_color_texture: Some(image.clone()),
                    base_color_channel: uv,
                    perceptual_roughness: 1.,
                    metallic: 0.,
                    reflectance: 0.,
                    uv_transform: Affine2::from_scale_angle_translation(
                        Vec2::new(st[0], st[1]),
                        0.,
                        Vec2::new(st[2], st[3]),
                    ),
                    ..default()
                },
                extension: RoomSurfaceOrder {
                    queue: surface_queue(&documents[index], index == 0),
                },
            });
            state.images.push(image);
            state.replacements.insert(original.id(), handle);
        }
    }
    for (entity, standard, room, mut mesh, applied) in &mut drawn {
        let Some(source) = applied
            .map(|value| value.original)
            .or_else(|| standard.map(|material| material.0.id()))
        else {
            continue;
        };
        let Some(replacement) = state.replacements.get(&source).cloned() else {
            continue;
        };
        if room.is_some_and(|material| material.0 == replacement) {
            continue;
        }
        if materials
            .get(&replacement)
            .is_some_and(|material| material.base.base_color_channel == bevy::pbr::UvChannel::Uv1)
            && meshes
                .get(&mesh.0)
                .is_some_and(|mesh| mesh.attribute(Mesh::ATTRIBUTE_UV_1).is_none())
        {
            state.error = Some("房间模型缺少这款外观所需的纹理坐标".into());
            return;
        }
        if applied.is_none() {
            let source_mesh = mesh.0.clone();
            let rendered = if let Some(handle) = state.render_meshes.get(&source_mesh.id()) {
                Some(handle.clone())
            } else if let Some(original) = meshes.get(&source_mesh) {
                let mut surface = original.clone();
                surface.remove_attribute(Mesh::ATTRIBUTE_COLOR);
                let handle = meshes.add(surface);
                state.render_meshes.insert(source_mesh.id(), handle.clone());
                Some(handle)
            } else {
                None
            };
            if let Some(rendered) = rendered {
                // Only colour attributes change. Keep the current scene's
                // geometry suppliers pointing at the actual mesh instances;
                // stale glTF handles would make every later navigation bake wait forever.
                if let Some(sources) = walk_sources.as_deref_mut() {
                    for handle in &mut sources.0 {
                        if *handle == source_mesh {
                            *handle = rendered.clone();
                        }
                    }
                }
                if let Some(sources) = ground_sources.as_deref_mut() {
                    for handle in &mut sources.0 {
                        if *handle == source_mesh {
                            *handle = rendered.clone();
                        }
                    }
                }
                mesh.0 = rendered;
            }
        }
        commands
            .entity(entity)
            .remove::<MeshMaterial3d<StandardMaterial>>()
            .insert(MeshMaterial3d(replacement))
            .insert(AppliedAppearance { original: source });
    }
    if state
        .images
        .iter()
        .any(|image| matches!(server.load_state(image), LoadState::Failed(_)))
    {
        state.error = Some("房间外观纹理载入失败".into());
        return;
    }
    state.ready = state
        .images
        .iter()
        .all(|image| server.is_loaded_with_dependencies(image));
    state.phase = format!(
        "images: {:?}; materials={}",
        state
            .images
            .iter()
            .map(|h| server.load_state(h))
            .collect::<Vec<_>>(),
        state.replacements.len()
    );
}

pub(crate) fn install(app: &mut App) {
    app.add_plugins(MaterialPlugin::<RoomMaterial>::default())
        .init_resource::<RoomAppearance>()
        .init_resource::<RoomAppearanceState>()
        .add_systems(Update, apply.after(crate::site::spawn_when_ready));
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn room_surfaces_preserve_source_queue_before_rugs_and_furniture() {
        let value = serde_json::json!({"materials":[
            {"name":"mat_mis0001_floor_floor1","renderQueue":2005},
            {"name":"mat_mis0001_wall_wall1","renderQueue":2060},
            {"name":"mat_mis0001_door_door1_base","renderQueue":2065}
        ]});
        assert_eq!(surface_queue(&value, false), 2005);
        assert_eq!(surface_queue(&value, true), 2060);
        assert!(surface_queue(&value, false) < 2007);
        let bindings_only = serde_json::json!({"surfaceBindings":{"floor":[]}});
        assert_eq!(surface_queue(&bindings_only, false), 2005);
        assert_eq!(
            RoomSurfaceOrderKey::from(&RoomSurfaceOrder { queue: 2005 }).0,
            2005
        );
    }
    #[test]
    fn skin_names_are_source_keys_not_paths() {
        assert!(safe_skin("mis0001"));
        assert!(!safe_skin("../skin"));
        assert!(!safe_skin("C:/skin"));
    }
    #[test]
    fn surface_selection_uses_the_authored_channel_not_the_door_material() {
        let value = serde_json::json!({"materials":[
            {"name":"mat_mis0001_door_door1_base","textures":{"_MainTex":"textures/door.png"}},
            {"name":"mat_mis0001_wall_wall1","textures":{"_MainTex":"textures/wall.png"}},
            {"name":"mat_mis0001_floor_floor1","textures":{"_MainTex":"textures/floor.png"}}
        ]});
        assert_eq!(
            surface_material(&value, true).unwrap().0,
            "textures/wall.png"
        );
        assert_eq!(
            surface_material(&value, false).unwrap().0,
            "textures/floor.png"
        );
    }
    #[test]
    fn source_surface_binding_handles_null_main_texture_and_exact_uv1() {
        let value = serde_json::json!({"materials":[{"name":"mat_env0010_wall_wall1","textures":{"_MainTex":null}}],
            "surfaceBindings":{"wall":[{"colorId":1,"uvSet":2,"texCoord":1,"texture":"textures/wall.png"},{"colorId":2,"uvSet":2,"texCoord":1,"texture":"textures/wall_red.png"}]}});
        let (texture, _, _, uv) = surface_material(&value, true).unwrap();
        assert_eq!(texture, "textures/wall.png");
        assert_eq!(uv, bevy::pbr::UvChannel::Uv1);
    }
    #[test]
    fn surface_binding_does_not_require_a_nonexistent_wall_material() {
        let value = serde_json::json!({"materials":[{"name":"mat_con0004_door_door1"}],
            "surfaceBindings":{"floor":[{"colorId":1,"uvSet":2,"texCoord":1,"texture":"textures/floor.png"}]}});
        assert_eq!(
            surface_material(&value, false).unwrap().0,
            "textures/floor.png"
        );
    }
}
