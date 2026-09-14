//! 资产根的解析——全树唯一允许的拒绝点。
//! native 读 `MOLY_ASSET_ROOT`；web 没有 env，读页面的 `?assets=` URL 前缀。
//!
//! web 选「URL 参数」不选「同目录约定」：约定无法同步判「缺」——要探测就得
//! 发请求，失败会散成逐资产 404，唯一拒绝点就没了；参数缺了当场拒绝。
//! 前缀是同源绝对路径且以 `/` 结尾：消费侧在 wasm 上按页源取 HTTP，
//! 丢斜杠或写整段外源 URL 会让一整族资产同时 404，在这里一次抓住。

use moly_assets::AssetSource;

/// Asset roots are canonical same-origin directory paths, not URL references.
pub fn validate_asset_prefix(base: &str) -> Result<(), String> {
    if !base.starts_with('/')
        || base.starts_with("//")
        || !base.ends_with('/')
        || base
            .bytes()
            .any(|b| b <= b' ' || b == 0x7f || matches!(b, b'\\' | b'%' | b'?' | b'#'))
        || base.split('/').any(|part| matches!(part, "." | ".."))
    {
        return Err("?assets= must be a canonical same-origin directory path ending in /; URL references, encoded separators, query and fragment are not allowed".into());
    }
    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
pub fn resolve() -> Result<AssetSource, String> {
    let raw = std::env::var("MOLY_ASSET_ROOT").map_err(|_| {
        "MOLY_ASSET_ROOT is not set; point it at the extracted-asset directory".to_owned()
    })?;
    if raw.trim().is_empty() {
        return Err("MOLY_ASSET_ROOT is empty; point it at the extracted-asset directory".into());
    }
    let path = std::path::PathBuf::from(raw);
    if !path.is_dir() {
        return Err(format!(
            "MOLY_ASSET_ROOT is not a directory: {}",
            path.display()
        ));
    }
    if path.join("asset-packs.json").is_file() {
        Ok(AssetSource::NativePacks { path })
    } else {
        Ok(AssetSource::NativeDir { path })
    }
}

#[cfg(target_arch = "wasm32")]
pub fn resolve() -> Result<AssetSource, String> {
    let window =
        web_sys::window().ok_or_else(|| "the wasm build only runs inside a page".to_owned())?;
    let search = window
        .location()
        .search()
        .map_err(|e| format!("could not read the page query string: {e:?}"))?;
    let params = web_sys::UrlSearchParams::new_with_str(&search)
        .map_err(|e| format!("could not parse the query string {search:?}: {e:?}"))?;
    let base = params.get("assets").ok_or_else(|| {
        "missing ?assets=<url-prefix>/ — the HTTP base assets are fetched from".to_owned()
    })?;
    validate_asset_prefix(&base)?;
    let location = window.location();
    let page = location.href().map_err(|_| "Could not read page URL")?;
    let url = web_sys::Url::new_with_base(&base, &page).map_err(|_| "Invalid asset URL prefix")?;
    if url.origin()
        != location
            .origin()
            .map_err(|_| "Could not read page origin")?
        || url.pathname() != base
        || !url.search().is_empty()
        || !url.hash().is_empty()
    {
        return Err("?assets= must resolve to a canonical path on the page origin".into());
    }
    match params.get("packs").as_deref() {
        Some("1") => Ok(AssetSource::HttpPacks { url: base }),
        None | Some("0") => Ok(AssetSource::HttpBase { url: base }),
        _ => Err("?packs= must be 0 or 1".into()),
    }
}
