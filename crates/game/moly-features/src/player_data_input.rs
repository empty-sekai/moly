//! Optional startup inputs for the shared player-data importer.

use crate::player_data::PlayerDataImport;
use bevy::prelude::App;

#[cfg(not(target_arch = "wasm32"))]
pub fn configure(app: &mut App) -> Result<(), String> {
    use moly_config::keys;
    let config = moly_config::global();
    let value = |key| config.text(key).map(|value| value.trim().to_owned());
    let uid = value(&keys::PLAYER_UID);
    let file = config.text(&keys::PLAYER_DATA_FILE);
    if uid.is_some() && file.is_some() {
        return Err("Choose either player_data.uid (MOLY_PLAYER_UID) or player_data.file (MOLY_PLAYER_DATA_FILE)".into());
    }
    let mut state = app.world_mut().resource_mut::<PlayerDataImport>();
    state.configure(value(&keys::PLAYER_API), value(&keys::PLAYER_REGION))?;
    if let Some(uid) = uid {
        state.request_uid(uid);
    }
    if let Some(file) = file {
        let text = crate::player_data::read_json_file(std::path::Path::new(&file))?;
        state.request_json(text, true);
    }
    Ok(())
}

#[cfg(target_arch = "wasm32")]
pub fn configure(app: &mut App) -> Result<(), String> {
    let window = web_sys::window().ok_or("Browser window is unavailable")?;
    let search = window
        .location()
        .search()
        .map_err(|_| "Could not read player import parameters")?;
    let params = web_sys::UrlSearchParams::new_with_str(&search)
        .map_err(|_| "Invalid player import parameters")?;
    let mut state = app.world_mut().resource_mut::<PlayerDataImport>();
    state.configure(params.get("player_api"), params.get("player_region"))?;
    if let Some(uid) = params.get("player_uid") {
        state.request_uid(uid);
    }
    Ok(())
}
