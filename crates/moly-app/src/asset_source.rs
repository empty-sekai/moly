//! 资产根的解析——全树唯一允许的拒绝点。
//! native 读 `MOLY_ASSET_ROOT`；web 没有 env，读页面的 `?assets=` URL 前缀。
//!
//! web 选「URL 参数」不选「同目录约定」：约定无法同步判「缺」——要探测就得
//! 发请求，失败会散成逐资产 404，唯一拒绝点就没了；参数缺了当场拒绝。
//! 前缀是同源绝对路径或明确许可的公共资源 HTTPS 根，且以 `/` 结尾。
//! 玩家数据与页面通信仍在同源 iframe 内，只有公开资源走 CDN。

use moly_assets::AssetSource;

fn validate_catalog_id(catalog: &Option<String>) -> Result<(), String> {
    if catalog.as_deref().is_some_and(|id| {
        id.len() != 64
            || !id
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    }) {
        return Err("asset catalog must be a full lowercase SHA-256".into());
    }
    Ok(())
}

/// Only immutable public snapshots and the content-addressed store may use CDN.
fn is_public_asset_path(path: &str) -> bool {
    if path == "/moly/asset-store/" {
        return true;
    }
    let Some(id) = path
        .strip_prefix("/moly/snapshots/")
        .and_then(|rest| rest.strip_suffix("/assets/"))
    else {
        return false;
    };
    id.len() <= 96
        && id.as_bytes().first().is_some_and(u8::is_ascii_alphanumeric)
        && id.bytes().all(|b| {
            b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'.' | b'_' | b'-')
        })
}

/// Split the strict HTTPS representation supplied by the host. Browser Url
/// parsing below is authoritative for host/port/IP canonicalization; this
/// shared admission rejects ambiguous separators and credentials beforehand.
fn public_asset_parts(base: &str) -> Option<(&str, &str)> {
    let authority_and_path = base.strip_prefix("https://")?;
    let slash = authority_and_path.find('/')?;
    let authority = &authority_and_path[..slash];
    if authority.is_empty()
        || authority.bytes().any(|b| {
            !b.is_ascii_lowercase()
                && !b.is_ascii_digit()
                && !matches!(b, b'.' | b'-' | b'_' | b':' | b'[' | b']')
        })
    {
        return None;
    }
    let offset = "https://".len() + slash;
    Some((&base[..offset], &base[offset..]))
}

/// Asset roots are canonical paths, optionally on a configured HTTPS CDN.
pub fn validate_asset_prefix(base: &str) -> Result<(), String> {
    let path = if base.starts_with("https://") {
        let (_, path) = public_asset_parts(base).ok_or("Invalid public Moly resource directory")?;
        if !is_public_asset_path(path) {
            return Err("Invalid public Moly resource directory".into());
        }
        path
    } else {
        base
    };
    if !path.starts_with('/')
        || path.starts_with("//")
        || !path.ends_with('/')
        || path
            .bytes()
            .any(|b| b <= b' ' || b == 0x7f || matches!(b, b'\\' | b'%' | b'?' | b'#'))
        || path.split('/').any(|part| matches!(part, "." | ".."))
    {
        return Err("?assets= must be a canonical same-origin directory or trusted public resource root ending in /".into());
    }
    Ok(())
}

/// The selected remote assets must use the origin explicitly passed by the
/// same-origin host. A second remote origin cannot arrive through assets alone.
pub fn validate_asset_selection(base: &str, configured_origin: Option<&str>) -> Result<(), String> {
    validate_asset_prefix(base)?;
    if let Some((origin, _)) = public_asset_parts(base) {
        if configured_origin != Some(origin) {
            return Err("Remote assets must match the configured resource_origin".into());
        }
    }
    Ok(())
}

#[cfg(test)]
mod public_resource_tests {
    use super::{validate_asset_prefix, validate_asset_selection};

    #[test]
    fn public_immutable_roots_accept_different_configured_https_origins() {
        for value in [
            "/moly/sources/local/",
            "https://assets-one.example/moly/snapshots/cn-6.0.0-a/assets/",
            "https://cdn-two.example:8443/moly/asset-store/",
        ] {
            assert!(validate_asset_prefix(value).is_ok(), "{value}");
        }
        for value in [
            "http://cdn.example/moly/asset-store/",
            "https://user@cdn.example/moly/asset-store/",
            "https://cdn.example/api/player/",
            "https://cdn.example/moly/sources/private/",
            "https://cdn.example/moly/snapshots/../cn-a/assets/",
            "https://cdn.example/moly/snapshots/cn-a/assets/?token=private",
            "https://cdn.example/moly/snapshots/cn-a/assets/%2f",
            "https://cdn.example/moly/snapshots/cn-a/assets/#fragment",
            "//cdn.example/moly/asset-store/",
        ] {
            assert!(validate_asset_prefix(value).is_err(), "{value}");
        }
    }

    #[test]
    fn remote_assets_require_the_selected_origin_without_a_baked_domain() {
        for origin in ["https://assets-one.example", "https://cdn-two.example:8443"] {
            let root = format!("{origin}/moly/asset-store/");
            assert!(validate_asset_selection(&root, Some(origin)).is_ok());
            assert!(validate_asset_selection(&root, None).is_err());
            assert!(validate_asset_selection(&root, Some("https://unselected.example")).is_err());
        }
        assert!(validate_asset_selection("/moly/sources/local/", None).is_ok());
    }
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
    let catalog = std::env::var("MOLY_ASSET_CATALOG").ok();
    validate_catalog_id(&catalog)?;
    if catalog.is_some() || path.join("asset-packs.json").is_file() {
        Ok(AssetSource::NativePacks { path, catalog })
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
    let configured_origin = params.get("resource_origin");
    validate_asset_selection(&base, configured_origin.as_deref())?;
    let location = window.location();
    let page = location.href().map_err(|_| "Could not read page URL")?;
    let url = web_sys::Url::new_with_base(&base, &page).map_err(|_| "Invalid asset URL prefix")?;
    let public_parts = public_asset_parts(&base);
    let allowed_origin = public_parts.map(|(origin, _)| origin.to_owned()).unwrap_or(
        location
            .origin()
            .map_err(|_| "Could not read page origin")?,
    );
    if url.origin() != allowed_origin
        || url.pathname() != public_parts.map(|(_, path)| path).unwrap_or(&base)
        || (public_parts.is_some() && url.href() != base)
        || !url.search().is_empty()
        || !url.hash().is_empty()
    {
        return Err("?assets= must resolve to its canonical trusted resource path".into());
    }
    let catalog = params.get("asset_catalog");
    validate_catalog_id(&catalog)?;
    match params.get("packs").as_deref() {
        Some("1") => Ok(AssetSource::HttpPacks { url: base, catalog }),
        None | Some("0") if catalog.is_none() => Ok(AssetSource::HttpBase { url: base }),
        None | Some("0") => Err("asset_catalog requires packs=1".into()),
        _ => Err("?packs= must be 0 or 1".into()),
    }
}
