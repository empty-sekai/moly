//! 家具装载与摆放：模型包按清单取件，摆放位置走位置律。
//!
//! 装载链：`fixture-models/index.json`（提取侧的包清单，`JsonAsset`
//! 通道）→ 摆放 mock 逐条点名包名 → 逐包装载 glb 的**默认 scene**
//! （提取侧把带 `hasFixtureView` 的 prefab 变体排在默认 scene，实测
//! 999 包全对上）→ 落位由律现算：足迹先按**朝向**重算
//! （`placed_footprint`），再走位置式（`field_position`：足迹对角中点 ·
//! 格值 0.25 · 墙外推 0.125），模型另绕 Y 转朝向角
//! （`direction_yaw_degrees`）→ scene 展开后换装
//! （`fixture_material.rs`）。
//!
//! ⚠ 朝向改的是**两样**东西：模型的旋转，与足迹两端（进而世界位）。
//! 只接前者、位置沿用存档列那对角，在奇数边足迹上逐值相同、在偶数边上
//! 错半格 —— 落地件与占用面因此共用 [`PlacementMock::placed`] 一个入口。
//!
//! 摆放清单是**服务端域**（真源里摆放来自玩家存档、经站点视图运行时
//! 实例化，离线数据里不存在），按范围通则做成**可下发的具名 mock**
//! （下表）：条目形状与存档列一一对应（包名 · 足迹 min/max 格 ·
//! Center.Y · LayoutType 位 · Direction），值是我方选择、逐条具名——
//! 与巡逻环替身同一裁决形态。位置不写死，全部经律现算。

#[path = "fixture_compact.rs"]
mod compact;
#[path = "fence.rs"]
mod fence;
#[path = "fixture_gallery.rs"]
mod gallery;
#[path = "fixture_layouts.rs"]
pub(crate) mod layouts;
#[path = "road.rs"]
pub(crate) mod road;

use bevy::asset::{LoadState, RecursiveDependencyLoadState};
use bevy::ecs::observer::On;
use bevy::gltf::Gltf;
use bevy::prelude::*;
use bevy::scene::{SceneInstanceReady, SceneRoot};
use moly_law::fixture::position::{
    direction_yaw_degrees, field_position, layout_type, WALL_LAYOUT_MASK,
};
use moly_law::fixture::{Direction, GridPosition};

/// 摆放 mock：服务端域的可下发清单（具名替身，见模块注释）。
///
/// 广场保留多类摆放，椅、蛋糕、桌、电脑与灯另置于初始镜头覆盖区。
/// 格 (x, 0, z) 的律位置约为 (0.25·x + 0.125, 0, 0.25·z + 0.125)。
/// 覆盖地板/墙两种布局（墙外推与标记面）、
/// clip 开/关、混合/不混合、窗外观 usage=1 的两种落点，外加两条
/// 范围外族（粒子、Object 模板）走保留桶。另九条是对话锚定摆放
/// （`fixture_id` 非 0）：对话剧本的 pairs 表按 fixtureId 锚定到具体
/// 摆放，锚定家具要摆在名册成员的巡区可达处（对话配对半径 3.3m，
/// 见 `talk.rs`）；其中椅（fixtureId 8）与游戏机（fixtureId 243）是
/// 家具动作点链的两条锚定臂（入座臂与「挂点条目缺」的环带反例臂，
/// 见各自的行注释）。余下 21 条铺自发光族的可见面（材质参数
/// `_BrightPhenomenaEmission` > 0 的 21 包全取，选位理由见该节注释）。
const PLACEMENTS: [PlacementMock; 38] = [
    // Rug coverage: a rectangular picnic sheet under the birthday chair,
    // and an alpha-clipped star beside the player, in the initial camera.
    PlacementMock {
        texture_id: 1,
        package: "mysekai__fixture__mdl_env0001_rug_picnicsheet1",
        min: GridPosition { x: -22, y: 0, z: 3 },
        max: GridPosition { x: -17, y: 0, z: 8 },
        center_y: 0,
        layout: layout_type::RUG,
        direction: Direction::Front,
        fixture_id: 0,
    },
    PlacementMock {
        texture_id: 1,
        package: "mysekai__fixture__mdl_cncollect_rug_star3",
        min: GridPosition { x: -12, y: 0, z: 9 },
        max: GridPosition { x: -9, y: 0, z: 12 },
        center_y: 0,
        layout: layout_type::RUG,
        direction: Direction::Front,
        fixture_id: 0,
    },
    // 生日椅：不透明、无 clip 的基形。
    PlacementMock {
        texture_id: 1,
        package: "mysekai__fixture__mdl_bir1103_fixture_chair1",
        min: GridPosition { x: -20, y: 0, z: 6 },
        max: GridPosition { x: -19, y: 0, z: 6 },
        center_y: 0,
        layout: layout_type::FLOOR,
        direction: Direction::Front,
        fixture_id: 0,
    },
    // 生日蛋糕：clip 变体；同包还有一条粒子族材质（范围外保留桶）。
    PlacementMock {
        texture_id: 1,
        package: "mysekai__fixture__mdl_bir1103_fixture_cake1",
        min: GridPosition { x: -13, y: 0, z: 0 },
        max: GridPosition { x: -12, y: 0, z: 0 },
        center_y: 0,
        layout: layout_type::FLOOR,
        direction: Direction::Front,
        fixture_id: 0,
    },
    // 生日气球：基形。
    PlacementMock {
        texture_id: 1,
        package: "mysekai__fixture__mdl_bir1103_fixture_balloon1",
        min: GridPosition { x: 1, y: 0, z: -31 },
        max: GridPosition { x: 1, y: 0, z: -31 },
        center_y: 0,
        layout: layout_type::FLOOR,
        direction: Direction::Front,
        fixture_id: 0,
    },
    // 生日花饰：基形。
    PlacementMock {
        texture_id: 1,
        package: "mysekai__fixture__mdl_bir1103_fixture_flower1",
        min: GridPosition { x: 3, y: 0, z: -31 },
        max: GridPosition { x: 3, y: 0, z: -31 },
        center_y: 0,
        layout: layout_type::FLOOR,
        direction: Direction::Front,
        fixture_id: 0,
    },
    // 墙架：墙布局（前墙，法线 (0,0,1)，z −= 0.125），Center.Y=6
    // （世界 y=1.5，墙面高度）。墙布局的 ShadowCaster 标记面挂在这条。
    PlacementMock {
        texture_id: 1,
        package: "mysekai__fixture__mdl_con0004_fixture_wallshelf1",
        min: GridPosition { x: 0, y: 6, z: -34 },
        max: GridPosition { x: 0, y: 6, z: -34 },
        center_y: 6,
        layout: layout_type::WALL_FRONT,
        direction: Direction::Front,
        fixture_id: 0,
    },
    // 窗：墙布局（后墙，法线 (0,0,−1)，z += 0.125），Center.Y=8（世界
    // y=2.0）。窗外观材质是二维选择表 usage=1 的两格（26/27），同包
    // 还有 Object 族模板材质（范围外保留桶）。
    PlacementMock {
        texture_id: 1,
        package: "mysekai__fixture__mdl_env0002_window_window1",
        min: GridPosition { x: 4, y: 8, z: -36 },
        max: GridPosition { x: 4, y: 8, z: -36 },
        center_y: 8,
        layout: layout_type::WALL_BACK,
        direction: Direction::Front,
        fixture_id: 0,
    },
    // 屏风（对话锚定，fixtureId 455）：单人/两人对话剧本的锚点。格
    // 足迹 4 宽 × 2 深（motionArea 同形），落在广场中部——名册三名
    // 成员的巡区最近路点都在配对半径内（世界位 (0.75, 0, −7.375)）。
    PlacementMock {
        texture_id: 1,
        package: "mysekai__fixture__mdl_env0002_fixture_byoubu1",
        min: GridPosition { x: 1, y: 0, z: -30 },
        max: GridPosition { x: 4, y: 0, z: -29 },
        center_y: 0,
        layout: layout_type::FLOOR,
        direction: Direction::Front,
        fixture_id: 455,
    },
    // 沙发（对话锚定，fixtureId 695）：三人目两人剧本的锚点，与屏风
    // 对称放在广场另一侧（世界位 (−0.5, 0, −7.375)）。
    PlacementMock {
        texture_id: 1,
        package: "mysekai__fixture__mdl_ext0019_fixture_sofa1",
        min: GridPosition {
            x: -4,
            y: 0,
            z: -30,
        },
        max: GridPosition {
            x: -1,
            y: 0,
            z: -29,
        },
        center_y: 0,
        layout: layout_type::FLOOR,
        direction: Direction::Front,
        fixture_id: 695,
    },
    // 蛋 1（对话锚定，fixtureId 837）：家具对话演出 op 族（eye/mouth/
    // timeline/emoticon/voice/look_at_to_npc）的锚点族 egg1–4 的首员。
    // 足迹数据退化（提取产物里该包足迹为空）⇒ 单格。摆在 unit1 巡逻环
    // 的东角（世界位 (4.875, 0, −7.625)，环角点 (5, −7.99) 距它 0.39m）
    // ——不是随便放：屏风 455 与沙发 695 相距仅 1.25m，两者的配对半径
    // （3.3m）几乎完全重叠成一个「广场中心簇」；蛋若摆进簇内，「近蛋」
    // 就恒与「近簇」同时成立，池里恒有两人档段，20:80 的成员数权重抽
    // 恒落两人档，蛋锚定的单人段（语料里该族全部单人）永远中不了签。
    // 摆在簇半径之外（4.1m）让「近蛋」成为独占状态：unit1 每圈路过东
    // 角，期间池里没有两人档段，单人段进得去也抽得出。
    PlacementMock {
        texture_id: 1,
        package: "mysekai__fixture__mdl_clb1102_fixture_egg1",
        min: GridPosition {
            x: 19,
            y: 0,
            z: -31,
        },
        max: GridPosition {
            x: 19,
            y: 0,
            z: -31,
        },
        center_y: 0,
        layout: layout_type::FLOOR,
        direction: Direction::Front,
        fixture_id: 837,
    },
    // 蛋 2（对话锚定，fixtureId 838）：摆在 unit2 巡逻环的北角（世界位
    // (0.375, 0, −3.375)，环角点 (0.5, −3.39) 距它 0.13m）。同族取位
    // 逻辑见蛋 1 注释——「近蛋」要成为独占状态：这里距屏风 455 4.0m、
    // 距沙发 695 4.1m，unit2 的整条北腿（z=−3.39）距两者都在配对半径
    // （3.3m）之外 ⇒ 走到北腿期间池里只有蛋锚定的单人段，单人段抽得出。
    // unit1 的环西北角 (1, −6) 也会进蛋 2 半径（2.7m）——那是附带的
    // 现场族，不破坏独占带。
    PlacementMock {
        texture_id: 1,
        package: "mysekai__fixture__mdl_clb1102_fixture_egg2",
        min: GridPosition { x: 1, y: 0, z: -14 },
        max: GridPosition { x: 1, y: 0, z: -14 },
        center_y: 0,
        layout: layout_type::FLOOR,
        direction: Direction::Front,
        fixture_id: 838,
    },
    // 蛋 3（对话锚定，fixtureId 839）：摆在 unit3 巡逻环的西南角（世界位
    // (−3.375, 0, −10.625)，环角点 (−3.5, −10.59) 距它 0.13m）。距沙发
    // 695 4.3m、距屏风 455 5.3m ⇒ 独占带落在 unit3 的南腿西段与西腿
    // 南段。蛋 4（840）不放：三个角位里剩下的一族离 455/695 簇都在
    // 半径内（3.4m 边缘以内），摆不出独占带；语料族在，后续换巡逻域
    // 布局即可达。
    PlacementMock {
        texture_id: 1,
        package: "mysekai__fixture__mdl_clb1102_fixture_egg3",
        min: GridPosition {
            x: -14,
            y: 0,
            z: -43,
        },
        max: GridPosition {
            x: -14,
            y: 0,
            z: -43,
        },
        center_y: 0,
        layout: layout_type::FLOOR,
        direction: Direction::Front,
        fixture_id: 839,
    },
    // ---- 自发光族的可见面（材质带 _BrightPhenomenaEmission > 0）----
    // 提取侧普查：999 包里 21 包 23 材质带该参数（值全 1.0，
    // _Enable_Emission 与 _UsePhenomenaLighting 同开）。这批铺全部
    // 21 包：14 件灯全摆（六件 mis 台灯同尺寸 0.209×0.305×0.209m、
    // 足迹同为单格，一件在广场中部，五件铺桌群一侧），7 件单件全取。
    // 全部 fixture_id=0：对话语料 282 个 fixtureId 的最小值是 5，配对面
    // 与出生清空带只读非 0 行 ⇒ 这批对对话域零扰动；个别件落在蛋锚
    // 的配对半径（3.3m）内也只改光照面，不进配对池。
    // 立灯与椅、蛋糕共同置于近景。
    PlacementMock {
        texture_id: 1,
        package: "mysekai__fixture__mdl_ext0001_fixture_lamp1",
        min: GridPosition { x: -19, y: 0, z: 0 },
        max: GridPosition { x: -19, y: 0, z: 0 },
        center_y: 0,
        layout: layout_type::FLOOR,
        direction: Direction::Front,
        fixture_id: 0,
    },
    // 路灯（ext0009）：0.89m 立灯，屏风东北侧（世界位
    // (1.625, 0, −7.375)）。
    PlacementMock {
        texture_id: 1,
        package: "mysekai__fixture__mdl_ext0009_fixture_lamp1",
        min: GridPosition { x: 6, y: 0, z: -30 },
        max: GridPosition { x: 6, y: 0, z: -30 },
        center_y: 0,
        layout: layout_type::FLOOR,
        direction: Direction::Front,
        fixture_id: 0,
    },
    // 路灯（ext0010）：广场西北（世界位 (−2.125, 0, −8.375)）。
    PlacementMock {
        texture_id: 1,
        package: "mysekai__fixture__mdl_ext0010_fixture_lamp1",
        min: GridPosition {
            x: -9,
            y: 0,
            z: -34,
        },
        max: GridPosition {
            x: -9,
            y: 0,
            z: -34,
        },
        center_y: 0,
        layout: layout_type::FLOOR,
        direction: Direction::Front,
        fixture_id: 0,
    },
    // 路灯（ext0008）：广场西南（世界位 (−2.125, 0, −6.875)）。
    PlacementMock {
        texture_id: 1,
        package: "mysekai__fixture__mdl_ext0008_fixture_lamp1",
        min: GridPosition {
            x: -9,
            y: 0,
            z: -28,
        },
        max: GridPosition {
            x: -9,
            y: 0,
            z: -28,
        },
        center_y: 0,
        layout: layout_type::FLOOR,
        direction: Direction::Front,
        fixture_id: 0,
    },
    // 路灯（ext0019）：细杆形（杆身 0.03m 见方），广场东北（世界位
    // (2.375, 0, −8.375)）。
    PlacementMock {
        texture_id: 1,
        package: "mysekai__fixture__mdl_ext0019_fixture_lamp1",
        min: GridPosition { x: 9, y: 0, z: -34 },
        max: GridPosition { x: 9, y: 0, z: -34 },
        center_y: 0,
        layout: layout_type::FLOOR,
        direction: Direction::Front,
        fixture_id: 0,
    },
    // 灯（env0002）：立灯，广场中部（世界位 (−0.375, 0, −8.125)）。
    // 同包的 motionArea 是普查里唯一非空的一条（6×3 里 4 格真）。
    PlacementMock {
        texture_id: 1,
        package: "mysekai__fixture__mdl_env0002_fixture_lamp1",
        min: GridPosition {
            x: -2,
            y: 0,
            z: -33,
        },
        max: GridPosition {
            x: -2,
            y: 0,
            z: -33,
        },
        center_y: 0,
        layout: layout_type::FLOOR,
        direction: Direction::Front,
        fixture_id: 0,
    },
    // 灯（env0012）：立灯，广场东南（世界位 (2.375, 0, −6.625)）。
    PlacementMock {
        texture_id: 1,
        package: "mysekai__fixture__mdl_env0012_fixture_lamp1",
        min: GridPosition { x: 9, y: 0, z: -27 },
        max: GridPosition { x: 9, y: 0, z: -27 },
        center_y: 0,
        layout: layout_type::FLOOR,
        direction: Direction::Front,
        fixture_id: 0,
    },
    // 台灯（mis0001）：mis 族六件同尺寸（实测包围盒 0.209×0.305×0.209m、
    // motionArea 空 ⇒ 足迹退化单格）之一，广场中部（世界位
    // (−0.375, 0, −7.625)）。同族余五件见下，铺桌群一侧。
    PlacementMock {
        texture_id: 1,
        package: "mysekai__fixture__mdl_mis0001_fixture_lamp1",
        min: GridPosition {
            x: -2,
            y: 0,
            z: -31,
        },
        max: GridPosition {
            x: -2,
            y: 0,
            z: -31,
        },
        center_y: 0,
        layout: layout_type::FLOOR,
        direction: Direction::Front,
        fixture_id: 0,
    },
    // 台灯（mis0002）：mis 族第二员，桌（西）与桌面电脑（东）之间的
    // 中缝南位（世界位 (3.125, 0, −8.125)）。
    PlacementMock {
        texture_id: 1,
        package: "mysekai__fixture__mdl_mis0002_fixture_lamp1",
        min: GridPosition {
            x: 12,
            y: 0,
            z: -33,
        },
        max: GridPosition {
            x: 12,
            y: 0,
            z: -33,
        },
        center_y: 0,
        layout: layout_type::FLOOR,
        direction: Direction::Front,
        fixture_id: 0,
    },
    // 台灯（mis0003）：中缝北位，与 mis0002 同列（世界位
    // (3.125, 0, −8.375)）。
    PlacementMock {
        texture_id: 1,
        package: "mysekai__fixture__mdl_mis0003_fixture_lamp1",
        min: GridPosition {
            x: 12,
            y: 0,
            z: -34,
        },
        max: GridPosition {
            x: 12,
            y: 0,
            z: -34,
        },
        center_y: 0,
        layout: layout_type::FLOOR,
        direction: Direction::Front,
        fixture_id: 0,
    },
    // 台灯（mis0004）：笔记本电脑东侧（世界位 (3.625, 0, −7.875)）。
    PlacementMock {
        texture_id: 1,
        package: "mysekai__fixture__mdl_mis0004_fixture_lamp1",
        min: GridPosition {
            x: 14,
            y: 0,
            z: -32,
        },
        max: GridPosition {
            x: 14,
            y: 0,
            z: -32,
        },
        center_y: 0,
        layout: layout_type::FLOOR,
        direction: Direction::Front,
        fixture_id: 0,
    },
    // 台灯（mis0005）：桌面电脑东端、桌群北缘（世界位
    // (3.875, 0, −8.375)）。
    PlacementMock {
        texture_id: 1,
        package: "mysekai__fixture__mdl_mis0005_fixture_lamp1",
        min: GridPosition {
            x: 15,
            y: 0,
            z: -34,
        },
        max: GridPosition {
            x: 15,
            y: 0,
            z: -34,
        },
        center_y: 0,
        layout: layout_type::FLOOR,
        direction: Direction::Front,
        fixture_id: 0,
    },
    // 台灯（mis0006）：桌群东南角（世界位 (3.875, 0, −7.875)）。与
    // mis0001 的包围盒逐轴相同、两 mesh 之一字节全同，另一 mesh 顶点
    // 数据不同——同形族里的近亲件，不是同一模型。
    PlacementMock {
        texture_id: 1,
        package: "mysekai__fixture__mdl_mis0006_fixture_lamp1",
        min: GridPosition {
            x: 15,
            y: 0,
            z: -32,
        },
        max: GridPosition {
            x: 15,
            y: 0,
            z: -32,
        },
        center_y: 0,
        layout: layout_type::FLOOR,
        direction: Direction::Front,
        fixture_id: 0,
    },
    // 壁灯（env0004）：墙布局（前墙，法线 (0,0,1)，z −= 0.125），
    // Center.Y=6（世界 y=1.5）。扁平壁灯壳（0.33×0.38×0.15m），与
    // 墙架同一面墙线（世界位 (0.625, 1.5, −8.5)）。
    PlacementMock {
        texture_id: 1,
        package: "mysekai__fixture__mdl_env0004_fixture_lamp1",
        min: GridPosition { x: 2, y: 6, z: -34 },
        max: GridPosition { x: 2, y: 6, z: -34 },
        center_y: 6,
        layout: layout_type::WALL_FRONT,
        direction: Direction::Front,
        fixture_id: 0,
    },
    // 地灯（non9001）：2cm 薄圆盘，贴地单格（世界位
    // (0.125, 0, −6.625)）。
    PlacementMock {
        texture_id: 1,
        package: "mysekai__fixture__mdl_non9001_fixture_groundlight1",
        min: GridPosition { x: 0, y: 0, z: -27 },
        max: GridPosition { x: 0, y: 0, z: -27 },
        center_y: 0,
        layout: layout_type::FLOOR,
        direction: Direction::Front,
        fixture_id: 0,
    },
    // 桌（twcollect）：1.02m 高，2×2 足迹。
    PlacementMock {
        texture_id: 1,
        package: "mysekai__fixture__mdl_twcollect_fixture_table1",
        min: GridPosition { x: -6, y: 0, z: 5 },
        max: GridPosition { x: -5, y: 0, z: 6 },
        center_y: 0,
        layout: layout_type::FLOOR,
        direction: Direction::Front,
        fixture_id: 0,
    },
    // 桌面电脑（non0005）：2×1 足迹，桌东北侧（世界位
    // (3.5, 0, −8.375)）。
    PlacementMock {
        texture_id: 1,
        package: "mysekai__fixture__mdl_non0005_system_desktop1",
        min: GridPosition {
            x: 13,
            y: 0,
            z: -34,
        },
        max: GridPosition {
            x: 14,
            y: 0,
            z: -34,
        },
        center_y: 0,
        layout: layout_type::FLOOR,
        direction: Direction::Front,
        fixture_id: 0,
    },
    // 笔记本电脑（non0005）：单格，放在近景桌面高度。
    PlacementMock {
        texture_id: 1,
        package: "mysekai__fixture__mdl_non0005_system_laptop1",
        min: GridPosition { x: -5, y: 4, z: 5 },
        max: GridPosition { x: -5, y: 4, z: 5 },
        center_y: 4,
        layout: layout_type::FLOOR,
        direction: Direction::Front,
        fixture_id: 0,
    },
    // 盆栽（twcollect）：0.85×0.72m，带水面材质，3×3 足迹（世界位
    // (1.625, 0, −6.625)）。
    PlacementMock {
        texture_id: 1,
        package: "mysekai__fixture__mdl_twcollect_fixture_planter1",
        min: GridPosition { x: 5, y: 0, z: -28 },
        max: GridPosition { x: 7, y: 0, z: -26 },
        center_y: 0,
        layout: layout_type::FLOOR,
        direction: Direction::Front,
        fixture_id: 0,
    },
    // 茶摊（cncollect）：带 On/Off 动画的立件，2×1 足迹（世界位
    // (−0.75, 0, −8.125)）。
    PlacementMock {
        texture_id: 1,
        package: "mysekai__fixture__mdl_cncollect_fixture_tea3",
        min: GridPosition {
            x: -4,
            y: 0,
            z: -33,
        },
        max: GridPosition {
            x: -3,
            y: 0,
            z: -33,
        },
        center_y: 0,
        layout: layout_type::FLOOR,
        direction: Direction::Front,
        fixture_id: 0,
    },
    // 珊瑚（con0002）：2×2 足迹，广场西侧（世界位 (−2.0, 0, −7.5)）。
    PlacementMock {
        texture_id: 1,
        package: "mysekai__fixture__mdl_con0002_fixture_coral1",
        min: GridPosition {
            x: -9,
            y: 0,
            z: -31,
        },
        max: GridPosition {
            x: -8,
            y: 0,
            z: -30,
        },
        center_y: 0,
        layout: layout_type::FLOOR,
        direction: Direction::Front,
        fixture_id: 0,
    },
    // 机关家具（对话锚定，fixtureId 423）：语料里唯一四员段（机关对
    // 话）的锚。足迹按该包 motionArea 真值格包围盒 10 宽 × 5 深（同
    // 屏风的足迹取法）。摆位卡巡游环西角的转弯段——锚点（世界位
    // (−10.25, 0, −3.625)）落在角点两侧的环弧上，盘沿环覆盖约 11m：
    // 四员参演员的第一圈过境整组穿过配对盘，四员段在池里成立；且
    // 该摆位的出生清空带不盖四员的自然出生弧（清空带会把出生推出
    // 盘外、丢掉第一圈过境，见 `npc.rs` 的 `tour_seed`）。
    // 锚定序号进对话抽签种子（`talk.rs` 的 `seed_for` 按锚序给成员
    // 号移位），重排本表条目会换掉抽签序列——追加在尾，别插队。
    PlacementMock {
        texture_id: 1,
        package: "mysekai__fixture__mdl_chr0004_fixture_nenerobo1",
        min: GridPosition {
            x: -46,
            y: 0,
            z: -17,
        },
        max: GridPosition {
            x: -37,
            y: 0,
            z: -13,
        },
        center_y: 0,
        layout: layout_type::FLOOR,
        direction: Direction::Front,
        fixture_id: 423,
    },
    // 蛋 4（对话锚定，fixtureId 840）：蛋族取位的第四员——名册巡游环
    // 东腿旁（世界位 (10.375, 0, 0.125)，环东腿 x=12 距它 1.6m，整环
    // 过境即入配对半径）。足迹数据退化（该包 motionArea 为空）⇒ 单格，
    // 同蛋 1–3。蛋 3 注里「蛋 4 不放」是三员巡逻代的取位结论（角位
    // 离广场簇摆不出独占带）；31 员巡游环下该族的 30 段单员对话靠
    // 整环逐圈过境进池，不依赖独占带。
    PlacementMock {
        texture_id: 1,
        package: "mysekai__fixture__mdl_clb1102_fixture_egg4",
        min: GridPosition { x: 41, y: 0, z: 0 },
        max: GridPosition { x: 41, y: 0, z: 0 },
        center_y: 0,
        layout: layout_type::FLOOR,
        direction: Direction::Front,
        fixture_id: 840,
    },
    // 椅（对话锚定，fixtureId 8）：无对话动作点链的入座臂。该家具的无对话
    // 指派覆盖 31 员名册里的 30 员，座 id 13 落在包内挂点条目（013：
    // 局部 (0,0,-0.3)，面朝椅背）上——锚定后此链有真实的贴合目标。
    // 格 (33..34, -25..-24) 摆在可行走面上：环带逐格探测与挂点 x/z 的
    // 面门都命中（首版摆位落进面网格的洞里，环带逐格全空、目标按未命中
    // 站定报出——冒烟照实抓到后迁到这里）。与全部既有摆放不重叠（最近
    // 的蛋 1 在 (19,-31)，距 15 格）。追加在尾：锚定序号进对话抽签种子，
    // 重排本表条目会换掉抽签序列。
    PlacementMock {
        texture_id: 1,
        package: "mysekai__fixture__mdl_mis0001_fixture_chair1",
        min: GridPosition {
            x: 33,
            y: 0,
            z: -25,
        },
        max: GridPosition {
            x: 34,
            y: 0,
            z: -24,
        },
        center_y: 0,
        layout: layout_type::FLOOR,
        direction: Direction::Front,
        fixture_id: 8,
    },
    // 游戏机（对话锚定，fixtureId 243）：环带兜底臂的天然反例——该家具
    // 无对话指派行的座 id 是 11，而包里只有挂点条目 13（id 不在条目集
    // 里），动作点解算按「挂点条目缺」回落环带并照实报。它是 1×1 格
    // 件，与椅隔一格摆在 (31, -24)，同在可行走面上（环带八格全中；
    // 对话道的座 13 挂点面门也过——unit 12 抽中它时走正臂）。
    PlacementMock {
        texture_id: 1,
        package: "mysekai__fixture__mdl_ext0008_fixture_gameconsole1",
        min: GridPosition {
            x: 31,
            y: 0,
            z: -24,
        },
        max: GridPosition {
            x: 31,
            y: 0,
            z: -24,
        },
        center_y: 0,
        layout: layout_type::FLOOR,
        direction: Direction::Front,
        fixture_id: 243,
    },
];

/// 一条摆放（服务端域 mock 的行形状，与存档列一一对应）。
#[derive(Clone, Copy)]
struct PlacementMock<P = &'static str> {
    package: P,
    texture_id: u32,
    min: GridPosition,
    max: GridPosition,
    center_y: i8,
    /// MysekaiLayoutType 的位组合（律的 `layout_type` 常量表）。
    layout: u8,
    /// 朝向：存档列带着，落地件按它转（见 [`PlacementMock::placed`]）。
    /// 它同时改**足迹**与**模型旋转**两样东西，不是只转模型。
    direction: Direction,
    /// 对话锚定的家具 id（提取产物 `summary.fixtures` 的键）：对话剧本
    /// 按 pairs 表的 fixtureId 锚到具体摆放。0＝未锚定对话的普通摆放
    /// （对话配对面读不到它）。
    fixture_id: i32,
}

/// 一条摆放算完朝向之后的落位：足迹两端 · 世界位 · 绕 Y 的朝向角。
///
/// 三个读者（落地 spawn · 对话锚点 · 编辑占用面）共用
/// [`PlacementMock::placed`] 这一个入口 —— 各自算一份的话，朝向只会接到
/// 其中一个上，而另两个会安静地留在 Front。
struct PlacedRow {
    /// 当前朝向下的足迹两端（转 90°/270° 时 w/d 相对存档列互换）。
    min: GridPosition,
    max: GridPosition,
    position: [f32; 3],
    /// 绕 Y 的朝向角，**弧度**（律给的是度）。
    yaw: f32,
}

impl<P: std::fmt::Display> PlacementMock<P> {
    /// 本条摆放的落位。位置与足迹都过律，不在这里算。
    ///
    /// 足迹先按朝向重算再喂位置式：源里 min/max 不是存档列，而是从
    /// (Center, GridSize, Direction) 推出来的，位置读的就是推出来的那对
    /// ⇒ 位置沿用 Front 的足迹会在偶数边足迹上错半格。
    ///
    /// 足迹颠倒是摆放表的数据断点，具名 panic —— 与本模块其余对账同一
    /// 处置（静默取一个错落点远差于响亮拒绝）。
    fn placed(&self) -> PlacedRow {
        let (mut center, size) =
            moly_law::fixture::position::footprint_to_center_size(self.min, self.max)
                .expect("validated fixture grid");
        center.y = self.center_y;
        let (min, max) = moly_law::fixture::position::layout_footprint(
            center,
            size,
            self.direction,
            self.layout,
        )
        .unwrap_or_else(|err| panic!("摆放 mock 的足迹无效（{}）：{err}", self.package));
        let position = field_position(min, max, self.center_y, self.layout)
            .unwrap_or_else(|err| panic!("摆放 mock 的布局无格类别（{}）：{err}", self.package));
        PlacedRow {
            min,
            max,
            position,
            yaw: direction_yaw_degrees(self.direction).to_radians(),
        }
    }
}

impl PlacementMock {
    fn into_owned(self) -> PlacementMock<String> {
        PlacementMock {
            texture_id: self.texture_id,
            package: self.package.to_owned(),
            min: self.min,
            max: self.max,
            center_y: self.center_y,
            layout: self.layout,
            direction: self.direction,
            fixture_id: self.fixture_id,
        }
    }
}

/// 冒烟旋钮 `MOLY_FIXTURE_DIRECTION`：把摆放表的朝向整片改写成
/// 0=Front / 1=Left / 2=Back / 3=Right。
///
/// 存在的**唯一**理由是让判据与冒烟能造出非 Front 的摆放：摆放表那一列
/// 全是 Front，而 Front 的旋转矩阵是单位元、足迹律是恒等 ⇒ 朝向接与不接
/// 在原表上逐值相同，画面上也看不出差别。表里的摆放行一条不改。
///
/// 非法值响亮拒绝，不静默退回 Front —— 一个打错的旋钮值静默变成 Front
/// 会让冒烟报出漂亮的绿数字。
fn direction_override() -> Option<Direction> {
    let raw = std::env::var("MOLY_FIXTURE_DIRECTION").ok()?;
    let raw = raw.trim().to_owned();
    if raw.is_empty() {
        return None;
    }
    let value: u8 = raw
        .parse()
        .unwrap_or_else(|_| panic!("MOLY_FIXTURE_DIRECTION 不是 0..3 的整数：{raw:?}"));
    Some(
        Direction::from_u8(value)
            .unwrap_or_else(|| panic!("MOLY_FIXTURE_DIRECTION 越界（闭集是 0..3）：{value}")),
    )
}

/// 摆放表资源（可下发形态：解析自上表；接服务端下发时换成回包解析）。
#[derive(Resource, Clone, Default)]
pub struct FixturePlacements {
    rows: Vec<PlacementMock<String>>,
    /// Opaque identities allocated by the offline layout owner. They do not
    /// change when a placed instance moves or rotates.
    instance_uids: Vec<String>,
    site_id: u32,
    site_type: String,
    level: u32,
    next_edit_uid: u64,
    floor: Option<crate::site::FloorGridLayout>,
}

pub struct PlacedFixture<'a> {
    pub uid: &'a str,
    pub package: &'a str,
    pub position: [f32; 3],
    pub yaw: f32,
}

/// Owned editable record. UID is the placed/offline item identity, not a
/// package or a screen-space proxy. Center/grid_size stay in the source's
/// unrotated layout frame; all readers derive the footprint through the law.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct EditableFixture {
    pub uid: String,
    pub package: String,
    pub texture_id: u32,
    pub fixture_id: i32,
    pub center: GridPosition,
    pub grid_size: moly_law::fixture::Vector3Int,
    pub layout: u8,
    pub direction: Direction,
}

impl EditableFixture {
    /// Editor candidates can leave the grid domain before Decide validates them.
    pub(crate) fn footprint(&self) -> Result<(GridPosition, GridPosition), String> {
        moly_law::fixture::position::layout_footprint(
            self.center,
            self.grid_size,
            self.direction,
            self.layout,
        )
    }

    pub(crate) fn occupancy(&self) -> Result<OccupancyRow, String> {
        let (min, max) = self.footprint()?;
        Ok(OccupancyRow {
            uid: self.uid.clone(),
            package: self.package.clone(),
            min,
            max,
            center_y: self.center.y,
            layout: self.layout,
            direction: self.direction,
            layout_center: self.center,
            layout_grid_size: self.grid_size,
        })
    }

    pub(crate) fn pose(&self) -> Result<Transform, String> {
        let (min, max) = self.footprint()?;
        let position = field_position(min, max, self.center.y, self.layout)?;
        Ok(
            Transform::from_translation(Vec3::from(position)).with_rotation(Quat::from_rotation_y(
                direction_yaw_degrees(self.direction).to_radians(),
            )),
        )
    }
}

/// All local one-shot binders reset against this generation when a map's
/// furniture instances are replaced. Source catalogs remain reusable.
#[derive(Resource, Default)]
pub(crate) struct FixtureLayoutRevision(pub u64);

#[derive(SystemSet, Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct FixtureLayoutSet;

/// Marks a layout owned by a content-library session. It is never remembered
/// by the saved-layout provider when the temporary site is left.
#[derive(Resource)]
pub(crate) struct TemporaryFixtureLayout;

#[derive(Resource)]
pub(crate) struct FixtureLoadFailure(pub(crate) String);

impl FixturePlacements {
    #[cfg(test)]
    pub(crate) fn test_layout(
        rows: &[EditableFixture],
        floor: crate::site::FloorGridLayout,
    ) -> Self {
        Self {
            site_id: 1,
            site_type: "first_floor".into(),
            level: floor.level,
            floor: Some(floor),
            ..Default::default()
        }
        .with_editor_rows(rows, 1)
        .expect("valid synthetic layout")
    }

    pub(crate) fn site_id(&self) -> u32 {
        self.site_id
    }
    pub(crate) fn site_type(&self) -> &str {
        &self.site_type
    }
    pub(crate) fn floor_grid(&self) -> Option<crate::site::FloorGridLayout> {
        self.floor
    }
    pub(crate) fn next_edit_uid(&self) -> u64 {
        self.next_edit_uid.max(1)
    }
    pub(crate) fn set_next_edit_uid(&mut self, next: u64) {
        self.next_edit_uid = next;
    }

    pub(crate) fn editor_rows(&self) -> Vec<EditableFixture> {
        self.rows
            .iter()
            .zip(&self.instance_uids)
            .map(|(row, uid)| {
                let (mut center, grid_size) =
                    moly_law::fixture::position::footprint_to_center_size(row.min, row.max)
                        .expect("validated offline layout footprint");
                center.y = row.center_y;
                EditableFixture {
                    texture_id: row.texture_id,
                    uid: uid.clone(),
                    package: row.package.clone(),
                    fixture_id: row.fixture_id,
                    center,
                    grid_size,
                    layout: row.layout,
                    direction: row.direction,
                }
            })
            .collect()
    }

    /// Build a replacement without changing the currently published layout.
    /// The caller persists this complete draft before installing it.
    pub(crate) fn with_editor_rows(
        &self,
        rows: &[EditableFixture],
        next_uid: u64,
    ) -> Result<Self, String> {
        if self.site_id == 0 {
            return Err("active site layout is not installed".into());
        }
        let mut seen = std::collections::HashSet::new();
        let mut next = self.clone();
        next.rows.clear();
        next.instance_uids.clear();
        for row in rows {
            if row.uid.is_empty() || !seen.insert(row.uid.as_str()) {
                return Err("editor supplied an empty or duplicate fixture UID".into());
            }
            if row.grid_size.x <= 0 || row.grid_size.y <= 0 || row.grid_size.z <= 0 {
                return Err(format!("{} has invalid source grid dimensions", row.uid));
            }
            let (min, max) =
                moly_law::fixture::position::footprint_front(row.center, row.grid_size);
            let (center, dimensions) =
                moly_law::fixture::position::footprint_to_center_size(min, max)?;
            // Source positions use signed bytes; reject wrapping before a
            // malformed move can become a plausible but different rectangle.
            if dimensions != row.grid_size || center.x != row.center.x || center.z != row.center.z {
                return Err(format!(
                    "{} footprint exceeds the signed grid domain",
                    row.uid
                ));
            }
            row.pose()?;
            next.rows.push(PlacementMock {
                texture_id: row.texture_id,
                package: row.package.clone(),
                fixture_id: row.fixture_id,
                min,
                max,
                center_y: row.center.y,
                layout: row.layout,
                direction: row.direction,
            });
            next.instance_uids.push(row.uid.clone());
        }
        next.next_edit_uid = next_uid.max(1);
        Ok(next)
    }

    pub(crate) fn append_editor_fixture(
        &mut self,
        uid: String,
        package: &'static str,
        fixture_id: i32,
        center: GridPosition,
        grid_size: moly_law::fixture::Vector3Int,
        direction: Direction,
    ) -> Result<(), String> {
        if self.site_id == 0 {
            return Err("active site layout is not installed".into());
        }
        if uid.is_empty() || self.instance_uids.contains(&uid) {
            return Err("editor supplied an empty or duplicate fixture UID".into());
        }
        let (min, max) = moly_law::fixture::position::footprint_front(center, grid_size);
        moly_law::fixture::position::footprint_to_center_size(min, max)?;
        self.rows.push(PlacementMock {
            texture_id: 1,
            package: package.to_owned(),
            min,
            max,
            center_y: center.y,
            layout: layout_type::FLOOR,
            direction,
            fixture_id,
        });
        self.instance_uids.push(uid);
        Ok(())
    }

    pub fn total(&self) -> usize {
        self.rows.len()
    }

    /// 已摆放的家具 id 全集（对话的锚定配对面按它核对锚点在不在）。
    pub fn fixture_ids(&self) -> Vec<i32> {
        self.rows.iter().map(|row| row.fixture_id).collect()
    }

    /// 锚定摆放（fixtureId 非 0）：对话演出侧按 (id, 模型包名) 取件。
    pub fn anchored(&self) -> Vec<(i32, &str)> {
        self.rows
            .iter()
            .filter(|row| row.fixture_id != 0)
            .map(|row| (row.fixture_id, row.package.as_str()))
            .collect()
    }

    /// 锚定摆放的世界位（对话配对面与名册巡游环的出生清空带按它取
    /// 锚点）：与 spawn 同一条律（[`PlacementMock::placed`]，朝向已算
    /// 进去），只返回表里锚定摆放的 id。
    pub fn anchor_positions(&self) -> Vec<(i32, [f32; 3])> {
        self.rows
            .iter()
            .filter(|row| row.fixture_id != 0)
            .map(|row| (row.fixture_id, row.placed().position))
            .collect()
    }

    /// 全部摆放行的落位（包名 · 世界位 · 绕 Y 朝向角，弧度）。挂点
    /// 世界位的组合表按它算（`fixture_attach`）——扫描面是**全部已
    /// 摆放实例**（含未锚定的普通摆放），与对话锚定面（[`Self::anchored`]）
    /// 是两个口径。
    pub fn placed_rows(&self) -> Vec<(&str, [f32; 3], f32)> {
        self.rows
            .iter()
            .map(|row| {
                let placed = row.placed();
                (row.package.as_str(), placed.position, placed.yaw)
            })
            .collect()
    }

    pub fn placed_instances(&self) -> Vec<PlacedFixture<'_>> {
        self.rows
            .iter()
            .zip(&self.instance_uids)
            .map(|(row, uid)| {
                let placed = row.placed();
                PlacedFixture {
                    uid,
                    package: &row.package,
                    position: placed.position,
                    yaw: placed.yaw,
                }
            })
            .collect()
    }

    /// 已摆放行的占用面（摆放编辑面对账用：重叠校验要把这些行的格占
    /// 算进站点占用表）。足迹是**当前朝向下的**（同 spawn 的入口）——
    /// 返回存档列那对角会让非 Front 的摆放在编辑面上占错格。
    pub fn occupancy_rows(&self) -> Vec<OccupancyRow> {
        self.rows
            .iter()
            .enumerate()
            .map(|(index, row)| {
                let placed = row.placed();
                let (mut center, grid_size) =
                    moly_law::fixture::position::footprint_to_center_size(row.min, row.max)
                        .expect("validated offline layout footprint");
                center.y = row.center_y;
                OccupancyRow {
                    uid: self.instance_uids[index].clone(),
                    layout_center: center,
                    layout_grid_size: grid_size,
                    package: row.package.clone(),
                    min: placed.min,
                    max: placed.max,
                    center_y: row.center_y,
                    layout: row.layout,
                    direction: row.direction,
                }
            })
            .collect()
    }
}

/// 一条已摆放的占用行（`PlacementMock` 的读出面：编辑面只需要这些列）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OccupancyRow {
    /// Same opaque identity as the saved/offline instance, never its package.
    pub uid: String,
    pub layout_center: GridPosition,
    /// The owner's declared footprint; consumers compare it to the master.
    pub layout_grid_size: moly_law::fixture::Vector3Int,
    pub package: String,
    pub min: GridPosition,
    pub max: GridPosition,
    pub center_y: i8,
    pub layout: u8,
    pub direction: Direction,
}

/// 家具根实体标记（一条摆放一个）。
#[derive(Component)]
pub struct FixtureRoot;

/// 家具根上的摆放行（换装按它判墙布局；对话配对面按 `fixture_id` 认
/// 锚点——0 是未锚定对话的摆放）。
#[derive(Component)]
pub struct FixturePlacement {
    layout: u8,
    /// 对话锚定的家具 id（见 [`PlacementMock::fixture_id`]）。
    pub(crate) fixture_id: i32,
}

/// 家具根上的源 glb 句柄（装载期留底）：对话演出的时间轴 op 要按它
/// 取该包的动画剪辑集（`named_animations`），脸部 ST 写不走它。
#[derive(Component)]
pub struct FixtureSource(pub Handle<Gltf>);

#[derive(Component)]
struct FixtureInstanceSeed {
    uid: String,
    package: String,
    master: i32,
}

/// Rendering ownership is independent of committed gameplay placement.
/// Editor previews opt into the same material chain without FixtureRoot,
/// FixturePlacement, occupancy, inventory or interaction identity.
#[derive(Component)]
pub(crate) struct FixtureVisualRoot {
    pub layout: u8,
}

impl FixtureVisualRoot {
    pub(crate) fn is_wall_layout(&self) -> bool {
        self.layout & WALL_LAYOUT_MASK != 0
    }
}

#[derive(Component)]
pub(crate) struct FixtureVisualSceneReady;

#[derive(Component)]
pub(crate) struct FixtureVisualReady;

#[derive(Component)]
struct FixtureIdentityResolved;

/// The actual source FixtureView root inside this scene instance. Its local
/// transform, not the logical placement's world position, selects variants.
#[derive(Component)]
pub(crate) struct FixtureViewInstance(pub Entity);

impl FixturePlacement {
    /// 墙布局判定：LayoutType 命中 0xF0 任一位（位置律同一条掩码）。
    pub fn is_wall_layout(&self) -> bool {
        self.layout & WALL_LAYOUT_MASK != 0
    }
}

/// 全部摆放的 scene 展开完毕（换装的门）；由 [`on_scene_ready`] 计数置位。
#[derive(Resource)]
pub struct FixtureScenesReady;

/// 已展开的 scene 实例计数（常驻；全部展开后不再变）。
#[derive(Resource, Default)]
struct FixtureScenesReadyCount(usize);

/// 已请求装载的包清单资产。
#[derive(Resource)]
struct FixtureIndexAsset(Handle<moly_assets::json::JsonAsset>);

/// 已请求装载的逐包 glb 句柄。
#[derive(Resource, Default)]
struct FixtureGltfAssets(Vec<Handle<Gltf>>);

/// 已展开的摆放数（spawn 闩的计数）。
#[derive(Resource, Default)]
struct FixtureSpawnedCount(usize);

/// Startup：请求装载包清单，摆放表落位。
pub fn load(mut commands: Commands, server: Res<AssetServer>) {
    let handle = server.load::<moly_assets::json::JsonAsset>(bevy::asset::AssetPath::from(
        "moly://fixture-models/index.json".to_owned(),
    ));
    commands.insert_resource(FixtureIndexAsset(handle));
    // Keep the resource present for existing consumers, but no global preset
    // may be planned before the source site ID and selected level are known.
    commands.insert_resource(FixturePlacements::default());
}

fn restore_selected_layout(
    mut commands: Commands,
    selection: Res<crate::site::SiteSelection>,
    sites: Option<Res<crate::site::Sites>>,
    saved: Res<layouts::SiteFixtureLayouts>,
    mut placements: ResMut<FixturePlacements>,
    mut revision: ResMut<FixtureLayoutRevision>,
    mut last_error: Local<Option<String>>,
    temporary: Option<Res<crate::site::TemporarySiteActive>>,
) {
    let Some(sites) = sites else {
        return;
    };
    let Some(site_id) = sites.site_id(selection.site_type()) else {
        return;
    };
    let Ok(floor) = selection.fixture_floor_grid(&sites) else {
        return;
    };
    if placements.site_id == site_id && placements.site_type == selection.site_type() {
        return;
    }
    let restored = if temporary.is_some() {
        // The temporary site is empty from its first frame. Never materialize
        // a saved layout only to delete it, or let it block preview admission.
        commands.insert_resource(TemporaryFixtureLayout);
        Ok(FixturePlacements {
            site_id,
            site_type: selection.site_type().to_owned(),
            level: floor.level,
            floor: Some(floor),
            ..Default::default()
        })
    } else {
        saved.restore(
            site_id,
            selection.site_type(),
            floor.level,
            selection.content(),
        )
    };
    match restored.and_then(|layout| {
        layouts::validate_floor_layout(&layout, floor)?;
        Ok(layout)
    }) {
        Ok(mut layout) => {
            info!("[offline-layout] site {} ({}) level {}: restored {} fixtures; layouts are per-site",
                site_id, selection.site_type(), floor.level, layout.total());
            layout.floor = Some(floor);
            *placements = layout;
            revision.0 = revision
                .0
                .checked_add(1)
                .expect("fixture layout revision exhausted");
            commands.remove_resource::<FixtureGltfAssets>();
            commands.remove_resource::<FixtureScenesReady>();
            commands.insert_resource(FixtureSpawnedCount::default());
            commands.insert_resource(FixtureScenesReadyCount::default());
            *last_error = None;
        }
        Err(error) if last_error.as_ref() != Some(&error) => {
            error!("[offline-layout] {error}; saved layout was retained and Save is disabled for this site");
            *last_error = Some(error);
        }
        Err(_) => {}
    }
}

/// Called after the caller releases player/NPC furniture ownership. Clear only
/// live instances and derived state, never the per-site storage or source data.
pub(crate) fn reload_current_layout(world: &mut World) {
    let roots: Vec<_> = world
        .query_filtered::<Entity, With<FixtureRoot>>()
        .iter(world)
        .collect();
    for root in roots {
        world.despawn(root);
    }
    world.remove_resource::<FixtureGltfAssets>();
    world.remove_resource::<FixtureLoadFailure>();
    world.remove_resource::<FixtureScenesReady>();
    world.remove_resource::<crate::fixture_material::FixtureMaterialsSwapped>();
    world.remove_resource::<crate::fixture_material::EmissionAccount>();
    world.remove_resource::<crate::fixture_attach::AttachWorlds>();
    world.remove_resource::<crate::fixture_talk::TimelinesPlanned>();
    world.remove_resource::<crate::fixture_talk::TimelineAssets>();
    world.insert_resource(FixtureSpawnedCount::default());
    world.insert_resource(FixtureScenesReadyCount::default());
    let mut revision = world.resource_mut::<FixtureLayoutRevision>();
    revision.0 = revision
        .0
        .checked_add(1)
        .expect("fixture layout revision exhausted");
}

pub(crate) fn install_temporary_layout(
    world: &mut World,
    rows: &[EditableFixture],
) -> Result<(), String> {
    let current = world
        .get_resource::<FixturePlacements>()
        .cloned()
        .ok_or_else(|| "独立场景的家具布局尚未就绪".to_owned())?;
    let replacement = current.with_editor_rows(rows, 1)?;
    crate::player_fixture_action::cancel_for_site_change(world);
    crate::npc_fixture_activity::cancel_for_site_change(world);
    crate::fixture_gimmick::cancel_for_site_change(world);
    crate::fixture_scene_inputs::invalidate_for_site_change(world);
    reload_current_layout(world);
    world.insert_resource(replacement);
    world.insert_resource(TemporaryFixtureLayout);
    Ok(())
}

pub(crate) fn clear_for_site_change(world: &mut World) {
    let temporary = world.remove_resource::<TemporaryFixtureLayout>().is_some();
    if !temporary {
        if let Some(layout) = world.get_resource::<FixturePlacements>().cloned() {
            world
                .resource_mut::<layouts::SiteFixtureLayouts>()
                .remember(&layout);
        }
    }
    reload_current_layout(world);
    world.insert_resource(FixturePlacements::default());
    crate::fixture_edit::clear_for_site_change(world);
}

pub(crate) fn reload_after_save(world: &mut World) {
    crate::player_fixture_action::cancel_for_site_change(world);
    crate::npc_fixture_activity::cancel_for_site_change(world);
    crate::fixture_gimmick::cancel_for_site_change(world);
    crate::fixture_scene_inputs::invalidate_for_site_change(world);
    reload_current_layout(world);
}

/// Update：清单到位后逐条对账（包存在、状态 exported、带 fixture 视图
/// 变体），然后逐包请求装载 glb。对账失败具名 panic——mock 点名的包
/// 不在盘上是数据断点，不是静默跳过。
fn plan_when_ready(
    mut commands: Commands,
    server: Res<AssetServer>,
    index: Option<Res<FixtureIndexAsset>>,
    placements: Res<FixturePlacements>,
    json: Res<Assets<moly_assets::json::JsonAsset>>,
    gltfs: Option<Res<FixtureGltfAssets>>,
) {
    if placements.site_id == 0 {
        return;
    }
    if gltfs.is_some() {
        return;
    }
    let Some(index) = index else {
        return;
    };
    match server.load_state(&index.0) {
        LoadState::Failed(err) => panic!("家具包清单装载失败：{err:?}"),
        LoadState::Loaded => {}
        _ => return,
    }
    let Some(asset) = json.get(&index.0) else {
        return;
    };
    let value: serde_json::Value = serde_json::from_str(&asset.0)
        .unwrap_or_else(|err| panic!("家具包清单不是合法 JSON：{err}"));
    let packages = value
        .get("packages")
        .and_then(|packages| packages.as_object())
        .unwrap_or_else(|| panic!("家具包清单缺 packages 对象"));
    let mut handles = Vec::with_capacity(placements.total());
    for row in &placements.rows {
        let entry = packages
            .get(&row.package)
            .unwrap_or_else(|| panic!("摆放 mock 点名的包不在清单里：{}", row.package));
        let status = entry.get("status").and_then(|v| v.as_str()).unwrap_or("");
        assert_eq!(
            status, "exported",
            "摆放 mock 点名的包未导出：{}（status = {status:?}）",
            row.package
        );
        let has_fixture_view = entry
            .get("hasFixtureView")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        assert!(
            has_fixture_view,
            "摆放 mock 点名的包没有 fixture 视图变体：{}",
            row.package
        );
        let glb = entry
            .get("glb")
            .and_then(|v| v.as_str())
            .unwrap_or_else(|| panic!("清单条目缺 glb 文件名：{}", row.package));
        handles.push(server.load::<Gltf>(bevy::asset::AssetPath::from(format!(
            "moly://fixture-models/{glb}"
        ))));
    }
    info!(
        "家具装载计划：摆放 mock {} 条，逐包装载 {} 个 glb",
        placements.total(),
        handles.len()
    );
    commands.insert_resource(FixtureGltfAssets(handles));
}

/// Update：逐包等到齐（glb 与全部依赖）后展开默认 scene 并按律落位。
/// 每包独立过门；失败具名 panic。计划资源经 commands 落位，本系统
/// 读它时用 Option 过门（同帧 flush 与否不影响正确性）。
fn spawn_when_ready(
    mut commands: Commands,
    server: Res<AssetServer>,
    gltfs: Res<Assets<Gltf>>,
    assets: Option<Res<FixtureGltfAssets>>,
    placements: Res<FixturePlacements>,
    mut spawned: ResMut<FixtureSpawnedCount>,
    scenes_ready: Option<Res<FixtureScenesReady>>,
    temporary: Option<Res<TemporaryFixtureLayout>>,
) {
    let Some(assets) = assets else {
        return;
    };
    if assets.0.is_empty() && scenes_ready.is_none() {
        commands.insert_resource(FixtureScenesReady);
    }
    let occupancy = placements.occupancy_rows();
    for (index, handle) in assets.0.iter().enumerate() {
        if index < spawned.0 {
            continue;
        }
        if let LoadState::Failed(err) = server.load_state(handle) {
            let reason = format!(
                "家具模型装载失败（{}）：{err:?}",
                placements.rows[index].package
            );
            if temporary.is_some() {
                commands.insert_resource(FixtureLoadFailure(reason));
                return;
            }
            panic!("{reason}");
        }
        if let RecursiveDependencyLoadState::Failed(err) =
            server.recursive_dependency_load_state(handle)
        {
            let reason = format!(
                "家具模型依赖装载失败（{}）：{err:?}",
                placements.rows[index].package
            );
            if temporary.is_some() {
                commands.insert_resource(FixtureLoadFailure(reason));
                return;
            }
            panic!("{reason}");
        }
        if !server.is_loaded_with_dependencies(handle) {
            return;
        }
        let Some(gltf) = gltfs.get(handle) else {
            return;
        };
        let row = &placements.rows[index];
        // 默认 scene = fixture 视图变体（提取侧约定，实测 999 包全对上）。
        let Some(scene) = gltf.default_scene.clone() else {
            panic!("家具 glb 没有默认 scene：{}", row.package);
        };
        let placed = row.placed();
        commands.spawn((
            SceneRoot(scene),
            FixtureRoot,
            FixtureVisualRoot { layout: row.layout },
            crate::fixture_colors::FixtureColorChoice {
                package: row.package.clone(),
                texture_id: row.texture_id,
            },
            fence::FixtureRow(index),
            FixturePlacement {
                layout: row.layout,
                fixture_id: row.fixture_id,
            },
            FixtureSource(handle.clone()),
            FixtureInstanceSeed {
                uid: placements.instance_uids[index].clone(),
                package: row.package.clone(),
                master: row.fixture_id,
            },
            crate::fixture_scene_inputs::FixtureScenePlacement(occupancy[index].clone()),
            Transform::from_translation(Vec3::from(placed.position))
                .with_rotation(Quat::from_rotation_y(placed.yaw)),
            // Imported vertex colors encode shader masks, not albedo. Reveal
            // the scene only after its source materials have been installed.
            Visibility::Hidden,
        ));
        info!(
            "家具摆放（{}）：存档格 min {:?} max {:?} center_y {} layout {:#04x} fixture_id {} \
             朝向 {:?} yaw {:.1}° → 足迹 min {:?} max {:?}（{}x{} 格）→ 世界 \
             ({:.3}, {:.3}, {:.3})",
            row.package,
            (row.min.x, row.min.y, row.min.z),
            (row.max.x, row.max.y, row.max.z),
            row.center_y,
            row.layout,
            row.fixture_id,
            row.direction,
            placed.yaw.to_degrees(),
            (placed.min.x, placed.min.y, placed.min.z),
            (placed.max.x, placed.max.y, placed.max.z),
            placed.max.x as i32 - placed.min.x as i32 + 1,
            placed.max.z as i32 - placed.min.z as i32 + 1,
            placed.position[0],
            placed.position[1],
            placed.position[2],
        );
        spawned.0 += 1;
    }
}

/// 全局观察者：scene 实例展开完毕时计数；全部摆放展开后立换装的闩
/// （`FixtureScenesReady`，换装系统只看它）。
fn on_scene_ready(
    trigger: On<SceneInstanceReady>,
    roots: Query<&FixtureRoot>,
    visuals: Query<(), With<FixtureVisualRoot>>,
    children: Query<&Children>,
    extras: Query<&bevy::gltf::GltfExtras>,
    placements: Res<FixturePlacements>,
    mut count: ResMut<FixtureScenesReadyCount>,
    mut commands: Commands,
) {
    if visuals.contains(trigger.event().entity) {
        commands
            .entity(trigger.event().entity)
            .insert(FixtureVisualSceneReady);
    }
    if roots.get(trigger.event().entity).is_err() {
        return;
    }
    fence::bind_scene(trigger.event().entity, &children, &extras, &mut commands);
    count.0 += 1;
    if count.0 == placements.total() {
        info!("家具 scene 全部展开：{}/{}", count.0, placements.total());
        commands.insert_resource(FixtureScenesReady);
    }
}

/// Update：落地件朝向的**读回**，scene 全部展开后报一次。
///
/// 为什么要这一条：摆放日志报的是**喂进去的值**，它只证明控制流走到了
/// 那一行；这条读的是实体上真正持有的 `Transform` —— 渲染消费的就是那个
/// 组件。本域上一次的缺口恰恰是「值算出来了、没有任何人消费」，所以
/// 朝向必须有一条读回，不能只有一条账本。
///
/// 读法不解 euler：把 `+Z` 用实体的旋转转一遍，`atan2(x, z)` 就是绕 Y
/// 的角（`from_rotation_y` 的逆）。同时验 **`+Y` 必须还是 `+Y`** ——
/// 那一臂抓「转错轴」，只看 yaw 的话绕 X 或 Z 的旋转也能给出一个数。
/// 期望与实测不符时 `error!`：这条路径上没有本机判据，至少要会喊。
fn report_orientation(
    mut done: Local<bool>,
    revision: Res<FixtureLayoutRevision>,
    mut seen_revision: Local<u64>,
    ready: Option<Res<FixtureScenesReady>>,
    placements: Res<FixturePlacements>,
    roots: Query<&Transform, With<FixtureRoot>>,
) {
    if *seen_revision != revision.0 {
        *done = false;
        *seen_revision = revision.0;
    }
    if *done || ready.is_none() {
        return;
    }
    let mut observed: Vec<i32> = Vec::new();
    let mut off_axis = 0usize;
    for transform in &roots {
        let forward = transform.rotation * Vec3::Z;
        let up = transform.rotation * Vec3::Y;
        if (up - Vec3::Y).length() > 1.0e-4 {
            off_axis += 1;
        }
        let degrees = forward.x.atan2(forward.z).to_degrees();
        observed.push(((degrees + 360.0) % 360.0).round() as i32);
    }
    if observed.len() != placements.total() {
        return;
    }
    *done = true;
    let mut expected: Vec<i32> = placements
        .rows
        .iter()
        .map(|row| direction_yaw_degrees(row.direction).round() as i32)
        .collect();
    expected.sort_unstable();
    observed.sort_unstable();
    let mut buckets: Vec<(i32, usize)> = Vec::new();
    for yaw in &observed {
        match buckets.last_mut() {
            Some((value, count)) if value == yaw => *count += 1,
            _ => buckets.push((*yaw, 1)),
        }
    }
    info!(
        "家具朝向读回（读实体上的 Transform，不是账本）：{} 件，绕 Y 的角分桶 {:?}，\
         非 Y 轴旋转 {} 件",
        observed.len(),
        buckets,
        off_axis,
    );
    if observed != expected || off_axis != 0 {
        error!(
            "家具朝向读回与摆放表不符：实测 {:?} 期望 {:?}，非 Y 轴旋转 {} 件",
            observed, expected, off_axis,
        );
    }
}

/// 家具装载与摆放插件。材质换装在 `fixture_material.rs` 的插件里，
/// 两边以 `FixtureScenesReady` 闩交接。
pub struct FixturePlugin;

fn bind_activity_identities(
    mut commands: Commands,
    tables: Option<Res<crate::fixture_activity_data::FixtureActivityTables>>,
    roots: Query<(Entity, &FixtureInstanceSeed), Without<FixtureIdentityResolved>>,
) {
    let Some(tables) = tables else {
        return;
    };
    for (entity, seed) in &roots {
        let Some(model) = seed.package.strip_prefix("mysekai__fixture__") else {
            continue;
        };
        let master = if seed.master != 0 {
            tables
                .fixture_master(seed.master)
                .filter(|row| row.model_name == model)
        } else {
            tables.unique_fixture_master_for_model(model)
        };
        if let Some(master) = master {
            commands.entity(entity).insert(
                crate::fixture_activity_state::FixtureActivityIdentity {
                    uid: seed.uid.clone(),
                    master_id: master.id,
                    model_package: seed.package.clone(),
                },
            );
        } else {
            warn!(
                "[fixture-activity] offline layout needs an explicit matching master: {} ({})",
                seed.uid, seed.package
            );
        }
        commands.entity(entity).insert(FixtureIdentityResolved);
    }
}

fn bind_source_views(
    mut commands: Commands,
    ready: Option<Res<FixtureScenesReady>>,
    points: Option<Res<crate::fixture_attach::AttachPoints>>,
    roots: Query<(Entity, &FixtureInstanceSeed), Without<FixtureViewInstance>>,
    children: Query<&Children>,
    extras: Query<&bevy::gltf::GltfExtras>,
) {
    let (Some(_), Some(points)) = (ready, points) else {
        return;
    };
    for (root, seed) in &roots {
        let Some(source) = points.instance_view(&seed.package) else {
            continue;
        };
        let mut stack = vec![root];
        let mut matches = Vec::new();
        while let Some(entity) = stack.pop() {
            if let Ok(kids) = children.get(entity) {
                stack.extend(kids.iter());
            }
            let Ok(extra) = extras.get(entity) else {
                continue;
            };
            let Ok(value) = serde_json::from_str::<serde_json::Value>(&extra.value) else {
                continue;
            };
            if value.get("sourcePathId").and_then(|v| v.as_i64()) == Some(source.transform)
                && value.get("gameObjectId").and_then(|v| v.as_i64()) == Some(source.game_object)
            {
                matches.push(entity);
            }
        }
        if let [view] = matches.as_slice() {
            commands.entity(root).insert(FixtureViewInstance(*view));
        }
    }
}

/// Publish the live ViewObject's local transform, not the placement root's
/// height. A removed view invalidates its binding so a replacement scene can
/// supply a new identity; stale heights must not select a timeline variant.
pub(crate) fn refresh_activity_view(
    mut commands: Commands,
    roots: Query<(
        Entity,
        &FixtureViewInstance,
        Option<&crate::fixture_activity_provider::SourceFixtureViewLocalY>,
    )>,
    transforms: Query<&Transform>,
) {
    use crate::fixture_activity_provider::SourceFixtureViewLocalY;
    for (root, view, current) in &roots {
        let Ok(transform) = transforms.get(view.0) else {
            commands
                .entity(root)
                .remove::<(FixtureViewInstance, SourceFixtureViewLocalY)>();
            continue;
        };
        let y = transform.translation.y;
        if !y.is_finite() {
            if current.is_some() {
                commands.entity(root).remove::<SourceFixtureViewLocalY>();
            }
        } else if current.is_none_or(|current| current.0.to_bits() != y.to_bits()) {
            commands.entity(root).insert(SourceFixtureViewLocalY(y));
        }
    }
}

impl Plugin for FixturePlugin {
    fn build(&self, app: &mut App) {
        road::install(app);
        app.init_resource::<layouts::SiteFixtureLayouts>()
            .init_resource::<FixtureLayoutRevision>()
            .init_resource::<FixtureSpawnedCount>()
            .init_resource::<FixtureScenesReadyCount>()
            .add_systems(Startup, load)
            .add_observer(on_scene_ready)
            .add_systems(
                Update,
                (
                    restore_selected_layout,
                    plan_when_ready,
                    spawn_when_ready,
                    bind_activity_identities,
                    bind_source_views,
                    refresh_activity_view,
                    report_orientation,
                    fence::update_connections,
                )
                    .chain()
                    .in_set(FixtureLayoutSet)
                    .after(crate::site::plan)
                    .after(crate::fixture_edit::FixtureEditSystems::Input)
                    .before(crate::walk_face::build),
            );
    }
}
