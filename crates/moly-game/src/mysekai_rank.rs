//! The master rank table (`mysekai-ranks.json`) and the rank model the menu
//! dialog and the info screen build from it.
//!
//! The table is master data, so it comes from the runtime root. The user's
//! total experience is server-side user state from the server document, one
//! value both screens read ([`UserTotalExp`]). A root without the table does not stop the
//! start; the first rank model asked of it panics with the missing file
//! named, as the source throws when its table is absent.

use bevy::asset::{Assets, LoadState};
use bevy::prelude::*;

use moly_assets::json::JsonAsset;
use moly_law::ui::mysekai_rank::{MasterMysekaiRank, MysekaiRankModel};

/// The table's load request; removed once it resolves.
#[derive(Resource)]
pub(crate) struct MysekaiRanksHandle(Handle<JsonAsset>);

/// The resolved table: its rows in master order, or why the root has none.
#[derive(Resource)]
pub(crate) struct MysekaiRanks(Result<Vec<MasterMysekaiRank>, String>);

/// `UserMysekaiGamedata.totalExp`, the server-side user state both rank
/// models read: the saved player data's value when it carries one, else the
/// server document's (its responses keep it current; the native overlay of
/// `MOLY_MENU_MOCK_TOTAL_EXP` lands in that document), else the document's
/// default before the document is installed.
#[derive(Resource)]
pub(crate) struct UserTotalExp(pub(crate) i32);

impl UserTotalExp {
    fn panel() -> i32 {
        crate::server::with_model(|model| model.document().gamedata.total_exp)
            .unwrap_or(crate::server::document::DEFAULT_TOTAL_EXP)
    }

    /// Imported player data's total experience, or the server document's.
    pub(crate) fn set(&mut self, imported: Option<i32>) {
        self.0 = imported.unwrap_or_else(Self::panel);
    }
}

impl Default for UserTotalExp {
    fn default() -> Self {
        UserTotalExp(crate::player_data::saved_total_exp().unwrap_or_else(Self::panel))
    }
}

/// Startup: request the table and seat the user's total experience.
pub(crate) fn load(mut commands: Commands, server: Res<AssetServer>) {
    commands.init_resource::<UserTotalExp>();
    commands.insert_resource(MysekaiRanksHandle(server.load::<JsonAsset>(moly_assets::mysekai_ranks())));
}

/// Update: parse the table once it arrives. A malformed table panics; a
/// missing file is recorded and named once.
pub(crate) fn parse(
    mut commands: Commands,
    server: Res<AssetServer>,
    jsons: Res<Assets<JsonAsset>>,
    handle: Option<Res<MysekaiRanksHandle>>,
) {
    let Some(handle) = handle else { return; };
    if let LoadState::Failed(err) = server.load_state(&handle.0) {
        let reason = format!("mysekai-ranks.json is not in this runtime root ({err})");
        warn!("[mysekai-rank] {reason}; the rank gauges refuse when a screen shows them");
        commands.insert_resource(MysekaiRanks(Err(reason)));
        commands.remove_resource::<MysekaiRanksHandle>();
        return;
    }
    let Some(json) = jsons.get(&handle.0) else { return; };
    let value: serde_json::Value = serde_json::from_str(&json.0)
        .unwrap_or_else(|err| panic!("mysekai-ranks.json is not JSON: {err}"));
    let rows = rows(&value).unwrap_or_else(|err| panic!("mysekai-ranks.json: {err}"));
    info!("[mysekai-rank] master rank table: {} rows, highest rank {}", rows.len(),
        rows.iter().map(|row| row.mysekai_rank).max().unwrap_or(0));
    commands.insert_resource(MysekaiRanks(Ok(rows)));
    commands.remove_resource::<MysekaiRanksHandle>();
}

/// The rows in the table's master order (`rowOrder`), each read from its
/// keyed entry.
fn rows(value: &serde_json::Value) -> Result<Vec<MasterMysekaiRank>, String> {
    if value["version"].as_i64() != Some(1) {
        return Err(format!("version {} is not 1", value["version"]));
    }
    let semantics = &value["semantics"];
    if semantics["table"].as_str() != Some("mysekaiRanks") || semantics["keyField"].as_str() != Some("id") {
        return Err("it is not the mysekaiRanks table keyed by id".into());
    }
    let order = value["rowOrder"].as_array().ok_or("no rowOrder")?;
    let entries = value["entries"].as_object().ok_or("no entries")?;
    if order.len() != entries.len() {
        return Err(format!("rowOrder has {} ids for {} entries", order.len(), entries.len()));
    }
    let int = |row: &serde_json::Value, field: &str| -> Result<i32, String> {
        row[field].as_i64().and_then(|v| i32::try_from(v).ok())
            .ok_or_else(|| format!("{field} of row {row} is not an int"))
    };
    order.iter().map(|id| {
        let id = id.as_i64().ok_or_else(|| format!("rowOrder id {id} is not an integer"))?;
        let row = entries.get(&id.to_string()).ok_or_else(|| format!("rowOrder id {id} has no entry"))?;
        let parsed = MasterMysekaiRank { id: int(row, "id")?, mysekai_rank: int(row, "mysekaiRank")?, total_exp: int(row, "totalExp")? };
        if i64::from(parsed.id) != id {
            return Err(format!("entry {id} carries id {}", parsed.id));
        }
        Ok(parsed)
    }).collect()
}

impl MysekaiRanks {
    /// `new MysekaiRankModel()` with the given total experience. Panics where
    /// the source throws, and on a root without the table.
    pub(crate) fn model(&self, total_exp: i32) -> MysekaiRankModel {
        let rows = self.0.as_ref().unwrap_or_else(|reason| panic!("rank model: {reason}"));
        MysekaiRankModel::new(rows, total_exp).unwrap_or_else(|err| panic!("rank model: {err}"))
    }
}
