//! 音频域第一批：BGM 换曲、区域环境音、A 套邻近环境音管理器、一次性
//! SE、九类音量面板。
//!
//! 两条常驻通道按真源各自的管理器拆开，不复合：
//!
//! - **BGM**（BGM 管理器 `MysekaiBGMManager` 的选曲）：按「当前站点 × 当前
//!   现象」现算。真源 `PlayBGMAsync(site, phenomenon)` 先问用户唱片设定（只在
//!   住宅类站点查；设定是服务端用户态，本仓是具名 mock，默认为空、可由环境
//!   变量下发，见 [`UserMusicPlaySettings`]），再走默认选曲
//!   `PlayDefaultBGMAsync`：现象 id 等于客户端配置的配送现象 id → 站点
//!   normal 档；现象主表亮度档非 none →
//!   站点×亮度档行；none → 现象自己的 BGM 行（主数据缓存按 mysekaiPhenomenaId
//!   建这张字典，重复键后者覆盖前者）。配送类站点（祭会场）的站点控制器恒以
//!   配送现象 id 起 BGM，与画面上是哪个现象无关。某支查不到行时真源不起播、
//!   现声照响。切曲时旧声淡出、新声淡入（CRI 淡入淡出器的线性律，见
//!   [`CROSS_FADE_SECONDS`]），同 cue 不重启；BGM 播放器以强制循环起播，没有
//!   循环点的 BGM 波形整轨循环。真源的选曲时机是进站（各站点控制器的
//!   SetupBGM）、换站动作（传送炮与家园/房间互移的动作状态）与摆设编辑回包；
//!   站内的环境交叉淡化（多人摆设更新走的 UpdateSite）不重新选曲。本仓的
//!   现象切换钮在真源里没有对应物，这边把它当成「以该现象进站」：站点或现象
//!   一变就按新的「站点 × 现象」重选。
//! - **区域环境音**（现象 SE 管理器 `MysekaiPhenomenaSEManager`）：平铺 2D——
//!   每帧的位置与朝向都不进这条链，音量就是「1.0 × 面板」。真源在环境建成
//!   （进站）与交叉淡化收尾两处各调一次：站点类型先过门（草原、海岸、花园、
//!   纪念地四类放，家园、三层房间、祭会场只停），过门则取该站本现象的行、
//!   否则该站「其它」条件的行；每次调用都先停旧声再放（同 cue 也从头放）；
//!   现象没有主表行时记错误并停。这边的触发点是天气链的每次真实提交
//!   （提交序号前进一次＝真源一次调用）。一个 cue 带多条波形时它是多轨序列
//!   （流星那条：洗牌选轨、各轨起播延迟、序列放完即重放），运行时根的音频
//!   索引没有导出序列结构，这条通道对它具名拒绝，不拿其中一条波形顶替。
//!
//! **A 套邻近环境音管理器**（声源对象管理器）是第三个消费者：声源组件 +
//! 距离调音 `clamp(1 - d/max, 0, 1)` + 单通道就近选择。声源由站点场景
//! 侧车的组件表喂（`site_sound` 模块挂上展开后的场景实体）——声源对象
//! 不挂在家具 prefab 上，它是站点场景侧的一等条目。
//!
//! **talk voice**（MySekaiTalkEngine 的 PlayVoice）是第四个消费者：对话
//! 域在行推进处投递行请求，本域同帧起播——真源逐行命令序里 voice 先于
//! text，voice 与文本行是行内同拍，不是窗体开拍。通道单声道：真源每次
//! 起播前 `StopVoiceAll`（在 ExistsCueName 判定之前，缺 cue 也先停旧声）；
//! 缺 cue 真源是「不播、对话照走」，这边照做并按本仓纪律把跳过记响。
//! 音量 = 1.0 × 面板 `vox_scenario`（talk voice 包的 ACB category 实名）。
//!
//! **一次性 SE 通道**（`SoundManager.PlaySEOneShot` 的事件族）是第五个：
//! 事件侧入队（采集受击、对话窗点跳/步进、摆放编辑动作），通道侧起播——
//! 平铺 2D（真源 PlaySEOneShot 走不挂 3d 源的播放器，位置不进这条链）、
//! Requests start individually after the scene's cue-name substitutions. The
//! ambient AudioSourceRepeater interval does not apply to button/one-shot sounds.
//! 音量类按 cue 家族语义归
//! 九类面板的 `se_ingame`/`se_ui` 两键（类→cue 绑定在 ACF 侧，盘上无
//! .acf，归法具名）。
//!
//! cue 到音频文件的绑定是 **(cue, 路由行的 package)**，不是 cue 名单义：
//! 同一 cue 名可以同时活在「多曲共装的包」与「自成一包的下载件」里
//! （同一首歌两份不同的循环区间），路由行自带的 package 字段才是选择面。
//! talk voice 的路由行是提取侧的映射语义：cue = `voice_<script>_<行>_<变体>`，
//! 包随 script（`mysekai__talk__voice__<script>`），cue 不必问包名。
//!
//! 音量分两层。**类别层**是九类总线（SE_INGAME…BGM）：类名与序 = ACF
//! 类别枚举（同名同序），是消费侧的类别接口，类别音量本身 mysekai 不写
//! （真源写者只在 streaming live 域）。**用户层**是本地档三旋钮 × 两组
//! （Live/System——`ApplicationLocalSettings` 的对应物，见
//! [`LocalVolumeSettings`]）：mysekai 只应用 System 组（BGM×0.7 · SE ·
//! Voice，0.7 即 [`BGM_VOLUME_FACTOR`]），Live 组是节奏域的档（持久化但不
//! 施加）。总线值由档派生（进场施加与选项页 `UpdateVolume` 两条写者，
//! 见 [`apply_system_volume`]）；环境变量 `MOLY_AUDIO_VOLUME_<类>` 降为
//! **装载期初始覆写**（mock 通则保留，非锁——施加链跑过即接管）。已接线
//! 消费 `bgm`、`se_area_ambient`、`vox_scenario`、`se_ingame`、`se_ui`
//! 五类，其余四类是骨架，等各自的通道落地再接。

use crate::character::AvatarRoot;
use crate::client_config::ClientConfigs;
use crate::site::SiteActive;
use crate::voice_pcm::{MeteredVoiceSource, VoiceSource};
use crate::weather::CurrentPhenomenon;
use crate::weather_transition::WeatherTransition;
use bevy::asset::{AssetPath, LoadState};
use bevy::audio::{
    AudioPlayer, AudioSink, AudioSinkPlayback, AudioSource, PlaybackSettings, Volume,
};
use bevy::prelude::*;
use moly_assets::json::JsonAsset;
use std::collections::{HashMap, HashSet, VecDeque};
use std::time::Duration;

/// 真源站点类型枚举 `MysekaiSiteType` 的序（站点主表 siteType 名按序对号）。
/// 现象环境音门按这个序做移位。
const SITE_TYPE_ORDER: [&str; 9] = [
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

/// 现象环境音门 `MysekaiUtility.IsPlaySitePhenomenaSound` 的位掩码：
/// `(0xF0 >> 站点类型序) & 1`——只有草原、海岸、花园、纪念地四类为真。
const SITE_PHENOMENA_SOUND_MASK: u32 = 0xF0;

/// 站点主表的站点类别名（`MysekaiSiteCategory`）。
const CATEGORY_HOUSING_HOME: &str = "housing_home";
const CATEGORY_HOUSING_ROOM: &str = "housing_room";
const CATEGORY_DELIVERY: &str = "delivery";

/// 现象环境音门。类型序越界时真源抛参数越界异常；这里同样响亮拒绝。
fn plays_site_phenomena_sound(site_type: &str) -> bool {
    let order = SITE_TYPE_ORDER
        .iter()
        .position(|name| *name == site_type)
        .unwrap_or_else(|| panic!("站点类型 {site_type} 不在站点类型枚举里（现象环境音门）"));
    (SITE_PHENOMENA_SOUND_MASK >> order) & 1 != 0
}

/// BGM 音量因子（真源声明级常量；乘在总线音量上）。
const BGM_VOLUME_FACTOR: f32 = 0.7;

/// BGM 切曲的淡入淡出时长（秒）：BGM 控制器构造时的默认淡化时长 0.25（BGM
/// 选曲界面打开后会改成 0.3/1.0，本仓没有那个界面）。真源把它按毫秒交给
/// CRI 播放器的淡入淡出器：旧声从它当时已到的电平线性降到 0、新声从 0 线性
/// 升到 1，两条同时起跑、各 0.25s（淡化曲线取默认的线性档，无起点偏移）。
/// CRI 的推进节拍是音频服务线程，每拍增量封顶 67ms；这边按渲染帧推进，帧长
/// 不是那个量，封顶不移植。旧声在电平归零后再保留 500ms 才停（静音段），
/// 这边归零即拆。
const CROSS_FADE_SECONDS: f32 = 0.25;

/// BGM intro 段交给 loop 段的提前量（秒）：一次性 intro 段播到「循环起点 -
/// 提前量」时把驻停的 loop 段放行，交接处的重叠被提前量封顶。真源的循环
/// 在 cue 播放器里逐样本无缝；vorbis 解码器没有 seek，两段式交接是近似，
/// 用「短重叠」换「不出现静默缺口」。
const BGM_HANDOFF_LEAD_SECONDS: f32 = 0.05;

/// Unity `Mathf.Epsilon`：最小非规格化正数（Rust 的 `f32::EPSILON` 是机器
/// epsilon，不是这个数，别用混）。
const UNITY_MIN_FLOAT: f32 = 1.4e-45;

// ---- 用户音量档（本地档）与九类总线 ----------------------------------------

/// 用户音量一组三旋钮（真源 `VolumeSettingData`：字段 Bgm/Se/Voice，序列化
/// 键同名；值域 0..=1 步 0.01——UI 侧是整数 0..100，读写两端各乘除 0.01）。
/// 默认 1.0（真源 `ApplicationLocalSettings` 构造时两组建档三字段全 1.0）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct VolumeSettingData {
    pub bgm: f32,
    pub se: f32,
    pub voice: f32,
}

impl Default for VolumeSettingData {
    fn default() -> Self {
        VolumeSettingData {
            bgm: 1.0,
            se: 1.0,
            voice: 1.0,
        }
    }
}

/// 本地档的音量半（真源 `ApplicationLocalSettings.LiveVolume` /
/// `SystemVolume` 两组各一）：Live/System × Bgm/Se/Voice 六浮点。**只有
/// System 组进 mysekai 的施加链**（进场 `SetupVolume(1.0, Bgm×0.7, Se,
/// Voice)` 只读 SystemVolume；0.7 落在 BGM 消费侧的
/// [`BGM_VOLUME_FACTOR`]）；Live 组是节奏域的档——mysekai 不应用，但持久
/// 化与选项页读写照做（忠实）。装载在 [`init_settings`]。
#[derive(Resource, Debug, Clone, Copy, PartialEq)]
pub(crate) struct LocalVolumeSettings {
    pub live: VolumeSettingData,
    pub system: VolumeSettingData,
}

impl Default for LocalVolumeSettings {
    fn default() -> Self {
        LocalVolumeSettings {
            live: VolumeSettingData::default(),
            system: VolumeSettingData::default(),
        }
    }
}

/// 九类音量总线：每类一个 0..=1 的档位值。**类名与序 = ACF 类别枚举**
/// （同名同序：SE_INGAME=0…BGM=8），是消费侧的类别接口；**值由本地档
/// System 组派生**（进场施加与选项页 `UpdateVolume` 两条写者，见
/// [`apply_system_volume`]）：`bgm←System.Bgm`（BGM 消费侧再乘 0.7）·
/// 六个 `se_*←System.Se` · `vox_scenario`/`vox_ingame←System.Voice`——
/// 播放器级施加（三播放器各乘自己全部 cue）在类别面上的形状。类别音量
/// 本身 mysekai 不写（真源写者只在 streaming live 域）。已接线消费：
/// `bgm`、`se_area_ambient`、`vox_scenario`、`se_ingame`、`se_ui`；其余
/// 四类是骨架，等各自的通道落地再接。环境变量
/// `MOLY_AUDIO_VOLUME_<类>` 是装载期初始覆写（[`init_settings`]）。
#[derive(Resource)]
pub(crate) struct VolumeBus {
    pub se_ingame: f32,
    pub se_ui: f32,
    pub se_scenario: f32,
    pub se_sl: f32,
    pub se_sl_call: f32,
    pub se_area_ambient: f32,
    pub vox_scenario: f32,
    pub vox_ingame: f32,
    pub bgm: f32,
}

#[derive(Resource, Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct AudioGate {
    pub enabled: bool,
}

impl Default for AudioGate {
    fn default() -> Self { Self { enabled: true } }
}

impl AudioGate {
    #[inline]
    fn factor(&self) -> f32 { if self.enabled { 1.0 } else { 0.0 } }
}

#[derive(Component)]
pub(crate) enum BusVolume {
    Ambient,
    Voice,
    Se(SeClass),
}

pub(crate) fn apply_sink_volumes(
    bus: Res<VolumeBus>,
    gate: Res<AudioGate>,
    mut sinks: Query<(&BusVolume, &mut AudioSink)>,
) {
    for (source, mut sink) in &mut sinks {
        let volume = match source {
            BusVolume::Ambient => bus.se_area_ambient,
            BusVolume::Voice => bus.vox_scenario,
            BusVolume::Se(class) => class.volume(&bus),
        } * gate.factor();
        if sink.volume().to_linear() != volume {
            sink.set_volume(Volume::Linear(volume));
        }
    }
}

/// 总线类的环境变量名后缀（装载期初始覆写用；序同 [`volume_fields`]）。
const VOLUME_ENTRY_NAMES: [&str; 9] = [
    "SE_INGAME",
    "SE_UI",
    "SE_SCENARIO",
    "SE_SL",
    "SE_SL_CALL",
    "SE_AREA_AMBIENT",
    "VOX_SCENARIO",
    "VOX_INGAME",
    "BGM",
];

/// 九类字段的可变借用数组（初始覆写按下标寻址）。
fn volume_fields(bus: &mut VolumeBus) -> [&mut f32; 9] {
    let VolumeBus {
        se_ingame,
        se_ui,
        se_scenario,
        se_sl,
        se_sl_call,
        se_area_ambient,
        vox_scenario,
        vox_ingame,
        bgm,
    } = bus;
    [
        se_ingame,
        se_ui,
        se_scenario,
        se_sl,
        se_sl_call,
        se_area_ambient,
        vox_scenario,
        vox_ingame,
        bgm,
    ]
}

impl Default for VolumeBus {
    /// 中性 1.0×9。真值由 [`init_settings`] 在 Startup 装载施加（本地档
    /// System 组派生 + 环境变量初始覆写）——这里不再读环境变量（首写者
    /// 已换成本地档）。
    fn default() -> Self {
        VolumeBus {
            se_ingame: 1.0,
            se_ui: 1.0,
            se_scenario: 1.0,
            se_sl: 1.0,
            se_sl_call: 1.0,
            se_area_ambient: 1.0,
            vox_scenario: 1.0,
            vox_ingame: 1.0,
            bgm: 1.0,
        }
    }
}

impl VolumeBus {
    /// 面板账目一行：九类逐类带值（日志推导用，不省略未接线的七类）。
    fn describe(&self) -> String {
        format!(
            "se_ingame {:.2} · se_ui {:.2} · se_scenario {:.2} · se_sl {:.2} · se_sl_call {:.2} · \
             se_area_ambient {:.2} · vox_scenario {:.2} · vox_ingame {:.2} · bgm {:.2}",
            self.se_ingame,
            self.se_ui,
            self.se_scenario,
            self.se_sl,
            self.se_sl_call,
            self.se_area_ambient,
            self.vox_scenario,
            self.vox_ingame,
            self.bgm,
        )
    }
}

/// System 三值施加（进场 `SetupVolume` 与选项页 `UpdateVolume` 同一条
/// 律）：三播放器各设一档——BGM/Bgm、SE/Se、Voice/Voice（master 恒 1.0）。
/// 映射到总线见 [`VolumeBus`] 的档头。`reason` 标写者（进场施加 / 选项页
/// UpdateVolume）；值没变不记行（拖动逐档施加不重复报同一值）。
pub(crate) fn apply_system_volume(bus: &mut VolumeBus, system: &VolumeSettingData, reason: &str) {
    let changed = bus.bgm != system.bgm
        || bus.se_ingame != system.se
        || bus.se_scenario != system.se
        || bus.se_sl != system.se
        || bus.se_sl_call != system.se
        || bus.se_area_ambient != system.se
        || bus.vox_scenario != system.voice
        || bus.vox_ingame != system.voice;
    bus.bgm = system.bgm;
    bus.se_ingame = system.se;
    bus.se_ui = system.se;
    bus.se_scenario = system.se;
    bus.se_sl = system.se;
    bus.se_sl_call = system.se;
    bus.se_area_ambient = system.se;
    bus.vox_scenario = system.voice;
    bus.vox_ingame = system.voice;
    if changed {
        info!(
            "[option] 施加（{reason}）：SetupVolume(1.0, bgm={:.2}, se={:.2}, voice={:.2}) → 总线 \
             [{}]（bgm 消费侧再乘 {} 进 BGM 通道；LiveVolume 不进施加——节奏域律）",
            system.bgm,
            system.se,
            system.voice,
            bus.describe(),
            BGM_VOLUME_FACTOR
        );
    }
}

/// StopVoiceAll also cancels deferred line requests at this command boundary.
/// The active dialogue owners are exclusive and share this global voice bus.
pub(crate) fn stop_voice_all(channel: &mut VoiceChannel, commands: &mut Commands) {
    if let Some(old) = channel.sink.take() {
        if let Ok(mut entity_commands) = commands.get_entity(old) {
            entity_commands.despawn();
        }
    }
    channel.cue = None;
    commands.queue(clear_pending_voice_lines);
}

/// Source PlayAsync disposes its talk engine after the Lua stream completes;
/// Dispose stops voice and cancels outstanding work. Queue at the same boundary
/// as the dialogue's final VoiceLine commands, before serve_voice's flush.
pub(crate) fn dispose_talk_voice(commands: &mut Commands) {
    commands.queue(|world: &mut World| {
        let old = {
            let mut channel = world.resource_mut::<VoiceChannel>();
            channel.cue = None;
            channel.sink.take()
        };
        if let Some(old) = old {
            world.despawn(old);
        }
        clear_pending_voice_lines(world);
    });
}

fn clear_pending_voice_lines(world: &mut World) {
    let pending: Vec<Entity> = world
        .query_filtered::<Entity, With<VoiceLine>>()
        .iter(world)
        .collect();
    for entity in pending {
        world.despawn(entity);
    }
}

// ---- 本地档的存取（ApplicationLocalSettings 音量半的持久化） ---------------

/// 真源走 `PersistentDataUtility`（Unity persistentDataPath 本地档）⇒ 照存
/// 本地。序列化器细节未提取 ⇒ **文件形是我方选值**：容器键照字段名
/// （LiveVolume/SystemVolume），字段键照真源序列化键（Bgm/Se/Voice）。
/// native 落用户数据目录（persistentDataPath 的对应位），`MOLY_SETTINGS_FILE`
/// 可覆写（验证用）；wasm 落 localStorage（浏览器端的「本地」）。
pub(crate) fn settings_sections(
    settings: &LocalVolumeSettings,
) -> [(&'static str, serde_json::Value); 2] {
    let group = |value: &VolumeSettingData| serde_json::json!({"Bgm": value.bgm, "Se": value.se, "Voice": value.voice});
    [
        ("LiveVolume", group(&settings.live)),
        ("SystemVolume", group(&settings.system)),
    ]
}

/// JSON 文本 → 档。整体不是 JSON 对象 ⇒ `None`（调用方响亮回默认——真源
/// 档读不出整档走构造默认的同律）；组或字段缺席 ⇒ 该位回默认（档是六
/// 浮点的并集，半档也算档）；数值越界（手改档）⇒ 钳回 0..=1。
fn parse_volume_settings(text: &str) -> Option<LocalVolumeSettings> {
    let doc: serde_json::Value = serde_json::from_str(text).ok()?;
    if !doc.is_object() {
        return None;
    }
    let group = |key: &str| -> VolumeSettingData {
        let mut data = VolumeSettingData::default();
        let Some(obj) = doc.get(key).filter(|v| v.is_object()) else {
            return data;
        };
        let clamp = |v: &serde_json::Value| -> Option<f32> {
            v.as_f64().map(|n| (n as f32).clamp(0.0, 1.0))
        };
        if let Some(v) = obj.get("Bgm").and_then(&clamp) {
            data.bgm = v;
        }
        if let Some(v) = obj.get("Se").and_then(&clamp) {
            data.se = v;
        }
        if let Some(v) = obj.get("Voice").and_then(&clamp) {
            data.voice = v;
        }
        data
    };
    Some(LocalVolumeSettings {
        live: group("LiveVolume"),
        system: group("SystemVolume"),
    })
}

/// Startup：装载本地档 → 进场施加 → 环境变量初始覆写（三行日志：装载行 ·
/// 施加行 · 覆写行）。这是 [`VolumeBus`] 的首写者链——进场律照
/// `SceneMysekai.Start → SetupVolume`（只应用 System 组）。
pub(crate) fn init_settings(mut commands: Commands, mut bus: ResMut<VolumeBus>) {
    let (settings, origin) = match crate::settings_store::read_text() {
        Ok(Some(text)) => {
            match parse_volume_settings(&text) {
                Some(settings) => (settings, "本地档"),
                None => {
                    warn!("[option] 本地档文本不是 JSON 对象，整档回默认（真源档读不出走构造默认的同律）");
                    (LocalVolumeSettings::default(), "默认（档损坏）")
                }
            }
        }
        Ok(None) => (LocalVolumeSettings::default(), "默认（无档·首跑）"),
        Err(err) => {
            warn!("[option] 本地档读取失败：{err}——整档回默认，本次会话的改动将无法落盘（fail-closed，不静默换位落盘）");
            (LocalVolumeSettings::default(), "默认（读取失败）")
        }
    };
    info!(
        "[option] 本地档装载：来源={origin} · Live{{bgm {:.2}, se {:.2}, voice {:.2}}} · \
         System{{bgm {:.2}, se {:.2}, voice {:.2}}}（LiveVolume 是 mysekai 不应用的节奏域档，持久化照做）",
        settings.live.bgm,
        settings.live.se,
        settings.live.voice,
        settings.system.bgm,
        settings.system.se,
        settings.system.voice,
    );
    apply_system_volume(
        &mut bus,
        &settings.system,
        "进场施加（SceneMysekai.Start → SetupVolume 同律：只读 SystemVolume）",
    );
    // 环境变量初始覆写（mock 通则保留）：逐类覆写总线值，响亮记名。非锁
    // ——施加链（进场之后的任何一次 UpdateVolume/保存）跑过即接管。
    let mut overridden: Vec<&str> = Vec::new();
    for (name, slot) in VOLUME_ENTRY_NAMES.iter().zip(volume_fields(&mut bus)) {
        let Ok(raw) = std::env::var(format!("MOLY_AUDIO_VOLUME_{name}")) else {
            continue;
        };
        match raw.parse::<f32>() {
            Ok(v) if (0.0..=1.0).contains(&v) => {
                *slot = v;
                overridden.push(name);
            }
            _ => warn!(
                "[option] MOLY_AUDIO_VOLUME_{name}={raw:?} 不是 0..=1 的数，不覆写（保留档派生值）"
            ),
        }
    }
    if !overridden.is_empty() {
        info!(
            "[option] 环境变量初始覆写 {} 类：{}（mock 通则；非锁——施加链跑过即接管）",
            overridden.len(),
            overridden.join(" · ")
        );
    }
    commands.insert_resource(settings);
}

/// 写档（选项页 OK 的 SaveToStorage 对应物）：序列化 → 落盘 → 读回解析
/// 校验，三步一行账。失败响亮（会话内档对象已改、盘上没改——下次装载
/// 回旧值，如实报）。
pub(crate) fn save_volume_settings(settings: &LocalVolumeSettings) {
    if let Err(err) = crate::settings_store::save_sections(&settings_sections(settings)) {
        warn!("[option] 本地档写盘失败：{err}——本次改动不持久（档对象在会话内已是新值）");
        return;
    }
    match crate::settings_store::read_text() {
        Ok(Some(stored)) => match parse_volume_settings(&stored) {
            Some(readback) => {
                let same = readback == *settings;
                info!(
                    "[option] 本地档落盘：{} → 读回解析{}（{}）",
                    stored_location(),
                    if same { "一致" } else { "不一致！" },
                    describe_settings(settings)
                );
                if !same {
                    warn!("[option] 读回值与写入值不一致——持久化层有问题，下次装载以盘上为准");
                }
            }
            None => warn!("[option] 落盘后读回解析失败（写入文本非 JSON 对象）"),
        },
        _ => warn!("[option] 落盘后读不回（写盘成功但读取失败）——持久化层有问题"),
    }
}

/// 档的六浮点一行账。
fn describe_settings(settings: &LocalVolumeSettings) -> String {
    format!(
        "Live{{bgm {:.2}, se {:.2}, voice {:.2}}} · System{{bgm {:.2}, se {:.2}, voice {:.2}}}",
        settings.live.bgm,
        settings.live.se,
        settings.live.voice,
        settings.system.bgm,
        settings.system.se,
        settings.system.voice,
    )
}

/// 档所在地的日志名（native 带路径，wasm 带 localStorage 键）。
fn stored_location() -> String {
    crate::settings_store::location()
}

// ---- 流表与路由表 ----------------------------------------------------------

/// 日志与拒绝信息里的资产名标签：非 ASCII 字符转码点写法。cue/包/档名空间
/// 来自游戏资产，将来某次重提取可能带入表外字符；进日志的只有这份转写。
fn label(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_ascii() {
                c.to_string()
            } else {
                format!("U+{:04X}", c as u32)
            }
        })
        .collect()
}

/// A decoded stream's asset-root-relative path and authored loop interval.
struct StreamRow {
    ogg: String,
    loops: bool,
    loop_start: f64,
    loop_end: f64,
    /// 整轨时长（秒）——talk voice 的起播账目用（行→cue→包→时长）。
    duration: f64,
    /// 这个 (cue, package) 键下的波形条数。大于 1 时这个 cue 是多轨序列：
    /// 选哪条、何时放由 cue 表的序列结构决定（客户端 ACB 数据，不是服务端
    /// 配置），运行时根的音频索引没有导出那份结构。
    ///
    /// 流星那条（序列类型 2，即 `CriAtomEx.CueType.Shuffle`；六轨；序列命令
    /// 带重放标志）在 CRI 原生运行时里的律：每起一轮序列只放一轨——位置 +1、
    /// 到尾归 0，放顺序表里该位置那一轨；位置归 0 时（首轮也算）重洗顺序表：
    /// 逐格与一个均匀抽中的格对换（不是 Fisher-Yates），轨数不少于 3 且不是
    /// 首轮时，若新表首格等于上一轮表的末格，再把首格与 [1, n-1] 里抽中的一格
    /// 对换，所以跨轮不连放同一轨。抽签是 xorshift128，种子取单调时钟纳秒数，
    /// 按 Mersenne Twister 的初始化递推铺成四个状态字；区间抽签 = 低端 +
    /// 随机数模区间宽。每轨的事件先把轨时钟往回拨它的起播延迟（毫秒项 ×1000 +
    /// 微秒项）再发 noteOn；波形无循环点，放完即止；这一轨的声音全部放完且
    /// 没有被叫停时，序列从头再起一轮（这就是 cue 长度记为无限的来由）。
    /// 移植需要提取侧先导出：序列类型、重放标志、各轨起播延迟、轨到 subsong。
    waveforms: usize,
}

/// (cue, package) → 流。同一键带多条流（多 subsong）时那是一个多轨序列
/// cue，流表只留最小 subsong 一条并记下波形条数：区域环境音通道见到多波形
/// 键具名拒绝；其余消费者（时间线 SE、一次性 SE）仍取这一条，是具名的
/// 替身（单流键 subsong 为空值，不与编号争——空值折算为最大，必输给任何
/// 编号）。
#[derive(Default)]
struct Streams(HashMap<(String, String), StreamRow>);

/// 一条路由的指向：cue 名（换曲判定用）+ 路由行自带的 package（流表
/// 选择面——同一 cue 名可活在多个包里，见模块注释）。
#[derive(Clone)]
struct RouteCue {
    cue: String,
    package: String,
}

impl RouteCue {
    /// 从路由行取指向（siteBgms/bgms/siteSounds/siteSoundFallbacks 同构）。
    fn from_row(row: &serde_json::Value) -> Self {
        Self {
            cue: field_str(row, "cue").to_string(),
            package: field_str(row, "package").to_string(),
        }
    }
}

/// 现象主表的亮度档（`MysekaiPhenomenaBrightnessType`：none/normal/bright/dark）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Brightness {
    None,
    Normal,
    Bright,
    Dark,
}

impl Brightness {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "none" => Some(Self::None),
            "normal" => Some(Self::Normal),
            "bright" => Some(Self::Bright),
            "dark" => Some(Self::Dark),
            _ => None,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Normal => "normal",
            Self::Bright => "bright",
            Self::Dark => "dark",
        }
    }
}

/// 一档现象的音频面（现象清单一项）。
struct PhenomenonAudio {
    /// 现象 id（配送祭会场那档是客户端配置里的配送现象 id）。
    id: i64,
    /// 现象主表行的亮度档；`None` = 没有主表行（配送祭会场那档）。
    master_brightness: Option<Brightness>,
    /// 现象自己的 BGM 行（亮度档 none 的现象用）。
    own_bgm: Option<RouteCue>,
    /// siteId → 本现象在该站的环境音行（主表序，同站取第一行）。
    site_sounds: HashMap<i64, RouteCue>,
}

/// 环境音选行的结果。
enum AmbientRoute<'a> {
    Play(&'a RouteCue),
    /// 现象没有主表行：真源记错误并停。
    NoMaster,
    /// 该站既没有本现象的行也没有「其它」行：真源取到空名，只停不放。
    /// `site_has_rows` = 该站在表里有别的行（真源这时不记错误）。
    NoRow {
        site_has_rows: bool,
    },
}

/// 选曲路由：BGM 与区域环境音各自独立（真源两个管理器各查各的表），都按
/// 「站点 × 现象」现算，不在装载期折成单站的表。
#[derive(Resource)]
pub(crate) struct Routing {
    /// (siteId, 亮度档) → BGM 路由（站点 BGM 表）。换曲判定按 cue 名：
    /// 同曲的两档不重启。
    site_bgms: HashMap<(i64, Brightness), RouteCue>,
    /// siteId → 「其它」条件的环境音行（同站取第一行）。
    site_sound_fallbacks: HashMap<i64, RouteCue>,
    /// 现象环境音表里有任何一行的站（真源对「该站一行都没有」记错误，对
    /// 「有行但本现象与其它条件都不中」只取到空名、不记错误）。
    sites_with_sounds: HashSet<i64>,
    /// 档名 → 该档的音频面。
    phenomena: HashMap<String, PhenomenonAudio>,
    streams: Streams,
}

/// talk voice 包的包名前缀（提取侧清单族 `mysekai/talk/voice/` 的扁平形）。
const TALK_VOICE_PACKAGE_PREFIX: &str = "mysekai__talk__voice__";

impl Routing {
    fn phenomenon(&self, name: &str) -> &PhenomenonAudio {
        // 天气与音频读同一份现象清单：档不在表里是结构不变量破裂。
        self.phenomena
            .get(name)
            .unwrap_or_else(|| panic!("音频路由表里没有档 {}", label(name)))
    }

    /// 默认 BGM 选曲（真源 `PlayDefaultBGMAsync`，入参现象 id 由站点控制器给：
    /// 配送类站点给配送现象 id，其余站点给该站当前现象）。`Err` = 真源这一支
    /// 查不到行、不起播。
    fn bgm_route(
        &self,
        site: &SiteActive,
        phenomenon: &str,
        delivery_phenomenon_id: i64,
    ) -> Result<&RouteCue, String> {
        let entry = self.phenomenon(phenomenon);
        let site_id = i64::from(site.site_id);
        let phenomenon_id = if site.category == CATEGORY_DELIVERY {
            delivery_phenomenon_id
        } else {
            entry.id
        };
        let brightness = if phenomenon_id == delivery_phenomenon_id {
            Brightness::Normal
        } else {
            match entry.master_brightness {
                None => return Err(format!("现象 id {phenomenon_id} 没有现象主表行")),
                Some(Brightness::None) => {
                    return entry.own_bgm.as_ref().ok_or_else(|| {
                        format!("现象 id {phenomenon_id} 的亮度档是 none，却没有现象 BGM 行")
                    });
                }
                Some(brightness) => brightness,
            }
        };
        self.site_bgms.get(&(site_id, brightness)).ok_or_else(|| {
            format!(
                "站点 BGM 表没有 site {site_id} 的 {} 档",
                brightness.label()
            )
        })
    }

    /// 现象环境音选行（真源 `GetSiteSEName`）：该站有本现象的行取第一行，
    /// 否则取该站「其它」条件的第一行。站点类型门在调用方。
    fn ambient_route(&self, site_id: i64, phenomenon: &str) -> AmbientRoute<'_> {
        let entry = self.phenomenon(phenomenon);
        if entry.master_brightness.is_none() {
            return AmbientRoute::NoMaster;
        }
        match entry
            .site_sounds
            .get(&site_id)
            .or_else(|| self.site_sound_fallbacks.get(&site_id))
        {
            Some(route) => AmbientRoute::Play(route),
            None => AmbientRoute::NoRow {
                site_has_rows: self.sites_with_sounds.contains(&site_id),
            },
        }
    }

    /// The timeline names both package and cue. A same-name cue in another
    /// package or the public UI sound bank is not a substitute.
    pub(crate) fn timeline_se_asset_path(&self, package: &str, cue: &str) -> Option<&str> {
        self.streams
            .0
            .get(&(cue.to_owned(), package.to_owned()))
            .map(|stream| stream.ogg.as_str())
    }
    /// talk voice 包里的全部 (cue, 包) 对，按 (cue, 包) 字典序——合成对话
    /// 注入口的确定性选材面（取前几条即固定样本）。
    pub(crate) fn talk_voice_cues(&self) -> Vec<(String, String)> {
        let mut rows: Vec<(String, String)> = self
            .streams
            .0
            .keys()
            .filter(|(_, package)| package.starts_with(TALK_VOICE_PACKAGE_PREFIX))
            .cloned()
            .collect();
        rows.sort();
        rows
    }
}

/// 装载请求；三份必达档案到达后由 `parse` 一次性吃掉。第四份 partvoice
/// 路由表**可缺席**：文件不在＝路由面缺席（消费侧 fail-closed 具名跳过），
/// 装载失败不拦三份必达档案的解析，只把 partvoice 面折成 `None`。
#[derive(Resource)]
pub(crate) struct AudioRequests {
    ui: Handle<JsonAsset>,
    index: Handle<JsonAsset>,
    loops: Handle<JsonAsset>,
    corpus: Handle<JsonAsset>,
    partvoice: Handle<JsonAsset>,
}

/// Startup：请求三份必达路由档案与 partvoice 路由表。index.json 与天气域
/// 共用同一份资产（AssetServer 按路径去重），loop.json 是音频自己的循环
/// 点表，corpus.json 是 talk voice 的语料账本（cue→包账面 + 上游无包具名
/// 清单），partvoice.json 是说话者/家具 → 变体语音包的查表面（可缺席）。
pub(crate) fn load(mut commands: Commands, server: Res<AssetServer>) {
    let index = server.load::<JsonAsset>(AssetPath::from("moly://phenomena/index.json".to_owned()));
    let loops = server.load::<JsonAsset>(AssetPath::from(
        "moly://phenomena/audio/loop.json".to_owned(),
    ));
    let corpus = server.load::<JsonAsset>(moly_assets::audio_corpus());
    let partvoice = server.load::<JsonAsset>(moly_assets::partvoice_routes());
    commands.insert_resource(AudioRequests {
        ui: server.load("moly://ui/audio/loop.json"),
        index,
        loops,
        corpus,
        partvoice,
    });
}

/// Update：两份档案到达后解出路由表。结构损伤（字段缺、区间倒挂、必有的
/// 行不在、路由指向的流不在）在此响亮 panic——路由表与流表出自同一份
/// 提取产物，对不上是提取缺陷，不是运行期分支。
pub(crate) fn parse(
    mut commands: Commands,
    server: Res<AssetServer>,
    jsons: Res<Assets<JsonAsset>>,
    requests: Option<ResMut<AudioRequests>>,
    bus: Res<VolumeBus>,
) {
    let Some(requests) = requests else {
        return; // 已解析并撤下
    };
    for (name, handle) in [
        ("index", &requests.index),
        ("loop", &requests.loops),
        ("ui", &requests.ui),
        ("corpus", &requests.corpus),
    ] {
        if let LoadState::Failed(err) = server.load_state(handle) {
            panic!("音频路由档案 {name} 装载失败：{err:?}");
        }
    }
    if !server.is_loaded(&requests.index)
        || !server.is_loaded(&requests.loops)
        || !server.is_loaded(&requests.corpus)
        || !server.is_loaded(&requests.ui)
    {
        return; // 还在装
    }
    // partvoice 路由表：三份必达之外的可缺席档案。文件不在＝路由面缺席
    // ——不 panic（消费侧按 fail-closed 具名跳过）；
    // Loaded 但结构损伤照样响亮（表在盘上却对不上形，是提取缺陷）。
    // 在途（Loading）则等它一起吃：请求资源一次性撤下，不留在半途。
    let partvoice_map = match server.load_state(&requests.partvoice) {
        LoadState::Failed(_) => {
            warn!(
                "partvoice 路由表缺席：partvoice 步全部具名跳过（不推默认包），voice_ 族不受影响"
            );
            None
        }
        LoadState::Loaded => {
            let doc = jsons
                .get(&requests.partvoice)
                .unwrap_or_else(|| panic!("partvoice.json 已装载但不在资产表里"));
            let doc: serde_json::Value = serde_json::from_str(&doc.0)
                .unwrap_or_else(|err| panic!("partvoice.json 不是合法 JSON：{err}"));
            Some(parse_partvoice(&doc))
        }
        _ => return, // 在途：下一帧再吃（本地文件一帧内必达）
    };
    let Some(index) = jsons.get(&requests.index) else {
        panic!("音频 index.json 不在资产表里");
    };
    let Some(loops) = jsons.get(&requests.loops) else {
        panic!("音频 loop.json 不在资产表里");
    };
    let Some(corpus) = jsons.get(&requests.corpus) else {
        panic!("音频 corpus.json 不在资产表里");
    };
    let index: serde_json::Value = serde_json::from_str(&index.0)
        .unwrap_or_else(|err| panic!("音频 index.json 不是合法 JSON：{err}"));
    let loops: serde_json::Value = serde_json::from_str(&loops.0)
        .unwrap_or_else(|err| panic!("音频 loop.json 不是合法 JSON：{err}"));
    let corpus: serde_json::Value = serde_json::from_str(&corpus.0)
        .unwrap_or_else(|err| panic!("音频 corpus.json 不是合法 JSON：{err}"));
    let mut routing = parse_routing(&index, &loops);
    let ui: serde_json::Value =
        serde_json::from_str(&jsons.get(&requests.ui).expect("UI audio loaded").0)
            .expect("UI audio catalog");
    routing.streams.0.extend(parse_streams(&ui, "").0);
    let voice_corpus = parse_corpus(&corpus);
    info!(
        "音频路由表就绪：站点 BGM {} 行 · 站点「其它」环境音 {} 行 · 现象 {} 档 · {} 个流键（按站点 × 现象现算）",
        routing.site_bgms.len(),
        routing.site_sound_fallbacks.len(),
        routing.phenomena.len(),
        routing.streams.0.len(),
    );
    info!(
        "talk voice 语料账本就绪：上游无包 {} 条具名（缺 cue 日志的分类依据）",
        voice_corpus.uncovered.len(),
    );
    info!(
        "九类总线当前值（由本地档 System 组派生，环境变量降为装载期初始覆写）：{}",
        bus.describe()
    );
    commands.insert_resource(routing);
    commands.insert_resource(voice_corpus);
    if let Some(map) = partvoice_map {
        info!(
            "partvoice 路由表就绪：参与者 {} 员（门拒 {}）· 家具 {} 件",
            map.participants.len(),
            map.participants
                .values()
                .filter(|route| matches!(route, ParticipantRoute::Gated(_)))
                .count(),
            map.fixtures.len(),
        );
        commands.insert_resource(map);
    }
    commands.remove_resource::<AudioRequests>();
}

/// 解析两份档案为路由表。行重复、亮度档不在枚举里、循环区间倒挂、任何
/// 一条可达路由指向的流不在，都在此 panic——这些是提取产物的结构损伤，
/// 不是运行期分支。
fn parse_routing(index: &serde_json::Value, loops: &serde_json::Value) -> Routing {
    // (site, 亮度档) → BGM 路由（BGM 管理器的站点选曲表，含 none 档的死行）。
    let mut site_bgms: HashMap<(i64, Brightness), RouteCue> = HashMap::new();
    for row in index["siteBgms"]
        .as_array()
        .unwrap_or_else(|| panic!("siteBgms 不是数组"))
    {
        let site = field_i64(row, "siteId");
        let brightness_name = field_str(row, "brightnessType");
        let brightness = Brightness::parse(brightness_name).unwrap_or_else(|| {
            panic!("siteBgms 里 site {site} 的亮度档 {brightness_name:?} 不在枚举里")
        });
        if site_bgms
            .insert((site, brightness), RouteCue::from_row(row))
            .is_some()
        {
            panic!("siteBgms 里 site {site} 的 {brightness_name} 档重复");
        }
    }
    // 站点表里出现的每一站都要有三个亮度档：任何一档现象都可能落到它。
    let mut sites: Vec<i64> = site_bgms.keys().map(|(site, _)| *site).collect();
    sites.sort_unstable();
    sites.dedup();
    for site in &sites {
        for brightness in [Brightness::Normal, Brightness::Bright, Brightness::Dark] {
            if !site_bgms.contains_key(&(*site, brightness)) {
                panic!("siteBgms 里 site {site} 缺 {} 档", brightness.label());
            }
        }
    }
    // 站点「其它」条件的环境音行（同站取第一行，与真源 FirstOrDefault 同）。
    let mut site_sound_fallbacks: HashMap<i64, RouteCue> = HashMap::new();
    for row in index["siteSoundFallbacks"]
        .as_array()
        .unwrap_or_else(|| panic!("siteSoundFallbacks 不是数组"))
    {
        site_sound_fallbacks
            .entry(field_i64(row, "siteId"))
            .or_insert_with(|| RouteCue::from_row(row));
    }

    let entries = index["phenomena"]
        .as_object()
        .unwrap_or_else(|| panic!("phenomena 不是对象表"));
    let mut phenomena: HashMap<String, PhenomenonAudio> = HashMap::new();
    for (name, entry) in entries {
        // master 为 null 是配送祭会场那档（配送判据在客户端配置里，提取侧
        // 「无主表行」就是它的形）。
        let master_brightness = entry["master"].as_object().map(|master| {
            let value = master["brightnessType"]
                .as_str()
                .unwrap_or_else(|| panic!("现象 {} 的 brightnessType 不是字符串", label(name)));
            Brightness::parse(value)
                .unwrap_or_else(|| panic!("现象 {} 的亮度档 {value:?} 不在枚举里", label(name)))
        });
        // 现象 BGM 字典按 mysekaiPhenomenaId 建、重复键后者覆盖前者：取最后一行。
        let own_bgm = entry["bgms"]
            .as_array()
            .and_then(|rows| rows.last())
            .map(RouteCue::from_row);
        if master_brightness == Some(Brightness::None) && own_bgm.is_none() {
            panic!("现象 {} 亮度档是 none 却没有 bgms 行", label(name));
        }
        let mut site_sounds: HashMap<i64, RouteCue> = HashMap::new();
        for row in entry["siteSounds"].as_array().into_iter().flatten() {
            site_sounds
                .entry(field_i64(row, "siteId"))
                .or_insert_with(|| RouteCue::from_row(row));
        }
        phenomena.insert(
            name.clone(),
            PhenomenonAudio {
                id: field_i64(entry, "id"),
                master_brightness,
                own_bgm,
                site_sounds,
            },
        );
    }

    let sites_with_sounds: HashSet<i64> = site_sound_fallbacks
        .keys()
        .copied()
        .chain(
            phenomena
                .values()
                .flat_map(|entry| entry.site_sounds.keys().copied()),
        )
        .collect();
    let routing = Routing {
        site_bgms,
        site_sound_fallbacks,
        sites_with_sounds,
        phenomena,
        streams: parse_streams(loops, "phenomena/"),
    };
    // 任何站点 × 现象可达的路由都必须落到一条解码好的流上。解析期全量核过
    // 之后，消费侧就不再需要「cue 无流」的运行期分支。
    let reachable = routing
        .site_bgms
        .values()
        .map(|route| ("BGM", route))
        .chain(
            routing
                .site_sound_fallbacks
                .values()
                .map(|route| ("环境音", route)),
        )
        .chain(routing.phenomena.values().flat_map(|entry| {
            entry
                .own_bgm
                .iter()
                .map(|route| ("BGM", route))
                .chain(entry.site_sounds.values().map(|route| ("环境音", route)))
        }));
    for (kind, route) in reachable {
        if !routing
            .streams
            .0
            .contains_key(&(route.cue.clone(), route.package.clone()))
        {
            panic!(
                "{kind}路由指向的流不在表里：{} @ {}",
                label(&route.cue),
                label(&route.package)
            );
        }
    }
    routing
}

/// 解 loop.json 为流表（见 [`Streams`]）。诊断行（提取侧对无波形 cue 写的
/// {cue, error}）具名跳过；可播放行字段缺失或循环区间倒挂响亮拒绝。
fn parse_streams(loops: &serde_json::Value, base: &str) -> Streams {
    // 流表：(cue, package) →（最小 subsong 的）一条流 + 该键的波形条数。
    let mut streams: HashMap<(String, String), (i64, StreamRow)> = HashMap::new();
    for package in loops["packages"]
        .as_array()
        .unwrap_or_else(|| panic!("loop.json 的 packages 不是数组"))
    {
        let package_name = field_str(package, "package").to_string();
        for stream in package["streams"]
            .as_array()
            .unwrap_or_else(|| panic!("loop.json 的 streams 不是数组"))
        {
            let cue = field_str(stream, "cue");
            // The extractor explicitly emits {cue, error} for a requested cue
            // with no source waveform (phenomena/audio.py, NO_CUE). It is a
            // diagnostic variant, not a playable stream with missing fields.
            // Do not invent loop=false, a path, or another cue as a substitute.
            if let Some(diagnostic) = stream.get("error") {
                let reason = diagnostic
                    .as_str()
                    .filter(|value| !value.is_empty())
                    .expect("audio diagnostic error must be a nonempty string");
                assert_eq!(
                    stream.as_object().map(|row| row.len()),
                    Some(2),
                    "audio diagnostic cannot also contain partial playable fields"
                );
                warn!("[audio-source] unavailable cue={cue} package={package_name}: {reason}");
                continue;
            }
            let cue_label = label(cue);
            // 单流键 subsong 是空值（自成一包的下载件一族）；多流键里全是
            // 编号——空值折算为最大，必输给任何编号。
            let subsong = stream["subsong"].as_i64().unwrap_or(i64::MAX);
            let loops_flag = stream["loop"]
                .as_bool()
                .unwrap_or_else(|| panic!("流 {cue_label} 的 loop 不是布尔"));
            let (loop_start, loop_end) = if loops_flag {
                let start = field_f64(stream, "loopStartSeconds");
                let end = field_f64(stream, "loopEndSeconds");
                if end <= start {
                    panic!("流 {cue_label} 的循环区间倒挂：{start:.3}..{end:.3}");
                }
                (start, end)
            } else {
                (0.0, 0.0)
            };
            let source = stream["ogg"]
                .as_str()
                .or_else(|| stream["wav"].as_str())
                .unwrap_or_else(|| panic!("流 {cue_label} 既无 ogg 也无 wav 路径"))
                .to_string();
            let row = StreamRow {
                ogg: format!("{base}{source}"),
                loops: loops_flag,
                loop_start,
                loop_end,
                duration: field_f64(stream, "durationSeconds"),
                waveforms: 1,
            };
            let key = (cue.to_string(), package_name.clone());
            match streams.get_mut(&key) {
                // 同键多流（多 subsong）：多轨序列 cue。保留最小 subsong 一条
                // 并累计波形条数（环境音通道据此具名拒绝，见 StreamRow）。
                Some((existing_subsong, existing_row)) => {
                    let waveforms = existing_row.waveforms + 1;
                    if subsong < *existing_subsong {
                        *existing_subsong = subsong;
                        *existing_row = row;
                    }
                    existing_row.waveforms = waveforms;
                }
                None => {
                    streams.insert(key, (subsong, row));
                }
            }
        }
    }
    let streams = Streams(streams.into_iter().map(|(k, (_, v))| (k, v)).collect());

    streams
}

/// JSON 行取字符串字段：缺字段是结构损伤，响亮拒绝。
fn field_str<'a>(row: &'a serde_json::Value, key: &str) -> &'a str {
    row[key]
        .as_str()
        .unwrap_or_else(|| panic!("音频表行缺字符串字段 {key}"))
}

/// JSON 行取整数字段：同上（subsong 允许空值，取值处自行处理）。
fn field_i64(row: &serde_json::Value, key: &str) -> i64 {
    row[key]
        .as_i64()
        .unwrap_or_else(|| panic!("音频表行缺整数字段 {key}"))
}

/// talk voice 语料账本（corpus.json 的消费切片）：上游无包 cue 的具名
/// 清单——缺 cue 日志拿它分类「具名上游缺口」与「未提取消费面」。
#[derive(Resource)]
pub(crate) struct VoiceCorpus {
    uncovered: Vec<String>,
}

/// 解析语料账本的 talk-voice 切片。结构损伤（数组缺、行缺键）在此响亮
/// panic——与路由表同一条资产边界纪律。
fn parse_corpus(corpus: &serde_json::Value) -> VoiceCorpus {
    let uncovered = corpus["families"]["talk-voice"]["uncovered"]
        .as_array()
        .unwrap_or_else(|| panic!("corpus.json 的 talk-voice.uncovered 不是数组"))
        .iter()
        .map(|row| {
            row["cue"]
                .as_str()
                .unwrap_or_else(|| panic!("corpus.json 的 uncovered 行缺 cue"))
                .to_string()
        })
        .collect();
    VoiceCorpus { uncovered }
}

/// talk voice cue 的路由行：cue = `voice_<script>_<行>_<变体>`，包随
/// script（提取侧 audio_corpus 的映射语义——「the consumer never asks
/// for a package; it plays cues」）；绑定键仍取 (cue, package)。
/// 非 `voice_` 前缀的 cue（partvoice 族等）不出包名，由调用方具名。
pub(crate) fn voice_package(cue: &str) -> Option<String> {
    let body = cue.strip_prefix("voice_")?;
    let stem = body.rsplit_once('_')?.0.rsplit_once('_')?.0;
    if stem.is_empty() {
        return None;
    }
    Some(format!("{TALK_VOICE_PACKAGE_PREFIX}{stem}"))
}

/// partvoice 路由表（partvoice.json 的消费切片）：说话者变体 → 变体语音
/// 包、家具角色 → 单参包。真源链里包随说话者：主链参与者过变体门后
/// **双载**（scenario 侧 + mysekai 侧各一份），蛋链家具角色无门单载——
/// 这里只存包名，cue 在不在包里由流表判（真源 ExistsCueName 的对应物）。
#[derive(Resource)]
pub(crate) struct PartVoiceMap {
    /// characterUnitId → 主链路由（门过=双包，门拒=具名理由——真源对
    /// 门拒者不载任何包）。
    participants: HashMap<i32, ParticipantRoute>,
    /// mysekaiFixtureId → 蛋链单参包名。
    fixtures: HashMap<i32, String>,
}

/// 主链参与者的路由结果。
enum ParticipantRoute {
    /// 门过：两份包名（扁平形，与流表键同构；scenario 侧包在流表里可以
    /// 还没解出——提取侧族在途时 cue 查不到，走缺 cue 具名）。
    Packages { scenario: String, mysekai: String },
    /// 门拒：理由具名（真源静默不载包；此处把理由带到跳过日志）。
    Gated(String),
}

/// 解析 partvoice 路由表。结构损伤（键不是整数、行不是对象、双包形缺
/// 键、值不是字符串）响亮 panic——表在盘上就对得上形，对不上是提取缺陷。
fn parse_partvoice(doc: &serde_json::Value) -> PartVoiceMap {
    let participants = doc["participants"]
        .as_object()
        .unwrap_or_else(|| panic!("partvoice.json 缺 participants 对象"))
        .iter()
        .map(|(key, row)| {
            let id: i32 = key
                .parse()
                .unwrap_or_else(|_| panic!("partvoice.json 的 participants 键 {key} 不是整数 id"));
            let route = if let Some(reason) = row.get("skip").and_then(|v| v.as_str()) {
                ParticipantRoute::Gated(reason.to_owned())
            } else {
                let package = |name: &str| {
                    row[name]
                        .as_str()
                        .unwrap_or_else(|| panic!("partvoice.json 的参与者 {id} 缺 {name} 包名"))
                };
                ParticipantRoute::Packages {
                    scenario: package("scenarioPackage").to_owned(),
                    mysekai: package("mysekaiPackage").to_owned(),
                }
            };
            (id, route)
        })
        .collect();
    let fixtures = doc["fixtures"]
        .as_object()
        .unwrap_or_else(|| panic!("partvoice.json 缺 fixtures 对象"))
        .iter()
        .map(|(key, row)| {
            let id: i32 = key
                .parse()
                .unwrap_or_else(|_| panic!("partvoice.json 的 fixtures 键 {key} 不是整数 id"));
            let package = row["mysekaiPackage"]
                .as_str()
                .unwrap_or_else(|| panic!("partvoice.json 的家具 {id} 缺 mysekaiPackage 包名"))
                .to_owned();
            (id, package)
        })
        .collect();
    PartVoiceMap {
        participants,
        fixtures,
    }
}

/// JSON 行取浮点字段：同上。
fn field_f64(row: &serde_json::Value, key: &str) -> f64 {
    row[key]
        .as_f64()
        .unwrap_or_else(|| panic!("音频表行缺浮点字段 {key}"))
}

// ---- BGM 通道 ---------------------------------------------------------------

/// BGM 一路在响的声音。两段式：intro 一次性段（循环起点之前）+ loop 常驻段
/// （生成即驻停，intro 到点放行）——真源 cue 的循环区间由播放器逐样本接，
/// 这边用两个 sink 交近似（见 [`BGM_HANDOFF_LEAD_SECONDS`]）。
struct BgmVoice {
    cue: String,
    intro: Option<Entity>,
    loop_sink: Option<Entity>,
    /// 循环区间（起播时从流表抄来；交接与账目用）。
    loop_start: f64,
    loop_end: f64,
    /// loop 段已放行（intro 已到交接点或已播空）。
    handoff_done: bool,
    /// 淡入计时：`None` = 冷启全量起播（首次放曲没有旧声可让），
    /// `Some(elapsed)` = 换曲淡入中。
    fade_in: Option<f32>,
    /// 当前请求的包络音量（淡出起点与账目）；实际 sink 音量逐个校准。
    applied_volume: f32,
}

/// 淡出中的一路旧声：换曲时从现声里搬出来，0.25s 线性归零后拆除。
struct FadingBgm {
    entities: Vec<Entity>,
    from_volume: f32,
    elapsed: f32,
}

/// BGM 通道运行态。
#[derive(Resource, Default)]
pub(crate) struct BgmChannel {
    voice: Option<BgmVoice>,
    fading: Vec<FadingBgm>,
    /// 已具名拒绝的 (站点, 档)：同一组合只报一次，换了组合再报。
    refused_for: Option<(u32, String)>,
    /// 已具名拒绝唱片设定的站点（设定只随站点变：在这站停留期间只报一次，
    /// 离站即清，再进再报）。
    music_refused_for: Option<u32>,
}

/// 用户唱片 BGM 设定（真源 `UserMysekaiMusicPlayFixtureSetting`：每站一行，
/// 唱片 id + 歌唱版本 id）。**服务端下发的用户态** ⇒ 具名 mock：默认为空
/// （任何站点都没有设定，默认选曲照常生效）；环境变量
/// `MOLY_AUDIO_MOCK_MUSIC_RECORD` 下发设定，条目语法 `站点id:唱片id:歌唱版本id`，
/// 逗号分隔多站，格式不对响亮拒绝（panic）。
///
/// 客户端决定的部分：只在住宅类站点（站点主表类别 housing_home /
/// housing_room）查设定，其余站点直接走默认选曲（已移植）；设定到 BGM 资源
/// 的解析也在客户端（`MysekaiMusicRecordModel`：唱片主表的曲目类型分原声带
/// 与乐曲两支，乐曲支经外部 id 查乐曲主表、按本机设置的默认歌唱类型与该曲的
/// 歌唱版本设定挑歌唱版本，得出 BGM 包与 cue；解析不出资源时回落默认选曲），
/// 这一半没有移植——运行时根没有乐曲音频集。于是一条设定在这里只会被 BGM
/// 通道响亮拒绝、照放默认选曲：真源在能解析出资源时放的是那张唱片，两边
/// 不一样，拒绝行说明这一点。
#[derive(Resource)]
pub(crate) struct UserMusicPlaySettings {
    /// siteId → (唱片 id, 歌唱版本 id)。
    per_site: HashMap<u32, (i64, i64)>,
}

/// 唱片设定 mock 的环境变量名。
const MUSIC_RECORD_MOCK_ENV: &str = "MOLY_AUDIO_MOCK_MUSIC_RECORD";

impl Default for UserMusicPlaySettings {
    fn default() -> Self {
        let mut per_site = HashMap::new();
        let Ok(raw) = std::env::var(MUSIC_RECORD_MOCK_ENV) else {
            return Self { per_site };
        };
        for entry in raw
            .split(',')
            .map(str::trim)
            .filter(|entry| !entry.is_empty())
        {
            let parts: Vec<&str> = entry.split(':').map(str::trim).collect();
            let [site, record, vocal] = parts.as_slice() else {
                panic!("{MUSIC_RECORD_MOCK_ENV} 条目 {entry:?} 不是 站点id:唱片id:歌唱版本id");
            };
            let (Ok(site), Ok(record), Ok(vocal)) = (
                site.parse::<u32>(),
                record.parse::<i64>(),
                vocal.parse::<i64>(),
            ) else {
                panic!("{MUSIC_RECORD_MOCK_ENV} 条目 {entry:?} 里有非整数");
            };
            if per_site.insert(site, (record, vocal)).is_some() {
                panic!("{MUSIC_RECORD_MOCK_ENV} 里站点 {site} 出现两次");
            }
        }
        info!(
            "[audio] 唱片 BGM 设定 mock：{} 站有设定（{MUSIC_RECORD_MOCK_ENV}）",
            per_site.len()
        );
        Self { per_site }
    }
}

impl UserMusicPlaySettings {
    fn setting(&self, site_id: u32) -> Option<(i64, i64)> {
        self.per_site.get(&site_id).copied()
    }
}

/// Update：BGM 逐帧——淡出旧声、推进淡入、intro→loop 交接、按档换曲。
pub(crate) fn advance_bgm(
    mut commands: Commands,
    server: Res<AssetServer>,
    time: Res<Time>,
    bus: Res<VolumeBus>,
    gate: Res<AudioGate>,
    phenomenon: Res<CurrentPhenomenon>,
    site: Option<Res<SiteActive>>,
    configs: Option<Res<ClientConfigs>>,
    music: Res<UserMusicPlaySettings>,
    routing: Option<Res<Routing>>,
    mut channel: ResMut<BgmChannel>,
    mut sinks: Query<&mut AudioSink>,
) {
    let Some(routing) = routing else {
        return; // 路由表未就绪
    };
    let dt = time.delta_secs();

    // 淡出推进：线性归零，到点拆除。
    channel.fading.retain_mut(|fading| {
        fading.elapsed += dt;
        let level = fading.from_volume * (1.0 - (fading.elapsed / CROSS_FADE_SECONDS).min(1.0)) * gate.factor();
        for entity in &fading.entities {
            if let Ok(mut sink) = sinks.get_mut(*entity) {
                sink.set_volume(Volume::Linear(level));
            }
        }
        if fading.elapsed >= CROSS_FADE_SECONDS {
            for entity in &fading.entities {
                if let Ok(mut entity_commands) = commands.get_entity(*entity) {
                    entity_commands.despawn();
                }
            }
            false
        } else {
            true
        }
    });

    // 当前声的音量与交接。
    let target_volume = BGM_VOLUME_FACTOR * bus.bgm * gate.factor();
    if let Some(voice) = channel.voice.as_mut() {
        let envelope = match &mut voice.fade_in {
            Some(elapsed) => {
                *elapsed += dt;
                (*elapsed / CROSS_FADE_SECONDS).min(1.0)
            }
            None => 1.0,
        };
        let volume = target_volume * envelope;
        // A player can acquire its AudioSink after this fade has already ended.
        // Reconcile each actual sink, not just the previous requested envelope.
        for entity in voice.intro.iter().chain(&voice.loop_sink) {
            if let Ok(mut sink) = sinks.get_mut(*entity) {
                if (sink.volume().to_linear() - volume).abs() > 1e-6 {
                    sink.set_volume(Volume::Linear(volume));
                }
            }
        }
        voice.applied_volume = volume;

        // Keep the handoff retryable if the loop sink has not materialized yet.
        // An absent intro here means a previously empty intro was reclaimed.
        let intro_status = voice.intro.and_then(|intro| {
            sinks.get(intro).ok().map(|sink| {
                (
                    sink.position().as_secs_f32()
                        >= voice.loop_start as f32 - BGM_HANDOFF_LEAD_SECONDS
                        || sink.empty(),
                    sink.empty(),
                )
            })
        });
        let handoff_due = voice.intro.is_none() || intro_status.is_some_and(|(due, _)| due);
        if handoff_due && !voice.handoff_done {
            if let Some(loop_sink) = voice.loop_sink {
                if let Ok(sink) = sinks.get(loop_sink) {
                    sink.play();
                    voice.handoff_done = true;
                    info!(
                        "BGM intro 收口：{} 进入循环段 [{:.3}, {:.3}]（intro 段 {:.3}s）",
                        label(&voice.cue),
                        voice.loop_start,
                        voice.loop_end,
                        voice.loop_start
                    );
                }
            }
        }
        if intro_status.is_some_and(|(_, empty)| empty) {
            if let Some(intro) = voice.intro.take() {
                if let Ok(mut entity_commands) = commands.get_entity(intro) {
                    entity_commands.despawn();
                }
            }
        }
    }

    // 选曲：站点或客户端配置未就绪时不动（现声照响，换站途中亦然）。
    let (Some(site), Some(configs)) = (site.as_deref(), configs.as_deref()) else {
        return;
    };
    let key = (site.site_id, phenomenon.0.clone());
    // 唱片设定只在住宅类站点查（真源 TryPlayUserSettingBGM 的类别门）。
    let music_setting = matches!(
        site.category.as_str(),
        CATEGORY_HOUSING_HOME | CATEGORY_HOUSING_ROOM
    )
    .then(|| music.setting(site.site_id))
    .flatten();
    match music_setting {
        Some((record, vocal)) => {
            if channel.music_refused_for != Some(site.site_id) {
                error!(
                    "[audio] 唱片 BGM 设定拒绝：站点 {} 设了唱片 {record}（歌唱版本 {vocal}），唱片到 BGM 资源的解析未移植（运行时根无乐曲音频集）；真源能解析出资源时放这张唱片，这里照放默认选曲，两边不一样",
                    site.site_id
                );
                channel.music_refused_for = Some(site.site_id);
            }
        }
        None => channel.music_refused_for = None,
    }
    let delivery_phenomenon_id =
        i64::from(configs.int(crate::client_config::KEY_DELIVERY_PHENOMENA_ID));
    let target = match routing.bgm_route(site, &phenomenon.0, delivery_phenomenon_id) {
        Ok(target) => {
            channel.refused_for = None;
            target
        }
        Err(reason) => {
            if channel.refused_for.as_ref() != Some(&key) {
                error!(
                    "[audio] BGM 不换：站点 {} · 档 {}：{reason}（真源这一支不起播，现声照响）",
                    site.site_id,
                    label(&phenomenon.0)
                );
                channel.refused_for = Some(key);
            }
            return;
        }
    };
    // 换曲判定：按 cue 名（真源同名 cue 不重放）。
    if channel
        .voice
        .as_ref()
        .is_some_and(|voice| voice.cue == target.cue)
    {
        return;
    }
    // 现声搬进淡出，按淡出起点的现值起算。
    if let Some(voice) = channel.voice.take() {
        let entities = voice
            .intro
            .iter()
            .chain(&voice.loop_sink)
            .copied()
            .collect();
        channel.fading.push(FadingBgm {
            entities,
            from_volume: voice.applied_volume,
            elapsed: 0.0,
        });
    }
    // 冷启 = 此刻没有任何声在响（现声无、淡出也无）——首曲全量起播。
    let cold = channel.fading.is_empty();
    let Some(stream) = routing
        .streams
        .0
        .get(&(target.cue.clone(), target.package.clone()))
    else {
        // 解析期已全量核过路由都在表里；运行期到不了这支。真到了就是表
        // 在运行中被换了形状——响亮拒绝。
        panic!(
            "BGM 档 {} 的流不在表里：{} @ {}",
            label(&phenomenon.0),
            label(&target.cue),
            label(&target.package)
        );
    };
    let volume_now = if cold { target_volume } else { 0.0 };
    let settings_volume = Volume::Linear(volume_now);
    let handle = server.load::<AudioSource>(AssetPath::from(format!("moly://{}", stream.ogg)));
    let (intro, loop_sink, handoff_done) = if stream.loops {
        if stream.loop_start > 0.0 {
            // 两段式：intro 一次性段 + 驻停的 loop 段。
            let intro = commands
                .spawn((
                    AudioPlayer::new(handle.clone()),
                    PlaybackSettings::ONCE
                        .with_duration(Duration::from_secs_f64(stream.loop_start))
                        .with_volume(settings_volume),
                ))
                .id();
            let loop_sink = commands
                .spawn((
                    AudioPlayer::new(handle),
                    PlaybackSettings::LOOP
                        .with_start_position(Duration::from_secs_f64(stream.loop_start))
                        .with_duration(Duration::from_secs_f64(stream.loop_end - stream.loop_start))
                        .with_volume(settings_volume)
                        .paused(),
                ))
                .id();
            (Some(intro), Some(loop_sink), false)
        } else {
            // 循环起点在 0：整轨就是循环体。
            let loop_sink = commands
                .spawn((
                    AudioPlayer::new(handle),
                    PlaybackSettings::LOOP
                        .with_duration(Duration::from_secs_f64(stream.loop_end))
                        .with_volume(settings_volume),
                ))
                .id();
            (None, Some(loop_sink), true)
        }
    } else {
        // 波形没有循环点（家园 normal 档就是这样）：BGM 播放器起播前把循环
        // 限次设成强制循环，CRI 播放器对这个档位按波形全长取模推进播放位置
        // ⇒ 整轨循环。
        let loop_sink = commands
            .spawn((
                AudioPlayer::new(handle),
                PlaybackSettings::LOOP.with_volume(settings_volume),
            ))
            .id();
        (None, Some(loop_sink), true)
    };
    channel.voice = Some(BgmVoice {
        cue: target.cue.clone(),
        intro,
        loop_sink,
        loop_start: stream.loop_start,
        loop_end: stream.loop_end,
        handoff_done,
        fade_in: if cold { None } else { Some(0.0) },
        applied_volume: volume_now,
    });
    info!(
        "BGM 换曲：站点 {} · 档 {} → {}（{}，音量 {:.2} = {BGM_VOLUME_FACTOR:.2} × {:.2}，{}）",
        site.site_id,
        label(&phenomenon.0),
        label(&target.cue),
        if cold { "冷启" } else { "交叉淡化 0.25s" },
        target_volume,
        bus.bgm,
        if stream.loops {
            format!(
                "intro {:.3}s → 循环 [{:.3}, {:.3}]",
                stream.loop_start, stream.loop_start, stream.loop_end
            )
        } else {
            "无循环点，整轨强制循环".to_string()
        },
    );
}

// ---- 区域环境音通道 ----------------------------------------------------------

/// 环境音通道运行态：单 sink，每次调用先停再放（同 cue 也从头放——真源现象
/// SE 管理器无差别 Stop+Play）。
#[derive(Resource, Default)]
pub(crate) struct AmbientChannel {
    sink: Option<Entity>,
    /// 已处理到的天气提交序号（每次真实提交＝真源一次现象 SE 调用）。
    handled_commit: Option<u64>,
    /// 在响的 cue（账目用）。
    playing: Option<String>,
}

/// Update：区域环境音——每次环境提交按「站点 × 现象」选行，平铺 2D，
/// 音量 = 1.0 × 面板，先停后放。
pub(crate) fn advance_ambient(
    mut commands: Commands,
    server: Res<AssetServer>,
    bus: Res<VolumeBus>,
    gate: Res<AudioGate>,
    transition: Option<Res<WeatherTransition>>,
    site: Option<Res<SiteActive>>,
    routing: Option<Res<Routing>>,
    mut channel: ResMut<AmbientChannel>,
) {
    let (Some(routing), Some(transition), Some(site)) = (routing, transition, site) else {
        return; // 路由表、天气链或站点未就绪
    };
    let (Some(serial), Some(committed)) =
        (transition.committed_serial(), transition.committed.as_ref())
    else {
        return; // 还没有任何环境提交
    };
    if channel.handled_commit == Some(serial) {
        return; // 这次提交已经处理过
    }
    if committed.site_id != site.site_id {
        return; // 提交属于上一站（换站途中）：等本站自己的提交
    }
    channel.handled_commit = Some(serial);
    // 真源每一支都先停旧声。
    if let Some(sink) = channel.sink.take() {
        if let Ok(mut entity_commands) = commands.get_entity(sink) {
            entity_commands.despawn();
        }
    }
    channel.playing = None;
    let site_id = i64::from(site.site_id);
    if !plays_site_phenomena_sound(&site.site_type) {
        info!(
            "环境音停：站点 {}（{}）不在现象环境音门内（真源只在草原、海岸、花园、纪念地四类站点放）",
            site.site_id, site.site_type
        );
        return;
    }
    let route = match routing.ambient_route(site_id, &committed.name) {
        AmbientRoute::Play(route) => route,
        AmbientRoute::NoMaster => {
            error!(
                "[audio] 环境音停：站点 {site_id} · 档 {} 没有现象主表行（真源记错误并停）",
                label(&committed.name)
            );
            return;
        }
        AmbientRoute::NoRow {
            site_has_rows: false,
        } => {
            error!(
                "[audio] 环境音停：站点 {site_id} 既没有档 {} 的行也没有「其它」行，该站在现象环境音表里一行都没有（真源记错误、取到空名，只停不放）",
                label(&committed.name)
            );
            return;
        }
        AmbientRoute::NoRow {
            site_has_rows: true,
        } => {
            info!(
                "环境音停：站点 {site_id} 既没有档 {} 的行也没有「其它」行（真源取到空名，只停不放，不记错误）",
                label(&committed.name)
            );
            return;
        }
    };
    let Some(stream) = routing
        .streams
        .0
        .get(&(route.cue.clone(), route.package.clone()))
    else {
        // 解析期已全量核过可达路由；到不了这支。
        panic!(
            "环境音路由的流不在表里：{} @ {}",
            label(&route.cue),
            label(&route.package)
        );
    };
    if stream.waveforms > 1 {
        error!(
            "[audio] 环境音拒绝：站点 {site_id} · 档 {} 的 cue {} @ {} 带 {} 条波形——这是多轨序列 cue，序列结构（类型、各轨延迟、重放标志、轨到波形）不在运行时根的音频索引里，不拿其中一条顶替",
            label(&committed.name),
            label(&route.cue),
            label(&route.package),
            stream.waveforms
        );
        return;
    }
    let volume = bus.se_area_ambient * gate.factor();
    let handle = server.load::<AudioSource>(AssetPath::from(format!("moly://{}", stream.ogg)));
    let settings = if stream.loops {
        PlaybackSettings::LOOP
            .with_start_position(Duration::from_secs_f64(stream.loop_start))
            .with_duration(Duration::from_secs_f64(stream.loop_end - stream.loop_start))
    } else {
        PlaybackSettings::ONCE
    };
    channel.sink = Some(
        commands
            .spawn((
                AudioPlayer::new(handle),
                settings.with_volume(Volume::Linear(volume)),
                BusVolume::Ambient,
            ))
            .id(),
    );
    channel.playing = Some(route.cue.clone());
    info!(
        "环境音：站点 {site_id} · 档 {} → {}（平铺 2D，音量 {:.2} = 1.0 × {:.2}，{}）",
        label(&committed.name),
        label(&route.cue),
        volume,
        bus.se_area_ambient,
        if stream.loops {
            format!("循环 [{:.3}, {:.3}]", stream.loop_start, stream.loop_end)
        } else {
            "整轨一次性".to_string()
        },
    );
}

// ---- A 套邻近环境音管理器 -----------------------------------------------------

/// 站点场景侧的声源（真源声源对象的等价物）：cue + 包名 + 最大距离。
/// 位置取实体的全局变换。游戏内容里声源不挂在家具 prefab 上（全部站点
/// 合计恰一处，草原站瀑布），由 `site_sound` 模块从站点侧车的组件表挂
/// 上展开后的场景实体；包名侧车里没有，挂组件时补共享 SE 包。
#[derive(Component)]
pub(crate) struct SoundObject {
    pub cue: String,
    pub package: String,
    /// 距离调音的截断距离；真源构造默认 10.0。
    pub max_distance: f32,
}

impl Default for SoundObject {
    fn default() -> Self {
        Self {
            cue: String::new(),
            package: String::new(),
            max_distance: 10.0,
        }
    }
}

/// A 套运行态：单通道 + 前后名两字段 + 当前音量 + 就绪行是否已报。
///
/// 名字两字段的语义（真源逐字段对齐）：`pre`/`current` 都空 = 未起播；
/// 相等 = 同一声源持续（音量按 epsilon 门更新）；不等 = 换声源（停旧放新）。
/// 出厂本体里换源支不回写 `current`（名字从此不再变，第二次换源按构造
/// 不会再触发——该管理器整方法被热修补丁替换，本仓按可读体实现：换源
/// 支补上 `current` 的回写，不补则换源支一次后永久失效）。
#[derive(Resource, Default)]
pub(crate) struct ProximityState {
    pre: String,
    current: String,
    current_volume: f32,
    playback: Option<Entity>,
    announced: bool,
}

/// Update：A 套就近声源管理——每帧取「距离调音音量最大」的声源单通道
/// 播放；起播音量为 0，次帧起由 epsilon 门拉到真值（真源起播即把 0 传给
/// 播放器，音量走次帧更新那条路，一帧静默是源行为，照搬）。
pub(crate) fn advance_proximity(
    mut commands: Commands,
    server: Res<AssetServer>,
    bus: Res<VolumeBus>,
    gate: Res<AudioGate>,
    routing: Option<Res<Routing>>,
    mut state: ResMut<ProximityState>,
    sources: Query<
        (&SoundObject, &GlobalTransform),
        Without<moly_assets::scene_state::SourceInactive>,
    >,
    avatars: Query<&GlobalTransform, With<AvatarRoot>>,
    mut sinks: Query<&mut AudioSink>,
) {
    if !state.announced {
        state.announced = true;
        info!(
            "A 套就近环境音就绪：声源 {} · 站点场景侧车喂入（site_sound 挂组件）· 距离律 clamp(1-d/max,0,1) · epsilon 门",
            sources.iter().count()
        );
    }
    let Some(routing) = routing else {
        return; // 路由表未就绪
    };
    if sources.is_empty() {
        return;
    }
    // 玩家视点位置：真源取 avatar 视变换的位置，这边是相机跟随的同一变换。
    let Ok(avatar) = avatars.single() else {
        return;
    };
    let position = avatar.translation();

    // 就近选择：音量严格大于才换（音量为 0 的声源在只有它时也选得上——
    // 真源初值是 int 最小值重释为 float，即 f32::MIN）。
    let mut best: Option<(&SoundObject, f32)> = None;
    let mut best_volume = f32::MIN;
    for (source, transform) in &sources {
        let volume = proximity_volume(position, transform.translation(), source.max_distance);
        if volume > best_volume {
            best = Some((source, volume));
            best_volume = volume;
        }
    }
    let Some((nearest, volume)) = best else {
        return;
    };
    let audible_volume = volume * gate.factor();

    if state.pre.is_empty() || state.current.is_empty() {
        // 起播支：起播音量 0（真源把 0 传给播放器，音量参数被丢弃），
        // `current_volume` 不动（次帧的 epsilon 门负责拉到真值）。
        state.playback = play_proximity(
            &mut commands,
            &server,
            &routing.streams,
            &nearest.cue,
            &nearest.package,
        );
        state.pre = nearest.cue.clone();
        state.current = nearest.cue.clone();
        info!(
            "A 套起播：{}（音量 0.000 起播，次帧 epsilon 门拉到 {:.3}）",
            label(&nearest.cue),
            volume
        );
    } else if state.pre == state.current {
        // 同声源支：epsilon 门更新音量（门宽 = max(ε×8, 大者×1e-6)，
        // 逐位对齐真源：绝对差过门才写，并按「用户音量 × 距离音量」更新
        // 播放器）。
        let epsilon =
            (UNITY_MIN_FLOAT * 8.0).max(state.current_volume.abs().max(audible_volume.abs()) * 1e-6);
        if (audible_volume - state.current_volume).abs() >= epsilon {
            state.current_volume = audible_volume;
            if let Some(entity) = state.playback {
                if let Ok(mut sink) = sinks.get_mut(entity) {
                    sink.set_volume(Volume::Linear(bus.se_area_ambient * audible_volume));
                }
            }
        }
    } else {
        // 换声源支：停旧放新（可读体补 `current` 回写，见类型注释）。
        if let Some(old) = state.playback {
            if let Ok(mut entity_commands) = commands.get_entity(old) {
                entity_commands.despawn();
            }
        }
        state.pre = state.current.clone();
        state.playback = play_proximity(
            &mut commands,
            &server,
            &routing.streams,
            &nearest.cue,
            &nearest.package,
        );
        state.current = nearest.cue.clone();
        info!(
            "A 套换源：{} → {}（音量 0.000 起播）",
            label(&state.pre),
            label(&nearest.cue)
        );
    }
}

/// 距离调音律：`clamp(1 - d/max, 0, 1)`。
fn proximity_volume(listener: Vec3, source: Vec3, max_distance: f32) -> f32 {
    let distance = (listener - source).length();
    (1.0 - distance / max_distance).clamp(0.0, 1.0)
}

/// A 套放一路 cue：起播音量 0（真源起播把 0 传给播放器）；cue 对不上流表
/// 静默跳过（真源播放器同款——A 套的声源由站点场景喂，喂进来的 cue 不在
/// 流表不拦运行）。
fn play_proximity(
    commands: &mut Commands,
    server: &Res<AssetServer>,
    streams: &Streams,
    cue: &str,
    package: &str,
) -> Option<Entity> {
    let Some(stream) = streams.0.get(&(cue.to_string(), package.to_string())) else {
        warn!(
            "A 套：cue {} @ {} 不在流表里，跳过",
            label(cue),
            label(package)
        );
        return None;
    };
    let handle = server.load::<AudioSource>(AssetPath::from(format!("moly://{}", stream.ogg)));
    let settings = if stream.loops {
        PlaybackSettings::LOOP
            .with_start_position(Duration::from_secs_f64(stream.loop_start))
            .with_duration(Duration::from_secs_f64(stream.loop_end - stream.loop_start))
    } else {
        PlaybackSettings::ONCE
    };
    Some(
        commands
            .spawn((
                AudioPlayer::new(handle),
                settings.with_volume(Volume::Linear(0.0)),
            ))
            .id(),
    )
}

// ---- talk voice 通道 ---------------------------------------------------------

/// 一条 voice 行的说话者指称：voice 步的 `who`（角色变体 id）或
/// fixture_voice 步的 `fixture`（家具 id）。partvoice 的包随说话者变体
/// （真源按参与者逐个载包），不随 cue——cue 名里的段不编码变体归属。
#[derive(Clone, Copy, Debug)]
pub(crate) enum VoiceWho {
    /// voice 步：characterUnitId（名册变体 id）。
    Participant(i32),
    /// fixture_voice 步：mysekaiFixtureId（家具角色的单参包，不拼 unit）。
    Fixture(i32),
}

/// Resolved actor instance, independent of the package-routing referent.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum VoiceSpeaker {
    Npc(Entity),
    Fixture(Entity),
}

/// 行推进处投递的 voice 行请求：对话域 spawn、本域同帧消费后拆除（真源
/// 行内命令序 voice 先于 text——起播与文本行同拍，不隔帧）。
#[derive(Component)]
pub(crate) struct VoiceLine {
    pub talk_id: i32,
    pub step: usize,
    pub cue: String,
    /// partvoice 的说话者指称（voice_ 族不读它：包随 cue 的 script 段）。
    pub who: Option<VoiceWho>,
    /// Current-cast instance; unresolved identity does not change audio routing.
    pub speaker: Option<VoiceSpeaker>,
}

/// A future authored voice command. Preparing one only requests the exact raw
/// audio asset; it never creates an AudioPlayer, changes the single voice bus,
/// or advances dialogue timing.
#[derive(Clone, Debug)]
pub(crate) struct VoicePrefetch {
    pub cue: String,
    pub who: Option<VoiceWho>,
}

impl VoicePrefetch {
    pub(crate) fn new(cue: String, who: Option<VoiceWho>) -> Self {
        Self { cue, who }
    }
}

const VOICE_PREFETCH_CACHE_LIMIT: usize = 24;

/// Small bounded strong-handle cache. Keeping the handle alive is what makes an
/// AssetServer request a real preload rather than a dropped speculative load.
#[derive(Resource, Default)]
pub(crate) struct VoicePrefetchCache {
    handles: HashMap<String, Handle<AudioSource>>,
    order: VecDeque<String>,
}

impl VoicePrefetchCache {
    fn get(&mut self, path: &str) -> Option<Handle<AudioSource>> {
        let handle = self.handles.get(path)?.clone();
        if let Some(index) = self.order.iter().position(|key| key == path) {
            self.order.remove(index);
        }
        self.order.push_back(path.to_owned());
        Some(handle)
    }

    fn insert(&mut self, path: String, handle: Handle<AudioSource>) {
        if self.handles.contains_key(&path) {
            let _ = self.get(&path);
            return;
        }
        while self.handles.len() >= VOICE_PREFETCH_CACHE_LIMIT {
            let Some(oldest) = self.order.pop_front() else {
                break;
            };
            self.handles.remove(&oldest);
        }
        self.order.push_back(path.clone());
        self.handles.insert(path, handle);
    }
}

/// Attached to the actual player, not to a parallel clock or a cue lookup.
/// The consumed request Entity remains a generation-bearing playback token;
/// it need not remain alive after the line request has been consumed.
#[derive(Component)]
pub(crate) struct VoicePlaybackIdentity {
    pub request: Entity,
    pub speaker: Option<VoiceSpeaker>,
}

/// Single dialogue-voice bus: a new Voice line stops the old one before lookup.
/// A live line can finish naturally while waiting for a manual click; disposing
/// the completed/skipped talk stops it and cancels deferred line requests.
#[derive(Resource, Default)]
pub(crate) struct VoiceChannel {
    sink: Option<Entity>,
    /// 在播 cue（起播时记下，账目行用）。
    cue: Option<String>,
}

impl VoiceChannel {
    /// Actual player entity, including while its asset is still loading.
    /// Presence alone is neither audible playback nor an analyzer sample.
    pub(crate) fn active_sink(&self) -> Option<Entity> {
        self.sink
    }
}

/// 一条 voice 行的路由裁决：voice_ 族按 cue 定包（既有域，行为不变）；
/// partvoice 族按说话者查路由表——真源链是「说话者变体 → 双载的包，cue
/// 在包里才响」，此处同构：包名查表、cue 查流表，miss 与门拒分类具名。
enum VoiceRoute {
    /// 流表命中：cue 在这个包里，起播。`partvoice` 标记走 partvoice 日志
    /// 面（voice_ 族的日志面原样保留，账目不混）。
    Play { package: String, partvoice: bool },
    /// voice_ 族的流表 miss：按语料账本分类具名（上游无包/未提取面）。
    TalkMiss { package: String },
    /// partvoice 结构性不播：路由表缺席 / 指称缺失 / 说话者不在表 / 门拒
    /// ——真源这几条都不载包（对话照走），此处具名记 info。
    Refuse { reason: String },
    /// partvoice cue 不在说话者的包里：真源 ExistsCueName miss 静默跳过，
    /// 此处记 warn 并带上试过的包（缺 cue 账目的分类依据）。
    Miss { who: String, packages: Vec<String> },
}

/// 行 → 裁决。voice_ 族先判（前缀直出包名）；其余 cue 若非 partvoice
/// 前缀也具名拒绝（词表外的 cue 不进任何族）。
fn route_voice(
    line: &VoiceLine,
    partvoice: Option<&PartVoiceMap>,
    streams: &Streams,
) -> VoiceRoute {
    if let Some(package) = voice_package(&line.cue) {
        return if streams.0.contains_key(&(line.cue.clone(), package.clone())) {
            VoiceRoute::Play {
                package,
                partvoice: false,
            }
        } else {
            VoiceRoute::TalkMiss { package }
        };
    }
    if !line.cue.starts_with("partvoice_") {
        return VoiceRoute::Refuse {
            reason: "cue 非 voice_/partvoice_ 族的已知前缀".to_owned(),
        };
    }
    let Some(map) = partvoice else {
        return VoiceRoute::Refuse {
            reason: "partvoice 路由表缺席，fail-closed".to_owned(),
        };
    };
    // 说话者 → 试探包集：主链双包并试（真源双载，cue 落在哪份就播哪份），
    // 蛋链单包；门拒/不在表=真源不载包，直接拒绝。
    let (who, packages): (String, Vec<String>) = match line.who {
        Some(VoiceWho::Participant(id)) => match map.participants.get(&id) {
            None => {
                return VoiceRoute::Refuse {
                    reason: format!("说话者 {id} 不在 partvoice 路由表"),
                }
            }
            Some(ParticipantRoute::Gated(reason)) => {
                return VoiceRoute::Refuse {
                    reason: format!("说话者 {id} 门拒（{reason}），真源不载包"),
                }
            }
            Some(ParticipantRoute::Packages { scenario, mysekai }) => (
                format!("说话者 {id}"),
                vec![mysekai.clone(), scenario.clone()],
            ),
        },
        Some(VoiceWho::Fixture(fixture)) => match map.fixtures.get(&fixture) {
            None => {
                return VoiceRoute::Refuse {
                    reason: format!("家具 {fixture} 不在 partvoice 路由表"),
                }
            }
            Some(package) => (format!("家具 {fixture}"), vec![package.clone()]),
        },
        None => {
            return VoiceRoute::Refuse {
                reason: "步无 who/fixture 指称，说话者变体无从定位".to_owned(),
            }
        }
    };
    for package in &packages {
        if streams.0.contains_key(&(line.cue.clone(), package.clone())) {
            return VoiceRoute::Play {
                package: package.clone(),
                partvoice: true,
            };
        }
    }
    VoiceRoute::Miss { who, packages }
}

/// Request exact future authored voice assets while a conversation is already
/// known. This deliberately performs no playback and never touches VoiceChannel:
/// the source StopVoiceAll/ExistsCueName/PlayVoice ordering remains owned by
/// serve_voice when the actual command reaches the script cursor.
pub(crate) fn prefetch_voice(
    server: Res<AssetServer>,
    routing: Option<Res<Routing>>,
    partvoice: Option<Res<PartVoiceMap>>,
    active: Option<Res<crate::talk::ActiveTalk>>,
    player: Option<Res<crate::player_talk::PlayerTalkSession>>,
    mut cache: ResMut<VoicePrefetchCache>,
) {
    let Some(routing) = routing else {
        return;
    };
    let mut requests = Vec::new();
    if let Some(active) = active {
        requests.extend(active.voice_prefetches());
    }
    if let Some(player) = player {
        requests.extend(player.voice_prefetches());
    }
    requests.truncate(VOICE_PREFETCH_CACHE_LIMIT);

    for request in requests {
        let line = VoiceLine {
            talk_id: 0,
            step: 0,
            cue: request.cue,
            who: request.who,
            speaker: None,
        };
        let VoiceRoute::Play { package, .. } =
            route_voice(&line, partvoice.as_deref(), &routing.streams)
        else {
            continue;
        };
        let Some(stream) = routing.streams.0.get(&(line.cue.clone(), package)) else {
            continue;
        };
        let path = format!("moly://{}", stream.ogg);
        if cache.get(&path).is_some() {
            continue;
        }
        let handle = server.load::<AudioSource>(AssetPath::from(path.clone()));
        debug!("[voice-prefetch] requested {}", label(&line.cue));
        cache.insert(path, handle);
    }
}

/// Update（对话链内、步进之后）：吃掉行请求——停旧声、查流表、起播或
/// 具名跳过。缺 cue 真源同款是跳过且对话照走（ExistsCueName 无 else 无
/// 日志），这里照做并按本仓纪律把跳过记响：上游无包的走语料账本具名，
/// 其余不在表里的点名为未提取消费面，不静默不 panic。
pub(crate) fn serve_voice(
    mut commands: Commands,
    time: Res<Time>,
    server: Res<AssetServer>,
    bus: Res<VolumeBus>,
    gate: Res<AudioGate>,
    corpus: Option<Res<VoiceCorpus>>,
    routing: Option<Res<Routing>>,
    partvoice: Option<Res<PartVoiceMap>>,
    mut prefetch: ResMut<VoicePrefetchCache>,
    mut channel: ResMut<VoiceChannel>,
    lines: Query<(Entity, &VoiceLine)>,
    sinks: Query<&AudioSink>,
    players: Query<&AudioPlayer<MeteredVoiceSource>>,
    sources: Query<&VoiceSource>,
    metered_sources: Res<Assets<MeteredVoiceSource>>,
) {
    // ONCE is not automatically despawned. Reap completion, removed players and
    // terminal asset failures, while retaining real in-flight asset loads.
    if let Some(entity) = channel.sink {
        if crate::voice_pcm::finished_or_failed(
            entity,
            &server,
            &sources,
            &players,
            &metered_sources,
            &sinks,
        ) {
            if let Ok(mut entity_commands) = commands.get_entity(entity) {
                entity_commands.despawn();
            }
            channel.sink = None;
            channel.cue = None;
        }
    }
    let Some(routing) = routing else {
        return; // 流表未就绪：行请求留池等（对话能走到 voice 步时表早已就绪）
    };
    for (entity, line) in &lines {
        // 单声道：先停旧声再查表（真源顺序——StopVoiceAll 在存在性判定前）。
        if let Some(old) = channel.sink.take() {
            if let Ok(mut entity_commands) = commands.get_entity(old) {
                entity_commands.despawn();
            }
        }
        match route_voice(&line, partvoice.as_deref(), &routing.streams) {
            VoiceRoute::Play { package, partvoice } => {
                let Some(stream) = routing.streams.0.get(&(line.cue.clone(), package.clone()))
                else {
                    // 裁决刚判过在表：同帧不可能被撤（流表是解析后只读资源）。
                    unreachable!("route_voice 判定在表后流表被改写")
                };
                // talk voice 的流全部非循环（整轨一次性）；音量 =
                // 1.0（真源 PlayVoice 基量）× 面板 vox_scenario。
                let volume = bus.vox_scenario * gate.factor();
                let path = format!("moly://{}", stream.ogg);
                let cached = prefetch.get(&path);
                let was_prefetched = cached.is_some();
                let handle = match cached {
                    Some(handle) => handle,
                    None => {
                        let handle = server.load::<AudioSource>(AssetPath::from(path.clone()));
                        prefetch.insert(path, handle.clone());
                        handle
                    }
                };
                let raw_ready_at_command =
                    matches!(server.get_load_state(handle.id()), Some(LoadState::Loaded));
                channel.sink = Some(
                    commands
                        .spawn((
                            VoiceSource::new(
                                handle,
                                time.elapsed_secs_f64(),
                                was_prefetched,
                                raw_ready_at_command,
                            ),
                            PlaybackSettings::ONCE.with_volume(Volume::Linear(volume)),
                            BusVolume::Voice,
                            VoicePlaybackIdentity {
                                request: entity,
                                speaker: line.speaker,
                            },
                        ))
                        .id(),
                );
                channel.cue = Some(line.cue.clone());
                if partvoice {
                    info!(
                        "partvoice 起播：段 {} 步 {} cue {} @ {}（时长 {:.2}s，整轨一次性，音量 {:.2} = 1.0 × 面板）",
                        line.talk_id,
                        line.step,
                        label(&line.cue),
                        label(&package),
                        stream.duration,
                        volume,
                    );
                } else {
                    info!(
                        "talk voice 起播：段 {} 步 {} cue {} @ {}（时长 {:.2}s，整轨一次性，音量 {:.2} = 1.0 × 面板）",
                        line.talk_id,
                        line.step,
                        label(&line.cue),
                        label(&package),
                        stream.duration,
                        volume,
                    );
                }
            }
            VoiceRoute::TalkMiss { package } => {
                let named = corpus
                    .as_deref()
                    .is_some_and(|corpus| corpus.uncovered.iter().any(|cue| cue == &line.cue));
                if named {
                    warn!(
                        "talk voice 缺 cue：段 {} 步 {} cue {}——上游无包（语料账本具名，真源跳过同款）",
                        line.talk_id,
                        line.step,
                        label(&line.cue),
                    );
                } else {
                    warn!(
                        "talk voice 缺 cue：段 {} 步 {} cue {} @ {} 不在流表（fixture-talks 语料面未提取），对话照走",
                        line.talk_id,
                        line.step,
                        label(&line.cue),
                        label(&package),
                    );
                }
            }
            VoiceRoute::Refuse { reason } => {
                info!(
                    "partvoice 跳过：段 {} 步 {} cue {}——{}，对话照走",
                    line.talk_id,
                    line.step,
                    label(&line.cue),
                    reason,
                );
            }
            VoiceRoute::Miss { who, packages } => {
                let tried: Vec<String> = packages.iter().map(|p| label(p)).collect();
                warn!(
                    "partvoice 缺 cue：段 {} 步 {} cue {} 不在 {} 的包 [{}]（真源 ExistsCueName miss 静默跳过同款），对话照走",
                    line.talk_id,
                    line.step,
                    label(&line.cue),
                    who,
                    tried.join(", "),
                );
            }
        }
        commands.entity(entity).despawn();
    }
    if channel.sink.is_none() {
        channel.cue = None; // 停旧声后没接上新声：账目不留尾巴
    }
}

// ---- 一次性 SE 通道（事件族） -------------------------------------------------

/// The field scene installs these three substitutions in SoundManager.
/// Unlisted cues retain their identity, including subwindow open/close cues.
fn themed_se_cue(cue: &str) -> &str {
    match cue {
        "SE_DECIDE1" => "se_mysekai_ui_decision",
        "SE_CANCEL" => "se_mysekai_ui_cancel",
        "SE_REWARD_DIALOG_OPEN" => "se_get_blueprint",
        other => other,
    }
}

/// 公共 mysekai SE 包（loop.json 的包键形）：真源常量
/// `MYSEKAI_COMMON_SE_NAME = "mysekai/sound/se/se_mysekai"`——一次性事件
/// SE 的 cue 全部活在共享包 `se_mysekai` 里（A 套声源侧也用同一份：站点
/// 场景喂入的声源 cue 不带包名，挂组件时由此补上）。
pub(crate) const SE_PACKAGE: &str = "mysekai__sound__se__se_mysekai";

/// 一次性 SE 的音量类（九类面板的消费键；真源 SoundDefine.SeCategoryType
/// 的两支）。类→cue 的绑定在 ACF 侧（盘上无 .acf 读不出），按 cue 家族
/// 语义归：场景动作（采集受击）→ Ingame，界面动作（对话窗点跳/步进、
/// 摆放编辑）→ Ui——归法是具名 mock。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeClass {
    Ingame,
    Ui,
}

impl SeClass {
    fn volume(self, bus: &VolumeBus) -> f32 {
        match self {
            Self::Ingame => bus.se_ingame,
            Self::Ui => bus.se_ui,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Ingame => "se_ingame",
            Self::Ui => "se_ui",
        }
    }
}

/// 一次一次性 SE 请求（事件侧入队的三元组）：cue + 音量类 + 日志标签。
#[derive(Debug, Clone)]
pub struct SeRequest {
    /// A generation-safe playback scope; None is an ordinary world/UI sound.
    pub owner: Option<Entity>,
    /// A plain cue, or an AnimationEvent's bundle path whose leaf is the cue.
    /// A supplied bundle path selects its own bank, never a same-name fallback.
    pub cue: String,
    pub class: SeClass,
    /// 事件来源标签：事件→cue→起播/拒发行的可推导链靠它拼接。
    pub source: &'static str,
}

/// 一次性 SE 请求队列。写者（harvest `on_damage`、talk 步进/窗体、
/// `fixture_edit` 动作）先入队，`advance_se` 后排空——链序在 schedule
/// 里显式约束。The second list holds source buttons'
/// `CustomSelectableDefine.PlaySE` calls; they resolve against the cue bank
/// when the queue drains.
#[derive(Resource, Default)]
pub struct SeRequests(pub Vec<SeRequest>, pub(crate) Vec<ButtonSe>);

/// One `CustomSelectableDefine.PlaySE(se, otherSeName)` call.
#[derive(Debug, Clone)]
pub(crate) struct ButtonSe {
    pub(crate) se: moly_law::ui::custom_button::SeType,
    pub(crate) other_se_name: String,
}

impl SeRequests {
    /// A source button's click sound through its serialized SE fields.
    pub(crate) fn source_button(
        &mut self,
        layouts: &crate::ui_layout::UiLayouts,
        key: &str,
        path: &str,
    ) {
        if let Some((se, other_se_name)) = layouts.button_se(key, path) {
            self.button(se, other_se_name);
        }
    }

    /// Queues a `PlaySE(se, otherSeName)` call.
    pub(crate) fn button(&mut self, se: moly_law::ui::custom_button::SeType, other_se_name: String) {
        self.1.push(ButtonSe { se, other_se_name });
    }
}

/// The stream a plain cue name plays from: the shared MySekai SE bank, then
/// the built-in and downloaded common menu banks.
fn plain_se_stream<'a>(streams: &'a Streams, cue: &str) -> Option<&'a StreamRow> {
    [SE_PACKAGE, "MenuCommon_Built_in", "MenuCommon"]
        .iter()
        .find_map(|package| streams.0.get(&(cue.to_owned(), (*package).to_owned())))
}

/// 一次性 SE 排空的排序锚：写者系统在各自插件里 `.before(Drain)` 自证
/// 先于读者（写者签名不必出模块——有的写者参数里有私有类型，排序走集
/// 不走系统名）。
#[derive(SystemSet, Debug, Hash, PartialEq, Eq, Clone)]
pub(crate) enum SeDrainSet {
    Drain,
}

/// Live one-shot players, playback accounting and once-per-cue missing warnings.
#[derive(Resource, Default)]
pub(crate) struct SeChannel {
    live: Vec<Entity>,
    requests: usize,
    played: usize,
    warned_missing: HashSet<String>,
}

/// Drain accepted requests through the same one-shot player on both targets.
/// Missing decoded cues are reported without substituting another sound.
/// Playback volume is the source player's unit volume times the selected bus.
pub(crate) fn advance_se(
    mut commands: Commands,
    server: Res<AssetServer>,
    bus: Res<VolumeBus>,
    gate: Res<AudioGate>,
    routing: Option<Res<Routing>>,
    mut queue: ResMut<SeRequests>,
    mut channel: ResMut<SeChannel>,
    sinks: Query<&AudioSink>,
    players: Query<&AudioPlayer<AudioSource>>,
    owners: Query<()>,
) {
    channel.live.retain(|entity| {
        if !one_shot_finished_or_failed(*entity, &server, &players, &sinks) {
            return true;
        }
        if let Ok(mut entity_commands) = commands.get_entity(*entity) {
            entity_commands.despawn();
        }
        false
    });
    let Some(routing) = routing else {
        return; // 路由表未就绪：请求留队（就绪后排空，窗口照样去重）
    };
    // PlaySE: a table cue plays; Other plays its serialized name only when
    // that cue exists in a loaded bank (else the source logs an error and
    // nothing plays); None plays nothing.
    let buttons = std::mem::take(&mut queue.1);
    for button in buttons {
        let exists = |cue: &str| plain_se_stream(&routing.streams, cue).is_some();
        match moly_law::ui::custom_button::play_se(button.se, &button.other_se_name, exists) {
            Some(cue) => queue.0.push(SeRequest {
                owner: None,
                cue,
                class: SeClass::Ui,
                source: "source-button",
            }),
            None if button.se == moly_law::ui::custom_button::SeType::Other => error!(
                "SE: button cue {:?} is not in a loaded bank; nothing plays",
                button.other_se_name
            ),
            None => {}
        }
    }
    for request in queue.0.drain(..) {
        if request
            .owner
            .is_some_and(|owner| owners.get(owner).is_err())
        {
            continue;
        }
        channel.requests += 1;
        let (cue, source_package) = match request.cue.rsplit_once('/') {
            Some((_, cue)) => (cue, Some(request.cue.replace('/', "__"))),
            None => (themed_se_cue(&request.cue), None),
        };
        let stream = if let Some(package) = source_package {
            routing.streams.0.get(&(cue.to_owned(), package))
        } else {
            plain_se_stream(&routing.streams, cue)
        };
        let Some(stream) = stream else {
            if channel.warned_missing.insert(request.cue.to_string()) {
                warn!(
                    "SE 跳过：cue {} 不在本快照的流表（fail-closed 跳过；除非真源的包里也没有这个 cue，这是提取/流表缺口；每 cue 只告警一次）",
                    label(&request.cue)
                );
            }
            continue;
        };
        let volume = request.class.volume(&bus) * gate.factor();
        let handle = server.load::<AudioSource>(AssetPath::from(format!("moly://{}", stream.ogg)));
        let entity = commands
            .spawn((
                AudioPlayer::new(handle),
                PlaybackSettings::ONCE.with_volume(Volume::Linear(volume)),
                BusVolume::Se(request.class),
            ))
            .id();
        if let Some(owner) = request.owner {
            commands.entity(entity).insert(ScopedSe(owner));
        }
        channel.live.push(entity);
        channel.played += 1;
        info!(
            "SE 起播：{}（{} · {} · 音量 {:.2} = 1.0 × {:.2} · 平铺 2D 一次性）",
            label(cue),
            request.source,
            request.class.label(),
            volume,
            volume,
        );
    }
}

#[derive(Component)]
pub(crate) struct ScopedSe(pub Entity);

/// Stop only the departing controller's queued and already-playing effects.
/// Other fixtures, UI clicks, BGM and ambient sound are intentionally untouched.
pub(crate) fn dispose_scoped_se(world: &mut World, owner: Entity) {
    if let Some(mut queue) = world.get_resource_mut::<SeRequests>() {
        queue.0.retain(|request| request.owner != Some(owner));
    }
    let entities: Vec<_> = world
        .query::<(Entity, &ScopedSe)>()
        .iter(world)
        .filter_map(|(entity, scope)| (scope.0 == owner).then_some(entity))
        .collect();
    for entity in &entities {
        if let Some(sink) = world.get::<AudioSink>(*entity) {
            sink.stop();
        }
        if let Ok(entity) = world.get_entity_mut(*entity) {
            entity.despawn();
        }
    }
    if let Some(mut channel) = world.get_resource_mut::<SeChannel>() {
        channel.live.retain(|entity| !entities.contains(entity));
    }
}

#[cfg(test)]
mod scoped_se_tests {
    use super::*;
    #[test]
    fn cancellation_retires_only_the_departing_owner() {
        let mut world = World::new();
        world.init_resource::<SeRequests>();
        world.init_resource::<SeChannel>();
        let first = world.spawn_empty().id();
        let second = world.spawn_empty().id();
        let old = world.spawn(ScopedSe(first)).id();
        let newer = world.spawn(ScopedSe(second)).id();
        world.resource_mut::<SeChannel>().live = vec![old, newer];
        for owner in [Some(first), Some(second), None] {
            world.resource_mut::<SeRequests>().0.push(SeRequest {
                owner,
                cue: "test".into(),
                class: SeClass::Ingame,
                source: "test",
            });
        }
        dispose_scoped_se(&mut world, first);
        assert!(world.get_entity(old).is_err());
        assert!(world.get_entity(newer).is_ok());
        assert_eq!(world.resource::<SeRequests>().0.len(), 2);
        assert_eq!(world.resource::<SeChannel>().live, vec![newer]);
    }
}

/// AudioSink absence alone means neither "still loading" nor "finished".
/// Bevy only creates a sink once its AudioSource is present; a failed load
/// otherwise leaves an ONCE player and its channel bookkeeping alive forever.
fn one_shot_finished_or_failed(
    entity: Entity,
    server: &AssetServer,
    players: &Query<&AudioPlayer<AudioSource>>,
    sinks: &Query<&AudioSink>,
) -> bool {
    let Ok(player) = players.get(entity) else {
        return true;
    };
    if let Ok(sink) = sinks.get(entity) {
        return sink.empty();
    }
    if let Some(LoadState::Failed(error)) = server.get_load_state(player.0.id()) {
        warn!(
            "audio one-shot {entity:?} failed loading {:?}: {error}; releasing player",
            player.0.path()
        );
        return true;
    }
    // Loaded-without-sink can also mean unavailable output or a pending Bevy
    // update; do not turn a wall-clock timeout into a playback-end event.
    false
}

// ---- 周期账目 ---------------------------------------------------------------

/// Update（2s）：四条通道各一行——档→曲（BGM）、档→cue→音量（环境音，
/// 平铺）、声源数→当前通道与音量（A 套）、请求/起播计数（一次性
/// SE）。
pub(crate) fn report(
    phenomenon: Res<CurrentPhenomenon>,
    site: Option<Res<SiteActive>>,
    routing: Option<Res<Routing>>,
    bgm: Res<BgmChannel>,
    ambient: Res<AmbientChannel>,
    proximity: Res<ProximityState>,
    sources: Query<(), With<SoundObject>>,
    voice: Res<VoiceChannel>,
    se: Res<SeChannel>,
    sinks: Query<&AudioSink>,
) {
    if routing.is_none() {
        return; // 路由表未就绪：parse 的就绪行是首条账目
    }
    let site_id = site.map_or_else(|| "未定".to_string(), |site| site.site_id.to_string());
    match &bgm.voice {
        Some(voice) => {
            let segment = if voice.loop_sink.is_some() && voice.handoff_done {
                "loop"
            } else if voice.loop_sink.is_some() {
                "intro"
            } else {
                "once"
            };
            info!(
                "BGM 账目：站点 {site_id} · 档 {} · 曲 {} · 段 {} · 音量 {:.2}",
                label(&phenomenon.0),
                label(&voice.cue),
                segment,
                voice.applied_volume
            );
        }
        None => info!(
            "BGM 账目：站点 {site_id} · 档 {} · 无曲",
            label(&phenomenon.0)
        ),
    }
    let ambient_volume = ambient
        .sink
        .and_then(|entity| sinks.get(entity).ok())
        .map(|sink| sink.volume().to_linear())
        .unwrap_or(0.0);
    match &ambient.playing {
        Some(cue) => info!(
            "环境音账目：站点 {site_id} · cue {} · 音量 {:.2}（平铺）",
            label(cue),
            ambient_volume
        ),
        None => info!("环境音账目：站点 {site_id} · 无"),
    }
    info!(
        "A 套账目：声源 {} · 当前 {} · 音量 {:.3}",
        sources.iter().count(),
        if proximity.current.is_empty() {
            "无".to_string()
        } else {
            label(&proximity.current)
        },
        proximity.current_volume
    );
    let voice_volume = voice
        .sink
        .and_then(|entity| sinks.get(entity).ok())
        .map(|sink| sink.volume().to_linear())
        .unwrap_or(0.0);
    match (&voice.cue, voice.sink) {
        (Some(cue), Some(entity)) if sinks.get(entity).map(|s| !s.empty()).unwrap_or(false) => {
            info!(
                "talk voice 账目：在播 {} · 音量 {:.2}",
                label(cue),
                voice_volume
            );
        }
        _ => info!("talk voice 账目：无在播"),
    }
    info!(
        "SE 账目：请求 {} · 起播 {} · 在响 {}",
        se.requests,
        se.played,
        se.live.len()
    );
}

/// 音频域资源初始化。当前档资源在这里兜底建（天气插件两侧同挂后此兜底
/// 照旧——`init_resource` 幂等，谁先谁建、值同为默认档）；通道运行态也在
/// 此落位。
pub(crate) fn install(app: &mut App) {
    app.init_resource::<VolumeBus>()
        .init_resource::<AudioGate>()
        .init_resource::<LocalVolumeSettings>()
        .init_resource::<CurrentPhenomenon>()
        .init_resource::<UserMusicPlaySettings>()
        .init_resource::<BgmChannel>()
        .init_resource::<AmbientChannel>()
        .init_resource::<ProximityState>()
        .init_resource::<VoicePrefetchCache>()
        .init_resource::<VoiceChannel>()
        .init_resource::<SeRequests>()
        .init_resource::<SeChannel>();
}

#[cfg(test)]
mod stream_manifest_tests {
    use super::*;
    use serde_json::json;

    fn manifest(rows: serde_json::Value) -> serde_json::Value {
        json!({"packages":[{"package":"source-bank","streams":rows}]})
    }
    fn valid() -> serde_json::Value {
        json!({"cue":"available","subsong":1,"loop":false,
            "ogg":"audio/source.ogg","durationSeconds":1.25})
    }
    #[test]
    fn source_diagnostic_does_not_poison_unrelated_audio_or_fabricate_a_stream() {
        let streams = parse_streams(
            &manifest(json!([valid(), {
                "cue":"missing","error":"no waveform in this archive carries this cue name"
            }])),
            "phenomena/",
        );
        assert_eq!(streams.0.len(), 1);
        assert!(streams
            .0
            .get(&("missing".into(), "source-bank".into()))
            .is_none());
        assert_eq!(
            streams.0[&("available".into(), "source-bank".into())].ogg,
            "phenomena/audio/source.ogg"
        );
    }
    #[test]
    #[should_panic(expected = "loop")]
    fn malformed_playable_row_still_fails_closed() {
        let mut row = valid();
        row["loop"] = json!(null);
        parse_streams(&manifest(json!([row])), "");
    }
    #[test]
    #[should_panic(expected = "partial playable")]
    fn diagnostic_cannot_hide_corrupt_playable_data() {
        let mut row = valid();
        row["error"] = json!("failed");
        parse_streams(&manifest(json!([row])), "");
    }
    #[test]
    #[should_panic(expected = "nonempty string")]
    fn malformed_diagnostic_is_not_silently_ignored() {
        parse_streams(&manifest(json!([{"cue":"bad","error":false}])), "");
    }
}
