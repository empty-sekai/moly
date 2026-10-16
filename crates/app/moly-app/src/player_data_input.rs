//! Optional startup inputs for the shared player-data importer.

use bevy::prelude::App;
use moly_game::player_data::PlayerDataImport;

#[cfg(not(target_arch = "wasm32"))]
pub fn configure(app: &mut App) -> Result<(), String> {
    let env = |name: &str| -> Result<Option<String>, String> {
        match std::env::var(name) {
            Ok(value) => Ok(Some(value.trim().to_owned())),
            Err(std::env::VarError::NotPresent) => Ok(None),
            Err(_) => Err(format!("{name} is not Unicode")),
        }
    };
    let uid = env("MOLY_PLAYER_UID")?;
    let file = std::env::var_os("MOLY_PLAYER_DATA_FILE");
    if uid.is_some() && file.is_some() {
        return Err("Choose either MOLY_PLAYER_UID or MOLY_PLAYER_DATA_FILE".into());
    }
    let mut state = app.world_mut().resource_mut::<PlayerDataImport>();
    state.configure(env("MOLY_PLAYER_API")?, env("MOLY_PLAYER_REGION")?)?;
    if let Some(uid) = uid {
        state.request_uid(uid);
    }
    if let Some(file) = file {
        let text = moly_game::player_data::read_json_file(std::path::Path::new(&file))?;
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
