//! Explicit offline site inputs. Missing levels defer to that site's saved
//! selection, then its named offline default. Home and room levels are separate.
//! These are expansion-stage overrides, never a fake Mysekai rank progression.

use moly_game::site::{OfflineSceneContent, SiteRequest};

const DEFAULT_SITE: &str = "grassland";

fn parse_content(raw: Option<&str>) -> Result<OfflineSceneContent, String> {
    match raw.map(str::trim) {
        None | Some("compact") => Ok(OfflineSceneContent::Compact),
        Some("full") => Ok(OfflineSceneContent::Full),
        _ => Err("scene_content must be compact or full".to_owned()),
    }
}

fn parse_level(raw: Option<&str>, name: &str) -> Result<Option<u32>, String> {
    raw.map(|raw| raw.trim().parse::<u32>().ok().filter(|v| *v != 0)
        .ok_or_else(|| format!("{name} must be a positive level number"))).transpose()
}

#[cfg(not(target_arch = "wasm32"))]
pub fn resolve() -> Result<SiteRequest, String> {
    let env = |name: &str| -> Result<Option<String>, String> {
        match std::env::var(name) {
            Ok(value) => Ok(Some(value)),
            Err(std::env::VarError::NotPresent) => Ok(None),
            Err(_) => Err(format!("{name} is not Unicode")),
        }
    };
    let site = match env("MOLY_SITE")? {
        Some(raw) if !raw.trim().is_empty() => raw.trim().to_owned(),
        Some(_) => return Err("MOLY_SITE is set but empty; it names a site type".into()),
        None => moly_game::player_data::preferred_site().unwrap_or_else(|| DEFAULT_SITE.to_owned()),
    };
    let room_level = parse_level(env("MOLY_ROOM_LEVEL")?.as_deref(), "MOLY_ROOM_LEVEL")?;
    let offline_home_level = parse_level(env("MOLY_OFFLINE_HOME_LEVEL")?.as_deref(), "MOLY_OFFLINE_HOME_LEVEL")?;
    let content = parse_content(env("MOLY_SCENE_CONTENT")?.as_deref())?;
    Ok(SiteRequest { site, room_level, offline_home_level, content })
}

#[cfg(target_arch = "wasm32")]
pub fn resolve() -> Result<SiteRequest, String> {
    let window = web_sys::window().ok_or_else(|| "the wasm build only runs inside a page".to_owned())?;
    let search = window.location().search().map_err(|e| format!("could not read query string: {e:?}"))?;
    let params = web_sys::UrlSearchParams::new_with_str(&search)
        .map_err(|e| format!("could not parse query string: {e:?}"))?;
    let site = match params.get("site") {
        Some(raw) if !raw.trim().is_empty() => raw.trim().to_owned(),
        Some(_) => return Err("?site= is empty; it names a site type".into()),
        None => moly_game::player_data::preferred_site().unwrap_or_else(|| DEFAULT_SITE.to_owned()),
    };
    let room_level = parse_level(params.get("level").as_deref(), "level")?;
    let offline_home_level = parse_level(params.get("offline_home_level").as_deref(), "offline_home_level")?;
    let content = parse_content(params.get("scene_content").as_deref())?;
    Ok(SiteRequest { site, room_level, offline_home_level, content })
}
