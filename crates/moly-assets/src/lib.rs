//! 提取产物到 Bevy Asset 的翻译层。加载失败是一个 error，不是登记表。
//!
//! 资产根怎么来——native 的 env、web 的 URL 参数——归入口 crate（moly-app）
//! 解析；这里只消费解析结果，拒绝全树只在那一处发生。

#[cfg(target_arch = "wasm32")]
mod http;
#[cfg(target_arch = "wasm32")]
mod http_directory;
#[cfg(any(target_arch = "wasm32", test))]
mod http_path;
pub mod material_passes;
pub mod material_textures;
mod packs;
pub mod player_data;
mod read_limits;
pub mod scene_state;
pub mod sidecar;
pub mod source_navigation;
pub mod ui_layout;

use bevy::app::App;
use bevy::asset::{io::AssetSourceBuilder, AssetApp, AssetPath};
use bevy::prelude::Resource;

pub mod character;
pub mod json;

/// 资产源 id，即 `moly://…` 路径的 scheme。
const SOURCE: &str = "moly";

/// 解析后的资产根：入口 crate 造它，这里装它，消费方读它。
#[derive(Clone, Resource)]
pub enum AssetSource {
    /// native：提取产物目录（入口已验存在且是目录）。
    NativeDir {
        path: std::path::PathBuf,
    },
    /// web：同源绝对路径前缀（保证以 `/` 结尾）——`platform_default`
    /// 在 wasm 上就是按页源取 HTTP 的读取器。
    HttpBase {
        url: String,
    },
    NativePacks {
        path: std::path::PathBuf,
    },
    HttpPacks {
        url: String,
    },
}

/// 装上 `moly` 资产源并把解析结果挂成资源。
///
/// 必须在 `DefaultPlugins` 之前调用：AssetPlugin 构建时固化全部资产源，
/// 之后注册只打一行 error 并被丢弃。
pub fn install(app: &mut App, source: AssetSource) {
    #[cfg(target_arch = "wasm32")]
    if let AssetSource::HttpBase { url } = &source {
        let root = url.clone();
        app.register_asset_source(
            SOURCE,
            AssetSourceBuilder::new(move || {
                Box::new(http_directory::HttpDirectoryReader::new(root.clone()))
            }),
        );
        app.insert_resource(source);
        return;
    }
    #[cfg(target_arch = "wasm32")]
    if let AssetSource::HttpPacks { url } = &source {
        let root = url.clone();
        app.register_asset_source(
            SOURCE,
            AssetSourceBuilder::new(move || Box::new(packs::PackReader::http(root.clone()))),
        );
        app.insert_resource(source);
        return;
    }
    let root = match &source {
        AssetSource::NativeDir { path } | AssetSource::NativePacks { path } => {
            path.to_string_lossy().to_string()
        }
        AssetSource::HttpBase { url } | AssetSource::HttpPacks { url } => url.clone(),
    };
    let builder = if matches!(
        source,
        AssetSource::NativePacks { .. } | AssetSource::HttpPacks { .. }
    ) {
        let mut reader = bevy::asset::io::AssetSource::get_default_reader(root);
        AssetSourceBuilder::new(move || Box::new(packs::PackReader::new(reader())))
    } else {
        AssetSourceBuilder::platform_default(&root, None)
    };
    app.register_asset_source(SOURCE, builder);
    app.insert_resource(source);
}

/// 站点主场景的资产路径；目录布局沿用提取产物，不是本仓的新约定。
pub fn site_scene(site: &str) -> AssetPath<'static> {
    AssetPath::from(format!("moly://site/scenes/{site}/{site}.glb"))
}

/// 站点 json 侧车的资产路径；与主 glTF 同目录同名，布局沿用提取产物。
/// 同一份文件同时装着 inactiveNodes 名单与材质表，两个消费方各自解析。
pub fn site_scene_json(site: &str) -> AssetPath<'static> {
    AssetPath::from(format!("moly://site/scenes/{site}/{site}.json"))
}

/// sidecar 顶层 `textures[]` 里的一条 URI 转成可装载路径。
pub fn site_texture(site: &str, uri: &str) -> AssetPath<'static> {
    AssetPath::from(format!("moly://site/scenes/{site}/{uri}"))
}

/// 晴天现象的 postprocess 档案；站点全局量里的雾三件来自它。
pub fn sunny_postprocess() -> AssetPath<'static> {
    AssetPath::from("moly://phenomena/001_sunny/postprocess.json".to_owned())
}

/// 角色包清单的资产路径；角色分母的唯一来源。
pub fn character_manifest() -> AssetPath<'static> {
    AssetPath::from("moly://manifest.json".to_owned())
}

/// 角色名册的资产路径；位移段名（待机/走姿）从这里取。
pub fn character_registry() -> AssetPath<'static> {
    AssetPath::from("moly://characters.json".to_owned())
}

/// 单个角色包的资产路径；文件名来自清单行。
pub fn character_glb(glb: &str) -> AssetPath<'static> {
    AssetPath::from(format!("moly://{glb}"))
}

/// 共享动作库的资产路径；剪辑按人形骨名链绑定，全部角色复用一份。
pub fn motion_library() -> AssetPath<'static> {
    AssetPath::from("moly://motion-library.glb".to_owned())
}

/// tweet 主表的资产路径：气泡的数据面（tweet 全表 + 摆设编辑反应池）。
pub fn tweet_master() -> AssetPath<'static> {
    AssetPath::from("moly://tweets.json".to_owned())
}

/// 全量音频的语料账本（`mysekai-audio` 提取命令的产物）：cue→包映射的
/// 账本与上游无包 cue 的具名清单——消费侧缺 cue 日志的分类依据。
pub fn audio_corpus() -> AssetPath<'static> {
    AssetPath::from("moly://phenomena/audio/corpus.json".to_owned())
}

/// partvoice 路由表（提取命令的路由产物）：说话者/家具 → 变体语音包名
/// 的查表面（真源链里包随说话者变体，不随 cue）。消费侧对它 fail-closed：
/// 文件缺席时具名跳过，不静默也不推默认包。
pub fn partvoice_routes() -> AssetPath<'static> {
    AssetPath::from("moly://phenomena/audio/partvoice.json".to_owned())
}

/// ClientConfig 可下发面板的资产路径（`client-config` 提取命令的产物）：
/// 四张类型字典（FloatConfigs/IntConfigs/StringConfigs/BoolConfigs）的
/// 定型面，整型 id 键。键在场与否、值是多少，都以这一份为准——消费侧
/// 不再散写具名常量。
pub fn client_config() -> AssetPath<'static> {
    AssetPath::from("moly://client-config.json".to_owned())
}

/// 生日派对主表的资产路径（`birthday-parties` 提取命令的产物）：站点
/// 地图庆典门的档期数据面。
pub fn birthday_parties() -> AssetPath<'static> {
    AssetPath::from("moly://birthday-parties.json".to_owned())
}

/// UI atlas 页纹图的资产路径（`ui` 提取命令的产物）：`textures/` 下的
/// 整页 PNG，文件名沿用提取产物（含 atlas 名、页号与内容指纹）。
pub fn ui_atlas_page(file: &str) -> AssetPath<'static> {
    AssetPath::from(format!("moly://ui/atlas/textures/{file}"))
}

/// UI atlas 裁件的资产路径：`sprites/<atlas>/<sprite>.png` 的单件裁图，
/// 目录布局沿用提取产物；sprite 名即真源序列化的 spriteName（经打包
/// 身份对齐的那一份账本落名），不是本仓的新约定。
pub fn ui_atlas_sprite(atlas: &str, sprite: &str) -> AssetPath<'static> {
    AssetPath::from(format!("moly://ui/atlas/sprites/{atlas}/{sprite}.png"))
}

/// 动作按钮图标的资产路径（`ui-action-icons` 提取命令的产物）。
///
/// 这一族**不在任何图集里**：真源把每张图标做成独立包、装到 RawImage
/// 上，所以按 Sprite 走的图集导出器看不见它们——「图集 51/51 全在盘」
/// 与「一张动作图标都没有」可以同时成立，本仓为此误判过一次。
/// `name` 是真源那张按钮类型表里的文件名，也是纹理自己的 m_Name；
/// ⚠ 与包名并非逐字符相同（有一件差一个大小写），所以按这个名字取。
pub fn ui_action_icon(name: &str) -> AssetPath<'static> {
    AssetPath::from(format!("moly://ui/action-icon/{name}.png"))
}

/// 家具主表切片的资产路径（`fixture-master-slice` 提取命令的产物）：
/// 家具类别、玩家动作类别、格占三列，按 assetbundleName 认包。
/// 「这件家具有没有交互按钮」只有这一份数据答得出。
pub fn mysekai_fixtures() -> AssetPath<'static> {
    AssetPath::from("moly://mysekai-fixtures.json".to_owned())
}

/// 已提取的完整设计图、道具与唱片主表；行按各自 id 寻址。
pub fn mysekai_blueprints() -> AssetPath<'static> {
    AssetPath::from("moly://mysekai-blueprints.json".to_owned())
}

pub fn mysekai_items() -> AssetPath<'static> {
    AssetPath::from("moly://mysekai-items.json".to_owned())
}

pub fn mysekai_music_records() -> AssetPath<'static> {
    AssetPath::from("moly://mysekai-music-records.json".to_owned())
}

/// 家具模型清单的资产路径：包名与 glb 文件名的对应表。
pub fn fixture_model_index() -> AssetPath<'static> {
    AssetPath::from("moly://fixture-models/index.json".to_owned())
}

/// 家具挂点档案的资产路径（`fixture-attach` 提取命令的产物）：全部包的
/// 挂点侧条目（`loc_start`/`loc_end` 对各自的 local 变换）。家具动作点
/// 世界位的唯一数据源——产品侧拿「实例摆放变换 × 挂点 local」现算。
pub fn fixture_attach_points() -> AssetPath<'static> {
    AssetPath::from("moly://fixture-attach/attach-points.json".to_owned())
}
