//! The player avatar's wear and the attachment of its parts.
//!
//! **The wear is server user data.** In the source what the player wears is
//! the four nullable ids of `UserAvatar` (costume / skinColor / accessory /
//! coordinate); `AvatarInfoData` writes them into the room properties as four
//! ints (null -> 0, sync keys A_CID/A_SCID/A_AID/A_CDID) and `AvatarData.Build`
//! reads them back (id 0 -> null). Here the server model holds `userAvatar`
//! in its document and its join response sets the client copy
//! (`crate::avatar_wear_data::ClientUserAvatar`); the ids resolve
//! through the avatar masters the server model reads
//! (`crate::avatar_wear_data::AvatarMasters`, present once they have
//! resolved). An id whose master is absent from the runtime root, or which
//! names no row, is named and read as null, as a master lookup that finds no
//! row is null in the source's chain. The chain, clause by clause:
//! * **Skin bundle** (material composition on the player chain):
//!   coordinate.costumeAssetbundleName ?? costume.assetbundleName ?? "default".
//!   A costume bundle is a **texture-only reskin**: the `skin` asset inside the
//!   bundle fills the `_SkinTex` slot and the mesh is unchanged.
//! * **Accessory bundle**: coordinate.accessoryAssetbundleName ??
//!   accessory.assetbundleName ?? none. A bundle is loaded only when the key is
//!   non-empty; `None` is a first-class branch — nothing is attached.
//! * **Skin tint**: **reads skinColor.Color only, null ⇒ (1,1,1,1)**. The player
//!   chain does **not** read coordinate.SkinColor — that is the formula of the
//!   audience chain's Avatar-Color family, and the two chains share the family
//!   but differ in form: the player chain's skin-colour read is a single one at
//!   `AvatarData+0x20`, with no `coordinate+0x48`.
//! * **No penlight.** The MySekai player never holds one, whatever the
//!   server sends: the room sync keys have no penlight column, the penlight
//!   mesh is mounted only by the audience and live chains (no MySekai
//!   player code calls it), and the player view's two penlight colour
//!   setters have no caller in the client. The player view reads the
//!   `Penlight_R`/`Penlight_L` bones only as its arm handles. So the wear
//!   has no penlight (`userAvatar` has no such id); the material's penlight
//!   properties keep their defaults.
//!
//! **Accessory assembly form**: the player chain bakes accessory meshes into
//! the merged mesh (part index in slot 1 goes into UV0.z); in this repository's
//! composed glb the part-index carrier is the x component of the second UV set
//! (the existing convention of `avatar_material`). A part mesh has no such
//! channel of its own (it is a standalone piece, and the number is baked only
//! at merge time) ⇒ assembly injects `UV_1 = (1,0)` at runtime (slot 1), and the
//! material shares the same AvatarMaterial values as the body — the same
//! dispatch rule, without merging meshes (merging is not done on the extraction
//! side, and performing mesh surgery at runtime is a separate change, out of
//! scope here).
//! The accessory hangs under the `Accessory_face` bone (the same node used for
//! skinned pieces; for non-skinned pieces the vertex rigid transform is also
//! relative to that bone's position).
//!
//! **Texture lookup**: the accessory/penlight glb does not wire its textures
//! into the standard PBR slots (they only sit in extras, which the loader does
//! not read) — but the loader registers the embedded images as label
//! sub-assets `Texture{index}`, so they are fetched by label (no filename
//! guessing). The accessory's Texture0 is accessoryTexture (the `_MainTex` of
//! the first Renderer's material).
//!
//! The latch component lives on the player entity: the assembly system waits for
//! the body reskin latch (`AvatarToon`, the equivalent signal that the scene is
//! expanded), then assembles once; `AccessoryWorn` prevents re-entry. The `None`
//! branch also latches (nothing attached is a first-class branch, and it has a
//! named line in the panel log).

use bevy::asset::{AssetPath, RecursiveDependencyLoadState};
use bevy::gltf::{Gltf, GltfMesh};
use bevy::prelude::*;
use std::borrow::Cow;
use std::collections::BTreeMap;

use crate::avatar_material::{AvatarMaterial, AvatarParams, AvatarToon};
use crate::player::PlayerControlled;
use crate::avatar_wear_data::{AvatarMasters, ClientUserAvatar};
use moly_law::shading::avatar as law;

/// Fallback value for "skin bundle not set": the literal "default" (the fallback
/// arm of the skin-bundle selection chain).
const DEFAULT_SKIN_BUNDLE: &str = "default";

/// Accessory mounting bone name (the name of the player chain's accessoryNode predicate).
const ACCESSORY_BONE: &str = "Accessory_face";

// ---------------------------------------------------------------------------
// 面板与资源
// ---------------------------------------------------------------------------

/// The wear: the values the chain derives plus the live handles. Built once,
/// when the join's copy and the masters are in, and kept to the end of the
/// process (dropping the last handle cancels a load in flight, so the handles
/// must live here).
#[derive(Resource)]
pub struct AvatarWear {
    /// Skin bundle name (output of the resolution chain; unset ⇒ "default").
    pub skin_bundle: String,
    /// Skin tint rgba (skinColor.Color; unset ⇒ white).
    pub skin_color: [f32; 4],
    /// Raw colour code of the skin tint (for logging; unset ⇒ "none").
    pub skin_color_code: Cow<'static, str>,
    /// Accessory bundle name (`None` = the null branch: nothing is attached).
    pub accessory_bundle: Option<String>,
    /// Skin texture (`_SkinTex`; the `skin` asset of the skin bundle).
    pub skin_tex: Handle<Image>,
    /// Accessory glb.
    pub accessory_gltf: Option<Handle<Gltf>>,
    /// Accessory texture (embedded Texture0 of the glb = accessoryTexture).
    pub accessory_tex: Option<Handle<Image>>,
}

/// Product path of the `skin` asset inside the skin bundle (extraction naming
/// convention: `<bundle>__skin.png`; the asset is taken from the bundle by the
/// asset name "skin").
fn skin_tex_path(bundle: &str) -> String {
    format!("moly://avatar-parts/skin/{bundle}/tex/{bundle}__skin.png")
}

/// glb path of an accessory bundle.
fn decoration_glb_path(bundle: &str) -> String {
    format!("moly://avatar-parts/decoration/{bundle}/{bundle}.glb")
}

/// Label sub-asset path of an embedded glb image (the loader registers every
/// embedded texture as `Texture{index}`, including textures that no material
/// references — the only way to fetch them without guessing file names).
fn glb_texture(glb: &str, index: usize) -> AssetPath<'static> {
    AssetPath::from(glb.to_owned()).with_label(format!("Texture{index}"))
}

/// The master row an id names; an absent master or a missing row is named
/// and read as null.
fn row_of<'a, T>(
    id: Option<i32>,
    rows: Option<&'a BTreeMap<i32, T>>,
    field: &str,
    master: &str,
) -> Option<&'a T> {
    let id = id?;
    let Some(rows) = rows else {
        warn!("[player] wear: userAvatar.{field} = {id} cannot resolve: {master} is absent from the runtime root; read as null");
        return None;
    };
    let row = rows.get(&id);
    if row.is_none() {
        warn!("[player] wear: userAvatar.{field} = {id} names no row of {master}; read as null");
    }
    row
}

/// `#rrggbb` colour code → srgb [0,1] (the shape of the masters' colorCode).
/// The bad-value warning carries the raw value and the value is treated as unset
/// (the fallback is white) instead of being silently swallowed.
fn parse_hex_color(raw: &str) -> Option<[f32; 4]> {
    let hex = raw.strip_prefix('#').unwrap_or(raw);
    if hex.len() != 6 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        warn!(
            "[player] wear panel: colour code {raw:?} is not #rrggbb; treated as unset (falls back to white)"
        );
        return None;
    }
    let value = u32::from_str_radix(hex, 16).ok()?;
    Some([
        ((value >> 16) & 0xff) as f32 / 255.0,
        ((value >> 8) & 0xff) as f32 / 255.0,
        (value & 0xff) as f32 / 255.0,
        1.0,
    ])
}

/// Update, until the resource lands: once the join's copy of `UserAvatar`
/// and the avatar masters are in, resolve the chain, send the load requests,
/// insert the resource and log the wear line (the named reading of the wear:
/// a different `userAvatar` changes this line and the attachment lines).
pub fn load(
    mut commands: Commands,
    server: Res<AssetServer>,
    user: Res<ClientUserAvatar>,
    masters: Option<Res<AvatarMasters>>,
) {
    // The masters are still loading, or the join has not reached the client.
    let Some(masters) = masters else {
        return;
    };
    if user.revision == 0 {
        return;
    }
    let avatar = user.avatar;
    let id = |id: Option<i32>| id.map_or_else(|| "null".to_owned(), |id| id.to_string());
    let coordinate = row_of(
        avatar.coordinate,
        masters.coordinates.as_ref(),
        "avatarCoordinateId",
        "avatar-coordinates.json",
    );
    let costume = row_of(
        avatar.costume,
        masters.costumes.as_ref(),
        "avatarCostumeId",
        "avatar-costumes.json",
    );
    let accessory = row_of(
        avatar.accessory,
        masters.accessories.as_ref(),
        "avatarAccessoryId",
        "avatar-accessories.json",
    );
    let skin = row_of(
        avatar.skin_color,
        masters.skin_colors.as_ref(),
        "avatarSkinColorId",
        "avatar-skin-colors.json",
    );
    // Skin bundle (player chain): coordinate.costume ?? costume ?? "default".
    let skin_bundle = coordinate
        .and_then(|row| row.costume_assetbundle_name.clone())
        .or_else(|| costume.cloned())
        .unwrap_or_else(|| DEFAULT_SKIN_BUNDLE.to_owned());
    // Accessory bundle (assembly chain): coordinate.accessory ?? accessory ?? none.
    let accessory_bundle = coordinate
        .and_then(|row| row.accessory_assetbundle_name.clone())
        .or_else(|| accessory.cloned());
    // Skin tint: the player chain reads skinColor.Color only (unset ⇒ white) and
    // not the coordinate colour (that is the audience chain's formula, see the
    // module header).
    let skin_color = skin
        .and_then(|code| parse_hex_color(code))
        .unwrap_or([1.0, 1.0, 1.0, 1.0]);
    let skin_color_code = Cow::from(skin.cloned().unwrap_or_else(|| "#ffffff".to_owned()));

    // The skin texture is sampled only by the GPU (the avatar material takes it
    // from the render world); there is no CPU reader of its texels and no other
    // requester.
    let skin_tex = moly_assets::residency::load_image(&server, skin_tex_path(&skin_bundle));
    let (accessory_gltf, accessory_tex) = match &accessory_bundle {
        Some(bundle) => {
            let glb = decoration_glb_path(bundle);
            (
                Some(server.load::<Gltf>(AssetPath::from(glb.clone()))),
                Some(server.load::<Image>(glb_texture(&glb, 0))),
            )
        }
        None => (None, None),
    };
    info!(
        "[player] wear (server userAvatar): coordinate={} costume={} accessory={} skinColor={} ⇒ skin bundle {skin_bundle} / tint {skin_color_code} / accessory {}",
        id(avatar.coordinate),
        id(avatar.costume),
        id(avatar.accessory),
        id(avatar.skin_color),
        accessory_bundle.as_deref().unwrap_or("none (nothing attached)"),
    );
    commands.insert_resource(AvatarWear {
        skin_bundle,
        skin_color,
        skin_color_code,
        accessory_bundle,
        skin_tex,
        accessory_gltf,
        accessory_tex,
    });
}

// ---------------------------------------------------------------------------
// 装配闩与标记
// ---------------------------------------------------------------------------

/// Accessory assembly latch (on the player entity; the `None` branch latches as
/// well — nothing attached is a first-class branch).
#[derive(Component)]
pub struct AccessoryWorn;

/// Marker on accessory mesh entities: excluded from the body reskin scan (an
/// accessory uses AvatarMaterial slot 1 and must not be swept up as a body part;
/// its mesh also has no part-index channel, so a sweep would hit the check and be
/// rejected loudly).
#[derive(Component)]
pub struct WearPart;

// ---------------------------------------------------------------------------
// 饰件装配
// ---------------------------------------------------------------------------

/// Update: attach the accessory once the body reskin latch is in place. A load
/// failure panics loudly (an unextracted piece is rejected by name); an
/// incomplete glb waits silently for the next frame.
pub fn attach_accessory(
    mut commands: Commands,
    server: Res<AssetServer>,
    wear: Res<AvatarWear>,
    gltfs: Res<Assets<Gltf>>,
    gltf_meshes: Res<Assets<GltfMesh>>,
    images: Res<Assets<Image>>,
    mut mesh_assets: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<AvatarMaterial>>,
    players: Query<Entity, (With<PlayerControlled>, With<crate::avatar_material::AudienceBody>, With<AvatarToon>, Without<AccessoryWorn>)>,
    children: Query<&Children>,
    names: Query<&Name>,
) {
    for player in &players {
        let Some(bundle) = wear.accessory_bundle.clone() else {
            commands.entity(player).insert(AccessoryWorn);
            info!("[player] avatar accessory: none set (null branch, nothing attached)");
            continue;
        };
        let Some(gltf_handle) = wear.accessory_gltf.clone() else {
            panic!("穿戴集面板有饰品 {bundle} 却没有它的装载请求：面板解析与装载脱节");
        };
        match server.recursive_dependency_load_state(&gltf_handle) {
            RecursiveDependencyLoadState::Failed(err) => {
                panic!("avatar 饰件 {bundle} 装载失败（未提取件具名拒绝）：{err:?}")
            }
            RecursiveDependencyLoadState::Loaded => {}
            _ => return, // 未到齐，下一帧再试
        }
        let gltf = gltfs.get(&gltf_handle).expect("装载门已过");
        // The prefab's first MeshFilter mesh is the one used; the loader lists
        // meshes in file order, so meshes[0] is the first entry in scene node order.
        let source = gltf
            .meshes
            .first()
            .unwrap_or_else(|| panic!("avatar 饰件 {bundle} 的 glb 没有网格"));
        let gltf_mesh = gltf_meshes
            .get(source)
            .unwrap_or_else(|| panic!("avatar 饰件 {bundle} 的网格资产不在 Assets 里"));
        let primitive = gltf_mesh
            .primitives
            .first()
            .unwrap_or_else(|| panic!("avatar 饰件 {bundle} 的网格没有 primitive"));
        let mut mesh = mesh_assets
            .get(&primitive.mesh)
            .unwrap_or_else(|| panic!("avatar 饰件 {bundle} 的 Mesh 资产不在 Assets 里"))
            .clone();
        // part-index injection: slot 1 (the part number is baked into UV0.z at
        // merge time; here the carrier is the x component of the second UV set, and
        // a part mesh has no such channel of its own).
        let vertices = mesh.count_vertices();
        mesh.insert_attribute(
            Mesh::ATTRIBUTE_UV_1,
            vec![[1.0f32, 0.0f32]; vertices],
        );
        let mesh_handle = mesh_assets.add(mesh);
        // The accessory's material uses the same values as the body (the
        // single-mesh single-material dispatch rule; slot 1 feeds the accessory texture).
        let mut law_material = law::default_material();
        law_material.skin_color = wear.skin_color;
        let material = materials.add(AvatarMaterial {
            params: AvatarParams::from_material(&law_material),
            skin_tex: wear.skin_tex.clone(),
            accessory_tex: wear.accessory_tex.clone(),
            penlight_body_tex: None,
            penlight_light_tex: None,
        });
        let Some(bone) = find_named(player, ACCESSORY_BONE, &children, &names) else {
            panic!("玩家 avatar 骨架里没有 {ACCESSORY_BONE} 挂点骨：合成体骨架断线");
        };
        let tex = wear
            .accessory_tex
            .clone()
            .expect("面板有饰品 ⇒ 装载请求必含贴图");
        let (width, height) = images
            .get(&tex)
            .map(|image| (image.width(), image.height()))
            .unwrap_or((0, 0));
        commands.entity(bone).with_child((
            Name::new(format!("wear_accessory_{bundle}")),
            WearPart,
            Mesh3d(mesh_handle),
            MeshMaterial3d(material),
            crate::markers::PlayerBodyRenderer,
            Transform::IDENTITY,
        ));
        commands.entity(player).insert(AccessoryWorn);
        info!(
            "[player] avatar 挂件装配：{bundle} 挂 {ACCESSORY_BONE} 骨（part 槽 1 _AccessoryTex；网格 {vertices} 顶点；贴图 {width}x{height}）"
        );
    }
}

// ---------------------------------------------------------------------------
// 公用
// ---------------------------------------------------------------------------

/// 深度优先按名找子树实体（先序，第一个命中即返回；emoticon 挂点骨同款）。
fn find_named(
    root: Entity,
    wanted: &str,
    children: &Query<&Children>,
    names: &Query<&Name>,
) -> Option<Entity> {
    let mut stack = vec![root];
    while let Some(entity) = stack.pop() {
        if let Ok(name) = names.get(entity) {
            if name.as_str() == wanted {
                return Some(entity);
            }
        }
        if let Ok(kids) = children.get(entity) {
            for kid in kids.iter().rev() {
                stack.push(kid);
            }
        }
    }
    None
}
