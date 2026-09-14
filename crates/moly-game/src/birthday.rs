//! 生日派对主表（`birthday-parties.json`）与站点地图庆典门的档期律。
//!
//! 真源入口：站点地图点开已解锁的庆典庭院，先问主管理器的
//! GetMasterBirthdayPartiesInSession——生日派对主表按「档期内」过滤；
//! 一行都没有则弹一钮对话框（不去），非空才走换站。档期判据是
//! TimeUtility.IsWithinTime 的两臂律（行上 startAt/closedAt，
//! epoch 毫秒；closedAt 0 = 无截止），见 [`is_within_time`]。
//!
//! 「现在」：真源的当前时刻是服务器对过表的基准加启动以来的实时偏移
//! （主管理器域）。本机离线无对表基准，墙钟 epoch 毫秒作替身；验收钩子
//! `MOLY_BIRTHDAY_NOW_MS` 把「现在」钉成定值——行值零 mock，钉的只是
//! 钟（同 MOLY_SITEMAP_AUTOCLICK_* 家族）。

use bevy::asset::{Assets, LoadState};
use bevy::prelude::*;

use moly_assets::json::JsonAsset;

// ---------------------------------------------------------------------------
// 装载：请求 → 解析
// ---------------------------------------------------------------------------

/// 主表 JSON 的装载请求；解析成功后即撤。
#[derive(Resource)]
pub(crate) struct BirthdayPartiesHandle(Handle<JsonAsset>);

/// Startup：请求装载生日派对主表。
pub(crate) fn load(mut commands: Commands, server: Res<AssetServer>) {
    commands.insert_resource(BirthdayPartiesHandle(
        server.load::<JsonAsset>(moly_assets::birthday_parties()),
    ));
}

/// Update：主表到齐即解析。装载失败响亮 panic（资产边界的唯一拒绝点）；
/// 行缺档期字段同样响亮拒绝——门读不了档期就是读不了行。
pub(crate) fn parse(
    mut commands: Commands,
    server: Res<AssetServer>,
    jsons: Res<Assets<JsonAsset>>,
    handle: Option<Res<BirthdayPartiesHandle>>,
) {
    let Some(handle) = handle else {
        return;
    };
    if let LoadState::Failed(err) = server.load_state(&handle.0) {
        panic!("生日派对主表装载失败：{err:?}");
    }
    let Some(json) = jsons.get(&handle.0) else {
        return;
    };
    let value: serde_json::Value = serde_json::from_str(&json.0)
        .unwrap_or_else(|err| panic!("生日派对主表不是合法 JSON：{err}"));
    let rows = value
        .get("rows")
        .and_then(|v| v.as_array())
        .unwrap_or_else(|| panic!("生日派对主表缺 rows 数组"));
    let mut parties = Vec::with_capacity(rows.len());
    for row in rows {
        parties.push(PartyRow {
            id: int_field(row, "id"),
            start_at: int_field(row, "startAt"),
            closed_at: int_field(row, "closedAt"),
            // 门只读档期；包名是日志里的行标签，快照间字段形状有差，
            // 缺了用行 id 标。
            assetbundle_name: row
                .get("assetbundleName")
                .and_then(|v| v.as_str())
                .map(str::to_owned),
        });
    }
    let now = now_ms();
    let in_session: Vec<String> = parties
        .iter()
        .filter(|row| is_within_time(now, row.start_at, row.closed_at))
        .map(|row| row.label())
        .collect();
    let clock = if std::env::var("MOLY_BIRTHDAY_NOW_MS").is_ok() {
        "MOLY_BIRTHDAY_NOW_MS 钉值"
    } else {
        "墙钟"
    };
    info!(
        "[birthday] 派对主表就绪：{} 行，now={}（{clock}），档期内 {} 行{}——庆典门按档期放行/拒",
        parties.len(),
        now,
        in_session.len(),
        if in_session.is_empty() {
            String::new()
        } else {
            format!("：{}", in_session.join("，"))
        }
    );
    commands.insert_resource(BirthdayParties { rows: parties });
    commands.remove_resource::<BirthdayPartiesHandle>();
}

/// 行上的整型字段；缺失或非整型响亮拒绝并具名行与字段。
fn int_field(row: &serde_json::Value, field: &str) -> i64 {
    row.get(field)
        .and_then(|v| v.as_i64())
        .unwrap_or_else(|| panic!("生日派对主表行 {:?} 缺整型字段 {field}", row.get("id")))
}

// ---------------------------------------------------------------------------
// 数据面与档期律
// ---------------------------------------------------------------------------

/// 一行派对（门读 id 与档期；其余列属门后的投递域，暂不建模）。
struct PartyRow {
    id: i64,
    start_at: i64,
    closed_at: i64,
    assetbundle_name: Option<String>,
}

impl PartyRow {
    /// 日志标签：包名优先，缺了用行 id。
    fn label(&self) -> String {
        self.assetbundle_name
            .clone()
            .unwrap_or_else(|| format!("#{}", self.id))
    }
}

/// 生日派对主表。
#[derive(Resource)]
pub(crate) struct BirthdayParties {
    rows: Vec<PartyRow>,
}

impl BirthdayParties {
    pub(crate) fn len(&self) -> usize {
        self.rows.len()
    }

    /// GetMasterBirthdayPartiesInSession：主表按档期过滤后的行标签
    /// （主表序；包名优先，缺了用行 id）。
    pub(crate) fn labels_in_session(&self, now_ms: i64) -> Vec<String> {
        self.rows
            .iter()
            .filter(|row| is_within_time(now_ms, row.start_at, row.closed_at))
            .map(|row| row.label())
            .collect()
    }
}

/// TimeUtility.IsWithinTime(checkAt, startAt, endAt) 的两臂律：
/// `endAt == 0`（无截止）⇒ `startAt != 0` 且 `startAt <= checkAt`；
/// `endAt != 0` ⇒ `startAt <= checkAt < endAt`。左闭右开，起点未定
/// （startAt 0）的行永不入档。
pub(crate) fn is_within_time(check_at: i64, start_at: i64, end_at: i64) -> bool {
    if end_at == 0 {
        start_at != 0 && start_at <= check_at
    } else {
        start_at <= check_at && check_at < end_at
    }
}

/// 「现在」的 epoch 毫秒：`MOLY_BIRTHDAY_NOW_MS` 钉值优先（验收钩子，
/// 非法值响亮拒绝——静默退墙钟会把「开门」验成「关门」）；否则墙钟。
pub(crate) fn now_ms() -> i64 {
    if let Ok(pinned) = std::env::var("MOLY_BIRTHDAY_NOW_MS") {
        return pinned.trim().parse::<i64>().unwrap_or_else(|_| {
            panic!("MOLY_BIRTHDAY_NOW_MS 不是整型毫秒：{pinned:?}")
        });
    }
    wall_now_ms()
}

/// 墙钟的 epoch 毫秒。wasm 上 std 的 SystemTime 是未实现的桩（运行时
/// panic），epoch 走 JS 的 Date.now；native 走系统时钟。
#[cfg(target_arch = "wasm32")]
fn wall_now_ms() -> i64 {
    js_sys::Date::now() as i64
}

#[cfg(not(target_arch = "wasm32"))]
fn wall_now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("系统时钟早于 Unix 纪元")
        .as_millis() as i64
}
