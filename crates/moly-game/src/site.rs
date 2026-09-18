//! 站点装载：sites.json 主表驱动的站点选择、glTF 展开与地表定位、房间
//! 模块拼装；多站切换入口（按键与冒烟巡游）也在这里。
//!
//! 一世界九站的位点与语义都在主表（scene 目录、sitePosition、类别、等
//! 级），支持集=主表全量九行，这里只消费它：站点类型 → 场景目录，一次
//! 装载一站。房间类站点（first_floor 一族，真源按 siteType 切到
//! MyRoomSiteController；二三楼与一楼共用一个场景包，只差
//! sitePosition.y）的场景包只有壳——体积与插槽，没有墙和地板；房间本
//! 体按等级从室内件拼装：模块包的两个 prefab scene（mdl_static_floor 与
//! mdl_static_wall，五个等级同构）加该等级的寻路面（室内导航包按等级
//! 落盘的面）。lv_01/02 的寻路面在盘上没有几何，这是 authored 状态不是
//! 缺口——此时模块地板兼寻路面，同一张网格正是 lv_03+ 导航面所引用者。
//!
//! 玩家可行走面（navmesh 约束面）的来源随站点包清单（index.json
//! `scenes[]`）走：grasslands 吃烘档高度网格 heightmesh-0.glb，其余有
//! `walkableGround` 的站吃显式导航输入面。运行时烘焙的站同样有导航
//! 输入，不能因没有离线瓦片而改用带陡壁与底面的渲染地表。室内按等级
//! 选择导航面，只有该等级未提供独立几何时才用模块地板。
//!
//! 站点几何是 site-local 的，世界偏移（sitePosition）归消费侧施加。本仓
//! 一次装载一站且以站为世界原点：广场格位锚定的摆放 mock 与取景数学都
//! 系在这一口径上，施加偏移会把锚点整体挪走。sitePosition 只入账与锚行，
//! 不施加；多站同屏的共存装载未接，挂账。
//!
//! 切换：Tab 循环、数字键直选（支持集内的站），或 `MOLY_SITE_TOUR_SECS`
//! 冒烟巡游仪表（无人值守逐站装载，末站驻留毕干净退出）。切换即拆站：
//! 场景实体连子树撤、站点域资源全清、写新选择让装载计划重走。名册与玩
//! 家实体保留（角色装配链不参与换站），新地面定案后由 npc/player 域的
//! 重播种面落位。

use crate::inactive_nodes::{self, SiteSettled};
use crate::site_material;
use crate::site_sound;
use bevy::app::AppExit;
use bevy::asset::{AssetPath, LoadState, RecursiveDependencyLoadState};
use bevy::ecs::message::MessageWriter;
use bevy::ecs::observer::On;
use bevy::gltf::{Gltf, GltfMesh};
use bevy::prelude::*;
use bevy::scene::{SceneInstanceReady, SceneRoot};
use moly_assets::json::JsonAsset;
use std::collections::HashMap;

/// 缺省站点：入口未传参时装载它（主表的 grasslands 行）。
pub const SITE: &str = "grasslands";

/// 支持集（主表行按 siteType 名对号，与真源的 switch 同形；scene 目录名
/// 是另一列，由行解析给出）。九站=主表全量，顺序即主表行序：Tab 循环、
/// 数字键直选与巡游都按它走。场景包站与房间站两条装载链都在；新站上屏
/// 是在此加名字，不是新链。
const SUPPORTED: [&str; 9] = [
    "home_site",
    "first_floor",
    "second_floor",
    "third_floor",
    "grassland",
    "shore",
    "flower_garden",
    "memorial_place",
    "festival_garden",
];

/// 房间类站点：真源按 siteType 名切到 MyRoomSiteController 的三行。
const ROOM_TYPES: [&str; 3] = ["first_floor", "second_floor", "third_floor"];

/// 采集场（主表 siteType 名；场景目录名是 grasslands，多一个 s——
/// 站点名与目录名不同列）：玩家位移律的采集档按它生效。
const GRASSLAND: &str = "grassland";

/// 模块包的两个 prefab scene 名。五个等级的模块包同构（scene 按 prefab
/// 根命名），缺一个都不是完整房间。
const MODULE_SCENES: [&str; 2] = ["mdl_static_floor", "mdl_static_wall"];

/// glTF 里承载地表的网格名（场景包那一类站点）。相机取景从它出发，
/// 永远不从全场景包围盒出发。
const GROUND: &str = "ground";

/// 地表网格名与 `GROUND` 不同的场景包（逐站对账节点树所得的包内真实
/// 命名，不是缺名）：flowergarden 把地表拆成两片，festivalgarden 的
/// 地表是 base 槽下 ground/floor 的两片广场面。
const GROUND_BY_SCENE: &[(&str, &[&str])] = &[
    ("flowergarden", &["ground1", "ground2"]),
    ("festivalgarden", &["floor_a", "white_line"]),
];

/// 模块包里地板网格名的公共前缀（每等级恰一张：small/medium/large）。
const MODULE_FLOOR_PREFIX: &str = "mdl_site_floor_";

/// 房间站的缺省等级：行内首等级（扩建的初始阶段）。
const DEFAULT_ROOM_LEVEL: u32 = 1;
/// Named offline home starter. A source save/rank owner may supply another
/// stage; this default is never copied into an indoor room selection.
const DEFAULT_OFFLINE_HOME_LEVEL: u32 = 5;

/// 站点包清单 collision[] 里「可行走地面」的角色名：出货烘好 navmesh
/// 的站，这张面就是烘焙输入（与烘档同源）。
const WALKABLE_GROUND_ROLE: &str = "walkableGround";

/// 出货烘档里唯一落盘了高度网格面的场景（站点包清单不记这份文件；八
/// 包普查只有它一份）。其余四户外站的烘档只有 VAND 瓦片字节（运行时
/// 格式，未解析），面走烘焙输入面。
const HEIGHTMESH_SCENES: [&str; 1] = ["grasslands"];

/// 冒烟巡游仪表的驻留秒数（环境变量 `MOLY_SITE_TOUR_SECS`，正数开启）。
/// 宿主侧仪表，与 player/emoticon 域的 `MOLY_*` 同款。
const TOUR_ENV: &str = "MOLY_SITE_TOUR_SECS";

/// 入口下发的站点选择（moly-app 解析 env / URL 参数的产物；缺省值在
/// 那里定，这里不读第二遍——参数拒绝点全树只在入口一处）。
#[derive(Clone, Resource)]
pub struct SiteRequest {
    pub site: String,
    pub room_level: Option<u32>,
    /// Named offline initial-layout input, not a user's rank or saved progress.
    pub offline_home_level: Option<u32>,
    pub content: OfflineSceneContent,
}

/// Product-owned offline population, never an NPC simulation or quality rule.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OfflineSceneContent {
    Compact,
    Full,
}

/// 当前请求的站点：换站入口改写它，装载计划读它。
#[derive(Clone, Resource)]
pub struct SiteSelection {
    site: String,
    /// Resolved/explicit stages by actual site type. Never carry the level of
    /// the map being left into a different floor or reset it on outdoor entry.
    levels: HashMap<String, u32>,
    content: OfflineSceneContent,
}

impl From<SiteRequest> for SiteSelection {
    fn from(request: SiteRequest) -> Self {
        let mut levels = HashMap::new();
        if let Some(level) = request.offline_home_level {
            levels.insert("home_site".to_owned(), level);
        }
        if let Some(level) = request.room_level {
            let target = if is_room(&request.site) {
                request.site.as_str()
            } else {
                "first_floor"
            };
            levels.insert(target.to_owned(), level);
        }
        Self {
            site: request.site,
            levels,
            content: request.content,
        }
    }
}

impl SiteSelection {
    pub(crate) fn for_player_data(data: &moly_assets::player_data::ImportedPlayerData) -> Self {
        Self {
            site: data.sites[0].site_type.clone(),
            levels: data
                .sites
                .iter()
                .map(|site| (site.site_type.clone(), site.level))
                .collect(),
            content: OfflineSceneContent::Compact,
        }
    }

    pub(crate) fn snapshot(&self) -> serde_json::Value {
        serde_json::json!({ "site": self.site, "levels": self.levels,
            "content": if self.content == OfflineSceneContent::Full { "full" } else { "compact" } })
    }

    pub(crate) fn from_snapshot(value: &serde_json::Value) -> Result<Self, String> {
        let site = value["site"]
            .as_str()
            .ok_or("Backup site is missing")?
            .to_owned();
        if !SUPPORTED.contains(&site.as_str()) {
            return Err("Backup site is unknown".into());
        }
        let levels = serde_json::from_value::<HashMap<String, u32>>(value["levels"].clone())
            .map_err(|_| "Backup levels are invalid")?;
        if levels
            .iter()
            .any(|(site, level)| !SUPPORTED.contains(&site.as_str()) || *level == 0)
        {
            return Err("Backup levels are invalid".into());
        }
        let content = match value["content"].as_str() {
            Some("full") => OfflineSceneContent::Full,
            Some("compact") => OfflineSceneContent::Compact,
            _ => return Err("Backup content selection is invalid".into()),
        };
        Ok(Self {
            site,
            levels,
            content,
        })
    }

    pub(crate) fn site_type(&self) -> &str {
        &self.site
    }
    pub(crate) fn content(&self) -> OfflineSceneContent {
        self.content
    }

    fn resolve_temporary_level(&mut self, sites: &Sites) -> Result<u32, String> {
        let row = sites.row(&self.site);
        let level = if let Some(level) = self.levels.get(&self.site) {
            *level
        } else if self.site == "home_site" {
            DEFAULT_OFFLINE_HOME_LEVEL
        } else if is_room(&self.site) {
            DEFAULT_ROOM_LEVEL
        } else if row.levels.len() == 1 {
            row.levels[0]
        } else {
            return Err(format!(
                "site {} requires an explicit expansion stage",
                self.site
            ));
        };
        if !row.levels.contains(&level) {
            return Err(format!(
                "site {} has no authored level {level} (available {:?})",
                self.site, row.levels
            ));
        }
        sites.floor_grid(&self.site, level)?;
        self.levels.insert(self.site.clone(), level);
        Ok(level)
    }

    pub(crate) fn resolve_level(
        &mut self,
        sites: &Sites,
        layouts: &crate::fixture::layouts::SiteFixtureLayouts,
    ) -> Result<u32, String> {
        let row = sites.row(&self.site);
        let level = if let Some(level) = self.levels.get(&self.site) {
            *level
        } else if let Some(level) = layouts.saved_level(row.id)? {
            level
        } else if self.site == "home_site" {
            DEFAULT_OFFLINE_HOME_LEVEL
        } else if self.site == "first_floor" {
            DEFAULT_ROOM_LEVEL
        } else if row.levels.len() == 1 {
            row.levels[0]
        } else {
            return Err(format!(
                "site {} requires an explicit expansion stage",
                self.site
            ));
        };
        if !row.levels.contains(&level) {
            return Err(format!(
                "site {} has no authored level {level} (available {:?})",
                self.site, row.levels
            ));
        }
        layouts.validate_target(
            row.id,
            &self.site,
            sites.floor_grid(&self.site, level)?,
            self.content,
        )?;
        self.levels.insert(self.site.clone(), level);
        Ok(level)
    }

    pub(crate) fn fixture_floor_grid(&self, sites: &Sites) -> Result<FloorGridLayout, String> {
        let level = self
            .levels
            .get(&self.site)
            .ok_or_else(|| format!("site {} expansion stage is not resolved yet", self.site))?;
        sites.floor_grid(&self.site, *level)
    }

    /// 当前选中的站点是不是房间类：名册的游走距离档按它切室内档
    /// （目标域读，见 `npc_objective`）。
    pub(crate) fn is_room(&self) -> bool {
        is_room(&self.site)
    }

    /// 当前选中的站点是不是采集场（grassland，缺省站）：玩家位移律按它
    /// 切采集档——步速换键、动画乘数换档（见 `player`）。
    pub(crate) fn is_grassland(&self) -> bool {
        self.site == GRASSLAND
    }
}

/// 主表一行的消费面。
struct SiteRow {
    id: u32,
    site_type: String,
    category: String,
    /// 场景目录名：主表行以它指向提取产物（glTF 主根与它同名）。
    scene: String,
    /// 现象特效的环境站名（主表 `assetbundleName` 列）：天气粒子链按它
    /// 挑 `unique__<站>` 变体。楼层三行同名（first_floor），正好折回
    /// 现象档案的七个环境站。
    asset_bundle: String,
    position: [f32; 3],
    levels: Vec<u32>,
}

/// The authored `floor` layout for one explicit site level.  The dimensions
/// are grid-cell counts, not a mesh/AABB extent; callers must pass the level
/// selected by the request/save layer rather than picking a convenient row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FloorGridLayout {
    pub level: u32,
    pub layout_id: u32,
    pub width: i32,
    pub height: i32,
    pub depth: i32,
}

/// 场景包站点的玩家可行走面来源（站点包清单 `scenes[]` 逐场景解析；
/// 出货烘好 navmesh 的场景才有——canon 五户外包，home 与室内壳 authored
/// 无烘档，面走地表/模块地板）。
#[derive(Clone)]
enum NavFace {
    /// 烘档的高度网格面（落盘者唯一：grasslands，导航消费的面，河缺口
    /// 保真）。
    Heightmesh,
    /// 烘焙输入面（`_nav_ground` 落盘 glb，站点相对路径）。已证与
    /// grasslands 高度网格逐格同源——就是玩家可行走面。
    BakeInput(String),
}

/// 主表、室内导航包与站点包清单的解析结果（装载一次常驻；换站在它上
/// 面重选）。
#[derive(Resource)]
pub(crate) struct Sites {
    rows: Vec<SiteRow>,
    /// Explicit `(site_type, requested_level)` floor dimensions from
    /// `sites.json`.  No first/max-level fallback is performed by this map.
    floor_layouts: HashMap<(String, u32), FloorGridLayout>,
    /// 房间等级 → 寻路面文件（相对室内导航包目录）。等级缺席或值为
    /// None 都是盘上 authored 状态：该等级的寻路面没有几何可读。
    walkable: HashMap<u32, Option<String>>,
    /// 场景目录名 → 玩家可行走面来源（出货烘档在场的场景才有）。
    nav_faces: HashMap<String, NavFace>,
}

impl Sites {
    /// 只取消费面用得到的列。形状不对响亮拒绝（资产边界的唯一拒绝点），
    /// 报错只带字段名与站点类型，不带主表文本（主表含站点显示名）。
    fn parse(sites: &str, navigation: &str, index: &str) -> Self {
        let value: serde_json::Value = serde_json::from_str(sites)
            .unwrap_or_else(|err| panic!("站点主表不是合法 JSON：{err}"));
        let mut floor_layouts = HashMap::new();
        let rows = value
            .get("sites")
            .and_then(|v| v.as_array())
            .unwrap_or_else(|| panic!("站点主表缺 sites 数组"))
            .iter()
            .map(|row| {
                let field = |name: &str| {
                    row.get(name)
                        .unwrap_or_else(|| panic!("站点主表行缺 {name}"))
                };
                let site_type = field("siteType")
                    .as_str()
                    .unwrap_or_else(|| panic!("站点主表行缺 siteType 字符串"))
                    .to_owned();
                let text = |name: &str| {
                    field(name)
                        .as_str()
                        .unwrap_or_else(|| panic!("站点 {site_type} 的 {name} 不是字符串"))
                        .to_owned()
                };
                let position = field("sitePosition");
                let axis = |name: &str| {
                    position
                        .get(name)
                        .and_then(|v| v.as_f64())
                        .unwrap_or_else(|| panic!("站点 {site_type} 缺 sitePosition.{name}"))
                        as f32
                };
                let level_rows = field("levels")
                    .as_array()
                    .unwrap_or_else(|| panic!("站点 {site_type} 的 levels 不是数组"))
                    .to_owned();
                let levels = level_rows
                    .iter()
                    .map(|level| {
                        let level_id = level
                            .get("level")
                            .and_then(|v| v.as_u64())
                            .unwrap_or_else(|| panic!("站点 {site_type} 的等级行缺 level"))
                            as u32;
                        let layouts = level
                            .get("layouts")
                            .and_then(|v| v.as_array())
                            .unwrap_or_else(|| panic!("站点 {site_type} 等级 {level_id} 缺 layouts 数组"));
                        let floor = layouts.iter().find(|layout| {
                            layout.get("layoutType").and_then(|v| v.as_str()) == Some("floor")
                        }).unwrap_or_else(|| panic!("站点 {site_type} 等级 {level_id} 缺 floor layout"));
                        let layout_id = floor
                            .get("id")
                            .and_then(|v| v.as_u64())
                            .unwrap_or_else(|| panic!("站点 {site_type} 等级 {level_id} 的 floor 缺 id"))
                            as u32;
                        let cells = floor
                            .get("cells")
                            .unwrap_or_else(|| panic!("站点 {site_type} 等级 {level_id} 的 floor 缺 cells"));
                        let cell = |axis: &str| {
                            cells
                                .get(axis)
                                .and_then(|v| v.as_i64())
                                .and_then(|v| i32::try_from(v).ok())
                                .unwrap_or_else(|| panic!("站点 {site_type} 等级 {level_id} 的 floor cells.{axis} 不是 i32"))
                        };
                        floor_layouts.insert((site_type.clone(), level_id), FloorGridLayout {
                            level: level_id,
                            layout_id,
                            width: cell("width"),
                            height: cell("height"),
                            depth: cell("depth"),
                        });
                        level_id
                    })
                    .collect();
                let category = text("category");
                let id = field("id").as_u64().and_then(|id| u32::try_from(id).ok())
                    .filter(|id| *id != 0).unwrap_or_else(|| panic!("站点 {site_type} 缺有效 id"));
                let scene = text("scene");
                let asset_bundle = text("assetbundleName");
                let position = [axis("x"), axis("y"), axis("z")];
                SiteRow {
                    id,
                    category,
                    site_type,
                    scene,
                    asset_bundle,
                    position,
                    levels,
                }
            })
            .collect();
        let nav: serde_json::Value = serde_json::from_str(navigation)
            .unwrap_or_else(|err| panic!("室内导航包清单不是合法 JSON：{err}"));
        let mut walkable = HashMap::new();
        for entry in nav
            .get("collision")
            .and_then(|v| v.as_array())
            .unwrap_or_else(|| panic!("室内导航包清单缺 collision 数组"))
        {
            let root = entry
                .get("root")
                .and_then(|v| v.as_str())
                .unwrap_or_else(|| panic!("室内导航面行缺 root"));
            let level: u32 = root
                .strip_prefix("lv_")
                .and_then(|rest| rest.parse().ok())
                .unwrap_or_else(|| panic!("室内导航面行的 root 不按 lv_NN 命名：{root}"));
            walkable.insert(
                level,
                entry
                    .get("file")
                    .and_then(|v| v.as_str())
                    .map(str::to_owned),
            );
        }
        // 导航输入与离线烘档是两种独立资产。显式 walkableGround 在运行时
        // 烘焙的站也有效，不能用瓦片是否在场代替导航输入是否在场。
        let index_value: serde_json::Value = serde_json::from_str(index)
            .unwrap_or_else(|err| panic!("站点包清单不是合法 JSON：{err}"));
        let mut nav_faces = HashMap::new();
        let scenes = index_value
            .get("scenes")
            .and_then(|v| v.as_object())
            .unwrap_or_else(|| panic!("站点包清单缺 scenes 表"));
        for (scene, entry) in scenes {
            let baked = entry
                .get("navmesh")
                .and_then(|v| v.as_array())
                .is_some_and(|tiles| !tiles.is_empty());
            let ground = entry
                .get("collision")
                .and_then(|v| v.as_array())
                .and_then(|rows| {
                    rows.iter().find(|row| {
                        row.get("role").and_then(|r| r.as_str()) == Some(WALKABLE_GROUND_ROLE)
                    })
                });
            let face = if baked && HEIGHTMESH_SCENES.contains(&scene.as_str()) {
                NavFace::Heightmesh
            } else if let Some(ground) = ground {
                let file = ground
                    .get("file")
                    .and_then(|f| f.as_str())
                    .unwrap_or_else(|| {
                        panic!("站点包清单：场景 {scene} 的 role={WALKABLE_GROUND_ROLE} 缺几何文件")
                    });
                NavFace::BakeInput(file.to_owned())
            } else if baked {
                panic!("站点包清单：场景 {scene} 的烘档在场但没有 role={WALKABLE_GROUND_ROLE} 的碰撞面行")
            } else {
                continue;
            };
            nav_faces.insert(scene.clone(), face);
        }
        Self {
            rows,
            floor_layouts,
            walkable,
            nav_faces,
        }
    }

    fn row(&self, site_type: &str) -> &SiteRow {
        self.rows
            .iter()
            .find(|row| row.site_type == site_type)
            .unwrap_or_else(|| panic!("主表没有站点类型 {site_type}（支持集 {SUPPORTED:?}）"))
    }

    /// 行查找的不响亮形：换站入口要在运行时核目标行，查无此站只能放弃
    /// 装载判断，不能 panic（装载计划的入口参数拒绝语义不受影响）。
    fn row_opt(&self, site_type: &str) -> Option<&SiteRow> {
        self.rows.iter().find(|row| row.site_type == site_type)
    }

    pub(crate) fn site_id(&self, site_type: &str) -> Option<u32> {
        self.row_opt(site_type).map(|row| row.id)
    }

    /// 场景包站点的可行走面来源；authored 无烘档的场景返回 None。
    fn nav_face(&self, scene: &str) -> Option<&NavFace> {
        self.nav_faces.get(scene)
    }

    /// Read an authored floor grid for an explicit request level. Absence is
    /// an input/data error; this method intentionally never chooses first/max.
    pub(crate) fn floor_grid(
        &self,
        site_type: &str,
        requested_level: u32,
    ) -> Result<FloorGridLayout, String> {
        if self.row_opt(site_type).is_none() {
            return Err(format!(
                "site type {site_type} is not in the loaded site table"
            ));
        }
        self.floor_layouts
            .get(&(site_type.to_owned(), requested_level))
            .copied()
            .ok_or_else(|| {
                format!("site {site_type} has no authored floor layout for level {requested_level}")
            })
    }
}

/// 当前站点的账目面：锚行与日志的名字、位点、房间拼装来源都读它。
#[derive(Clone, Resource)]
pub struct SiteActive {
    pub site_id: u32,
    pub level: u32,
    pub site_type: String,
    pub category: String,
    /// 场景目录名：inactiveNodes 路径以它为前缀（glTF 主根同名）。
    pub scene: String,
    /// 环境站名（主表 `assetbundleName` 列）：天气粒子链按它挑现象档案
    /// 的 `unique__<站>` 变体。
    pub env_site: String,
    pub position: [f32; 3],
    /// 房间类站点的拼装账目；场景包站点为 None。
    pub room: Option<RoomInfo>,
    /// 场景包站点的可行走面账目名（锚行报它，面从哪来逐站可对账）；
    /// 房间站的寻路面账目在 [`RoomInfo`] 里，此列为 None。
    pub nav_face: Option<String>,
}

/// 房间等级与寻路面来源（锚行报它：寻路面从哪来是可对账的）。
#[derive(Clone)]
pub struct RoomInfo {
    pub level: u32,
    /// Some = 室内导航包按等级落盘的面文件；None = 盘上 authored 无几何，
    /// 模块地板兼寻路面。
    pub walkable_file: Option<String>,
}

/// 已请求装载的站点资产族。字段随站点类别而异：房间站多模块与寻路面，
/// 户外站多 navmesh 面。主 glTF 字段开放给同 crate 的换装系统读
/// `named_materials`。
#[derive(Resource)]
pub struct SiteAssets {
    pub(crate) gltf: Handle<Gltf>,
    /// 场景包站的地表网格名（逐站对账，见 `GROUND`/`GROUND_BY_SCENE`）；
    /// 房间站为空（地表来自模块）。
    grounds: Vec<String>,
    pub(crate) module: Option<Handle<Gltf>>,
    walkable: Option<Handle<Gltf>>,
    navmesh: Option<Handle<Gltf>>,
    /// Source HomeSiteObstacleController.UpdateView: rank >= site level.
    home_obstacle_levels: Vec<u32>,
}

/// 地表网格的全部 primitive；站点展开后常驻——它同时是「站点已展开」
/// 的闩（换站撤下它，闩在下一站重新起跳）。
#[derive(Resource)]
pub struct GroundMeshes(pub Vec<Handle<Mesh>>);

/// 玩家可行走面（navmesh 约束面）的网格 primitive；与 [`GroundMeshes`]
/// 同批展开，消费在 `walk_face` 域。户外站来自站点包清单解析出的面源
/// （高度网格或烘焙输入面），居住类站与地表同源（authored 无 navmesh，
/// 真源运行时烘——地表即面的替身）。
#[derive(Resource)]
pub struct WalkFaceMeshes(pub Vec<Handle<Mesh>>);

/// 站点场景实体标记（场景包站一个；房间站的壳、模块两面、寻路面各一）。
#[derive(Component)]
pub struct SiteRoot;

/// A renderable scene root is withheld until its source materials are installed.
/// Navigation-only roots never carry this marker and stay hidden permanently.
/// Scene instantiation and navigation do not depend on presentation visibility.
#[derive(Component)]
pub(crate) struct SiteVisualPending;

/// scene 实例已展开完毕（全部站点 scene 就绪）；由 `on_scene_ready` 计数
/// 置位。
#[derive(Resource)]
pub struct SiteReady;

/// 全部站点 scene 就绪的常驻闩：站点域外按它对齐时机（站点材质换装的
/// 门）。与 [`SiteReady`] 同时置位，但不被清扫链撤下——换站拆资源时才撤，
/// 新一代 scene 全就绪再立。
#[derive(Resource)]
pub struct SiteScenesReady;

/// 尚未展开完毕的站点 scene 数。
#[derive(Resource)]
pub(crate) struct SiteScenePending(usize);

/// 站点内容定案的代数：每次定案（inactiveNodes 清扫完成）自增，跨站单调。
/// 换站后的重播种面与锚行以它为门——`SiteSettled` 同帧被取景行撤下，
/// Update 侧等不到（时机全解在 inactive_nodes 模块注释）。
#[derive(Clone, Copy, Resource)]
pub struct GroundEpoch(pub u64);

/// Startup：请求主表、室内导航包与站点包清单三份 JSON。站点 glTF 待主
/// 表解析后由 [`plan`] 按选择请求。
pub fn load(mut commands: Commands, server: Res<AssetServer>) {
    commands.insert_resource(MasterHandles {
        sites: server.load::<JsonAsset>(AssetPath::from("moly://site/sites.json".to_owned())),
        navigation: server.load::<JsonAsset>(AssetPath::from(
            "moly://site/indoor/navigation/navigation_mesh/navigation_mesh.json".to_owned(),
        )),
        index: server.load::<JsonAsset>(AssetPath::from("moly://site/index.json".to_owned())),
    });
}

/// 三份清单的装载请求；解析成功后即撤。
#[derive(Resource)]
pub(crate) struct MasterHandles {
    sites: Handle<JsonAsset>,
    navigation: Handle<JsonAsset>,
    index: Handle<JsonAsset>,
}

/// Update：三份清单到齐后解析一次。装载失败响亮 panic，未到齐静默等
/// 下一帧。
pub(crate) fn parse_masters(
    mut commands: Commands,
    server: Res<AssetServer>,
    jsons: Res<Assets<JsonAsset>>,
    handles: Option<Res<MasterHandles>>,
) {
    let Some(handles) = handles else {
        return; // 未请求，或已解析并撤下
    };
    let (Some(sites), Some(navigation), Some(index)) = (
        jsons.get(&handles.sites),
        jsons.get(&handles.navigation),
        jsons.get(&handles.index),
    ) else {
        for (name, handle) in [
            ("站点主表", &handles.sites),
            ("室内导航包清单", &handles.navigation),
            ("站点包清单", &handles.index),
        ] {
            if let LoadState::Failed(err) = server.load_state(handle) {
                panic!("{name}装载失败：{err:?}");
            }
        }
        return; // 还在装
    };
    commands.insert_resource(Sites::parse(&sites.0, &navigation.0, &index.0));
    commands.remove_resource::<MasterHandles>();
}

/// Update：装载计划。站点域资源在（首站或换站后）缺 [`SiteAssets`] 时按
/// 当前选择重建：解析行、算房间拼装面、请求全部 glTF 与侧车。选择在支
/// 持集外或等级不在行内是入口参数错误，响亮 panic。
pub(crate) fn plan(
    mut commands: Commands,
    server: Res<AssetServer>,
    sites: Option<Res<Sites>>,
    selection: Option<ResMut<SiteSelection>>,
    assets: Option<Res<SiteAssets>>,
    layouts: Res<crate::fixture::layouts::SiteFixtureLayouts>,
    temporary: Option<Res<TemporarySiteActive>>,
    mut last_input_error: Local<Option<String>>,
) {
    if assets.is_some() {
        return;
    }
    let (Some(sites), Some(mut selection)) = (sites, selection) else {
        return;
    };
    let site_level = match if temporary.is_some() {
        selection.resolve_temporary_level(&sites)
    } else {
        selection.resolve_level(&sites, &layouts)
    } {
        Ok(level) => {
            *last_input_error = None;
            level
        }
        Err(error) => {
            if last_input_error.as_ref() != Some(&error) {
                error!("[site] {error}");
            }
            *last_input_error = Some(error);
            return;
        }
    };
    let row = sites.row(&selection.site);
    let room = is_room(&row.site_type).then(|| RoomInfo {
        level: site_level,
        walkable_file: sites.walkable.get(&site_level).cloned().flatten(),
    });
    let module = room.as_ref().map(|_| {
        server.load::<Gltf>(AssetPath::from(format!(
            "moly://site/indoor/modules/lv_{:02}/lv_{:02}.glb",
            site_level, site_level
        )))
    });
    let walkable = room
        .as_ref()
        .and_then(|room| room.walkable_file.as_ref())
        .map(|file| {
            server.load::<Gltf>(AssetPath::from(format!(
                "moly://site/indoor/navigation/navigation_mesh/{file}"
            )))
        });
    // 玩家可行走面来自清单中的显式导航输入（草原另用烘档高度网格）。
    // 有无离线瓦片不影响独立导航输入的装载；无独立面时才沿用地表。
    let navmesh = sites.nav_face(&row.scene).map(|face| match face {
        NavFace::Heightmesh => server.load::<Gltf>(AssetPath::from(format!(
            "moly://site/scenes/{}/navmesh/heightmesh-0.glb",
            row.scene
        ))),
        NavFace::BakeInput(file) => {
            server.load::<Gltf>(AssetPath::from(format!("moly://site/{file}")))
        }
    });
    let nav_face_name = sites.nav_face(&row.scene).map(|face| match face {
        NavFace::Heightmesh => "navmesh/heightmesh-0.glb".to_owned(),
        NavFace::BakeInput(file) => file.clone(),
    });
    let grounds = if is_room(&row.site_type) {
        Vec::new()
    } else {
        ground_names(&row.scene)
            .iter()
            .map(|s| (*s).to_owned())
            .collect()
    };
    commands.insert_resource(SiteActive {
        site_id: row.id,
        level: site_level,
        site_type: row.site_type.clone(),
        category: row.category.clone(),
        scene: row.scene.clone(),
        env_site: row.asset_bundle.clone(),
        position: row.position,
        room,
        nav_face: nav_face_name,
    });
    commands.insert_resource(SiteAssets {
        gltf: server.load::<Gltf>(moly_assets::site_scene(&row.scene)),
        grounds,
        module,
        walkable,
        navmesh,
        home_obstacle_levels: if row.site_type == "home_site" {
            row.levels
                .iter()
                .copied()
                .filter(|rank| *rank >= site_level)
                .collect()
        } else {
            Vec::new()
        },
    });
    // 侧车（inactiveNodes 名单、材质表与 A 套声源表）跟着站点走：换站后
    // 重新起请求。
    inactive_nodes::request(&mut commands, &server, &row.scene);
    site_material::request(&mut commands, &server, &row.scene);
    site_sound::request(&mut commands, &server, &row.scene);
}

/// Update：glTF 族及依赖到齐后展开 scene 并记下寻路面网格。到齐判据用
/// AssetServer 的依赖图；每个柄独立过门，失败具名 panic。
pub fn spawn_when_ready(
    mut commands: Commands,
    server: Res<AssetServer>,
    gltfs: Res<Assets<Gltf>>,
    gltf_meshes: Res<Assets<GltfMesh>>,
    assets: Option<Res<SiteAssets>>,
    spawned: Option<Res<GroundMeshes>>,
) {
    if spawned.is_some() {
        return;
    }
    let Some(assets) = assets else {
        return;
    };
    if !ready(&server, &assets.gltf, "站点主 glTF") {
        return;
    }
    // navmesh 面极小，到齐只会早于主包；先过门再发任何展开命令，避免
    // 半途 return 把主 scene 铺两遍。
    if let Some(handle) = &assets.navmesh {
        if !ready(&server, handle, "可行走面 glTF") {
            return;
        }
    }
    let Some(gltf) = gltfs.get(&assets.gltf) else {
        return;
    };
    // glTF 的语义单位是默认 scene；没有它，这个文件对任何查看器都不是站点。
    let Some(scene) = gltf.default_scene.clone() else {
        panic!("站点主 glTF 没有默认 scene");
    };
    // A room can wait for its module or navigation after its main scene is
    // loaded. Plan all roots before emitting commands, so that a pending asset
    // cannot leave a partial expansion to be spawned again next frame.
    let mut scenes = vec![(scene, false)];
    // These are the original authored obstacle prefabs, not scaled terrain or
    // generated fences. Default home L5 adds only the authored-empty rank5 root.
    for rank in &assets.home_obstacle_levels {
        let name = format!("rank{rank}");
        let obstacle = gltf
            .named_scenes
            .get(name.as_str())
            .unwrap_or_else(|| panic!("home glTF lacks authored obstacle scene {name}"));
        scenes.push((obstacle.clone(), false));
    }
    let (ground, face) = match (&assets.module, &assets.walkable) {
        // 场景包站点：地表是主 glTF 里承载地表的网格（相机取景的锚，名
        // 单逐站对账）。有独立导航输入的站另装可行走面
        // ——隐藏实体，只采样不进画面（真源里它也只在导航上被读）；
        // 清单未提供独立导航输入时才让地表兼面。
        (None, None) => {
            let mut ground = Vec::new();
            for name in &assets.grounds {
                let Some(handle) = gltf.named_meshes.get(name.as_str()) else {
                    panic!("站点主 glTF 没有名为 {name} 的地表网格");
                };
                ground.extend(ground_meshes(&gltf_meshes, handle));
            }
            let face = match &assets.navmesh {
                Some(handle) => {
                    let Some(navmesh) = gltfs.get(handle) else {
                        return;
                    };
                    let Some((_, mesh)) = navmesh.named_meshes.iter().next() else {
                        panic!("可行走面 glTF 没有网格");
                    };
                    let face = ground_meshes(&gltf_meshes, mesh);
                    let Some(scene) = navmesh.default_scene.clone() else {
                        panic!("可行走面 glTF 没有默认 scene");
                    };
                    scenes.push((scene, true));
                    Some(face)
                }
                None => None,
            };
            (ground, face)
        }
        // 房间站：壳之外再拼模块两面（按场景名定位），寻路面按账目来源取。
        (Some(module), walkable) => {
            if !ready(&server, module, "房间模块 glTF") {
                return;
            }
            let Some(module) = gltfs.get(module) else {
                return;
            };
            for name in MODULE_SCENES {
                let Some(scene) = module.named_scenes.get(name) else {
                    panic!("房间模块 glTF 没有名为 {name} 的 scene");
                };
                scenes.push((scene.clone(), false));
            }
            match walkable {
                // 寻路面有落盘几何：铺成不可见实体（寻路面采样只读顶点，
                // 不进画面——真源里它也只在导航烘焙时被读）。
                Some(handle) => {
                    if !ready(&server, handle, "室内寻路面 glTF") {
                        return;
                    }
                    let Some(walkable) = gltfs.get(handle) else {
                        return;
                    };
                    let Some((_, mesh)) = walkable.named_meshes.iter().next() else {
                        panic!("室内寻路面 glTF 没有网格");
                    };
                    let meshes = ground_meshes(&gltf_meshes, mesh);
                    let Some(scene) = walkable.default_scene.clone() else {
                        panic!("室内寻路面 glTF 没有默认 scene");
                    };
                    scenes.push((scene, true));
                    (meshes, None)
                }
                // authored 无几何：模块地板兼寻路面（画面上它本来就是地板）。
                None => {
                    let name = module
                        .named_meshes
                        .keys()
                        .find(|name| name.starts_with(MODULE_FLOOR_PREFIX))
                        .cloned()
                        .unwrap_or_else(|| {
                            panic!("房间模块 glTF 没有 {MODULE_FLOOR_PREFIX} 前缀的地板网格")
                        });
                    let mesh = module.named_meshes.get(&name).expect("键取自同一张表");
                    (ground_meshes(&gltf_meshes, mesh), None)
                }
            }
        }
        // 模块与寻路面成对构造（计划侧同源于 room 账目），残缺是组装层坏了。
        _ => unreachable!("站点资产族残缺：module 与 walkable 不成对"),
    };
    if ground.is_empty() {
        panic!("寻路面网格未随 glTF 到达");
    }
    // 未提供独立导航输入时，地表网格兼可行走面；室内模块地板也走此臂。
    let face = face.unwrap_or_else(|| ground.clone());
    let pending = scenes.len();
    for (scene_index, (scene, hidden)) in scenes.into_iter().enumerate() {
        let mut root = commands.spawn((SceneRoot(scene), SiteRoot, Visibility::Hidden));
        if scene_index == 0 {
            root.insert(crate::fixture_scene_inputs::SiteCoordinateOrigin);
        }
        if !hidden {
            root.insert(SiteVisualPending);
        }
    }
    commands.insert_resource(GroundMeshes(ground));
    commands.insert_resource(WalkFaceMeshes(face));
    commands.insert_resource(SiteScenePending(pending));
}

/// 一条网格柄的全部 primitive 地表。
fn ground_meshes(
    gltf_meshes: &Res<Assets<GltfMesh>>,
    ground: &Handle<GltfMesh>,
) -> Vec<Handle<Mesh>> {
    let Some(ground) = gltf_meshes.get(ground) else {
        panic!("地表网格未随 glTF 到达");
    };
    ground.primitives.iter().map(|p| p.mesh.clone()).collect()
}

/// 场景包站承载地表的网格名（逐站对账：多数包恰一片 `ground`，两个包
/// 另有命名——[`GROUND_BY_SCENE`]）。
fn ground_names(scene: &str) -> &'static [&'static str] {
    GROUND_BY_SCENE
        .iter()
        .find(|(name, _)| *name == scene)
        .map(|(_, names)| *names)
        .unwrap_or(&[GROUND])
}

/// 一个柄的依赖图到齐判据；失败具名 panic（资产边界的唯一拒绝点）。
fn ready(server: &Res<AssetServer>, handle: &Handle<Gltf>, label: &str) -> bool {
    if let LoadState::Failed(err) = server.load_state(handle) {
        panic!("{label}装载失败：{err:?}");
    }
    if let RecursiveDependencyLoadState::Failed(err) =
        server.recursive_dependency_load_state(handle)
    {
        panic!("{label}的依赖装载失败：{err:?}");
    }
    server.is_loaded_with_dependencies(handle)
}

/// 全局观察者：站点 scene 实例逐个展开完毕，计数归零置位 [`SiteReady`]。
/// 事件是全球触发的，用根实体标记认出本站的实例；计数资源在场的世代才
/// 计数——拆站后的迟到事件实体已撤，查不到根自然落空。
pub(crate) fn on_scene_ready(
    trigger: On<SceneInstanceReady>,
    roots: Query<(), With<SiteRoot>>,
    pending: Option<ResMut<SiteScenePending>>,
    mut commands: Commands,
) {
    if roots.get(trigger.event().entity).is_err() {
        return;
    }
    let Some(mut pending) = pending else {
        return;
    };
    pending.0 = pending.0.saturating_sub(1);
    info!(
        "[site-ready] scene root {:?} ready; {} roots remaining",
        trigger.event().entity,
        pending.0
    );
    if pending.0 == 0 {
        commands.insert_resource(SiteReady);
        commands.insert_resource(SiteScenesReady);
        commands.remove_resource::<SiteScenePending>();
    }
}

/// Update：换站入口。Tab 循环、数字键直选、巡游仪表到点，三路汇成一次
/// 拆站重选：场景实体连子树撤、站点域资源全清、写新选择让 [`plan`] 重
/// 走装载链。名册、玩家、家具与一切站点外域不动。
pub(crate) fn read_switch(
    mut commands: Commands,
    keys: Res<ButtonInput<KeyCode>>,
    time: Res<Time>,
    mut tour: Local<Tour>,
    selection: Res<SiteSelection>,
    roots: Query<Entity, With<SiteRoot>>,
    active: Option<Res<SiteActive>>,
    sites: Option<Res<Sites>>,
    epoch: Option<Res<GroundEpoch>>,
    pending: Option<Res<SiteChangeRequest>>,
    preview: Option<Res<TemporarySiteChangeRequest>>,
    mut exit: MessageWriter<AppExit>,
    edit: Option<Res<crate::fixture_edit::EditSession>>,
    layouts: Res<crate::fixture::layouts::SiteFixtureLayouts>,
    panel: Res<crate::game_settings::SettingsPanel>,
    library: Res<crate::content_library::ContentLibrary>,
) {
    let manual_input = !panel.blocks_world_input() && !library.blocks_world_input();
    let mut requested = manual_input
        .then(|| key_request(&keys, active.as_deref()))
        .flatten();
    if requested.is_none() && manual_input {
        requested = tour_request(&mut tour, &time, active.as_deref(), &epoch, &mut exit);
    }
    // 小地图点的名与按键同级；请求无论是否抢先落地都当帧撤（不跨帧）。
    let temporary_request = preview.as_deref().map(|request| request.0.clone());
    if let Some(pending) = pending.as_deref() {
        if requested.is_none() {
            requested = Some(pending.0.clone());
        }
        commands.remove_resource::<SiteChangeRequest>();
        commands.remove_resource::<TemporarySiteChangeRequest>();
    } else if let Some(site) = temporary_request.as_ref() {
        requested = Some(site.clone());
        commands.remove_resource::<TemporarySiteChangeRequest>();
    }
    let Some(site) = requested else {
        return;
    };
    // A preview is a fresh, unsaved scene even when its site name is the same.
    // Returning from that preview must likewise rebuild the saved original.
    let reload_same_site = preview.is_some() || (pending.is_some() && library.owns_scene());
    if site == selection.site && !reload_same_site {
        return;
    }
    if sites
        .as_deref()
        .and_then(|sites| sites.row_opt(&site))
        .is_none()
    {
        warn!("[site] cannot switch to unknown/not-yet-loaded site {site}");
        return;
    }
    if edit
        .as_deref()
        .is_some_and(crate::fixture_edit::has_unsaved_layout)
    {
        warn!("[site] layout has unsaved changes; save this map before switching");
        return;
    }
    // Admission is side-effect-free. An incompatible target, malformed save
    // or unknown package cannot tear down the map the user is currently in.
    let temporary = pending.is_none() && temporary_request.as_deref() == Some(site.as_str());
    let mut next = (*selection).clone();
    next.site = site;
    let Some(sites) = sites.as_deref() else {
        return;
    };
    let resolved = if temporary {
        next.resolve_temporary_level(sites)
    } else {
        next.resolve_level(sites, &layouts)
    };
    if let Err(error) = resolved {
        warn!("[site] switch refused: {error}; current map and saved data were retained");
        return;
    }
    if temporary {
        commands.insert_resource(TemporarySiteActive);
    } else {
        // Do not change source identity until normal admission succeeds.
        commands.remove_resource::<TemporarySiteActive>();
    }
    queue_transition(&mut commands, roots.iter().collect(), next);
    tour.hops += 1;
}

/// Queue the shared scene teardown before publishing a new site selection.
pub(crate) fn queue_transition(commands: &mut Commands, roots: Vec<Entity>, next: SiteSelection) {
    // Release furniture ownership and detach the logical player before any
    // scene subtree disappears or the next site reseeds that same player.
    commands.queue(crate::player_fixture_action::cancel_for_site_change);
    commands.queue(crate::npc_fixture_activity::cancel_for_site_change);
    commands.queue(crate::fixture_gimmick::cancel_for_site_change);
    commands.queue(crate::fixture_scene_inputs::invalidate_for_site_change);
    commands.queue(crate::fixture::clear_for_site_change);
    for root in roots {
        commands.entity(root).despawn();
    }
    commands.remove_resource::<SiteAssets>();
    commands.remove_resource::<GroundMeshes>();
    commands.remove_resource::<SiteReady>();
    commands.remove_resource::<SiteScenesReady>();
    commands.remove_resource::<SiteSettled>();
    commands.remove_resource::<SiteScenePending>();
    crate::walk_face::teardown(commands);
    site_material::teardown(commands);
    site_sound::teardown(commands);
    crate::uber_particle::teardown(commands);
    crate::weather_fx::teardown(commands);
    // GroundEpoch stays monotonic across the transition.
    commands.remove_resource::<SiteActive>();
    inactive_nodes::teardown(commands);
    commands.insert_resource(next);
}

/// 按键路：Tab 下一站（一键一义：Tab 只归换站，天气循环在 weather 的
/// C 键），数字键直选支持集第 n 站（主表行序）。
fn key_request(keys: &Res<ButtonInput<KeyCode>>, active: Option<&SiteActive>) -> Option<String> {
    let index = active
        .map(|a| SUPPORTED.iter().position(|s| *s == a.site_type))
        .flatten()
        .unwrap_or(0);
    if keys.just_pressed(KeyCode::Tab) {
        return Some(SUPPORTED[(index + 1) % SUPPORTED.len()].to_owned());
    }
    const DIGITS: [KeyCode; SUPPORTED.len()] = [
        KeyCode::Digit1,
        KeyCode::Digit2,
        KeyCode::Digit3,
        KeyCode::Digit4,
        KeyCode::Digit5,
        KeyCode::Digit6,
        KeyCode::Digit7,
        KeyCode::Digit8,
        KeyCode::Digit9,
    ];
    for (digit, site) in DIGITS.into_iter().zip(SUPPORTED) {
        if keys.just_pressed(digit) {
            return Some(site.to_owned());
        }
    }
    None
}

/// 巡游路：站点定案后计时（装载慢的站不吃驻留预算），到点换下一站；
/// 支持集走满的下一跳是干净退出，不是再绕一圈。
fn tour_request(
    tour: &mut Tour,
    time: &Res<Time>,
    active: Option<&SiteActive>,
    epoch: &Option<Res<GroundEpoch>>,
    exit: &mut MessageWriter<AppExit>,
) -> Option<String> {
    if !tour.enabled {
        return None;
    }
    // 两道门缺一不可：站账目在（拆站后、新站定案前不算驻留）且定案代数
    // 没翻新（新一代从零计时）。代数跨拆站保留，单看它会把装载间隙当
    // 驻留，一帧烧光全部跳数。
    let Some(current) = active else {
        tour.elapsed = 0.0;
        return None;
    };
    let Some(epoch) = epoch else {
        return None;
    };
    if tour.epoch_seen != epoch.0 {
        tour.epoch_seen = epoch.0;
        tour.elapsed = 0.0;
        return None;
    }
    tour.elapsed += time.delta_secs();
    if tour.elapsed < tour.period {
        return None;
    }
    if tour.hops >= SUPPORTED.len() as u32 {
        if !tour.exiting {
            info!("[site] 巡游走满支持集（{} 站），退出", SUPPORTED.len());
            exit.write(AppExit::Success);
            tour.exiting = true;
        }
        return None;
    }
    let index = SUPPORTED
        .iter()
        .position(|s| *s == current.site_type)
        .unwrap_or(0);
    Some(SUPPORTED[(index + 1) % SUPPORTED.len()].to_owned())
}

/// 巡游仪表的跨帧状态。
pub(crate) struct Tour {
    enabled: bool,
    period: f32,
    elapsed: f32,
    epoch_seen: u64,
    hops: u32,
    exiting: bool,
}

impl Default for Tour {
    fn default() -> Self {
        let period = std::env::var(TOUR_ENV)
            .ok()
            .and_then(|raw| raw.trim().parse::<f32>().ok())
            .filter(|secs| *secs > 0.0);
        Self {
            enabled: period.is_some(),
            period: period.unwrap_or(0.0),
            elapsed: 0.0,
            epoch_seen: 0,
            hops: 0,
            exiting: false,
        }
    }
}

fn is_room(site_type: &str) -> bool {
    ROOM_TYPES.contains(&site_type)
}

/// Update（站点定案后的下一帧）：每站一条装载锚行。这里统计保留的
/// 实体/网格（包含休眠实例），不是可见数量；源休眠数量由实例状态行报告。
pub fn report_anchor(
    epoch: Option<Res<GroundEpoch>>,
    mut last: Local<u64>,
    active: Option<Res<SiteActive>>,
    roots: Query<Entity, With<SiteRoot>>,
    children: Query<&Children>,
    meshes: Query<(), With<Mesh3d>>,
) {
    let Some(epoch) = epoch else {
        return;
    };
    if epoch.0 == *last {
        return;
    }
    *last = epoch.0;
    let Some(active) = active else {
        return;
    };
    let (mut entities, mut mesh_count) = (0usize, 0usize);
    for root in &roots {
        let (n, m) = subtree_stats(root, &children, &meshes);
        entities += n;
        mesh_count += m;
    }
    let room = active
        .room
        .as_ref()
        .map(|room| {
            let walkable = room
                .walkable_file
                .as_deref()
                .unwrap_or("模块地板（盘上 authored 无几何）");
            format!("，房间等级 {}，寻路面 {walkable}", room.level)
        })
        .unwrap_or_default();
    // 房间站的寻路面账目随 room 段走；场景包站的面源单报一列。
    let face = match (&active.room, active.nav_face.as_deref()) {
        (Some(_), _) => String::new(),
        (None, Some(file)) => format!("，可行走面 {file}"),
        (None, None) => "，可行走面 地表兼面（authored 无 navmesh）".to_owned(),
    };
    let [x, y, z] = active.position;
    info!(
        "站点装载锚 {}（类别 {}）：场景 {}，sitePosition ({:.0},{:.0},{:.0}){room}{face}，保留实体 {entities}，保留网格 {mesh_count}",
        active.site_type, active.category, active.scene, x, y, z
    );
}

/// 子树统计：(保留实体数, 保留网格实体数)，包括休眠节点。
fn subtree_stats(
    root: Entity,
    children: &Query<&Children>,
    meshes: &Query<(), With<Mesh3d>>,
) -> (usize, usize) {
    let mut entities = 0usize;
    let mut mesh_count = 0usize;
    let mut queue = vec![root];
    while let Some(entity) = queue.pop() {
        entities += 1;
        if meshes.get(entity).is_ok() {
            mesh_count += 1;
        }
        if let Ok(kids) = children.get(entity) {
            for kid in kids {
                queue.push(*kid);
            }
        }
    }
    (entities, mesh_count)
}

/// 运行时的换站请求（小地图点击已解锁站点）：下一帧 [`read_switch`] 消费，
/// 与按键/巡游汇入同一条拆站重装链。请求一进消费即撤，不跨帧存活。
#[derive(Clone, Resource)]
pub struct SiteChangeRequest(pub String);

#[derive(Clone, Resource)]
pub(crate) struct TemporarySiteChangeRequest(pub String);

#[derive(Resource)]
pub(crate) struct TemporarySiteActive;

/// 站点域插件的挂载点：选择资源由入口参数起值。
pub struct SitePlugin(pub SiteRequest);

impl Plugin for SitePlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(SiteSelection::from(self.0.clone()))
            .add_observer(on_scene_ready);
    }
}
