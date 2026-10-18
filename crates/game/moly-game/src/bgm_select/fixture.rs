//! The player view's system fixture icon: the music-play fixture type's icon
//! from `mysekaiSystemFixtureTypeIcons`.
//!
//! `SystemFixtureIcon.Setup(MysekaiSystemFixtureType.music_play)` reads the
//! row of the type and loads its `assetBundleName` through the texture
//! loader (a RawImage's texture, not a sprite);
//! `AssetBundleNames.GetMysekaiSystemFixtureTypeName` is the identity, so
//! the package and its file share the name. A region whose master hosts do
//! not serve the row, or whose UI root carries no such texture, shows no
//! icon.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use bevy::prelude::*;
use moly_assets::json::master::{self, MasterData, MasterTable};

use super::{MUSIC_PLAY_FIXTURE, TEXTURE_PREFIX};

fn parse_icons(text: &str) -> Result<HashMap<String, String>, String> {
    let rows = master::read_rows(text, |row| {
        Ok(Some((
            master::text(row, "mysekaiSystemFixtureType")?.to_owned(),
            master::text(row, "assetBundleName")?.to_owned(),
        )))
    })?;
    Ok(rows.into_iter().collect())
}

/// The type name to the package of its icon.
pub(crate) const FIXTURE_TYPE_ICONS: MasterTable<HashMap<String, String>> = MasterTable {
    table: "mysekaiSystemFixtureTypeIcons",
    name: "mysekaiSystemFixtureTypeIcons (the BGM select fixture icon)",
    parse: parse_icons,
};

/// The type icons: `None` while they load, `Some` once they have (empty when
/// the region's master hosts do not serve the table).
fn icons() -> &'static Mutex<Option<HashMap<String, String>>> {
    static ICONS: OnceLock<Mutex<Option<HashMap<String, String>>>> = OnceLock::new();
    ICONS.get_or_init(|| Mutex::new(None))
}

/// Requests the type icon table.
pub(crate) fn request(masters: &mut MasterData) {
    masters.request(&FIXTURE_TYPE_ICONS);
}

/// Update: files the rows once they resolve; a table the hosts do not serve
/// is named once and leaves the icon off.
pub(crate) fn keep(mut data: ResMut<MasterData>) {
    let mut icons = icons().lock().unwrap_or_else(|poison| poison.into_inner());
    if icons.is_some() {
        return;
    }
    let Some(result) = data.take(&FIXTURE_TYPE_ICONS) else {
        return;
    };
    match result {
        Ok(rows) => *icons = Some(rows),
        Err(error) => {
            warn!(
                "[bgm-select] the fixture type icons are absent ({error}); the player view's fixture icon stays off"
            );
            *icons = Some(HashMap::new());
        }
    }
}

/// The alias of the music-play icon once the rows have resolved and the UI
/// root carries the type's texture of that name.
pub(crate) fn music_play_alias(layouts: &crate::ui_layout::UiLayouts) -> Option<String> {
    let icons = icons().lock().unwrap_or_else(|poison| poison.into_inner());
    let texture = icons.as_ref()?.get(MUSIC_PLAY_FIXTURE)?;
    let alias = format!("{TEXTURE_PREFIX}{MUSIC_PLAY_FIXTURE}/{texture}");
    layouts.has_runtime_texture(&alias).then_some(alias)
}
