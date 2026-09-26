//! Bind authored room skins to the structural room module and draw the module
//! with the source room programs (`room_shell.rs`).
//!
//! Source: the module prefab keeps its own four materials (the wall and floor
//! renderers that `WallView` / `FloorView` drive, and the two entrance
//! renderers). A skin change looks the skin texture up in the skin bundle by
//! the pattern `ClientConfig.Mysekai.MyRoomWallAppearanceAssetName` /
//! `MyRoomFloorAssetName` formatted with the skin bundle name and the colour
//! id (`AssetBundleLoader.FindAssetByPattern`), reads the uv set from the
//! pattern's first group, and puts both into the renderer through a
//! MaterialPropertyBlock: `_MainTex`, and `_BaseTextureMappingMode` on the
//! wall / `_UVSelection` on the floor, both "uv set n - 1". The skin bundle's
//! own materials are never drawn by the room. The entrances keep the module
//! texture. The module meshes are drawn with their vertex colours:
//! `Mysekai/Room/Floor` multiplies the texture by them, and `Mysekai/Object`
//! blends them (usage 10) or reads their red channel as wall occlusion
//! (usage 2), whose edge factor reads the fourth uv set. Bindings are
//! instance-owned; the module's materials and other rooms are never mutated.
//! The module meshes gain the third and fourth uv sets the module glb
//! carries, which the engine's glTF loader does not map
//! (`room_shell::attach_source_uv_sets`).
//!
//! Which skin and colour: [`RoomAppearance`] is resolved per room load from
//! the account's housing layout rows (the site's own, then the first
//! floor's), else the ClientConfig default fixtures with colour 1; the skin is
//! the surface fixture's master `assetbundleName`.
//!
//! What cannot be computed from a module file is refused piece by piece, not
//! room by room. A wall mesh without the fourth set (a module file exported
//! before the uv-set export) is drawn without the edge factor, and one error
//! line per room load names it. A surface whose material samples a uv set its
//! mesh lacks keeps its module material and is named; every other surface is
//! drawn with the source programs. Every mesh is checked before any is
//! swapped, so no surface ends up half-swapped.
use crate::client_config::{
    ClientConfigs, KEY_MY_ROOM_FLOOR_ASSET_NAME, KEY_MY_ROOM_WALL_APPEARANCE_ASSET_NAME,
};
use crate::room_shell::{self, RoomShellMaterial};
use crate::site::{GroundEpoch, SiteAssets, SiteScenesReady, SiteSelection};
use bevy::{
    asset::{AssetId, LoadState},
    gltf::{Gltf, GltfMesh},
    prelude::*,
};
use moly_assets::json::JsonAsset;
use std::collections::HashMap;

/// The two module slots that take the skin texture (the wall and floor
/// renderers of `WallView` / `FloorView`); the entrance slots keep theirs.
const WALL_SLOT: &str = "mat_wall_main";
const FLOOR_SLOT: &str = "mat_floor_main";

/// `ClientConfig.Mysekai.MyRoomDefaultFloorAppearanceId` (IntConfigs 105) and
/// `MyRoomDefaultWallAppearanceId` (IntConfigs 106): the fixture ids
/// `SiteLayoutUtility.SetDefaultFloorAppearanceId` /
/// `SetDefaultWallAppearanceId` give a room without a saved surface.
const KEY_MY_ROOM_DEFAULT_FLOOR_APPEARANCE_ID: i32 = 105;
const KEY_MY_ROOM_DEFAULT_WALL_APPEARANCE_ID: i32 = 106;
/// `ClientConfig.Mysekai.DefaultMyRoomAssetBundleName` (StringConfigs 98):
/// `MysekaiFixtureUtility.GetSurfaceAppearanceAssetBundleName` returns it
/// when the fixture master has no row for the id.
const KEY_DEFAULT_MY_ROOM_ASSET_BUNDLE_NAME: i32 = 98;
/// The colour (texture) id both `SetDefault*AppearanceId` pass.
const DEFAULT_COLOR_ID: u32 = 1;
/// `MysekaiFixtureSurfaceAppearanceType`: wall_appearance 0,
/// floor_appearance 1. `OnAfterDeserialize` parses the row's string with
/// `TryGetEnum(ignoreCase: true, default 1)`.
const WALL_APPEARANCE: u8 = 0;
const FLOOR_APPEARANCE: u8 = 1;
const FIXTURE_MASTER: &str = "moly://mysekai-fixtures.json";

/// The room's two surface appearances: the skin bundle names (the surface
/// fixture's master `assetbundleName`) and the colour ids (the texture id)
/// the room is drawn with.
///
/// Source (`MyRoomSiteController.SetCurrentFloorAppearanceId` /
/// `SetCurrentWallAppearanceId`, run when a room site is set up): the site's
/// own housing layout row of the channel (`GetFloorAppearanceData` /
/// `GetWallAppearanceData`: the first `mysekaiFixtureSurfaceAppearances`
/// entry of that type), else the first floor's, else the ClientConfig
/// default fixture with colour 1. The layout rows are the account's server
/// data; the product keeps them in the local layout document (an imported
/// account carries them), read here on every room load.
#[derive(Resource, Clone, Debug, PartialEq, Eq)]
pub(crate) struct RoomAppearance {
    pub(crate) wall: String,
    pub(crate) floor: String,
    pub(crate) wall_color: u32,
    pub(crate) floor_color: u32,
    /// The room load (ground epoch) the values were resolved for.
    resolved: Option<u64>,
    /// The skins the resolution last wrote: a different value at the next
    /// room load was written by another owner (the content library's
    /// surface preview) and is kept.
    written: Option<(String, String)>,
}
impl Default for RoomAppearance {
    fn default() -> Self {
        Self {
            wall: String::new(),
            floor: String::new(),
            wall_color: DEFAULT_COLOR_ID,
            floor_color: DEFAULT_COLOR_ID,
            resolved: None,
            written: None,
        }
    }
}
impl RoomAppearance {
    /// The surfaces of the room loaded for `epoch` are resolved.
    pub(crate) fn resolved_for(&self, epoch: u64) -> bool {
        self.resolved == Some(epoch)
    }
}

/// The fixture master's `assetbundleName` by fixture id, for
/// `GetSurfaceAppearanceAssetBundleName`.
#[derive(Resource)]
struct SurfaceMaster {
    handle: Handle<JsonAsset>,
    bundles: Option<HashMap<i64, String>>,
}

fn load_master(mut commands: Commands, server: Res<AssetServer>) {
    commands.insert_resource(SurfaceMaster {
        handle: server.load(FIXTURE_MASTER),
        bundles: None,
    });
}

fn parse_master(text: &str) -> Result<HashMap<i64, String>, String> {
    let doc: serde_json::Value =
        serde_json::from_str(text).map_err(|error| format!("fixture master: {error}"))?;
    let rows = doc["fixtures"]
        .as_array()
        .ok_or("fixture master has no fixtures array")?;
    rows.iter()
        .map(|row| {
            let id = row["id"].as_i64().ok_or("fixture master row without id")?;
            let bundle = row["assetbundleName"]
                .as_str()
                .ok_or_else(|| format!("fixture master row {id} without assetbundleName"))?;
            Ok((id, bundle.to_owned()))
        })
        .collect()
}

/// `TryGetEnum(ignoreCase: true, default floor_appearance)` over the row's
/// type string. Whether `TryGetEnum` also accepts the numeric form was not
/// read; the names are what the server sends.
fn surface_type(value: &serde_json::Value) -> u8 {
    match value.as_str() {
        Some(text) if text.eq_ignore_ascii_case("wall_appearance") => WALL_APPEARANCE,
        _ => FLOOR_APPEARANCE,
    }
}

/// `GetFloorAppearanceData` / `GetWallAppearanceData` of one saved site
/// record: the first row of the channel's type, as (fixture id, texture id).
fn saved_surface(record: &serde_json::Value, channel: u8) -> Result<Option<(i64, u32)>, String> {
    let rows = match record.get("mysekaiFixtureSurfaceAppearances") {
        None | Some(serde_json::Value::Null) => return Ok(None),
        Some(rows) => rows
            .as_array()
            .ok_or("mysekaiFixtureSurfaceAppearances is not an array")?,
    };
    let Some(row) = rows
        .iter()
        .find(|row| surface_type(&row["mysekaiFixtureSurfaceAppearanceType"]) == channel)
    else {
        return Ok(None);
    };
    let id = row["mysekaiFixtureId"]
        .as_i64()
        .ok_or("surface appearance row without mysekaiFixtureId")?;
    let texture = row["textureId"]
        .as_u64()
        .and_then(|value| u32::try_from(value).ok())
        .ok_or("surface appearance row without textureId")?;
    Ok(Some((id, texture)))
}

/// The saved site record of `site_id` in the local layout document, if any.
fn saved_record(document: &serde_json::Value, site_id: u32) -> Option<&serde_json::Value> {
    document
        .get(crate::fixture::layouts::SECTION)?
        .get("sites")?
        .get(site_id.to_string())
}

/// One channel: the site's own row, else the first floor's, else the
/// ClientConfig default with colour 1; then the bundle name of the fixture.
fn resolve_channel(
    document: &serde_json::Value,
    site_id: u32,
    first_floor: Option<u32>,
    channel: u8,
    configs: &ClientConfigs,
    bundles: &HashMap<i64, String>,
) -> Result<(String, u32, &'static str), String> {
    let mut chosen = None;
    for (site, origin) in [
        (Some(site_id), "the room's layout"),
        (first_floor, "the first floor's layout"),
    ] {
        let Some(record) = site.and_then(|site| saved_record(document, site)) else {
            continue;
        };
        if let Some(row) = saved_surface(record, channel)? {
            chosen = Some((row, origin));
            break;
        }
    }
    let ((id, color), origin) = chosen.unwrap_or_else(|| {
        let key = if channel == WALL_APPEARANCE {
            KEY_MY_ROOM_DEFAULT_WALL_APPEARANCE_ID
        } else {
            KEY_MY_ROOM_DEFAULT_FLOOR_APPEARANCE_ID
        };
        (
            (configs.int(key) as i64, DEFAULT_COLOR_ID),
            "the ClientConfig default",
        )
    });
    let bundle = bundles.get(&id).cloned().unwrap_or_else(|| {
        configs
            .string(KEY_DEFAULT_MY_ROOM_ASSET_BUNDLE_NAME)
            .to_owned()
    });
    Ok((bundle, color, origin))
}

/// Update: resolve the room's surfaces once per room load.
fn resolve(
    active: Option<Res<crate::site::SiteActive>>,
    epoch: Option<Res<GroundEpoch>>,
    sites: Option<Res<crate::site::Sites>>,
    configs: Option<Res<ClientConfigs>>,
    master: Option<ResMut<SurfaceMaster>>,
    json: Res<Assets<JsonAsset>>,
    mut appearance: ResMut<RoomAppearance>,
) {
    let (Some(active), Some(epoch), Some(sites), Some(configs), Some(mut master)) =
        (active, epoch, sites, configs, master)
    else {
        return;
    };
    if !active.is_indoor() || appearance.resolved == Some(epoch.0) {
        return;
    }
    if master.bundles.is_none() {
        let Some(text) = json.get(&master.handle) else {
            return;
        };
        match parse_master(&text.0) {
            Ok(bundles) => master.bundles = Some(bundles),
            Err(reason) => panic!("[room-shell] {reason}"),
        }
    }
    let bundles = master.bundles.as_ref().expect("parsed above");
    let document = crate::settings_store::read_document().unwrap_or_else(|reason| {
        error!("[room-shell] the local layout document cannot be read ({reason}): the room surfaces take the defaults");
        serde_json::Value::Object(Default::default())
    });
    let first_floor = sites.site_id("first_floor");
    let mut resolved = Vec::new();
    for channel in [WALL_APPEARANCE, FLOOR_APPEARANCE] {
        match resolve_channel(
            &document,
            active.site_id,
            first_floor,
            channel,
            &configs,
            bundles,
        ) {
            Ok(value) => resolved.push(value),
            Err(reason) => {
                error!("[room-shell] {}: a saved surface appearance row is malformed ({reason}): the room takes the defaults", active.site_type);
                resolved = [WALL_APPEARANCE, FLOOR_APPEARANCE]
                    .into_iter()
                    .map(|channel| {
                        resolve_channel(
                            &serde_json::Value::Null,
                            active.site_id,
                            None,
                            channel,
                            &configs,
                            bundles,
                        )
                        .expect("the default chain reads no document")
                    })
                    .collect();
                break;
            }
        }
    }
    let [(wall, wall_color, wall_origin), (floor, floor_color, floor_origin)] =
        <[_; 2]>::try_from(resolved).expect("two channels");
    let current = (appearance.wall.clone(), appearance.floor.clone());
    let overridden = appearance
        .written
        .as_ref()
        .is_some_and(|written| *written != current);
    if overridden {
        info!(
            "[room-shell] {}: surfaces {} / {} were set by another owner; kept (colours {} / {})",
            active.site_type, current.0, current.1, appearance.wall_color, appearance.floor_color
        );
    } else {
        info!(
            "[room-shell] {}: wall {wall} colour {wall_color} from {wall_origin}; floor {floor} colour {floor_color} from {floor_origin}",
            active.site_type
        );
        appearance.wall = wall;
        appearance.floor = floor;
        appearance.wall_color = wall_color;
        appearance.floor_color = floor_color;
    }
    appearance.written = Some((appearance.wall.clone(), appearance.floor.clone()));
    appearance.resolved = Some(epoch.0);
}
#[derive(Resource, Default)]
pub(crate) struct RoomAppearanceState {
    pub(crate) ready: bool,
    pub(crate) phase: String,
    pub(crate) error: Option<String>,
    key: Option<(u64, String, String, u32, u32)>,
    /// Wall skin, floor skin, then the module's own sidecar.
    sources: Vec<Handle<JsonAsset>>,
    images: Vec<Handle<Image>>,
    /// Module material -> its room program material.
    replacements: HashMap<AssetId<StandardMaterial>, Replacement>,
    /// Source behaviour the product cannot draw from the extracted data, named
    /// once per build of the materials.
    named_gaps: Vec<String>,
    /// Module materials kept because a mesh drawn with them lacks the uv set
    /// their room material samples (named once per room load).
    refused: Vec<AssetId<StandardMaterial>>,
    refused_names: Vec<String>,
    /// Wall meshes drawn without the edge factor (no fourth uv set), and
    /// whether that has been named for this room load.
    walls_without_uv3: usize,
    walls_without_uv3_named: bool,
}

/// One module material's room program material.
#[derive(Clone)]
struct Replacement {
    material: Handle<RoomShellMaterial>,
    /// The module material's name (the slot).
    slot: String,
    /// The mesh uv set the main texture is sampled with.
    uv: u32,
    /// The usage-2 wall edge factor, which reads the fourth set.
    reads_uv3: bool,
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

/// `string.Format(pattern, bundleName, colorId)` for the two placeholders the
/// room patterns carry; any other brace is refused.
fn format_pattern(pattern: &str, bundle: &str, color_id: u32) -> Result<String, String> {
    let formatted = pattern
        .replace("{0}", bundle)
        .replace("{1}", &color_id.to_string());
    if formatted.contains('{') || formatted.contains('}') {
        return Err(format!("房间外观贴图名模式 {pattern} 含未移植的占位符"));
    }
    Ok(formatted)
}

/// `Regex.Match(name, pattern).Groups[1]` for the one pattern shape the room
/// configs use: literal text around a single greedy `(.*)` group. The match
/// is unanchored (leftmost start) and the group greedy (the last occurrence
/// of the tail). `Ok(None)` when the name does not match.
fn match_single_group<'a>(name: &'a str, pattern: &str) -> Result<Option<&'a str>, String> {
    let Some((head, tail)) = pattern.split_once("(.*)") else {
        return Err(format!("房间外观贴图名模式 {pattern} 没有分组"));
    };
    let literal = |text: &str| {
        text.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    };
    if !literal(head) || !literal(tail) {
        return Err(format!("房间外观贴图名模式 {pattern} 含未移植的正则语法"));
    }
    let Some(at) = name.find(head) else {
        return Ok(None);
    };
    let start = at + head.len();
    Ok(name[start..]
        .rfind(tail)
        .map(|end| &name[start..start + end]))
}

/// The engine texture name of an extracted texture file: the extractor
/// writes `textures/<name>-<8 hex digits>.png`.
fn texture_name(uri: &str) -> Result<&str, String> {
    let stem = uri
        .strip_prefix("textures/")
        .and_then(|rest| rest.strip_suffix(".png"))
        .ok_or_else(|| format!("房间纹理引用 {uri} 不是 textures/<名>.png"))?;
    match stem.rsplit_once('-') {
        Some((name, hash))
            if !name.is_empty()
                && hash.len() == 8
                && hash.bytes().all(|b| b.is_ascii_hexdigit()) =>
        {
            Ok(name)
        }
        _ => Err(format!("房间纹理引用 {uri} 缺少提取侧的 8 位散列后缀")),
    }
}

/// The mesh uv set a skin texture is drawn with. `WallView` writes
/// `_BaseTextureMappingMode` 0 / 2 / 3 and `FloorView` `_UVSelection`
/// 0 / 1 / 2 for uv set 1 / 2 / 3 (a group that does not parse, or parses
/// to 0, counts as uv set 1; any other value maps to 0); both programs read
/// mesh uv 0 / 1 / 2 for those values.
fn uv_set_index(group: &str) -> u32 {
    let parsed = group.trim().parse::<i32>().unwrap_or(0);
    match if parsed == 0 { 1 } else { parsed } {
        2 => 1,
        3 => 2,
        _ => 0,
    }
}

/// The skin texture of one channel and the mesh uv set it is drawn with.
fn skin_surface<'a>(
    document: &'a serde_json::Value,
    pattern: &str,
) -> Result<(&'a str, u32), String> {
    let textures = document
        .get("textures")
        .and_then(|v| v.as_array())
        .ok_or("房间外观缺少贴图清单")?;
    let mut hits = Vec::new();
    for uri in textures.iter().filter_map(|v| v.as_str()) {
        if let Some(group) = match_single_group(texture_name(uri)?, pattern)? {
            hits.push((uri, group));
        }
    }
    let [(uri, group)] = hits.as_slice() else {
        return Err(format!(
            "房间外观按模式 {pattern} 找到 {} 张贴图，需要恰好一张",
            hits.len()
        ));
    };
    if uri.contains("..") || uri.contains('\\') || uri.contains(':') {
        return Err("房间纹理引用不是安全的相对路径".into());
    }
    Ok((uri, uv_set_index(group)))
}

/// The declared colour space of a texture a room program samples: the
/// programs encode every sample back to the stored domain, which is only an
/// identity round trip for a texture loaded as sRGB.
fn colour_texture(document: &serde_json::Value, uri: &str) -> Result<(), String> {
    match document
        .pointer("/textureColourSpace")
        .and_then(|v| v.get(uri))
        .and_then(|v| v.as_bool())
    {
        Some(false) => Err(format!(
            "房间纹理 {uri} 声明为线性色彩空间，房间程序把采样编回存储域，只接受 sRGB"
        )),
        _ => Ok(()),
    }
}

/// Resolve the four module materials against the source programs.
#[allow(clippy::too_many_arguments)]
fn build(
    documents: &[serde_json::Value],
    appearance: &RoomAppearance,
    module_dir: &str,
    configs: &ClientConfigs,
    module: &Gltf,
    gltf_meshes: &Assets<GltfMesh>,
    meshes: &mut Assets<Mesh>,
    server: &AssetServer,
    materials: &mut Assets<RoomShellMaterial>,
    state: &mut RoomAppearanceState,
) -> Result<(), String> {
    let [wall_doc, floor_doc, module_doc] = documents else {
        return Err("房间外观数据不全".into());
    };
    let [with_uv2, with_uv3] = room_shell::attach_source_uv_sets(module, gltf_meshes, meshes)?;
    let wall_pattern = format_pattern(
        configs.string(KEY_MY_ROOM_WALL_APPEARANCE_ASSET_NAME),
        &appearance.wall,
        appearance.wall_color,
    )?;
    let floor_pattern = format_pattern(
        configs.string(KEY_MY_ROOM_FLOOR_ASSET_NAME),
        &appearance.floor,
        appearance.floor_color,
    )?;
    let (wall_uri, wall_uv) = skin_surface(wall_doc, &wall_pattern)?;
    let (floor_uri, floor_uv) = skin_surface(floor_doc, &floor_pattern)?;
    colour_texture(wall_doc, wall_uri)?;
    colour_texture(floor_doc, floor_uri)?;
    let records = module_doc
        .get("materials")
        .and_then(|v| v.as_array())
        .ok_or("房间模块缺少材质索引")?;
    let mut slots = Vec::new();
    for record in records {
        let name = record
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or("房间模块材质缺少名字")?;
        let original = module
            .named_materials
            .get(name)
            .ok_or_else(|| format!("房间模块 glb 没有材质 {name}"))?;
        let (uv, path) = match name {
            WALL_SLOT => (
                Some(wall_uv),
                format!("moly://site/skins/{}/{wall_uri}", appearance.wall),
            ),
            FLOOR_SLOT => (
                Some(floor_uv),
                format!("moly://site/skins/{}/{floor_uri}", appearance.floor),
            ),
            _ => {
                let uri = record
                    .pointer("/textures/_MainTex")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| format!("房间模块材质 {name} 没有主贴图"))?;
                if !uri.starts_with("textures/")
                    || uri.contains("..")
                    || uri.contains('\\')
                    || uri.contains(':')
                {
                    return Err("房间纹理引用不是安全的相对路径".into());
                }
                colour_texture(module_doc, uri)?;
                (None, format!("{module_dir}/{uri}"))
            }
        };
        let resolved = room_shell::resolve(record, uv)?;
        let index = resolved.uv_index();
        let image = moly_assets::residency::load_image(server, path);
        let handle = materials.add(resolved.with_texture(image.clone()));
        state.images.push(image);
        slots.push(name.to_owned());
        state.replacements.insert(
            original.id(),
            Replacement {
                material: handle,
                slot: name.to_owned(),
                uv: index,
                reads_uv3: resolved.reads_uv3,
            },
        );
    }
    for required in [WALL_SLOT, FLOOR_SLOT] {
        if !slots.iter().any(|slot| slot == required) {
            return Err(format!("房间模块缺少 {required} 材质槽"));
        }
    }
    for gap in &state.named_gaps {
        warn!("[room-shell] {gap}");
    }
    info!(
        "[room-shell] module materials on the source programs: {:?} ({} named above differ from the source); wall {wall_uri} uv{wall_uv}, floor {floor_uri} uv{floor_uv}; module primitives with uv2 {with_uv2}, uv3 {with_uv3}",
        slots,
        state.named_gaps.len()
    );
    Ok(())
}

fn apply(
    selection: Res<SiteSelection>,
    epoch: Option<Res<GroundEpoch>>,
    scenes: Option<Res<SiteScenesReady>>,
    site: Option<Res<SiteAssets>>,
    appearance: Res<RoomAppearance>,
    configs: Option<Res<ClientConfigs>>,
    mut state: ResMut<RoomAppearanceState>,
    server: Res<AssetServer>,
    json: Res<Assets<JsonAsset>>,
    gltfs: Res<Assets<Gltf>>,
    gltf_meshes: Res<Assets<GltfMesh>>,
    mut materials: ResMut<Assets<RoomShellMaterial>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut commands: Commands,
    drawn: Query<
        (
            Entity,
            Option<&MeshMaterial3d<StandardMaterial>>,
            Option<&MeshMaterial3d<RoomShellMaterial>>,
            &Mesh3d,
            Option<&AppliedAppearance>,
        ),
        Or<(
            With<MeshMaterial3d<StandardMaterial>>,
            With<MeshMaterial3d<RoomShellMaterial>>,
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
    let Some(module_path) = server.get_path(module_handle).map(|path| path.to_string()) else {
        state.error = Some("房间模块没有资产路径".into());
        return;
    };
    let Some((module_dir, module_sidecar)) = module_path.strip_suffix(".glb").and_then(|stem| {
        stem.rsplit_once('/')
            .map(|(dir, _)| (dir.to_owned(), format!("{stem}.json")))
    }) else {
        state.error = Some(format!("房间模块路径 {module_path} 不是 .glb"));
        return;
    };
    if !appearance.resolved_for(epoch.0) {
        state.phase = "surface appearances".into();
        return;
    }
    let key = (
        epoch.0,
        appearance.wall.clone(),
        appearance.floor.clone(),
        appearance.wall_color,
        appearance.floor_color,
    );
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
            .chain(std::iter::once(server.load::<JsonAsset>(module_sidecar)))
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
                state.error = Some("房间外观或房间模块的原始资源缺失，请重新提取".into());
                return;
            }
        }
        let Some(configs) = configs.as_deref() else {
            state.phase = "client config".into();
            return;
        };
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
        let appearance = appearance.clone();
        let mut built = RoomAppearanceState {
            key: state.key.take(),
            sources: std::mem::take(&mut state.sources),
            ..default()
        };
        let result = build(
            &documents,
            &appearance,
            &module_dir,
            configs,
            module,
            &gltf_meshes,
            &mut meshes,
            &server,
            &mut materials,
            &mut built,
        );
        *state = built;
        if let Err(reason) = result {
            // Once per key: a refused build is not retried until the site,
            // epoch or appearance changes.
            error!("[room-shell] {reason}");
            state.replacements.clear();
            state.error = Some(reason);
            return;
        }
    }
    // Every mesh is checked before any is swapped. A surface (module
    // material) with a mesh that lacks the uv set its room material samples is
    // refused as a whole, so none of its meshes is swapped; the other surfaces
    // are. A wall mesh without the fourth set is swapped and drawn without the
    // edge factor (its pipeline has no fourth-set input).
    let mut swaps = Vec::new();
    let mut refused_now: Vec<(AssetId<StandardMaterial>, String)> = Vec::new();
    let mut walls_without_uv3 = 0usize;
    for (entity, standard, room, mesh, applied) in &drawn {
        let Some(source) = applied
            .map(|value| value.original)
            .or_else(|| standard.map(|material| material.0.id()))
        else {
            continue;
        };
        if state.refused.contains(&source) {
            continue;
        }
        let Some(replacement) = state.replacements.get(&source).cloned() else {
            continue;
        };
        if room.is_some_and(|material| material.0 == replacement.material) {
            continue;
        }
        let (sampled, set) = match replacement.uv {
            0 => (Mesh::ATTRIBUTE_UV_0, "TEXCOORD_0"),
            1 => (Mesh::ATTRIBUTE_UV_1, "TEXCOORD_1"),
            _ => (room_shell::ATTRIBUTE_UV_2, "TEXCOORD_2"),
        };
        if let Some(mesh) = meshes.get(&mesh.0) {
            if mesh.attribute(sampled).is_none() {
                if !refused_now.iter().any(|(id, _)| *id == source) {
                    refused_now.push((
                        source,
                        format!(
                            "{}: samples {set}, which a mesh drawn with it lacks",
                            replacement.slot
                        ),
                    ));
                }
                continue;
            }
            if replacement.reads_uv3 && mesh.attribute(room_shell::ATTRIBUTE_UV_3).is_none() {
                walls_without_uv3 += 1;
            }
        }
        swaps.push((entity, replacement.material, source));
    }
    swaps.retain(|(_, _, source)| !refused_now.iter().any(|(id, _)| id == source));
    for (id, reason) in refused_now {
        error!("[room-shell] surface refused, kept on its module material: {reason}");
        state.refused.push(id);
        state.refused_names.push(reason);
    }
    state.walls_without_uv3 += walls_without_uv3;
    if state.walls_without_uv3 > 0 && !state.walls_without_uv3_named {
        state.walls_without_uv3_named = true;
        error!(
            "[room-shell] {} wall mesh(es) have no fourth uv set (TEXCOORD_3): the module file predates the uv-set export, so the wall edge factor is left out; re-extract the release data with the current extractor",
            state.walls_without_uv3
        );
    }
    if !swaps.is_empty() {
        info!(
            "[room-shell] {} module meshes switched to the source programs",
            swaps.len()
        );
    }
    for (entity, replacement, source) in swaps {
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
        "images: {:?}; materials={}; named gaps: {:?}; refused surfaces: {:?}; walls without the fourth uv set: {}",
        state
            .images
            .iter()
            .map(|h| server.load_state(h))
            .collect::<Vec<_>>(),
        state.replacements.len(),
        state.named_gaps,
        state.refused_names,
        state.walls_without_uv3
    );
}

impl RoomAppearanceState {
    /// The appearance of the room loaded for `epoch` is applied, or refused.
    pub(crate) fn settled_for(&self, epoch: u64) -> bool {
        self.key.as_ref().is_some_and(|key| key.0 == epoch) && (self.ready || self.error.is_some())
    }
}

pub(crate) fn install(app: &mut App) {
    bevy::asset::embedded_asset!(app, "shaders/room_shell.wgsl");
    crate::gpu_image_release::prepare_after_images::<RoomShellMaterial>(app);
    app.add_plugins(MaterialPlugin::<RoomShellMaterial>::default())
        .init_resource::<RoomAppearance>()
        .init_resource::<RoomAppearanceState>()
        .add_systems(Startup, load_master)
        .add_systems(
            Update,
            (resolve, apply)
                .chain()
                .after(crate::site::spawn_when_ready),
        );
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn skin_names_are_source_keys_not_paths() {
        assert!(safe_skin("mis0001"));
        assert!(!safe_skin("../skin"));
        assert!(!safe_skin("C:/skin"));
    }
}
