//! `FixtureController.SetupSystemFixture`: the action a placed system or gate
//! fixture offers, and the action button it maps to.
//!
//! The source reads the fixture's master row. A gate fixture (fixture type
//! gate, 5) gets the action sensor type `Gate` and the effect action
//! `UpdateGate`. Any other fixture looks up its `mysekaiSystemFixtures` row
//! (`GetMasterSystemFixture`); without one it gets nothing. With one, a jump
//! table over the row's type (0 to 7) sets the sensor type:
//!
//! | system fixture type | sensor type (`MysekaiPlayerActionType`) | also |
//! |---|---|---|
//! | craft_tool (0) | CraftTool (7) | |
//! | chest (1) | Chest (5) | |
//! | home (2) | Home (2) | |
//! | blueprint_shop (3) | SecretShop (13) | |
//! | convert (4) | Convert (10) | effect action `UpdateConvert` |
//! | mysekai_information (5) | MysekaiInfo (8) | |
//! | music_play (6) | MusicPlay (9) | effect action `UpdateMusicPlay` |
//! | avatar_dress_up (7) | AvatarDressUp (12) | |
//!
//! `birthday` (8) is above the table's range: the source returns without a
//! sensor type, so a birthday fixture offers no action through this table.
//! Its cut-scene button reaches the button stack on its own path, gated by
//! `MysekaiFixtureUtility.CanShowBirthdayCutSceneButton` (not visiting, no
//! birthday context, `IsWithinBirthdayTimeByFixtureId`), which is not ported
//! here; see [`birthday_cut_scene_button_condition`].
//!
//! The sensor type maps to the button type through
//! [`ButtonType::from_action_type`]. Which of the classified buttons the
//! product stacks today is [`STACKED_ACTIONS`]; the others are classified
//! and withheld by name.
//!
//! Named gaps: the effect actions (`UpdateGate`, `UpdateConvert`,
//! `UpdateMusicPlay`: the fixture's activate effect while its function runs)
//! are not ported.
//!
//! Placed fixtures are identified by package (`mysekai__fixture__` +
//! `assetbundleName`), so a package whose master rows disagree on the
//! classification is refused by name.

use std::collections::HashMap;

use bevy::prelude::*;
use moly_assets::json::master::{self, MasterData, MasterTable};
use moly_assets::json::JsonAsset;
use moly_law::action_button::{ButtonType, FixtureType, PlayerActionType};
use serde_json::Value;

/// `Sekai.MysekaiSystemFixtureType`, by the master's string value.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum SystemFixtureType {
    CraftTool = 0,
    Chest = 1,
    Home = 2,
    BlueprintShop = 3,
    Convert = 4,
    MysekaiInformation = 5,
    MusicPlay = 6,
    AvatarDressUp = 7,
    Birthday = 8,
}

impl SystemFixtureType {
    /// The master's `mysekaiSystemFixtureType` string.
    pub(crate) fn from_master(value: &str) -> Option<Self> {
        Some(match value {
            "craft_tool" => Self::CraftTool,
            "chest" => Self::Chest,
            "home" => Self::Home,
            "blueprint_shop" => Self::BlueprintShop,
            "convert" => Self::Convert,
            "mysekai_information" => Self::MysekaiInformation,
            "music_play" => Self::MusicPlay,
            "avatar_dress_up" => Self::AvatarDressUp,
            "birthday" => Self::Birthday,
            _ => return None,
        })
    }
}

/// `SetupSystemFixture`'s sensor type for a fixture of `fixture_type` whose
/// system fixture row (if any) has type `system`. `None` where the source
/// sets no sensor type.
pub(crate) fn setup_system_fixture(
    fixture_type: FixtureType,
    system: Option<SystemFixtureType>,
) -> Option<PlayerActionType> {
    if fixture_type == FixtureType::Gate {
        return Some(PlayerActionType::Gate);
    }
    Some(match system? {
        SystemFixtureType::CraftTool => PlayerActionType::CraftTool,
        SystemFixtureType::Chest => PlayerActionType::Chest,
        SystemFixtureType::Home => PlayerActionType::Home,
        SystemFixtureType::BlueprintShop => PlayerActionType::SecretShop,
        SystemFixtureType::Convert => PlayerActionType::Convert,
        SystemFixtureType::MysekaiInformation => PlayerActionType::MysekaiInfo,
        SystemFixtureType::MusicPlay => PlayerActionType::MusicPlay,
        SystemFixtureType::AvatarDressUp => PlayerActionType::AvatarDressUp,
        // Above the jump table's range: no sensor type.
        SystemFixtureType::Birthday => return None,
    })
}

/// The button a sensor type shows. The product has no visiting mode (it is
/// always the player's own MySekai), so `IsVisiting` is false.
pub(crate) fn system_fixture_button(action: PlayerActionType) -> ButtonType {
    ButtonType::from_action_type(action, false)
}

/// The classified actions whose buttons the product stacks. Each needs its
/// `IsActionButtonTypeAvailable` rule and its click arm:
/// - `MusicPlay` -> `OpenMysekaiBGMSelect`: `CanShowBGMSelectButton` is
///   `!IsVisiting`, which holds in this product; the click opens the BGM
///   select screen ([`crate::bgm_select`]).
///
/// The others are withheld: their availability rules are not ported here.
pub(crate) const STACKED_ACTIONS: &[PlayerActionType] = &[PlayerActionType::MusicPlay];

/// The hook for `CanShowBirthdayCutSceneButton` (not visiting, no
/// `BirthdayContext`, `IsWithinBirthdayTimeByFixtureId`). Not ported: it
/// answers `None` ("not decided"), and a birthday fixture shows no button
/// through this module.
#[allow(dead_code)] // The birthday cut-scene button builds on it.
pub(crate) fn birthday_cut_scene_button_condition(_master_fixture_id: i32) -> Option<bool> {
    None
}

/// The system fixture master the classification reads.
const SYSTEM_FIXTURES: MasterTable<HashMap<i32, SystemFixtureType>> = MasterTable {
    table: "mysekaiSystemFixtures",
    name: "mysekaiSystemFixtures (the system fixture actions)",
    parse: parse_system_fixtures,
};

/// `mysekaiSystemFixtures`: the type of each row's fixture, by master fixture
/// id. A fixture named by two rows of different types is refused.
fn parse_system_fixtures(text: &str) -> Result<HashMap<i32, SystemFixtureType>, String> {
    let mut by_fixture = HashMap::new();
    for row in master::rows(text)? {
        let fixture = master::int32(&row, "mysekaiFixtureId")?;
        let raw = master::text(&row, "mysekaiSystemFixtureType")?;
        let kind = SystemFixtureType::from_master(raw)
            .ok_or_else(|| format!("row {}: unknown mysekaiSystemFixtureType {raw}", row["id"]))?;
        if let Some(previous) = by_fixture.insert(fixture, kind) {
            if previous != kind {
                return Err(format!(
                    "fixture {fixture} is named by system fixture rows of types {previous:?} and {kind:?}"
                ));
            }
        }
    }
    Ok(by_fixture)
}

/// The classification of every placed package: its fixture type and system
/// fixture type, once both tables have resolved.
#[derive(Resource, Default)]
pub(crate) struct SystemFixtures {
    fixture_master: Option<Handle<JsonAsset>>,
    system: Option<Result<HashMap<i32, SystemFixtureType>, String>>,
    /// `None` until resolved; `Err` names why no system fixture offers a
    /// button on this root.
    by_package: Option<Result<HashMap<String, Option<SystemFixtureType>>, String>>,
}

impl SystemFixtures {
    /// Both tables have resolved (to a table or to a named failure).
    pub(crate) fn resolved(&self) -> bool {
        self.by_package.is_some()
    }

    /// The system fixture type of a placed package (`None`: no system row).
    pub(crate) fn system_type(&self, package: &str) -> Result<Option<SystemFixtureType>, String> {
        match &self.by_package {
            None => Err("the system fixture table has not resolved".into()),
            Some(Err(reason)) => Err(reason.clone()),
            Some(Ok(by_package)) => Ok(by_package.get(package).copied().flatten()),
        }
    }

    /// `SetupSystemFixture` for a placed package of `fixture_type`.
    pub(crate) fn player_action(
        &self,
        fixture_type: FixtureType,
        package: &str,
    ) -> Result<Option<PlayerActionType>, String> {
        if fixture_type == FixtureType::Gate {
            return Ok(setup_system_fixture(fixture_type, None));
        }
        Ok(setup_system_fixture(
            fixture_type,
            self.system_type(package)?,
        ))
    }

    /// `MysekaiFixtureUtility.IsMusicPlay(fixtureId)`: the fixture's system
    /// fixture row is of type music_play.
    #[allow(dead_code)] // Read by the BGM select screen's entry.
    pub(crate) fn is_music_play(&self, master_fixture_id: i32) -> bool {
        matches!(
            &self.system,
            Some(Ok(rows)) if rows.get(&master_fixture_id) == Some(&SystemFixtureType::MusicPlay)
        )
    }
}

/// Startup: request both tables.
pub(crate) fn load(
    mut fixtures: ResMut<SystemFixtures>,
    mut masters: ResMut<MasterData>,
    server: Res<AssetServer>,
) {
    fixtures.fixture_master = Some(server.load(moly_assets::mysekai_fixtures()));
    masters.request(&SYSTEM_FIXTURES);
}

/// Update: classify every package once both tables are in.
pub(crate) fn resolve(
    mut fixtures: ResMut<SystemFixtures>,
    mut masters: ResMut<MasterData>,
    server: Res<AssetServer>,
    jsons: Res<Assets<JsonAsset>>,
) {
    if fixtures.resolved() {
        return;
    }
    if fixtures.system.is_none() {
        match masters.take(&SYSTEM_FIXTURES) {
            None => return,
            Some(result) => fixtures.system = Some(result.map_err(|error| error.to_string())),
        }
    }
    let Some(handle) = fixtures.fixture_master.clone() else {
        return;
    };
    let master = match server.load_state(&handle) {
        bevy::asset::LoadState::Failed(error) => Err(format!(
            "the fixture master {} failed to load ({error})",
            moly_assets::mysekai_fixtures()
        )),
        _ => match jsons.get(&handle) {
            None => return,
            Some(asset) => serde_json::from_str::<Value>(&asset.0)
                .map_err(|error| format!("the fixture master is not JSON ({error})")),
        },
    };
    let system = fixtures.system.clone().expect("taken above");
    let classified = master.and_then(|master| classify(&master, system?));
    match &classified {
        Ok(by_package) => {
            let mut counts: HashMap<SystemFixtureType, usize> = HashMap::new();
            for kind in by_package.values().flatten() {
                *counts.entry(*kind).or_default() += 1;
            }
            let mut counts: Vec<_> = counts.into_iter().collect();
            counts.sort_by_key(|(kind, _)| *kind as i32);
            info!(
                "[system-fixture] SetupSystemFixture table: {} packages with a system fixture row, by type {counts:?}; stacked actions {STACKED_ACTIONS:?}",
                by_package.values().flatten().count()
            );
        }
        Err(reason) => {
            error!("[system-fixture] no system fixture offers a button on this root: {reason}")
        }
    }
    fixtures.by_package = Some(classified);
}

/// Package -> system fixture type of every master row naming it.
fn classify(
    master: &Value,
    system: HashMap<i32, SystemFixtureType>,
) -> Result<HashMap<String, Option<SystemFixtureType>>, String> {
    let rows = master["fixtures"]
        .as_array()
        .ok_or("the fixture master has no fixtures array")?;
    let mut by_package: HashMap<String, Option<SystemFixtureType>> = HashMap::new();
    for row in rows {
        let id = row["id"]
            .as_i64()
            .and_then(|id| i32::try_from(id).ok())
            .ok_or("a fixture master row has no 32-bit id")?;
        let bundle = row["assetbundleName"]
            .as_str()
            .ok_or_else(|| format!("fixture master row {id} has no assetbundleName"))?;
        let package = format!("mysekai__fixture__{bundle}");
        let kind = system.get(&id).copied();
        if let Some(previous) = by_package.get(&package) {
            if *previous != kind {
                return Err(format!(
                    "package {package} is named by fixture master rows with system fixture types {previous:?} and {kind:?}; placed rows are identified by package"
                ));
            }
        }
        by_package.insert(package, kind);
    }
    Ok(by_package)
}

pub(crate) fn install(app: &mut App) {
    app.init_resource::<SystemFixtures>()
        .add_systems(Startup, load)
        .add_systems(Update, resolve.before(crate::action_button::advance));
}
