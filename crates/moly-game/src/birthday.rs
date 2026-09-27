//! 生日派对主表（`birthdayParties`，经区域主表镜像读入）与站点地图庆典门的档期律。
//!
//! 真源入口：站点地图点开已解锁的庆典庭院，先问主管理器的
//! GetMasterBirthdayPartiesInSession——生日派对主表按「档期内」过滤；
//! 一行都没有则弹一钮对话框（不去），非空才走换站。档期判据是
//! TimeUtility.IsWithinTime 的两臂律（行上 startAt/closedAt，
//! epoch 毫秒；closedAt 0 = 无截止），见 [`is_within_time`]。
//!
//! "Now": the source's current time is the server date it last got plus the
//! real time since. Here it is the one server clock of the server model
//! (device time by default, or the fixed time the server document sets);
//! before the model is installed, device time. The native instrument
//! `MOLY_BIRTHDAY_NOW_MS` fixes that server clock through the model's native
//! overlay; game mode reads no environment variable.
//!
//! A table that is absent or malformed is named once by the master layer;
//! then no party is in session, the gate shows its no-party dialog, the
//! delivery and gather replies refuse by naming the table, and the rest of
//! the game runs as usual.

use bevy::prelude::*;

use moly_assets::json::master::{self, MasterData, MasterTable};

// ---------------------------------------------------------------------------
// 装载：请求 → 解析
// ---------------------------------------------------------------------------

/// The party master the gate and the delivery read.
const PARTIES: MasterTable<Vec<PartyRow>> = MasterTable {
    table: "birthdayParties",
    name: "birthdayParties (the festival gate and the delivery)",
    parse: parse_parties,
};

/// Startup：请求生日派对主表。
pub(crate) fn load(mut masters: ResMut<MasterData>) {
    masters.request(&PARTIES);
}

/// Update：主表到齐即落座。缺表或表形不对由主表层具名一次，落座一份
/// 零行、带缺表原因的主表：档期内恒无派对。
pub(crate) fn parse(mut commands: Commands, mut masters: ResMut<MasterData>) {
    let Some(result) = masters.take(&PARTIES) else {
        return;
    };
    let parties = match result {
        Ok(parties) => parties,
        Err(error) => {
            info!("[birthday] no party table: no party is in session");
            commands.insert_resource(BirthdayParties {
                rows: Vec::new(),
                missing: Some(error.to_string()),
            });
            return;
        }
    };
    let now = now_ms();
    let in_session: Vec<String> = parties
        .iter()
        .filter(|row| is_within_time(now, row.start_at, row.closed_at))
        .map(|row| row.label())
        .collect();
    let clock = if crate::server::server_now_ms().is_some() {
        "server clock"
    } else {
        "device time (the server model is not installed yet)"
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
    commands.insert_resource(BirthdayParties {
        rows: parties,
        missing: None,
    });
}

/// `birthdayParties`：行在主表序；缺整型档期字段的行具名拒绝整表。
fn parse_parties(text: &str) -> Result<Vec<PartyRow>, String> {
    master::rows(text)?
        .iter()
        .map(|row| {
            Ok(PartyRow {
                id: master::int(row, "id")?,
                start_at: master::int(row, "startAt")?,
                closed_at: master::int(row, "closedAt")?,
                // 门只读档期；包名是日志里的行标签，缺了用行 id 标。
                assetbundle_name: row
                    .get("assetbundleName")
                    .and_then(|v| v.as_str())
                    .map(str::to_owned),
                // The delivery site reads these two columns of the parties in
                // session; the gate does not.
                delivery_item_material_id: row
                    .get("deliveryItemMaterialId")
                    .and_then(|v| v.as_i64()),
                delivery_reward_material_id: row
                    .get("deliveryRewardMysekaiMaterialId")
                    .and_then(|v| v.as_i64()),
            })
        })
        .collect()
}

// ---------------------------------------------------------------------------
// 数据面与档期律
// ---------------------------------------------------------------------------

/// 一行派对（门读 id 与档期；配送站另读两列物料 id）。
struct PartyRow {
    id: i64,
    start_at: i64,
    closed_at: i64,
    assetbundle_name: Option<String>,
    /// `deliveryItemMaterialId`: the material the delivery spends.
    delivery_item_material_id: Option<i64>,
    /// `deliveryRewardMysekaiMaterialId`: the mysekai material a reward
    /// drop is.
    delivery_reward_material_id: Option<i64>,
}

/// One party in session as the delivery site reads it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct InSessionParty {
    pub(crate) id: i64,
    pub(crate) label: String,
    pub(crate) delivery_item_material_id: i64,
    pub(crate) delivery_reward_material_id: i64,
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
    /// Why the table is missing (named once by the master layer); it then
    /// has no rows.
    missing: Option<String>,
}

impl BirthdayParties {
    /// Why the root has no party table, for the replies that refuse by
    /// naming it.
    pub(crate) fn missing(&self) -> Option<&str> {
        self.missing.as_deref()
    }

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

    /// GetMasterBirthdayPartiesInSession for the delivery site: the rows in
    /// session, in master order, with the two material columns. A row in
    /// session without them is refused by name: the delivery cannot spend
    /// or drop without them.
    pub(crate) fn in_session(&self, now_ms: i64) -> Vec<InSessionParty> {
        self.rows
            .iter()
            .filter(|row| is_within_time(now_ms, row.start_at, row.closed_at))
            .map(|row| InSessionParty {
                id: row.id,
                label: row.label(),
                delivery_item_material_id: row.delivery_item_material_id.unwrap_or_else(|| {
                    panic!("生日派对主表行 {} 缺 deliveryItemMaterialId", row.label())
                }),
                delivery_reward_material_id: row.delivery_reward_material_id.unwrap_or_else(|| {
                    panic!("生日派对主表行 {} 缺 deliveryRewardMysekaiMaterialId", row.label())
                }),
            })
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

/// The epoch millisecond "now": the server clock, or device time before the
/// server model is installed.
pub(crate) fn now_ms() -> i64 {
    crate::server::server_now_ms().unwrap_or_else(crate::server::clock::device_now_ms)
}
