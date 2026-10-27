//! 资产根的解析入口。
//!
//! native 读 `MOLY_ASSET_ROOT`（缺省退回启动器装好的资源），由
//! `crate::asset_source` 实现——native 游戏与 server 共用那一个函数；
//! web 没有 env，读页面的 `?assets=` URL 前缀，浏览器游戏读启动种子里的同名字段，
//! 两条路走同一套准入（准入也在 `crate::asset_source`）。
//!
//! web 选「URL 参数」不选「同目录约定」：约定无法同步判「缺」——要探测就得
//! 发请求，失败会散成逐资产 404，唯一拒绝点就没了；参数缺了当场拒绝。
//! 前缀是同源绝对路径或明确许可的公共资源 HTTPS 根，且以 `/` 结尾。
//! 玩家数据与页面通信仍在同源 iframe 内，只有公开资源走 CDN。

#[cfg(target_arch = "wasm32")]
use crate::AssetSource;

pub use crate::asset_source::{
    is_canonical_directory, public_asset_parts, validate_asset_selection, RemoteBaseOverrides,
    MASTER_BASE, MASTER_HARUKI_BASE, MASTER_MIRROR_BASE, STORAGE_BASE, STORAGE_MIRROR_BASE,
};

/// Native: the resource root, read by the same function the server reads.
#[cfg(not(target_arch = "wasm32"))]
pub use crate::asset_source::{resolve, resolve_remote};

/// Pages and the browser game: the same bases the native entry reads, with
/// each one required to be its canonical URL. The repository template itself
/// is checked with a repository substituted for `{repo}`, which the URL of a
/// `{repo}` segment would otherwise escape.
#[cfg(target_arch = "wasm32")]
pub fn canonical_bases(
    bases: crate::remote::RemoteBases,
) -> Result<crate::remote::RemoteBases, String> {
    let checked = [
        bases.storage.clone(),
        bases.storage_mirror.clone(),
        bases.master.clone(),
        bases.master_mirror.clone(),
        MASTER_HARUKI_BASE.replace("{repo}", crate::remote::Region::Jp.repository()),
    ];
    for base in &checked {
        let url = web_sys::Url::new(base).map_err(|_| format!("Invalid remote base {base}"))?;
        if url.href() != *base || !url.search().is_empty() || !url.hash().is_empty() {
            return Err(format!(
                "remote base {base} must be its canonical HTTPS directory"
            ));
        }
    }
    Ok(bases)
}

/// Pages: `?storage_base=`, `?storage_mirror_base=`, `?master_base=`,
/// `?master_mirror_base=` and `?master_haruki_base=` (a template naming
/// `{repo}`) replace the hosts.
#[cfg(target_arch = "wasm32")]
pub fn resolve_remote() -> Result<crate::remote::RemoteBases, String> {
    let window =
        web_sys::window().ok_or_else(|| "the wasm build only runs inside a page".to_owned())?;
    let search = window
        .location()
        .search()
        .map_err(|e| format!("could not read the page query string: {e:?}"))?;
    let params = web_sys::UrlSearchParams::new_with_str(&search)
        .map_err(|e| format!("could not parse the query string {search:?}: {e:?}"))?;
    let (storage, storage_mirror, master, master_mirror, master_repository) = (
        params.get("storage_base"),
        params.get("storage_mirror_base"),
        params.get("master_base"),
        params.get("master_mirror_base"),
        params.get("master_haruki_base"),
    );
    let bases = crate::asset_source::remote_bases(RemoteBaseOverrides {
        storage: storage.as_deref(),
        storage_mirror: storage_mirror.as_deref(),
        master: master.as_deref(),
        master_mirror: master_mirror.as_deref(),
        master_repository: master_repository.as_deref(),
    })?;
    canonical_bases(bases)
}

/// One admission for the page query and the game seed.
#[cfg(target_arch = "wasm32")]
pub fn resolve_selection(
    base: String,
    packs: bool,
    catalog: Option<String>,
    configured_origin: Option<&str>,
    configured_base: Option<&str>,
) -> Result<AssetSource, String> {
    validate_asset_selection(&base, configured_origin, configured_base)?;
    let window =
        web_sys::window().ok_or_else(|| "the wasm build only runs inside a page".to_owned())?;
    if let Some(root) = configured_base {
        let url = web_sys::Url::new(root).map_err(|_| "Invalid resource_base")?;
        if url.href() != root || !url.search().is_empty() || !url.hash().is_empty() {
            return Err("resource_base must be its canonical HTTPS directory".into());
        }
    }
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
    validate_catalog_id(&catalog)?;
    match (packs, catalog) {
        (true, catalog) => Ok(AssetSource::HttpPacks { url: base, catalog }),
        (false, None) => Ok(AssetSource::HttpBase { url: base }),
        (false, Some(_)) => Err("asset_catalog requires packs=1".into()),
    }
}

/// A configured release id is a full lowercase SHA-256.
#[cfg(target_arch = "wasm32")]
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
    let packs = match params.get("packs").as_deref() {
        Some("1") => true,
        None | Some("0") => false,
        _ => return Err("?packs= must be 0 or 1".into()),
    };
    resolve_selection(
        base,
        packs,
        params.get("asset_catalog"),
        params.get("resource_origin").as_deref(),
        params.get("resource_base").as_deref(),
    )
}
