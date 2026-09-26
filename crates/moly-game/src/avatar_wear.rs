//! 玩家 avatar 的穿戴集面板（可下发 mock）与挂件装配。
//!
//! **穿戴集是服务端用户态**：真源里玩家穿什么由 `UserAvatar` 的四列
//! Nullable id（costume / skinColor / accessory / coordinate）决定，
//! 经 `AvatarInfoData` 转四列 int（null → 0）写进房间属性
//! （同步键 A_CID/A_SCID/A_AID/A_CDID），`AvatarData.Build` 再按
//! id==0 ⇒ 列 null 还原。按范围通则：机制客户端照原样还原，穿戴集
//! 走可下发 mock 面板具名（MenuMock 形：环境变量下发）。
//!
//! **面板 mock 的是解析后的派生值，不是 id**：id→包名/色码的 master 表
//! （avatarCostumes/…）不在本仓管线（提取缺口，已上报），沿面板先例
//! （rank_level 直发 totalExp 查表的派生结果）直接下发包名与色码。
//! 解析链照真源逐句：
//! * **皮肤包**（玩家链材质合成）：coordinate.costumeAssetbundleName ??
//!   costume.assetbundleName ?? "default"——服装包是**纯贴图换肤**，
//!   包内 `skin` 资产贴 `_SkinTex` 槽，网格不变（master 的 coordinate 行
//!   costumeAssetbundleName = 行自身包名，125 行全表一致 ⇒ 面板的
//!   coordinate 值即派生的皮肤包名）。
//! * **饰品包**：coordinate.accessoryAssetbundleName ??
//!   accessory.assetbundleName ?? 无（真源 key 非空才装载；None 是
//!   一等分支——不挂）。
//! * **肤色调色**：**只读 skinColor.Color，null ⇒ (1,1,1,1)**。玩家链
//!   **不读 coordinate.SkinColor**——那是观众链 Avatar-Color 族的式子，
//!   两链同族不同形（玩家链那段逐句里只有 AvatarData+0x20 一处皮肤色
//!   读取，没有 coordinate+0x48）。
//! * **No penlight.** The MySekai player never holds one, whatever the
//!   server sends: the room sync keys have no penlight column, the penlight
//!   mesh is mounted only by the audience and live chains (no MySekai
//!   player code calls it), and the player view's two penlight colour
//!   setters have no caller in the client. The player view reads the
//!   `Penlight_R`/`Penlight_L` bones only as its arm handles. So there is
//!   no penlight entry on this panel; the material's penlight properties
//!   keep their defaults.
//!
//! **挂件装配形态**：真源玩家链把饰品网格合批进合并网格（part-index
//! 槽 1 进 UV0.z）；本仓合成体 glb 的 part-index 载体是第二套 UV 的
//! x 分量（`avatar_material` 的既有约定）。件网格本身没有该通道（独立件，
//! 真源在合批时才烘）⇒ 装配时运行时注入 `UV_1 = (1,0)`（槽 1），
//! 材质与身体共享同一套 AvatarMaterial 值——同一分派律，网格不合批
//! （提取侧没做合批，运行时做网格手术是另一条路，不在本单）。
//! 饰品挂 `Accessory_face` 骨下（真源对蒙皮件挂同一节点；非蒙皮件的
//! 顶点刚体变换也是按该骨的位置）。
//!
//! **贴图取件**：饰件/荧光棒的 glb 不把贴图接进标准 PBR 槽（只在
//! extras 里，装载器不读）——但内嵌图像由装载器注册为标签子资产
//! `Texture{index}`，按标签取件（不猜文件名）。饰件的 Texture0 即真源
//! accessoryTexture（首个 Renderer 材质的 `_MainTex`）。
//!
//! 闩组件挂在玩家实体上：装配系统等身体换装闩（`AvatarToon`，场景
//! 已展开的等价信号）到位后装配一次，`AccessoryWorn` 防重进；None
//! 分支也闩（不挂是真源的一等分支，面板行已具名）。

use bevy::asset::{AssetPath, RecursiveDependencyLoadState};
use bevy::gltf::{Gltf, GltfMesh};
use bevy::prelude::*;
use std::borrow::Cow;

use crate::avatar_material::{AvatarMaterial, AvatarParams, AvatarToon};
use crate::player::PlayerControlled;
use moly_law::shading::avatar as law;

/// 皮肤包「未设置」的回退值：真源字面量 "default"（皮肤包选择链的兜底臂）。
const DEFAULT_SKIN_BUNDLE: &str = "default";

/// 饰件挂点骨名（真源玩家链 accessoryNode 谓词的名字）。
const ACCESSORY_BONE: &str = "Accessory_face";

// ---------------------------------------------------------------------------
// 面板与资源
// ---------------------------------------------------------------------------

/// 穿戴集：解析后的派生值 + 存活句柄。Startup 一次性解析环境变量，
/// 常驻到进程结束（末句柄丢弃即取消在飞装载 ⇒ 句柄必须活在这里）。
#[derive(Resource)]
pub struct AvatarWear {
    /// 皮肤包名（解析链输出；未设置 ⇒ "default"）。
    pub skin_bundle: String,
    /// 肤色调色 rgba（真源 skinColor.Color；未设置 ⇒ 白）。
    pub skin_color: [f32; 4],
    /// 肤色调色的原始色码（日志用；未设置 ⇒ "none"）。
    pub skin_color_code: Cow<'static, str>,
    /// 饰件包名（None = 真源 null 分支：不挂）。
    pub accessory_bundle: Option<String>,
    /// 皮肤贴图（`_SkinTex`；皮肤包的 `skin` 资产）。
    pub skin_tex: Handle<Image>,
    /// 饰件 glb。
    pub accessory_gltf: Option<Handle<Gltf>>,
    /// 饰件贴图（glb 内嵌 Texture0 = 真源 accessoryTexture）。
    pub accessory_tex: Option<Handle<Image>>,
}

/// 皮肤包内 `skin` 资产的产物路径（提取侧命名约定：`<包>__skin.png`；
/// 真源按资产名 "skin" 从包里取）。
fn skin_tex_path(bundle: &str) -> String {
    format!("moly://avatar/skin/{bundle}/tex/{bundle}__skin.png")
}

/// 饰件包的 glb 路径。
fn decoration_glb_path(bundle: &str) -> String {
    format!("moly://avatar/decoration/{bundle}/{bundle}.glb")
}

/// glb 内嵌图像的标签子资产路径（装载器把全部内嵌贴图注册为
/// `Texture{index}`；不被材质引用的贴图也注册——这是不猜文件名取件
/// 的唯一通道）。
fn glb_texture(glb: &str, index: usize) -> AssetPath<'static> {
    AssetPath::from(glb.to_owned()).with_label(format!("Texture{index}"))
}

/// 面板原始四列（环境变量；全可缺省 = 真源未设置）。
struct WearPanel {
    coordinate: Option<String>,
    costume: Option<String>,
    accessory: Option<String>,
    skin_color: Option<String>,
}

impl WearPanel {
    fn from_env() -> Self {
        WearPanel {
            coordinate: env_str("MOLY_AVATAR_MOCK_COORDINATE"),
            costume: env_str("MOLY_AVATAR_MOCK_COSTUME"),
            accessory: env_str("MOLY_AVATAR_MOCK_ACCESSORY"),
            skin_color: env_str("MOLY_AVATAR_MOCK_SKIN_COLOR"),
        }
    }

    fn shown<'a>(&self, value: &'a Option<String>) -> &'a str {
        value.as_deref().unwrap_or("none")
    }
}

/// 环境变量取字符串（空白视为未设置；MenuMock 形）。
fn env_str(name: &str) -> Option<String> {
    match std::env::var(name) {
        Ok(raw) => {
            let trimmed = raw.trim().to_owned();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed)
            }
        }
        Err(_) => None,
    }
}

/// `#rrggbb` 色码 → srgb [0,1]（master 表 colorCode 的形状）。坏值警告行
/// 带原值、按未设置处理（真源回退白），不静默吞。
fn parse_hex_color(raw: &str) -> Option<[f32; 4]> {
    let hex = raw.strip_prefix('#').unwrap_or(raw);
    if hex.len() != 6 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        warn!(
            "[player] 穿戴集面板：色码 {raw:?} 不是 #rrggbb，按未设置处理（真源回退白）"
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

/// Startup：解析面板、发装载请求、落资源，并打面板行（穿戴集的具名读数：
/// 改面板值 → 这一行与装配行一起变，即「面板驱动装配」的日志判据）。
pub fn load(mut commands: Commands, server: Res<AssetServer>) {
    let panel = WearPanel::from_env();
    // 皮肤包（真源玩家链）：coordinate.costume ?? costume ?? "default"。
    // master 的 coordinate 行 costumeAssetbundleName = 行自身包名 ⇒ 面板的
    // coordinate 值即派生皮肤包名。
    let skin_bundle = panel
        .coordinate
        .clone()
        .or_else(|| panel.costume.clone())
        .unwrap_or_else(|| DEFAULT_SKIN_BUNDLE.to_owned());
    // 饰件包（真源装配链）：coordinate.accessory ?? accessory ?? 无。
    let accessory_bundle = panel.coordinate.clone().or_else(|| panel.accessory.clone());
    // 肤色调色：玩家链只读 skinColor.Color（未设置 ⇒ 白）；不读
    // coordinate 的色（观众链的式子，见模块头注）。
    let skin_color = panel
        .skin_color
        .as_deref()
        .and_then(parse_hex_color)
        .unwrap_or([1.0, 1.0, 1.0, 1.0]);
    let skin_color_code = Cow::from(
        panel
            .skin_color
            .clone()
            .unwrap_or_else(|| "#ffffff".to_owned()),
    );

    // 皮肤贴图只由 GPU 采样（avatar 材质取渲染世界里的纹理），没有读纹素的
    // CPU 读者，也没有别的请求方。
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
        "[player] 穿戴集面板（mock 下发）：coordinate={} costume={} accessory={} skinColor={} ⇒ 皮肤包 {skin_bundle} / 调色 {skin_color_code} / 饰件 {}",
        panel.shown(&panel.coordinate),
        panel.shown(&panel.costume),
        panel.shown(&panel.accessory),
        panel.shown(&panel.skin_color),
        accessory_bundle.as_deref().unwrap_or("无（不挂）"),
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

/// 饰件装配闩（玩家实体；None 分支也闩——不挂是真源一等分支）。
#[derive(Component)]
pub struct AccessoryWorn;

/// 挂件网格实体标记：身体换装扫描的排除面（饰件走 AvatarMaterial 槽 1，
/// 不该被身体换装扫走；且件网格无 part-index 通道，被扫到会在判据处
/// 响亮拒绝）。
#[derive(Component)]
pub struct WearPart;

// ---------------------------------------------------------------------------
// 饰件装配
// ---------------------------------------------------------------------------

/// Update：身体换装闩到位后装饰品。装载失败响亮 panic（未提取件具名
/// 拒绝）；glb 未到齐静默等下一帧。
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub(crate) fn attach_accessory(
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
            info!("[player] avatar 挂件：饰品未设置（真源 null 分支，不挂）");
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
        // 真源取 prefab 里第一个 MeshFilter 的网格；装载器的 meshes 按文件序
        // 排列，meshes[0] 即场景节点序里第一件。
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
        // part-index 注入：槽 1（真源在合批时把 part 序号烘进 UV0.z；本仓
        // 载体是第二套 UV 的 x 分量，件网格原生没有该通道）。
        let vertices = mesh.count_vertices();
        mesh.insert_attribute(
            Mesh::ATTRIBUTE_UV_1,
            vec![[1.0f32, 0.0f32]; vertices],
        );
        let mesh_handle = mesh_assets.add(mesh);
        // 材质与身体同一套值（真源单网格单材质的分派律；槽 1 喂饰件贴图）。
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
