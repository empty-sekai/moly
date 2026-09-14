//! 小地图天气钮的现象主表：服务端下发数据的服务端 mock（范围通则的
//! 具名形态——一行常量表，不是散落的字面量）。
//!
//! 行内容与主表的 MessagePack 键一一对应：id · assetbundleName ·
//! iconAssetbundleName · englishName · name。日文名只作显示数据进图集，
//! 永不进日志（日志只打 id/英文名/长度）。
//!
//! 覆盖面：14 行 = 小地图图标族 icon_env_* 的全部 14 名（提取产物贴图
//! 清单与主表行一一对应）。现象清单里不在本表的档（庆典园地）在真源
//! 侧就查不到主表行——天气钮不换图标，同形。

/// 真源常量：默认现象 id（主表类声明的 DEFAULT_PHENOMENA_ID）。
pub(crate) const DEFAULT_PHENOMENA_ID: i32 = 1;

/// 一行现象主表。
pub(crate) struct PhenomenaRow {
    pub(crate) id: i32,
    /// assetbundleName：现象档名（001_sunny 形），与天气系统当前档名同键。
    pub(crate) asset: &'static str,
    /// iconAssetbundleName：Sprite 装载名前缀（env_sunny 形），装载名 =
    /// "icon_" + 本字段（真源 UpdatePhenomena 的装载式）。
    pub(crate) icon: &'static str,
    /// englishName：天气钮的第二行文本。
    pub(crate) en: &'static str,
    /// name：天气钮的第一行文本（日文）。
    pub(crate) jp: &'static str,
}

/// 全部 14 行，按 id 升序。
pub(crate) const PHENOMENA_ROWS: &[PhenomenaRow] = &[
    PhenomenaRow {
        id: 1,
        asset: "001_sunny",
        icon: "env_sunny",
        en: "SUNNY",
        jp: "晴天",
    },
    PhenomenaRow {
        id: 2,
        asset: "002_evening",
        icon: "env_evening",
        en: "EVENING",
        jp: "傍晚",
    },
    PhenomenaRow {
        id: 3,
        asset: "003_night",
        icon: "env_night",
        en: "NIGHT",
        jp: "夜晚",
    },
    PhenomenaRow {
        id: 4,
        asset: "004_fine",
        icon: "env_fine",
        en: "FINE",
        jp: "夏天",
    },
    PhenomenaRow {
        id: 5,
        asset: "005_fullmoon",
        icon: "env_fullmoon",
        en: "FULLMOON",
        jp: "满月",
    },
    PhenomenaRow {
        id: 6,
        asset: "006_rain",
        icon: "env_rain",
        en: "RAIN",
        jp: "雨天",
    },
    PhenomenaRow {
        id: 7,
        asset: "007_rainnight",
        icon: "env_rainnight",
        en: "RAINNIGHT",
        jp: "雨夜",
    },
    PhenomenaRow {
        id: 8,
        asset: "008_thunder",
        icon: "env_thunder",
        en: "THUNDER",
        jp: "雷电",
    },
    PhenomenaRow {
        id: 9,
        asset: "009_meteorshower",
        icon: "env_meteorshower",
        en: "METEOR SHOWER",
        jp: "流星雨",
    },
    PhenomenaRow {
        id: 10,
        asset: "010_snow",
        icon: "env_snow",
        en: "SNOW",
        jp: "雪天",
    },
    PhenomenaRow {
        id: 11,
        asset: "011_snownight",
        icon: "env_snownight",
        en: "SNOWNIGHT",
        jp: "雪夜",
    },
    PhenomenaRow {
        id: 14,
        asset: "014_sekai",
        icon: "env_sekai",
        en: "SEKAI",
        jp: "「世界」",
    },
    PhenomenaRow {
        id: 15,
        asset: "015_cloud",
        icon: "env_cloud",
        en: "CLOUD",
        jp: "多云",
    },
    PhenomenaRow {
        id: 17,
        asset: "017_rainbow",
        icon: "env_rainbow",
        en: "RAINBOW",
        jp: "彩虹",
    },
];
